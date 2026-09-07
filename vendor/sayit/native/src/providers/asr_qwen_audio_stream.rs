// 千问 Qwen-Audio-3.0 流式语音识别 — qwen-audio-3.0-asr-flash-streaming
//
// ⚠️ 协议与 asr_qwen_realtime.rs **不是同一套**，别指望改个模型名就能复用：
//   · asr_qwen_realtime → `/api-ws/v1/realtime`，OpenAI-Realtime 风格事件
//     （session.update / input_audio_buffer.append，音频 base64 塞进 JSON）；
//   · 这一份            → `/api-ws/v1/inference`，DashScope 原生 duplex 指令
//     （run-task → task-started → 裸二进制音频帧 → result-generated →
//      finish-task → task-finished）。
//
// 实测结论（`dev-scripts/probe_qwen_audio30_stream.py`，打的是真实接口）：
//   · 通用域名 `dashscope.aliyuncs.com` 与业务空间专属域名都能用 —— 这个模型
//     **不需要 WorkspaceId**（qwen3-asr-flash-realtime 才需要）。填了就走专属域名
//     （阿里官方称更稳），没填照样工作，所以设置页不把它列为必填项。
//   · `result-generated` 里 `sentence.text` 是**当前句的全量文本**，不是增量。
//   · 开头十几条 `result-generated` 的 text 是空串（VAD 还没判出语音），必须丢掉，
//     否则悬浮窗会先闪一下空字幕。
//   · 每个 `sentence_end=true` 的句子自带句尾标点（中文「。」、英文 ". "），
//     所以分句拼接**不加分隔符**；asr_qwen_realtime 那边补「，」是因为它的分句不带标点。
//   · 音频不按实时节奏发也能用（10.84s 音频一次性推完，2.6s 出全文），
//     所以一次性识别那条路不需要 sleep 假装实时。

use super::diag;
use super::types::{AsrProviderConfig, AsrResult, TestResult};
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{Sink, SinkExt, StreamExt};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite;
use tungstenite::client::IntoClientRequest;
use tungstenite::http::header::{AUTHORIZATION, USER_AGENT};
use tungstenite::http::HeaderValue;

const MODEL: &str = "qwen-audio-3.0-asr-flash-streaming";
const SCOPE: &str = "qwen/audio30";
const WS_PATH: &str = "/api-ws/v1/inference";
/// 没配业务空间时的通用域名。实测与专属域名同样可用。
const GENERIC_HOST: &str = "dashscope.aliyuncs.com";

/// 音频帧大小：16kHz 单声道 16-bit 下正好 100ms。
///
/// 官方示例用的就是 3200 字节，probe 脚本也是按这个尺寸实测通过的 ——
/// 一次性路径不改成"一大帧发完"是因为那个形状没验证过，而这个验证过。
const FRAME_BYTES: usize = 3200;

/// 即时热词权重。文档区间 [1,5]，越大越倾向输出该词；50 是「超级热词」
/// （召回大幅提升但最多 50 个）。取 4 是要偏置效果又不想把普通词条推成超级热词。
const HOTWORD_WEIGHT: u32 = 4;

/// 即时热词条数上限。文档只对「超级热词」明确了 50 的上限，普通热词没给数字；
/// 这里自己设一个上限并记日志，避免词库很大时 run-task 被整条拒掉 ——
/// 那会让识别**完全不可用**，而不是"热词少生效几个"。
const HOTWORD_LIMIT: usize = 100;

type WsStream = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;
type WsSink = SplitSink<WsStream, tungstenite::Message>;

// ─────────────────────────── 协议编解码 ───────────────────────────

fn ws_url(workspace: &str) -> String {
    let workspace = workspace.trim();
    if workspace.is_empty() {
        format!("wss://{}{}", GENERIC_HOST, WS_PATH)
    } else {
        format!("wss://{}.cn-beijing.maas.aliyuncs.com{}", workspace, WS_PATH)
    }
}

