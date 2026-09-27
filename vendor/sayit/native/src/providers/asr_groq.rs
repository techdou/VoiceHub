// OpenAI 形态的 `POST /audio/transcriptions` —— 目前接了两家：
//   · Groq（whisper-large-v3-turbo / whisper-large-v3）
//   · OpenAI 官方（gpt-transcribe / gpt-4o-transcribe / gpt-4o-mini-transcribe / whisper-1）
//
// 两家共用这一份实现：端点路径、multipart 字段、响应形状完全一样，只有 base URL 和
// 可用模型不同。**不要为 OpenAI 再抄一份文件** —— 那会让标点 prompt、WAV 封装、
// 空结果日志这些真正有内容的部分各自漂移。
//
// **multipart 上传音频文件**，这是本项目唯一用这种形态的 ASR 供应商。其余几家
// （豆包 / 千问 / MiMo）要么是 WebSocket 协议，要么是 chat/completions + base64
// data URL，都不是这个端点。
//
// 与 asr_mimo.rs 的差异：
//   · 鉴权是 Bearer（MiMo 是 api-key 头）；
//   · 音频以 multipart 的 file 字段上传，不是塞进 JSON 的 data URL —— 十几秒的音频
//     用 base64 塞 JSON 会膨胀 1/3，multipart 是原始字节；
//   · 响应是 `{"text": "..."}`，不是 OpenAI chat 的 choices[0].message.content。
//
// 端点写死在这里而不做成可配置项：ASR 侧的约定是「供应商只能从内置清单里选」，
// 因为每家的协议都要一份专门实现，能填任意地址只会让用户以为随便填个地址就能用。
// 模型则是可选的（extra.model），因为换模型不改协议 —— 前端目录里那份清单说明了
// 每家有哪些，见 features/settings/asrProviderCatalog.ts 的 AsrProviderEntry.models。

use super::diag;
use super::types::{AsrProviderConfig, AsrResult, TestResult};
use std::time::Instant;

/// 一家「OpenAI 形态转写」的接入点。
struct Endpoint {
    url: &'static str,
    /// 前端没指定模型时用它
    default_model: &'static str,
    /// 日志作用域，出问题时要一眼看出是哪家
    scope: &'static str,
    /// 前端选定的 `extra.model` 在这个端点上是否可用。
    ///
    /// `openai_live_transcribe` 选的是**流式**模型（gpt-live-transcribe），而
    /// `/audio/transcriptions` 上没有这个模型。那张卡关掉实时字幕时会回落到这条
    /// HTTP 路径（见 registry.rs），照着把流式模型名发出去只会换来一个含义模糊的
    /// 400 —— 而用户完全不知道自己发了个文件端点不认识的模型。所以这一档忽略
    /// 选定值，一律用 default_model。
    honors_selected_model: bool,
    /// 用户填了 `extra.baseUrl` 就用他的，空着用上面的 `url`。
    ///
    /// 为什么内置几家也开：拿中转站 / 反代接 OpenAI 很常见 —— 协议是官方那套，
    /// 只是换了个域名。不给填等于逼这些用户用不了。
    /// 安全边界在前端：只有标了 `supportsCustomUrl` 的模型才会把地址发下来，
    /// 所以档案里的残留地址到不了这里（见 asrEndpointUrl）。
    allows_custom_url: bool,
    /// 地址**必填**（协议卡）。此时 `url` 是空串，没有可回落的内置地址。
    requires_custom_url: bool,
    /// 第三方实现对 `prompt` 的态度不一，有的直接 400。宽容档位在失败后去掉它重试一次。
    ///
    /// 官方那几家不需要这层：它们都接受 prompt，而 prompt 是中文标点的唯一来源
    /// （见 PUNCTUATION_PROMPT），无条件丢掉会让转写结果变差。
    retry_without_prompt: bool,
}

const GROQ: Endpoint = Endpoint {
    url: "https://api.groq.com/openai/v1/audio/transcriptions",
    default_model: "whisper-large-v3-turbo",
    scope: "groq/asr",
    honors_selected_model: true,
    allows_custom_url: true,
    requires_custom_url: false,
    retry_without_prompt: false,
};

const OPENAI: Endpoint = Endpoint {
    url: "https://api.openai.com/v1/audio/transcriptions",
    default_model: "gpt-transcribe",
    scope: "openai/asr",
    honors_selected_model: true,
    allows_custom_url: true,
    requires_custom_url: false,
    retry_without_prompt: false,
};

