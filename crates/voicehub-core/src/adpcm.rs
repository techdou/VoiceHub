//! IMA / DVI ADPCM 编解码。
//!
//! 遥控器固件（RC001 / RC003）以**高半字节优先**输出 nibble；ARN9 固件
//! 相反。解码器可在运行时翻转顺序（按 2A24 型号识别结果设置）。
//! 附带编码器：单测做往返校验、模拟遥控器页合成音频时复用。

#[derive(Debug, Clone)]
#[derive(Default)]
pub struct ImaAdpcmCodec {
    predictor: i32,
    step_index: i32,
    low_nibble_first: bool,
}

const STEP_TABLE: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45,
    50, 55, 60, 66, 73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230,
    253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658, 724, 796, 876, 963,
    1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272, 2499, 2749, 3024, 3327,
    3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493, 10442,
    11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794,
    32767,
];
const INDEX_TABLE: [i32; 8] = [-1, -1, -1, -1, 2, 4, 6, 8];


impl ImaAdpcmCodec {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn predictor(&self) -> i16 {
        self.predictor.clamp(i16::MIN as i32, i16::MAX as i32) as i16
    }

    pub fn step_index(&self) -> u8 {
        self.step_index.clamp(0, 88) as u8
    }

    pub fn low_nibble_first(&self) -> bool {
        self.low_nibble_first
    }

    /// ARN9 固件使用低半字节优先；连接时按型号识别结果调用。
    pub fn set_low_nibble_first(&mut self, low_first: bool) {
        self.low_nibble_first = low_first;
    }

    pub fn reset(&mut self) {
        self.reset_with(0, 0);
    }

    /// 流中 0x0A DecoderSync 携带的预测器 / 步进索引，作用于下一整帧。
    pub fn reset_with(&mut self, predictor: i16, step_index: u8) {
        self.predictor = predictor as i32;
        self.step_index = (step_index as i32).clamp(0, 88);
    }

    pub fn decode(&mut self, data: &[u8]) -> Vec<i16> {
        let mut samples = Vec::with_capacity(data.len() * 2);
        for byte in data {
            let (first, second) = if self.low_nibble_first {
                (byte & 0x0F, byte >> 4)
            } else {
                (byte >> 4, byte & 0x0F)
            };
            samples.push(self.decode_nibble(first as i32));
            samples.push(self.decode_nibble(second as i32));
        }
        samples
    }

    fn decode_nibble(&mut self, nibble: i32) -> i16 {
        let step = STEP_TABLE[self.step_index as usize];
        let mut diff = step >> 3;
        if nibble & 1 != 0 {
            diff += step >> 2;
        }
        if nibble & 2 != 0 {
            diff += step >> 1;
        }
        if nibble & 4 != 0 {
            diff += step;
        }
        self.predictor += if nibble & 8 != 0 { -diff } else { diff };
        self.predictor = self.predictor.clamp(i16::MIN as i32, i16::MAX as i32);
        self.step_index =
            (self.step_index + INDEX_TABLE[(nibble & 7) as usize]).clamp(0, 88);
        self.predictor as i16
    }

    pub fn encode(&mut self, samples: &[i16]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(samples.len().div_ceil(2));
        let chunks = samples.chunks(2);
        for pair in chunks {
            let hi = self.encode_nibble(pair[0]);
            let lo = pair.get(1).map(|&s| self.encode_nibble(s)).unwrap_or(0);
            bytes.push(if self.low_nibble_first {
                (lo << 4) | hi
            } else {
                (hi << 4) | lo
            });
        }
        bytes
    }

