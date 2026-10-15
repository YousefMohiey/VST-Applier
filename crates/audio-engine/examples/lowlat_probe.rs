//! Probe: what low-latency shared-mode periods do the devices actually
//! support? Reports `GetSharedModeEnginePeriod` (IAudioClient3) per active
//! endpoint — the decisive number for whether a low-latency backend can
//! beat the Windows default 10 ms period on this machine.
//!
//! Usage: cargo run -q -p audio-engine --example lowlat_probe
use windows::core::Interface;
use windows::Win32::Media::Audio::{
    eCapture, eRender, EDataFlow, IAudioClient3, IMMDeviceEnumerator, MMDeviceEnumerator,
    DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED,
};

unsafe fn probe(dir: EDataFlow, label: &str) {
    let enumerator: IMMDeviceEnumerator =
        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).unwrap();
    let coll = enumerator.EnumAudioEndpoints(dir, DEVICE_STATE_ACTIVE).unwrap();
    let n = coll.GetCount().unwrap();
    println!("== {label}: {n} active endpoint(s)");
    for i in 0..n {
        let dev = coll.Item(i).unwrap();
        let id = dev
            .GetId()
            .ok()
            .map(|p| p.to_string().unwrap_or_default())
            .unwrap_or_default();
        let short = id
            .trim_start_matches("{0.0.0.00000000}.")
            .to_string();
        match dev.Activate::<IAudioClient3>(CLSCTX_ALL, None) {
            Ok(client3) => {
                match client3.GetMixFormat() {
                    Ok(pwfx) => {
                        let (sr, ch) = unsafe { ((*pwfx).nSamplesPerSec, (*pwfx).nChannels) };
                        let (mut def, mut fund, mut min, mut max) = (0u32, 0u32, 0u32, 0u32);
                        let hr = unsafe {
                            client3.GetSharedModeEnginePeriod(
                                pwfx, &mut def, &mut fund, &mut min, &mut max,
                            )
                        };
                        let min_ms = 1000.0 * min as f64 / sr as f64;
                        let def_ms = 1000.0 * def as f64 / sr as f64;
                        println!(
                            "  [{i}] sr={sr} ch={ch} :: periods frames(default={def} min={min} max={max}) \
                             => default {def_ms:.2} ms, min {min_ms:.2} ms  (hr={hr:?})  id…{short}"
                        );
                        CoTaskMemFree(Some(pwfx as *const _ as *const core::ffi::c_void));
                    }
                    Err(e) => println!("  [{i}] mix format failed: {e}"),
                }
            }
            Err(e) => println!("  [{i}] IAudioClient3 activate failed: {e}"),
        }
    }
}

fn main() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        probe(eRender, "OUTPUT");
        probe(eCapture, "INPUT");
        let _ = IAudioClient3::IID; // keep the interface import used
    }
}
