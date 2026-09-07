// 本地 ASR 的命令层 —— 推理本身在 gguf_asr.rs（transcribe.cpp / ggml / GGUF）。
//
// 这里负责：解 base64 → i16 转 f32 → Silero VAD 确认存在人声并裁剪首尾
// → 交给引擎；以及回收上一代 ONNX 模型。原始录音不会被修改。
//
// 历史：0.1.3 之前本地识别走 sherpa-onnx（ONNX Runtime，纯 CPU、INT8 动态量化）。
// Qwen3-ASR 这类自回归模型在那条路上慢一个量级，已整体换成 GGUF/ggml，
// sherpa-onnx 依赖连同 Whisper/Paraformer/FireRed/FunASR 的加载器一并删除。
// 原因与实测数字见 dev-docs/local-asr-ggml-migration.md。

use serde::Serialize;
use std::time::Instant;

use super::downloader::{model_dir, models_dir};
use super::gguf_asr;

/// 采样率固定 16 kHz —— 前端采集就重采样到这个值，引擎也只接受这个值。
const SR: usize = 16000;

#[derive(Debug, Clone, Serialize)]
pub struct LocalAsrResult {
    pub text: String,
    pub elapsed_ms: u64,
}

fn decode_pcm(audio_b64: &str) -> Result<Vec<f32>, String> {
    let pcm_bytes = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        audio_b64,
    )
    .map_err(|e| format!("Failed to decode base64 audio: {}", e))?;

    if pcm_bytes.len() % 2 != 0 {
        return Err(format!(
            "PCM data length must be a multiple of a 16-bit sample; received {} bytes",
            pcm_bytes.len()
        ));
    }

    Ok(pcm_bytes
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
        .collect())
}

#[tauri::command]
pub async fn local_transcribe(
    audio_b64: String,
    model_id: String,
    language: Option<String>,
    accelerator: Option<String>,
) -> Result<LocalAsrResult, String> {
    let samples = decode_pcm(&audio_b64).map_err(|e| {
        crate::providers::diag::fail("local/asr", "decode_pcm", e)
    })?;
    if samples.is_empty() {
        crate::providers::diag::empty_result("local/asr", "Input audio was empty; model inference was skipped");
        return Ok(LocalAsrResult { text: String::new(), elapsed_ms: 0 });
    }

    let accel = accelerator.unwrap_or_else(|| "auto".to_string());
    tokio::task::spawn_blocking(move || {
        use crate::providers::diag;
        let lang = language.as_deref().unwrap_or("auto");
        let start = Instant::now();
        let audio_sec = samples.len() as f64 / SR as f64;
        diag::log(
            "local/asr",
            "start",
            &format!(
                "model={} accel={} lang={} audio_sec={:.1}",
                model_id, accel, lang, audio_sec
            ),
        );
        // 先砍掉首尾静音：解码耗时基本与音频长度成正比，两头的空白是纯浪费
        let trimmed = match super::local_vad::detect_speech_span(&samples, SR) {
            Ok(Some(span)) => &samples[span],
            Ok(None) => {
                // 这是「真的没人说话」，与识别失败要分得清 —— 前端两种都显示
                // 「未检测到有效声音」，只有日志能区分
                diag::empty_result(
                    "local/asr",
                    &format!("VAD detected no speech; model inference was skipped audio_sec={:.1}", audio_sec),
                );
                return Ok(LocalAsrResult {
                    text: String::new(),
                    elapsed_ms: start.elapsed().as_millis() as u64,
                });
            }
            Err(e) => {
                // 防幻觉是硬约束：无法可靠判断是否有人声时，绝不能回退到会把静音
                // 送进生成式模型的旧路径。保留错误便于诊断 VAD/本机环境问题。
                return Err(diag::fail(
                    "local/asr",
                    "vad",
                    format!("Voice activity detection failed; recognition was canceled: {e}"),
                ));
            }
        };
        let text = gguf_asr::transcribe(&model_id, lang, &accel, trimmed, SR)
            .map_err(|e| diag::fail("local/asr", "transcribe", e))?;
        let elapsed_ms = start.elapsed().as_millis() as u64;
        if text.trim().is_empty() {
            // VAD 认定有人声、模型却什么都没输出 —— 模型/量化档/加速器组合有问题的信号
            diag::empty_result(
                "local/asr",
                &format!(
                    "VAD detected speech but the model produced no output model={} accel={} audio_sec={:.1} elapsed={}ms",
                    model_id, accel, audio_sec, elapsed_ms
                ),
            );
        } else {
            diag::ok("local/asr", elapsed_ms, text.chars().count());
        }
        Ok(LocalAsrResult { text, elapsed_ms })
    })
    .await
    .map_err(|e| {
        crate::providers::diag::fail("local/asr", "join", format!("Inference task failed: {}", e))
    })?
}

