//! Setup schemas (mirrors `VoiceSetupProfile` + `AppSettings`).
//!
//! The JSON field names intentionally match the C# app so profiles and
//! settings stay cross-compatible:
//! `%APPDATA%\VstApplier\{session.json, profiles\<name>.json, settings.json}`.

#![allow(non_snake_case)]
//! Field names intentionally match the C# JSON schema (cross-compatible
//! profiles), hence non-snake-case.

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceSetupPluginEntry {
    #[serde(default)]
    pub Name: String,
    #[serde(default)]
    pub Path: String,
    #[serde(default = "default_format")]
    pub Format: String,
    #[serde(default = "default_true")]
    pub Enabled: bool,
    #[serde(default)]
    pub State: Option<String>,
}

fn default_format() -> String {
    "Vst2".into()
}
fn default_true() -> bool {
    true
}

impl VoiceSetupPluginEntry {
    pub fn is_vst3(&self) -> bool {
        self.Format.eq_ignore_ascii_case("Vst3")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceSetupProfile {
    #[serde(default = "schema_v1")]
    pub SchemaVersion: u32,
    #[serde(default)]
    pub Name: String,
    #[serde(default)]
    pub ActiveProfile: String,
    #[serde(default)]
    pub PluginFolder: String,
    #[serde(default)]
    pub InputDeviceId: String,
    #[serde(default)]
    pub InputDeviceName: String,
    #[serde(default)]
    pub OutputDeviceId: String,
    #[serde(default)]
    pub OutputDeviceName: String,
    #[serde(default = "default_buffer")]
    pub BufferSize: u32,
    #[serde(default)]
    pub Plugins: Vec<VoiceSetupPluginEntry>,
}

fn schema_v1() -> u32 {
    1
}
fn default_buffer() -> u32 {
    1024
}

impl Default for VoiceSetupProfile {
    fn default() -> Self {
        VoiceSetupProfile {
            SchemaVersion: 1,
            Name: String::new(),
            ActiveProfile: String::new(),
            PluginFolder: String::new(),
            InputDeviceId: String::new(),
            InputDeviceName: String::new(),
            OutputDeviceId: String::new(),
            OutputDeviceName: String::new(),
            BufferSize: 1024,
            Plugins: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default = "default_true")]
    pub AutoStartMicrophone: bool,
    #[serde(default = "default_true")]
    pub CloseToTray: bool,
    #[serde(default)]
    pub StartWithWindows: bool,
    #[serde(default = "default_true")]
    pub KeepCableClean: bool,
    /// UI theme: "dark", "light" or "system". Fresh installs default to
    /// "system" (follow the Windows app theme); the user's pick is saved
    /// the moment they change it.
    #[serde(default = "default_theme")]
    pub Theme: String,
    /// Check GitHub Releases for a newer build (startup + manual).
    #[serde(default = "default_true")]
    pub CheckUpdates: bool,
    /// Built-in AI denoise (DeepFilterNet3) on the microphone input,
    /// applied before the plugin chain. Default off (CPU cost).
    #[serde(default)]
    pub AiDenoise: bool,
}

fn default_theme() -> String {
    // First use: follow the Windows app theme.
    "system".to_string()
}

impl Default for AppSettings {
    fn default() -> Self {
        AppSettings {
            AutoStartMicrophone: true,
            CloseToTray: true,
            StartWithWindows: false,
            KeepCableClean: true,
            Theme: "system".to_string(),
            CheckUpdates: true,
            AiDenoise: false,
        }
    }
}
