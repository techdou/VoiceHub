// Google Gemini 实时转写 — gemini-3.5-transcribe-live
//
// 走 Gemini Live API 的 WebSocket（BidiGenerateContent）。Live API 本体是「语音对话」，
// 但这个模型是**转写专用**的：只出文字、不说话。
//
// 协议：
//   1. 连 wss://…/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent?key=…
//   2. 首条消息必须是 `setup`（模型 + 只要文本 + 打开输入转写）
//   3. 等 `setupComplete`
//   4. `realtimeInput.audio` 送 base64 PCM，mimeType 里带采样率
//   5. 读 `serverContent.inputTranscription.text`（增量）
//   6. 收尾发 `realtimeInput.audioStreamEnd`
//
// ⚠️ 与 asr_openai_realtime.rs 完全不是一套事件名，别互相套用。
//
// ── MEASURED 2026-09-16（`dev-scripts/probe_new_asr_providers.py --target gemini-live`，
//    打的是真实接口，AI Studio 项目**没有绑定结算信息**，免费层可用）──
// 实测事件序列（3.05s 中文音频）：
//   {"setupComplete": {}}
//   {"serverContent": {}, "voiceActivity": {"type":"ACTIVITY_START","audioOffset":"0.840s"}}
//   {"serverContent": {"interimInputTranscription": {"text": "语音输入"}}}
//   {"serverContent": {"interimInputTranscription": {"text": "语音 输入法"}}}
//   {"serverContent": {"interimInputTranscription": {"text": "语音输入法测试成"}}}
//   {"serverContent": {"interimInputTranscription": {"text": "语音输入法测试成 功。"}}}
//   {"serverContent": {"inputTranscription": {"text": "语音输入法测试成功。"}}}
//   {"serverContent": {"generationComplete": true}}
//
// 由此定下来的四件事（前两条是照文档写会踩的坑）：
//   A. **实时字幕的文本在 `interimInputTranscription`**，`inputTranscription` 是
//      一轮结束时的定稿、整段只来一条。只读后者的话「实时字幕」这个功能等于不存在 ——
//      不报错，字幕就是永远不出现。
//   B. **两个字段的 text 都是全量**，不是增量。interim 要覆盖、定稿按轮次追加。
//      当成增量追加会得到「语音输入语音 输入法语音输入法测试成…」这种叠加串。
//      注意中间态带临时空格（"语音 输入法"），定稿里没有 —— 所以 interim 只能上屏，
//      最终结果必须用定稿那条。
//   C. `realtimeInput.audio` 与 `realtimeInput.mediaChunks` **两种形状都能用**，
//      转写结果完全一致。保留前者（当前文档形态）。
//   D. 结束信号是 **`generationComplete`**（不是 turnComplete）。`setupComplete`
//      用的是 camelCase。`voiceActivity` 是 VAD 事件，忽略即可。
//
// ── 密钥为什么放在 URL 的 query 里 ──
// 不情愿，但 Live API 只接受 `?key=` 或短期 ephemeral token；它没有
// Authorization 头的路子（asr_gemini.rs 那条 HTTP 路径用的是 x-goog-api-key 头）。
// 所以**这个 URL 绝不能进日志**：下面 diag::log 里只记 host 与 model，不记完整 URL。

use super::{diag, types::AsrProviderConfig};
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite;
use tungstenite::client::IntoClientRequest;
use tungstenite::http::header::USER_AGENT;
use tungstenite::http::HeaderValue;

const WS_HOST: &str = "generativelanguage.googleapis.com";
const WS_PATH: &str =
    "/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent";
const SCOPE: &str = "gemini/live";
const DEFAULT_MODEL: &str = "gemini-3.5-transcribe-live";
const SAMPLE_RATE: u32 = 16000;

type WsStream = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;
type WsSink = SplitSink<WsStream, tungstenite::Message>;

// ─────────────────────────── 协议编解码 ───────────────────────────

fn resolve_model(config: &AsrProviderConfig) -> String {
    config
        .extra
        .get("model")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_MODEL)
        .to_string()
}

