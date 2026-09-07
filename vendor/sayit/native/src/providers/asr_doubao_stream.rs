// 豆包流式语音识别 2.0 — 使用流式输入模式（bigmodel_nostream）
// 录完后一次性发送 PCM 音频，等最终结果返回

use super::diag;
use super::doubao_auth::{self, DoubaoAuth};
use super::doubao_protocol;
use super::types::{AsrProviderConfig, AsrResult, TestResult};
use futures_util::{SinkExt, StreamExt};
use std::time::Instant;
use tokio_tungstenite::tungstenite;

const WS_URL: &str = "wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_nostream";
const SCOPE: &str = "doubao/nostream";

/// 建立 nostream 连接：按「小时版 → 并发版」依次试资源 ID。
///
/// 之前这里写死小时版，而双向流式路径早就在两者间兜底了 —— 于是只开通并发版的账号
/// 出现「实时字幕能用、普通录音识别连不上」这种自相矛盾的现象。两条路径口径统一。
async fn connect(
    auth: &DoubaoAuth<'_>,
    scope: &str,
) -> Result<(tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, &'static str), String> {
    let mut last_err = String::from("Unknown error");
    for resource_id in doubao_auth::SAUC_RESOURCE_CANDIDATES {
        let mut builder = tungstenite::http::Request::builder()
            .uri(WS_URL)
            .header("Host", "openspeech.bytedance.com")
            .header("X-Api-Resource-Id", *resource_id)
            .header("X-Api-Connect-Id", uuid::Uuid::new_v4().to_string())
            .header(
                "Sec-WebSocket-Key",
                tungstenite::handshake::client::generate_key(),
            )
            .header("Sec-WebSocket-Version", "13")
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket");
        for (name, value) in auth.headers() {
            builder = builder.header(name, value);
        }
        let request = builder
            .body(())
            .map_err(|e| diag::fail(scope, "build_request", format!("Failed to build request: {}", e)))?;

        match tokio_tungstenite::connect_async(request).await {
            Ok((ws, response)) => {
                doubao_auth::log_ws_logid(
                    &format!("{} resourceId={} auth={}", scope, resource_id, auth.mode_name()),
                    &response,
                );
                return Ok((ws, resource_id));
            }
            Err(e) => {
                last_err = e.to_string();
                // 逐个资源都记：只开通了并发版的账号会在小时版这一轮失败一次再成功，
                // 看到这条不代表最终失败，但它是判断「账号开的是哪种计费」的直接证据。
                diag::log(
                    scope,
                    "connect_attempt_failed",
                    &format!(
                        "resourceId={} auth={} err={}",
                        resource_id,
                        auth.mode_name(),
                        diag::truncate(&last_err, 200)
                    ),
                );
            }
        }
    }
    Err(diag::fail(
        scope,
        "connect",
        format!("WebSocket connection failed: {}", last_err),
    ))
}

