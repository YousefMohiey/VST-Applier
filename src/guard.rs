//! Cable session guard (mirrors `CableSessionGuard`).
//!
//! Background thread, 2 s poll. Only acts when the selected output looks
//! like a virtual cable (`cable`/`virtual`/`voicemeeter` in the name).
//! Foreign sessions (any PID except our own) that are not already muted by
//! the user get muted so app audio never leaks into the mic; everything we
//! mute is restored on disable / device switch / exit.

use std::collections::HashSet;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

/// Score render-device names like `VirtualCableService` and report whether
/// the cable looks ready. Returns `(ready, message)`.
pub fn cable_status(outputs: &[&str]) -> (bool, String) {
    let score = |n: &str| -> i32 {
        if n == "CABLE Input" {
            100
        } else if n.starts_with("CABLE Input") {
            if n.contains("16ch") { 70 } else { 90 }
        } else if n.contains("VB-Audio Virtual Cable") {
            40
        } else {
            0
        }
    };
    let best = outputs.iter().map(|n| score(n)).max().unwrap_or(0);
    if best >= 90 {
        (true, "Virtual cable ready".into())
    } else if best > 0 {
        (false, "Virtual cable incomplete — check VB-CABLE".into())
    } else {
        (false, "Install VB-CABLE for routing".into())
    }
}

fn is_cable_name(name: &str) -> bool {
    let n = name.to_lowercase();
    n.contains("cable") || n.contains("virtual") || n.contains("voicemeeter")
}

pub struct CableGuard {
    enabled: Arc<AtomicBool>,
    output: Arc<Mutex<String>>,
    stop: Arc<AtomicBool>,
    _worker: Option<std::thread::JoinHandle<()>>,
}

impl CableGuard {
    pub fn new() -> Self {
        let enabled = Arc::new(AtomicBool::new(true));
        let output = Arc::new(Mutex::new(String::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let worker = {
            let enabled = enabled.clone();
            let output = output.clone();
            let stop = stop.clone();
            std::thread::spawn(move || worker_loop(enabled, output, stop))
        };
        CableGuard { enabled, output, stop, _worker: Some(worker) }
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
    }

    pub fn set_output(&self, name: &str) {
        // Called every UI frame: lock but don't allocate unless changed.
        let mut out = self.output.lock().unwrap();
        if out.as_str() != name {
            *out = name.to_string();
        }
    }
}

impl Drop for CableGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn worker_loop(
    enabled: Arc<AtomicBool>,
    output: Arc<Mutex<String>>,
    stop: Arc<AtomicBool>,
) {
    let mut muted: HashSet<String> = HashSet::new();
    let mut last_device = String::new();
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(2000));
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let dev = output.lock().unwrap().clone();
        // Fallible COM work lives in `tick`; failures are swallowed and
        // retried next tick (as in C#).
        if dev != last_device {
            restore_all(&mut muted);
            last_device = dev.clone();
        }
        if !enabled.load(Ordering::Relaxed) {
            restore_all(&mut muted);
            continue;
        }
        if !is_cable_name(&dev) {
            restore_all(&mut muted);
            continue;
        }
        // Self-heal every tick: if anything re-muted us, undo it.
        unmute_own_sessions(Some(&dev));
        guard_device(&dev, &mut muted);
    }
    restore_all(&mut muted);
}

fn restore_all(muted: &mut HashSet<String>) {
    if muted.is_empty() {
        return;
    }
    let ids: Vec<String> = muted.drain().collect();
    for_each_session(None, &mut |instance_id, vol| {
        if ids.contains(&instance_id) {
            unsafe {
                let _ = vol.SetMute(windows::Win32::Foundation::FALSE, std::ptr::null());
            }
        }
    });
}

fn guard_device(dev_name: &str, muted: &mut HashSet<String>) {
    for_each_session(Some(dev_name), &mut |instance_id, vol| {
        unsafe {
            if let Ok(mute) = vol.GetMute() {
                if mute.as_bool() {
                    return; // User-muted: leave alone.
                }
            }
            if vol
                .SetMute(windows::Win32::Foundation::TRUE, std::ptr::null())
                .is_ok()
            {
                muted.insert(instance_id);
            }
        }
    });
}

use windows::Win32::Media::Audio::{
    eCapture, eRender, IAudioSessionControl, IAudioSessionControl2, IAudioSessionEnumerator,
    IAudioSessionManager2, IMMDevice, IMMDeviceCollection, IMMDeviceEnumerator,
    ISimpleAudioVolume, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ,
};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::UI::Shell::PropertiesSystem::{PDFF_DEFAULT, PSFormatForDisplay};
use windows::core::Interface;

