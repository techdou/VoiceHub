// 千问实时语音识别 — qwen3-asr-flash-realtime
// WebSocket 流式：边录边发，OpenAI Realtime 风格协议
//
// 两种模式：
//  - 普通模式（realtime=false）：send 只发音频，finish 时才收集全部转录结果（默认）。
//  - 实时显示模式（realtime=true）：后台 reader 持续读取识别事件，把中间结果通过
//    `asr-partial` 事件上抛给前端做悬浮窗实时上屏；finish 返回累计的最终文本。

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
use tungstenite::http::header::{AUTHORIZATION, USER_AGENT};
use tungstenite::http::HeaderValue;

// 用稳定版模型名（不带日期快照）：带日期的快照版免费额度可能已耗尽，
// 稳定版按账号额度计费，实测可正常建连并返回实时中间结果。
const MODEL: &str = "qwen3-asr-flash-realtime";

type WsStream = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;
type WsSink = SplitSink<WsStream, tungstenite::Message>;

/// 普通模式会话：整条 WebSocket。
static SESSION: once_cell::sync::Lazy<Arc<Mutex<Option<WsStream>>>> =
    once_cell::sync::Lazy::new(|| Arc::new(Mutex::new(None)));

/// 实时模式：拆分后的发送端。
static RT_SINK: once_cell::sync::Lazy<Arc<Mutex<Option<WsSink>>>> =
    once_cell::sync::Lazy::new(|| Arc::new(Mutex::new(None)));

/// 实时模式后台 reader 任务句柄。
static RT_READER: once_cell::sync::Lazy<Arc<Mutex<Option<JoinHandle<()>>>>> =
    once_cell::sync::Lazy::new(|| Arc::new(Mutex::new(None)));

/// 实时模式共享状态。
/// committed：已确定分句拼接的最终文本；partial：当前进行中的增量缓冲。
#[derive(Default)]
struct RtState {
    committed: String,
    partial: String,
    finished: bool,
    error: Option<String>,
}

impl RtState {
    /// 实时展示用文本 = 已确定 + 进行中
    fn display(&self) -> String {
        if self.partial.is_empty() {
            self.committed.clone()
        } else if self.committed.is_empty() {
            self.partial.clone()
        } else {
            format!("{}{}", self.committed, self.partial)
        }
    }
}

static RT_STATE: once_cell::sync::Lazy<Arc<Mutex<RtState>>> =
    once_cell::sync::Lazy::new(|| Arc::new(Mutex::new(RtState::default())));

static RT_ACTIVE: AtomicBool = AtomicBool::new(false);