/// setup 里的模型名要带 `models/` 前缀（Live API 用的是资源名，不是裸模型 ID）。
/// 用户填了带前缀的也不重复加。
fn qualified_model(model: &str) -> String {
    if model.starts_with("models/") {
        model.to_string()
    } else {
        format!("models/{}", model)
    }
}

fn ws_url(api_key: &str) -> String {
    format!(
        "wss://{}{}?key={}",
        WS_HOST,
        WS_PATH,
        urlencoding_minimal(api_key)
    )
}

/// 只转义 API key 里可能出现的、会破坏 query 的字符。
///
/// 不引入一个 URL 编码依赖：Google 的 key 是 `[A-Za-z0-9_-]` 这一类，本来无需转义。
/// 这里只兜住「用户粘进来时带了空格或换行」的情况 —— 那种情况下不转义会让
/// 建连报一个和密钥毫无关系的 URL 解析错误，很难往回定位。
fn urlencoding_minimal(raw: &str) -> String {
    raw.trim()
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
                vec![c]
            } else {
                format!("%{:02X}", c as u32).chars().collect()
            }
        })
        .collect()
}

/// 首条 setup 消息。
///
/// `responseModalities: ["TEXT"]` 是必要的：Live API 默认会**合成语音回话**，
/// 那会白烧 token、也会让 serverContent 里塞满我们不要的音频数据。
fn setup_payload(model: &str) -> serde_json::Value {
    serde_json::json!({
        "setup": {
            "model": qualified_model(model),
            "generationConfig": { "responseModalities": ["TEXT"] },
            // 打开「把输入音频转成文字」。转写专用模型可能默认就开着，
            // 显式给上不会有坏处（见文件头 C 条）。
            "inputAudioTranscription": {}
        }
    })
}

/// 音频消息。
///
/// mimeType 里必须带采样率（`audio/pcm;rate=16000`）—— 这是 Live API 声明采样率的
/// 唯一途径，没有单独的 sampleRate 字段。写错的后果是识别出一堆乱码而不是报错。
///
/// 若实测发现服务端只认旧形态，把 `"audio": {...}` 换成
/// `"mediaChunks": [{...}]`（数组），其余不变。
fn audio_payload(pcm_b64: &str) -> serde_json::Value {
    serde_json::json!({
        "realtimeInput": {
            "audio": {
                "mimeType": format!("audio/pcm;rate={}", SAMPLE_RATE),
                "data": pcm_b64
            }
        }
    })
}

fn audio_end_payload() -> serde_json::Value {
    serde_json::json!({ "realtimeInput": { "audioStreamEnd": true } })
}

/// **进行中**的转写（`interimInputTranscription`）。这条才是实时字幕的来源。
///
/// 实测：说话期间每隔几百毫秒来一条，text 是**当前的全量文本**（不是增量），
/// 所以要覆盖而不是追加。中间态还会带临时空格（"语音 输入法"），最终定稿里没有 ——
/// 所以它只适合上屏，不能拿来当最终结果。
fn interim_transcription(ev: &serde_json::Value) -> Option<&str> {
    ev.pointer("/serverContent/interimInputTranscription/text")
        .or_else(|| ev.pointer("/serverContent/interim_input_transcription/text"))
        .and_then(|t| t.as_str())
}

/// **定稿**的转写（`inputTranscription`）。一轮结束时来一条，text 也是全量。
///
/// 同时认 camelCase 与 snake_case：Live API 的 JSON 是 camelCase，
/// 但 proto 名是 snake_case，两种写法在不同网关/版本里都出现过。
fn final_transcription(ev: &serde_json::Value) -> Option<&str> {
    ev.pointer("/serverContent/inputTranscription/text")
        .or_else(|| ev.pointer("/serverContent/input_transcription/text"))
        .and_then(|t| t.as_str())
}

fn is_setup_complete(ev: &serde_json::Value) -> bool {
    ev.get("setupComplete").is_some() || ev.get("setup_complete").is_some()
}