/// Iterate `(instance_id, volume)` over sessions of render devices.
/// `wanted = Some(name)` restricts to the device with that friendly name;
/// `None` visits every render device. Skips our own PID and PID 0.
pub(crate) fn for_each_session(wanted: Option<&str>, f: &mut dyn FnMut(String, ISimpleAudioVolume)) {
    unsafe {
        if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() {
            return;
        }
        let result = (|| -> windows::core::Result<()> {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let collection: IMMDeviceCollection =
                enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
            let count = collection.GetCount()?;
            for i in 0..count {
                let device: IMMDevice = collection.Item(i)?;
                let name = friendly_name(&device).unwrap_or_default();
                if wanted.map(|w| w != name).unwrap_or(false) {
                    continue;
                }
                let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None)?;
                let sessions: IAudioSessionEnumerator = manager.GetSessionEnumerator()?;
                let scount = sessions.GetCount()?;
                for j in 0..scount {
                    let ctl: IAudioSessionControl = match sessions.GetSession(j as i32) {
                        Ok(c) => c,
                        Err(_) => continue,
                    };
                    let c2: IAudioSessionControl2 = match ctl.cast() {
                        Ok(c) => c,
                        Err(_) => continue,
                    };
                    let pid = c2.GetProcessId().unwrap_or(0);
                    let my_pid = windows::Win32::System::Threading::GetCurrentProcessId();
                    if pid == my_pid || pid == 0 {
                        continue;
                    }
                    let vol: ISimpleAudioVolume = match ctl.cast() {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    let instance_id = c2
                        .GetSessionInstanceIdentifier()
                        .ok()
                        .and_then(|s| s.to_string().ok())
                        .unwrap_or_default();
                    f(instance_id, vol);
                }
            }
            Ok(())
        })();
        let _ = result;
        CoUninitialize();
    }
}

/// Headless mixer inspection: prints every render session (pid, instance id,
/// mute, level) plus endpoint master states. Used by `--sessions`.
pub(crate) fn dump_sessions() {
    let my_pid = unsafe { windows::Win32::System::Threading::GetCurrentProcessId() };
    println!("own pid: {my_pid}");
    // Capture endpoint masters (mic mute/level lives here).
    unsafe {
        if CoInitializeEx(None, COINIT_MULTITHREADED).is_ok() {
            if let Ok(enumerator) = CoCreateInstance::<Option<&windows::core::IUnknown>, IMMDeviceEnumerator>(
                &MMDeviceEnumerator,
                None,
                CLSCTX_ALL,
            ) {
                if let Ok(collection) = enumerator.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE) {
                    if let Ok(count) = collection.GetCount() {
                        for i in 0..count {
                            let Ok(device) = collection.Item(i) else { continue };
                            let name = friendly_name(&device).unwrap_or_default();
                            let master = device
                                .Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)
                                .ok()
                                .and_then(|v| {
                                    Some((
                                        v.GetMute().ok()?.as_bool(),
                                        v.GetMasterVolumeLevelScalar().unwrap_or(-1.0),
                                    ))
                                });
                            println!("capture '{name}': master={master:?}");
                        }
                    }
                }
            }
            CoUninitialize();
        }
    }
    // Endpoint masters.
    unsafe {
        if CoInitializeEx(None, COINIT_MULTITHREADED).is_ok() {
            if let Ok(enumerator) = CoCreateInstance::<Option<&windows::core::IUnknown>, IMMDeviceEnumerator>(
                &MMDeviceEnumerator,
                None,
                CLSCTX_ALL,
            ) {
                if let Ok(collection) = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) {
                    if let Ok(count) = collection.GetCount() {
                        for i in 0..count {
                            let Ok(device) = collection.Item(i) else { continue };
                            let name = friendly_name(&device).unwrap_or_default();
                            let master = device
                                .Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)
                                .ok()
                                .and_then(|v| {
                                    Some((
                                        v.GetMute().ok()?.as_bool(),
                                        v.GetMasterVolumeLevelScalar().unwrap_or(-1.0),
                                    ))
                                });
                            println!("endpoint '{name}': master={master:?}");
                        }
                    }
                }
            }
            CoUninitialize();
        }
    }
    // Sessions per endpoint (all devices).
    for_each_session_with_pid(&mut |ep, pid, iid, mute, level| {
        let mine = if pid == my_pid { "  <-- US" } else { "" };
        println!("session on '{ep}': pid={pid} mute={mute} level={level:.2} id={iid}{mine}");
    });
}

