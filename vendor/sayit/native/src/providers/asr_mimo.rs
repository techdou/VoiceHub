// 小米 MiMo ASR — mimo-v2.5-asr
// OpenAI chat/completions 兼容接口，支持 Base64 data URL 音频（wav/mp3）。
// 与千问的差异：鉴权头是 `api-key`（非 Bearer）、无 app_id、body 为 OpenAI chat 结构、
// 响应为 choices[0].message.content（纯字符串）。

use super::diag;
use super::types::{AsrProviderConfig, AsrResult, TestResult};
use std::time::Instant;

const API_URL: &str = "https://api.xiaomimimo.com/v1/chat/completions";
const SCOPE: &str = "mimo/asr";

/// 将 16kHz 单声道 16-bit PCM 封装为 WAV 容器（MiMo 只接受 wav/mp3，不接受裸 PCM）。
fn pcm_to_wav(pcm: &[u8], sr: u32) -> Vec<u8> {
    let ds = pcm.len() as u32;
    let mut w = Vec::with_capacity(44 + pcm.len());
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + ds).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&sr.to_le_bytes());
    w.extend_from_slice(&(sr * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&ds.to_le_bytes());
    w.extend_from_slice(pcm);
    w
}

/// 从供应商额外配置读取识别语言（auto|zh|en），默认 auto。
fn resolve_language(config: &AsrProviderConfig) -> String {
    config
        .extra
        .get("language")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("auto")
        .to_string()
}

fn build_body(data_url: &str, language: &str) -> serde_json::Value {
    serde_json::json!({
        "model": "mimo-v2.5-asr",
        "messages": [
            {
                "role": "user",
                "content": [
                    { "type": "input_audio", "input_audio": { "data": data_url } }
                ]
            }
        ],
        "asr_options": { "language": language }
    })
}

/// 解析 OpenAI chat 响应中的转写文本：choices[0].message.content。
/// content 可能是字符串，也可能是分段数组，两种都兼容。
fn extract_text(data: &serde_json::Value) -> String {
    let content = data
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"));
    match content {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|seg| {
                seg.get("text")
                    .and_then(|t| t.as_str())
                    .or_else(|| seg.as_str())
            })
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

pub async fn transcribe(
    audio_pcm_b64: &str,
    sample_rate: u32,
    config: &AsrProviderConfig,
    _hotwords: &[String],
) -> Result<AsrResult, String> {
    let pcm = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        audio_pcm_b64,
    )
    .map_err(|e| diag::fail(SCOPE, "decode_b64", format!("Failed to decode base64 audio: {}", e)))?;

    if pcm.is_empty() {
        diag::empty_result(SCOPE, "Input audio was empty; provider request was skipped");
        return Ok(AsrResult { text: String::new(), elapsed_ms: 0 });
    }

    let wav = pcm_to_wav(&pcm, sample_rate);
    let wav_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &wav);
    let data_url = format!("data:audio/wav;base64,{}", wav_b64);
    let language = resolve_language(config);
    let body = build_body(&data_url, &language);

    let audio_sec = pcm.len() as f64 / (sample_rate.max(1) as f64 * 2.0);
    diag::log(
        SCOPE,
        "start",
        &format!(
            "pcm_bytes={} audio_sec={:.1} rate={} language={}",
            pcm.len(),
            audio_sec,
            sample_rate,
            language
        ),
    );

    let client = super::http_client::shared();
    let start = Instant::now();

    let resp = client
        .post(API_URL)
        .header("api-key", &config.api_key)
        .header("Content-Type", "application/json")
        .json(&body)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| diag::fail(SCOPE, "http_send", format!("Request failed: {}", e)))?;

    let elapsed_ms = start.elapsed().as_millis() as u64;
    let http_summary = diag::http_summary(resp.status(), resp.headers());

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(diag::fail(
            SCOPE,
            "http_status",
            // 原来是 `&body[..body.len().min(300)]`，中文报错体会切在汉字中间 panic
            format!(
                "MiMo ASR error {} [{}]: {}",
                status,
                http_summary,
                diag::truncate(&body, 300)
            ),
        ));
    }

    let body_text = resp
        .text()
        .await
        .map_err(|e| diag::fail(SCOPE, "read_body", format!("Failed to read response: {}", e)))?;
    let data: serde_json::Value = serde_json::from_str(&body_text).map_err(|e| {
        diag::fail(
            SCOPE,
            "parse_json",
            format!(
                "Failed to parse response: {} [{}] response excerpt: {}",
                e,
                http_summary,
                diag::truncate(&body_text, 200)
            ),
        )
    })?;

    let text = extract_text(&data);
    if text.is_empty() {
        diag::empty_result(
            SCOPE,
            &format!(
                "Response contained no transcript audio_sec={:.1} elapsed={}ms [{}] {}",
                audio_sec,
                elapsed_ms,
                http_summary,
                diag::describe_json(&body_text)
            ),
        );
    } else {
        diag::ok(SCOPE, elapsed_ms, text.chars().count());
    }

    Ok(AsrResult { text, elapsed_ms })
}

pub async fn test_connection(config: &AsrProviderConfig) -> TestResult {
    let silence = vec![0u8; 16000]; // 0.5s 静音
    let wav = pcm_to_wav(&silence, 16000);
    let wav_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &wav);
    let data_url = format!("data:audio/wav;base64,{}", wav_b64);
    let body = build_body(&data_url, "auto");

    let client = super::http_client::shared();
    let start = Instant::now();

    let result = client
        .post(API_URL)
        .header("api-key", &config.api_key)
        .header("Content-Type", "application/json")
        .json(&body)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await;

    let elapsed_ms = start.elapsed().as_millis() as u64;

    match result {
        Ok(resp) if resp.status().is_success() => TestResult {
            ok: true,
            message: format!("Connection successful ({}ms)", elapsed_ms),
            elapsed_ms,
            detail: String::new(),
        },
        Ok(resp) => {
            let status = resp.status();
            let summary = diag::http_summary(status, resp.headers());
            let body = resp.text().await.unwrap_or_default();
            TestResult {
                ok: false,
                message: diag::fail(
                    "mimo/asr-test",
                    "http_status",
                    format!(
                        "API error {} [{}]: {}",
                        status,
                        summary,
                        diag::truncate(&body, 100)
                    ),
                ),
                elapsed_ms,
                detail: String::new(),
            }
        }
        Err(e) => TestResult {
            ok: false,
            message: diag::fail("mimo/asr-test", "http_send", format!("Connection failed: {}", e)),
            elapsed_ms,
            detail: String::new(),
        },
    }
}
