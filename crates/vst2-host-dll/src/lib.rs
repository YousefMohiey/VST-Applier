//! Full Rust port of `native/Vst2HostNative/Vst2HostNative.cpp`.
//!
//! The DLL boundary is identical to the C++ original: same file name
//! (`Vst2HostNative.dll`), same `Vst2Host_*` exports (C ABI), same return
//! convention (0 = ok, negative = error code, message via
//! `Vst2Host_GetLastError`), same state-blob layout (little-endian
//! `[u32 magic | u32 version = 1 | payload]`; VST2 uses the `VSTC` chunk blob
//! with the `VSTD` parameter-dump fallback, and the legacy `SNJC`/`SNJP`
//! magics are still accepted on load).
//!
//! Crash isolation matches the C++ original: every call into plugin code runs
//! inside Windows SEH (`__try/__except`, provided by the tiny C stub shipped
//! with the `microseh` crate) plus a Rust `catch_unwind` trap. A plugin that
//! raises a hardware exception (access violation, ...) or panics is reported
//! through the same "raised a native exception" error codes and messages.
//!
//! Notes for maintainers:
//! * No exported entry point may unwind: each `Vst2Host_*` function is a thin
//!   shell around an `*_impl` body guarded by [`guard`] / [`guard_i32`].
//! * The plugin's `AEffect` is read/written through the pointer it returned
//!   from `VSTPluginMain`; [`AEffect`] mirrors the C++ layout field for field
//!   (both `#[repr(C)]`, 192 bytes on x64).

#![allow(non_snake_case)]
#![allow(dead_code)] // AEffect has fields a host never touches.

use std::cell::{Cell, RefCell};
use std::ffi::{c_void, CStr};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::OnceLock;

use windows::core::{PCSTR, PCWSTR, PWSTR};
use windows::Win32::Foundation::{FreeLibrary, HMODULE, HLOCAL, HWND, LocalFree, RECT};
use windows::Win32::Globalization::{MultiByteToWideChar, CP_UTF8, MULTI_BYTE_TO_WIDE_CHAR_FLAGS};
use windows::Win32::Storage::FileSystem::{
    GetFileAttributesW, FILE_ATTRIBUTE_DIRECTORY, INVALID_FILE_ATTRIBUTES,
};
use windows::Win32::System::Diagnostics::Debug::{
    FormatMessageW, FORMAT_MESSAGE_ALLOCATE_BUFFER, FORMAT_MESSAGE_FROM_SYSTEM,
    FORMAT_MESSAGE_IGNORE_INSERTS,
};
use windows::Win32::System::LibraryLoader::{
    GetModuleHandleExW, GetProcAddress, LoadLibraryW, GET_MODULE_HANDLE_EX_FLAG_PIN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetWindowRect, SetWindowPos, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER,
};

// ---------------------------------------------------------------------------
// Constants (mirror of the C++ anonymous namespace)
// ---------------------------------------------------------------------------

pub type Vst2HostInstanceHandle = *mut c_void;

const K_OK: i32 = 0;
const K_ERROR_INVALID_ARGUMENT: i32 = -1;
const K_ERROR_FILE_NOT_FOUND: i32 = -2;
const K_ERROR_LOAD_LIBRARY: i32 = -3;
const K_ERROR_ENTRY_POINT: i32 = -4;
const K_ERROR_PLUGIN_LOAD: i32 = -5;
const K_ERROR_UNSUPPORTED: i32 = -6;
const K_ERROR_NOT_CONFIGURED: i32 = -7;

const K_CHUNK_STATE_BLOB_MAGIC: u32 = 0x4354_5356; // 'VSTC'
const K_PARAM_STATE_BLOB_MAGIC: u32 = 0x4454_5356; // 'VSTD'
const K_LEGACY_CHUNK_STATE_BLOB_MAGIC: u32 = 0x434A_4E53; // legacy (v1.3) 'SNJC'
const K_LEGACY_PARAM_STATE_BLOB_MAGIC: u32 = 0x504A_4E53; // legacy (v1.3) 'SNJP'
const K_STATE_BLOB_VERSION: u32 = 1;

const K_EFFECT_MAGIC: i32 = 0x5673_7450; // 'VstP'

const EFF_OPEN: i32 = 0;
const EFF_CLOSE: i32 = 1;
const EFF_SET_SAMPLE_RATE: i32 = 10;
const EFF_SET_BLOCK_SIZE: i32 = 11;
const EFF_MAINS_CHANGED: i32 = 12;
const EFF_EDIT_GET_RECT: i32 = 13;
const EFF_EDIT_OPEN: i32 = 14;
const EFF_EDIT_CLOSE: i32 = 15;
const EFF_EDIT_IDLE: i32 = 19;
const EFF_GET_CHUNK: i32 = 23;
const EFF_SET_CHUNK: i32 = 24;
const EFF_GET_EFFECT_NAME: i32 = 45;
const EFF_GET_VENDOR_STRING: i32 = 47;
const EFF_GET_PRODUCT_STRING: i32 = 48;

const EFF_FLAGS_HAS_EDITOR: i32 = 1 << 0;
const EFF_FLAGS_CAN_REPLACING: i32 = 1 << 4;

const AUDIO_MASTER_AUTOMATE: i32 = 0;
const AUDIO_MASTER_VERSION: i32 = 1;
const AUDIO_MASTER_SIZE_WINDOW: i32 = 15;
const AUDIO_MASTER_GET_SAMPLE_RATE: i32 = 16;
const AUDIO_MASTER_GET_BLOCK_SIZE: i32 = 17;
const AUDIO_MASTER_GET_INPUT_LATENCY: i32 = 18;
const AUDIO_MASTER_GET_OUTPUT_LATENCY: i32 = 19;
const AUDIO_MASTER_CAN_DO: i32 = 37;
const AUDIO_MASTER_GET_LANGUAGE: i32 = 38;
const AUDIO_MASTER_COCKOS_EXTENSION: i32 = 0xDEAD_BEEF_u32 as i32;
const COCKOS_GET_API_FUNCTION: i32 = 0xDEAD_F00D_u32 as i32;
const COCKOS_GET_HOST_CONTEXT: i32 = 0xDEAD_F00E_u32 as i32;

const INVALID_HANDLE_ERROR: &str = "Invalid VST2 host handle.";

// ---------------------------------------------------------------------------
// VST2 2.4 ABI
//
// `extern "system"` is `__stdcall` on x86 and __cdecl's x64 equivalent: on the
// x64 target this DLL is built for, __cdecl and __stdcall are one and the same
// calling convention, so these are ABI-identical to the C++ `__cdecl` typedefs.
// ---------------------------------------------------------------------------