/// 热词 → `parameters.vocabulary`（`{词: 权重}`）。
///
/// 与 asr_qwen.rs 的做法不同：那边只能把热词拼成一段上下文文本，这个模型有专门的
/// 即时热词字段，权重可控，也不存在「模型把整串热词当识别结果吐回来」的回显风险。
fn build_vocabulary(hotwords: &[String]) -> Option<(serde_json::Value, usize, usize)> {
    let mut seen = std::collections::HashSet::new();
    let words: Vec<&str> = hotwords
        .iter()
        .map(|w| w.trim())
        .filter(|w| !w.is_empty())
        .filter(|w| seen.insert(w.to_lowercase()))
        .collect();
    if words.is_empty() {
        return None;
    }
    let total = words.len();
    let mut map = serde_json::Map::new();
    for w in words.iter().take(HOTWORD_LIMIT) {
        map.insert((*w).to_string(), serde_json::json!(HOTWORD_WEIGHT));
    }
    let used = map.len();
    Some((serde_json::Value::Object(map), used, total))
}

fn run_task_payload(task_id: &str, sample_rate: u32, hotwords: &[String]) -> serde_json::Value {
    let mut parameters = serde_json::json!({
        "format": "pcm",
        "sample_rate": sample_rate,
    });
    if let Some((vocabulary, used, total)) = build_vocabulary(hotwords) {
        parameters["vocabulary"] = vocabulary;
        diag::log(
            SCOPE,
            "hotwords",
            &format!("vocabulary={} of={} weight={}", used, total, HOTWORD_WEIGHT),
        );
    }
    serde_json::json!({
        "header": { "action": "run-task", "task_id": task_id, "streaming": "duplex" },
        "payload": {
            "task_group": "audio",
            "task": "asr",
            "function": "recognition",
            "model": MODEL,
            "parameters": parameters,
            "input": {},
        }
    })
}

fn finish_task_payload(task_id: &str) -> serde_json::Value {
    serde_json::json!({
        "header": { "action": "finish-task", "task_id": task_id, "streaming": "duplex" },
        "payload": { "input": {} }
    })
}

fn event_name(ev: &serde_json::Value) -> &str {
    ev.get("header")
        .and_then(|h| h.get("event"))
        .and_then(|e| e.as_str())
        .unwrap_or("")
}

/// task-failed 的原因。错误码和文案都在 header 里，不在 payload。
fn task_failed_reason(ev: &serde_json::Value) -> String {
    let header = ev.get("header");
    let code = header
        .and_then(|h| h.get("error_code"))
        .and_then(|c| c.as_str())
        .unwrap_or("unknown");
    let message = header
        .and_then(|h| h.get("error_message"))
        .and_then(|m| m.as_str())
        .unwrap_or("Unknown error");
    format!("{}: {}", code, diag::truncate(message, 200))
}

/// `payload.output.sentence` → (当前句文本, 是否已断句)
fn sentence_of(ev: &serde_json::Value) -> Option<(&str, bool)> {
    let sentence = ev.get("payload")?.get("output")?.get("sentence")?;
    let text = sentence.get("text").and_then(|t| t.as_str()).unwrap_or("");
    let ended = sentence
        .get("sentence_end")
        .and_then(|e| e.as_bool())
        .unwrap_or(false);
    Some((text, ended))
}

/// 一次会话累积到的文本。committed = 已断句的句子，partial = 进行中的那一句。
#[derive(Default)]
struct Transcript {
    committed: String,
    partial: String,
    finished: bool,
    error: Option<String>,
    /// 收到过多少条带文本的中间结果，只用于日志
    partial_events: usize,
}

/// 一条事件对累积状态做了什么，调用方据此决定要不要上抛 / 退出循环。
enum Applied {
    /// 与文本无关的事件（task-started、空文本的中间结果等）
    Ignored,
    /// 可显示文本变了
    Updated,
    Finished,
    Failed,
}

impl Transcript {
    /// 实时展示用文本 = 已断句 + 进行中。分句自带标点，所以直接相接。
    fn display(&self) -> String {
        format!("{}{}", self.committed, self.partial)
    }

