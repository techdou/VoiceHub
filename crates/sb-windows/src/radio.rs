//! 蓝牙无线电自愈（僵死 GATT 链路恢复）。
//!
//! 应用被强杀后 Windows 可能残留僵死链路：FromIdAsync 仍返回对象但
//! CCCD 订阅写入 E_ABORT，普通重试永远失败；公开 API 中只有关开蓝牙
//! 无线电能触达 OS 侧僵死（参考实现真机取证结论）。
//! 仅在自动重连循环里、连续失败达阈值且未超上限时调用。

use std::future::IntoFuture;
use std::time::Duration;

use windows::Devices::Radios::{Radio, RadioKind, RadioState};

/// 连续失败多少次后触发一次无线电恢复（默认退避 2/4/8/16/30s 下
/// 第 5 次失败约 60 秒后——覆盖常规瞬时失败又不让用户久等）。
pub const RADIO_RECOVERY_AFTER_FAILURES: u32 = 5;
/// 每个僵死周期最多恢复次数：超过后停止抖动，由 UI 提示人工介入。
pub const RADIO_RECOVERY_MAX_CYCLES: u32 = 2;
const RADIO_OFF_HOLD: Duration = Duration::from_secs(2);

/// 纯决策：是否应触发无线电恢复（单测覆盖）。
pub fn should_cycle(consecutive_failures: u32, cycles_done: u32) -> bool {
    consecutive_failures >= RADIO_RECOVERY_AFTER_FAILURES && cycles_done < RADIO_RECOVERY_MAX_CYCLES
}

fn find_bluetooth_radio() -> windows::core::Result<Option<Radio>> {
    let operation = Radio::GetRadiosAsync()?;
    let radios = futures::executor::block_on(operation.into_future())?;
    let count = radios.Size()?;
    for index in 0..count {
        let radio = radios.GetAt(index)?;
        if radio.Kind()? == RadioKind::Bluetooth {
            return Ok(Some(radio));
        }
    }
    Ok(None)
}

fn set_state(radio: &Radio, state: RadioState) -> windows::core::Result<RadioState> {
    let operation = radio.SetStateAsync(state)?;
    let _access = futures::executor::block_on(operation.into_future())?;
    radio.State()
}

/// 关开一次蓝牙无线电（阻塞约 2–4 秒，在 BLE 工作线程的重连间歇调用）。
/// Err 时已尽力恢复打开；调用方把错误写入状态提示人工介入。
pub fn cycle_bluetooth_radio() -> Result<(), String> {
    let Some(radio) = find_bluetooth_radio().map_err(|e| format!("枚举蓝牙无线电失败：{e}"))? else {
        return Err("未找到蓝牙无线电".to_owned());
    };

    match set_state(&radio, RadioState::Off) {
        Ok(state) if state == RadioState::Off => {}
        Ok(state) => return Err(format!("关闭蓝牙无线电未生效（当前 {state:?}）")),
        Err(error) => return Err(format!("关闭蓝牙无线电失败：{error}")),
    }
    std::thread::sleep(RADIO_OFF_HOLD);

    match set_state(&radio, RadioState::On) {
        Ok(state) if state == RadioState::On => Ok(()),
        _ => {
            std::thread::sleep(Duration::from_secs(1));
            match set_state(&radio, RadioState::On) {
                Ok(state) if state == RadioState::On => Ok(()),
                Ok(state) => Err(format!("蓝牙无线电恢复打开未确认（当前 {state:?}），请手动打开蓝牙")),
                Err(error) => Err(format!("蓝牙无线电恢复打开失败：{error}，请手动打开蓝牙")),
            }
        }
    }
}

/// 蓝牙无线电是否存在（诊断用）。
pub fn probe_radio() -> Result<bool, String> {
    match find_bluetooth_radio() {
        Ok(Some(_)) => Ok(true),
        Ok(None) => Ok(false),
        Err(error) => Err(format!("{error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycles_only_after_threshold_and_within_cap() {
        assert!(!should_cycle(0, 0));
        assert!(!should_cycle(4, 0));
        assert!(should_cycle(5, 0));
        assert!(should_cycle(9, 1));
        assert!(!should_cycle(5, 2));
        assert!(!should_cycle(30, 2));
    }
}
