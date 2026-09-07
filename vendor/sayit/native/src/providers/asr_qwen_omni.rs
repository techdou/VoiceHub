// Qwen-Omni-Realtime ASR+AI — 通过 WebSocket 实时接口
// 使用 Manual 模式：发送音频 → commit → create_response → 接收文本
// 同时充当 ASR 和 AI，输出模态设为仅文本

use super::diag;
use super::types::{AsrProviderConfig, AsrResult, TestResult};
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use std::time::Instant;
use tokio_tungstenite::tungstenite;

/// 默认模型
const DEFAULT_MODEL: &str = "qwen3-omni-flash-realtime";
const SCOPE: &str = "qwen/omni";

fn ws_url(model: &str) -> String {
    format!(
        "wss://dashscope.aliyuncs.com/api-ws/v1/realtime?model={}",
        model
    )
}

#[allow(dead_code)]
fn pcm_to_wav(pcm: &[u8], sr: u32) -> Vec<u8> {
    let ds = pcm.len() as u32;
    let mut w = Vec::with_capacity(44 + pcm.len());
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + ds).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // mono
    w.extend_from_slice(&sr.to_le_bytes());
    w.extend_from_slice(&(sr * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&ds.to_le_bytes());
    w.extend_from_slice(pcm);
    w
}

/// 获取模型 ID（从 extra 字段或使用默认值）
fn get_model(config: &AsrProviderConfig) -> String {
    config
        .extra
        .get("model")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_MODEL)
        .to_string()
}

/// 获取 system prompt（从 extra 字段）
fn get_instructions(config: &AsrProviderConfig) -> String {
    config
        .extra
        .get("instructions")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("你是一个语音转文字助手。请将用户的语音内容准确转写为文字，保持原意，适当添加标点符号，不要添加任何额外的解释或评论。")
        .to_string()
}