    fn apply(&mut self, ev: &serde_json::Value) -> Applied {
        match event_name(ev) {
            "result-generated" => {
                let Some((text, ended)) = sentence_of(ev) else {
                    return Applied::Ignored;
                };
                // 开头那十几条空文本的中间结果必须丢掉，否则悬浮窗先闪一下空字幕
                if text.is_empty() {
                    return Applied::Ignored;
                }
                if ended {
                    self.committed.push_str(text);
                    self.partial.clear();
                } else {
                    // text 是当前句的全量文本（不是增量），直接覆盖
                    self.partial.clear();
                    self.partial.push_str(text);
                }
                self.partial_events += 1;
                Applied::Updated
            }
            "task-finished" => {
                self.finished = true;
                Applied::Finished
            }
            "task-failed" => {
                self.error = Some(task_failed_reason(ev));
                self.finished = true;
                Applied::Failed
            }
            _ => Applied::Ignored,
        }
    }
}

// ─────────────────────────── 建连与开任务 ───────────────────────────

struct Task {
    ws: WsStream,
    task_id: String,
}

/// 建连 → 发 run-task → 等 task-started。拿到 Task 才算可以发音频。
async fn start_task(
    config: &AsrProviderConfig,
    workspace: &str,
    sample_rate: u32,
    hotwords: &[String],
) -> Result<Task, String> {
    if config.api_key.trim().is_empty() {
        return Err(diag::fail_code(
            SCOPE,
            "credentials",
            "provider_bad_key",
            "Qwen ASR is missing the API Key; complete it in Settings".to_string(),
        ));
    }

    let url = ws_url(workspace);
    let mut request = url.as_str().into_client_request().map_err(|e| {
        diag::fail(
            SCOPE,
            "build_request",
            format!("Failed to build request: {}", e),
        )
    })?;
    request.headers_mut().insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", config.api_key.trim())).map_err(|e| {
            diag::fail_code(
                SCOPE,
                "authorization_header",
                "provider_bad_key",
                format!("Invalid Authorization header: {}", e),
            )
        })?,
    );
    // UA 不是这个端点的硬要求（实测不带也能连），但我们所有出网请求都带：
    // 自有 ALB 与第三方 nginx 网关会按「无 UA」直接 403，且报错里绝不提 UA。
    request.headers_mut().insert(
        USER_AGENT,
        HeaderValue::from_static(concat!("SayIt/", env!("CARGO_PKG_VERSION"))),
    );
    let workspace = workspace.trim();
    if !workspace.is_empty() {
        if let Ok(value) = HeaderValue::from_str(workspace) {
            request.headers_mut().insert("X-DashScope-WorkSpace", value);
        }
    }

    let (mut ws, response) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|e| {
            diag::fail(
                SCOPE,
                "connect",
                format!("WebSocket connection failed: {}", e),
            )
        })?;

    let task_id = uuid::Uuid::new_v4().to_string();
    diag::log(
        SCOPE,
        "connected",
        &format!(
            "status={} model={} host={} rate={}",
            response.status(),
            MODEL,
            if workspace.is_empty() { GENERIC_HOST } else { "workspace" },
            sample_rate
        ),
    );

    let payload = run_task_payload(&task_id, sample_rate, hotwords);
    ws.send(tungstenite::Message::Text(
        serde_json::to_string(&payload).unwrap().into(),
    ))
    .await
    .map_err(|e| {
        diag::fail(
            SCOPE,
            "send_run_task",
            format!("Failed to send run-task: {}", e),
        )
    })?;

    // 必须等到 task-started 才算就绪。额度耗尽 / 模型未开通时服务端回 task-failed
    // 或直接关连接，都要在这里变成 Err，让上层回退到别的路径而不是空转。
    let deadline = Duration::from_secs(15);
    let started = tokio::time::timeout(deadline, async {
        while let Some(msg) = ws.next().await {
            let msg = msg.map_err(|e| {
                diag::fail(
                    SCOPE,
                    "recv_ack",
                    format!("Failed to receive acknowledgement: {}", e),
                )
            })?;
            match msg {
                tungstenite::Message::Text(text) => {
                    let Ok(ev) = serde_json::from_str::<serde_json::Value>(&text) else {
                        continue;
                    };
                    match event_name(&ev) {
                        "task-started" => return Ok(true),
                        "task-failed" => {
                            return Err(diag::fail(
                                SCOPE,
                                "task_failed",
                                format!("Server rejected the task: {}", task_failed_reason(&ev)),
                            ))
                        }
                        _ => {}
                    }
                }
                tungstenite::Message::Close(frame) => {
                    let reason = frame
                        .map(|f| f.reason.to_string())
                        .unwrap_or_else(|| "No reason".to_string());
                    return Err(diag::fail(
                        SCOPE,
                        "closed_before_ready",
                        format!("Server closed the session: {}", reason),
                    ));
                }
                _ => {}
            }
        }
        Ok(false)
    })
    .await
    .map_err(|_| {
        diag::fail(
            SCOPE,
            "task_started_timeout",
            "Timed out waiting for the task to start".to_string(),
        )
    })??;

    if !started {
        return Err(diag::fail(
            SCOPE,
            "closed_before_ready",
            "WebSocket closed before the task started".to_string(),
        ));
    }
    diag::log(SCOPE, "task_started", "");
    Ok(Task { ws, task_id })
}

