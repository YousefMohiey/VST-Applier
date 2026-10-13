//! Self-update from GitHub Releases (mirrors an auto-updater that the C#
//! app never had). No installer: the release asset is a zip holding
//! `vst-applier.exe` + the two native host DLLs, and install swaps those
//! three files next to the running exe, then restarts.
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

/// Expected files inside the release zip.
const PAYLOAD: &[&str] = &["vst-applier.exe", "Vst2HostNative.dll", "Vst3HostNative.dll"];

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
    let archive =
        std::fs::File::open(&zip_path).map_err(|e| format!("staging failed: {e}"))?;
    let mut zip = zip::ZipArchive::new(archive).map_err(|e| format!("bad zip: {e}"))?;
    // `enclosed_name` refuses absolute paths and `..` escapes; only
    // bare file names land flat in staging.
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
    let _ = std::fs::remove_file(&zip_path);
    for need in PAYLOAD {
        if !stage.join(need).exists() {
            return Err(format!("zip is missing {need}"));
        }
    }
    Ok(stage)
}

/// Swap the staged files over the live install and restart.
/// Never returns on success (the process exits for the swap).
pub fn install_and_restart(stage: &std::path::Path) -> String {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => return format!("install failed: {e}"),
    };
    let dir = match exe.parent() {
        Some(d) => d.to_path_buf(),
        None => return "install failed: no exe dir".to_string(),
    };
    let bat = stage.join("updater.bat");
    let script = format!(
        "@echo off\r\n\
         :wait\r\n\
         tasklist /FI \"PID eq {pid}\" 2>NUL | find \"{pid}\" >NUL && (timeout /t 1 /nobreak >NUL & goto wait)\r\n\
         copy /Y \"{stage}\\vst-applier.exe\" \"{dir}\\vst-applier.exe\"\r\n\
         copy /Y \"{stage}\\Vst2HostNative.dll\" \"{dir}\\Vst2HostNative.dll\"\r\n\
         copy /Y \"{stage}\\Vst3HostNative.dll\" \"{dir}\\Vst3HostNative.dll\"\r\n\
         start \"\" \"{dir}\\vst-applier.exe\"\r\n\
         del \"%~f0\"\r\n",
        pid = std::process::id(),
        stage = stage.display(),
        dir = dir.display(),
    );
    if let Err(e) = std::fs::write(&bat, script) {
        return format!("install failed: {e}");
    }
    #[cfg(windows)]
    let spawn = std::os::windows::process::CommandExt::creation_flags(
        &mut std::process::Command::new("cmd"),
        0x00000008, // DETACHED_PROCESS: no console window of its own
    )
    .args(["/C", &bat.to_string_lossy().into_owned()])
    .current_dir(&dir)
    .spawn();
    #[cfg(not(windows))]
    let spawn = std::process::Command::new("cmd").args(["/C"]).spawn();
    match spawn {
        Ok(_) => std::process::exit(0),
        Err(e) => format!("install failed: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::tag_is_newer;

    #[test]
    fn newer_logic() {
        assert!(tag_is_newer("v9.9.9"));
        assert!(!tag_is_newer(super::APP_VERSION));
        assert!(!tag_is_newer("v0.0.0"));
        assert!(!tag_is_newer("garbage"));
    }
}