pub async fn transcribe(
    audio_pcm_b64: &str,
    sample_rate: u32,
    config: &AsrProviderConfig,
    hotwords: &[String],
) -> Result<AsrResult, String> {
    let pcm_data = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        audio_pcm_b64,
    )
    .map_err(|e| diag::fail(SCOPE, "decode_b64", format!("Failed to decode base64 audio: {}", e)))?;

    if pcm_data.is_empty() {
        diag::empty_result(SCOPE, "Input audio was empty; provider request was skipped");
        return Ok(AsrResult { text: String::new(), elapsed_ms: 0 });
    }

    let auth = DoubaoAuth::from_config(config);
    if let Some(missing) = auth.missing_field() {
        return Err(diag::fail_code(
            SCOPE,
            "credentials",
            "provider_bad_key",
            format!("Doubao ASR is missing {}; complete it in Settings", missing),
        ));
    }
    let uid = auth.uid().to_string();

    // 进门先记「送出去的是什么」：音频多长、采样率、几个热词、哪套鉴权。
    // 长音频失败而短音频正常这类问题，第一步要确认的就是客户端确实把整段音频发了出去。
    let audio_sec = pcm_data.len() as f64 / (sample_rate.max(1) as f64 * 2.0);
    diag::log(
        SCOPE,
        "start",
        &format!(
            "pcm_bytes={} audio_sec={:.1} rate={} hotwords={} auth={}",
            pcm_data.len(),
            audio_sec,
            sample_rate,
            hotwords.len(),
            auth.mode_name()
        ),
    );

    let start = Instant::now();

    let (mut ws, resource_id) = connect(&auth, SCOPE).await?;

    // 1. 发送 full client request
    let mut request_params = serde_json::json!({
        "model_name": "bigmodel",
        "enable_itn": true,
        "enable_punc": true,
        "result_type": "full",
        "show_utterances": true
    });
    // 注入热词（如果有）：流式接口热词须放在 request.corpus.context
    if let Some(ctx) = doubao_protocol::build_hotword_context(hotwords) {
        request_params["corpus"] = serde_json::json!({ "context": ctx });
    }

    let client_request = serde_json::json!({
        "user": { "uid": uid },
        "audio": {
            "format": "pcm",
            "rate": sample_rate,
            "bits": 16,
            "channel": 1
        },
        "request": request_params
    });

    let request_frame = doubao_protocol::build_full_client_request(
        &serde_json::to_string(&client_request).unwrap(),
    );
    ws.send(tungstenite::Message::Binary(request_frame.into()))
        .await
        .map_err(|e| diag::fail(SCOPE, "send_request", format!("Failed to send request: {}", e)))?;

    // 等待服务端确认
    if let Some(msg) = ws.next().await {
        let msg =
            msg.map_err(|e| diag::fail(SCOPE, "recv_ack", format!("Failed to receive acknowledgement: {}", e)))?;
        if let tungstenite::Message::Binary(data) = msg {
            let resp = doubao_protocol::parse_server_response(&data)
                .map_err(|e| diag::fail(SCOPE, "parse_ack", e))?;
            if resp.is_error {
                // 这里的 payload 是服务端的错误说明（额度/资源/鉴权），不是识别文本
                return Err(diag::fail(
                    SCOPE,
                    "server_error_on_ack",
                    format!(
                        "Server error: {}{}",
                        resp.payload,
                        doubao_auth::explain_payload(&resp.payload)
                    ),
                ));
            }
        }
    }

    // 2. 发送音频数据（直接发 PCM，不转 WAV）
    // nostream 模式下服务端等最后一包才处理，一次性发完最快
    let audio_frame = doubao_protocol::build_audio_request(&pcm_data, true);
    ws.send(tungstenite::Message::Binary(audio_frame.into()))
        .await
        .map_err(|e| diag::fail(SCOPE, "send_audio", format!("Failed to send audio: {}", e)))?;

    // 3. 接收结果（bigmodel_async 双向流式：每包输入对应一包返回，取最终结果）
    let mut final_text = String::new();
    // 有没有收到「最后一包」标记。这个标记是区分下面两种情况的唯一依据：
    //   收到了 + 文本为空 = 服务端确实没听出内容（用户可能真的没说话）
    //   没收到 + 文本为空 = 这次调用失败了，服务端在给结果前就断开
    // 以前两种都返回 Ok(空文本)，于是失败被显示成「未检测到有效声音」，
    // 日志里一个字都没有 —— 排查一次要来回问用户好几轮。
    let mut saw_last = false;
    let mut last_payload_desc = String::from("(no result packet received)");

    while let Some(msg) = ws.next().await {
        let msg = msg.map_err(|e| diag::fail(SCOPE, "recv", format!("Failed to receive result: {}", e)))?;
        match msg {
            tungstenite::Message::Binary(data) => {
                let resp = doubao_protocol::parse_server_response(&data)
                    .map_err(|e| diag::fail(SCOPE, "parse_result", e))?;
                if resp.is_error {
                    return Err(diag::fail(
                        SCOPE,
                        "server_error",
                        format!(
                            "Recognition error: {}{}",
                            resp.payload,
                            doubao_auth::explain_payload(&resp.payload)
                        ),
                    ));
                }

                last_payload_desc = diag::describe_json(&resp.payload);

                // 解析 JSON 结果，持续更新 final_text
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&resp.payload) {
                    if let Some(text) = json.get("result").and_then(|r| r.get("text")).and_then(|t| t.as_str()) {
                        if !text.is_empty() {
                            final_text = text.to_string();
                        }
                    }
                }

                if resp.is_last {
                    saw_last = true;
                    break;
                }
            }
            tungstenite::Message::Close(frame) => {
                let reason = frame
                    .map(|f| format!("code={} reason={}", f.code, diag::truncate(&f.reason, 200)))
                    .unwrap_or_else(|| "No details".to_string());
                if !saw_last {
                    let _ = ws.close(None).await;
                    // 服务端主动断开且没给最终结果 —— 额度耗尽、资源未开通、超限都长这样。
                    // 必须报错，不能假装识别成功却没内容。
                    return Err(diag::fail(
                        SCOPE,
                        "closed_before_result",
                        format!("Doubao closed the connection before recognition completed ({})", reason),
                    ));
                }
                diag::log(SCOPE, "close", &reason);
                break;
            }
            _ => {}
        }
    }

    // 关闭连接
    let _ = ws.close(None).await;

    let elapsed_ms = start.elapsed().as_millis() as u64;

    // 流结束了却始终没见到最后一包，同样是没跑完的调用
    if !saw_last {
        return Err(diag::fail(
            SCOPE,
            "stream_ended_without_result",
            format!(
                "Doubao connection ended without a final result (received_chars={} elapsed={}ms)",
                final_text.chars().count(),
                elapsed_ms
            ),
        ));
    }

    if final_text.is_empty() {
        diag::empty_result(
            SCOPE,
            &format!(
                "Server returned a final result with an empty transcript resourceId={} audio_sec={:.1} elapsed={}ms {}",
                resource_id, audio_sec, elapsed_ms, last_payload_desc
            ),
        );
    } else {
        diag::ok(SCOPE, elapsed_ms, final_text.chars().count());
    }

    Ok(AsrResult {
        text: final_text,
        elapsed_ms,
    })
}

pub async fn test_connection(config: &AsrProviderConfig) -> TestResult {
    let auth = DoubaoAuth::from_config(config);
    if let Some(missing) = auth.missing_field() {
        return TestResult {
            ok: false,
            message: format!("{} has not been configured", missing),
            elapsed_ms: 0,
            detail: String::new(),
        };
    }

    let start = Instant::now();

    match connect(&auth, "doubao/nostream-test").await {
        Ok((mut ws, resource_id)) => {
            let _ = ws.close(None).await;
            let elapsed_ms = start.elapsed().as_millis() as u64;
            TestResult {
                ok: true,
                message: format!("Connection successful ({}ms)", elapsed_ms),
                elapsed_ms,
                // 资源 ID 决定计费方式（小时版/并发版），测通了顺手告诉用户命中的是哪个
                detail: format!("Resource: {}", resource_id),
            }
        }
        Err(e) => {
            let elapsed_ms = start.elapsed().as_millis() as u64;
            TestResult {
                ok: false,
                message: e,
                elapsed_ms,
                detail: String::new(),
            }
        }
    }
}