/// OpenAI 流式那张卡回落到文件端点时用的档位。
/// 端点与 OPENAI 相同，区别只有一个：不接受选定模型（那是流式专用的）。
const OPENAI_FILE_FALLBACK: Endpoint = Endpoint {
    url: OPENAI.url,
    default_model: "gpt-transcribe",
    scope: "openai/asr-fallback",
    honors_selected_model: false,
    // 流式那个模型在前端没标 supportsCustomUrl，所以地址不会发下来；
    // 这里也显式关掉，免得「我没给这个模型填地址、回落时却用了别处的地址」。
    allows_custom_url: false,
    requires_custom_url: false,
    retry_without_prompt: false,
};

/// 「OpenAI 兼容转写」那张卡：协议固定、地址由用户填。
///
/// 对面可能是 whisper.cpp / faster-whisper / FunASR 的 OpenAI 兼容服务、硅基流动，
/// 或者某个聚合网关。所以这一档比内置几家宽容：prompt 被拒时去掉重试，
/// 响应不是 JSON 时按纯文本收（见 extract_text_or_plain）。
const OPENAI_COMPAT: Endpoint = Endpoint {
    url: "",
    default_model: "whisper-1",
    scope: "openai-compat/asr",
    honors_selected_model: true,
    allows_custom_url: true,
    requires_custom_url: true,
    retry_without_prompt: true,
};

/// provider id → 接入点。未知 id 回落到 Groq（registry 只会把这几个 id 分发过来，
/// 加一家时这里和 registry 要一起改）。
fn endpoint_for(provider: &str) -> &'static Endpoint {
    match provider {
        "openai_transcribe" => &OPENAI,
        "openai_live_transcribe" => &OPENAI_FILE_FALLBACK,
        "openai_compat_transcribe" => &OPENAI_COMPAT,
        _ => &GROQ,
    }
}

/// 把用户填的 base URL 接成完整的转写地址。
///
/// 用户会填成什么样无法预设，实际见过这三种，都要能用：
///   · `http://127.0.0.1:8000/v1`            → 补 `/audio/transcriptions`
///   · `http://127.0.0.1:8000/v1/`           → 同上，先去掉尾斜杠
///   · `http://.../v1/audio/transcriptions`  → 已经是完整地址，原样用
/// 不做第四种猜测（比如自动补 `/v1`）：猜错会打到一个存在但语义不同的路径上，
/// 那种失败比「404 地址不对」难查得多。
fn join_transcriptions_url(base: &str) -> String {
    let trimmed = base.trim().trim_end_matches('/');
    if trimmed.ends_with("/audio/transcriptions") {
        trimmed.to_string()
    } else {
        format!("{}/audio/transcriptions", trimmed)
    }
}

/// 本次要打的地址。
fn resolve_url(config: &AsrProviderConfig, endpoint: &Endpoint) -> Result<String, String> {
    let supplied = if endpoint.allows_custom_url {
        config
            .extra
            .get("baseUrl")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or("")
    } else {
        ""
    };
    if !supplied.is_empty() {
        return Ok(join_transcriptions_url(supplied));
    }
    if endpoint.requires_custom_url {
        return Err(diag::fail(
            endpoint.scope,
            "missing_base_url",
            "No endpoint address configured for this service".to_string(),
        ));
    }
    Ok(endpoint.url.to_string())
}

/// 本次要用的模型：前端选定的（extra.model）优先，否则用该家的默认。
///
/// 不校验模型名是否属于这一家 —— 服务商上下线模型比我们改代码勤，硬校验只会让
/// 新模型必须等一次客户端更新才能用。前端目录已经限定了可选项，这里只兜住空值，
/// 以及 `honors_selected_model = false` 那一档（说明见 Endpoint 上的注释）。
fn resolve_model(config: &AsrProviderConfig, endpoint: &Endpoint) -> String {
    if !endpoint.honors_selected_model {
        return endpoint.default_model.to_string();
    }
    config
        .extra
        .get("model")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(endpoint.default_model)
        .to_string()
}

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

