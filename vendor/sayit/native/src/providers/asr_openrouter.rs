// OpenRouter 转写 — 一把 key 通往多家 STT 模型
//
// 走 OpenRouter 自己的 `POST /api/v1/audio/transcriptions`：**JSON body + base64 音频**
// （`input_audio: { data, format }`），不是 multipart。
//
// ⚠️ 别改成复用 asr_groq.rs。OpenRouter 确实**也**接受 OpenAI 风格的 multipart
// （官方说把 base_url 指过来就能用 OpenAI SDK），但那条路上 `prompt` 字段是
// **「接受后忽略」**的 —— 而 asr_groq.rs 全靠那个 prompt 给中文短句加标点
// （见它的 PUNCTUATION_PROMPT，缺了它中文转写一个标点都没有）。
// 复用等于悄悄丢掉标点，所以这里独立一份，走原生 JSON 形态。
//
// ── 关于标点这件事（这是选模型的关键，不是小事）──
// OpenRouter 的两种形态都没有 prompt 通道（JSON 的参数表里也没有），所以
// **Whisper 系模型在这里没法做标点引导**。默认模型因此选 `openai/gpt-transcribe`：
// 新一代 token 计费的模型自带标点，不依赖「拿带标点的句子做示范」那套。
// 用户要选 whisper-1 / whisper-large-v3 也可以，只是中文短句可能没有句尾标点。
// 半角转全角在前端 textPostProcess.ts 的 normalizeChinesePunctuation 里做，
// 那解决的是宽度不是有无。
//
// ── 模型 slug 必须带厂商前缀 ──
// `openai/whisper-1` 而不是 `whisper-1`。写错会得到一个 404 "model not found"。
// 可用清单在前端目录里（asrProviderCatalog.ts 的 openrouter_transcribe.models），
// 想拿到当前完整清单跑
// `python dev-scripts/probe_new_asr_providers.py --target openrouter --list-models`
// （它打的是 /api/v1/models?output_modalities=transcription）。
//
// ── 未经真实接口验证 ──
// 照官方文档实现，没有 OpenRouter key。用
// `python dev-scripts/probe_new_asr_providers.py --target openrouter` 打一次真接口。

use super::diag;
use super::types::{AsrProviderConfig, AsrResult, TestResult};
use std::time::Instant;

const API_URL: &str = "https://openrouter.ai/api/v1/audio/transcriptions";
const SCOPE: &str = "openrouter/asr";

/// 默认模型。选自带标点的那一代，理由见文件头「关于标点这件事」。
const DEFAULT_MODEL: &str = "openai/gpt-transcribe";

/// 我们送的音频容器。裸 PCM 对方认不出来，要封 WAV。
const AUDIO_FORMAT: &str = "wav";

/// 可选的排名归属头。OpenRouter 用它在自己站上做调用量榜单，纯自愿。
///
/// 带上是为了让 SayIt 的调用量归到项目名下（对项目有点好处，对用户无成本）；
/// 它们与鉴权、计费、路由都无关，去掉也一样能用。
const REFERER: &str = "https://sayitapp.site";
const TITLE: &str = "SayIt";

/// 将 16kHz 单声道 16-bit PCM 封装为 WAV 容器。
///
/// 与 asr_groq / asr_mimo / asr_gemini 里那几份是同一个 WAV 头，刻意各留一份：
/// 抽成公共函数后任何一家改采样格式都会牵动其余几家，而它们本来毫无关系。
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

/// 识别语言：设置里存的是 auto|zh|en|…，auto 必须**整个省略 language 字段**。
///
/// 不能传字符串 "auto" —— 文档要的是 ISO-639-1 码，省略才代表自动检测。
fn resolve_language(config: &AsrProviderConfig) -> Option<String> {
    let raw = config
        .extra
        .get("language")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("auto");
    match raw {
        "auto" => None,
        other => Some(other.to_string()),
    }
}