/// 这一轮结束了。两个信号都认：只认一个的话，缺的那种情况会一直等到超时。
fn is_turn_done(ev: &serde_json::Value) -> bool {
    let truthy = |v: Option<&serde_json::Value>| v.map(|x| x.as_bool() != Some(false)).unwrap_or(false);
    truthy(ev.pointer("/serverContent/turnComplete"))
        || truthy(ev.pointer("/serverContent/turn_complete"))
        || truthy(ev.pointer("/serverContent/generationComplete"))
        || truthy(ev.pointer("/serverContent/generation_complete"))
}

/// 服务端错误。Live API 把它放在顶层 `error`，也可能是 gRPC 风格的 `{code,message}`。
fn error_message(ev: &serde_json::Value) -> Option<String> {
    let err = ev.get("error")?;
    let msg = err
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or("Unknown error");
    let code = err
        .get("code")
        .map(|c| format!(" code={}", c))
        .unwrap_or_default();
    Some(format!("{}{}", msg, code))
}

// ─────────────────────────── 累积状态 ───────────────────────────

/// 一次会话累积到的文本。
///
/// committed = 已定稿的轮次（inputTranscription），partial = 进行中的那一轮
/// （interimInputTranscription）。两个字段的 text 都是**全量**，所以 partial 是
/// 覆盖、committed 是按轮次追加。
///
/// 一次录音通常只有一轮，但服务端会按语音活动自己分轮（实测有 voiceActivity 事件），
/// 所以按多轮处理 —— 只认一轮的话，说话中间停顿一下就会丢掉后半段。
#[derive(Default)]
struct Transcript {
    committed: String,
    partial: String,
    finished: bool,
    error: Option<String>,
    interim_events: usize,
    final_events: usize,
}

enum Applied {
    Ignored,
    Updated,
    Finished,
    Failed,
}

impl Transcript {
    fn display(&self) -> String {
        format!("{}{}", self.committed, self.partial)
    }

    fn apply(&mut self, ev: &serde_json::Value) -> Applied {
        if let Some(err) = error_message(ev) {
            self.error = Some(err);
            self.finished = true;
            return Applied::Failed;
        }
        // 定稿先判：它和 interim 不会同时出现，但定稿更要紧，不该被漏掉
        if let Some(text) = final_transcription(ev) {
            self.partial.clear();
            if text.is_empty() {
                return Applied::Ignored;
            }
            self.committed.push_str(text);
            self.final_events += 1;
            // 定稿也要上屏一次：它会把中间态里的临时空格修掉，
            // 不 emit 的话悬浮窗会停在"语音输入法测试成 功。"那版
            return Applied::Updated;
        }
        if let Some(text) = interim_transcription(ev) {
            if text.is_empty() {
                return Applied::Ignored;
            }
            // **全量覆盖**，不是追加。实测每条 interim 都带着前面所有字
            // （"语音输入" → "语音 输入法" → "语音输入法测试成"），
            // 追加会得到一串反复叠加的乱码。
            self.partial.clear();
            self.partial.push_str(text);
            self.interim_events += 1;
            return Applied::Updated;
        }
        if is_turn_done(ev) {
            self.finished = true;
            return Applied::Finished;
        }
        Applied::Ignored
    }
}

// ─────────────────────────── 建连 ───────────────────────────