/// 打开千问实时 ASR 会话
///
/// hotwords：热词列表，通过 session.input_audio_transcription.corpus.text 上下文偏置（最多 10000 tokens）
/// realtime：是否开启实时显示（中间结果上抛）
#[tauri::command]
pub async fn qwen_stream_open(
    app: AppHandle,
    config: AsrProviderConfig,
    hotwords: Option<Vec<String>>,
    realtime: Option<bool>,
    workspace_id: Option<String>,
) -> Result<(), String> {
    let realtime = realtime.unwrap_or(false);

    // 关闭遗留会话
    cleanup_realtime().await;
    {
        let mut session = SESSION.lock().await;
        if let Some(mut ws) = session.take() {
            let _ = ws.close(None).await;
        }
    }

    // 实时语音识别需使用「地域 + 业务空间」专属端点（北京）；
    // 未提供 WorkspaceId 时退回通用端点（通常只用于非实时/测试）。
    let workspace = workspace_id.unwrap_or_default();
    let workspace = workspace.trim();
    let url = if workspace.is_empty() {
        format!("wss://dashscope.aliyuncs.com/api-ws/v1/realtime?model={}", MODEL)
    } else {
        format!(
            "wss://{}.cn-beijing.maas.aliyuncs.com/api-ws/v1/realtime?model={}",
            workspace, MODEL
        )
    };

    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|e| diag::fail("qwen/realtime", "build_request", format!("Failed to build request: {}", e)))?;
    request.headers_mut().insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", config.api_key))
            .map_err(|e| diag::fail_code("qwen/realtime", "authorization_header", "provider_bad_key", format!("Invalid Authorization header: {}", e)))?,
    );
    request.headers_mut().insert(
        USER_AGENT,
        HeaderValue::from_static(concat!("SayIt/", env!("CARGO_PKG_VERSION"))),
    );

    let (mut ws, response) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|e| diag::fail("qwen/realtime", "connect", format!("WebSocket connection failed: {}", e)))?;
    crate::commands::system::write_log_line(&format!(
        "[RUST] [qwen_stream] connected status={} model={} realtime={}",
        response.status(),
        MODEL,
        realtime
    ));

    // 发送 session.update 配置
    let mut input_audio_transcription = serde_json::json!({
        "language": "zh"
    });
    if let Some(words) = &hotwords {
        if let Some(ctx) = super::asr_qwen::build_hotword_context_text(words) {
            input_audio_transcription["corpus"] = serde_json::json!({ "text": ctx });
        }
    }

    // 字段对齐官方 SDK：modalities: ["text"] + 音频格式/采样率 + 语言 + VAD。
    let session_update = serde_json::json!({
        "event_id": "init_session",
        "type": "session.update",
        "session": {
            "modalities": ["text"],
            "input_audio_format": "pcm",
            "sample_rate": 16000,
            "input_audio_transcription": input_audio_transcription,
            "turn_detection": {
                "type": "server_vad",
                "threshold": 0.0,
                "silence_duration_ms": 400
            }
        }
    });

    ws.send(tungstenite::Message::Text(
        serde_json::to_string(&session_update).unwrap().into(),
    ))
    .await
    .map_err(|e| diag::fail("qwen/realtime", "send_session_update", format!("Failed to send session.update: {}", e)))?;

    // 必须等到 session.updated 才算就绪（说明配置被接受、模型可用）。
    // 若额度耗尽/无权限，服务端会在 session.created 后直接关闭连接（携带原因），
    // 此时下面循环拿不到 session.updated → 返回 Err → 上层回退到一次性识别。
    let mut session_ready = false;
    while let Some(msg) = ws.next().await {
        let msg = msg.map_err(|e| diag::fail("qwen/realtime", "recv_ack", format!("Failed to receive acknowledgement: {}", e)))?;
        match msg {
            tungstenite::Message::Text(text) => {
                if let Ok(data) = serde_json::from_str::<serde_json::Value>(&text) {
                    let event_type = data.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    if event_type == "session.updated" {
                        session_ready = true;
                        crate::commands::system::write_log_line("[RUST] [qwen_stream] session updated");
                        break;
                    }
                    if event_type == "error" {
                        let err_msg = data
                            .get("error")
                            .and_then(|e| e.get("message"))
                            .and_then(|m| m.as_str())
                            .unwrap_or("Unknown error");
                        return Err(diag::fail("qwen/realtime", "server_error", format!("Server error: {}", err_msg)));
                    }
                }
            }
            tungstenite::Message::Close(frame) => {
                let reason = frame
                    .map(|f| f.reason.to_string())
                    .unwrap_or_else(|| "No reason".to_string());
                crate::commands::system::write_log_line(&format!(
                    "[RUST] [qwen_stream] closed before ready: {}",
                    reason
                ));
                return Err(diag::fail("qwen/realtime", "closed_before_ready", format!("Server closed the session: {}", reason)));
            }
            _ => {}
        }
    }
    if !session_ready {
        return Err(diag::fail("qwen/realtime", "closed_before_ready", "WebSocket closed before the session became ready".to_string()));
    }

    if realtime {
        let (sink, stream) = ws.split();
        {
            let mut st = RT_STATE.lock().await;
            *st = RtState::default();
        }
        *RT_SINK.lock().await = Some(sink);
        let handle = tokio::spawn(run_realtime_reader(stream, app.clone(), RT_STATE.clone()));
        *RT_READER.lock().await = Some(handle);
        RT_ACTIVE.store(true, Ordering::SeqCst);
    } else {
        RT_ACTIVE.store(false, Ordering::SeqCst);
        *SESSION.lock().await = Some(ws);
    }

    Ok(())
}

