//! ONNX Runtime discovery / initialisation and execution-provider reporting.
//!
//! The plugin ships `onnxruntime.dll` (DirectML build) + `DirectML.dll` in its
//! own folder. Windows' default DLL search order does **not** include the
//! folder of a DLL loaded by full path, so we resolve both explicitly:
//!
//! * `onnxruntime.dll` is loaded by absolute path (`ort::init_from`), which
//!   bypasses the loader search entirely;
//! * `DirectML.dll` is pre-loaded by absolute path before the first DirectML
//!   session. ONNX Runtime's DML provider calls `LoadLibraryA("DirectML.dll")`
//!   itself, and Windows resolves that to the already-loaded module of the
//!   same base name — independent of the search path.
//!
//! If the DLLs are missing the plugin still loads and passes audio through;
//! the editor says so. Nothing here is allowed to take the host down.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use windows::core::PCWSTR;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};
use windows::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetModuleHandleExW, LoadLibraryW,
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};

/// Which execution provider the inference sessions actually run on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EpKind {
    /// DirectML (GPU) — the fast path.
    DirectMl,
    /// ONNX Runtime CPU EP — fallback, same models, slower.
    Cpu,
    /// ONNX Runtime itself could not be loaded; plugin passes audio through.
    Missing,
}

impl EpKind {
    pub fn short(&self) -> &'static str {
        match self {
            EpKind::DirectMl => "GPU",
            EpKind::Cpu => "CPU",
            EpKind::Missing => "OFF",
        }
    }
}

/// Process-wide runtime facts (cheap to clone: strings only).
#[derive(Clone, Debug)]
pub struct RuntimeStatus {
    /// DXGI adapter description (DirectML target), when known.
    pub adapter: String,
    /// `onnxruntime.dll` version reported by the runtime itself, e.g. "1.24.4".
    pub ort_version: String,
    /// Full path of the loaded `onnxruntime.dll`.
    pub dll: Option<PathBuf>,
    /// Human-readable one-liner for logs.
    pub detail: String,
}

impl RuntimeStatus {
    fn missing(detail: impl Into<String>) -> Self {
        Self {
            adapter: String::new(),
            ort_version: String::new(),
            dll: None,
            detail: detail.into(),
        }
    }

    pub fn available(&self) -> bool {
        self.dll.is_some()
    }

    /// The line the editor shows under "INFERENCE".
    pub fn status_line(&self, ep: EpKind) -> String {
        match ep {
            EpKind::DirectMl => {
                if self.adapter.is_empty() {
                    format!("DirectML / ONNX Runtime {}", self.ort_version)
                } else {
                    format!("DirectML / {}", self.adapter)
                }
            }
            EpKind::Cpu => "CPU fallback (DirectML unavailable)".to_string(),
            EpKind::Missing => "ONNX Runtime missing - audio passes through".to_string(),
        }
    }
}

static STATUS: OnceLock<RuntimeStatus> = OnceLock::new();

/// Initialise the process-wide ORT environment (idempotent).
///
/// `dir_hint` is normally the folder the plugin DLL lives in. The search order
/// is: `DF_ORT_DIR` env var → `dir_hint` → this module's own folder (the
/// plugin DLL, or the test binary's folder) → the running executable's folder.
pub fn init(dir_hint: Option<&Path>) -> RuntimeStatus {
    STATUS.get_or_init(|| probe_and_init(dir_hint)).clone()
}

/// Status after [`init`] has been called at least once; `None` before that.
pub fn status() -> Option<RuntimeStatus> {
    STATUS.get().cloned()
}