    fn encode_nibble(&mut self, sample: i16) -> u8 {
        let step = STEP_TABLE[self.step_index as usize];
        let target = sample as i32;
        let mut diff = target - self.predictor;
        let sign = if diff < 0 { 8u8 } else { 0u8 };
        if sign != 0 {
            diff = -diff;
        }
        let mut nibble = 0u8;
        // 从大到小贪心逼近，保证编码器与解码器的量化规则一致。
        let mut quant = diff;
        if quant >= step {
            nibble |= 4;
            quant -= step;
        }
        if quant >= step >> 1 {
            nibble |= 2;
            quant -= step >> 1;
        }
        if quant >= step >> 2 {
            nibble |= 1;
        }
        nibble |= sign;

        // 与解码端共用同一状态推进，保证往返一致性。
        let mut dec_diff = step >> 3;
        if nibble & 1 != 0 {
            dec_diff += step >> 2;
        }
        if nibble & 2 != 0 {
            dec_diff += step >> 1;
        }
        if nibble & 4 != 0 {
            dec_diff += step;
        }
        self.predictor += if nibble & 8 != 0 { -dec_diff } else { dec_diff };
        self.predictor = self.predictor.clamp(i16::MIN as i32, i16::MAX as i32);
        self.step_index =
            (self.step_index + INDEX_TABLE[(nibble & 7) as usize]).clamp(0, 88);
        nibble
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_static_reference_frame() {
        // 全 0x88：sign 位恒为 1、index 增量为 -1 → 向负向小步漂移。
        let mut codec = ImaAdpcmCodec::new();
        let samples = codec.decode(&[0x88, 0x88, 0x88, 0x88]);
        assert_eq!(samples.len(), 8);
        assert!(samples.windows(2).all(|w| w[0] >= w[1]));
        assert_eq!(codec.step_index(), 0);
    }

    #[test]
    fn nibble_order_swap_changes_decoding() {
        let mut a = ImaAdpcmCodec::new();
        let mut b = ImaAdpcmCodec::new();
        b.set_low_nibble_first(true);
        let bytes = [0x12, 0x9F, 0x36];
        let sa = a.decode(&bytes);
        let sb = b.decode(&bytes);
        // IMA 解码是顺序状态机：先解哪个半字节会改变后续全部样本，
        // 因此两种顺序产出的序列必然不同（首个样本即分叉）。
        assert_ne!(sa[0], sb[0]);
        assert_ne!(sa, sb);
    }

    #[test]
    fn encode_decode_roundtrip_works_in_both_orders() {
        let input: Vec<i16> = (0..400)
            .map(|i| ((i as f32 * 0.1).sin() * 6000.0) as i16)
            .collect();
        for low_first in [false, true] {
            let mut encoder = ImaAdpcmCodec::new();
            encoder.set_low_nibble_first(low_first);
            let encoded = encoder.encode(&input);
            let mut decoder = ImaAdpcmCodec::new();
            decoder.set_low_nibble_first(low_first);
            let decoded = decoder.decode(&encoded);
            let max_err = input
                .iter()
                .zip(decoded.iter())
                .skip(8)
                .map(|(a, b)| (a - b).unsigned_abs())
                .max()
                .unwrap();
            assert!(max_err < 1600, "low_first={low_first} max err {max_err}");
        }
    }

    #[test]
    fn encode_decode_roundtrip_tracks_input() {
        let mut codec = ImaAdpcmCodec::new();
        let input: Vec<i16> = (0..1600)
            .map(|i| ((i as f32 * 0.08).sin() * 8000.0) as i16)
            .collect();
        let encoded = codec.encode(&input);
        let mut decoder = ImaAdpcmCodec::new();
        let decoded = decoder.decode(&encoded);
        assert_eq!(decoded.len(), input.len());
        // ADPCM 有量化误差，但正弦波误差应远小于幅度。
        let max_err = input
            .iter()
            .zip(decoded.iter())
            .skip(8) // 跳过首个瞬态
            .map(|(a, b)| (a - b).unsigned_abs())
            .max()
            .unwrap();
        assert!(max_err < 1600, "max quantization error {max_err}");
    }

    #[test]
    fn decoder_sync_resets_state() {
        let mut codec = ImaAdpcmCodec::new();
        codec.decode(&[0x7F, 0x11, 0x22]);
        assert_ne!((codec.predictor(), codec.step_index()), (0, 0));
        codec.reset_with(-100, 12);
        assert_eq!(codec.predictor(), -100);
        assert_eq!(codec.step_index(), 12);
    }
}
