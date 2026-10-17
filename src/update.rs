//! Self-update from GitHub Releases (mirrors an auto-updater that the C#
//! app never had). No installer: the release asset is a zip holding
//! `vst-applier.exe` + the two native host DLLs + the bundled plugin,
//! and install swaps those files next to the running exe (the plugin
//! goes to `Plugins\VST-Applier\`), then restarts.
//!
//! Flow (all network/disk work on worker threads, never the UI thread):
//! `Check` -> compare tag with `APP_VERSION` -> `Download` the zip asset
//! into a staging dir -> `Install`: write `updater.bat`, spawn it
//! detached, exit. The batch waits for our PID to vanish (exe is locked
//! while running), copies the staged files over, relaunches, cleans up.

use std::path::PathBuf;

/// Baked-in version of this binary (`Cargo.toml`).
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Where releases live. Same repo the user publishes (owner/repo).
pub const UPDATE_OWNER: &str = "YousefMohiey";
pub const UPDATE_REPO: &str = "VST-Applier";

/// Files the release zip MUST contain (app + native host DLLs).
const PAYLOAD: &[&str] = &["vst-applier.exe", "Vst2HostNative.dll", "Vst3HostNative.dll"];

/// Bundled plugin entry in the zip (optional: an app-only zip still
/// updates the app, just not the plugin — logged, not an error).
const PLUGIN_FILE: &str = "Yousef Noise Suppression.dll";

/// Minimal file log for update progress. The app has its own logger,
/// but the CLI example harnesses include this module standalone (no
/// `crate::log_line` there), so we write the same rust-log.txt directly.
fn log_hint(msg: &str) {
    use std::io::Write;
    let path = app_state::settings_dir().join("rust-log.txt");
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "[{ts}] {msg}");
    }
}

/// True when the staged download carries the bundled plugin payload.
pub fn staged_has_plugin(stage: &std::path::Path) -> bool {
    stage.join(PLUGIN_FILE).exists()
}

#[derive(Debug, Clone)]
pub struct ReleaseInfo {
    pub tag: String,
    pub name: String,
    pub zip_url: String,
    pub zip_name: String,
}

fn api_url() -> String {
    format!("https://api.github.com/repos/{UPDATE_OWNER}/{UPDATE_REPO}/releases/latest")
}

/// Numeric `v1.2.3` / `1.2.3` compare. Anything unparsable counts as
/// "not newer" (never nag on weird tags).
pub fn tag_is_newer(tag: &str) -> bool {
    fn parts(s: &str) -> Option<[u64; 3]> {
        let s = s.strip_prefix('v').unwrap_or(s);
        // Ignore pre-release suffixes (`-rc1`) and build metadata.
        let s = s.split(['-', '+']).next().unwrap_or(s);
        let mut it = s.split('.');
        let mut out = [0u64; 3];
        for slot in out.iter_mut() {
            *slot = it.next()?.trim().parse().ok()?;
        }
        Some(out)
    }
    match (parts(tag), parts(APP_VERSION)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// Blocking: fetch the latest release and pick the zip asset.
/// `Err` is a human-readable line for the status bar.
pub fn check_latest() -> Result<Option<ReleaseInfo>, String> {
    let mut resp = ureq::get(&api_url())
        .header("User-Agent", "vst-applier")
        .header("Accept", "application/vnd.github+json")
        .config()
        .timeout_global(Some(std::time::Duration::from_secs(20)))
        .build()
        .call()
        .map_err(|e| format!("update check failed: {e}"))?;
    let body = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("update check failed: {e}"))?;
    let v: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("update check failed: {e}"))?;
    let tag = v.get("tag_name").and_then(|t| t.as_str()).unwrap_or("").to_string();
    if tag.is_empty() {
        return Err("update check failed: no releases yet".into());
    }
    if !tag_is_newer(&tag) {
        return Ok(None);
    }
    let name = v.get("name").and_then(|t| t.as_str()).unwrap_or(&tag).to_string();
    let assets = v.get("assets").and_then(|a| a.as_array()).cloned().unwrap_or_default();
    let mut pick: Option<(String, String)> = None;
    for a in &assets {
        let n = a.get("name").and_then(|x| x.as_str()).unwrap_or("");
        let u = a
            .get("browser_download_url")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        if n.ends_with(".zip") && !u.is_empty() {
            // Prefer an asset whose name mentions the app.
            if n.to_lowercase().contains("vst-applier") {
                pick = Some((n.to_string(), u.to_string()));
                break;
            }
            pick.get_or_insert((n.to_string(), u.to_string()));
        }
    }
    match pick {
        Some((zip_name, zip_url)) => Ok(Some(ReleaseInfo { tag, name, zip_url, zip_name })),
        None => Err(format!("{tag} has no zip asset")),
    }
}

/// Staging dir for a pending update (cleared before each download).
pub fn staging_dir() -> PathBuf {
    app_state::settings_dir().join("pending")
}

/// Blocking: download `info`'s zip into staging and extract it.
/// Returns the staging dir on success.
pub fn download_and_stage(info: &ReleaseInfo) -> Result<PathBuf, String> {
    let stage = staging_dir();
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage).map_err(|e| format!("staging failed: {e}"))?;
    let zip_path = stage.join(&info.zip_name);
    let resp = ureq::get(&info.zip_url)
        .header("User-Agent", "vst-applier")
        .config()
        .timeout_global(Some(std::time::Duration::from_secs(180)))
        .build()
        .call()
        .map_err(|e| format!("download failed: {e}"))?;
    let mut reader = resp.into_body().into_reader();
    let mut file =
        std::fs::File::create(&zip_path).map_err(|e| format!("staging failed: {e}"))?;
    std::io::copy(&mut reader, &mut file).map_err(|e| format!("download failed: {e}"))?;
    drop(file);
    extract_flat(&zip_path, &stage)?;
    let _ = std::fs::remove_file(&zip_path);
    for need in PAYLOAD {
        if !stage.join(need).exists() {
            return Err(format!("zip is missing {need}"));
        }
    }
    if !staged_has_plugin(&stage) {
        log_hint("update: zip carries no bundled-plugin payload (app-only update)");
    }
    Ok(stage)
}

