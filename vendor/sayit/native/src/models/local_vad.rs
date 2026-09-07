//! 本地 ASR 前置语音检测。
//!
//! Qwen3-ASR 这类生成式模型遇到纯静音或环境底噪时，可能生成并不存在的文字。
//! 这里参考 Handy 的做法，用独立的 Silero VAD 在推理前确认音频里确实有人声。
//! 与 Handy 不同的是，我们只用 VAD 确定首尾边界，保留句子中间的停顿；历史记录
//! 保存的也仍然是未经裁剪的原始录音。

use std::ops::Range;

use silero_vad_crs::{
    get_timestamps_from_probs_with_config, SileroVad, TimestampConfig,
};

/// 与 Handy 一致使用较宽松的 0.3，优先避免漏掉轻声说话。
const SPEECH_THRESHOLD: f32 = 0.3;
/// 过滤总时长不足约 60ms 的候选语音段，抑制短促点击声和提示音。
/// 这与 Handy 的“连续两帧起声”目标相同，但 Silero 时间戳算法带滞回，行为并非逐帧等价。
const MIN_SPEECH_MS: usize = 60;
/// 至少持续 450ms 的非语音才切分语音段，减少短暂停顿导致的误切分。
const MIN_SILENCE_MS: usize = 450;
/// 起止各留约 450ms，避免切掉辅音、轻声开头和句尾。
const SPEECH_PAD_MS: usize = 450;

/// 返回原音频中从第一段人声开始到最后一段人声结束的连续范围。
///
/// `Ok(None)` 明确表示没有检测到人声，调用方应跳过 ASR；`Err` 表示无法可靠
/// 判断是否有人声，为避免生成式模型对静音产生幻觉，调用方也必须停止本次 ASR。
/// 范围中会保留所有句中停顿，只裁掉首尾的非语音。
pub fn detect_speech_span(
    samples: &[f32],
    sample_rate: usize,
) -> Result<Option<Range<usize>>, String> {
    if samples.is_empty() {
        return Ok(None);
    }

    let mut vad = SileroVad::with_sample_rate(sample_rate)
        .map_err(|e| format!("Failed to create Silero VAD: {e}"))?;
    let window_size_samples = vad.source_window_samples();
    let probabilities = vad
        .forward_audio(samples)
        .map_err(|e| format!("Silero VAD inference failed: {e}"))?;

    let segments = get_timestamps_from_probs_with_config(
        &probabilities,
        samples.len(),
        TimestampConfig {
            sampling_rate: sample_rate,
            threshold: SPEECH_THRESHOLD,
            min_speech_duration_ms: MIN_SPEECH_MS,
            min_silence_duration_ms: MIN_SILENCE_MS,
            speech_pad_ms: SPEECH_PAD_MS,
            window_size_samples,
            ..Default::default()
        },
    );

    let (Some(first), Some(last)) = (segments.first(), segments.last()) else {
        return Ok(None);
    };
    let start = first.start.min(samples.len());
    let end = last.end.min(samples.len());
    if start >= end {
        return Ok(None);
    }

    log::debug!(
        "Silero VAD: {:.2}s -> {:.2}s, speech_segments={}",
        samples.len() as f64 / sample_rate as f64,
        (end - start) as f64 / sample_rate as f64,
        segments.len(),
    );
    Ok(Some(start..end))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: usize = 16_000;

    fn read_test_speech() -> Vec<f32> {
        let bytes = include_bytes!("../../resources/test_zh.wav");
        bytes[44..]
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
            .collect()
    }

    #[test]
    fn pure_silence_is_rejected() {
        let audio = vec![0.0; SR * 3];
        assert_eq!(detect_speech_span(&audio, SR).unwrap(), None);
    }

    #[test]
    fn steady_background_noise_is_rejected() {
        let mut state = 0x1234_5678u32;
        let audio: Vec<f32> = (0..SR * 3)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((state >> 8) as f32 / 16_777_215.0 - 0.5) * 0.02
            })
            .collect();
        assert_eq!(detect_speech_span(&audio, SR).unwrap(), None);
    }

    #[test]
    fn short_transient_noise_is_rejected() {
        let mut audio = vec![0.0; SR];
        // 模拟一次约 32ms 的敲击/提示音；不应满足持续人声条件。
        for (i, sample) in audio[SR / 2..SR / 2 + 512].iter_mut().enumerate() {
            let t = i as f32 / SR as f32;
            *sample = 0.4 * (2.0 * std::f32::consts::PI * 1_500.0 * t).sin();
        }
        assert_eq!(detect_speech_span(&audio, SR).unwrap(), None);
    }

    #[test]
    fn bundled_speech_is_detected() {
        let speech = read_test_speech();
        assert!(
            detect_speech_span(&speech, SR).unwrap().is_some(),
            "真实中文语音不应被 VAD 拒绝",
        );
    }

    #[test]
    fn quiet_speech_is_detected() {
        let speech: Vec<f32> = read_test_speech().into_iter().map(|s| s * 0.2).collect();
        assert!(
            detect_speech_span(&speech, SR).unwrap().is_some(),
            "降低约 14dB 的轻声语音不应被 VAD 拒绝",
        );
    }

    #[test]
    fn internal_pause_is_preserved() {
        let speech = read_test_speech();
        let half = speech.len() / 2;
        let mut audio = vec![0.0; SR];
        audio.extend_from_slice(&speech[..half]);
        let pause_start = audio.len();
        audio.extend(std::iter::repeat(0.0).take(SR * 2));
        let pause_end = audio.len();
        audio.extend_from_slice(&speech[half..]);
        audio.extend(std::iter::repeat(0.0).take(SR));

        let span = detect_speech_span(&audio, SR)
            .unwrap()
            .expect("前后两段真实语音应被检测到");
        assert!(span.start < pause_start);
        assert!(span.end > pause_end);
        assert!(audio[span.clone()][pause_start - span.start..pause_end - span.start]
            .iter()
            .all(|sample| *sample == 0.0));
    }
}