async fn open_session(config: &AsrProviderConfig) -> Result<(WsStream, String), String> {
    if config.api_key.trim().is_empty() {
        return Err(diag::fail_code(
            SCOPE,
            "credentials",
            "provider_bad_key",
            "Gemini live transcription is missing the API Key; complete it in Settings".to_string(),
        ));
    }

    let model = resolve_model(config);
    let url = ws_url(&config.api_key);
    let mut request = url.as_str().into_client_request().map_err(|e| {
        // 刻意不把 e 里可能带的完整 URL 拼进消息：URL 里有密钥。
        let _ = e;
        diag::fail(
            SCOPE,
            "build_request",
            "Failed to build the request; check the API Key for stray characters".to_string(),
        )
    })?;
    request.headers_mut().insert(
        USER_AGENT,
        HeaderValue::from_static(concat!("SayIt/", env!("CARGO_PKG_VERSION"))),
    );

    let (mut ws, response) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|e| {
            diag::fail(
                SCOPE,
                "connect",
                format!("WebSocket connection failed: {}", e),
            )
        })?;
    // 只记 host 与 model —— URL 里带密钥，绝不整条落盘。
    diag::log(
        SCOPE,
        "connected",
        &format!(
            "status={} host={} model={} rate={}",
            response.status(),
            WS_HOST,
            model,
            SAMPLE_RATE
        ),
    );

    ws.send(tungstenite::Message::Text(
        serde_json::to_string(&setup_payload(&model))
            .unwrap()
            .into(),
    ))
    .await
    .map_err(|e| diag::fail(SCOPE, "send_setup", format!("Failed to send setup: {}", e)))?;

    // 必须等到 setupComplete：密钥无效、模型没开通、setup 字段不认时服务端会回
    // error 或直接关连接，都要在这里变成 Err，让上层回落到一次性 HTTP 路径。
    let ready = tokio::time::timeout(Duration::from_secs(15), async {
        while let Some(msg) = ws.next().await {
            let msg = msg.map_err(|e| {
                diag::fail(
                    SCOPE,
                    "recv_ack",
                    format!("Failed to receive acknowledgement: {}", e),
                )
            })?;
            // Live API 的服务端消息可能是文本也可能是二进制里的 JSON，两种都试着解
            let text = match msg {
                tungstenite::Message::Text(t) => t.to_string(),
                tungstenite::Message::Binary(b) => match String::from_utf8(b.to_vec()) {
                    Ok(s) => s,
                    Err(_) => continue,
                },
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
                _ => continue,
            };
            let Ok(ev) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            if is_setup_complete(&ev) {
                return Ok(true);
            }
            if let Some(err) = error_message(&ev) {
                return Err(diag::fail(
                    SCOPE,
                    "server_error",
                    format!("Server rejected the session: {}", err),
                ));
            }
        }
        Ok(false)
    })
    .await
    .map_err(|_| {
        diag::fail(
            SCOPE,
            "setup_timeout",
            "Timed out waiting for the live session to start".to_string(),
        )
    })??;

    if !ready {
        return Err(diag::fail(
            SCOPE,
            "closed_before_ready",
            "WebSocket closed before setup completed".to_string(),
        ));
    }
    diag::log(SCOPE, "session_ready", &format!("model={}", model));
    Ok((ws, model))
}

// ─────────────────────────── 流式会话 ───────────────────────────

static SINK: once_cell::sync::Lazy<Arc<Mutex<Option<WsSink>>>> =
    once_cell::sync::Lazy::new(|| Arc::new(Mutex::new(None)));
static READER: once_cell::sync::Lazy<Arc<Mutex<Option<JoinHandle<()>>>>> =
    once_cell::sync::Lazy::new(|| Arc::new(Mutex::new(None)));
static STATE: once_cell::sync::Lazy<Arc<Mutex<Transcript>>> =
    once_cell::sync::Lazy::new(|| Arc::new(Mutex::new(Transcript::default())));
static ACTIVE: AtomicBool = AtomicBool::new(false);