/// Extract every file entry of `zip_path` flat into `stage` (bare file
/// names only; `enclosed_name` refuses absolute paths and `..` escapes).
fn extract_flat(zip_path: &std::path::Path, stage: &std::path::Path) -> Result<(), String> {
    let archive =
        std::fs::File::open(zip_path).map_err(|e| format!("staging failed: {e}"))?;
    let mut zip = zip::ZipArchive::new(archive).map_err(|e| format!("bad zip: {e}"))?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| format!("bad zip: {e}"))?;
        let name = match entry.enclosed_name() {
            Some(p) => match p.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            },
            None => continue,
        };
        if name.is_empty() || entry.is_dir() {
            continue;
        }
        let out = stage.join(&name);
        let mut dst =
            std::fs::File::create(&out).map_err(|e| format!("staging failed: {e}"))?;
        std::io::copy(&mut entry, &mut dst).map_err(|e| format!("staging failed: {e}"))?;
    }
    Ok(())
}

/// Swap the staged files over the live install and restart.
///
/// The swap runs in a detached PowerShell script (stock cmd batch jobs
/// from a console-less GUI process proved flaky: the .bat was written
/// but never executed, so pressing "Install + Restart" did nothing).
/// The script drops an `install.log` heartbeat as its first action —
/// we wait for it before exiting, so a launch failure surfaces as an
/// error in the dialog instead of a silent no-op. The copy itself is
/// retried once ELEVATED (UAC) because installs under `Program Files`
/// are not writable from the normal user token.
///
/// On success the process exits so the script can replace the locked
/// exe, then it relaunches the app. Returns an error line otherwise.
pub fn install_and_restart(stage: &std::path::Path) -> String {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => return format!("install failed: {e}"),
    };
    let dir = match exe.parent() {
        Some(d) => d.to_path_buf(),
        None => return "install failed: no exe dir".to_string(),
    };
    let ps = stage.join("install.ps1");
    let log = stage.join("install.log");
    let _ = std::fs::remove_file(&log);
    let script = include_str!("install_ps1.txt")
        .replace("@STAGE@", &stage.display().to_string())
        .replace("@DIR@", &dir.display().to_string())
        .replace("@PID@", &std::process::id().to_string());
    if let Err(e) = std::fs::write(&ps, script) {
        return format!("install failed: {e}");
    }
    // CREATE_NO_WINDOW: hidden console so PowerShell/PowerShell-out
    // semantics are normal, no window pops. Null stdio: the GUI build
    // has no console, and cmd/PS misbehave with truly invalid handles.
    let spawn = std::os::windows::process::CommandExt::creation_flags(
        std::process::Command::new("powershell")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null()),
        0x08000000,
    )
    .args([
        "-NoProfile",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
        &ps.to_string_lossy().into_owned(),
    ])
    .spawn();
    match spawn {
        Ok(_) => {}
        Err(e) => return format!("install failed: could not start installer ({e})"),
    }
    // Wait for the script's heartbeat before giving up our exe lock.
    // Without this, a script that never starts means the app exits and
    // NOTHING happens (the exact bug users hit).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
    while std::time::Instant::now() < deadline {
        if log.exists() {
            std::process::exit(0);
        }
        std::thread::sleep(std::time::Duration::from_millis(120));
    }
    "install failed: the update script did not start".to_string()
}

#[cfg(test)]
mod tests {
    use super::{extract_flat, staged_has_plugin, tag_is_newer};

    #[test]
    fn newer_logic() {
        assert!(tag_is_newer("v9.9.9"));
        assert!(!tag_is_newer(super::APP_VERSION));
        assert!(!tag_is_newer("v0.0.0"));
        assert!(!tag_is_newer("garbage"));
    }

    #[test]
    fn zip_extraction_carries_the_plugin() {
        // The updater must stage all four release files flat, including
        // the bundled plugin it then copies into Plugins\VST-Applier.
        let dir = std::env::temp_dir().join(format!("va_upd_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("tmp");
        let zip_path = dir.join("t.zip");
        {
            let f = std::fs::File::create(&zip_path).expect("zip");
            let mut z = zip::ZipWriter::new(f);
            let opts = zip::write::SimpleFileOptions::default();
            for n in [
                "vst-applier.exe",
                "Vst2HostNative.dll",
                "Vst3HostNative.dll",
                "Yousef Noise Suppression.dll",
            ] {
                z.start_file(n, opts).expect("entry");
                std::io::Write::write_all(&mut z, b"x").expect("write");
            }
            z.finish().expect("finish");
        }
        let stage = dir.join("stage");
        std::fs::create_dir_all(&stage).expect("stage");
        extract_flat(&zip_path, &stage).expect("extract");
        for n in [
            "vst-applier.exe",
            "Vst2HostNative.dll",
            "Vst3HostNative.dll",
            "Yousef Noise Suppression.dll",
        ] {
            assert!(stage.join(n).exists(), "missing {n}");
        }
        assert!(staged_has_plugin(&stage));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
