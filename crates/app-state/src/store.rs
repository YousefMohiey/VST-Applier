//! Persistence under `%APPDATA%\VstApplier` (mirrors `VoiceSetupStore`
//! + `AppSettingsStore`). All reads are `try/catch -> default`: corrupt
//! JSON means a fresh start, never a crash.

use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::profile::{AppSettings, VoiceSetupProfile};

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// `%APPDATA%\VstApplier`, falling back to the legacy `SnjVoiceChanger`
/// folder content (copied once, best-effort, never deleted).
pub fn settings_dir() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("VstApplier");
    let legacy = base.join("SnjVoiceChanger");
    if !dir.exists() && legacy.exists() {
        let _ = copy_dir(&legacy, &dir);
    }
    dir
}

fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &to)?;
        } else {
            let _ = fs::copy(entry.path(), to);
        }
    }
    Ok(())
}

fn profiles_dir() -> PathBuf {
    settings_dir().join("profiles")
}

pub fn load_session() -> VoiceSetupProfile {
    let path = settings_dir().join("session.json");
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_session(profile: &VoiceSetupProfile) -> Result<()> {
    let dir = settings_dir();
    fs::create_dir_all(&dir)?;
    let text = serde_json::to_string_pretty(profile)?;
    fs::write(dir.join("session.json"), text)?;
    Ok(())
}

pub fn profile_names() -> Vec<String> {
    let dir = profiles_dir();
    let mut names = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("json") {
                if let Some(stem) = p.file_stem().and_then(|x| x.to_str()) {
                    names.push(stem.to_string());
                }
            }
        }
    }
    names.sort();
    names
}

pub fn load_profile(name: &str) -> Option<VoiceSetupProfile> {
    let path = profiles_dir().join(format!("{name}.json"));
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
}

pub fn save_profile(profile: &VoiceSetupProfile) -> Result<()> {
    let dir = profiles_dir();
    fs::create_dir_all(&dir)?;
    let text = serde_json::to_string_pretty(profile)?;
    fs::write(dir.join(format!("{}.json", profile.Name)), text)?;
    Ok(())
}

pub fn delete_profile(name: &str) -> Result<()> {
    let path = profiles_dir().join(format!("{name}.json"));
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

/// Mirrors `IsValidProfileName`: max 48 chars, no path separators or
/// reserved names.
pub fn is_valid_profile_name(name: &str) -> bool {
    let t = name.trim();
    if t.is_empty() || t.len() > 48 {
        return false;
    }
    if t == "." || t == ".." {
        return false;
    }
    let bad = ['/', '\\', ':', '*', '?', '"', '<', '>', '|'];
    !t.chars().any(|c| bad.contains(&c) || c.is_control())
}

pub fn load_settings() -> AppSettings {
    let path = settings_dir().join("settings.json");
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_settings(settings: &AppSettings) -> Result<()> {
    let dir = settings_dir();
    fs::create_dir_all(&dir)?;
    let text = serde_json::to_string_pretty(settings)?;
    fs::write(dir.join("settings.json"), text)?;
    Ok(())
}
