//! 探针：枚举渲染端点并逐一尝试按 audio.rs 同款参数打开，
//! 定位“CABLE Input（立体声）选不上”的具体错误。

use wasapi::{DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat};

fn main() {
    let _ = wasapi::initialize_mta();
    let enumerator = match DeviceEnumerator::new() {
        Ok(e) => e,
        Err(error) => {
            println!("DeviceEnumerator::new 失败：{error}");
            return;
        }
    };
    let collection = match enumerator.get_device_collection(&Direction::Render) {
        Ok(c) => c,
        Err(error) => {
            println!("get_device_collection 失败：{error}");
            return;
        }
    };
    println!("=== 渲染端点逐一打开测试 ===");
    for device in &collection {
        let device = match device {
            Ok(d) => d,
            Err(error) => {
                println!("枚举项错误：{error}");
                continue;
            }
        };
        let name = match device.get_friendlyname() {
            Ok(n) => n,
            Err(error) => {
                println!("取名字失败：{error}");
                continue;
            }
        };
        let candidate = voicehub_windows::is_virtual_cable_input_name(&name);
        let id = device.get_id().unwrap_or_default();
        // 与 audio.rs AudioSink::open 完全一致的打开路径。
        let result = (|| -> Result<(), String> {
            let mut client = device
                .get_iaudioclient()
                .map_err(|e| format!("get_iaudioclient: {e}"))?;
            let format = WaveFormat::new(16, 16, &SampleType::Int, 16_000, 1, None);
            let (default_period, _) = client
                .get_device_period()
                .map_err(|e| format!("get_device_period: {e}"))?;
            client
                .initialize_client(
                    &format,
                    &Direction::Render,
                    &StreamMode::PollingShared {
                        autoconvert: true,
                        buffer_duration_hns: default_period,
                    },
                )
                .map_err(|e| format!("initialize_client: {e}"))?;
            Ok(())
        })();
        let mark = candidate_mark(candidate);
        match result {
            Ok(()) => println!("  [OK]   {name}{mark}"),
            Err(error) => println!("  [FAIL] {name}{mark} → {error}"),
        }
        println!("         id={id}");
    }

    println!("=== 录音端点（capture）===");
    let collection = match enumerator.get_device_collection(&Direction::Capture) {
        Ok(c) => c,
        Err(error) => {
            println!("get_device_collection(capture) 失败：{error}");
            return;
        }
    };
    for device in &collection {
        let device = match device {
            Ok(d) => d,
            Err(error) => {
                println!("枚举项错误：{error}");
                continue;
            }
        };
        let name = device.get_friendlyname().unwrap_or_default();
        let id = device.get_id().unwrap_or_default();
        println!("  {name}");
        println!("         id={id}");
    }
    wasapi::deinitialize();
}

fn candidate_mark(candidate: bool) -> &'static str {
    if candidate {
        "  · CABLE 候选"
    } else {
        ""
    }
}
