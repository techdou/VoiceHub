//! ATVV（小米蓝牙遥控器语音通道）协议编解码。
//!
//! 协议事实经 techdou/remote-mic-app（Swift 实现）与
//! GetSayAll/remote-mic-app-windows（Rust 实现）交叉验证：
//! 服务 `AB5E0001-…` 下挂 transmit/audio/control 三个特征；
//! 控制通知以单字节 opcode 开头，音频通知为纯 ADPCM 字节流。

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// ATVV 服务与特征 UUID。
pub struct AtvvUuids;

impl AtvvUuids {
    pub const SERVICE: &'static str = "AB5E0001-5A21-4F05-BC7D-AF01F617B664";
    pub const TRANSMIT: &'static str = "AB5E0002-5A21-4F05-BC7D-AF01F617B664";
    pub const AUDIO: &'static str = "AB5E0003-5A21-4F05-BC7D-AF01F617B664";
    pub const CONTROL: &'static str = "AB5E0004-5A21-4F05-BC7D-AF01F617B664";
}

/// 标准电池 / 设备信息服务（型号识别、电量、电源状态）。
pub struct AuxUuids;

impl AuxUuids {
    pub const BATTERY_SERVICE: &'static str = "0000180F-0000-1000-8000-00805F9B34FB";
    pub const BATTERY_LEVEL: &'static str = "00002A19-0000-1000-8000-00805F9B34FB";
    pub const BATTERY_LEVEL_STATUS: &'static str = "00002BED-0000-1000-8000-00805F9B34FB";
    pub const DEVICE_INFORMATION: &'static str = "0000180A-0000-1000-8000-00805F9B34FB";
    pub const MODEL_NUMBER: &'static str = "00002A24-0000-1000-8000-00805F9B34FB";
}

/// control 特征通知解析结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AtvvControlEvent {
    /// 0x0B：能力响应。
    Capabilities(AtvvCapabilities),
    /// 0x08：遥控器请求开麦（按住语音键）。
    MicrophoneOpenRequested,
    /// 0x04：音频流开始。`session_id` 用于后续关麦 / 延长。
    StreamStarted {
        interaction: Option<u8>,
        codec: Option<u8>,
        session_id: u8,
    },
    /// 0x00：音频流结束。
    StreamStopped,
    /// 0x0A（设备发出）：解码器同步，作用于下一整帧。
    DecoderSync { predictor: i16, step_index: u8 },
    Unknown { opcode: u8 },
}

impl AtvvControlEvent {
    pub fn parse(bytes: &[u8]) -> Result<Self, AtvvError> {
        let opcode = *bytes.first().ok_or(AtvvError::PacketTooShort)?;
        match opcode {
            0x0B => Ok(Self::Capabilities(AtvvCapabilities::parse(bytes)?)),
            0x08 => Ok(Self::MicrophoneOpenRequested),
            0x04 => Ok(Self::StreamStarted {
                interaction: bytes.get(1).copied(),
                codec: bytes.get(2).copied(),
                session_id: bytes.get(3).copied().unwrap_or(0),
            }),
            0x00 => Ok(Self::StreamStopped),
            0x0A => {
                if bytes.len() < 7 {
                    return Err(AtvvError::PacketTooShort);
                }
                Ok(Self::DecoderSync {
                    predictor: i16::from_be_bytes([bytes[4], bytes[5]]),
                    step_index: bytes[6],
                })
            }
            _ => Ok(Self::Unknown { opcode }),
        }
    }
}

/// 主机 → 遥控器（写入 transmit 特征）的命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AtvvCommand {
    GetCapabilitiesV10,
    MicrophoneOpen { version: u16, codec: u8 },
    MicrophoneClose { version: u16, session_id: u8 },
    MicrophoneExtend { version: u16, session_id: u8 },
}