fn build_body(
    model: &str,
    wav_b64: String,
    language: Option<&str>,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": model,
        "input_audio": { "data": wav_b64, "format": AUDIO_FORMAT },
        // temperature 0：转写要照实还原，不要多样性。
        "temperature": 0,
        // 只要 text 一个字段。verbose_json 能给时间戳，但部分模型会**直接 400 拒掉**
        // （文档点名 openai/gpt-4o-transcribe 与 microsoft/mai-transcribe-1.5），
        // 而我们一个时间戳都不用 —— 为了用不上的字段换一类模型失效不值得。
        "response_format": "json",
    });
    if let Some(lang) = language {
        body["language"] = serde_json::json!(lang);
    }
    body
}

/// 响应形状是 `{"text": "...", "usage": {...}}`。
fn extract_text(data: &serde_json::Value) -> String {
    data.get("text")
        .and_then(|t| t.as_str())
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// 这次调用花了多少、走的哪家 —— OpenRouter 会在 usage 里回报。
///
/// 值得记：OpenRouter 是个路由层，同一个模型可能由不同上游承载，
/// 出问题时「花了多少钱」和「实际耗时」是判断打到哪一家的线索。
/// **只记数字，不记文本。**
fn describe_usage(data: &serde_json::Value) -> String {
    let usage = match data.get("usage") {
        Some(u) => u,
        None => return "usage=absent".to_string(),
    };
    let num = |key: &str| {
        usage
            .get(key)
            .and_then(|v| v.as_f64())
            .map(|v| format!("{}={}", key, v))
    };
    let parts: Vec<String> = ["seconds", "total_tokens", "cost"]
        .iter()
        .filter_map(|k| num(k))
        .collect();
    if parts.is_empty() {
        "usage=empty".to_string()
    } else {
        parts.join(" ")
    }
}

/// 生成 ID，出问题时拿它去 OpenRouter 后台查这一次调用。
fn generation_id(headers: &reqwest::header::HeaderMap) -> String {
    headers
        .get("x-generation-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-")
        .to_string()
}

pub async fn transcribe(
    audio_pcm_b64: &str,
    sample_rate: u32,
    config: &AsrProviderConfig,
    hotwords: &[String],
) -> Result<AsrResult, String> {
    // OpenRouter 这条路没有任何地方能放热词：原生 JSON 的参数表里没有 prompt，
    // 它也接受的那套 multipart 里 prompt 是「收下后忽略」（见文件头）。
    // 所以 capabilities.rs 把它记作 ProtocolHasNoSlot —— 和「我们没接」是两件事。
    // 留痕理由同 asr_groq.rs。
    if !hotwords.is_empty() {
        diag::log(
            SCOPE,
            "hotwords_ignored",
            &format!("count={} reason=protocol_has_no_slot", hotwords.len()),
        );
    }

    if config.api_key.trim().is_empty() {
        return Err(diag::fail_code(
            SCOPE,
            "credentials",
            "provider_bad_key",
            "OpenRouter is missing the API Key; complete it in Settings".to_string(),
        ));
    }

    let pcm = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, audio_pcm_b64)
        .map_err(|e| {
            diag::fail(
                SCOPE,
                "decode_b64",
                format!("Failed to decode base64 audio: {}", e),
            )
        })?;
    if pcm.is_empty() {
        diag::empty_result(SCOPE, "Input audio was empty; provider request was skipped");
        return Ok(AsrResult {
            text: String::new(),
            elapsed_ms: 0,
        });
    }

    let audio_sec = pcm.len() as f64 / (sample_rate.max(1) as f64 * 2.0);
    let model = resolve_model(config);
    let language = resolve_language(config);
    let wav = pcm_to_wav(&pcm, sample_rate);
    let wav_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &wav);

    diag::log(
        SCOPE,
        "start",
        &format!(
            "model={} wav_bytes={} audio_sec={:.1} rate={} language={}",
            model,
            wav.len(),
            audio_sec,
            sample_rate,
            language.as_deref().unwrap_or("auto(omitted)")
        ),
    );

    let client = super::http_client::shared();
    let start = Instant::now();
    let resp = client
        .post(API_URL)
        .header(
            "Authorization",
            format!("Bearer {}", config.api_key.trim()),
        )
        .header("HTTP-Referer", REFERER)
        .header("X-Title", TITLE)
        .json(&build_body(&model, wav_b64, language.as_deref()))
        // 上游对每个请求有 60s 超时，我们留够余量再加上传时间
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
        .map_err(|e| diag::fail(SCOPE, "http_send", format!("Request failed: {}", e)))?;

    let elapsed_ms = start.elapsed().as_millis() as u64;
    let http_summary = diag::http_summary(resp.status(), resp.headers());
    let gen_id = generation_id(resp.headers());

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(diag::fail(
            SCOPE,
            "http_status",
            format!(
                "OpenRouter transcription error {} [{}] gen={}: {}",
                status,
                http_summary,
                gen_id,
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
        // 空结果走成功路径，前端只会显示「未检测到有效声音」。不留这条日志的话，
        // 「真的没说话」和「这次调用其实失败了」就再也分不开（见 pitfalls 第 15 条）。
        // gen_id 尤其要记：OpenRouter 是路由层，拿它才能查到究竟是哪家上游返回了空。
        diag::empty_result(
            SCOPE,
            &format!(
                "Response contained no transcript model={} audio_sec={:.1} elapsed={}ms \
                 [{}] gen={} {}",
                model,
                audio_sec,
                elapsed_ms,
                http_summary,
                gen_id,
                describe_usage(&data)
            ),
        );
    } else {
        diag::log(
            SCOPE,
            "usage",
            &format!("model={} gen={} {}", model, gen_id, describe_usage(&data)),
        );
        diag::ok(SCOPE, elapsed_ms, text.chars().count());
    }

    Ok(AsrResult { text, elapsed_ms })
}

pub async fn test_connection(config: &AsrProviderConfig) -> TestResult {
    let model = resolve_model(config);
    // 0.5s 静音：能过鉴权与模型解析就够了，转写结果为空是预期的。
    let silence = vec![0u8; 16000];
    let wav = pcm_to_wav(&silence, 16000);
    let wav_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &wav);

    let client = super::http_client::shared();
    let start = Instant::now();
    let result = client
        .post(API_URL)
        .header(
            "Authorization",
            format!("Bearer {}", config.api_key.trim()),
        )
        .header("HTTP-Referer", REFERER)
        .header("X-Title", TITLE)
        .json(&build_body(&model, wav_b64, None))
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await;

    let elapsed_ms = start.elapsed().as_millis() as u64;
    match result {
        Ok(resp) if resp.status().is_success() => TestResult {
            ok: true,
            message: format!("Connection successful ({}ms)", elapsed_ms),
            elapsed_ms,
            detail: format!("model: {}", model),
        },
        Ok(resp) => {
            let status = resp.status();
            let summary = diag::http_summary(status, resp.headers());
            let gen_id = generation_id(resp.headers());
            let body = resp.text().await.unwrap_or_default();
            TestResult {
                ok: false,
                message: diag::fail(
                    SCOPE,
                    "http_status",
                    format!(
                        "API error {} [{}] gen={}: {}",
                        status,
                        summary,
                        gen_id,
                        diag::truncate(&body, 150)
                    ),
                ),
                elapsed_ms,
                detail: String::new(),
            }
        }
        Err(e) => TestResult {
            ok: false,
            message: diag::fail(SCOPE, "http_send", format!("Connection failed: {}", e)),
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
            provider: "openrouter_transcribe".to_string(),
            api_key: "sk-or-test".to_string(),
            app_id: String::new(),
            extra,
        }
    }

    #[test]
    fn model_falls_back_to_a_punctuating_default() {
        assert_eq!(resolve_model(&config(serde_json::json!({}))), DEFAULT_MODEL);
        assert_eq!(
            resolve_model(&config(serde_json::json!({ "model": "  " }))),
            DEFAULT_MODEL
        );
        assert_eq!(
            resolve_model(&config(serde_json::json!({ "model": "openai/whisper-1" }))),
            "openai/whisper-1"
        );
    }

    /// 默认模型必须带厂商前缀 —— 不带会得到 404 "model not found"。
    #[test]
    fn default_model_is_namespaced() {
        assert!(DEFAULT_MODEL.contains('/'), "slug must be vendor-prefixed");
    }

    /// body 的形状是这份实现的全部前提。尤其 input_audio 是**对象**（data + format），
    /// 不是 OpenAI 那种 multipart 的 file 字段。
    #[test]
    fn body_matches_the_documented_shape() {
        let body = build_body(DEFAULT_MODEL, "QUJD".to_string(), None);
        assert_eq!(body["model"], DEFAULT_MODEL);
        assert_eq!(body["input_audio"]["data"], "QUJD");
        assert_eq!(body["input_audio"]["format"], "wav");
        assert_eq!(body["temperature"], 0);
        // json 而不是 verbose_json：后者会被一部分模型直接 400 拒掉
        assert_eq!(body["response_format"], "json");
        // auto 时绝不能出现 language 字段
        assert!(body.get("language").is_none());
    }

    #[test]
    fn explicit_language_is_passed_but_auto_is_omitted() {
        assert_eq!(resolve_language(&config(serde_json::json!({}))), None);
        assert_eq!(
            resolve_language(&config(serde_json::json!({ "language": "auto" }))),
            None
        );
        assert_eq!(
            resolve_language(&config(serde_json::json!({ "language": "zh" }))),
            Some("zh".to_string())
        );
        let body = build_body(DEFAULT_MODEL, "x".to_string(), Some("ja"));
        assert_eq!(body["language"], "ja");
    }

    #[test]
    fn extracts_text_field() {
        let data = serde_json::json!({ "text": "  语音输入法测试成功。  " });
        assert_eq!(extract_text(&data), "语音输入法测试成功。");
        assert_eq!(extract_text(&serde_json::json!({})), "");
    }

    /// 实测（2026-09-16）：余额不足时 OpenRouter 返回 402，而 402 以前一条分类都
    /// 没命中，界面显示成「连接失败」—— 真实原因是账户没钱，用户看不出该干什么。
    /// 这条钉住 diag 侧的分类（前端那半在 errorMessages.test.ts）。
    #[test]
    fn payment_required_is_classified_as_insufficient_balance() {
        let message = super::super::diag::fail(
            SCOPE,
            "http_status",
            "OpenRouter transcription error 402 Payment Required [http=402] gen=-: \
             {\"error\":{\"message\":\"This request requires at least $0.50 in balance for audio\"}}"
                .to_string(),
        );
        assert!(
            message.contains("provider_insufficient_balance"),
            "402 must not fall back to connect_failed: {}",
            message
        );
        // 服务商给的具体金额要留在消息里，用户才知道要充多少
        assert!(message.contains("$0.50"));
    }

    /// usage 只记数字。这条同时挡住「有人往这里加 text 字段」——
    /// diag 的硬约束是绝不记识别文本。
    #[test]
    fn usage_reports_numbers_only() {
        let data = serde_json::json!({
            "text": "秘密内容",
            "usage": { "seconds": 3.05, "total_tokens": 143, "cost": 0.00012 }
        });
        let summary = describe_usage(&data);
        assert!(summary.contains("seconds=3.05"));
        assert!(summary.contains("total_tokens=143"));
        assert!(summary.contains("cost=0.00012"));
        assert!(!summary.contains("秘密"), "must never carry transcript text");

        assert_eq!(describe_usage(&serde_json::json!({})), "usage=absent");
        assert_eq!(
            describe_usage(&serde_json::json!({ "usage": {} })),
            "usage=empty"
        );
    }

    #[test]
    fn wav_header_is_well_formed() {
        let pcm = vec![0u8; 320];
        let wav = pcm_to_wav(&pcm, 16000);
        assert_eq!(wav.len(), 44 + pcm.len());
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        let data_size = u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]);
        assert_eq!(data_size as usize, pcm.len());
    }
}
