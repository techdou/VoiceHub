// Google Gemini 转写 — gemini-3.5-transcribe
//
// 走 Gemini API 的 `POST /v1beta/models/{model}:generateContent`，音频以
// `inline_data` 内联上传（base64 的 WAV）。
//
// ── 为什么是 Gemini API 而不是 Cloud Speech-to-Text（Chirp 3）──
// Chirp 3 的准确率也在第一档，但它在 Google Cloud 那套体系里：要建 GCP 项目、开
// Speech-to-Text API、拿服务账号 JSON、走 OAuth2 换 access token（还要处理过期刷新）。
// 我们这一页的前提是「粘一把 API Key 就能用」—— 让用户去配服务账号，绝大多数人到不了
// 能用的那一步。Gemini API 只要一个 `x-goog-api-key`，和别的供应商一样。
//
// ── 为什么用 generateContent 而不是新的 Interactions API ──
// 官方现在把 generateContent 标成 legacy、推荐 Interactions。但 legacy 仍然可用，
// 形状简单（一个 POST、一个 JSON），而 Interactions 换来的能力（多轮、工具）
// 我们一条都用不上。等 legacy 真要下线再迁。
//
// ── MEASURED 2026-09-16（`dev-scripts/probe_new_asr_providers.py --target gemini-file`，
//    打的是真实接口，用一个**没有绑定结算信息**的 AI Studio 项目）──
//   · 免费层可用：HTTP 200、`serviceTier: standard`，3.05s 中文音频 2113ms 返回。
//     不需要绑账单。（绑过账单又欠费的项目反而不行：那种会 429
//     "prepayment credits are depleted"，因为它已经离开免费层了。）
//   · **响应形状与通用模型不同**：转写专用模型把结果放在
//     `parts[0].audioTranscription.text`，不是 `parts[0].text`。
//     照通用模型那样只读 `text` 会得到「HTTP 200 但一个字都没有」——
//     而 finishReason=STOP、promptFeedback=null，看上去毫无异常。见 extract_text。
//   · `audio/wav` + inline_data 被接受（原 note B 确认）。
//   · 模型不答话、不加前缀，直接给出带句尾标点的纯转写（原 note C 确认）。
//     它的返回字段本身就是专用的 audioTranscription，说明这个模型压根不走"回答"那条路。
//   · token 计费：3.05s 音频 = 77 audio tokens，我们这段 prompt = 66 text tokens。
//     prompt 已经和音频一个量级了，往里加东西前先想想值不值。
//
// ── 仍未验证的一处 ──
//   A. **热词与语种有没有专用字段**。官方宣传里有 custom vocabulary biasing 与
//      语种检测，但公开文档没给出 generateContent 下的字段名。这里一律写进 prompt
//      文本 —— 对 Gemini 这种指令跟随的模型 prompt 本来就有效，只是不如专用字段精确，
//      而且占 token（见上面那条）。若实测发现有专用字段（如 `speechConfig`），
//      改成用它更好。

use super::diag;
use super::types::{AsrProviderConfig, AsrResult, TestResult};
use std::time::Instant;

const API_BASE: &str = "https://generativelanguage.googleapis.com/v1beta/models";
const SCOPE: &str = "gemini/asr";
const DEFAULT_MODEL: &str = "gemini-3.5-transcribe";

/// 转写指令。
///
/// **这不是可选的润色，缺了它模型可能去"回答"音频内容。** 与 Whisper 那种
/// 「拿带标点的句子做示范」不同，Gemini 是指令跟随的，所以这里写的是明确要求。
///
/// 每一条都对应一种实际会出错的行为：
///   · 「只输出转写文本」—— 不然会得到「这段音频里说话人提到……」这样的描述；
///   · 「不要加解释或前后缀」—— 不然会出现「以下是转写内容：」这种开场白，
///     而它会被原样插进用户的文档；
///   · 「听不出内容就返回空」—— 不然会编一段合理的话（幻听），这比没有结果更糟。
const TRANSCRIBE_PROMPT: &str = "Transcribe this audio verbatim. Output only the transcript text with natural punctuation. Do not add explanations, prefixes, quotation marks, timestamps, or speaker labels. If the audio contains no intelligible speech, output nothing at all.";

