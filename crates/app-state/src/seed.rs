//! First-run defaults seeding (mirrors `DefaultSetupSeeder`).
//!
//! Seeds only when NO `settings.json` AND NO `profiles/*.json` exist —
//! existing users are never touched. Copies `<exe-dir>/defaults/*` into
//! `%APPDATA%\VstApplier`, replacing the `{APP}` placeholder with the
//! install dir (backslashes JSON-escaped).

use std::fs;
use std::path::{Path, PathBuf};

use crate::store::settings_dir;

pub fn ensure_seeded(exe_dir: &Path) {
    let target = settings_dir();
    let has_settings = target.join("settings.json").exists();
    let has_profiles = target
        .join("profiles")
        .read_dir()
        .map(|mut r| r.any(|e| e.ok().map(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json")).unwrap_or(false)))
        .unwrap_or(false);
    if has_settings || has_profiles {
        return;
    }
    let defaults = exe_dir.join("defaults");
    if !defaults.is_dir() {
        return;
    }
    let app_dir = exe_dir.to_string_lossy().replace('\\', "\\\\");
    let _ = seed_dir(&defaults, &target, &app_dir);
}

fn seed_dir(src: &Path, dst: &Path, app_escaped: &str) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            seed_dir(&entry.path(), &to, app_escaped)?;
        } else {
            match fs::read_to_string(entry.path()) {
                Ok(text) => {
                    let replaced = text.replace("{APP}", app_escaped);
                    fs::write(to, replaced)?;
                }
                Err(_) => {
                    // Binary file: copy verbatim.
                    let _ = fs::copy(entry.path(), to);
                }
            }
        }
    }
    Ok(())
}

/// Default plugin folder: `<exe>/Plugins`, else `<exe>/common/VST`,
/// else `<exe>/Plugins` (created on demand by the user).
pub fn default_plugin_folder(exe_dir: &Path) -> PathBuf {
    let plugins = exe_dir.join("Plugins");
    if plugins.is_dir() {
        return plugins;
    }
    let common = exe_dir.join("common").join("VST");
    if common.is_dir() {
        return common;
    }
    plugins
}