#[tauri::command]
pub async fn preload_local_model(
    model_id: String,
    accelerator: Option<String>,
) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let start = Instant::now();
        let accel = accelerator.as_deref().unwrap_or("auto");
        gguf_asr::preload(&model_id, accel)?;
        Ok(format!("Model loaded ({}ms)", start.elapsed().as_millis()))
    })
    .await
    .map_err(|e| format!("Preload task failed: {}", e))?
}

/// 释放本地 ASR 模型占用的内存。切换到云 API / 服务器模式时调用，
/// 否则常驻的权重（几百 MB ~ 数 GB）会一直占用到应用退出。
#[tauri::command]
pub async fn unload_local_model() -> Result<(), String> {
    tokio::task::spawn_blocking(gguf_asr::unload)
        .await
        .map_err(|e| format!("Release task failed: {}", e))
}

/// 动态调整本地模型的空闲卸载时间。0 = 常驻；其余值由设置页限制为
/// 10 / 30 / 60 分钟。守护线程只启动一次，这里仅更新它读取的原子配置。
#[tauri::command]
pub fn set_local_model_idle_unload(idle_minutes: u64) -> Result<(), String> {
    if !matches!(idle_minutes, 0 | 10 | 30 | 60) {
        return Err(format!("Unsupported model idle unload interval: {idle_minutes}"));
    }
    gguf_asr::set_idle_unload_minutes(idle_minutes);
    Ok(())
}

// ── 上一代 ONNX 模型的回收 ──

/// 上一代（sherpa-onnx 时期）用过的模型目录名。新引擎读不了这些文件，
/// 留着只是白占几百 MB ~ 1 GB 磁盘，所以启动时清掉。
///
/// 用白名单而不是"扫描整个模型根目录"：模型根目录是用户可改的（可能指向
/// 一个还放着别的东西的文件夹），绝不能在那儿做递归删除。
const LEGACY_MODEL_IDS: &[&str] = &[
    "sensevoice-small",
    "qwen3-asr-0.6b",
    "paraformer-zh",
    "whisper-tiny",
    "whisper-base",
    "whisper-small",
    "whisper-medium",
    "funasr-nano",
    "funasr-nano-int8",
    "fire-red-asr2-ctc",
    "fire-red-asr2-aed",
];

/// 目录看起来确实是旧的 ONNX 模型吗？要求：目录存在、里面有 .onnx，
/// 且**没有** .gguf（万一用户把新权重放进了同名目录，不能误删）。
fn looks_like_legacy_onnx_model(dir: &std::path::Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    let mut has_onnx = false;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        match entry
            .path()
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
        {
            Some(ext) if ext == "gguf" => return false,
            Some(ext) if ext == "onnx" => has_onnx = true,
            _ => {}
        }
    }
    has_onnx
}

fn dir_size(dir: &std::path::Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| {
            let p = e.path();
            if p.is_dir() {
                dir_size(&p)
            } else {
                std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0)
            }
        })
        .sum()
}

/// 删掉旧引擎的模型目录，返回释放的字节数。
///
/// 换引擎后这些 ONNX 权重永远用不上了，所以直接回收，不再问用户
/// （旧 silero_vad.onnx 也一起，新引擎不需要 VAD 模型）。
pub fn reclaim_legacy_models() -> u64 {
    let root = models_dir();
    if !root.exists() {
        return 0;
    }

    let mut freed = 0u64;
    for id in LEGACY_MODEL_IDS {
        let dir = model_dir(id);
        if !looks_like_legacy_onnx_model(&dir) {
            continue;
        }
        let size = dir_size(&dir);
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => {
                freed += size;
                log::info!(
                    "Removed legacy engine model {} ({:.1} MB)",
                    id,
                    size as f64 / 1024.0 / 1024.0
                );
            }
            Err(e) => log::warn!("Failed to remove legacy engine model {}: {}", id, e),
        }
    }

    // 旧的外置 ONNX VAD 权重：新实现使用静态内嵌权重，不再需要这个文件。
    let vad = root.join("silero_vad.onnx");
    if vad.is_file() {
        let size = std::fs::metadata(&vad).map(|m| m.len()).unwrap_or(0);
        if std::fs::remove_file(&vad).is_ok() {
            freed += size;
            log::info!("Removed legacy VAD weights silero_vad.onnx");
        }
    }

    if freed > 0 {
        log::info!("Removed {:.1} MB of legacy engine models", freed as f64 / 1024.0 / 1024.0);
    }
    freed
}