#[repr(C)]
struct AEffect {
    magic: i32,
    dispatcher: Option<
        unsafe extern "system" fn(*mut AEffect, i32, i32, isize, *mut c_void, f32) -> isize,
    >,
    process: Option<unsafe extern "system" fn(*mut AEffect, *mut *mut f32, *mut *mut f32, i32)>,
    set_parameter: Option<unsafe extern "system" fn(*mut AEffect, i32, f32)>,
    get_parameter: Option<unsafe extern "system" fn(*mut AEffect, i32) -> f32>,
    num_programs: i32,
    num_params: i32,
    num_inputs: i32,
    num_outputs: i32,
    flags: i32,
    reserved1: isize,
    reserved2: isize,
    initial_delay: i32,
    real_qualities: i32,
    off_qualities: i32,
    io_ratio: f32,
    object: *mut c_void,
    user: *mut c_void,
    unique_id: i32,
    version: i32,
    process_replacing:
        Option<unsafe extern "system" fn(*mut AEffect, *mut *mut f32, *mut *mut f32, i32)>,
    process_double_replacing:
        Option<unsafe extern "system" fn(*mut AEffect, *mut *mut f64, *mut *mut f64, i32)>,
    future: [u8; 56],
}

#[repr(C)]
struct ERect {
    top: i16,
    left: i16,
    bottom: i16,
    right: i16,
}

type AudioMasterCallback =
    unsafe extern "system" fn(*mut AEffect, i32, i32, isize, *mut c_void, f32) -> isize;
type VstPluginMainProc = unsafe extern "system" fn(AudioMasterCallback) -> *mut AEffect;

// ---------------------------------------------------------------------------
// Isolation: no Rust panic and no hardware exception may cross the DLL boundary
// ---------------------------------------------------------------------------
//
// Every call into plugin code runs inside two nested guards:
//   * `microseh::try_seh` — the same Windows SEH safety net the former C++
//     host used (`__try __except`, provided by microseh's tiny C stub). A
//     plugin that faults with a hardware exception (access violation,
//     illegal instruction, ...) is caught and reported as an error instead
//     of taking the whole process down.
//   * `catch_unwind` — Rust panics.
// The panic trap is outermost and the SEH guard innermost, so a hardware
// fault is always contained by the C stub before Rust unwind machinery sees it.

/// Run `f`; on panic or hardware exception return `fallback`.
fn guard<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    let mut f = Some(f);
    match microseh::try_seh(|| catch_unwind(AssertUnwindSafe(|| (f.take().unwrap())()))) {
        Ok(Ok(value)) => value,
        Ok(Err(_)) | Err(_) => fallback,
    }
}

/// `guard` with a `Result` in place of a fallback value.
fn guard_result<T>(f: impl FnOnce() -> T) -> Result<T, ()> {
    let mut f = Some(f);
    match microseh::try_seh(|| catch_unwind(AssertUnwindSafe(|| (f.take().unwrap())()))) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(_)) | Err(_) => Err(()),
    }
}

fn guard_i32(f: impl FnOnce() -> i32) -> i32 {
    guard(K_ERROR_PLUGIN_LOAD, f)
}

// ---------------------------------------------------------------------------
// Win32 helpers
// ---------------------------------------------------------------------------

/// Mirror of `GetWindowsErrorMessage`: `FormatMessageW` text with trailing
/// whitespace stripped, `Windows error <code>` when the system has no text.
fn get_windows_error_message(error_code: u32) -> String {
    unsafe {
        let mut buffer: *mut u16 = std::ptr::null_mut();
        let length = FormatMessageW(
            FORMAT_MESSAGE_ALLOCATE_BUFFER
                | FORMAT_MESSAGE_FROM_SYSTEM
                | FORMAT_MESSAGE_IGNORE_INSERTS,
            None,
            error_code,
            0,
            PWSTR(&mut buffer as *mut *mut u16 as *mut u16),
            0,
            None,
        );

        if length == 0 || buffer.is_null() {
            return format!("Windows error {error_code}");
        }

        let message = String::from_utf16_lossy(std::slice::from_raw_parts(buffer, length as usize));
        let _ = LocalFree(HLOCAL(buffer as *mut c_void));

        message.trim_end_matches(['\r', '\n', ' ']).to_string()
    }
}

/// Mirror of `ToWide`: UTF-8 (CP_UTF8) to UTF-16, empty for null/empty input.
fn to_wide(value: *const u8) -> Vec<u16> {
    if value.is_null() {
        return Vec::new();
    }

    unsafe {
        let bytes = CStr::from_ptr(value as *const i8).to_bytes_with_nul();
        if bytes.len() <= 1 {
            return Vec::new();
        }

        let length = MultiByteToWideChar(CP_UTF8, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), bytes, None);
        if length <= 1 {
            return Vec::new();
        }

        // `length` counts the terminating NUL, matching the C++
        // `std::wstring result(length - 1, L'\0')` followed by a full-length
        // write that lands in the terminator slot.
        let mut result = vec![0u16; length as usize];
        MultiByteToWideChar(
            CP_UTF8,
            MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0),
            bytes,
            Some(&mut result),
        );
        result.truncate(length as usize - 1);
        result
    }
}

fn invalid_handle_message() -> *const u16 {
    static MESSAGE: OnceLock<Vec<u16>> = OnceLock::new();
    MESSAGE
        .get_or_init(|| {
            INVALID_HANDLE_ERROR
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect()
        })
        .as_ptr()
}

/// Mirror of `ResizeEditorParent`: grow/shrink `parent` so that its CLIENT
/// area is exactly `client_width` x `client_height`. VST2 plugins draw into
/// the client area; sizing the whole window to the plugin rect clips the UI
/// under the caption and borders.
unsafe fn resize_editor_parent(parent: HWND, client_width: i32, client_height: i32) {
    if parent.is_invalid() || client_width <= 0 || client_height <= 0 {
        return;
    }

    let mut window_rect = RECT::default();
    let mut client_rect = RECT::default();
    if GetWindowRect(parent, &mut window_rect).is_err()
        || GetClientRect(parent, &mut client_rect).is_err()
    {
        return;
    }

    let frame_w = (window_rect.right - window_rect.left) - (client_rect.right - client_rect.left);
    let frame_h = (window_rect.bottom - window_rect.top) - (client_rect.bottom - client_rect.top);

    let _ = SetWindowPos(
        parent,
        HWND(std::ptr::null_mut()),
        0,
        0,
        client_width + frame_w,
        client_height + frame_h,
        SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
    );
}

// ---------------------------------------------------------------------------
// audioMaster state (thread-locals in the C++, same routing here)
// ---------------------------------------------------------------------------