/// 热词条数上限。全部塞进 prompt，给太多会稀释每一个的作用，也会把 prompt 撑长。
///
/// ⚠️ 这是**客户端自设**的截断，界面必须说出来。漏说的代价是静默：配了 150 个词
/// 只发 100 个，而热词页既不显示上限、又够不到 `HOTWORD_SOFT_LIMIT`（200）那条
/// 通用提醒，用户完全看不到后 50 个被丢掉了。声明处是 `capabilities.rs::client_cap`。
const HOTWORD_LIMIT: usize = 100;

/// 给 `capabilities.rs` 的测试用，让那边的说明值和这里的请求参数钉在一起。
/// 只在测试里存在：说明和请求参数刻意各留一份，理由见 capabilities.rs。
#[cfg(test)]
pub fn hotword_limit_for_docs() -> usize {
    HOTWORD_LIMIT
}

/// 将 16kHz 单声道 16-bit PCM 封装为 WAV 容器。
///
/// 与 asr_groq.rs / asr_mimo.rs 里那两份是同一个 WAV 头，刻意各留一份：
/// 抽成公共函数后任何一家改采样格式都会牵动另外两家，而它们本来毫无关系。
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

/// 流式那张卡的 provider id。它关掉实时字幕时会回落到这条 HTTP 路径。
const LIVE_PROVIDER: &str = "gemini_live_transcribe";

/// 本次要用的模型：前端选定的优先，否则默认。
///
/// **`gemini_live_transcribe` 的选定模型在这里一律忽略。** 那张卡选的是流式模型
/// （gemini-3.5-transcribe-live），generateContent 上没有这个模型；关掉实时字幕
/// 回落到这条路时照发只会换来一个含义模糊的 404/400，而用户完全不知道自己发了
/// 一个文件端点不认识的模型。同一个取舍在 asr_groq.rs 用 honors_selected_model 表达。
fn resolve_model(config: &AsrProviderConfig) -> String {
    if config.provider == LIVE_PROVIDER {
        return DEFAULT_MODEL.to_string();
    }
    config
        .extra
        .get("model")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_MODEL)
        .to_string()
}

/// 识别语言：设置里存的是 auto|zh|en|…，auto 时**整句不提语言**。
///
/// 不能写「language: auto」之类的字样 —— 那是给模型看的自然语言，
/// 写一个它不认识的语言名只会让它困惑。auto 就是让它自己判断。
fn language_hint(config: &AsrProviderConfig) -> Option<String> {
    let raw = config
        .extra
        .get("language")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("auto");
    match raw {
        "auto" => None,
        other => Some(format!(" The audio is in this language: {}.", other)),
    }
}

/// 组 prompt：转写指令 + 可选语言 + 可选热词。
fn build_prompt(config: &AsrProviderConfig, hotwords: &[String]) -> String {
    let mut prompt = String::from(TRANSCRIBE_PROMPT);
    if let Some(lang) = language_hint(config) {
        prompt.push_str(&lang);
    }
    let mut seen = std::collections::HashSet::new();
    let words: Vec<&str> = hotwords
        .iter()
        .map(|w| w.trim())
        .filter(|w| !w.is_empty())
        .filter(|w| seen.insert(w.to_lowercase()))
        .take(HOTWORD_LIMIT)
        .collect();
    if !words.is_empty() {
        diag::log(
            SCOPE,
            "hotwords",
            &format!("vocabulary={} of={}", words.len(), hotwords.len()),
        );
        prompt.push_str(
            " These terms may appear in the audio; spell them exactly as written here: ",
        );
        prompt.push_str(&words.join(", "));
        prompt.push('.');
    }
    prompt
}