/// 把 PCM 切成 100ms 的二进制帧发出去。
async fn send_frames<S>(sink: &mut S, pcm: &[u8], stage: &'static str) -> Result<(), String>
where
    S: Sink<tungstenite::Message, Error = tungstenite::Error> + Unpin,
{
    for frame in pcm.chunks(FRAME_BYTES) {
        sink.send(tungstenite::Message::Binary(frame.to_vec().into()))
            .await
            .map_err(|e| diag::fail(SCOPE, stage, format!("Failed to send audio: {}", e)))?;
    }
    Ok(())
}

// ─────────────────────────── 一次性识别 ───────────────────────────

/// 录完再发：整段 PCM 推完 → finish-task → 收全文。
///
/// 设置页的「识别测试」和关掉实时字幕时的录音都走这里，而它们都不传业务空间 ID，
/// 所以这条路必须能用通用域名 —— 已实测可用。
pub async fn transcribe(
    audio_pcm_b64: &str,
    sample_rate: u32,
    config: &AsrProviderConfig,
    hotwords: &[String],
) -> Result<AsrResult, String> {
    let pcm =
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, audio_pcm_b64).map_err(
            |e| {
                diag::fail(
                    SCOPE,
                    "decode_b64",
                    format!("Failed to decode base64 audio: {}", e),
                )
            },
        )?;
    if pcm.is_empty() {
        diag::empty_result(SCOPE, "Input audio was empty; provider request was skipped");
        return Ok(AsrResult {
            text: String::new(),
            elapsed_ms: 0,
        });
    }

    let workspace = config
        .extra
        .get("workspaceId")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let audio_sec = pcm.len() as f64 / (sample_rate.max(1) as f64 * 2.0);
    diag::log(
        SCOPE,
        "start",
        &format!(
            "pcm_bytes={} audio_sec={:.1} rate={} hotwords={}",
            pcm.len(),
            audio_sec,
            sample_rate,
            hotwords.len()
        ),
    );

    let start = Instant::now();
    let Task { mut ws, task_id } = start_task(config, workspace, sample_rate, hotwords).await?;
    send_frames(&mut ws, &pcm, "send_audio").await?;
    ws.send(tungstenite::Message::Text(
        serde_json::to_string(&finish_task_payload(&task_id))
            .unwrap()
            .into(),
    ))
    .await
    .map_err(|e| {
        diag::fail(
            SCOPE,
            "send_finish",
            format!("Failed to send finish-task: {}", e),
        )
    })?;

    let mut transcript = Transcript::default();
    // 上限按音频长度放宽：10.8s 音频实测 2.6s 收全，长音频线性长一些。
    let collect_budget = Duration::from_secs(20) + Duration::from_secs_f64(audio_sec.min(300.0));
    let closed_early = tokio::time::timeout(collect_budget, async {
        while let Some(msg) = ws.next().await {
            let msg = msg.map_err(|e| {
                diag::fail(SCOPE, "recv", format!("Failed to receive result: {}", e))
            })?;
            match msg {
                tungstenite::Message::Text(text) => {
                    let Ok(ev) = serde_json::from_str::<serde_json::Value>(&text) else {
                        continue;
                    };
                    match transcript.apply(&ev) {
                        Applied::Finished => return Ok(false),
                        Applied::Failed => {
                            return Err(diag::fail(
                                SCOPE,
                                "task_failed",
                                format!(
                                    "Recognition failed: {}",
                                    transcript.error.clone().unwrap_or_default()
                                ),
                            ))
                        }
                        _ => {}
                    }
                }
                // 服务端提前关连接：协议没跑完，不能当成"识别到空文本"上报成功，
                // 否则真实原因（额度、权限、协议错）会被显示成「未检测到有效声音」。
                tungstenite::Message::Close(frame) => {
                    let reason = frame
                        .map(|f| f.reason.to_string())
                        .unwrap_or_else(|| "No reason".to_string());
                    diag::log(SCOPE, "closed_before_finished", &reason);
                    return Ok(true);
                }
                _ => {}
            }
        }
        Ok(true)
    })
    .await
    .map_err(|_| {
        diag::fail(
            SCOPE,
            "collect_timeout",
            "Timed out waiting for the recognition result".to_string(),
        )
    })??;

    let _ = ws.close(None).await;
    let text = transcript.display().trim().to_string();
    let elapsed_ms = start.elapsed().as_millis() as u64;

    if closed_early && text.is_empty() {
        return Err(diag::fail(
            SCOPE,
            "closed_before_finished",
            "The server closed the connection before returning a result".to_string(),
        ));
    }
    if text.is_empty() {
        diag::empty_result(
            SCOPE,
            &format!(
                "Task finished with no transcript audio_sec={:.1} elapsed={}ms events={}",
                audio_sec, elapsed_ms, transcript.partial_events
            ),
        );
    } else {
        diag::ok(SCOPE, elapsed_ms, text.chars().count());
    }
    Ok(AsrResult { text, elapsed_ms })
}