thread_local! {
    static AUDIO_MASTER_OPCODES: RefCell<Vec<i32>> = const { RefCell::new(Vec::new()) };
    /// Editor parent for audioMasterSizeWindow: the plugin calls it on the
    /// thread that opened/pumps the editor (our UI thread), so a thread-local
    /// is the correct routing without handle plumbing.
    static SIZE_WINDOW_PARENT: Cell<*mut c_void> = const { Cell::new(std::ptr::null_mut()) };
    static COCKOS_FUNCTION_REQUESTS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn push_audio_master_opcode(opcode: i32) {
    let _ = AUDIO_MASTER_OPCODES.try_with(|list| {
        if let Ok(mut list) = list.try_borrow_mut() {
            list.push(opcode);
        }
    });
}

fn clear_audio_master_opcodes() {
    let _ = AUDIO_MASTER_OPCODES.try_with(|list| {
        if let Ok(mut list) = list.try_borrow_mut() {
            list.clear();
        }
    });
}

fn format_audio_master_opcodes() -> String {
    let opcodes = AUDIO_MASTER_OPCODES
        .try_with(|list| list.try_borrow().map(|list| list.clone()).unwrap_or_default())
        .unwrap_or_default();

    if opcodes.is_empty() {
        return "none".to_string();
    }

    opcodes
        .iter()
        .map(|opcode| opcode.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

fn push_cockos_request(request: String) {
    let _ = COCKOS_FUNCTION_REQUESTS.try_with(|list| {
        if let Ok(mut list) = list.try_borrow_mut() {
            list.push(request);
        }
    });
}

fn clear_cockos_function_requests() {
    let _ = COCKOS_FUNCTION_REQUESTS.try_with(|list| {
        if let Ok(mut list) = list.try_borrow_mut() {
            list.clear();
        }
    });
}

fn format_cockos_function_requests() -> String {
    let requests = COCKOS_FUNCTION_REQUESTS
        .try_with(|list| list.try_borrow().map(|list| list.clone()).unwrap_or_default())
        .unwrap_or_default();

    if requests.is_empty() {
        return "none".to_string();
    }

    requests.join(",")
}

fn size_window_parent() -> HWND {
    HWND(
        SIZE_WINDOW_PARENT
            .try_with(|cell| cell.get())
            .unwrap_or(std::ptr::null_mut()),
    )
}

fn set_size_window_parent(parent: *mut c_void) {
    let _ = SIZE_WINDOW_PARENT.try_with(|cell| cell.set(parent));
}

// ---------------------------------------------------------------------------
// Cockos (REAPER) host extension stubs
// ---------------------------------------------------------------------------

static G_REAPER_CONFIG_NUM_CPU: i32 = 1;
static G_REAPER_FX_LOAD_STATE_CONTEXT: u8 = 0;
static G_EMPTY_REAPER_INI_PATH: [u8; 1] = [0];

unsafe extern "system" fn cockos_get_zero_time() -> f64 {
    0.0
}

unsafe extern "system" fn cockos_get_stopped_play_state() -> i32 {
    0
}

unsafe extern "system" fn cockos_get_stopped_play_state_ex(_: *mut c_void) -> i32 {
    0
}

unsafe extern "system" fn cockos_set_edit_cursor_position(_: f64, _: bool, _: bool) {}

unsafe extern "system" fn cockos_get_set_repeat(_: i32) -> i32 {
    0
}

unsafe extern "system" fn cockos_get_project_path(buffer: *mut u8, buffer_size: i32) {
    if !buffer.is_null() && buffer_size > 0 {
        unsafe { *buffer = 0 };
    }
}

unsafe extern "system" fn cockos_transport_no_op() {}

unsafe extern "system" fn cockos_audio_is_not_running() -> i32 {
    0
}

unsafe extern "system" fn cockos_get_config_var(name: *const u8, size_out: *mut i32) -> *mut c_void {
    if !size_out.is_null() {
        unsafe { *size_out = 0 };
    }

    if name.is_null() {
        return std::ptr::null_mut();
    }

    let name = unsafe { CStr::from_ptr(name as *const i8) }.to_bytes();
    if name == b"__numcpu" {
        if !size_out.is_null() {
            unsafe { *size_out = std::mem::size_of::<i32>() as i32 };
        }

        return &G_REAPER_CONFIG_NUM_CPU as *const i32 as *mut c_void;
    }

    if name == b"__fx_loadstate_ctx" {
        if !size_out.is_null() {
            unsafe { *size_out = std::mem::size_of::<u8>() as i32 };
        }

        return &G_REAPER_FX_LOAD_STATE_CONTEXT as *const u8 as *mut c_void;
    }

    std::ptr::null_mut()
}

unsafe extern "system" fn cockos_get_ini_file() -> *const u8 {
    G_EMPTY_REAPER_INI_PATH.as_ptr()
}

unsafe extern "system" fn cockos_plugin_register(_: *const u8, _: *mut c_void) -> i32 {
    0
}

/// Mirror of `ResolveCockosHostFunction`.
fn resolve_cockos_host_function(function_name: &[u8]) -> isize {
    macro_rules! cockos_address {
        ($function:expr) => {
            $function as *const () as usize
        };
    }

    let table: &[(&[u8], usize)] = &[
        (
            &b"GetPlayPosition"[..],
            cockos_address!(cockos_get_zero_time),
        ),
        (
            &b"GetPlayPosition2"[..],
            cockos_address!(cockos_get_zero_time),
        ),
        (
            &b"GetCursorPosition"[..],
            cockos_address!(cockos_get_zero_time),
        ),
        (
            &b"GetPlayState"[..],
            cockos_address!(cockos_get_stopped_play_state),
        ),
        (
            &b"GetPlayStateEx"[..],
            cockos_address!(cockos_get_stopped_play_state_ex),
        ),
        (
            &b"SetEditCurPos"[..],
            cockos_address!(cockos_set_edit_cursor_position),
        ),
        (
            &b"GetSetRepeat"[..],
            cockos_address!(cockos_get_set_repeat),
        ),
        (
            &b"GetProjectPath"[..],
            cockos_address!(cockos_get_project_path),
        ),
        (
            &b"OnPlayButton"[..],
            cockos_address!(cockos_transport_no_op),
        ),
        (
            &b"OnStopButton"[..],
            cockos_address!(cockos_transport_no_op),
        ),
        (
            &b"OnPauseButton"[..],
            cockos_address!(cockos_transport_no_op),
        ),
        (
            &b"IsInRealTimeAudio"[..],
            cockos_address!(cockos_audio_is_not_running),
        ),
        (
            &b"Audio_IsRunning"[..],
            cockos_address!(cockos_audio_is_not_running),
        ),
        (
            &b"get_config_var"[..],
            cockos_address!(cockos_get_config_var),
        ),
        (&b"get_ini_file"[..], cockos_address!(cockos_get_ini_file)),
        (
            &b"plugin_register"[..],
            cockos_address!(cockos_plugin_register),
        ),
    ];

    for (name, address) in table {
        if *name == function_name {
            return *address as isize;
        }
    }

    0
}

/// Mirror of `ResolveAudioMasterCanDo`.
unsafe fn resolve_audio_master_can_do(ptr: *const c_void) -> isize {
    if ptr.is_null() {
        return 0;
    }

    let capability = CStr::from_ptr(ptr as *const i8).to_bytes();
    if capability == b"sendVstEvents"
        || capability == b"sendVstMidiEvent"
        || capability == b"receiveVstEvents"
        || capability == b"receiveVstMidiEvent"
        || capability == b"sizeWindow"
    {
        return 1;
    }

    0
}

unsafe fn audio_master_impl(opcode: i32, index: i32, value: isize, ptr: *mut c_void) -> isize {
    push_audio_master_opcode(opcode);

    if opcode == AUDIO_MASTER_COCKOS_EXTENSION {
        if index == COCKOS_GET_API_FUNCTION && !ptr.is_null() {
            let function_name = CStr::from_ptr(ptr as *const i8).to_bytes();
            let resolved_function = resolve_cockos_host_function(function_name);
            push_cockos_request(format!(
                "{}{}",
                String::from_utf8_lossy(function_name),
                if resolved_function != 0 { ":ok" } else { ":null" }
            ));
            return resolved_function;
        }

        if index == COCKOS_GET_HOST_CONTEXT {
            return 0;
        }

        return 0;
    }

    match opcode {
        AUDIO_MASTER_AUTOMATE => 0,
        AUDIO_MASTER_VERSION => 2400,
        AUDIO_MASTER_GET_SAMPLE_RATE => 48000,
        AUDIO_MASTER_GET_BLOCK_SIZE => 512,
        AUDIO_MASTER_GET_INPUT_LATENCY | AUDIO_MASTER_GET_OUTPUT_LATENCY => 0,
        AUDIO_MASTER_CAN_DO => resolve_audio_master_can_do(ptr),
        AUDIO_MASTER_SIZE_WINDOW => {
            resize_editor_parent(size_window_parent(), index, value as i32);
            1
        }
        AUDIO_MASTER_GET_LANGUAGE => 1,
        _ => 0,
    }
}

unsafe extern "system" fn audio_master(
    _effect: *mut AEffect,
    opcode: i32,
    index: i32,
    value: isize,
    ptr: *mut c_void,
    _opt: f32,
) -> isize {
    guard(0, || audio_master_impl(opcode, index, value, ptr))
}

// ---------------------------------------------------------------------------
// Safe(ish) plugin call wrappers (mirror of Call*Safely)
// ---------------------------------------------------------------------------

unsafe fn call_entry_point_safely(
    entry_point: VstPluginMainProc,
    raised_exception: &mut bool,
) -> *mut AEffect {
    *raised_exception = false;

    match guard_result(|| {
        clear_cockos_function_requests();
        entry_point(audio_master)
    }) {
        Ok(effect) => effect,
        Err(_) => {
            *raised_exception = true;
            std::ptr::null_mut()
        }
    }
}

unsafe fn call_dispatcher_safely(
    effect: *mut AEffect,
    opcode: i32,
    index: i32,
    value: isize,
    ptr: *mut c_void,
    opt: f32,
    raised_exception: &mut bool,
) -> isize {
    *raised_exception = false;

    let Some(dispatcher) = (*effect).dispatcher else {
        return 0;
    };

    match guard_result(|| dispatcher(effect, opcode, index, value, ptr, opt)) {
        Ok(result) => result,
        Err(_) => {
            *raised_exception = true;
            0
        }
    }
}

unsafe fn call_process_replacing_safely(
    effect: *mut AEffect,
    inputs: *mut *mut f32,
    outputs: *mut *mut f32,
    sample_frames: i32,
    raised_exception: &mut bool,
) {
    *raised_exception = false;

    let Some(process_replacing) = (*effect).process_replacing else {
        return;
    };

    if guard_result(|| process_replacing(effect, inputs, outputs, sample_frames)).is_err() {
        *raised_exception = true;
    }
}

/// The C++ called `effect->dispatcher` directly (unchecked) in a few places;
/// those calls are still panic-guarded here so nothing can unwind out of the
/// DLL, but the result is discarded exactly like the original.
unsafe fn call_dispatcher_unchecked(
    effect: *mut AEffect,
    opcode: i32,
    index: i32,
    value: isize,
    ptr: *mut c_void,
    opt: f32,
) {
    let Some(dispatcher) = (*effect).dispatcher else {
        return;
    };

    let _ = guard_result(|| dispatcher(effect, opcode, index, value, ptr, opt));
}

// ---------------------------------------------------------------------------
// Host instance
// ---------------------------------------------------------------------------

struct Vst2HostInstance {
    module: HMODULE,
    effect: *mut AEffect,
    opened: bool,
    processing_configured: bool,
    editor_open: bool,
    editor_parent: *mut c_void,
    input_channels: i32,
    output_channels: i32,
    /// NUL-terminated UTF-16; `Vst2Host_GetLastError` hands out its pointer.
    last_error: Vec<u16>,
    plugin_name: Vec<u16>,
}

impl Vst2HostInstance {
    fn new() -> Self {
        Self {
            module: HMODULE(std::ptr::null_mut()),
            effect: std::ptr::null_mut(),
            opened: false,
            processing_configured: false,
            editor_open: false,
            editor_parent: std::ptr::null_mut(),
            input_channels: 0,
            output_channels: 0,
            last_error: vec![0],
            plugin_name: Vec::new(),
        }
    }

    fn fail(&mut self, code: i32, message: &str) -> i32 {
        self.set_last_error(message);
        code
    }

    fn set_ok(&mut self) {
        self.set_last_error("");
    }

    fn set_last_error(&mut self, message: &str) {
        self.last_error.clear();
        self.last_error.extend(message.encode_utf16());
        self.last_error.push(0);
    }

    /// Mirror of `ClosePlugin`: close a live editor, stop the plugin,
    /// `effClose` it and only then release the module. Called from `LoadPlugin`
    /// (before loading a replacement) and from `Drop`.
    unsafe fn close_plugin(&mut self) {
        if !self.effect.is_null() && self.opened && (*self.effect).dispatcher.is_some() {
            if self.editor_open {
                call_dispatcher_unchecked(
                    self.effect,
                    EFF_EDIT_CLOSE,
                    0,
                    0,
                    std::ptr::null_mut(),
                    0.0,
                );
                self.editor_open = false;
            }

            call_dispatcher_unchecked(
                self.effect,
                EFF_MAINS_CHANGED,
                0,
                0,
                std::ptr::null_mut(),
                0.0,
            );
            call_dispatcher_unchecked(self.effect, EFF_CLOSE, 0, 0, std::ptr::null_mut(), 0.0);
        }

        self.effect = std::ptr::null_mut();
        self.opened = false;
        self.processing_configured = false;
        self.input_channels = 0;
        self.output_channels = 0;
        self.plugin_name.clear();

        if !self.module.is_invalid() {
            let _ = FreeLibrary(self.module);
            self.module = HMODULE(std::ptr::null_mut());
        }
    }
}

impl Drop for Vst2HostInstance {
    fn drop(&mut self) {
        // Guarded again here: `Drop` itself must never unwind.
        let _ = guard_result(|| unsafe { self.close_plugin() });
    }
}

/// Mirror of `ReadPluginStrings`: query name/vendor/product, keep the name.
unsafe fn read_plugin_strings(host: &mut Vst2HostInstance) {
    if host.effect.is_null() || (*host.effect).dispatcher.is_none() {
        return;
    }

    let mut name = [0u8; 256];
    call_dispatcher_unchecked(
        host.effect,
        EFF_GET_EFFECT_NAME,
        0,
        0,
        name.as_mut_ptr() as *mut c_void,
        0.0,
    );
    host.plugin_name = to_wide(name.as_ptr());

    let mut vendor = [0u8; 256];
    call_dispatcher_unchecked(
        host.effect,
        EFF_GET_VENDOR_STRING,
        0,
        0,
        vendor.as_mut_ptr() as *mut c_void,
        0.0,
    );

    let mut product = [0u8; 256];
    call_dispatcher_unchecked(
        host.effect,
        EFF_GET_PRODUCT_STRING,
        0,
        0,
        product.as_mut_ptr() as *mut c_void,
        0.0,
    );
}

struct Vst2ChunkInfo {
    index: i32,
    data: *mut c_void,
    size: isize,
}

impl Vst2ChunkInfo {
    fn new() -> Self {
        Self {
            index: -1,
            data: std::ptr::null_mut(),
            size: 0,
        }
    }
}

/// Mirror of `TryGetChunk`.
unsafe fn try_get_chunk(
    host: &Vst2HostInstance,
    chunk_info: &mut Vst2ChunkInfo,
    raised_exception: &mut bool,
) -> bool {
    *chunk_info = Vst2ChunkInfo::new();
    *raised_exception = false;

    if host.effect.is_null() || (*host.effect).dispatcher.is_none() {
        return false;
    }

    for index in 0..=1i32 {
        let mut chunk: *mut c_void = std::ptr::null_mut();
        let size = call_dispatcher_safely(
            host.effect,
            EFF_GET_CHUNK,
            index,
            0,
            &mut chunk as *mut *mut c_void as *mut c_void,
            0.0,
            raised_exception,
        );

        if *raised_exception {
            return false;
        }

        if size > 0 && !chunk.is_null() {
            chunk_info.index = index;
            chunk_info.data = chunk;
            chunk_info.size = size;
            return true;
        }
    }

    false
}

// ---------------------------------------------------------------------------
// Blob helpers (little-endian, memcpy-equivalent)
// ---------------------------------------------------------------------------

fn write_u32_le(cursor: *mut u8, value: u32) {
    unsafe { std::ptr::copy_nonoverlapping(value.to_le_bytes().as_ptr(), cursor, 4) };
}

fn write_f32_le(cursor: *mut u8, value: f32) {
    unsafe { std::ptr::copy_nonoverlapping(value.to_le_bytes().as_ptr(), cursor, 4) };
}

fn read_u32_le(cursor: *const u8) -> u32 {
    let mut bytes = [0u8; 4];
    unsafe { std::ptr::copy_nonoverlapping(cursor, bytes.as_mut_ptr(), 4) };
    u32::from_le_bytes(bytes)
}

fn read_f32_le(cursor: *const u8) -> f32 {
    let mut bytes = [0u8; 4];
    unsafe { std::ptr::copy_nonoverlapping(cursor, bytes.as_mut_ptr(), 4) };
    f32::from_le_bytes(bytes)
}

// ---------------------------------------------------------------------------
// Export bodies
// ---------------------------------------------------------------------------

unsafe fn load_plugin_impl(host: &mut Vst2HostInstance, plugin_path: *const u16) -> i32 {
    host.close_plugin();

    let attributes = GetFileAttributesW(PCWSTR(plugin_path));
    if attributes == INVALID_FILE_ATTRIBUTES || (attributes & FILE_ATTRIBUTE_DIRECTORY.0) != 0 {
        return host.fail(K_ERROR_FILE_NOT_FOUND, "VST2 plugin DLL was not found.");
    }

    let module = match LoadLibraryW(PCWSTR(plugin_path)) {
        Ok(module) => module,
        Err(error) => {
            let code = (error.code().0 as u32) & 0xFFFF;
            let message = format!("Unable to load VST2 DLL: {}", get_windows_error_message(code));
            return host.fail(K_ERROR_LOAD_LIBRARY, &message);
        }
    };

    // Hosts never unload plugin DLLs — Reaper, Cubase and Ableton keep
    // modules resident for the whole process lifetime. We used to
    // FreeLibrary on every close, which unloads and reloads a 54 MB Rust
    // plugin DLL on each start/stop; that segfaulted on about the fourth
    // reload (static/TLS/destructor state). Pin the module so it loads
    // exactly once per process; per-instance state still tears down.
    unsafe {
        let mut pinned = HMODULE::default();
        let _ = GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_PIN,
            PCWSTR(plugin_path),
            &mut pinned,
        );
    }

    let entry_point = GetProcAddress(module, PCSTR(c"VSTPluginMain".as_ptr() as *const u8))
        .or_else(|| GetProcAddress(module, PCSTR(c"main".as_ptr() as *const u8)));

    let Some(entry_point) = entry_point else {
        let _ = FreeLibrary(module);
        return host.fail(
            K_ERROR_ENTRY_POINT,
            "VST2 DLL does not export VSTPluginMain or main.",
        );
    };

    clear_audio_master_opcodes();
    let mut entry_point_raised_exception = false;
    let entry_point: VstPluginMainProc = std::mem::transmute(entry_point);
    let effect = call_entry_point_safely(entry_point, &mut entry_point_raised_exception);

    if entry_point_raised_exception {
        let _ = FreeLibrary(module);
        return host.fail(
            K_ERROR_PLUGIN_LOAD,
            "VST2 entrypoint raised a native exception.",
        );
    }

    if effect.is_null() {
        let _ = FreeLibrary(module);
        let message = format!(
            "VST2 entrypoint returned null AEffect. audioMaster opcodes: {}; Cockos requests: {}",
            format_audio_master_opcodes(),
            format_cockos_function_requests()
        );
        return host.fail(K_ERROR_PLUGIN_LOAD, &message);
    }

    if (*effect).magic != K_EFFECT_MAGIC {
        let _ = FreeLibrary(module);
        return host.fail(K_ERROR_PLUGIN_LOAD, "VST2 AEffect magic is invalid.");
    }

    if (*effect).dispatcher.is_none() {
        let _ = FreeLibrary(module);
        return host.fail(
            K_ERROR_PLUGIN_LOAD,
            "VST2 plugin does not provide a dispatcher.",
        );
    }

    host.module = module;
    host.effect = effect;

    call_dispatcher_unchecked(host.effect, EFF_OPEN, 0, 0, std::ptr::null_mut(), 0.0);
    host.opened = true;
    read_plugin_strings(host);
    host.set_ok();

    K_OK
}

unsafe fn setup_processing_impl(
    host: &mut Vst2HostInstance,
    sample_rate: f64,
    max_block_size: i32,
    input_channels: i32,
    output_channels: i32,
) -> i32 {
    if host.effect.is_null() {
        return host.fail(K_ERROR_NOT_CONFIGURED, "No VST2 plugin has been loaded.");
    }

    if sample_rate <= 0.0 || max_block_size <= 0 || input_channels <= 0 || output_channels <= 0 {
        return host.fail(K_ERROR_INVALID_ARGUMENT, "Invalid VST2 processing setup.");
    }

    if (*host.effect).process_replacing.is_none() {
        return host.fail(
            K_ERROR_UNSUPPORTED,
            "VST2 plugin does not provide processReplacing.",
        );
    }

    if (*host.effect).num_inputs < input_channels || (*host.effect).num_outputs < output_channels {
        return host.fail(
            K_ERROR_UNSUPPORTED,
            "VST2 plugin channel layout is unsupported.",
        );
    }

    host.input_channels = input_channels;
    host.output_channels = output_channels;
    host.processing_configured = true;

    call_dispatcher_unchecked(
        host.effect,
        EFF_SET_SAMPLE_RATE,
        0,
        0,
        std::ptr::null_mut(),
        sample_rate as f32,
    );
    call_dispatcher_unchecked(
        host.effect,
        EFF_SET_BLOCK_SIZE,
        0,
        max_block_size as isize,
        std::ptr::null_mut(),
        0.0,
    );
    call_dispatcher_unchecked(
        host.effect,
        EFF_MAINS_CHANGED,
        0,
        1,
        std::ptr::null_mut(),
        0.0,
    );
    host.set_ok();

    K_OK
}

unsafe fn process_float32_impl(
    host: &mut Vst2HostInstance,
    input_interleaved: *const f32,
    output_interleaved: *mut f32,
    frame_count: i32,
) -> i32 {
    if input_interleaved.is_null() || output_interleaved.is_null() || frame_count < 0 {
        return K_ERROR_INVALID_ARGUMENT;
    }

    if !host.processing_configured {
        return host.fail(
            K_ERROR_NOT_CONFIGURED,
            "VST2 processing has not been configured.",
        );
    }

    if frame_count == 0 {
        host.set_ok();
        return K_OK;
    }

    if host.effect.is_null() || (*host.effect).process_replacing.is_none() {
        return host.fail(
            K_ERROR_NOT_CONFIGURED,
            "VST2 processReplacing is not available.",
        );
    }

    let plugin_input_channels = host.input_channels.max((*host.effect).num_inputs);
    let plugin_output_channels = host.output_channels.max((*host.effect).num_outputs);
    let frames = frame_count as usize;

    let mut input_channels: Vec<Vec<f32>> =
        vec![vec![0.0f32; frames]; plugin_input_channels as usize];
    let mut output_channels: Vec<Vec<f32>> =
        vec![vec![0.0f32; frames]; plugin_output_channels as usize];

    let mut input_pointers: Vec<*mut f32> = input_channels
        .iter_mut()
        .map(|channel| channel.as_mut_ptr())
        .collect();
    let mut output_pointers: Vec<*mut f32> = output_channels
        .iter_mut()
        .map(|channel| channel.as_mut_ptr())
        .collect();

    for frame in 0..frames {
        let input_frame_offset = frame * host.input_channels as usize;
        for channel in 0..host.input_channels as usize {
            input_channels[channel][frame] = *input_interleaved.add(input_frame_offset + channel);
        }
    }

    let mut process_raised_exception = false;
    call_process_replacing_safely(
        host.effect,
        input_pointers.as_mut_ptr(),
        output_pointers.as_mut_ptr(),
        frame_count,
        &mut process_raised_exception,
    );

    if process_raised_exception {
        return host.fail(
            K_ERROR_PLUGIN_LOAD,
            "VST2 processReplacing raised a native exception.",
        );
    }

    for frame in 0..frames {
        let output_frame_offset = frame * host.output_channels as usize;
        for channel in 0..host.output_channels as usize {
            *output_interleaved.add(output_frame_offset + channel) = output_channels[channel][frame];
        }
    }

    host.set_ok();

    K_OK
}

unsafe fn save_state_impl(host: &mut Vst2HostInstance, buffer: *mut u8, buffer_size: i32) -> i32 {
    if buffer_size < 0 {
        return host.fail(
            K_ERROR_INVALID_ARGUMENT,
            "State buffer size must not be negative.",
        );
    }

    if host.effect.is_null() || !host.opened {
        return host.fail(K_ERROR_NOT_CONFIGURED, "No VST2 plugin has been loaded.");
    }

    let mut dispatcher_raised_exception = false;
    let mut chunk_info = Vst2ChunkInfo::new();
    if try_get_chunk(host, &mut chunk_info, &mut dispatcher_raised_exception) {
        let total_size = 12 + chunk_info.size as usize;
        if buffer.is_null() {
            host.set_ok();
            return total_size as i32;
        }

        if (buffer_size as usize) < total_size {
            return host.fail(K_ERROR_INVALID_ARGUMENT, "State buffer is too small.");
        }

        let mut cursor = buffer;
        write_u32_le(cursor, K_CHUNK_STATE_BLOB_MAGIC);
        cursor = cursor.add(4);
        write_u32_le(cursor, K_STATE_BLOB_VERSION);
        cursor = cursor.add(4);
        write_u32_le(cursor, chunk_info.size as u32);
        cursor = cursor.add(4);
        std::ptr::copy_nonoverlapping(
            chunk_info.data as *const u8,
            cursor,
            chunk_info.size as usize,
        );

        host.set_ok();
        return total_size as i32;
    }

    if dispatcher_raised_exception {
        return host.fail(
            K_ERROR_PLUGIN_LOAD,
            "VST2 chunk query raised a native exception.",
        );
    }

    // No chunk support: fall back to a plain parameter value dump so the
    // plugin still restores its sound from a saved profile.
    if (*host.effect).num_params <= 0
        || (*host.effect).get_parameter.is_none()
        || (*host.effect).set_parameter.is_none()
    {
        host.set_ok();
        return 0;
    }

    let param_count = (*host.effect).num_params as u32;
    let total_size = 12 + param_count as usize * std::mem::size_of::<f32>();
    if buffer.is_null() {
        host.set_ok();
        return total_size as i32;
    }

    if (buffer_size as usize) < total_size {
        return host.fail(K_ERROR_INVALID_ARGUMENT, "State buffer is too small.");
    }

    let mut cursor = buffer;
    write_u32_le(cursor, K_PARAM_STATE_BLOB_MAGIC);
    cursor = cursor.add(4);
    write_u32_le(cursor, K_STATE_BLOB_VERSION);
    cursor = cursor.add(4);
    write_u32_le(cursor, param_count);
    cursor = cursor.add(4);

    let get_parameter = (*host.effect).get_parameter.expect("checked above");
    for index in 0..param_count {
        let value = get_parameter(host.effect, index as i32);
        write_f32_le(cursor, value);
        cursor = cursor.add(4);
    }

    host.set_ok();
    total_size as i32
}

unsafe fn load_state_impl(host: &mut Vst2HostInstance, buffer: *const u8, buffer_size: i32) -> i32 {
    if buffer.is_null() || buffer_size <= 0 {
        return host.fail(K_ERROR_INVALID_ARGUMENT, "State buffer is required.");
    }

    if host.effect.is_null() || !host.opened {
        return host.fail(K_ERROR_NOT_CONFIGURED, "No VST2 plugin has been loaded.");
    }

    if (buffer_size as usize) < 12 {
        return host.fail(K_ERROR_INVALID_ARGUMENT, "State buffer is too small.");
    }

    let mut cursor = buffer;
    let magic = read_u32_le(cursor);
    cursor = cursor.add(4);
    let version = read_u32_le(cursor);
    cursor = cursor.add(4);
    let payload_size = read_u32_le(cursor);
    cursor = cursor.add(4);

    if version != K_STATE_BLOB_VERSION {
        return host.fail(
            K_ERROR_INVALID_ARGUMENT,
            "State buffer version is not supported.",
        );
    }

    if magic == K_PARAM_STATE_BLOB_MAGIC || magic == K_LEGACY_PARAM_STATE_BLOB_MAGIC {
        if (buffer_size as usize) < 12 + payload_size as usize * std::mem::size_of::<f32>()
            || payload_size == 0
        {
            return host.fail(K_ERROR_INVALID_ARGUMENT, "State buffer is truncated.");
        }

        let Some(set_parameter) = (*host.effect).set_parameter else {
            return host.fail(
                K_ERROR_UNSUPPORTED,
                "VST2 plugin does not accept parameter changes.",
            );
        };

        let available_params = (*host.effect).num_params;
        for index in 0..payload_size {
            let value = read_f32_le(cursor);
            cursor = cursor.add(4);

            if (index as i32) < available_params {
                let _ = guard_result(|| set_parameter(host.effect, index as i32, value));
            }
        }

        host.set_ok();
        return K_OK;
    }

    if magic != K_CHUNK_STATE_BLOB_MAGIC && magic != K_LEGACY_CHUNK_STATE_BLOB_MAGIC {
        return host.fail(
            K_ERROR_INVALID_ARGUMENT,
            "State buffer is not a recognized VST-Applier state blob.",
        );
    }

    if (buffer_size as usize) < 12 + payload_size as usize {
        return host.fail(K_ERROR_INVALID_ARGUMENT, "State buffer is truncated.");
    }

    let mut dispatcher_raised_exception = false;
    let mut chunk_info = Vst2ChunkInfo::new();
    if !try_get_chunk(host, &mut chunk_info, &mut dispatcher_raised_exception) {
        if dispatcher_raised_exception {
            return host.fail(
                K_ERROR_PLUGIN_LOAD,
                "VST2 chunk query raised a native exception.",
            );
        }

        return host.fail(
            K_ERROR_UNSUPPORTED,
            "VST2 plugin does not support state chunks.",
        );
    }

    let mut set_chunk_raised_exception = false;
    call_dispatcher_safely(
        host.effect,
        EFF_SET_CHUNK,
        chunk_info.index,
        payload_size as isize,
        cursor as *mut c_void,
        0.0,
        &mut set_chunk_raised_exception,
    );

    if set_chunk_raised_exception {
        return host.fail(
            K_ERROR_PLUGIN_LOAD,
            "VST2 effSetChunk raised a native exception.",
        );
    }

    host.set_ok();
    K_OK
}

unsafe fn open_editor_impl(host: &mut Vst2HostInstance, parent_hwnd: *mut c_void) -> i32 {
    if host.effect.is_null() {
        return host.fail(K_ERROR_NOT_CONFIGURED, "No VST2 plugin has been loaded.");
    }

    if ((*host.effect).flags & EFF_FLAGS_HAS_EDITOR) == 0 {
        return host.fail(K_ERROR_UNSUPPORTED, "VST2 plugin does not report an editor.");
    }

    if parent_hwnd.is_null() {
        return host.fail(
            K_ERROR_INVALID_ARGUMENT,
            "VST2 editor parent window handle is null.",
        );
    }

    call_dispatcher_unchecked(host.effect, EFF_EDIT_OPEN, 0, 0, parent_hwnd, 0.0);
    host.editor_open = true;
    host.editor_parent = parent_hwnd;
    set_size_window_parent(host.editor_parent);

    // The pre-open rect is often a placeholder: re-query after open
    // and fit the parent to whatever the plugin reports now.
    let mut post_rect: *mut ERect = std::ptr::null_mut();
    let mut post_raised = false;
    call_dispatcher_safely(
        host.effect,
        EFF_EDIT_GET_RECT,
        0,
        0,
        &mut post_rect as *mut *mut ERect as *mut c_void,
        0.0,
        &mut post_raised,
    );
    if !post_raised && !post_rect.is_null() {
        let rect = &*post_rect;
        resize_editor_parent(
            HWND(host.editor_parent),
            (rect.right - rect.left) as i32,
            (rect.bottom - rect.top) as i32,
        );
    }
    host.set_ok();

    K_OK
}

unsafe fn get_editor_size_impl(
    host: &mut Vst2HostInstance,
    width: *mut i32,
    height: *mut i32,
) -> i32 {
    *width = 0;
    *height = 0;

    if host.effect.is_null() || (*host.effect).dispatcher.is_none() {
        return host.fail(K_ERROR_NOT_CONFIGURED, "No VST2 plugin has been loaded.");
    }

    let mut editor_rect: *mut ERect = std::ptr::null_mut();
    let mut dispatcher_raised_exception = false;
    call_dispatcher_safely(
        host.effect,
        EFF_EDIT_GET_RECT,
        0,
        0,
        &mut editor_rect as *mut *mut ERect as *mut c_void,
        0.0,
        &mut dispatcher_raised_exception,
    );

    if dispatcher_raised_exception {
        return host.fail(
            K_ERROR_PLUGIN_LOAD,
            "VST2 effEditGetRect raised a native exception.",
        );
    }

    if editor_rect.is_null() {
        return host.fail(K_ERROR_UNSUPPORTED, "VST2 editor did not provide a size.");
    }

    let rect = &*editor_rect;
    let editor_width = (rect.right - rect.left) as i32;
    let editor_height = (rect.bottom - rect.top) as i32;
    if editor_width <= 0 || editor_height <= 0 {
        return host.fail(K_ERROR_UNSUPPORTED, "VST2 editor size is invalid.");
    }

    *width = editor_width;
    *height = editor_height;
    host.set_ok();

    K_OK
}

// ---------------------------------------------------------------------------
// Exported C ABI (identical to Vst2HostNative.h)
// ---------------------------------------------------------------------------

/// `Vst2Host_GetApiVersion` — always 1.
#[no_mangle]
pub unsafe extern "C" fn Vst2Host_GetApiVersion() -> i32 {
    1
}

#[no_mangle]
pub unsafe extern "C" fn Vst2Host_Create() -> Vst2HostInstanceHandle {
    guard(std::ptr::null_mut(), || {
        Box::into_raw(Box::new(Vst2HostInstance::new())) as Vst2HostInstanceHandle
    })
}

#[no_mangle]
pub unsafe extern "C" fn Vst2Host_Destroy(host: Vst2HostInstanceHandle) {
    if host.is_null() {
        return;
    }

    guard((), || {
        drop(Box::from_raw(host as *mut Vst2HostInstance));
    });
}

#[no_mangle]
pub unsafe extern "C" fn Vst2Host_LoadPlugin(
    host: Vst2HostInstanceHandle,
    plugin_path: *const u16,
) -> i32 {
    guard_i32(|| {
        if host.is_null() || plugin_path.is_null() || *plugin_path == 0 {
            return K_ERROR_INVALID_ARGUMENT;
        }

        let native_host = &mut *(host as *mut Vst2HostInstance);
        load_plugin_impl(native_host, plugin_path)
    })
}

#[no_mangle]
pub unsafe extern "C" fn Vst2Host_SetupProcessing(
    host: Vst2HostInstanceHandle,
    sample_rate: f64,
    max_block_size: i32,
    input_channels: i32,
    output_channels: i32,
) -> i32 {
    guard_i32(|| {
        if host.is_null() {
            return K_ERROR_INVALID_ARGUMENT;
        }

        let native_host = &mut *(host as *mut Vst2HostInstance);
        setup_processing_impl(
            native_host,
            sample_rate,
            max_block_size,
            input_channels,
            output_channels,
        )
    })
}

#[no_mangle]
pub unsafe extern "C" fn Vst2Host_ProcessFloat32(
    host: Vst2HostInstanceHandle,
    input_interleaved: *const f32,
    output_interleaved: *mut f32,
    frame_count: i32,
) -> i32 {
    guard_i32(|| {
        if host.is_null() {
            return K_ERROR_INVALID_ARGUMENT;
        }

        let native_host = &mut *(host as *mut Vst2HostInstance);
        process_float32_impl(native_host, input_interleaved, output_interleaved, frame_count)
    })
}

/// Two-step state read: `buffer = NULL, capacity = 0` returns the total byte
/// size (0 = the plugin has no state), otherwise the blob is written and the
/// byte count returned. Negative = error.
#[no_mangle]
pub unsafe extern "C" fn Vst2Host_SaveState(
    host: Vst2HostInstanceHandle,
    buffer: *mut u8,
    capacity: i32,
) -> i32 {
    guard_i32(|| {
        if host.is_null() {
            return K_ERROR_INVALID_ARGUMENT;
        }

        let native_host = &mut *(host as *mut Vst2HostInstance);
        save_state_impl(native_host, buffer, capacity)
    })
}

#[no_mangle]
pub unsafe extern "C" fn Vst2Host_LoadState(
    host: Vst2HostInstanceHandle,
    data: *const u8,
    size: i32,
) -> i32 {
    guard_i32(|| {
        if host.is_null() {
            return K_ERROR_INVALID_ARGUMENT;
        }

        let native_host = &mut *(host as *mut Vst2HostInstance);
        load_state_impl(native_host, data, size)
    })
}

#[no_mangle]
pub unsafe extern "C" fn Vst2Host_OpenEditor(
    host: Vst2HostInstanceHandle,
    parent_hwnd: *mut c_void,
) -> i32 {
    guard_i32(|| {
        if host.is_null() {
            return K_ERROR_INVALID_ARGUMENT;
        }

        let native_host = &mut *(host as *mut Vst2HostInstance);
        open_editor_impl(native_host, parent_hwnd)
    })
}

#[no_mangle]
pub unsafe extern "C" fn Vst2Host_GetEditorSize(
    host: Vst2HostInstanceHandle,
    width: *mut i32,
    height: *mut i32,
) -> i32 {
    guard_i32(|| {
        if host.is_null() || width.is_null() || height.is_null() {
            return K_ERROR_INVALID_ARGUMENT;
        }

        let native_host = &mut *(host as *mut Vst2HostInstance);
        get_editor_size_impl(native_host, width, height)
    })
}

#[no_mangle]
pub unsafe extern "C" fn Vst2Host_EditorIdle(host: Vst2HostInstanceHandle) {
    guard((), || {
        if host.is_null() {
            return;
        }

        let native_host = &mut *(host as *mut Vst2HostInstance);
        if native_host.effect.is_null()
            || (*native_host.effect).dispatcher.is_none()
            || !native_host.editor_open
        {
            return;
        }

        let mut dispatcher_raised_exception = false;
        call_dispatcher_safely(
            native_host.effect,
            EFF_EDIT_IDLE,
            0,
            0,
            std::ptr::null_mut(),
            0.0,
            &mut dispatcher_raised_exception,
        );
    });
}

#[no_mangle]
pub unsafe extern "C" fn Vst2Host_CloseEditor(host: Vst2HostInstanceHandle) {
    guard((), || {
        if host.is_null() {
            return;
        }

        let native_host = &mut *(host as *mut Vst2HostInstance);
        if native_host.effect.is_null() || !native_host.editor_open {
            return;
        }

        call_dispatcher_unchecked(
            native_host.effect,
            EFF_EDIT_CLOSE,
            0,
            0,
            std::ptr::null_mut(),
            0.0,
        );
        native_host.editor_open = false;
        native_host.editor_parent = std::ptr::null_mut();
        set_size_window_parent(std::ptr::null_mut());
    });
}

#[no_mangle]
pub unsafe extern "C" fn Vst2Host_GetLastError(host: Vst2HostInstanceHandle) -> *const u16 {
    if host.is_null() {
        return invalid_handle_message();
    }

    guard(invalid_handle_message(), || {
        let native_host = &*(host as *const Vst2HostInstance);
        native_host.last_error.as_ptr()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deliberately invalid pointer read must come back as `Err` instead of
    /// taking the test process down: proves the SEH guard is armed.
    #[test]
    fn seh_guard_catches_hardware_fault() {
        let result = guard_result(|| unsafe {
            core::ptr::read_volatile(core::mem::align_of::<i32>() as *const i32)
        });
        assert!(result.is_err(), "hardware fault must be caught");
        assert_eq!(guard(7, || 1 + 1), 2, "guard still passes values through");
    }

    /// Rust panics must be caught by the same guard.
    #[test]
    fn guard_catches_rust_panic() {
        let result = guard_result(|| -> i32 { panic!("intentional test panic") });
        assert!(result.is_err());
    }
}