pub async fn transcribe(
    audio_pcm_b64: &str,
    _sample_rate: u32,
    config: &AsrProviderConfig,
    hotwords: &[String],
) -> Result<AsrResult, String> {
    let pcm = base64::engine::general_purpose::STANDARD
        .decode(audio_pcm_b64)
        .map_err(|e| diag::fail(SCOPE, "decode_b64", format!("Failed to decode base64 audio: {}", e)))?;

    if pcm.is_empty() {
        diag::empty_result(SCOPE, "Input audio was empty; provider request was skipped");
        return Ok(AsrResult {
            text: String::new(),
            elapsed_ms: 0,
        });
    }

    let model = get_model(config);
    let mut instructions = get_instructions(config);
    // 热词上下文偏置：追加到 system instructions
    if let Some(ctx) = super::asr_qwen::build_hotword_context_text(hotwords) {
        instructions.push_str("\n\n请特别注意以下专业术语/词汇的识别：");
        instructions.push_str(&ctx);
    }
    let url = ws_url(&model);

    // 构建 WebSocket 请求
    let request = tungstenite::http::Request::builder()
        .uri(&url)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .header("Sec-WebSocket-Key", tungstenite::handshake::client::generate_key())
        .header("Sec-WebSocket-Version", "13")
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Host", "dashscope.aliyuncs.com")
        .body(())
        .map_err(|e| diag::fail(SCOPE, "build_request", format!("Failed to build request: {}", e)))?;

    let audio_sec = pcm.len() as f64 / 32000.0; // 16kHz / 16bit / mono
    diag::log(
        SCOPE,
        "start",
        &format!(
            "pcm_bytes={} audio_sec={:.1} model={} hotwords={}",
            pcm.len(),
            audio_sec,
            model,
            hotwords.len()
        ),
    );

    let start = Instant::now();

    let (mut ws, _) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|e| diag::fail(SCOPE, "connect", format!("WebSocket connection failed: {}", e)))?;

    // 等待 session.created
    wait_for_event(&mut ws, "session.created", SCOPE).await?;

    // 发送 session.update — 仅输出文本，禁用 VAD（Manual 模式）
    let session_update = serde_json::json!({
        "type": "session.update",
        "session": {
            "modalities": ["text"],
            "instructions": instructions,
            "input_audio_format": "pcm",
            "turn_detection": null
        }
    });
    ws.send(tungstenite::Message::Text(session_update.to_string().into()))
        .await
        .map_err(|e| {
            diag::fail(SCOPE, "send_session_update", format!("Failed to send session.update: {}", e))
        })?;

    // 等待 session.updated
    wait_for_event(&mut ws, "session.updated", SCOPE).await?;

    // 发送音频数据（PCM 16kHz 16bit mono，分块发送）
    // Qwen Omni 接受原始 PCM，不需要 WAV 头
    // 但如果采样率不是 16kHz，需要注意
    let chunk_size = 3200; // 100ms @ 16kHz 16bit mono
    let total_chunks = pcm.len().div_ceil(chunk_size);
    for (idx, chunk) in pcm.chunks(chunk_size).enumerate() {
        let audio_b64 = base64::engine::general_purpose::STANDARD.encode(chunk);
        let append_event = serde_json::json!({
            "type": "input_audio_buffer.append",
            "audio": audio_b64
        });
        ws.send(tungstenite::Message::Text(append_event.to_string().into()))
            .await
            // 带上第几包：长音频中途被切断和第一包就发不出去，成因完全不同
            .map_err(|e| {
                diag::fail(
                    SCOPE,
                    "send_audio",
                    format!("Failed to send audio chunk {}/{}: {}", idx + 1, total_chunks, e),
                )
            })?;
    }

    // 提交音频并请求响应
    let commit = serde_json::json!({ "type": "input_audio_buffer.commit" });
    ws.send(tungstenite::Message::Text(commit.to_string().into()))
        .await
        .map_err(|e| diag::fail(SCOPE, "send_commit", format!("Failed to send commit: {}", e)))?;

    let create_response = serde_json::json!({ "type": "response.create" });
    ws.send(tungstenite::Message::Text(create_response.to_string().into()))
        .await
        .map_err(|e| {
            diag::fail(SCOPE, "send_response_create", format!("Failed to send response.create: {}", e))
        })?;

    diag::log(SCOPE, "audio_sent", &format!("chunks={}", total_chunks));

    // 收集响应文本
    let mut result_text = String::new();
    let mut input_transcript = String::new();
    let timeout = tokio::time::Duration::from_secs(60);

    loop {
        let msg = match tokio::time::timeout(timeout, ws.next()).await {
            Err(_) => {
                let _ = ws.close(None).await;
                return Err(diag::fail(
                    SCOPE,
                    "recv_timeout",
                    format!(
                        "Timed out waiting for a response (60s); audio={:.1}s received_chars={}",
                        audio_sec,
                        result_text.chars().count()
                    ),
                ));
            }
            Ok(None) => break, // 连接已关闭
            Ok(Some(Err(e))) => {
                // 连接错误（包括对方关闭后仍发消息），跳出循环用已有结果
                if !result_text.is_empty() || !input_transcript.is_empty() {
                    diag::log(
                        SCOPE,
                        "recv_error_after_result",
                        &format!("A result was already received; treating as success: {}", diag::truncate(&e.to_string(), 200)),
                    );
                    break;
                }
                return Err(diag::fail(SCOPE, "recv", format!("Failed to receive message: {}", e)));
            }
            Ok(Some(Ok(m))) => m,
        };

        match msg {
            tungstenite::Message::Text(text) => {
                let event: serde_json::Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                let event_type = event
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("");

                match event_type {
                    "response.text.delta" => {
                        if let Some(delta) = event.get("delta").and_then(|d| d.as_str()) {
                            result_text.push_str(delta);
                        }
                    }
                    "response.audio_transcript.delta" => {
                        if let Some(delta) = event.get("delta").and_then(|d| d.as_str()) {
                            result_text.push_str(delta);
                        }
                    }
                    "conversation.item.input_audio_transcription.completed" => {
                        if let Some(t) = event.get("transcript").and_then(|t| t.as_str()) {
                            input_transcript = t.to_string();
                        }
                    }
                    "response.text.done" | "response.audio_transcript.done" => {
                        if let Some(t) = event.get("text").and_then(|t| t.as_str()) {
                            if !t.is_empty() {
                                result_text = t.to_string();
                            }
                        }
                        if let Some(t) = event.get("transcript").and_then(|t| t.as_str()) {
                            if !t.is_empty() {
                                result_text = t.to_string();
                            }
                        }
                    }
                    "response.done" => {
                        break;
                    }
                    "error" => {
                        let err_msg = event
                            .get("error")
                            .and_then(|e| e.get("message"))
                            .and_then(|m| m.as_str())
                            .unwrap_or("Unknown error");
                        let err_code = event
                            .get("error")
                            .and_then(|e| e.get("code"))
                            .and_then(|c| c.as_str())
                            .unwrap_or("-");
                        let _ = ws.close(None).await;
                        return Err(diag::fail(
                            SCOPE,
                            "server_error",
                            format!("Qwen Omni error [{}]: {}", err_code, err_msg),
                        ));
                    }
                    _ => {}
                }
            }
            tungstenite::Message::Close(frame) => {
                let reason = frame
                    .map(|f| format!("code={} reason={}", f.code, diag::truncate(&f.reason, 200)))
                    .unwrap_or_else(|| "No details".to_string());
                diag::log(SCOPE, "close", &reason);
                break;
            }
            _ => {}
        }
    }

    let _ = ws.close(None).await;
    let elapsed_ms = start.elapsed().as_millis() as u64;

    // 如果 AI 没有输出文本但有输入转录，使用输入转录
    let used_input_transcript = result_text.trim().is_empty() && !input_transcript.is_empty();
    let final_text = if used_input_transcript {
        input_transcript
    } else {
        result_text
    };

    if final_text.trim().is_empty() {
        diag::empty_result(
            SCOPE,
            &format!(
                "Conversation ended without any transcript audio_sec={:.1} elapsed={}ms model={} chunks={}",
                audio_sec, elapsed_ms, model, total_chunks
            ),
        );
    } else {
        diag::ok(SCOPE, elapsed_ms, final_text.chars().count());
        if used_input_transcript {
            // 模型没给回答、只给了输入转写。结果可用，但说明 instructions 可能没生效
            diag::log(SCOPE, "used_input_transcript", "Model produced no output; used the input transcript fallback");
        }
    }

    Ok(AsrResult {
        text: final_text,
        elapsed_ms,
    })
}