/// 供前端查询本次启动回收了多少空间（可用于提示"已释放 XXX MB"）。
#[tauri::command]
pub fn legacy_models_reclaimed_bytes() -> u64 {
    RECLAIMED.load(std::sync::atomic::Ordering::Relaxed)
}

static RECLAIMED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// 启动时在后台回收旧模型（几百 MB 的删除别卡启动路径）。
pub fn spawn_legacy_reclaim() {
    std::thread::spawn(|| {
        let freed = reclaim_legacy_models();
        RECLAIMED.store(freed, std::sync::atomic::Ordering::Relaxed);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_detection_requires_onnx_and_rejects_gguf() {
        let tmp = std::env::temp_dir().join(format!("sayit-legacy-test-{}", std::process::id()));
        let onnx_dir = tmp.join("old");
        let gguf_dir = tmp.join("new");
        let mixed_dir = tmp.join("mixed");
        for d in [&onnx_dir, &gguf_dir, &mixed_dir] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(onnx_dir.join("model.int8.onnx"), b"x").unwrap();
        std::fs::write(gguf_dir.join("m-Q8_0.gguf"), b"x").unwrap();
        // 同名目录里既有旧 onnx 又有新 gguf 时，必须判为"不是旧模型"，不能删
        std::fs::write(mixed_dir.join("model.int8.onnx"), b"x").unwrap();
        std::fs::write(mixed_dir.join("m-Q8_0.gguf"), b"x").unwrap();

        assert!(looks_like_legacy_onnx_model(&onnx_dir));
        assert!(!looks_like_legacy_onnx_model(&gguf_dir));
        assert!(!looks_like_legacy_onnx_model(&mixed_dir));
        assert!(!looks_like_legacy_onnx_model(&tmp.join("does-not-exist")));

        std::fs::remove_dir_all(&tmp).ok();
    }

    fn run_local_transcribe(audio_b64: String, model_id: &str) -> Result<LocalAsrResult, String> {
        // 与 main.rs 保持同一启动顺序；dynamic-backends 模式必须先注册 GGML 后端。
        super::super::gguf_asr::init_backends();
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("创建测试 runtime 失败")
            .block_on(local_transcribe(
                audio_b64,
                model_id.to_string(),
                Some("auto".to_string()),
                Some("auto".to_string()),
            ))
    }

    /// 走真实生产路径：base64 → i16→f32 → Silero VAD → GGUF 引擎。
    #[test]
    fn end_to_end_base64_to_text() {
        let model_id = "sensevoice-small-gguf";
        if !super::super::gguf_asr::model_is_downloaded(model_id) {
            eprintln!("skip: {model_id} 未下载");
            return;
        }

        // 前后各加 2s 静音，验证 VAD 裁剪后仍能识别中间的真实语音。
        let wav = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/test_zh.wav");
        let bytes = std::fs::read(&wav).unwrap();
        let speech = &bytes[44..];
        let silence = vec![0u8; SR * 2 * 2]; // 2s、16-bit PCM
        let mut pcm_bytes = Vec::new();
        pcm_bytes.extend_from_slice(&silence);
        pcm_bytes.extend_from_slice(speech);
        pcm_bytes.extend_from_slice(&silence);

        let b64 = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &pcm_bytes,
        );
        let result = run_local_transcribe(b64, model_id).unwrap();
        assert!(
            !result.text.trim().is_empty(),
            "VAD 后仍应识别出内容，got {:?}",
            result.text
        );
        super::super::gguf_asr::unload();
    }

    #[test]
    fn silence_short_circuits_before_loading_model() {
        let pcm_bytes = vec![0u8; SR * 2 * 3]; // 3s、16-bit PCM 纯静音
        let b64 = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &pcm_bytes,
        );

        // 故意传不存在的模型：若 VAD 没有先短路，这里会因模型不存在而报错。
        let result = run_local_transcribe(b64, "model-that-must-not-be-loaded").unwrap();
        assert!(result.text.is_empty());
    }

    #[test]
    fn decode_pcm_maps_i16_to_unit_range() {
        // i16 -32768 / 0 / 32767 → -1.0 / 0.0 / ~1.0
        let bytes: Vec<u8> = [i16::MIN, 0i16, i16::MAX]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
        let pcm = decode_pcm(&b64).unwrap();
        assert_eq!(pcm.len(), 3);
        assert!((pcm[0] + 1.0).abs() < 1e-6);
        assert_eq!(pcm[1], 0.0);
        assert!((pcm[2] - 1.0).abs() < 1e-4);
    }

    #[test]
    fn decode_pcm_rejects_incomplete_i16_sample() {
        let b64 = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            [0u8, 1, 2],
        );
        let err = decode_pcm(&b64).unwrap_err();
        assert!(err.contains("16-bit"), "unexpected error: {err}");
    }
}