fn build_form(
    wav: Vec<u8>,
    model: &str,
    language: Option<&str>,
    scope: &str,
    with_prompt: bool,
) -> Result<reqwest::multipart::Form, String> {
    let part = reqwest::multipart::Part::bytes(wav)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| diag::fail(scope, "build_form", format!("Failed to build the audio part: {}", e)))?;

    let mut form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("model", model.to_string())
        // 显式要 json：不少第三方实现的默认 response_format 不是 json
        // （有的给纯文本、有的给 srt），不写这一行就得靠运气。
        .text("response_format", "json");
    if with_prompt {
        form = form.text("prompt", PUNCTUATION_PROMPT);
    }
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

/// 同上，但响应不是 JSON 时按纯文本收。
///
/// 只给「地址由用户填」那一档用。第三方实现即使收到 `response_format=json` 也可能
/// 回一段裸文本（whisper.cpp 的若干包装就是这样）。这时候硬报 parse_json 等于
/// 把一次**成功的转写**说成失败，而用户看不出问题在哪一层。
///
/// 反过来不能对内置几家开这个口子：那边回非 JSON 说明出了真问题（网关错误页、
/// 被劫持的响应），当成转写文本插进用户文档比报错糟得多。
fn extract_text_or_plain(body: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(body) {
        Ok(data) => extract_text(&data),
        Err(_) => body.trim().to_string(),
    }
}

/// 响应体像不像是在抱怨 `prompt` 这个字段。
///
/// 只在这一个判据成立时才去掉 prompt 重试，不做「失败就重试」—— 密钥错、模型不存在
/// 这类 400 一旦也触发重试，每次转写都要白发两遍（AI 润色那边的降级判定踩过同一个坑，
/// 见 decisions.md「默认替所有 OpenAI 兼容端点关闭思考」）。
fn complains_about_prompt(body: &str) -> bool {
    let lower = body.to_lowercase();
    lower.contains("prompt")
        && (lower.contains("unknown")
            || lower.contains("unexpected")
            || lower.contains("not allowed")
            || lower.contains("unsupported")
            || lower.contains("extra")
            || lower.contains("invalid"))
}