/// 等待指定类型的事件
async fn wait_for_event(
    ws: &mut (impl StreamExt<Item = Result<tungstenite::Message, tungstenite::Error>> + Unpin),
    expected_type: &str,
    scope: &str,
) -> Result<serde_json::Value, String> {
    let timeout = tokio::time::Duration::from_secs(10);
    let stage = format!("wait:{}", expected_type);
    loop {
        let msg = match tokio::time::timeout(timeout, ws.next()).await {
            Err(_) => {
                return Err(diag::fail(
                    scope,
                    &stage,
                    format!("Timed out waiting for {}", expected_type),
                ))
            }
            Ok(None) => {
                return Err(diag::fail(
                    scope,
                    &stage,
                    format!("Connection closed while waiting for {}", expected_type),
                ))
            }
            Ok(Some(Err(e))) => {
                return Err(diag::fail(scope, &stage, format!("Failed to receive message: {}", e)))
            }
            Ok(Some(Ok(m))) => m,
        };

        match msg {
            tungstenite::Message::Text(text) => {
                let event: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
                    diag::fail(scope, &stage, format!("Failed to parse event: {}", e))
                })?;

                let event_type = event
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("");

                if event_type == "error" {
                    let err_msg = event
                        .get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(|m| m.as_str())
                        .unwrap_or("Unknown error");
                    let err_code = event
                        .get("error")
                        .and_then(|e| e.get("code"))
                        .and_then(|c| c.as_str())
                        .unwrap_or("-");
                    return Err(diag::fail(
                        scope,
                        &stage,
                        format!("Qwen Omni error [{}]: {}", err_code, err_msg),
                    ));
                }

                if event_type == expected_type {
                    return Ok(event);
                }
            }
            tungstenite::Message::Close(frame) => {
                let reason = frame
                    .map(|f| format!("code={}, reason={}", f.code, diag::truncate(&f.reason, 200)))
                    .unwrap_or_else(|| "No details".to_string());
                return Err(diag::fail(
                    scope,
                    &stage,
                    format!("Server closed the connection while waiting for {} ({})", expected_type, reason),
                ));
            }
            _ => {}
        }
    }
}

pub async fn test_connection(config: &AsrProviderConfig) -> TestResult {
    let model = get_model(config);
    let url = ws_url(&model);

    let request = tungstenite::http::Request::builder()
        .uri(&url)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .header("Sec-WebSocket-Key", tungstenite::handshake::client::generate_key())
        .header("Sec-WebSocket-Version", "13")
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Host", "dashscope.aliyuncs.com")
        .body(())
        .unwrap();

    let start = Instant::now();

    match tokio_tungstenite::connect_async(request).await {
        Ok((mut ws, _)) => {
            // 尝试等待 session.created
            let result = wait_for_event(&mut ws, "session.created", "qwen/omni-test").await;
            let _ = ws.close(None).await;
            let elapsed_ms = start.elapsed().as_millis() as u64;
            match result {
                Ok(_) => TestResult {
                    ok: true,
                    message: format!("Connection successful, model: {} ({}ms)", model, elapsed_ms),
                    elapsed_ms,
                    detail: String::new(),
                },
                Err(e) => TestResult {
                    ok: false,
                    // wait_for_event already returns a stable sayit_error envelope. Do not
                    // bury it inside another sentence or the frontend can no longer decode it.
                    message: e,
                    elapsed_ms,
                    detail: format!("Protocol handshake failed after {}ms", elapsed_ms),
                },
            }
        }
        Err(e) => {
            let elapsed_ms = start.elapsed().as_millis() as u64;
            TestResult {
                ok: false,
                message: diag::fail("qwen/omni-test", "connect", format!("Connection failed: {}", e)),
                elapsed_ms,
                detail: String::new(),
            }
        }
    }
}