pub async fn test_connection(config: &AsrProviderConfig) -> TestResult {
    let start = Instant::now();
    // 只握手 + 开任务 + 立刻收尾：能开出任务就说明密钥、模型、地域都对得上。
    // 不发音频（这个命令只回答"能不能连"，真实转写由设置页的识别测试负责）。
    let result = async {
        let workspace = config
            .extra
            .get("workspaceId")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let Task { mut ws, task_id } = start_task(config, workspace, 16000, &[]).await?;
        let _ = ws
            .send(tungstenite::Message::Text(
                serde_json::to_string(&finish_task_payload(&task_id))
                    .unwrap()
                    .into(),
            ))
            .await;
        let _ = ws.close(None).await;
        Ok::<(), String>(())
    }
    .await;

    let elapsed_ms = start.elapsed().as_millis() as u64;
    match result {
        Ok(()) => TestResult {
            ok: true,
            message: format!("Connection successful ({}ms)", elapsed_ms),
            elapsed_ms,
            detail: format!("model: {}", MODEL),
        },
        Err(message) => TestResult {
            ok: false,
            message,
            elapsed_ms,
            detail: String::new(),
        },
    }
}

// ─────────────────────────── 流式会话 ───────────────────────────

static SINK: once_cell::sync::Lazy<Arc<Mutex<Option<WsSink>>>> =
    once_cell::sync::Lazy::new(|| Arc::new(Mutex::new(None)));
static READER: once_cell::sync::Lazy<Arc<Mutex<Option<JoinHandle<()>>>>> =
    once_cell::sync::Lazy::new(|| Arc::new(Mutex::new(None)));
static STATE: once_cell::sync::Lazy<Arc<Mutex<Transcript>>> =
    once_cell::sync::Lazy::new(|| Arc::new(Mutex::new(Transcript::default())));
static TASK_ID: once_cell::sync::Lazy<Arc<Mutex<String>>> =
    once_cell::sync::Lazy::new(|| Arc::new(Mutex::new(String::new())));
static ACTIVE: AtomicBool = AtomicBool::new(false);

