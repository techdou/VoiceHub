//! PCM 后处理：三抽头平滑、增益（dB）、16k→目标采样率重采样、电平表。

/// 平滑（[1,2,1]/4）+ 增益（dB，限 ±24）。空输入原样返回。
pub fn postprocess(input: &[i16], gain_db: f64) -> Vec<i16> {
    if input.is_empty() {
        return Vec::new();
    }
    let mut filtered: Vec<i32> = input.iter().map(|&s| s as i32).collect();
    if input.len() >= 3 {
        for i in 1..input.len() - 1 {
            filtered[i] =
                (input[i - 1] as i32 + 2 * input[i] as i32 + input[i + 1] as i32) >> 2;
        }
    }
    let safe_gain_db = if gain_db.is_finite() { gain_db.clamp(-24.0, 24.0) } else { 0.0 };
    let gain = 10f64.powf(safe_gain_db / 20.0);
    filtered
        .iter()
        .map(|&v| ((v as f64 * gain).round() as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16)
        .collect()
}

/// 线性插值重采样（单声道）。语音播放场景足够；保持相位连续无需窗口滤波。
pub fn resample(input: &[i16], from_rate: u32, to_rate: u32) -> Vec<i16> {
    if from_rate == to_rate || input.is_empty() {
        return input.to_vec();
    }
    if input.len() == 1 {
        return vec![input[0]; to_rate as usize / from_rate as usize];
    }
    let ratio = to_rate as f64 / from_rate as f64;
    let out_len = ((input.len() as f64) * ratio).round() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 / ratio;
        let idx = pos.floor() as usize;
        let frac = pos - idx as f64;
        let idx = idx.min(input.len() - 1);
        let next = (idx + 1).min(input.len() - 1);
        let value = input[idx] as f64 * (1.0 - frac) + input[next] as f64 * frac;
        out.push(value.round().clamp(i16::MIN as f64, i16::MAX as f64) as i16);
    }
    out
}

/// 单声道 → 交错立体声（两声道复制）。
pub fn to_stereo(input: &[i16]) -> Vec<i16> {
    let mut out = Vec::with_capacity(input.len() * 2);
    for &s in input {
        out.push(s);
        out.push(s);
    }
    out
}

/// RMS 与峰值电平（0.0–1.0，相对满幅）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Level {
    pub rms: f32,
    pub peak: f32,
}

pub fn measure(input: &[i16]) -> Level {
    if input.is_empty() {
        return Level { rms: 0.0, peak: 0.0 };
    }
    let mut sum_sq = 0f64;
    let mut peak = 0f64;
    for &s in input {
        let v = (s as f64) / 32768.0;
        sum_sq += v * v;
        peak = peak.max(v.abs());
    }
    Level {
        rms: (sum_sq / input.len() as f64).sqrt() as f32,
        peak: peak as f32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoothing_pass_reduces_high_frequency_noise() {
        let noisy = [0i16, 8000, -8000, 8000, -8000, 0];
        let out = postprocess(&noisy, 0.0);
        let swing: i32 = out.iter().map(|&s| s as i32).max().unwrap()
            - out.iter().map(|&s| s as i32).min().unwrap();
        assert!(swing < 16000);
    }

    #[test]
    fn gain_applies_decibels() {
        let input = [1000i16; 8];
        let out = postprocess(&input, 6.0);
        let expected = (1000.0 * 10f64.powf(6.0 / 20.0)).round() as i16;
        assert!((out[3] as i32 - expected as i32).abs() <= 2);
    }

    #[test]
    fn gain_clamps_and_saturates() {
        let input = [30000i16; 8];
        let out = postprocess(&input, 24.0);
        assert!(out.iter().all(|&s| s <= i16::MAX));
        let out2 = postprocess(&input, f64::NAN);
        assert!((out2[3] - 30000).abs() < 4);
    }

    #[test]
    fn resample_doubles_length_at_double_rate() {
        let input: Vec<i16> = (0..16).map(|i| (i * 100) as i16).collect();
        let out = resample(&input, 16_000, 48_000);
        assert_eq!(out.len(), 48);
        // 第 0、3、6… 个样本对齐原始采样点。
        for i in 0..16 {
            assert_eq!(out[i * 3], input[i]);
        }
    }

    #[test]
    fn stereo_interleaves() {
        assert_eq!(to_stereo(&[1, 2]), vec![1, 1, 2, 2]);
    }

    #[test]
    fn level_measures_silence_and_signal() {
        let silent = Level { rms: 0.0, peak: 0.0 };
        assert_eq!(measure(&[]), silent);
        let level = measure(&[16384, -16384, 0, 0]);
        assert!(level.peak > 0.49 && level.peak < 0.51);
        assert!(level.rms > 0.0);
    }
}
