//! FFI bindings + safe wrappers for the native VST host DLLs.
//!
//! Mirrors the C# `NativeVstHostApi` / `NativeVst2HostApi` P/Invoke surface:
//! two DLLs (`Vst3HostNative.dll`, `Vst2HostNative.dll`) exposing `Vst3Host_*`
//! / `Vst2Host_*` C exports. Return convention: 0 = ok, negative = error
//! code, details via `GetLastError`. State blobs are little-endian
//! `[u32 magic | u32 version=1 | payload...]`, base64-encoded into profiles.

#![allow(non_snake_case)]
//! Raw export names mirror the native DLLs exactly, hence non-snake-case.

use std::ffi::{c_void, OsStr};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use libloading::{Library, Symbol};
use thiserror::Error;

pub const API_VERSION: i32 = 1;

/// Opaque native host instance.
pub type HostHandle = *mut c_void;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginFormat {
    Vst2,
    Vst3,
}

impl PluginFormat {
    pub fn dll_name(self) -> &'static str {
        match self {
            PluginFormat::Vst2 => "Vst2HostNative.dll",
            PluginFormat::Vst3 => "Vst3HostNative.dll",
        }
    }
    pub fn prefix(self) -> &'static str {
        match self {
            PluginFormat::Vst2 => "Vst2Host",
            PluginFormat::Vst3 => "Vst3Host",
        }
    }
}

#[derive(Debug, Error)]
pub enum HostError {
    #[error("host library not found: {0} ({1})")]
    LibraryNotFound(String, String),
    #[error("host export missing: {0}")]
    MissingExport(String),
    #[error("native call failed ({code}): {message}")]
    CallFailed { code: i32, message: String },
    #[error("bad state blob: {0}")]
    BadState(String),
    #[error("invalid path")]
    InvalidPath,
}

pub type Result<T> = std::result::Result<T, HostError>;

// ---------------------------------------------------------------------------
// Raw exports
// ---------------------------------------------------------------------------