/// 后台 reader：读取识别事件，累积文本并上抛中间结果。
async fn run_realtime_reader(
    mut stream: SplitStream<WsStream>,
    app: AppHandle,
    state: Arc<Mutex<RtState>>,
) {
    crate::commands::system::write_log_line("[RUST] [qwen_stream] realtime reader spawned");
    let mut emit_count = 0usize;
    while let Some(msg) = stream.next().await {
        let msg = match msg {
            Ok(m) => m,
            Err(_) => {
                let mut s = state.lock().await;
                s.finished = true;
                break;
            }
        };

        let text = match msg {
            tungstenite::Message::Text(t) => t,
            tungstenite::Message::Close(_) => {
                let mut s = state.lock().await;
                s.finished = true;
                break;
            }
            _ => continue,
        };

        let data = match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let event_type = data.get("type").and_then(|t| t.as_str()).unwrap_or("");

        match event_type {
            // Qwen 实时中间结果：text 为当前句到目前为止的识别文本，stash 为不稳定的预测尾巴。
            // 这是 qwen3-asr-flash-realtime 实际使用的增量事件（非 OpenAI 风格的 .delta）。
            "conversation.item.input_audio_transcription.text" => {
                let cur = data.get("text").and_then(|t| t.as_str()).unwrap_or("");
                let stash = data.get("stash").and_then(|t| t.as_str()).unwrap_or("");
                if !cur.is_empty() || !stash.is_empty() {
                    let display = {
                        let mut s = state.lock().await;
                        // text 是"当前句"的全量文本（非增量），直接覆盖 partial
                        s.partial = format!("{}{}", cur, stash);
                        s.display()
                    };
                    emit_count += 1;
                    if emit_count == 1 {
                        crate::commands::system::write_log_line(
                            "[RUST] [qwen_stream] emitted first asr-partial (text)",
                        );
                    }
                    let _ = app.emit(
                        "asr-partial",
                        serde_json::json!({ "text": display, "provider": "qwen" }),
                    );
                }
            }
            // 兼容 OpenAI 风格的增量事件（部分快照/模型可能使用）
            "conversation.item.input_audio_transcription.delta" => {
                if let Some(delta) = data.get("delta").and_then(|t| t.as_str()) {
                    if !delta.is_empty() {
                        let display = {
                            let mut s = state.lock().await;
                            s.partial.push_str(delta);
                            s.display()
                        };
                        let _ = app.emit(
                            "asr-partial",
                            serde_json::json!({ "text": display, "provider": "qwen" }),
                        );
                    }
                }
            }
            // 分句确定：并入 committed，清空 partial，并上抛
            "conversation.item.input_audio_transcription.completed" => {
                if let Some(transcript) = data.get("transcript").and_then(|t| t.as_str()) {
                    if !transcript.is_empty() {
                        let display = {
                            let mut s = state.lock().await;
                            if !s.committed.is_empty() {
                                s.committed.push_str("，");
                            }
                            s.committed.push_str(transcript);
                            s.partial.clear();
                            s.display()
                        };
                        let _ = app.emit(
                            "asr-partial",
                            serde_json::json!({ "text": display, "provider": "qwen" }),
                        );
                    }
                }
            }
            "session.finished" => {
                let mut s = state.lock().await;
                s.finished = true;
                break;
            }
            "error" => {
                let err_msg = data
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .unwrap_or("Unknown error")
                    .to_string();
                let mut s = state.lock().await;
                s.error = Some(err_msg);
                s.finished = true;
                break;
            }
            _ => {}
        }
    }
    crate::commands::system::write_log_line(&format!(
        "[RUST] [qwen_stream] reader stopped emits={}",
        emit_count
    ));
    let mut s = state.lock().await;
    s.finished = true;
}

async fn cleanup_realtime() {
    RT_ACTIVE.store(false, Ordering::SeqCst);
    if let Some(mut sink) = RT_SINK.lock().await.take() {
        let _ = sink.close().await;
    }
    if let Some(handle) = RT_READER.lock().await.take() {
        handle.abort();
    }
    let mut st = RT_STATE.lock().await;
    *st = RtState::default();
}

