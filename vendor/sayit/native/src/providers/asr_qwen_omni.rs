// Qwen-Omni-Realtime ASR+AI — 通过 WebSocket 实时接口
// 使用 Manual 模式：发送音频 → commit → create_response → 接收文本
// 同时充当 ASR 和 AI，输出模态设为仅文本

use super::diag;
use super::types::{AsrProviderConfig, AsrResult, TestResult};
use base64::Engine;
use futures_util::{FutureExt, SinkExt, StreamExt};
use std::time::Instant;
use tokio_tungstenite::tungstenite;

/// `extra.model` 缺失时用哪个。
///
/// 2026-09-23 从 `qwen3-omni-flash-realtime` 换过来：那个模型已被阿里公告下线并开始
/// 缩容，实测连它的实时端点直接读超时。回落值指向一个点了就坏的模型，比报错更难查
/// —— 用户看到的是「一直转圈然后什么都没有」，而日志里只有一个超时。
const DEFAULT_MODEL: &str = "qwen3.5-omni-flash-realtime";

/// `session.audio.output.voice` 发什么。
///
/// 我们只要文本，这个值永远不会被合成出来 —— 它存在的唯一原因是 3.8 会在
/// `response.create` 那一步校验音色。取 Tina 是因为官方文档说它是 3.8 的默认音色，
/// 而且实测 3.5 两个模型也接受它。详见 start_session 里 session.update 那段注释。
const OUTPUT_VOICE: &str = "Tina";

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

/// 一次 Omni 会话里累积下来的东西。
#[derive(Default)]
struct OmniTranscript {
    /// 模型的回答（response.text.* / response.audio_transcript.*）
    result_text: String,
    /// 服务端对**输入音频**的转写。模型没给回答时用它兜底
    input_transcript: String,
}

impl OmniTranscript {
    fn is_empty(&self) -> bool {
        self.result_text.trim().is_empty() && self.input_transcript.trim().is_empty()
    }

    /// 最终文本 + 是否走了「输入转写」兜底
    fn finish(self) -> (String, bool) {
        if self.result_text.trim().is_empty() && !self.input_transcript.is_empty() {
            (self.input_transcript, true)
        } else {
            (self.result_text, false)
        }
    }
}

/// 处理一条服务端事件之后该怎么走。
#[derive(Debug, PartialEq)]
enum OmniStep {
    Continue,
    /// 收到 response.done，本轮正常结束
    Completed,
    /// 服务端自己报了错
    Failed { code: String, message: String },
}

/// 收响应的过程是怎么结束的。
///
/// 这个区分是本次修复的核心：空文本到底算「用户真没说话」还是「协议没跑完」，
/// 全看是不是收到过 `response.done`。原来三条退出路径（流结束 / 收到 Close 帧 /
/// response.done）一律返回 `Ok("")`，于是**任何**协议层失败都显示成「未检测到有效声音」，
/// 把用户引去查麦克风。用户报的「长录音转录失败提示无有效声音」就是这个形态。
enum OmniEnd {
    Completed,
    /// 服务端关了连接，带上 close 帧里的说明
    Closed(String),
    /// 流直接没了，连 close 帧都没有
    StreamEnded,
}

impl OmniEnd {
    /// 一个字都没拿到时：`None` = 可以当成一次空转写（用户真没说话）；
    /// `Some(stage)` = 必须报错，stage 是写进日志和错误信封的阶段名。
    ///
    /// 判据只有一条：**收到过 response.done 吗**。没收到就说明这轮对话没跑完，
    /// 空文本不是结论、是症状。
    fn empty_failure_stage(&self) -> Option<&'static str> {
        match self {
            OmniEnd::Completed => None,
            OmniEnd::Closed(_) => Some("closed_before_response"),
            OmniEnd::StreamEnded => Some("stream_ended_before_response"),
        }
    }
}

