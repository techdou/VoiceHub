// Groq Whisper ASR — whisper-large-v3-turbo
//
// 走 OpenAI 的 `POST /audio/transcriptions`：**multipart 上传音频文件**，
// 这是本项目第一个用这种形态的 ASR 供应商。其余几家（豆包 / 千问 / MiMo）要么是
// WebSocket 协议，要么是 chat/completions + base64 data URL，都不是这个端点。
//
// 与 asr_mimo.rs 的差异：
//   · 鉴权是 Bearer（MiMo 是 api-key 头）；
//   · 音频以 multipart 的 file 字段上传，不是塞进 JSON 的 data URL —— 十几秒的音频
//     用 base64 塞 JSON 会膨胀 1/3，multipart 是原始字节；
//   · 响应是 `{"text": "..."}`，不是 OpenAI chat 的 choices[0].message.content。
//
// 端点写死在这里而不做成可配置项：ASR 侧的约定是「供应商只能从内置清单里选」，
// 因为每家的协议都要一份专门实现，能填任意地址只会让用户以为随便填个地址就能用。

use super::diag;
use super::types::{AsrProviderConfig, AsrResult, TestResult};
use std::time::Instant;

const API_URL: &str = "https://api.groq.com/openai/v1/audio/transcriptions";
const MODEL: &str = "whisper-large-v3-turbo";
const SCOPE: &str = "groq/asr";

/// 标点引导。**这不是可选的润色，缺了它短句中文转写会一个标点都没有。**
///
/// Whisper 的 `prompt` 不是指令，而是「上文样例」：解码时模型会模仿它的书写风格。
/// 所以想要标点，prompt 自己就必须是**带标点的句子** —— 写成「请加标点」毫无作用。
///
/// 实测（dev-scripts/groq-asr-punct-probe.mjs，打的是真实接口）：
///   · 中文短句、不带 prompt   → "语音输入法测试成功"     一个标点都没有
///   · 中文短句、带这个 prompt → "语音输入法测试成功。"
///   · 英文                   → 两种情况都带标点，加了不变差
///
/// 为什么写成中英混排：默认语言是 auto。实测纯中文 prompt 并不会把英文音频带偏
/// （输出仍是纯英文），而混排能让两种语言都拿到标点，且都不会被改写成另一种语言。
///
/// **它管不了全角还是半角。** 另一轮实测（groq-asr-comma-probe.mjs，用一段真的需要
/// 逗号的中文语音）显示：四种 prompt 连同完全不给 prompt，输出一字不差 ——
/// 逗号和问号是半角、句号是全角，混着来。那是模型自身行为，改 prompt 没有任何影响。
/// 半角转全角在前端 textPostProcess.ts 的 normalizeChinesePunctuation 里做。
///
/// 另外验证过静音不会把这段文字当成转写结果吐回来（那会被直接插进用户的文档）。
const PUNCTUATION_PROMPT: &str = "以下是中文转写，使用全角标点，例如逗号、句号和问号。The following is an English transcript with half-width punctuation, such as commas, periods, and question marks.";

/// 将 16kHz 单声道 16-bit PCM 封装为 WAV 容器。
///
/// 前端一路传下来的都是裸 PCM（见 types.rs 的 CloudTranscribeRequest），而
/// /audio/transcriptions 按文件名与内容嗅探格式，裸 PCM 它认不出来。
/// 这 18 行与 asr_mimo.rs 里那份是同一个 WAV 头，刻意各留一份：抽成公共函数后
/// 任何一家改采样格式都会牵动另一家，而它们本来毫无关系。
pub(super) fn pcm_to_wav(pcm: &[u8], sr: u32) -> Vec<u8> {
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

/// 识别语言：设置里存的是 auto|zh|en，Whisper 只接受 ISO-639-1。
///
/// auto 必须**整个省略 language 字段**，不能传字符串 "auto" —— Whisper 会把它当成
/// 一个不存在的语言代码，行为不确定（可能报 400，也可能默默按英语转写）。
/// 返回 None 表示让服务端自己检测。
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

fn build_form(wav: Vec<u8>, language: Option<&str>) -> Result<reqwest::multipart::Form, String> {
    let part = reqwest::multipart::Part::bytes(wav)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| diag::fail(SCOPE, "build_form", format!("Failed to build the audio part: {}", e)))?;

    let mut form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("model", MODEL)
        .text("response_format", "json")
        .text("prompt", PUNCTUATION_PROMPT);
    if let Some(lang) = language {
        form = form.text("language", lang.to_string());
    }
    Ok(form)
}