/// Walk every render session, invoking `f` with (endpoint friendly name,
/// owner PID, volume control). No filtering — callers decide what matches.
fn walk_sessions(f: &mut dyn FnMut(String, u32, ISimpleAudioVolume)) {
    unsafe {
        if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() {
            return;
        }
        let result = (|| -> windows::core::Result<()> {
            use windows::core::Interface;
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let collection: IMMDeviceCollection =
                enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
            let count = collection.GetCount()?;
            for i in 0..count {
                let device: IMMDevice = collection.Item(i)?;
                let name = friendly_name(&device).unwrap_or_default();
                let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None)?;
                let sessions: IAudioSessionEnumerator = manager.GetSessionEnumerator()?;
                let scount = sessions.GetCount()?;
                for j in 0..scount {
                    let ctl: IAudioSessionControl = match sessions.GetSession(j as i32) {
                        Ok(c) => c,
                        Err(_) => continue,
                    };
                    let pid = ctl
                        .cast::<IAudioSessionControl2>()
                        .and_then(|c2| c2.GetProcessId())
                        .unwrap_or(u32::MAX);
                    let vol: ISimpleAudioVolume = match ctl.cast() {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    f(name.clone(), pid, vol);
                }
            }
            Ok(())
        })();
        let _ = result;
        CoUninitialize();
    }
}

/// Unmute our own sessions on the given render device (or all render
/// devices when `wanted` is None).
///
/// Why this exists: Windows persists per-app session mute state per
/// endpoint. During the old-vs-new guard war both apps muted each other,
/// and those mutes stick to *future* sessions of the same exe on the same
/// endpoint — a permanently self-silenced app that looks perfectly healthy
/// in every meter. Routing must audibly mean "unmute me".
pub(crate) fn unmute_own_sessions(wanted: Option<&str>) {
    let my_pid = unsafe { windows::Win32::System::Threading::GetCurrentProcessId() };
    walk_sessions(&mut |ep, pid, vol| {
        if pid != my_pid {
            return;
        }
        if wanted.map(|w| w != ep).unwrap_or(false) {
            return;
        }
        unsafe {
            let _ = vol.SetMute(windows::Win32::Foundation::FALSE, std::ptr::null());
        }
    });
}

/// Unmute sessions owned by processes named `exe` (e.g. repair the old
/// app's persisted mute after a guard war). Returns sessions unmuted.
/// No-op when no such process runs.
pub(crate) fn unmute_exe(exe: &str, wanted: Option<&str>) -> usize {
    let pids = crate::conflict::pids_of(exe);
    if pids.is_empty() {
        return 0;
    }
    let mut n = 0;
    walk_sessions(&mut |ep, pid, vol| {
        if !pids.contains(&pid) {
            return;
        }
        if wanted.map(|w| w != ep).unwrap_or(false) {
            return;
        }
        unsafe {
            if vol
                .SetMute(windows::Win32::Foundation::FALSE, std::ptr::null())
                .is_ok()
            {
                n += 1;
            }
        }
    });
    n
}

/// Like `for_each_session` but also reports the endpoint name and owning PID
/// (including our own sessions and PID 0).
pub(crate) fn for_each_session_with_pid(
    f: &mut dyn FnMut(String, u32, String, bool, f32),
) {
    use windows::core::Interface;
    unsafe {
        if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() {
            return;
        }
        let result = (|| -> windows::core::Result<()> {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let collection: IMMDeviceCollection =
                enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
            let count = collection.GetCount()?;
            for i in 0..count {
                let device: IMMDevice = collection.Item(i)?;
                let name = friendly_name(&device).unwrap_or_default();
                let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None)?;
                let sessions: IAudioSessionEnumerator = manager.GetSessionEnumerator()?;
                let scount = sessions.GetCount()?;
                for j in 0..scount {
                    let ctl: IAudioSessionControl = match sessions.GetSession(j as i32) {
                        Ok(c) => c,
                        Err(_) => continue,
                    };
                    let c2: IAudioSessionControl2 = match ctl.cast() {
                        Ok(c) => c,
                        Err(_) => continue,
                    };
                    let pid = c2.GetProcessId().unwrap_or(0);
                    let vol: ISimpleAudioVolume = match ctl.cast() {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    let iid = c2
                        .GetSessionInstanceIdentifier()
                        .ok()
                        .and_then(|s| s.to_string().ok())
                        .unwrap_or_default();
                    let mute = vol.GetMute().map(|m| m.as_bool()).unwrap_or(false);
                    let level = vol.GetMasterVolume().unwrap_or(-1.0);
                    f(name.clone(), pid, iid, mute, level);
                }
            }
            Ok(())
        })();
        let _ = result;
        CoUninitialize();
    }
}
fn friendly_name(device: &IMMDevice) -> windows::core::Result<String> {
    unsafe {
        let store = device.OpenPropertyStore(STGM_READ)?;
        let prop = store.GetValue(&PKEY_Device_FriendlyName)?;
        // Display-format the value instead of picking the PROPVARIANT union
        // apart by hand; works for any variant type the driver returns.
        let mut buf = [0u16; 256];
        PSFormatForDisplay(&PKEY_Device_FriendlyName, &prop, PDFF_DEFAULT, &mut buf)?;
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Ok(String::from_utf16_lossy(&buf[..len]))
    }
}