/// 打开流式会话。
///
/// 与 asr_qwen_realtime 不同，这里**不分「实时 / 普通」两条代码路径**：无论要不要
/// 上抛中间结果，都必须有后台 reader 一直读。这个协议在发音频期间每 ~100ms 就推一条
/// result-generated，没人读就会把连接堵死；realtime 只决定要不要 emit。
#[tauri::command]
pub async fn qwen_audio_stream_open(
    app: AppHandle,
    config: AsrProviderConfig,
    hotwords: Option<Vec<String>>,
    realtime: Option<bool>,
    workspace_id: Option<String>,
) -> Result<(), String> {
    let realtime = realtime.unwrap_or(false);
    cleanup().await;

    let hotwords = hotwords.unwrap_or_default();
    let workspace = workspace_id.unwrap_or_default();
    let Task { ws, task_id } = start_task(&config, &workspace, 16000, &hotwords).await?;

    let (sink, stream) = ws.split();
    *STATE.lock().await = Transcript::default();
    *TASK_ID.lock().await = task_id;
    *SINK.lock().await = Some(sink);
    let handle = tokio::spawn(run_reader(stream, app, STATE.clone(), realtime));
    *READER.lock().await = Some(handle);
    ACTIVE.store(true, Ordering::SeqCst);
    diag::log(SCOPE, "stream_open", &format!("realtime={}", realtime));
    Ok(())
}

async fn run_reader(
    mut stream: SplitStream<WsStream>,
    app: AppHandle,
    state: Arc<Mutex<Transcript>>,
    realtime: bool,
) {
    let mut emitted = 0usize;
    let mut ended_cleanly = false;
    let mut close_reason = String::new();
    while let Some(msg) = stream.next().await {
        let text = match msg {
            Ok(tungstenite::Message::Text(t)) => t,
            Ok(tungstenite::Message::Close(frame)) => {
                close_reason = frame
                    .map(|f| f.reason.to_string())
                    .unwrap_or_else(|| "no reason".to_string());
                break;
            }
            Err(e) => {
                close_reason = e.to_string();
                break;
            }
            Ok(_) => continue,
        };
        let Ok(ev) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };

        let (applied, display) = {
            let mut s = state.lock().await;
            let applied = s.apply(&ev);
            (applied, s.display())
        };
        match applied {
            Applied::Updated => {
                if realtime {
                    emitted += 1;
                    if emitted == 1 {
                        diag::log(SCOPE, "first_partial", "");
                    }
                    let _ = app.emit(
                        "asr-partial",
                        serde_json::json!({ "text": display, "provider": "qwen_audio_stream" }),
                    );
                }
            }
            Applied::Finished | Applied::Failed => {
                ended_cleanly = true;
                break;
            }
            Applied::Ignored => {}
        }
    }
    let mut s = state.lock().await;
    s.finished = true;
    // 协议没跑完就断了要记下来。否则 finish 只能返回一个空字符串，而空字符串会被
    // 显示成「未检测到有效声音」，把用户引去查麦克风 —— 真实原因（额度、鉴权、
    // 网络）就此消失。见 pitfalls 第 15 条。
    if !ended_cleanly && s.error.is_none() {
        s.error = Some(format!(
            "connection closed before task-finished: {}",
            diag::truncate(&close_reason, 200)
        ));
    }
    diag::log(
        SCOPE,
        "reader_stopped",
        &format!(
            "emits={} events={} clean={}",
            emitted, s.partial_events, ended_cleanly
        ),
    );
}

async fn cleanup() {
    ACTIVE.store(false, Ordering::SeqCst);
    if let Some(mut sink) = SINK.lock().await.take() {
        let _ = sink.close().await;
    }
    if let Some(handle) = READER.lock().await.take() {
        handle.abort();
    }
    *STATE.lock().await = Transcript::default();
    TASK_ID.lock().await.clear();
}