pub async fn transcribe(
    audio_pcm_b64: &str,
    sample_rate: u32,
    config: &AsrProviderConfig,
    hotwords: &[String],
) -> Result<AsrResult, String> {
    let endpoint = endpoint_for(&config.provider);
    let scope = endpoint.scope;

    // 这条路不把热词发给服务端 —— `prompt` 是它唯一能放词表的位置，而我们拿它做了
    // 中文标点引导（见 PUNCTUATION_PROMPT，缺了它中文短句一个标点都没有）。
    //
    // 留这行日志的理由：热词被忽略时转写照样成功，用户只觉得"识别得不太准"，
    // 而在这行存在之前**日志里一个字都查不到**。issue #67 是 FunASR 上游维护者
    // 读源码发现的，不是从日志里看出来的 —— 那说明这个缺口对我们自己也是不可见的。
    // 能力声明在 providers/capabilities.rs，这里只负责留痕。
    if !hotwords.is_empty() {
        diag::log(
            scope,
            "hotwords_ignored",
            &format!(
                "count={} reason=prompt_field_used_for_punctuation",
                hotwords.len()
            ),
        );
    }
    let pcm = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        audio_pcm_b64,
    )
    .map_err(|e| diag::fail(scope, "decode_b64", format!("Failed to decode base64 audio: {}", e)))?;

    if pcm.is_empty() {
        diag::empty_result(scope, "Input audio was empty; provider request was skipped");
        return Ok(AsrResult { text: String::new(), elapsed_ms: 0 });
    }

    let audio_sec = pcm.len() as f64 / (sample_rate.max(1) as f64 * 2.0);
    let language = resolve_language(config);
    let model = resolve_model(config, endpoint);
    let url = resolve_url(config, endpoint)?;
    let wav = pcm_to_wav(&pcm, sample_rate);

    diag::log(
        scope,
        "start",
        &format!(
            "model={} url={} wav_bytes={} audio_sec={:.1} rate={} language={}",
            model,
            url,
            wav.len(),
            audio_sec,
            sample_rate,
            language.as_deref().unwrap_or("auto(omitted)")
        ),
    );

    let client = super::http_client::shared();
    let start = Instant::now();

    // 第一次带 prompt（中文标点全靠它）。只有「地址由用户填」那一档、且服务端明确
    // 抱怨 prompt 时，才去掉它重试一次 —— 判据见 complains_about_prompt。
    let mut with_prompt = true;
    let (body_text, http_summary, elapsed_ms) = loop {
        let form = build_form(wav.clone(), &model, language.as_deref(), scope, with_prompt)?;
        let resp = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", config.api_key))
            .multipart(form)
            .timeout(std::time::Duration::from_secs(60))
            .send()
            .await
            .map_err(|e| diag::fail(scope, "http_send", format!("Request failed: {}", e)))?;

        let elapsed_ms = start.elapsed().as_millis() as u64;
        let http_summary = diag::http_summary(resp.status(), resp.headers());

        if resp.status().is_success() {
            let body_text = resp.text().await.map_err(|e| {
                diag::fail(scope, "read_body", format!("Failed to read response: {}", e))
            })?;
            break (body_text, http_summary, elapsed_ms);
        }

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if with_prompt && endpoint.retry_without_prompt && complains_about_prompt(&body) {
            diag::log(
                scope,
                "retry_without_prompt",
                &format!(
                    "The endpoint rejected the prompt field; retrying without it \
                     (Chinese punctuation may be missing). {}",
                    diag::truncate(&body, 200)
                ),
            );
            with_prompt = false;
            continue;
        }
        return Err(diag::fail(
            scope,
            "http_status",
            format!(
                "Transcription error {} [{}]: {}",
                status,
                http_summary,
                diag::truncate(&body, 300)
            ),
        ));
    };

    // 只有**协议卡**容忍裸文本响应，内置几家仍然要求 JSON（理由见函数注释）。
    //
    // ⚠️ 这里判的是 requires_custom_url 而不是 allows_custom_url：后者现在对
    // Groq / OpenAI 也是真的（它们支持换成中转站地址），拿它当判据会让这两家
    // 也开始把非 JSON 响应当转写文本收 —— 那种响应在官方端点上意味着真出了问题
    // （网关错误页、被劫持的响应），当成文本插进用户文档比报错糟得多。
    let text = if endpoint.requires_custom_url {
        extract_text_or_plain(&body_text)
    } else {
        let data: serde_json::Value = serde_json::from_str(&body_text).map_err(|e| {
            diag::fail(
                scope,
                "parse_json",
                format!(
                    "Failed to parse response: {} [{}] response excerpt: {}",
                    e,
                    http_summary,
                    diag::truncate(&body_text, 200)
                ),
            )
        })?;
        extract_text(&data)
    };
    if text.is_empty() {
        // 空结果走成功路径，前端只会显示「未检测到有效声音」。不留这条日志的话，
        // 「真的没说话」和「这次调用其实失败了」就再也分不开（见 pitfalls #15）。
        diag::empty_result(
            scope,
            &format!(
                "Response contained no transcript audio_sec={:.1} elapsed={}ms [{}] {}",
                audio_sec,
                elapsed_ms,
                http_summary,
                diag::describe_json(&body_text)
            ),
        );
    } else {
        diag::ok(scope, elapsed_ms, text.chars().count());
    }

    Ok(AsrResult { text, elapsed_ms })
}