/// 响应形状是 `{"text": "..."}`，比 chat/completions 简单得多。
fn extract_text(data: &serde_json::Value) -> String {
    data.get("text")
        .and_then(|t| t.as_str())
        .unwrap_or_default()
        .trim()
        .to_string()
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

    let audio_sec = pcm.len() as f64 / (sample_rate.max(1) as f64 * 2.0);
    let language = resolve_language(config);
    let wav = pcm_to_wav(&pcm, sample_rate);

    diag::log(
        SCOPE,
        "start",
        &format!(
            "model={} wav_bytes={} audio_sec={:.1} rate={} language={}",
            MODEL,
            wav.len(),
            audio_sec,
            sample_rate,
            language.as_deref().unwrap_or("auto(omitted)")
        ),
    );

    let form = build_form(wav, language.as_deref())?;
    let client = super::http_client::shared();
    let start = Instant::now();

    let resp = client
        .post(API_URL)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .multipart(form)
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
            format!(
                "Groq ASR error {} [{}]: {}",
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
        // 「真的没说话」和「这次调用其实失败了」就再也分不开（见 pitfalls #15）。
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
    // 0.5s 静音：Whisper 会返回空文本但 HTTP 200，足以验证密钥与网络是否可用。
    // 音频再短会被服务端以「too short」拒掉，那就分不清是密钥问题还是音频问题了。
    let silence = vec![0u8; 16000];
    let wav = pcm_to_wav(&silence, 16000);

    let form = match build_form(wav, None) {
        Ok(f) => f,
        Err(e) => {
            return TestResult {
                ok: false,
                message: e,
                elapsed_ms: 0,
                detail: String::new(),
            }
        }
    };

    let client = super::http_client::shared();
    let start = Instant::now();

    let result = client
        .post(API_URL)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .multipart(form)
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
                    "groq/asr-test",
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
            message: diag::fail("groq/asr-test", "http_send", format!("Connection failed: {}", e)),
            elapsed_ms,
            detail: String::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with_language(value: serde_json::Value) -> AsrProviderConfig {
        AsrProviderConfig {
            provider: "groq_whisper".to_string(),
            api_key: String::new(),
            app_id: String::new(),
            extra: value,
        }
    }

    /// auto 必须省略 language 字段，而不是把 "auto" 当语言代码发出去。
    #[test]
    fn auto_language_is_omitted() {
        assert_eq!(resolve_language(&config_with_language(serde_json::json!({}))), None);
        assert_eq!(
            resolve_language(&config_with_language(serde_json::json!({"language": "auto"}))),
            None
        );
        assert_eq!(
            resolve_language(&config_with_language(serde_json::json!({"language": "  "}))),
            None
        );
    }

    #[test]
    fn explicit_language_is_passed_through() {
        assert_eq!(
            resolve_language(&config_with_language(serde_json::json!({"language": "zh"}))),
            Some("zh".to_string())
        );
    }

    /// WAV 头必须是 44 字节，且长度字段要跟数据长度对得上 —— 写错的话服务端只会回
    /// 一个含义模糊的 400，很难往回定位到这里。
    #[test]
    fn wav_header_is_well_formed() {
        let pcm = vec![0u8; 320];
        let wav = pcm_to_wav(&pcm, 16000);
        assert_eq!(wav.len(), 44 + pcm.len());
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        let riff_size = u32::from_le_bytes([wav[4], wav[5], wav[6], wav[7]]);
        assert_eq!(riff_size as usize, 36 + pcm.len());
        let data_size = u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]);
        assert_eq!(data_size as usize, pcm.len());
    }

    /// 这个 prompt 是靠「示范」生效的：它自己不带标点就一点作用都没有，
    /// 而少了其中一种文字，auto 路径下那种语言就会失去引导。
    /// 有人图省事把它改短时，这条会拦住。
    ///
    /// 注意它**不负责**全角/半角 —— 实测改 prompt 对宽度毫无影响，
    /// 那件事在前端 normalizeChinesePunctuation 里做。
    #[test]
    fn punctuation_prompt_demonstrates_punctuation_in_both_scripts() {
        assert!(PUNCTUATION_PROMPT.contains('。'), "缺中文句号，中文转写会没有标点");
        assert!(PUNCTUATION_PROMPT.contains('，'), "缺中文逗号");
        assert!(PUNCTUATION_PROMPT.contains('.'), "缺英文句号");
        assert!(PUNCTUATION_PROMPT.contains(','), "缺英文逗号");
        assert!(
            PUNCTUATION_PROMPT.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)),
            "缺中文字符"
        );
        assert!(
            PUNCTUATION_PROMPT.chars().any(|c| c.is_ascii_alphabetic()),
            "缺英文字符"
        );
    }

    #[test]
    fn extracts_text_field() {
        let data = serde_json::json!({ "text": "  hello there  " });
        assert_eq!(extract_text(&data), "hello there");
        assert_eq!(extract_text(&serde_json::json!({})), "");
    }
}