#[tauri::command]
pub async fn qwen_audio_stream_send(pcm_b64: String) -> Result<(), String> {
    if !ACTIVE.load(Ordering::SeqCst) {
        return Err(diag::fail(
            SCOPE,
            "send_without_session",
            "Session is not open".to_string(),
        ));
    }
    let pcm = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &pcm_b64)
        .map_err(|e| {
            diag::fail(
                SCOPE,
                "decode_b64",
                format!("Failed to decode base64 audio: {}", e),
            )
        })?;
    let mut sink = SINK.lock().await;
    let s = sink.as_mut().ok_or_else(|| {
        diag::fail(
            SCOPE,
            "send_without_session",
            "Session is not open".to_string(),
        )
    })?;
    send_frames(s, &pcm, "send_audio").await
}

#[tauri::command]
pub async fn qwen_audio_stream_finish() -> Result<String, String> {
    if !ACTIVE.load(Ordering::SeqCst) {
        return Err(diag::fail(
            SCOPE,
            "finish_without_session",
            "Session is not open".to_string(),
        ));
    }
    let task_id = TASK_ID.lock().await.clone();
    {
        let mut sink = SINK.lock().await;
        let s = sink.as_mut().ok_or_else(|| {
            diag::fail(
                SCOPE,
                "finish_without_session",
                "Session is not open".to_string(),
            )
        })?;
        // 发送失败不致命：可能服务端已经收尾，继续读已累计的文本即可。
        if let Err(e) = s
            .send(tungstenite::Message::Text(
                serde_json::to_string(&finish_task_payload(&task_id))
                    .unwrap()
                    .into(),
            ))
            .await
        {
            diag::log(SCOPE, "finish_send_err_ignored", &e.to_string());
        }
    }

    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        {
            let st = STATE.lock().await;
            if st.finished {
                let err = st.error.clone();
                let text = st.display().trim().to_string();
                let events = st.partial_events;
                drop(st);
                cleanup().await;
                // 已经识别出文字就交回去，哪怕收尾时断了 —— 丢掉它等于让用户白说一遍。
                // 但断连原因仍要留痕。
                if !text.is_empty() {
                    if let Some(err) = &err {
                        diag::log(SCOPE, "finished_with_error", err);
                    }
                    return Ok(text);
                }
                if let Some(err) = err {
                    return Err(diag::fail(
                        SCOPE,
                        "task_failed",
                        format!("Recognition failed: {}", err),
                    ));
                }
                diag::empty_result(
                    SCOPE,
                    &format!("Task finished with no transcript events={}", events),
                );
                return Ok(String::new());
            }
        }
        if Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }

    // 超时也把已拿到的文本交回去：丢掉它等于让用户白说一遍。
    let text = STATE.lock().await.display().trim().to_string();
    diag::log(
        SCOPE,
        "finish_timeout",
        &format!("returning partial chars={}", text.chars().count()),
    );
    cleanup().await;
    Ok(text)
}