impl AtvvCommand {
    /// 编码为字节。`None` 表示该固件版本不支持此命令（如旧版无 Extend）。
    pub fn encode(&self) -> Option<Vec<u8>> {
        match *self {
            Self::GetCapabilitiesV10 => Some(vec![0x0A, 0x01, 0x00, 0x00, 0x03, 0x03]),
            Self::MicrophoneOpen { version, codec: _ } if version >= 0x0100 => Some(vec![0x0C, 0x00]),
            Self::MicrophoneOpen { codec, .. } => Some(vec![0x0C, 0x00, codec]),
            Self::MicrophoneClose { version, session_id } if version >= 0x0100 => {
                Some(vec![0x0D, session_id])
            }
            Self::MicrophoneClose { .. } => Some(vec![0x0D]),
            Self::MicrophoneExtend { version, session_id } if version >= 0x0100 => {
                Some(vec![0x0E, session_id])
            }
            Self::MicrophoneExtend { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtvvCapabilities {
    pub version: u16,
    pub codecs: u8,
    pub interaction: u8,
    pub frame_size: usize,
    pub selected_codec: u8,
    pub sample_rate: u32,
}

impl AtvvCapabilities {
    /// 解析 0x0B 能力响应。兼容旧固件（version < 0x0100 的 9 字节布局）
    /// 与“以 0x0100 版本号宣告但按旧布局填 codec”的混合固件。
    pub fn parse(bytes: &[u8]) -> Result<Self, AtvvError> {
        if bytes.len() < 7 {
            return Err(AtvvError::PacketTooShort);
        }
        if bytes[0] != 0x0B {
            return Err(AtvvError::UnexpectedMessage(bytes[0]));
        }

        let version = u16::from_be_bytes([bytes[1], bytes[2]]);
        let (mut codecs, mut interaction) = if version >= 0x0100 {
            (bytes[3], bytes[4])
        } else {
            if bytes.len() < 9 {
                return Err(AtvvError::PacketTooShort);
            }
            (bytes[4], 0)
        };

        // 混合固件：宣告 0x0100 却把 codec 位图放在旧偏移（bytes[4]）。
        if version >= 0x0100 && codecs == 0 && bytes.len() >= 9 && bytes[4] & 0x03 != 0 {
            codecs = bytes[4];
            interaction = 0x03;
        }

        let selected_codec = if codecs & 0x02 != 0 { 0x02 } else { 0x01 };
        let sample_rate = if selected_codec == 0x02 { 16_000 } else { 8_000 };
        let advertised_frame_size = u16::from_be_bytes([bytes[5], bytes[6]]) as usize;

        Ok(Self {
            version,
            codecs,
            interaction,
            frame_size: if advertised_frame_size == 0 {
                120
            } else {
                advertised_frame_size
            },
            selected_codec,
            sample_rate,
        })
    }

    pub fn supports_16k_audio(&self) -> bool {
        self.sample_rate == 16_000
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AtvvError {
    #[error("ATVV packet is too short")]
    PacketTooShort,
    #[error("unexpected ATVV message 0x{0:02X}")]
    UnexpectedMessage(u8),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_v1_capabilities_with_16k_codec() {
        let caps =
            AtvvCapabilities::parse(&[0x0B, 0x01, 0x00, 0x02, 0x03, 0x00, 120]).unwrap();
        assert_eq!(caps.version, 0x0100);
        assert_eq!(caps.selected_codec, 0x02);
        assert_eq!(caps.sample_rate, 16_000);
        assert_eq!(caps.frame_size, 120);
        assert!(caps.supports_16k_audio());
    }

    #[test]
    fn accepts_legacy_codec_layout_under_v1_banner() {
        let caps =
            AtvvCapabilities::parse(&[0x0B, 0x01, 0x00, 0x00, 0x02, 0x00, 120, 0, 0]).unwrap();
        assert_eq!(caps.selected_codec, 0x02);
        assert_eq!(caps.interaction, 0x03);
    }

    #[test]
    fn rejects_malformed_capabilities() {
        assert_eq!(AtvvCapabilities::parse(&[]), Err(AtvvError::PacketTooShort));
        assert_eq!(
            AtvvCapabilities::parse(&[0x00, 0x01, 0x00, 0x02, 0x03, 0x00, 120]),
            Err(AtvvError::UnexpectedMessage(0x00))
        );
    }

    #[test]
    fn default_frame_size_when_advertised_zero() {
        let caps =
            AtvvCapabilities::parse(&[0x0B, 0x01, 0x00, 0x02, 0x03, 0x00, 0x00]).unwrap();
        assert_eq!(caps.frame_size, 120);
    }

    #[test]
    fn encodes_version_specific_microphone_commands() {
        assert_eq!(
            AtvvCommand::MicrophoneOpen { version: 0x0100, codec: 2 }.encode(),
            Some(vec![0x0C, 0x00])
        );
        assert_eq!(
            AtvvCommand::MicrophoneOpen { version: 0x0001, codec: 2 }.encode(),
            Some(vec![0x0C, 0x00, 0x02])
        );
        assert_eq!(
            AtvvCommand::MicrophoneClose { version: 0x0100, session_id: 7 }.encode(),
            Some(vec![0x0D, 7])
        );
        assert_eq!(
            AtvvCommand::MicrophoneClose { version: 0x0001, session_id: 7 }.encode(),
            Some(vec![0x0D])
        );
        assert_eq!(
            AtvvCommand::MicrophoneExtend { version: 0x0100, session_id: 7 }.encode(),
            Some(vec![0x0E, 7])
        );
        assert_eq!(
            AtvvCommand::MicrophoneExtend { version: 0x0001, session_id: 7 }.encode(),
            None
        );
        assert_eq!(
            AtvvCommand::GetCapabilitiesV10.encode(),
            Some(vec![0x0A, 0x01, 0x00, 0x00, 0x03, 0x03])
        );
    }

    #[test]
    fn parses_stream_and_decoder_sync_events() {
        assert_eq!(
            AtvvControlEvent::parse(&[0x04, 0x03, 0x02, 0x07]).unwrap(),
            AtvvControlEvent::StreamStarted {
                interaction: Some(0x03),
                codec: Some(0x02),
                session_id: 0x07,
            }
        );
        assert_eq!(
            AtvvControlEvent::parse(&[0x0A, 0x00, 0x00, 0x00, 0xFF, 0x9C, 12]).unwrap(),
            AtvvControlEvent::DecoderSync { predictor: -100, step_index: 12 }
        );
        assert_eq!(
            AtvvControlEvent::parse(&[0x00]).unwrap(),
            AtvvControlEvent::StreamStopped
        );
        assert_eq!(
            AtvvControlEvent::parse(&[0x08]).unwrap(),
            AtvvControlEvent::MicrophoneOpenRequested
        );
        assert_eq!(
            AtvvControlEvent::parse(&[0x55]).unwrap(),
            AtvvControlEvent::Unknown { opcode: 0x55 }
        );
    }

    #[test]
    fn rejects_short_decoder_sync() {
        assert_eq!(
            AtvvControlEvent::parse(&[0x0A, 0x00, 0x00]),
            Err(AtvvError::PacketTooShort)
        );
    }
}