macro_rules! decl_api {
    ($($name:ident : $sig:ty),* $(,)?) => {
        pub struct RawApi {
            _lib: Library,
            $(pub $name: Symbol<'static, $sig>),*,
            pub get_editor_size: Option<Symbol<'static, FnGetEditorSize>>,
            pub editor_idle: Option<Symbol<'static, FnEditorIdle>>,
        }

        impl RawApi {
            /// # Safety
            /// Loads a DLL; caller must ensure it is the genuine host library.
            #[allow(unsafe_op_in_unsafe_fn)]
            pub unsafe fn load(dll_path: &Path, prefix: &str) -> Result<Self> {
                let lib = Library::new(dll_path).map_err(|e| {
                    HostError::LibraryNotFound(
                        dll_path.display().to_string(),
                        e.to_string(),
                    )
                })?;
                // Extend lifetime: RawApi owns `_lib`, symbols stay valid.
                let lib_ref: &'static Library =
                    &*(&lib as *const Library);
                macro_rules! sym {
                    ($export:expr) => {
                        lib_ref.get($export.as_bytes()).map_err(|_| {
                            HostError::MissingExport($export)
                        })?
                    };
                }
                let api = RawApi {
                    $($name: sym!(format!("{}_{}", prefix, stringify!($name)))),*,
                    get_editor_size: lib_ref
                        .get(format!("{prefix}_GetEditorSize").as_bytes())
                        .ok(),
                    editor_idle: lib_ref
                        .get(format!("{prefix}_EditorIdle").as_bytes())
                        .ok(),
                    _lib: lib,
                };
                Ok(api)
            }
        }
    };
}

type FnGetApiVersion = unsafe extern "C" fn() -> i32;
type FnCreate = unsafe extern "C" fn() -> HostHandle;
type FnDestroy = unsafe extern "C" fn(handle: HostHandle);
type FnLoadPlugin = unsafe extern "C" fn(handle: HostHandle, path: *const u16) -> i32;
type FnSetupProcessing = unsafe extern "C" fn(
    handle: HostHandle,
    sample_rate: f64,
    max_block: i32,
    inputs: i32,
    outputs: i32,
) -> i32;
type FnProcessFloat32 = unsafe extern "C" fn(
    handle: HostHandle,
    input: *const f32,
    output: *mut f32,
    frames: i32,
) -> i32;
type FnSaveState =
    unsafe extern "C" fn(handle: HostHandle, buffer: *mut u8, capacity: i32) -> i32;
type FnLoadState =
    unsafe extern "C" fn(handle: HostHandle, data: *const u8, size: i32) -> i32;
type FnOpenEditor =
    unsafe extern "C" fn(handle: HostHandle, hwnd: *mut c_void) -> i32;
type FnCloseEditor = unsafe extern "C" fn(handle: HostHandle);
type FnGetLastError = unsafe extern "C" fn(handle: HostHandle) -> *const u16;
type FnGetEditorSize = unsafe extern "C" fn(
    handle: HostHandle,
    width: *mut i32,
    height: *mut i32,
) -> i32;
type FnEditorIdle = unsafe extern "C" fn(handle: HostHandle);

decl_api! {
    GetApiVersion: FnGetApiVersion,
    Create: FnCreate,
    Destroy: FnDestroy,
    LoadPlugin: FnLoadPlugin,
    SetupProcessing: FnSetupProcessing,
    ProcessFloat32: FnProcessFloat32,
    SaveState: FnSaveState,
    LoadState: FnLoadState,
    OpenEditor: FnOpenEditor,
    CloseEditor: FnCloseEditor,
    GetLastError: FnGetLastError,
}

// ---------------------------------------------------------------------------
// Safe host wrapper
// ---------------------------------------------------------------------------

/// A loaded plugin instance in one native host.
pub struct PluginHost {
    api: RawApi,
    handle: HostHandle,
    pub format: PluginFormat,
    pub channels: i32,
}

impl PluginHost {
    /// Load `plugin_path` through the host DLL next to `host_dll`.
    pub fn load(
        host_dll: &Path,
        format: PluginFormat,
        plugin_path: &Path,
    ) -> Result<Self> {
        let api = unsafe { RawApi::load(host_dll, format.prefix()) }?;
        let version = unsafe { (api.GetApiVersion)() };
        if version != API_VERSION {
            return Err(HostError::CallFailed {
                code: version,
                message: format!("unsupported host API version {version}"),
            });
        }
        let handle = unsafe { (api.Create)() };
        if handle.is_null() {
            return Err(HostError::CallFailed {
                code: -1,
                message: "host Create returned null".into(),
            });
        }
        let host = PluginHost { api, handle, format, channels: 2 };
        let wide = wide_path(plugin_path)?;
        let rc = unsafe { (host.api.LoadPlugin)(handle, wide.as_ptr()) };
        if rc != 0 {
            let msg = host.last_error();
            unsafe { (host.api.Destroy)(handle) };
            // Prevent double-free in Drop.
            std::mem::forget(host);
            // Rebuild a minimal value to destroy correctly is messy; instead
            // leak is avoided by destroying above. Reconstruct never.
            return Err(HostError::CallFailed { code: rc, message: msg });
        }
        Ok(host)
    }

    pub fn setup_processing(
        &mut self,
        sample_rate: f64,
        max_block: i32,
        channels: i32,
    ) -> Result<()> {
        let rc = unsafe {
            (self.api.SetupProcessing)(self.handle, sample_rate, max_block, channels, channels)
        };
        if rc != 0 {
            let msg = self.last_error();
            return Err(HostError::CallFailed { code: rc, message: msg });
        }
        // Only a successful setup owns the channel count: callers use
        // `channels` to choose the stereo or mono processing path.
        self.channels = channels;
        Ok(())
    }

    /// Process interleaved stereo `input` into `output` (same length).
    pub fn process_f32(&mut self, input: &[f32], output: &mut [f32]) -> Result<()> {
        debug_assert_eq!(input.len(), output.len());
        assert_eq!(input.len() % self.channels as usize, 0);
        let frames = (input.len() / self.channels as usize) as i32;
        let rc = unsafe {
            (self.api.ProcessFloat32)(
                self.handle,
                input.as_ptr(),
                output.as_mut_ptr(),
                frames,
            )
        };
        if rc != 0 {
            let msg = self.last_error();
            return Err(HostError::CallFailed { code: rc, message: msg });
        }
        Ok(())
    }

    /// State snapshot. The native call returns the blob size directly:
    /// negative = error, 0 = no state, positive = bytes. Single protocol
    /// for both DLLs: query with a null buffer, then fill.
    pub fn save_state(&mut self) -> Result<Option<Vec<u8>>> {
        let size = unsafe { (self.api.SaveState)(self.handle, std::ptr::null_mut(), 0) };
        if size < 0 {
            let msg = self.last_error();
            return Err(HostError::CallFailed { code: size, message: msg });
        }
        if size == 0 {
            return Ok(None);
        }
        let mut buf = vec![0u8; size as usize];
        let rc = unsafe { (self.api.SaveState)(self.handle, buf.as_mut_ptr(), size) };
        if rc < 0 {
            let msg = self.last_error();
            return Err(HostError::CallFailed { code: rc, message: msg });
        }
        buf.truncate(rc.max(0) as usize);
        if buf.len() < 8 {
            return Ok(None);
        }
        Ok(Some(buf))
    }

    pub fn load_state(&mut self, blob: &[u8]) -> Result<()> {
        let rc = unsafe {
            (self.api.LoadState)(self.handle, blob.as_ptr(), blob.len() as i32)
        };
        if rc != 0 {
            let msg = self.last_error();
            return Err(HostError::CallFailed { code: rc, message: msg });
        }
        Ok(())
    }

    /// # Safety
    /// `hwnd` must be a valid parent window handle.
    pub unsafe fn open_editor(&mut self, hwnd: *mut c_void) -> Result<()> {
        let rc = unsafe { (self.api.OpenEditor)(self.handle, hwnd) };
        if rc != 0 {
            let msg = self.last_error();
            return Err(HostError::CallFailed { code: rc, message: msg });
        }
        Ok(())
    }

    pub fn close_editor(&mut self) -> Result<()> {
        unsafe { (self.api.CloseEditor)(self.handle) };
        Ok(())
    }

    /// VST2 only: preferred editor size. Returns None when the host does not
    /// export the editor-size entry point (e.g. the VST3 host).
    pub fn editor_size(&mut self) -> Option<(i32, i32)> {
        let f = self.api.get_editor_size.as_ref()?;
        let (mut w, mut h) = (0, 0);
        let rc = unsafe { f(self.handle, &mut w, &mut h) };
        if rc == 0 && w > 0 && h > 0 {
            Some((w, h))
        } else {
            None
        }
    }

    /// VST2 only: pump editor idle. No-op when the export is missing.
    pub fn editor_idle(&mut self) {
        if let Some(f) = self.api.editor_idle.as_ref() {
            unsafe { f(self.handle) };
        }
    }

    pub fn last_error(&self) -> String {
        let ptr = unsafe { (self.api.GetLastError)(self.handle) };
        if ptr.is_null() {
            return "unknown error".to_string();
        }
        // Null-terminated UTF-16, capped at a sane length.
        let len = (0..4096)
            .find(|&i| unsafe { *ptr.add(i) } == 0)
            .unwrap_or(4096);
        let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
        String::from_utf16_lossy(slice)
    }
}

impl Drop for PluginHost {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { (self.api.Destroy)(self.handle) };
            self.handle = std::ptr::null_mut();
        }
    }
}