fn probe_and_init(dir_hint: Option<&Path>) -> RuntimeStatus {
    let mut tried: Vec<String> = Vec::new();
    let mut lib: Option<PathBuf> = None;

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(d) = std::env::var_os("DF_ORT_DIR") {
        candidates.push(PathBuf::from(d));
    }
    if let Some(d) = dir_hint {
        candidates.push(d.to_path_buf());
    }
    if let Some(d) = own_module_dir() {
        candidates.push(d);
    }
    if let Some(d) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
    {
        candidates.push(d);
    }

    for dir in &candidates {
        let p = dir.join("onnxruntime.dll");
        if p.is_file() {
            lib = Some(p);
            break;
        } else {
            tried.push(p.display().to_string());
        }
    }

    let dll = match lib {
        Some(p) => p,
        None => {
            return RuntimeStatus::missing(format!(
                "onnxruntime.dll not found ({} locations checked)",
                tried.len().max(1)
            ));
        }
    };

    // DirectML.dll must be resident before ORT's DML provider looks for it.
    if let Some(dir) = dll.parent() {
        let dm = dir.join("DirectML.dll");
        if dm.is_file() {
            let wide = to_wide(dm.as_os_str());
            unsafe {
                let _ = LoadLibraryW(PCWSTR(wide.as_ptr()));
            }
        }
    }

    match ort::init_from(&dll) {
        Ok(builder) => {
            let _ = builder.commit(); // false = another instance won the race
        }
        Err(e) => {
            return RuntimeStatus::missing(format!(
                "onnxruntime.dll at {} failed to load: {e}",
                dll.display()
            ));
        }
    }

    let ort_version = unsafe { ort_version_string() };
    let adapter = unsafe { primary_adapter_name() };
    RuntimeStatus {
        detail: format!("ONNX Runtime {ort_version} loaded from {}", dll.display()),
        adapter,
        ort_version,
        dll: Some(dll),
    }
}

fn to_wide(s: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    s.encode_wide().chain(std::iter::once(0)).collect()
}

/// Directory of the module that contains this function (the plugin DLL in
/// production, the test executable under `cargo test`).
fn own_module_dir() -> Option<PathBuf> {
    unsafe {
        let mut hmod = HMODULE::default();
        let f: *const u16 = own_module_dir as *const u16;
        let flags = GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
            | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
        if GetModuleHandleExW(flags, PCWSTR(f), &mut hmod).is_err() {
            return None;
        }
        let mut buf = [0u16; 1024];
        let n = GetModuleFileNameW(hmod, &mut buf);
        if n == 0 {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..n as usize]);
        Path::new(&path).parent().map(|p| p.to_path_buf())
    }
}

/// `onnxruntime.dll`'s own version string, read through its exported
/// `OrtGetApiBase` (exact, e.g. "1.24.4"); falls back to the file's version
/// resource if the export can't be resolved.
unsafe fn ort_version_string() -> String {
    use windows::core::PCSTR;
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

    if let Ok(h) = GetModuleHandleW(windows::core::w!("onnxruntime.dll")) {
        if let Some(f) = GetProcAddress(h, PCSTR(b"OrtGetApiBase\0".as_ptr())) {
            let get_base: unsafe extern "system" fn() -> *const ort::sys::OrtApiBase =
                std::mem::transmute(f);
            let base = get_base();
            if !base.is_null() {
                let p = ((*base).GetVersionString)();
                if !p.is_null() {
                    let s = std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned();
                    if !s.is_empty() {
                        return s;
                    }
                }
            }
        }
    }
    String::new()
}

/// Name of DXGI adapter 0 — the device DirectML binds to by default.
unsafe fn primary_adapter_name() -> String {
    let factory: IDXGIFactory1 = match CreateDXGIFactory1() {
        Ok(f) => f,
        Err(_) => return String::new(),
    };
    let adapter = match factory.EnumAdapters1(0) {
        Ok(a) => a,
        Err(_) => return String::new(),
    };
    match adapter.GetDesc1() {
        Ok(desc) => {
            let end = desc
                .Description
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(desc.Description.len());
            String::from_utf16_lossy(&desc.Description[..end])
                .trim()
                .to_string()
        }
        Err(_) => String::new(),
    }
}
