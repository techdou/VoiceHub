//! 探针：区分“同线 pin 互斥”与“释放后拆线延迟”两种失败模式。
//! 实验矩阵：
//!   X1: 持有 2ch 时开 16ch
//!   X2: 持有 16ch 时开 2ch
//!   X3: 持有 16ch → drop → 立刻开 2ch（复刻声桥切换路径）
//!   X4: X3 失败时的重试轮数（200ms 间隔）

use wasapi::{
    AudioClient, DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat,
};

struct Sink {
    _client: AudioClient,
}

fn open_endpoint(enumerator: &DeviceEnumerator, name_part: &str) -> Result<Sink, String> {
    let collection = enumerator
        .get_device_collection(&Direction::Render)
        .map_err(|e| format!("{e}"))?;
    for device in &collection {
        let device = device.map_err(|e| format!("{e}"))?;
        let name = device.get_friendlyname().map_err(|e| format!("{e}"))?;
        if !name.contains(name_part) {
            continue;
        }
        let mut client = device.get_iaudioclient().map_err(|e| format!("{e}"))?;
        let format = WaveFormat::new(16, 16, &SampleType::Int, 16_000, 1, None);
        let (default_period, _) = client.get_device_period().map_err(|e| format!("{e}"))?;
        client
            .initialize_client(
                &format,
                &Direction::Render,
                &StreamMode::PollingShared {
                    autoconvert: true,
                    buffer_duration_hns: default_period,
                },
            )
            .map_err(|e| format!("{e}"))?;
        return Ok(Sink { _client: client });
    }
    Err(format!("未找到含 “{name_part}” 的端点"))
}

fn main() {
    let _ = wasapi::initialize_mta();
    let enumerator = DeviceEnumerator::new().expect("enumerator");
    const STEREO: &str = "CABLE Input (";
    const CH16: &str = "CABLE In 16ch";

    // X1: 持有 2ch 时开 16ch。
    let stereo = open_endpoint(&enumerator, STEREO);
    println!("打开 2ch: {}", verdict(&stereo));
    let hold_16_while_stereo = match stereo.as_ref() {
        Ok(_) => open_endpoint(&enumerator, CH16),
        Err(e) => Err(format!("2ch没开成：{e}")),
    };
    println!("X1 持有2ch开16ch: {}", verdict(&hold_16_while_stereo));
    drop(stereo);
    drop(hold_16_while_stereo);
    std::thread::sleep(std::time::Duration::from_millis(500));

    // X2: 持有 16ch 时开 2ch。
    let ch16 = open_endpoint(&enumerator, CH16);
    println!("打开 16ch: {}", verdict(&ch16));
    let open_stereo_while_16 = match ch16.as_ref() {
        Ok(_) => open_endpoint(&enumerator, STEREO),
        Err(e) => Err(format!("16ch没开成：{e}")),
    };
    println!("X2 持有16ch开2ch: {}", verdict(&open_stereo_while_16));
    drop(open_stereo_while_16);

    // X3: 复刻声桥切换路径——drop 16ch 后立刻开 2ch。
    drop(ch16);
    let immediate = open_endpoint(&enumerator, STEREO);
    println!("X3 drop后立刻开2ch: {}", verdict(&immediate));
    drop(immediate);

    // X4: 若 X3 失败，重试几轮能成。
    let ch16b = open_endpoint(&enumerator, CH16);
    println!("重开 16ch: {}", verdict(&ch16b));
    drop(ch16b);
    for attempt in 1..=10 {
        match open_endpoint(&enumerator, STEREO) {
            Ok(sink) => {
                println!("X4 第 {attempt} 轮(200ms×)后 2ch 打开成功");
                drop(sink);
                break;
            }
            Err(_) => {
                println!("X4 第 {attempt} 轮失败，200ms 后重试");
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        }
    }
    wasapi::deinitialize();
}

fn verdict<T>(result: &Result<T, String>) -> String {
    match result {
        Ok(_) => "OK".into(),
        Err(e) => format!("FAIL → {e}"),
    }
}