/// 把一条服务端事件并进累积结果。
///
/// 抽成独立函数是因为它要在**两处**跑：发送音频期间的边发边读，和发完之后的主循环。
/// 顺带也就能测了 —— 这个 provider 之前一条测试都没有。
fn apply_omni_event(event: &serde_json::Value, acc: &mut OmniTranscript) -> OmniStep {
    let event_type = event.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match event_type {
        "response.text.delta" | "response.audio_transcript.delta" => {
            if let Some(delta) = event.get("delta").and_then(|d| d.as_str()) {
                acc.result_text.push_str(delta);
            }
        }
        "conversation.item.input_audio_transcription.completed" => {
            if let Some(text) = event.get("transcript").and_then(|t| t.as_str()) {
                acc.input_transcript = text.to_string();
            }
        }
        "response.text.done" | "response.audio_transcript.done" => {
            // done 带的是全量，优先用它 —— 比把 delta 拼起来更不容易缺字
            for key in ["text", "transcript"] {
                if let Some(text) = event.get(key).and_then(|t| t.as_str()) {
                    if !text.is_empty() {
                        acc.result_text = text.to_string();
                    }
                }
            }
        }
        "response.done" => return OmniStep::Completed,
        "error" => {
            let err = event.get("error");
            let field = |name: &str, fallback: &str| {
                err.and_then(|e| e.get(name))
                    .and_then(|v| v.as_str())
                    .unwrap_or(fallback)
                    .to_string()
            };
            return OmniStep::Failed {
                code: field("code", "-"),
                message: field("message", "Unknown error"),
            };
        }
        _ => {}
    }
    OmniStep::Continue
}

fn close_frame_reason(frame: Option<tungstenite::protocol::CloseFrame>) -> String {
    frame
        .map(|f| format!("code={} reason={}", f.code, diag::truncate(&f.reason, 200)))
        .unwrap_or_else(|| "no close frame details".to_string())
}