/// 打开流式会话。热词参数收下但目前不用 —— Live API 这条路没有公开的热词字段，
/// 保留参数是为了和其他流式供应商的 open 签名一致（前端统一传）。
#[tauri::command]
pub async fn gemini_live_open(
    app: AppHandle,
    config: AsrProviderConfig,
    hotwords: Option<Vec<String>>,
    realtime: Option<bool>,
) -> Result<(), String> {
    let realtime = realtime.unwrap_or(false);
    cleanup().await;

    let hotword_count = hotwords.as_ref().map(|h| h.len()).unwrap_or(0);
    let (ws, model) = open_session(&config).await?;

    let (sink, stream) = ws.split();
    *STATE.lock().await = Transcript::default();
    *SINK.lock().await = Some(sink);
    let handle = tokio::spawn(run_reader(stream, app, STATE.clone(), realtime));
    *READER.lock().await = Some(handle);
    ACTIVE.store(true, Ordering::SeqCst);
    diag::log(
        SCOPE,
        "stream_open",
        &format!(
            "realtime={} model={} hotwords_ignored={}",
            realtime, model, hotword_count
        ),
    );
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
            Ok(tungstenite::Message::Text(t)) => t.to_string(),
            Ok(tungstenite::Message::Binary(b)) => match String::from_utf8(b.to_vec()) {
                Ok(s) => s,
                Err(_) => continue,
            },
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
                        serde_json::json!({ "text": display, "provider": "gemini_live" }),
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
    // 协议没跑完就断了必须留痕，否则 finish 只能返回空串，而空串会被显示成
    // 「未检测到有效声音」，把用户引去查麦克风（见 pitfalls 第 15 条）。
    if !ended_cleanly && s.error.is_none() {
        s.error = Some(format!(
            "connection closed before the turn completed: {}",
            diag::truncate(&close_reason, 200)
        ));
    }
    diag::log(
        SCOPE,
        "reader_stopped",
        &format!(
            "emits={} interim={} final={} clean={}",
            emitted, s.interim_events, s.final_events, ended_cleanly
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
}

#[tauri::command]
pub async fn gemini_live_send(pcm_b64: String) -> Result<(), String> {
    if !ACTIVE.load(Ordering::SeqCst) {
        return Err(diag::fail(
            SCOPE,
            "send_without_session",
            "Session is not open".to_string(),
        ));
    }
    // 音频本来就是 base64 过来的，这个协议也要 base64 塞 JSON，所以不解码。
    let payload = serde_json::to_string(&audio_payload(&pcm_b64)).unwrap();
    let mut sink = SINK.lock().await;
    let s = sink.as_mut().ok_or_else(|| {
        diag::fail(
            SCOPE,
            "send_without_session",
            "Session is not open".to_string(),
        )
    })?;
    s.send(tungstenite::Message::Text(payload.into()))
        .await
        .map_err(|e| diag::fail(SCOPE, "send_audio", format!("Failed to send audio: {}", e)))
}

#[tauri::command]
pub async fn gemini_live_finish() -> Result<String, String> {
    if !ACTIVE.load(Ordering::SeqCst) {
        return Err(diag::fail(
            SCOPE,
            "finish_without_session",
            "Session is not open".to_string(),
        ));
    }
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
                serde_json::to_string(&audio_end_payload()).unwrap().into(),
            ))
            .await
        {
            diag::log(SCOPE, "audio_end_send_err_ignored", &e.to_string());
        }
    }

    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        {
            let st = STATE.lock().await;
            if st.finished {
                let err = st.error.clone();
                let text = st.display().trim().to_string();
                let chunks = st.final_events;
                drop(st);
                cleanup().await;
                // 已经识别出文字就交回去，哪怕收尾时断了 —— 丢掉它等于让用户白说一遍。
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
                        format!("Transcription failed: {}", err),
                    ));
                }
                diag::empty_result(
                    SCOPE,
                    &format!("Session finished with no transcript chunks={}", chunks),
                );
                return Ok(String::new());
            }
        }
        if Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }

    // 超时也把已拿到的文本交回去。见文件头 D 条：turnComplete 不来时就靠这里兜底。
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
pub async fn gemini_live_close() -> Result<(), String> {
    cleanup().await;
    Ok(())
}

// ─────────────────────────── 连通性测试 ───────────────────────────