fn build_body(prompt: &str, wav_b64: String) -> serde_json::Value {
    serde_json::json!({
        "contents": [{
            "role": "user",
            "parts": [
                { "text": prompt },
                { "inline_data": { "mime_type": "audio/wav", "data": wav_b64 } }
            ]
        }],
        // temperature 0：转写要的是照实还原，不是多样性。
        "generationConfig": { "temperature": 0 }
    })
}

/// 从响应里取文本。
///
/// **两种 part 形状都要认**（实测 2026-09-16，见文件头 MEASURED）：
///   · 转写专用模型 `gemini-3.5-transcribe` 给的是
///     `{"audioTranscription": {"text": "语音输入法测试成功。"}}`；
///   · 通用模型（gemini-*-flash 那些）给的是普通的 `{"text": "..."}`。
///
/// 只认后者的代价极其隐蔽：HTTP 200、`finishReason: STOP`、`promptFeedback: null`，
/// 一切正常，就是提不出字 —— 前端把它显示成「未检测到有效声音」，
/// 用户会去查麦克风（pitfalls 第 15 条）。第一次实测就是这么栽的。
///
/// parts 可能有多段，要全部拼起来 —— 只取 `parts[0]` 会在模型分段输出时丢掉后半截。
fn extract_text(data: &serde_json::Value) -> String {
    let Some(parts) = data
        .pointer("/candidates/0/content/parts")
        .and_then(|p| p.as_array())
    else {
        return String::new();
    };
    parts
        .iter()
        .filter_map(|p| {
            p.pointer("/audioTranscription/text")
                .and_then(|t| t.as_str())
                .or_else(|| p.get("text").and_then(|t| t.as_str()))
        })
        .collect::<Vec<_>>()
        .join("")
        .trim()
        .to_string()
}

/// 为什么这次没有文本 —— 内容审查拦截、模型自己停在了别的原因上，
/// 或者**响应里有 parts 但我们一个字段都不认识**。
///
/// 不区分的话，这些全都会显示成「未检测到有效声音」，把用户引去查麦克风。
/// 两种要单独说清的：
///   · safety 拦截 —— 这段音频永远不会有结果，重试没有意义；
///   · 认不出的 part 字段 —— 那是**我们的 bug**，不是用户的音频问题。
///     第一次实测就栽在这个上（专用模型用 audioTranscription 而不是 text），
///     而当时只报了 finishReason=STOP，看上去像一切正常但就是没说话。
///     把 part 的键名列出来，下次照着改 extract_text 就行。
fn describe_empty(data: &serde_json::Value) -> String {
    if let Some(reason) = data
        .pointer("/promptFeedback/blockReason")
        .and_then(|r| r.as_str())
    {
        return format!("blocked by content filter: {}", reason);
    }
    let finish = data
        .pointer("/candidates/0/finishReason")
        .and_then(|r| r.as_str());
    if let Some(parts) = data
        .pointer("/candidates/0/content/parts")
        .and_then(|p| p.as_array())
    {
        if !parts.is_empty() {
            let keys: Vec<&str> = parts
                .iter()
                .filter_map(|p| p.as_object())
                .flat_map(|o| o.keys().map(|k| k.as_str()))
                .collect();
            return format!(
                "response had {} part(s) but no field we recognize (keys: {}) finishReason={} \
                 -- this is our parsing bug, not an audio problem",
                parts.len(),
                keys.join(","),
                finish.unwrap_or("?")
            );
        }
    }
    match finish {
        Some(reason) => format!("finishReason={}", reason),
        None => "no candidates in response".to_string(),
    }
}

fn endpoint(model: &str) -> String {
    format!("{}/{}:generateContent", API_BASE, model)
}