/// 收尾：把累积结果变成这次调用的返回值。
///
/// 单独抽出来有两个原因：① 早退路径和正常路径必须用同一套判断，不能各写一遍；
/// ② 这样才测得到 —— 本次修复的行为差别全在这个函数里（空结果到底算失败还是算
/// 「用户真没说话」）。
fn finish_omni(
    acc: OmniTranscript,
    end: OmniEnd,
    elapsed_ms: u64,
    context: &str,
) -> Result<AsrResult, String> {
    if acc.is_empty() {
        match end.empty_failure_stage() {
            None => diag::empty_result(
                SCOPE,
                &format!("Conversation completed without any transcript {}", context),
            ),
            Some(stage) => {
                let detail = match &end {
                    OmniEnd::Closed(reason) => format!(
                        "Server closed the connection before returning any transcript ({})",
                        reason
                    ),
                    _ => "The connection ended before returning any transcript (no close frame)"
                        .to_string(),
                };
                return Err(diag::fail(SCOPE, stage, format!("{} {}", detail, context)));
            }
        }
    }

    let (final_text, used_input_transcript) = acc.finish();
    if !final_text.trim().is_empty() {
        diag::ok(SCOPE, elapsed_ms, final_text.chars().count());
        if used_input_transcript {
            // 模型没给回答、只给了输入转写。结果可用，但说明 instructions 可能没生效
            diag::log(
                SCOPE,
                "used_input_transcript",
                "Model produced no output; used the input transcript fallback",
            );
        }
    }

    Ok(AsrResult {
        text: final_text,
        elapsed_ms,
    })
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
    //
    // ⚠️ `audio.output.voice` 看着是多余的：我们 `modalities` 只要 text，压根不要
    // 合成语音。**但它是 qwen3.8-omni-flash-realtime 能用的前提，别当冗余清掉。**
    //
    // 实测（2026-09-23，dev-scripts/probe_qwen38_omni_voice.py）：不带这个字段时，
    // 3.8 在 `response.create` 那一步回 `<400> InternalError.Algo.InvalidParameter:
    // Voice 'Chelsie' is not supported.` —— 服务端拿了一个 3.5 时代的默认音色去校验，
    // 即使这一轮不会产出音频。3.8 把默认音色换成了 Tina，并把字段挪到
    // `session.audio.output.voice`（官方文档称它优先于兼容字段 `session.voice`）。
    //
    // 为什么不按模型分支：实测 3.5 plus/flash 带上这个字段同样正常，所以**一份会话
    // 形状通吃三代**。按模型拆两套 session 会多出一条只在某一代上跑过的代码路径。
    //
    // 排查提示：错误信息只提音色、不提模型，容易被当成「账号没开通音色」或
    // 「模型不可用」。真正的判据是 `response.create` 之后立刻来一个 error 事件。
    let session_update = serde_json::json!({
        "type": "session.update",
        "session": {
            "modalities": ["text"],
            "instructions": instructions,
            "input_audio_format": "pcm",
            "turn_detection": null,
            "audio": { "output": { "voice": OUTPUT_VOICE } }
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
    let mut acc = OmniTranscript::default();
    // 发送途中就收到了 response.done。按协议不该发生（turn_detection=null 时只有
    // response.create 能触发响应），但真发生了必须停下来 —— 继续发完再 commit 会开第二轮，
    // 而主接收循环等不到新的 response.done，只能挂满 60 秒超时。
    let mut completed_while_sending = false;
    'sending: for (idx, chunk) in pcm.chunks(chunk_size).enumerate() {
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

        // ── 边发边读。**不能只发不收。** ──
        //
        // 这份实现原来是「把全部音频发完，再开始读 socket」，而仓库里另外五个流式
        // provider（豆包 / Gemini Live / OpenAI realtime / 千问 audio3x / 千问 realtime）
        // 都是 split 之后并发读的。只发不读有两个后果：
        //   ① tungstenite 只在**读到** Ping 时才排一个 Pong，一路不读就等于不回心跳；
        //   ② 服务端在发送期间报的 error 事件我们全错过。随后连接被关、下一次 send
        //      失败，日志里只剩一句「第 N 包发不出去」，服务端给的真实原因丢了。
        // 5 分钟录音是 3000 条 append、约 13MB JSON，这段时间不短。
        //
        // now_or_never 只轮询一次、拿不到就走，不拖慢发送节奏。
        loop {
            let polled = ws.next().now_or_never().flatten();
            let Some(message) = polled else { break };
            match message {
                Ok(tungstenite::Message::Text(text)) => {
                    let Ok(event) = serde_json::from_str::<serde_json::Value>(&text) else {
                        continue;
                    };
                    match apply_omni_event(&event, &mut acc) {
                        OmniStep::Continue => {}
                        OmniStep::Completed => {
                            diag::log(
                                SCOPE,
                                "completed_while_sending",
                                &format!("chunks_sent={}/{}", idx + 1, total_chunks),
                            );
                            completed_while_sending = true;
                            break 'sending;
                        }
                        OmniStep::Failed { code, message } => {
                            let _ = ws.close(None).await;
                            return Err(diag::fail(
                                SCOPE,
                                "server_error_while_sending",
                                format!(
                                    "Qwen Omni error [{}] while sending chunk {}/{} (audio={:.1}s): {}",
                                    code, idx + 1, total_chunks, audio_sec, message
                                ),
                            ));
                        }
                    }
                }
                Ok(tungstenite::Message::Close(frame)) => {
                    return Err(diag::fail(
                        SCOPE,
                        "closed_while_sending",
                        format!(
                            "Server closed the connection while sending chunk {}/{} (audio={:.1}s): {}",
                            idx + 1, total_chunks, audio_sec, close_frame_reason(frame)
                        ),
                    ));
                }
                Ok(_) => {}
                Err(e) => {
                    return Err(diag::fail(
                        SCOPE,
                        "recv_while_sending",
                        format!(
                            "Read failed while sending chunk {}/{} (audio={:.1}s): {}",
                            idx + 1, total_chunks, audio_sec, e
                        ),
                    ));
                }
            }
        }
    }

    // 发送途中这一轮就结束了：手里已经有结果，绝不能再 commit（那会开第二轮）。
    if completed_while_sending {
        let _ = ws.close(None).await;
        let elapsed_ms = start.elapsed().as_millis() as u64;
        return finish_omni(
            acc,
            OmniEnd::Completed,
            elapsed_ms,
            &format!(
                "audio_sec={:.1} elapsed={}ms model={} chunks={} ended_while_sending=true",
                audio_sec, elapsed_ms, model, total_chunks
            ),
        );
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

    // 收集响应文本。`acc` 在发送阶段就已经开始收了（见上面「边发边读」）。
    let timeout = tokio::time::Duration::from_secs(60);
    let end;

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
                        acc.result_text.chars().count()
                    ),
                ));
            }
            Ok(None) => {
                end = OmniEnd::StreamEnded;
                break;
            }
            Ok(Some(Err(e))) => {
                // 连接错误（包括对方关闭后仍发消息）。已经拿到文本就当成功收尾。
                if !acc.is_empty() {
                    diag::log(
                        SCOPE,
                        "recv_error_after_result",
                        &format!("A result was already received; treating as success: {}", diag::truncate(&e.to_string(), 200)),
                    );
                    end = OmniEnd::Completed;
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
                match apply_omni_event(&event, &mut acc) {
                    OmniStep::Continue => {}
                    OmniStep::Completed => {
                        end = OmniEnd::Completed;
                        break;
                    }
                    OmniStep::Failed { code, message } => {
                        let _ = ws.close(None).await;
                        return Err(diag::fail(
                            SCOPE,
                            "server_error",
                            format!("Qwen Omni error [{}]: {}", code, message),
                        ));
                    }
                }
            }
            tungstenite::Message::Close(frame) => {
                let reason = close_frame_reason(frame);
                diag::log(SCOPE, "close", &reason);
                end = OmniEnd::Closed(reason);
                break;
            }
            _ => {}
        }
    }

    let _ = ws.close(None).await;
    let elapsed_ms = start.elapsed().as_millis() as u64;
    let context = format!(
        "audio_sec={:.1} elapsed={}ms model={} chunks={}",
        audio_sec, elapsed_ms, model, total_chunks
    );

    // 空结果算失败还是算「真没说话」，判断全在 finish_omni 里（那里有长注释）。
    //
    // ⚠️ 服务端为什么会在长音频上掐断，**尚未用真实接口验证过**。这次改动只保证一件事：
    // 下次发生时错误信息里带着 close code / reason，而不是一句「未检测到有效声音」
    // 把用户引去查麦克风。
    finish_omni(acc, end, elapsed_ms, &context)
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