pub async fn test_connection(config: &AsrProviderConfig) -> TestResult {
    let endpoint = endpoint_for(&config.provider);
    // 0.5s 静音：这些模型会返回空文本但 HTTP 200，足以验证密钥与网络是否可用。
    // 音频再短会被服务端以「too short」拒掉，那就分不清是密钥问题还是音频问题了。
    let silence = vec![0u8; 16000];
    let wav = pcm_to_wav(&silence, 16000);
    let model = resolve_model(config, endpoint);

    // 地址没填时在这里就给出明确结论，而不是拿一个空 URL 去发请求 ——
    // 那会换来一条含义模糊的传输层错误。
    let url = match resolve_url(config, endpoint) {
        Ok(u) => u,
        Err(e) => {
            return TestResult {
                ok: false,
                message: e,
                elapsed_ms: 0,
                detail: String::new(),
            }
        }
    };

    let form = match build_form(wav, &model, None, endpoint.scope, true) {
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
        .post(&url)
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
                    &format!("{}-test", endpoint.scope),
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
            message: diag::fail(
                &format!("{}-test", endpoint.scope),
                "http_send",
                format!("Connection failed: {}", e),
            ),
            elapsed_ms,
            detail: String::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with_language(value: serde_json::Value) -> AsrProviderConfig {
        config_for("groq_whisper", value)
    }

    fn config_for(provider: &str, extra: serde_json::Value) -> AsrProviderConfig {
        AsrProviderConfig {
            provider: provider.to_string(),
            api_key: String::new(),
            app_id: String::new(),
            extra,
        }
    }

    /// 用户填地址的三种常见写法都要接对。
    ///
    /// 接错的表现是 404，而 404 在这个场景下歧义极大 —— 用户会以为自己的服务没起来、
    /// 或者模型名写错了，很难想到是客户端把路径拼坏了。
    #[test]
    fn base_url_is_joined_into_a_transcriptions_path() {
        let expected = "http://127.0.0.1:8000/v1/audio/transcriptions";
        assert_eq!(join_transcriptions_url("http://127.0.0.1:8000/v1"), expected);
        assert_eq!(join_transcriptions_url("http://127.0.0.1:8000/v1/"), expected);
        assert_eq!(join_transcriptions_url("  http://127.0.0.1:8000/v1  "), expected);
        // 已经是完整地址就别再接一层
        assert_eq!(join_transcriptions_url(expected), expected);
        assert_eq!(join_transcriptions_url(&format!("{expected}/")), expected);
    }

    /// 地址没填时必须当场失败，而不是拿空 URL 去发请求。
    #[test]
    fn a_custom_endpoint_without_an_address_fails_before_sending() {
        let missing = config_for("openai_compat_transcribe", serde_json::json!({}));
        assert!(resolve_url(&missing, endpoint_for("openai_compat_transcribe")).is_err());
        let blank = config_for("openai_compat_transcribe", serde_json::json!({ "baseUrl": "   " }));
        assert!(resolve_url(&blank, endpoint_for("openai_compat_transcribe")).is_err());

        let filled = config_for(
            "openai_compat_transcribe",
            serde_json::json!({ "baseUrl": "http://localhost:9000/v1" }),
        );
        assert_eq!(
            resolve_url(&filled, endpoint_for("openai_compat_transcribe")).unwrap(),
            "http://localhost:9000/v1/audio/transcriptions",
        );
    }

    /// 内置几家：地址**空着用官方的、填了用用户的**。
    ///
    /// 允许覆盖是因为中转站 / 反代很常见（协议是官方那套，只换域名）。
    /// 安全边界不在这里，在前端：只有标了 supportsCustomUrl 的模型才会把地址发下来，
    /// 所以档案里的残留地址到不了这一层（见 asrEndpointUrl 的注释）。
    #[test]
    fn builtin_endpoints_default_to_official_and_honor_an_override() {
        for (provider, official) in [
            ("groq_whisper", "api.groq.com"),
            ("openai_transcribe", "api.openai.com"),
        ] {
            let endpoint = endpoint_for(provider);

            let bare = config_for(provider, serde_json::json!({}));
            assert!(resolve_url(&bare, endpoint).unwrap().contains(official));
            let blank = config_for(provider, serde_json::json!({ "baseUrl": "   " }));
            assert!(resolve_url(&blank, endpoint).unwrap().contains(official));

            let relayed = config_for(
                provider,
                serde_json::json!({ "baseUrl": "https://relay.example/v1" }),
            );
            assert_eq!(
                resolve_url(&relayed, endpoint).unwrap(),
                "https://relay.example/v1/audio/transcriptions",
            );
        }
    }

    /// 流式那张卡回落到文件端点时**不**用用户地址。
    ///
    /// 前端没给那个模型标 supportsCustomUrl，所以正常路径上地址根本不会发下来；
    /// 这里再关一道，免得「我没给这个模型填地址、它回落时却用了别处的地址」。
    #[test]
    fn the_live_card_fallback_never_uses_a_custom_url() {
        let config = config_for(
            "openai_live_transcribe",
            serde_json::json!({ "baseUrl": "https://relay.example/v1" }),
        );
        let url = resolve_url(&config, endpoint_for("openai_live_transcribe")).unwrap();
        assert!(url.contains("api.openai.com"), "got {url}");
    }

    /// 去掉 prompt 重试的判据要窄：只在服务端确实在抱怨这个字段时才重试。
    ///
    /// 放宽成「失败就重试」会让密钥错、模型不存在这类 400 每次都白发两遍请求。
    #[test]
    fn prompt_retry_only_triggers_on_complaints_about_prompt() {
        assert!(complains_about_prompt(
            r#"{"error":{"message":"Unknown parameter: 'prompt'"}}"#
        ));
        assert!(complains_about_prompt(
            r#"{"detail":"Extra inputs are not permitted: prompt"}"#
        ));
        assert!(complains_about_prompt("unsupported field prompt"));

        assert!(!complains_about_prompt(r#"{"error":{"message":"Invalid API key"}}"#));
        assert!(!complains_about_prompt(r#"{"error":{"message":"model not found"}}"#));
        assert!(!complains_about_prompt("Internal Server Error"));
        // 只提到 prompt、但没说它有问题的，也不该触发重试
        assert!(!complains_about_prompt("your prompt was transcribed"));
    }

    /// 只有**协议卡**才容忍裸文本响应，内置几家必须要求 JSON。
    ///
    /// 判据是 requires_custom_url，不是 allows_custom_url —— 后者现在对 Groq /
    /// OpenAI 也是真的（它们支持换中转站地址）。搞混的后果是这两家也开始把
    /// 网关错误页当成转写文本插进用户文档。
    #[test]
    fn only_the_protocol_card_tolerates_a_plain_text_body() {
        assert!(endpoint_for("openai_compat_transcribe").requires_custom_url);
        assert!(!endpoint_for("groq_whisper").requires_custom_url);
        assert!(!endpoint_for("openai_transcribe").requires_custom_url);
        assert!(!endpoint_for("openai_live_transcribe").requires_custom_url);

        assert_eq!(extract_text_or_plain(r#"{"text":"来自 JSON"}"#), "来自 JSON");
        assert_eq!(extract_text_or_plain("  裸文本响应  "), "裸文本响应");
    }

    /// 两家共用这份实现，分发只靠 provider id —— 认错一家就会把请求打到另一家的
    /// 域名上，而拿到的只会是一个 401，看不出是路由错了。
    #[test]
    fn provider_id_picks_the_right_endpoint() {
        let groq = endpoint_for("groq_whisper");
        assert!(groq.url.contains("api.groq.com"));
        assert_eq!(groq.default_model, "whisper-large-v3-turbo");

        let openai = endpoint_for("openai_transcribe");
        assert!(openai.url.contains("api.openai.com"));
        assert_eq!(openai.default_model, "gpt-transcribe");

        // 两家的日志作用域必须能区分，否则日志里看不出这次是打给谁的
        assert_ne!(groq.scope, openai.scope);
    }

    /// OpenAI 流式那张卡关掉字幕会回落到文件端点，而它选的模型
    /// （gpt-live-transcribe）在那个端点上不存在 —— 必须被换成 gpt-transcribe。
    /// 这条钉住的是「回落之后静默 400」这个坑。
    #[test]
    fn live_card_falling_back_ignores_the_streaming_model() {
        let endpoint = endpoint_for("openai_live_transcribe");
        assert!(endpoint.url.contains("api.openai.com"));
        assert!(!endpoint.honors_selected_model);
        assert_eq!(
            resolve_model(
                &config_for(
                    "openai_live_transcribe",
                    serde_json::json!({ "model": "gpt-live-transcribe" })
                ),
                endpoint
            ),
            "gpt-transcribe"
        );
    }

    /// 前端选定的模型要真的被用上。这条钉住的是用户报的那个缺口：
    /// 目录里加了 whisper-large-v3，但如果这里仍用常量，选了也还是发 turbo。
    #[test]
    fn selected_model_overrides_the_default() {
        let groq = endpoint_for("groq_whisper");
        assert_eq!(
            resolve_model(&config_for("groq_whisper", serde_json::json!({"model": "whisper-large-v3"})), groq),
            "whisper-large-v3"
        );
        let openai = endpoint_for("openai_transcribe");
        assert_eq!(
            resolve_model(&config_for("openai_transcribe", serde_json::json!({"model": "gpt-4o-mini-transcribe"})), openai),
            "gpt-4o-mini-transcribe"
        );
    }

    /// 没选、空串、只有空白，一律回落到该家的默认，绝不把空模型名发出去
    /// （服务端只会回一个含义模糊的 400）。
    #[test]
    fn blank_model_falls_back_to_default() {
        let groq = endpoint_for("groq_whisper");
        assert_eq!(
            resolve_model(&config_for("groq_whisper", serde_json::json!({})), groq),
            "whisper-large-v3-turbo"
        );
        assert_eq!(
            resolve_model(&config_for("groq_whisper", serde_json::json!({"model": "   "})), groq),
            "whisper-large-v3-turbo"
        );
        assert_eq!(
            resolve_model(&config_for("openai_transcribe", serde_json::json!({"model": ""})), endpoint_for("openai_transcribe")),
            "gpt-transcribe"
        );
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