pub async fn transcribe(
    audio_pcm_b64: &str,
    sample_rate: u32,
    config: &AsrProviderConfig,
    hotwords: &[String],
) -> Result<AsrResult, String> {
    if config.api_key.trim().is_empty() {
        return Err(diag::fail_code(
            SCOPE,
            "credentials",
            "provider_bad_key",
            "Gemini transcription is missing the API Key; complete it in Settings".to_string(),
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
    let prompt = build_prompt(config, hotwords);
    let wav = pcm_to_wav(&pcm, sample_rate);
    let wav_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &wav);

    diag::log(
        SCOPE,
        "start",
        &format!(
            "model={} wav_bytes={} audio_sec={:.1} rate={} hotwords={}",
            model,
            wav.len(),
            audio_sec,
            sample_rate,
            hotwords.len()
        ),
    );

    let client = super::http_client::shared();
    let start = Instant::now();
    let resp = client
        .post(endpoint(&model))
        // 密钥走请求头而不是 query 参数：query 会被各层日志、代理、错误页原样记下来。
        .header("x-goog-api-key", config.api_key.trim())
        .json(&build_body(&prompt, wav_b64))
        .timeout(std::time::Duration::from_secs(120))
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
            format!(
                "Gemini transcription error {} [{}]: {}",
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
        // 空结果走成功路径，前端只会显示「未检测到有效声音」。不留这条日志的话，
        // 「真的没说话」和「被内容审查拦了」就再也分不开（见 pitfalls 第 15 条）。
        diag::empty_result(
            SCOPE,
            &format!(
                "Response contained no transcript audio_sec={:.1} elapsed={}ms [{}] {}",
                audio_sec,
                elapsed_ms,
                http_summary,
                describe_empty(&data)
            ),
        );
    } else {
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
        .post(endpoint(&model))
        .header("x-goog-api-key", config.api_key.trim())
        .json(&build_body(TRANSCRIBE_PROMPT, wav_b64))
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
            let body = resp.text().await.unwrap_or_default();
            TestResult {
                ok: false,
                message: diag::fail(
                    SCOPE,
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
            provider: "gemini_transcribe".to_string(),
            api_key: "key".to_string(),
            app_id: String::new(),
            extra,
        }
    }

    #[test]
    fn model_falls_back_to_the_default() {
        assert_eq!(resolve_model(&config(serde_json::json!({}))), DEFAULT_MODEL);
        assert_eq!(
            resolve_model(&config(serde_json::json!({ "model": "  " }))),
            DEFAULT_MODEL
        );
        assert_eq!(
            resolve_model(&config(serde_json::json!({ "model": "gemini-3.5-flash" }))),
            "gemini-3.5-flash"
        );
    }

    /// 流式那张卡关掉字幕会回落到这条 HTTP 路径，而它选的模型
    /// （gemini-3.5-transcribe-live）在 generateContent 上不存在 —— 必须被换掉。
    #[test]
    fn live_card_falling_back_ignores_the_streaming_model() {
        let mut cfg = config(serde_json::json!({ "model": "gemini-3.5-transcribe-live" }));
        cfg.provider = LIVE_PROVIDER.to_string();
        assert_eq!(resolve_model(&cfg), DEFAULT_MODEL);
    }

    #[test]
    fn endpoint_targets_generate_content() {
        assert_eq!(
            endpoint("gemini-3.5-transcribe"),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.5-transcribe:generateContent"
        );
    }

    /// auto 时**一个字都不能提语言** —— 写个模型不认识的语言名只会让它困惑。
    #[test]
    fn auto_language_adds_no_hint() {
        assert_eq!(language_hint(&config(serde_json::json!({}))), None);
        assert_eq!(
            language_hint(&config(serde_json::json!({ "language": "auto" }))),
            None
        );
        assert!(language_hint(&config(serde_json::json!({ "language": "zh" })))
            .unwrap()
            .contains("zh"));
    }

    /// prompt 里那三条禁令是防「模型答话而不是转写」的唯一手段，被人图省事删掉时这条会拦住。
    #[test]
    fn prompt_forbids_commentary_and_hallucination() {
        let prompt = build_prompt(&config(serde_json::json!({})), &[]);
        assert!(prompt.contains("Output only the transcript"));
        assert!(prompt.contains("Do not add explanations"));
        assert!(prompt.contains("output nothing at all"));
    }

    #[test]
    fn hotwords_go_into_the_prompt_deduped_and_capped() {
        let words = vec!["SayIt".to_string(), "sayit".to_string(), " Kiro ".to_string()];
        let prompt = build_prompt(&config(serde_json::json!({})), &words);
        assert!(prompt.contains("SayIt, Kiro"));

        let many: Vec<String> = (0..HOTWORD_LIMIT + 20).map(|i| format!("w{}", i)).collect();
        let capped = build_prompt(&config(serde_json::json!({})), &many);
        assert!(capped.contains("w0"));
        assert!(!capped.contains(&format!("w{}", HOTWORD_LIMIT + 5)));
    }

    #[test]
    fn body_carries_audio_inline() {
        let body = build_body("say it", "QUJD".to_string());
        let parts = &body["contents"][0]["parts"];
        assert_eq!(parts[0]["text"], "say it");
        assert_eq!(parts[1]["inline_data"]["mime_type"], "audio/wav");
        assert_eq!(parts[1]["inline_data"]["data"], "QUJD");
        assert_eq!(body["generationConfig"]["temperature"], 0);
    }

    /// parts 可能分段，只取 parts[0] 会丢后半截。
    #[test]
    fn extracts_and_joins_all_text_parts() {
        let data = serde_json::json!({
            "candidates": [{ "content": { "parts": [{ "text": "  你好" }, { "text": "，世界。 " }] } }]
        });
        assert_eq!(extract_text(&data), "你好，世界。");
        assert_eq!(extract_text(&serde_json::json!({})), "");
    }

    /// 实测响应的原样形状（2026-09-16）。转写专用模型用 audioTranscription 包一层，
    /// 只读 `text` 的话这条会失败 —— 而线上表现是「HTTP 200 但一个字都没有」。
    #[test]
    fn extracts_the_dedicated_audio_transcription_field() {
        let data = serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{ "audioTranscription": { "text": "语音输入法测试成功。" } }],
                    "role": "model"
                },
                "finishReason": "STOP"
            }],
            "modelVersion": "gemini-3.5-transcribe"
        });
        assert_eq!(extract_text(&data), "语音输入法测试成功。");
    }

    /// 两种形状混在一起也要按顺序拼全（真出现分段时不至于丢半截）。
    #[test]
    fn handles_both_part_shapes_together() {
        let data = serde_json::json!({
            "candidates": [{ "content": { "parts": [
                { "audioTranscription": { "text": "前半" } },
                { "text": "后半" }
            ] } }]
        });
        assert_eq!(extract_text(&data), "前半后半");
    }

    /// 空结果的原因必须能区分：被审查拦掉是「重试也没用」，和「没说话」完全不同。
    #[test]
    fn empty_reason_distinguishes_safety_block() {
        let blocked = serde_json::json!({ "promptFeedback": { "blockReason": "SAFETY" } });
        assert!(describe_empty(&blocked).contains("content filter"));
        assert!(describe_empty(&blocked).contains("SAFETY"));

        let stopped = serde_json::json!({ "candidates": [{ "finishReason": "MAX_TOKENS" }] });
        assert!(describe_empty(&stopped).contains("MAX_TOKENS"));

        assert!(describe_empty(&serde_json::json!({})).contains("no candidates"));
    }

    /// 认不出字段时必须直说「这是我们的解析 bug」并列出键名。
    ///
    /// 第一次实测就卡在这里：当时只报 finishReason=STOP，看上去像用户没说话，
    /// 而真相是响应里躺着一段完美的转写、只是包在一个我们没读的字段里。
    #[test]
    fn empty_reason_calls_out_unrecognized_part_fields() {
        let data = serde_json::json!({
            "candidates": [{
                "content": { "parts": [{ "somethingNew": { "text": "hi" } }] },
                "finishReason": "STOP"
            }]
        });
        let why = describe_empty(&data);
        assert!(why.contains("somethingNew"), "must name the unknown key: {}", why);
        assert!(why.contains("parsing bug"), "must say it is our bug: {}", why);
        assert!(why.contains("STOP"));
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