#[cfg(test)]
mod tests {
    use super::{apply_omni_event, OmniStep, OmniTranscript};
    use serde_json::json;

    fn feed(events: &[serde_json::Value]) -> (OmniTranscript, Vec<OmniStep>) {
        let mut acc = OmniTranscript::default();
        let steps = events
            .iter()
            .map(|event| apply_omni_event(event, &mut acc))
            .collect();
        (acc, steps)
    }

    #[test]
    fn text_deltas_are_appended_and_done_overrides_with_the_full_text() {
        let (acc, steps) = feed(&[
            json!({ "type": "response.text.delta", "delta": "语音" }),
            json!({ "type": "response.text.delta", "delta": "输入" }),
            json!({ "type": "response.text.done", "text": "语音输入法测试成功。" }),
            json!({ "type": "response.done" }),
        ]);
        // done 带全量，优先用它 —— 拼 delta 更容易缺字
        assert_eq!(acc.result_text, "语音输入法测试成功。");
        assert_eq!(steps.last(), Some(&OmniStep::Completed));
    }

    #[test]
    fn audio_transcript_events_feed_the_same_accumulator() {
        let (acc, _) = feed(&[
            json!({ "type": "response.audio_transcript.delta", "delta": "abc" }),
            json!({ "type": "response.audio_transcript.done", "transcript": "abcdef" }),
        ]);
        assert_eq!(acc.result_text, "abcdef");
    }

    /// 空 done 不能把已经攒好的 delta 清掉。
    #[test]
    fn an_empty_done_payload_keeps_the_accumulated_deltas() {
        let (acc, _) = feed(&[
            json!({ "type": "response.text.delta", "delta": "有内容" }),
            json!({ "type": "response.text.done", "text": "" }),
        ]);
        assert_eq!(acc.result_text, "有内容");
    }

    /// 模型没回答、只有服务端对输入音频的转写时走兜底。
    #[test]
    fn falls_back_to_the_input_transcription_when_the_model_says_nothing() {
        let (acc, _) = feed(&[json!({
            "type": "conversation.item.input_audio_transcription.completed",
            "transcript": "只有输入转写",
        })]);
        assert!(!acc.is_empty());
        let (text, used_fallback) = acc.finish();
        assert_eq!(text, "只有输入转写");
        assert!(used_fallback);
    }

    #[test]
    fn the_model_answer_wins_over_the_input_transcription() {
        let (acc, _) = feed(&[
            json!({ "type": "conversation.item.input_audio_transcription.completed", "transcript": "输入" }),
            json!({ "type": "response.text.delta", "delta": "回答" }),
        ]);
        let (text, used_fallback) = acc.finish();
        assert_eq!(text, "回答");
        assert!(!used_fallback);
    }

    #[test]
    fn server_errors_carry_code_and_message_through() {
        let (_, steps) = feed(&[json!({
            "type": "error",
            "error": { "code": "quota_exceeded", "message": "insufficient balance" },
        })]);
        assert_eq!(
            steps[0],
            OmniStep::Failed {
                code: "quota_exceeded".to_string(),
                message: "insufficient balance".to_string(),
            }
        );
    }

