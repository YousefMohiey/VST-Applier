//! Start-with-Windows via `HKCU\...\Run` (mirrors `AutoStartManager`).
//!
//! Value: `VST-Applier = "<exe>" --tray`. Rewritten on every launch while
//! enabled so a moved exe heals itself; failures are swallowed.

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "VST-Applier";

pub fn apply(enabled: bool) {
    if enabled {
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let value = format!("\"{}\" --tray", exe.display());
        if let Ok(key) = windows_registry::CURRENT_USER.create(RUN_KEY) {
            let _ = key.set_string(VALUE_NAME, &value);
        }
    } else if let Ok(key) = windows_registry::CURRENT_USER.create(RUN_KEY) {
        let _ = key.remove_value(VALUE_NAME);
    }
}