// `PluginHost` is moved between the UI thread and the audio thread only at
// route (re)start; never shared. Mark Send explicitly (raw handle is !Send).
unsafe impl Send for PluginHost {}

fn wide_path(path: &Path) -> Result<Vec<u16>> {
    let s: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    if s.len() <= 1 {
        return Err(HostError::InvalidPath);
    }
    let _ = OsStr::new("");
    Ok(s)
}

// ---------------------------------------------------------------------------
// State blob helpers (magic + version header, base64 for JSON)
// ---------------------------------------------------------------------------

/// Known state magics. `VSTC`/`VSTA` = full chunk, `VSTD`/`VSTP` = param
/// dump fallback. `SNJ*` are legacy read-compatible magics from the C# app.
pub mod magic {
    pub const VST2_CHUNK: u32 = u32::from_le_bytes(*b"VSTC");
    pub const VST2_PARAMS: u32 = u32::from_le_bytes(*b"VSTD");
    pub const VST3_FULL: u32 = u32::from_le_bytes(*b"VSTA");
    pub const VST3_PARAMS: u32 = u32::from_le_bytes(*b"VSTP");
    pub const LEGACY: [u32; 4] = [
        u32::from_le_bytes(*b"SNJC"),
        u32::from_le_bytes(*b"SNJP"),
        u32::from_le_bytes(*b"SNJS"),
        u32::from_le_bytes(*b"SNJQ"),
    ];
    pub const VERSION: u32 = 1;
}

pub use base64::engine::general_purpose::STANDARD as B64;
pub use base64::Engine as _;

/// Validate a raw state blob header; returns the magic on success.
pub fn check_blob(blob: &[u8]) -> Result<u32> {
    if blob.len() < 8 {
        return Err(HostError::BadState("blob shorter than header".into()));
    }
    let magic = u32::from_le_bytes(blob[0..4].try_into().unwrap());
    let version = u32::from_le_bytes(blob[4..8].try_into().unwrap());
    let known = magic == magic::VST2_CHUNK
        || magic == magic::VST2_PARAMS
        || magic == magic::VST3_FULL
        || magic == magic::VST3_PARAMS
        || magic::LEGACY.contains(&magic);
    if !known {
        return Err(HostError::BadState(format!("unknown magic {magic:#x}")));
    }
    if version != magic::VERSION {
        return Err(HostError::BadState(format!(
            "unsupported state version {version}"
        )));
    }
    Ok(magic)
}

/// Decode a base64 `State` field from a profile; None = defaults.
pub fn decode_state_field(field: Option<&str>) -> Result<Option<Vec<u8>>> {
    match field {
        None => Ok(None),
        Some("") => Ok(None),
        Some(s) => {
            let blob = B64
                .decode(s)
                .map_err(|e| HostError::BadState(e.to_string()))?;
            if blob.is_empty() {
                return Ok(None);
            }
            check_blob(&blob)?;
            Ok(Some(blob))
        }
    }
}

pub fn encode_state_field(blob: Option<&[u8]>) -> Option<String> {
    blob.and_then(|b| {
        if b.is_empty() {
            None
        } else {
            Some(B64.encode(b))
        }
    })
}