    /// 报错事件缺字段时也不能 panic —— 报错路径本来就是最没人测的那条。
    #[test]
    fn a_bare_error_event_still_produces_a_failure() {
        let (_, steps) = feed(&[json!({ "type": "error" })]);
        assert_eq!(
            steps[0],
            OmniStep::Failed { code: "-".to_string(), message: "Unknown error".to_string() },
        );
    }

    #[test]
    fn unknown_events_are_ignored_and_leave_the_result_empty() {
        let (acc, steps) = feed(&[
            json!({ "type": "session.updated" }),
            json!({ "type": "input_audio_buffer.committed" }),
            json!({ "type": "rate_limits.updated" }),
        ]);
        assert!(acc.is_empty());
        assert!(steps.iter().all(|s| *s == OmniStep::Continue));
    }

    /// 只有空白字符不算有内容 —— 否则「协议没跑完」会被当成一次成功的空转写，
    /// 又回到「显示未检测到有效声音」那个坑里。
    #[test]
    fn whitespace_only_text_counts_as_empty() {
        let (acc, _) = feed(&[json!({ "type": "response.text.done", "text": "   \n  " })]);
        assert!(acc.is_empty());
    }
}

#[cfg(test)]
mod empty_ending_tests {
    use super::{finish_omni, OmniEnd, OmniTranscript};

    fn transcript(result: &str) -> OmniTranscript {
        OmniTranscript {
            result_text: result.to_string(),
            input_transcript: String::new(),
        }
    }

    /// 空文本 + 服务端掐断 ⇒ 必须是 Err。
    ///
    /// 这条是本次修复的核心断言。改回 `Ok("")` 它就红 —— 而 `Ok("")` 正是用户看到
    /// 「长录音失败提示无有效声音」的原因（前端拿到空文本只会显示那一句）。
    #[test]
    fn an_aborted_conversation_with_no_text_is_an_error() {
        let err = finish_omni(
            OmniTranscript::default(),
            OmniEnd::Closed("code=1011 reason=internal".to_string()),
            1234,
            "audio_sec=180.0",
        )
        .expect_err("协议没跑完又一个字都没拿到，必须报错");
        assert!(err.contains("code=1011"), "错误信息要带上 close code，否则等于没说原因：{err}");
    }

    #[test]
    fn a_stream_that_just_ends_with_no_text_is_also_an_error() {
        assert!(finish_omni(OmniTranscript::default(), OmniEnd::StreamEnded, 1, "ctx").is_err());
    }

    /// 对话正常走完但确实没有文本 —— 这才是「用户没说话」，要返回空文本让前端提示。
    #[test]
    fn a_completed_conversation_with_no_text_stays_a_successful_empty_result() {
        let result = finish_omni(OmniTranscript::default(), OmniEnd::Completed, 1, "ctx")
            .expect("正常走完的空结果不是错误");
        assert_eq!(result.text, "");
    }

    /// 已经拿到文本时，连接怎么断的都不影响 —— 保留原来的宽容。
    #[test]
    fn text_already_received_survives_an_abrupt_close() {
        let result = finish_omni(
            transcript("已经识别出来的内容"),
            OmniEnd::Closed("code=1006".to_string()),
            1,
            "ctx",
        )
        .expect("有文本就算成功");
        assert_eq!(result.text, "已经识别出来的内容");
    }

    /// 这三条就是本次修复的全部行为差别。
    ///
    /// 用户报的是「千问 Omni 长录音转录失败，提示无有效声音」。原来三条退出路径
    /// 一律返回 `Ok("")`，前端只能显示「未检测到有效声音」，把人引去查麦克风 ——
    /// 真实原因（服务端掐断连接）在界面上完全看不到。
    ///
    /// ⚠️ 若把这三条改成同一个结果，就等于把那个 bug 装回去了。
    #[test]
    fn only_a_completed_conversation_may_report_an_empty_transcript() {
        assert_eq!(
            OmniEnd::Completed.empty_failure_stage(),
            None,
            "收到过 response.done，空文本才可以解释成「用户没说话」",
        );
        assert_eq!(
            OmniEnd::Closed("code=1011 reason=internal".to_string()).empty_failure_stage(),
            Some("closed_before_response"),
            "服务端掐断连接必须报错，而不是显示「未检测到有效声音」",
        );
        assert_eq!(
            OmniEnd::StreamEnded.empty_failure_stage(),
            Some("stream_ended_before_response"),
            "连 close 帧都没有同样是协议没跑完",
        );
    }
}