pub async fn test_connection(config: &AsrProviderConfig) -> super::types::TestResult {
    let start = Instant::now();
    let model = resolve_model(config);
    let result = async {
        let (mut ws, _) = open_session(config).await?;
        let _ = ws.close(None).await;
        Ok::<(), String>(())
    }
    .await;

    let elapsed_ms = start.elapsed().as_millis() as u64;
    match result {
        Ok(()) => super::types::TestResult {
            ok: true,
            message: format!("Connection successful ({}ms)", elapsed_ms),
            elapsed_ms,
            detail: format!("model: {}", model),
        },
        Err(message) => super::types::TestResult {
            ok: false,
            message,
            elapsed_ms,
            detail: String::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(extra: serde_json::Value) -> AsrProviderConfig {
        AsrProviderConfig {
            provider: "gemini_live_transcribe".to_string(),
            api_key: "AIza-test".to_string(),
            app_id: String::new(),
            extra,
        }
    }

    #[test]
    fn model_falls_back_to_the_live_default() {
        assert_eq!(resolve_model(&config(serde_json::json!({}))), DEFAULT_MODEL);
        assert_eq!(
            resolve_model(&config(serde_json::json!({ "model": " " }))),
            DEFAULT_MODEL
        );
    }

    /// setup 要的是资源名（models/xxx），给裸模型 ID 会被拒；已经带前缀的不能再加一层。
    #[test]
    fn model_is_qualified_exactly_once() {
        assert_eq!(
            qualified_model("gemini-3.5-transcribe-live"),
            "models/gemini-3.5-transcribe-live"
        );
        assert_eq!(
            qualified_model("models/gemini-3.5-transcribe-live"),
            "models/gemini-3.5-transcribe-live"
        );
    }

    /// responseModalities 必须限定成 TEXT —— 不限定的话 Live API 会合成语音回话，
    /// 白烧 token 还把 serverContent 塞满我们不要的音频。
    #[test]
    fn setup_asks_for_text_only_and_input_transcription() {
        let payload = setup_payload(DEFAULT_MODEL);
        assert_eq!(payload["setup"]["model"], "models/gemini-3.5-transcribe-live");
        assert_eq!(
            payload["setup"]["generationConfig"]["responseModalities"][0],
            "TEXT"
        );
        assert!(payload["setup"]["inputAudioTranscription"].is_object());
    }

    /// 采样率只能通过 mimeType 声明，没有单独字段。写错不会报错，只会识别出乱码。
    #[test]
    fn audio_mime_type_carries_the_sample_rate() {
        let payload = audio_payload("QUJD");
        assert_eq!(
            payload["realtimeInput"]["audio"]["mimeType"],
            "audio/pcm;rate=16000"
        );
        assert_eq!(payload["realtimeInput"]["audio"]["data"], "QUJD");
        assert_eq!(audio_end_payload()["realtimeInput"]["audioStreamEnd"], true);
    }

    /// 密钥里混进空格/换行时（粘贴常见）不转义会得到一个和密钥毫无关系的 URL 解析错误。
    /// 同时确认 URL 拼装本身没写错。
    #[test]
    fn ws_url_strips_whitespace_from_the_key() {
        let url = ws_url(" AIza_abc-123 \n");
        assert!(url.starts_with("wss://generativelanguage.googleapis.com/ws/"));
        assert!(url.ends_with("?key=AIza_abc-123"));
        assert!(!url.contains(' '));
        assert!(!url.contains('\n'));
    }

    #[test]
    fn setup_complete_accepts_both_spellings() {
        assert!(is_setup_complete(&serde_json::json!({ "setupComplete": {} })));
        assert!(is_setup_complete(&serde_json::json!({ "setup_complete": {} })));
        assert!(!is_setup_complete(&serde_json::json!({ "serverContent": {} })));
    }

    /// 两种命名、两种结束信号都要认：只认一个的话，缺的那种会一直等到超时才收尾。
    #[test]
    fn transcript_reads_both_namings_and_turn_signals() {
        let camel = serde_json::json!({
            "serverContent": { "inputTranscription": { "text": "你好" } }
        });
        assert_eq!(final_transcription(&camel), Some("你好"));
        let snake = serde_json::json!({
            "serverContent": { "input_transcription": { "text": "hi" } }
        });
        assert_eq!(final_transcription(&snake), Some("hi"));
        let interim = serde_json::json!({
            "serverContent": { "interimInputTranscription": { "text": "进行中" } }
        });
        assert_eq!(interim_transcription(&interim), Some("进行中"));
        // 两个字段必须分得开：interim 不能被当成定稿，否则中间态那些临时空格会进最终文本
        assert_eq!(final_transcription(&interim), None);
        assert_eq!(interim_transcription(&camel), None);

        // 实测来的是 generationComplete；turnComplete 也保留着以防换法
        assert!(is_turn_done(&serde_json::json!({
            "serverContent": { "generationComplete": true }
        })));
        assert!(is_turn_done(&serde_json::json!({
            "serverContent": { "turnComplete": true }
        })));
        assert!(!is_turn_done(&serde_json::json!({
            "serverContent": { "turnComplete": false }
        })));
    }

    /// 用**实测的原样事件序列**跑一遍（2026-09-16 那次探测的完整回放）。
    ///
    /// 这条同时钉住两件照文档写会做错的事：interim 是全量（要覆盖，不是追加），
    /// 以及定稿要替掉中间态 —— 注意实测的中间态里有临时空格"语音输入法测试成 功。"，
    /// 最终必须是没有空格的那版。
    #[test]
    fn replays_the_measured_event_stream() {
        let mut t = Transcript::default();
        let interims = ["语音输入", "语音 输入法", "语音输入法测试成", "语音输入法测试成 功。"];
        for text in interims {
            assert!(matches!(
                t.apply(&serde_json::json!({
                    "serverContent": { "interimInputTranscription": { "text": text } }
                })),
                Applied::Updated
            ));
            // 每一步都只显示当前那一条，绝不叠加
            assert_eq!(t.display(), text);
        }
        assert_eq!(t.interim_events, 4);

        assert!(matches!(
            t.apply(&serde_json::json!({
                "serverContent": { "inputTranscription": { "text": "语音输入法测试成功。" } }
            })),
            Applied::Updated
        ));
        assert_eq!(t.display(), "语音输入法测试成功。");
        assert_eq!(t.final_events, 1);

        assert!(matches!(
            t.apply(&serde_json::json!({ "serverContent": { "generationComplete": true } })),
            Applied::Finished
        ));
        assert!(t.finished);
    }

    /// 服务端按语音活动自己分轮时，定稿要按轮次累加 —— 只留最后一轮会丢掉前半段。
    #[test]
    fn multiple_final_turns_accumulate() {
        let mut t = Transcript::default();
        t.apply(&serde_json::json!({
            "serverContent": { "inputTranscription": { "text": "第一句。" } }
        }));
        t.apply(&serde_json::json!({
            "serverContent": { "interimInputTranscription": { "text": "第二" } }
        }));
        assert_eq!(t.display(), "第一句。第二");
        t.apply(&serde_json::json!({
            "serverContent": { "inputTranscription": { "text": "第二句。" } }
        }));
        assert_eq!(t.display(), "第一句。第二句。");
    }

    #[test]
    fn empty_chunks_and_unknown_events_are_ignored() {
        let mut t = Transcript::default();
        assert!(matches!(
            t.apply(&serde_json::json!({
                "serverContent": { "interimInputTranscription": { "text": "" } }
            })),
            Applied::Ignored
        ));
        assert!(matches!(
            t.apply(&serde_json::json!({ "usageMetadata": { "totalTokenCount": 3 } })),
            Applied::Ignored
        ));
        // 实测会来的 VAD 事件，必须当无关事件放过
        assert!(matches!(
            t.apply(&serde_json::json!({
                "serverContent": {},
                "voiceActivity": { "type": "ACTIVITY_START", "audioOffset": "0.840s" }
            })),
            Applied::Ignored
        ));
        assert_eq!(t.interim_events, 0);
        assert_eq!(t.final_events, 0);
    }

    #[test]
    fn server_error_is_captured_with_code() {
        let mut t = Transcript::default();
        assert!(matches!(
            t.apply(&serde_json::json!({
                "error": { "code": 429, "message": "RESOURCE_EXHAUSTED" }
            })),
            Applied::Failed
        ));
        let err = t.error.unwrap();
        assert!(err.contains("RESOURCE_EXHAUSTED"));
        assert!(err.contains("429"));
    }
}