/// 发送音频数据
#[tauri::command]
pub async fn qwen_stream_send(pcm_b64: String) -> Result<(), String> {
    let event = serde_json::json!({
        "event_id": format!("audio_{}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis()),
        "type": "input_audio_buffer.append",
        "audio": pcm_b64
    });
    let payload = serde_json::to_string(&event).unwrap();

    if RT_ACTIVE.load(Ordering::SeqCst) {
        let mut sink = RT_SINK.lock().await;
        let s = sink.as_mut().ok_or_else(|| diag::fail("qwen/realtime", "send_without_session", "Session is not open".to_string()))?;
        s.send(tungstenite::Message::Text(payload.into()))
            .await
            .map_err(|e| diag::fail("qwen/realtime", "send_audio", format!("Failed to send audio: {}", e)))?;
        return Ok(());
    }

    let mut session = SESSION.lock().await;
    let ws = session.as_mut().ok_or_else(|| diag::fail("qwen/realtime", "send_without_session", "Session is not open".to_string()))?;
    ws.send(tungstenite::Message::Text(payload.into()))
        .await
        .map_err(|e| diag::fail("qwen/realtime", "send_audio", format!("Failed to send audio: {}", e)))?;

    Ok(())
}

/// 结束会话并等待最终结果
#[tauri::command]
pub async fn qwen_stream_finish() -> Result<String, String> {
    let finish_event = serde_json::json!({
        "event_id": "finish",
        "type": "session.finish"
    });
    let finish_payload = serde_json::to_string(&finish_event).unwrap();

    // 实时模式：通过发送端发 finish，等待后台 reader 收尾
    if RT_ACTIVE.load(Ordering::SeqCst) {
        {
            let mut sink = RT_SINK.lock().await;
            let s = sink.as_mut().ok_or_else(|| diag::fail("qwen/realtime", "finish_without_session", "Session is not open".to_string()))?;
            s.send(tungstenite::Message::Text(finish_payload.into()))
                .await
                .map_err(|e| diag::fail("qwen/realtime", "send_finish", format!("Failed to send finish event: {}", e)))?;
        }

        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            {
                let st = RT_STATE.lock().await;
                if st.finished {
                    if let Some(err) = st.error.clone() {
                        drop(st);
                        cleanup_realtime().await;
                        return Err(diag::fail("qwen/realtime", "recognition_error", format!("Recognition error: {}", err)));
                    }
                    // display() = 已确定分句 + 尚未 completed 的当前句，避免丢掉最后一句
                    let text = st.display();
                    drop(st);
                    cleanup_realtime().await;
                    return Ok(text);
                }
            }
            if Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }

        let text = RT_STATE.lock().await.display();
        cleanup_realtime().await;
        return Ok(text);
    }

    // 普通模式：整条会话同步收结果
    let mut session = SESSION.lock().await;
    let ws = session.as_mut().ok_or_else(|| diag::fail("qwen/realtime", "finish_without_session", "Session is not open".to_string()))?;

    ws.send(tungstenite::Message::Text(finish_payload.into()))
        .await
        .map_err(|e| diag::fail("qwen/realtime", "send_finish", format!("Failed to send finish event: {}", e)))?;

    let mut final_text = String::new();
    let mut completed_count = 0usize;

    while let Some(msg) = ws.next().await {
        let msg = msg.map_err(|e| diag::fail("qwen/realtime", "recv", format!("Failed to receive result: {}", e)))?;
        if let tungstenite::Message::Text(text) = msg {
            if let Ok(data) = serde_json::from_str::<serde_json::Value>(&text) {
                let event_type = data.get("type").and_then(|t| t.as_str()).unwrap_or("");

                match event_type {
                    "conversation.item.input_audio_transcription.completed" => {
                        if let Some(transcript) = data.get("transcript").and_then(|t| t.as_str()) {
                            if !transcript.is_empty() {
                                completed_count += 1;
                                if !final_text.is_empty() {
                                    final_text.push_str("，");
                                }
                                final_text.push_str(transcript);
                            }
                        }
                    }
                    "session.finished" => {
                        break;
                    }
                    "error" => {
                        let err_msg = data
                            .get("error")
                            .and_then(|e| e.get("message"))
                            .and_then(|m| m.as_str())
                            .unwrap_or("Unknown error");
                        let _ = ws.close(None).await;
                        *session = None;
                        return Err(diag::fail("qwen/realtime", "recognition_error", format!("Recognition error: {}", err_msg)));
                    }
                    _ => {}
                }
            }
        }
    }

    crate::commands::system::write_log_line(&format!(
        "[RUST] [qwen_stream] finished completedSegments={} utf8Len={} utf16Len={}",
        completed_count,
        final_text.len(),
        final_text.encode_utf16().count()
    ));
    let _ = ws.close(None).await;
    *session = None;

    Ok(final_text)
}

/// 关闭会话
#[tauri::command]
pub async fn qwen_stream_close() -> Result<(), String> {
    if RT_ACTIVE.load(Ordering::SeqCst) {
        cleanup_realtime().await;
        return Ok(());
    }
    let mut session = SESSION.lock().await;
    if let Some(mut ws) = session.take() {
        let _ = ws.close(None).await;
    }
    Ok(())
}