#[tauri::command]
pub async fn qwen_audio_stream_close() -> Result<(), String> {
    cleanup().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 没配业务空间也要能工作：设置页的识别测试就不传它。
    #[test]
    fn ws_url_falls_back_to_the_generic_host() {
        assert_eq!(
            ws_url(""),
            "wss://dashscope.aliyuncs.com/api-ws/v1/inference"
        );
        assert_eq!(
            ws_url("  "),
            "wss://dashscope.aliyuncs.com/api-ws/v1/inference"
        );
        assert_eq!(
            ws_url("ws-abc"),
            "wss://ws-abc.cn-beijing.maas.aliyuncs.com/api-ws/v1/inference"
        );
    }

    #[test]
    fn run_task_payload_matches_the_documented_shape() {
        let payload = run_task_payload("tid", 16000, &[]);
        assert_eq!(payload["header"]["action"], "run-task");
        assert_eq!(payload["header"]["task_id"], "tid");
        assert_eq!(payload["header"]["streaming"], "duplex");
        assert_eq!(payload["payload"]["task_group"], "audio");
        assert_eq!(payload["payload"]["task"], "asr");
        assert_eq!(payload["payload"]["function"], "recognition");
        assert_eq!(payload["payload"]["model"], MODEL);
        assert_eq!(payload["payload"]["parameters"]["format"], "pcm");
        assert_eq!(payload["payload"]["parameters"]["sample_rate"], 16000);
        // input 是必填，没有上下文时也要是一个空对象
        assert!(payload["payload"]["input"].is_object());
        assert!(payload["payload"]["parameters"].get("vocabulary").is_none());
    }

    #[test]
    fn hotwords_become_weighted_instant_vocabulary() {
        let words = vec!["SayIt".to_string(), " Kiro ".to_string()];
        let payload = run_task_payload("tid", 16000, &words);
        let vocab = &payload["payload"]["parameters"]["vocabulary"];
        assert_eq!(vocab["SayIt"], HOTWORD_WEIGHT);
        assert_eq!(vocab["Kiro"], HOTWORD_WEIGHT);
    }

    #[test]
    fn hotwords_are_deduped_and_capped() {
        let (_, used, total) = build_vocabulary(&[
            "SayIt".to_string(),
            "sayit".to_string(),
            "".to_string(),
            "  ".to_string(),
        ])
        .unwrap();
        assert_eq!((used, total), (1, 1));

        let many: Vec<String> = (0..HOTWORD_LIMIT + 20).map(|i| format!("w{}", i)).collect();
        let (_, used, total) = build_vocabulary(&many).unwrap();
        assert_eq!(used, HOTWORD_LIMIT);
        assert_eq!(total, HOTWORD_LIMIT + 20);

        assert!(build_vocabulary(&[]).is_none());
        assert!(build_vocabulary(&["   ".to_string()]).is_none());
    }

    fn result_event(text: &str, ended: bool) -> serde_json::Value {
        serde_json::json!({
            "header": { "event": "result-generated" },
            "payload": { "output": { "sentence": { "text": text, "sentence_end": ended } } }
        })
    }

    /// 实测形状：中间结果是**当前句的全量文本**，不是增量。
    /// 按增量累加过一次就会得到「语音输入语音输入法语音输入法测试」这种叠字。
    #[test]
    fn partials_replace_rather_than_append() {
        let mut t = Transcript::default();
        t.apply(&result_event("语音输入", false));
        t.apply(&result_event("语音输入法测试", false));
        assert_eq!(t.display(), "语音输入法测试");
    }

    /// 开头那批空文本的中间结果不能产生一次「更新」，否则悬浮窗会先闪一下空字幕。
    #[test]
    fn empty_partials_are_ignored() {
        let mut t = Transcript::default();
        assert!(matches!(t.apply(&result_event("", false)), Applied::Ignored));
        assert_eq!(t.partial_events, 0);
        assert_eq!(t.display(), "");
    }

    /// 分句自带句尾标点，所以拼接不补分隔符。
    #[test]
    fn finished_sentences_are_joined_without_a_separator() {
        let mut t = Transcript::default();
        t.apply(&result_event("The quick brown fox. ", true));
        t.apply(&result_event("Second one", false));
        assert_eq!(t.display(), "The quick brown fox. Second one");
        t.apply(&result_event("Second one here.", true));
        assert_eq!(t.display(), "The quick brown fox. Second one here.");
        assert_eq!(t.partial, "");
    }

    #[test]
    fn task_finished_and_failed_are_terminal() {
        let mut t = Transcript::default();
        assert!(matches!(
            t.apply(&serde_json::json!({ "header": { "event": "task-finished" } })),
            Applied::Finished
        ));
        assert!(t.finished);

        let mut t = Transcript::default();
        let failed = serde_json::json!({
            "header": { "event": "task-failed", "error_code": "InvalidParameter", "error_message": "bad model" }
        });
        assert!(matches!(t.apply(&failed), Applied::Failed));
        assert!(t.finished);
        assert_eq!(
            t.error.as_deref(),
            Some("InvalidParameter: bad model")
        );
    }

    /// task-started 之类的事件不该被当成文本更新。
    #[test]
    fn unrelated_events_are_ignored() {
        let mut t = Transcript::default();
        assert!(matches!(
            t.apply(&serde_json::json!({ "header": { "event": "task-started" } })),
            Applied::Ignored
        ));
        assert!(!t.finished);
        assert_eq!(t.display(), "");
    }
}
