//! 100% Rust reimplementation of the C++ `Vst3HostNative.dll`
//! (`origin/csharp-legacy:Vst3HostNative/Vst3HostNative.cpp`), with the
//! exact same exports, semantics and state-blob layouts:
//!
//! * little-endian `[u32 magic | u32 version=1 | payload]` blobs,
//!   `VSTA` = component + edit-controller state pair,
//!   `VSTP` = parameter-dump fallback;
//! * return convention `0 = ok, negative = error`, details via
//!   `Vst3Host_GetLastError` (UTF-16, valid until the next host call);
//! * no Rust panic and no hardware exception may cross the DLL boundary:
//!   every plugin-facing export runs under a Windows SEH guard (`__try/__except`,
//!   the tiny C stub shipped with the `microseh` crate) plus a `catch_unwind`
//!   trap, so a plugin that faults is reported as an error, not a crash.
#![allow(non_snake_case)]
#![allow(clippy::missing_safety_doc)]

mod module;
mod objects;
mod plugprovider;
mod vst3abi;

use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};

use module::{ClassInfo, Module, PluginFactory};
use objects::{
    ComponentHandlerObj, HostApplicationObj, MemoryStreamObj, ParameterChangesObj, PlugFrameObj,
};
use plugprovider::{PlugProvider, PluginContextFactory};
use vst3abi::*;

// ---------------------------------------------------------------------------
// Error codes (mirror of the C++ anonymous-namespace constants)
// ---------------------------------------------------------------------------

const K_SUCCESS: i32 = 0;
const K_ERROR_INVALID_HOST: i32 = -1;
const K_ERROR_INVALID_ARGUMENT: i32 = -2;
const K_ERROR_FILE_NOT_FOUND: i32 = -3;
const K_ERROR_UNSUPPORTED: i32 = -4;
const K_ERROR_NOT_CONFIGURED: i32 = -5;
#[allow(dead_code)]
const K_ERROR_NOT_IMPLEMENTED: i32 = -6;
const K_ERROR_PLUGIN_LOAD: i32 = -7;
const K_ERROR_NO_FACTORY: i32 = -8;
const K_ERROR_NO_AUDIO_EFFECT: i32 = -9;
const K_ERROR_NO_EDITOR: i32 = -10;
const K_ERROR_EDITOR_ATTACH: i32 = -11;

const K_STATE_BLOB_MAGIC: u32 = 0x41545356; // 'VSTA'
const K_PARAM_STATE_BLOB_MAGIC: u32 = 0x50545356; // 'VSTP'
const K_LEGACY_STATE_BLOB_MAGIC: u32 = 0x534A4E53; // legacy (v1.3) state magic
const K_LEGACY_PARAM_STATE_BLOB_MAGIC: u32 = 0x514A4E53; // legacy (v1.3) param magic
const K_STATE_BLOB_VERSION: u32 = 1;
const K_MAX_STATE_BLOB_SIZE: u64 = 64 * 1024 * 1024;

pub type Vst3HostInstanceHandle = *mut c_void;

/// UTF-16 "Invalid host handle." (static, mirrors kNullHostError).
const NULL_HOST_ERROR_LEN: usize = 21;
static NULL_HOST_ERROR: [u16; NULL_HOST_ERROR_LEN] = build_utf16(b"Invalid host handle.");

const fn build_utf16<const N: usize>(s: &[u8]) -> [u16; N] {
    let mut out = [0u16; N];
    let mut i = 0;
    while i < N {
        if i < s.len() {
            out[i] = s[i] as u16;
        }
        i += 1;
    }
    out
}

// ---------------------------------------------------------------------------
// Isolation helpers: Rust panics + Windows hardware exceptions
// ---------------------------------------------------------------------------
//
// Every call into plugin code runs inside two nested guards:
//   * `microseh::try_seh` — the Windows SEH safety net (`__try __except`,
//     provided by microseh's tiny C stub): a plugin faulting with a hardware
//     exception (access violation, illegal instruction, ...) is caught and
//     reported as an error instead of taking the process down.
//   * `catch_unwind` — Rust panics.
// The panic trap is outermost and the SEH guard innermost, so a hardware
// fault is always contained by the C stub before Rust unwind machinery sees it.

/// Why a guarded call failed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GuardFailure {
    /// The plugin (or host code) panicked.
    Panic,
    /// A hardware exception (access violation, illegal instruction, ...).
    Hardware,
}

/// Run `f` inside the SEH guard + panic trap.
///
/// Nesting matters: the panic trap is *innermost* and the SEH guard
/// outermost, so a Rust panic is caught before it could unwind through the
/// `extern "C"` C stub (which would abort the process), while a hardware
/// exception unwinds past the panic trap into the C stub's `__except`.
#[inline]
pub fn guarded<T, F: FnOnce() -> T>(f: F) -> Result<T, GuardFailure> {
    let mut f = Some(f);
    match microseh::try_seh(|| catch_unwind(AssertUnwindSafe(|| (f.take().unwrap())()))) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(_)) => Err(GuardFailure::Panic),
        Err(_) => Err(GuardFailure::Hardware),
    }
}

/// Runs `f`, converting any failure into `fallback`. Used by every vtable
/// trampoline the plugin can call through.
#[inline]
pub fn catch_panic_i32(fallback: i32, f: impl FnOnce() -> i32) -> i32 {
    match guarded(f) {
        Ok(value) => value,
        Err(_) => fallback,
    }
}

// ---------------------------------------------------------------------------
// Vst3HostInstance
// ---------------------------------------------------------------------------

struct EditorState {
    plug_view: Option<InterfacePtr<IPlugViewObj>>,
    plug_frame: Option<Box<PlugFrameObj>>,
    controller: Option<InterfacePtr<IEditControllerObj>>,
    attached: bool,
}

/// `CloseEditorState (EditorState&)` + `CloseEditorState (unique_ptr&)`.
fn close_editor_state(editor: &mut Option<Box<EditorState>>) {
    if let Some(state) = editor.as_mut() {
        if let Some(plug_view) = &state.plug_view {
            unsafe {
                plug_view.set_frame(std::ptr::null_mut());
                if state.attached {
                    plug_view.removed();
                    state.attached = false;
                }
            }
        }
        state.plug_view = None;
        state.plug_frame = None;
        state.controller = None;
    }
    *editor = None;
}

struct ProcessingState {
    component: Option<InterfacePtr<IComponentObj>>,
    processor: Option<InterfacePtr<IAudioProcessorObj>>,
    process_context: ProcessContext,
    process_data: ProcessData,
    /// Owned `IParameterChanges` reference (mirrors the `ParameterChanges`
    /// value member + its `IPtr` teardown semantics).
    input_parameter_changes: *mut ParameterChangesObj,
    input_buses: [AudioBusBuffers; 1],
    output_buses: [AudioBusBuffers; 1],
    input_channel_pointers: [*mut f32; 2],
    output_channel_pointers: [*mut f32; 2],
    input_buffers: [Vec<f32>; 2],
    output_buffers: [Vec<f32>; 2],
    sample_rate: f64,
    max_block_size: i32,
    input_channels: i32,
    output_channels: i32,
    active: bool,
    processing: bool,
}

impl ProcessingState {
    /// Allocates the state on the heap (never moved afterwards: the
    /// ProcessData/AudioBusBuffers pointers refer into this allocation).
    fn create() -> Box<Self> {
        let mut state = Box::new(Self {
            component: None,
            processor: None,
            process_context: ProcessContext::default(),
            process_data: unsafe { std::mem::zeroed() },
            input_parameter_changes: ParameterChangesObj::create(1000),
            input_buses: [AudioBusBuffers {
                num_channels: 0,
                silence_flags: 0,
                channel_buffers32: std::ptr::null_mut(),
            }],
            output_buses: [AudioBusBuffers {
                num_channels: 0,
                silence_flags: 0,
                channel_buffers32: std::ptr::null_mut(),
            }],
            input_channel_pointers: [std::ptr::null_mut(); 2],
            output_channel_pointers: [std::ptr::null_mut(); 2],
            input_buffers: [Vec::new(), Vec::new()],
            output_buffers: [Vec::new(), Vec::new()],
            sample_rate: 0.0,
            max_block_size: 0,
            input_channels: 0,
            output_channels: 0,
            active: false,
            processing: false,
        });
        // Wire the intra-struct pointers once the allocation is stable.
        let this: *mut Self = &mut *state;
        unsafe {
            (*this).process_data.inputs = (*this).input_buses.as_mut_ptr();
            (*this).process_data.outputs = (*this).output_buses.as_mut_ptr();
            (*this).process_data.input_parameter_changes =
                (*this).input_parameter_changes as *mut c_void;
            (*this).process_data.process_context = &mut (*this).process_context as *mut _;
        }
        state
    }

    /// Pins the per-channel buffer pointers (buffers never resize after
    /// `SetupProcessing`, so the pointers stay valid).
    fn wire_channel_pointers(&mut self) {
        for channel in 0..2 {
            self.input_channel_pointers[channel] = self.input_buffers[channel].as_mut_ptr();
            self.output_channel_pointers[channel] = self.output_buffers[channel].as_mut_ptr();
        }
        self.input_buses[0].channel_buffers32 = self.input_channel_pointers.as_mut_ptr();
        self.output_buses[0].channel_buffers32 = self.output_channel_pointers.as_mut_ptr();
    }
}

impl Drop for ProcessingState {
    fn drop(&mut self) {
        // StopProcessingState + member teardown.
        if let Some(processor) = &self.processor {
            if self.processing {
                unsafe {
                    processor.set_processing(0);
                }
            }
        }
        self.processing = false;

        if let Some(component) = &self.component {
            if self.active {
                unsafe {
                    component.set_active(0);
                }
            }
        }
        self.active = false;

        if !self.input_parameter_changes.is_null() {
            unsafe {
                com_release(self.input_parameter_changes);
            }
            self.input_parameter_changes = std::ptr::null_mut();
        }
    }
}

struct Vst3HostInstance {
    plugin_path: Vec<u16>,
    last_error: Vec<u16>,
    sample_rate: f64,
    max_block_size: i32,
    input_channels: i32,
    output_channels: i32,
    processing_configured: bool,
    module: Option<Module>,
    selected_effect_class: Option<ClassInfo>,
    has_selected_effect_class: bool,
    selected_effect_class_id: String,
    selected_effect_class_name: String,
    plugin_context: Box<HostApplicationObj>,
    component_handler: Box<ComponentHandlerObj>,
    plug_provider: Option<Box<PlugProvider>>,
    editor: Option<Box<EditorState>>,
    processing: Option<Box<ProcessingState>>,
}

impl Vst3HostInstance {
    fn new() -> Self {
        Self {
            plugin_path: Vec::new(),
            last_error: vec![0],
            sample_rate: 0.0,
            max_block_size: 0,
            input_channels: 0,
            output_channels: 0,
            processing_configured: false,
            module: None,
            selected_effect_class: None,
            has_selected_effect_class: false,
            selected_effect_class_id: String::new(),
            selected_effect_class_name: String::new(),
            plugin_context: HostApplicationObj::create(),
            component_handler: ComponentHandlerObj::create(),
            plug_provider: None,
            editor: None,
            processing: None,
        }
    }

    fn clear_error(&mut self) {
        self.last_error = vec![0];
    }

    /// `Fail (code, message)`.
    fn fail(&mut self, code: i32, message: &str) -> i32 {
        self.set_error(message);
        code
    }

    fn set_error(&mut self, message: &str) {
        if message.is_empty() {
            // "Unknown error."
            self.last_error = "Unknown error.".encode_utf16().chain([0]).collect();
            return;
        }
        let mut units: Vec<u16> = message.encode_utf16().collect();
        units.push(0);
        self.last_error = units;
    }

    fn plugin_context_ptr(&mut self) -> *mut c_void {
        HostApplicationObj::as_ptr(&mut self.plugin_context)
    }

    fn component_handler_ptr(&mut self) -> *mut c_void {
        ComponentHandlerObj::as_ptr(&mut self.component_handler)
    }

    /// `ResetPlugin ()`.
    fn reset_plugin(&mut self) {
        self.component_handler.clear_pending_changes();
        close_editor_state(&mut self.editor);
        self.processing = None;
        self.plug_provider = None;
        self.reset_processing_configuration();
        clear_plugin_context_if_owned(self);
        self.has_selected_effect_class = false;
        self.selected_effect_class_id.clear();
        self.selected_effect_class_name.clear();
        self.selected_effect_class = None;
        self.module = None;
        self.plugin_path.clear();
    }

    /// `ResetProcessingConfiguration ()`.
    fn reset_processing_configuration(&mut self) {
        self.sample_rate = 0.0;
        self.max_block_size = 0;
        self.input_channels = 0;
        self.output_channels = 0;
        self.processing_configured = false;
    }

    /// Borrows the module factory as an owned reference (addRef'd), so the
    /// borrow of `self` ends before callers keep mutating the host.
    fn module_factory(&mut self) -> Option<PluginFactory> {
        let module = self.module.as_ref()?;
        if module.factory.is_null() {
            return None;
        }
        Some(unsafe { module.factory.clone_ptr() })
    }
}

/// `ClearPluginContextIfOwned ()`.
fn clear_plugin_context_if_owned(host: &mut Vst3HostInstance) {
    let factory = PluginContextFactory::instance();
    let context = host.plugin_context_ptr();
    if factory.plugin_context() == context {
        factory.set_plugin_context(std::ptr::null_mut());
    }
}

impl Drop for Vst3HostInstance {
    fn drop(&mut self) {
        // ~Vst3HostInstance: ClearHostParameterState (); CloseEditorState
        // (editor); CloseProcessingState (processing); plugProvider =
        // nullptr; ClearPluginContextIfOwned (). The explicit calls keep the
        // C++ teardown order even though Rust would otherwise drop the
        // provider field before the processing/editor states.
        self.component_handler.clear_pending_changes();
        close_editor_state(&mut self.editor);
        self.processing = None;
        self.plug_provider = None;
        clear_plugin_context_if_owned(self);
    }
}

fn from_handle(host: Vst3HostInstanceHandle) -> *mut Vst3HostInstance {
    host as *mut Vst3HostInstance
}

// ---------------------------------------------------------------------------
// Plugin provider helper
// ---------------------------------------------------------------------------

/// `EnsurePluginProvider (...)`.
fn ensure_plugin_provider(host: &mut Vst3HostInstance, factory: &PluginFactory) -> bool {
    if host.plug_provider.is_none() {
        let class_info = host.selected_effect_class.clone().unwrap_or_default();
        let mut provider = Box::new(PlugProvider::new(factory, class_info, true));
        if !provider.initialize() {
            return false;
        }
        host.plug_provider = Some(provider);
    }

    let handler = host.component_handler_ptr();
    if let Some(provider) = host.plug_provider.as_mut() {
        if let Some(controller) = provider.get_controller_ptr() {
            unsafe {
                controller.set_component_handler(handler);
            }
        }
    }

    true
}

// ---------------------------------------------------------------------------
// Helpers (path, strings, processing data)
// ---------------------------------------------------------------------------

fn wide_from_ptr(p: *const u16) -> Vec<u16> {
    let mut out = Vec::new();
    if p.is_null() {
        return out;
    }
    let mut i = 0isize;
    unsafe {
        while *p.offset(i) != 0 {
            out.push(*p.offset(i));
            i += 1;
        }
    }
    out
}

fn ends_with_vst3(path: &[u16]) -> bool {
    const K_EXTENSION_LENGTH: usize = 5;
    if path.len() < K_EXTENSION_LENGTH {
        return false;
    }
    let suffix = &path[path.len() - K_EXTENSION_LENGTH..];
    let extension: [u16; 5] = [b'.' as u16, b'v' as u16, b's' as u16, b't' as u16, b'3' as u16];
    suffix
        .iter()
        .zip(extension.iter())
        .all(|(a, b)| (*a as u8).to_ascii_lowercase() == (*b as u8).to_ascii_lowercase())
}

fn path_exists(path: &[u16]) -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{GetFileAttributesW, INVALID_FILE_ATTRIBUTES};

    let mut with_nul = path.to_vec();
    with_nul.push(0);
    let attributes = unsafe { GetFileAttributesW(PCWSTR(with_nul.as_ptr())) };
    attributes != INVALID_FILE_ATTRIBUTES
}

/// `WideToUtf8` — strict UTF-16 -> UTF-8 (throws in C++ on invalid input).
fn wide_to_utf8(value: &[u16]) -> Result<String, &'static str> {
    if value.is_empty() {
        return Ok(String::new());
    }
    String::from_utf16(value).map_err(|_| "Plugin path is not valid UTF-16.")
}

/// `Utf8ToWide` — lossy byte-passthrough on invalid UTF-8, as in C++.
fn utf8_to_wide(value: &str) -> Vec<u16> {
    value.encode_utf16().collect()
}

/// `BuildError (prefix, detail)`.
fn build_error(prefix: &str, detail: &str) -> Vec<u16> {
    let mut result: Vec<u16> = utf8_to_wide(prefix);
    if !detail.is_empty() {
        result.extend(utf8_to_wide(detail));
    }
    result
}

fn is_supported_channel_count(channel_count: i32) -> bool {
    channel_count == 1 || channel_count == 2
}

fn get_speaker_arrangement(channel_count: i32) -> u64 {
    if channel_count == 1 {
        K_MONO
    } else {
        K_STEREO
    }
}

fn has_audio_bus(component: &InterfacePtr<IComponentObj>, direction: i32) -> bool {
    unsafe { component.get_bus_count(K_AUDIO, direction) > 0 }
}

fn is_all_silent(input_interleaved: *const f32, frame_count: i32, channel_count: i32) -> bool {
    if input_interleaved.is_null() || frame_count <= 0 || channel_count <= 0 {
        return true;
    }
    let sample_count = frame_count as usize * channel_count as usize;
    for sample in 0..sample_count {
        if unsafe { *input_interleaved.add(sample) } != 0.0f32 {
            return false;
        }
    }
    true
}

/// `PrepareProcessingData (...)`.
fn prepare_processing_data(
    component_handler: &ComponentHandlerObj,
    processing: &mut ProcessingState,
    input_interleaved: *const f32,
    frame_count: i32,
) {
    unsafe {
        (*processing.input_parameter_changes).clear_queue();
    }
    component_handler.transfer_changes_to(unsafe { &mut *processing.input_parameter_changes });

    for channel in 0..processing.input_channels as usize {
        let channel_buffer = &mut processing.input_buffers[channel];
        for frame in 0..frame_count as usize {
            channel_buffer[frame] = unsafe {
                *input_interleaved.add(frame * processing.input_channels as usize + channel)
            };
        }
        processing.input_channel_pointers[channel] = channel_buffer.as_mut_ptr();
    }

    for channel in 0..processing.output_channels as usize {
        let channel_buffer = &mut processing.output_buffers[channel];
        channel_buffer[..frame_count as usize].fill(0.0f32);
        processing.output_channel_pointers[channel] = channel_buffer.as_mut_ptr();
    }

    processing.input_buses[0].num_channels = processing.input_channels;
    processing.input_buses[0].silence_flags =
        if is_all_silent(input_interleaved, frame_count, processing.input_channels) {
            K_ALL_CHANNELS_SILENT
        } else {
            0
        };
    processing.input_buses[0].channel_buffers32 = processing.input_channel_pointers.as_mut_ptr();

    processing.output_buses[0].num_channels = processing.output_channels;
    processing.output_buses[0].silence_flags = 0;
    processing.output_buses[0].channel_buffers32 = processing.output_channel_pointers.as_mut_ptr();

    processing.process_context.sample_rate = processing.sample_rate;

    processing.process_data.process_mode = K_REALTIME;
    processing.process_data.symbolic_sample_size = K_SAMPLE_32;
    processing.process_data.num_samples = frame_count;
    processing.process_data.num_inputs = 1;
    processing.process_data.num_outputs = 1;
    processing.process_data.inputs = processing.input_buses.as_mut_ptr();
    processing.process_data.outputs = processing.output_buses.as_mut_ptr();
    processing.process_data.input_parameter_changes =
        processing.input_parameter_changes as *mut c_void;
    processing.process_data.output_parameter_changes = std::ptr::null_mut();
    processing.process_data.input_events = std::ptr::null_mut();
    processing.process_data.output_events = std::ptr::null_mut();
    processing.process_data.process_context = &mut processing.process_context as *mut _;
}

/// `WriteProcessingOutput (...)`.
fn write_processing_output(
    processing: &ProcessingState,
    output_interleaved: *mut f32,
    frame_count: i32,
) {
    for frame in 0..frame_count as usize {
        for channel in 0..processing.output_channels as usize {
            unsafe {
                *output_interleaved.add(frame * processing.output_channels as usize + channel) =
                    processing.output_buffers[channel][frame];
            }
        }
    }
}

// ---------------------------------------------------------------------------
// State save/load
// ---------------------------------------------------------------------------

/// `SaveControllerParameterState (...)`.
fn save_controller_parameter_state(
    host: &mut Vst3HostInstance,
    controller: Option<&InterfacePtr<IEditControllerObj>>,
    buffer: *mut u8,
    buffer_size: i32,
) -> i32 {
    let Some(controller) = controller else {
        return host.fail(
            K_ERROR_PLUGIN_LOAD,
            "VST3 component state save failed and the plugin has no edit controller for a parameter fallback.",
        );
    };

    let parameter_count = unsafe { controller.get_parameter_count() };
    if parameter_count < 0 {
        return host.fail(
            K_ERROR_PLUGIN_LOAD,
            "VST3 component state save failed and the edit controller did not report its parameters.",
        );
    }

    let pair_size = std::mem::size_of::<u32>() + std::mem::size_of::<f64>();
    let total_size = 12 + parameter_count as usize * pair_size;

    if buffer.is_null() {
        host.clear_error();
        return total_size as i32;
    }

    if buffer_size < 0 || (buffer_size as usize) < total_size {
        return host.fail(K_ERROR_INVALID_ARGUMENT, "State buffer is too small.");
    }

    let count = parameter_count as u32;
    let mut cursor = buffer;
    unsafe {
        std::ptr::copy_nonoverlapping(K_PARAM_STATE_BLOB_MAGIC.to_le_bytes().as_ptr(), cursor, 4);
        cursor = cursor.add(4);
        std::ptr::copy_nonoverlapping(K_STATE_BLOB_VERSION.to_le_bytes().as_ptr(), cursor, 4);
        cursor = cursor.add(4);
        std::ptr::copy_nonoverlapping(count.to_le_bytes().as_ptr(), cursor, 4);
        cursor = cursor.add(4);
    }

    for index in 0..parameter_count {
        let mut info = ParameterInfo::default();
        let mut parameter_id: u32 = 0xFFFFFFFF;
        let mut value: f64 = 0.0;

        if unsafe { controller.get_parameter_info(index, &mut info) } == K_RESULT_TRUE {
            parameter_id = info.id;
            value = unsafe { controller.get_param_normalized(info.id) };
        }

        unsafe {
            std::ptr::copy_nonoverlapping(parameter_id.to_le_bytes().as_ptr(), cursor, 4);
            cursor = cursor.add(4);
            std::ptr::copy_nonoverlapping(value.to_le_bytes().as_ptr(), cursor, 8);
            cursor = cursor.add(8);
        }
    }

    host.clear_error();
    total_size as i32
}

/// Owned MemoryStream reference with the C++ scope-exit release semantics.
struct StreamGuard(*mut MemoryStreamObj);

impl StreamGuard {
    fn new(stream: *mut MemoryStreamObj) -> Self {
        Self(stream)
    }
    fn as_cvoid(&self) -> *mut c_void {
        self.0 as *mut c_void
    }
}

impl Drop for StreamGuard {
    fn drop(&mut self) {
        unsafe {
            if !self.0.is_null() {
                com_release(self.0);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Shared call wrapper
// ---------------------------------------------------------------------------

/// Null-handle check + panic containment for every plugin-facing export.
unsafe fn with_host<F: FnOnce(&mut Vst3HostInstance) -> i32>(
    host: Vst3HostInstanceHandle,
    f: F,
) -> i32 {
    let native_host = from_handle(host);
    if native_host.is_null() {
        return K_ERROR_INVALID_HOST;
    }
    match guarded(|| f(&mut *native_host)) {
        Ok(code) => code,
        Err(kind) => {
            let message = match kind {
                GuardFailure::Panic => "Native host panicked.",
                GuardFailure::Hardware => "Native host raised a native exception.",
            };
            let _ = guarded(|| (*native_host).set_error(message));
            K_ERROR_PLUGIN_LOAD
        }
    }
}

// ---------------------------------------------------------------------------
// Exports
// ---------------------------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn Vst3Host_GetApiVersion() -> i32 {
    1
}

#[no_mangle]
pub unsafe extern "C" fn Vst3Host_Create() -> Vst3HostInstanceHandle {
    match guarded(|| {
        let host = Box::new(Vst3HostInstance::new());
        Box::into_raw(host) as Vst3HostInstanceHandle
    }) {
        Ok(handle) => handle,
        Err(_) => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn Vst3Host_Destroy(host: Vst3HostInstanceHandle) {
    let native_host = from_handle(host);
    if native_host.is_null() {
        return;
    }
    let _ = guarded(|| {
        drop(Box::from_raw(native_host));
    });
}

#[no_mangle]
pub unsafe extern "C" fn Vst3Host_LoadPlugin(
    host: Vst3HostInstanceHandle,
    plugin_path: *const u16,
) -> i32 {
    with_host(host, |native_host| {
        if plugin_path.is_null() || *plugin_path == 0 {
            return native_host.fail(K_ERROR_INVALID_ARGUMENT, "Plugin path is required.");
        }

        let path = wide_from_ptr(plugin_path);
        if !ends_with_vst3(&path) {
            return native_host.fail(K_ERROR_INVALID_ARGUMENT, "Plugin path must end with .vst3.");
        }

        if !path_exists(&path) {
            return native_host.fail(K_ERROR_FILE_NOT_FOUND, "Plugin path does not exist.");
        }

        native_host.reset_plugin();

        let utf8_path = match wide_to_utf8(&path) {
            Ok(p) => p,
            Err(detail) => {
                let message = build_error("Invalid plugin path: ", detail);
                let message = String::from_utf16_lossy(&message);
                native_host.set_error(&message);
                return K_ERROR_INVALID_ARGUMENT;
            }
        };

        let module = match Module::create(&utf8_path) {
            Ok(module) => module,
            Err(detail) => {
                let detail = if detail.is_empty() {
                    "unknown SDK error".to_string()
                } else {
                    detail
                };
                let message = build_error("VST3 SDK failed to load the module: ", &detail);
                let message = String::from_utf16_lossy(&message);
                native_host.set_error(&message);
                return K_ERROR_PLUGIN_LOAD;
            }
        };

        if module.factory.is_null() {
            return native_host.fail(
                K_ERROR_NO_FACTORY,
                "VST3 module did not provide a plugin factory.",
            );
        }

        let mut selected_class: Option<ClassInfo> = None;
        for class_info in module.factory.class_infos() {
            if class_info.category == "Audio Module Class" {
                selected_class = Some(class_info);
                break;
            }
        }

        let Some(selected_class) = selected_class else {
            return native_host.fail(
                K_ERROR_NO_AUDIO_EFFECT,
                "VST3 module does not contain an audio effect class.",
            );
        };

        native_host.plugin_path = path;
        native_host.selected_effect_class_id = selected_class.id_to_string();
        native_host.selected_effect_class_name = selected_class.name.clone();
        native_host.selected_effect_class = Some(selected_class);
        native_host.has_selected_effect_class = true;
        native_host.module = Some(module);
        native_host.clear_error();
        K_SUCCESS
    })
}

#[no_mangle]
pub unsafe extern "C" fn Vst3Host_SetupProcessing(
    host: Vst3HostInstanceHandle,
    sample_rate: f64,
    max_block_size: i32,
    input_channels: i32,
    output_channels: i32,
) -> i32 {
    with_host(host, |native_host| {
        if !sample_rate.is_finite() || sample_rate <= 0.0 {
            return native_host.fail(
                K_ERROR_INVALID_ARGUMENT,
                "Sample rate must be greater than zero.",
            );
        }
        if max_block_size <= 0 {
            return native_host.fail(
                K_ERROR_INVALID_ARGUMENT,
                "Maximum block size must be greater than zero.",
            );
        }
        if !is_supported_channel_count(input_channels)
            || !is_supported_channel_count(output_channels)
        {
            return native_host.fail(
                K_ERROR_UNSUPPORTED,
                "Only mono and stereo channel counts are supported.",
            );
        }
        if input_channels != output_channels && !(input_channels == 1 && output_channels == 2) {
            return native_host.fail(
                K_ERROR_UNSUPPORTED,
                "Only matching channel counts or mono input to stereo output are supported.",
            );
        }
        if native_host.module.is_none() || !native_host.has_selected_effect_class {
            return native_host.fail(K_ERROR_NOT_CONFIGURED, "No VST3 plugin has been loaded.");
        }

        // CloseProcessingState (processing) + ResetProcessingConfiguration ()
        native_host.processing = None;
        native_host.reset_processing_configuration();
        PluginContextFactory::instance().set_plugin_context(native_host.plugin_context_ptr());

        let Some(factory) = native_host.module_factory() else {
            return native_host.fail(
                K_ERROR_NO_FACTORY,
                "VST3 module did not provide a plugin factory.",
            );
        };

        if !ensure_plugin_provider(native_host, &factory) {
            return native_host.fail(
                K_ERROR_PLUGIN_LOAD,
                "VST3 plugin component/controller initialization failed.",
            );
        }

        if native_host.plug_provider.is_none() {
            return native_host.fail(
                K_ERROR_PLUGIN_LOAD,
                "VST3 plugin component/controller initialization failed.",
            );
        }
        let component = native_host
            .plug_provider
            .as_ref()
            .unwrap()
            .get_component_ptr();
        let Some(component) = component else {
            return native_host.fail(
                K_ERROR_PLUGIN_LOAD,
                "VST3 plugin did not provide an audio component.",
            );
        };

        let Some(processor) = component.query::<IAudioProcessorObj>(&iid::I_AUDIO_PROCESSOR)
        else {
            return native_host.fail(
                K_ERROR_UNSUPPORTED,
                "VST3 component does not implement IAudioProcessor.",
            );
        };

        if unsafe { processor.can_process_sample_size(K_SAMPLE_32) } != K_RESULT_TRUE {
            return native_host.fail(
                K_ERROR_UNSUPPORTED,
                "VST3 audio processor does not support 32-bit float processing.",
            );
        }
        if !has_audio_bus(&component, K_INPUT) {
            return native_host.fail(
                K_ERROR_UNSUPPORTED,
                "VST3 audio processor does not provide an audio input bus.",
            );
        }
        if !has_audio_bus(&component, K_OUTPUT) {
            return native_host.fail(
                K_ERROR_UNSUPPORTED,
                "VST3 audio processor does not provide an audio output bus.",
            );
        }

        let mut input_arrangement = get_speaker_arrangement(input_channels);
        let mut output_arrangement = get_speaker_arrangement(output_channels);
        if unsafe {
            processor.set_bus_arrangements(&mut input_arrangement, 1, &mut output_arrangement, 1)
        } != K_RESULT_TRUE
        {
            return native_host.fail(
                K_ERROR_UNSUPPORTED,
                "VST3 audio processor does not support the requested mono/stereo bus arrangement.",
            );
        }

        let input_bus_count = unsafe { component.get_bus_count(K_AUDIO, K_INPUT) };
        for bus_index in 0..input_bus_count {
            let activate = bus_index == 0;
            let result =
                unsafe { component.activate_bus(K_AUDIO, K_INPUT, bus_index, activate as u8) };
            if activate && result != K_RESULT_TRUE {
                return native_host.fail(
                    K_ERROR_UNSUPPORTED,
                    "VST3 audio processor input bus activation failed.",
                );
            }
        }

        let output_bus_count = unsafe { component.get_bus_count(K_AUDIO, K_OUTPUT) };
        for bus_index in 0..output_bus_count {
            let activate = bus_index == 0;
            let result =
                unsafe { component.activate_bus(K_AUDIO, K_OUTPUT, bus_index, activate as u8) };
            if activate && result != K_RESULT_TRUE {
                return native_host.fail(
                    K_ERROR_UNSUPPORTED,
                    "VST3 audio processor output bus activation failed.",
                );
            }
        }

        let mut process_setup = ProcessSetup {
            process_mode: K_REALTIME,
            symbolic_sample_size: K_SAMPLE_32,
            max_samples_per_block: max_block_size,
            sample_rate,
        };
        if unsafe { processor.setup_processing(&mut process_setup) } != K_RESULT_TRUE {
            return native_host.fail(
                K_ERROR_PLUGIN_LOAD,
                "VST3 audio processor setupProcessing failed.",
            );
        }

        if unsafe { component.set_active(1) } != K_RESULT_TRUE {
            return native_host.fail(K_ERROR_PLUGIN_LOAD, "VST3 audio processor activation failed.");
        }

        let mut processing = ProcessingState::create();
        processing.component = Some(component);
        processing.processor = Some(processor);
        processing.active = true;

        // Some VST3 plugins return kResultFalse from setProcessing(true)
        // even after a successful setupProcessing/setActive path. Treat
        // process() itself as the authoritative check, otherwise those
        // plugins are permanently bypassed.
        unsafe {
            let _ = processing.processor.as_ref().unwrap().set_processing(1);
        }
        processing.processing = true;
        processing.sample_rate = sample_rate;
        processing.max_block_size = max_block_size;
        processing.input_channels = input_channels;
        processing.output_channels = output_channels;
        processing.process_context = ProcessContext::default();
        processing.process_context.sample_rate = sample_rate;
        processing.process_context.tempo = 120.0;

        for channel in 0..input_channels as usize {
            processing.input_buffers[channel].resize(max_block_size as usize, 0.0);
        }
        for channel in 0..output_channels as usize {
            processing.output_buffers[channel].resize(max_block_size as usize, 0.0);
        }
        processing.wire_channel_pointers();

        native_host.sample_rate = sample_rate;
        native_host.max_block_size = max_block_size;
        native_host.input_channels = input_channels;
        native_host.output_channels = output_channels;
        native_host.processing_configured = true;
        native_host.processing = Some(processing);
        native_host.clear_error();
        K_SUCCESS
    })
}

#[no_mangle]
pub unsafe extern "C" fn Vst3Host_ProcessFloat32(
    host: Vst3HostInstanceHandle,
    input_interleaved: *const f32,
    output_interleaved: *mut f32,
    frame_count: i32,
) -> i32 {
    with_host(host, |native_host| {
        if !native_host.processing_configured {
            return native_host.fail(K_ERROR_NOT_CONFIGURED, "Processing has not been configured.");
        }
        if frame_count < 0 {
            return native_host.fail(
                K_ERROR_INVALID_ARGUMENT,
                "Frame count must not be negative.",
            );
        }
        if frame_count > native_host.max_block_size {
            return native_host.fail(
                K_ERROR_INVALID_ARGUMENT,
                "Frame count exceeds configured maximum block size.",
            );
        }
        if frame_count == 0 {
            native_host.clear_error();
            return K_SUCCESS;
        }
        if input_interleaved.is_null() || output_interleaved.is_null() {
            return native_host.fail(
                K_ERROR_INVALID_ARGUMENT,
                "Input and output buffers are required.",
            );
        }

        let processing_ready = match native_host.processing.as_ref() {
            Some(processing) => {
                processing.processor.is_some()
                    && processing.processing
                    && processing.output_channels > 0
            }
            None => false,
        };
        if !processing_ready {
            if let Some(processing) = native_host.processing.as_ref() {
                if processing.processor.is_none() {
                    return native_host.fail(
                        K_ERROR_NOT_CONFIGURED,
                        "VST3 audio processor is not configured for processing.",
                    );
                }
                if !processing.processing {
                    return native_host.fail(
                        K_ERROR_NOT_CONFIGURED,
                        "VST3 audio processor is not configured for processing.",
                    );
                }
                return native_host.fail(
                    K_ERROR_UNSUPPORTED,
                    "VST3 audio processor has no configured output channels.",
                );
            }
            return native_host.fail(
                K_ERROR_NOT_CONFIGURED,
                "VST3 audio processor is not configured for processing.",
            );
        }

        // Raw pointers: the host fields and the processing state are
        // disjoint allocations, and the plugin callback must not be
        // blocked by Rust borrows.
        let handler: *const ComponentHandlerObj = &*native_host.component_handler;
        let processing: *mut ProcessingState =
            native_host.processing.as_mut().unwrap().as_mut() as *mut ProcessingState;

        prepare_processing_data(
            unsafe { &*handler },
            unsafe { &mut *processing },
            input_interleaved,
            frame_count,
        );

        let processor = unsafe { (*processing).processor.as_ref().unwrap().as_ptr() };
        let result = unsafe { (*processor).process(&mut (*processing).process_data) };
        if result != K_RESULT_TRUE {
            return native_host.fail(K_ERROR_PLUGIN_LOAD, "VST3 audio processor process failed.");
        }

        write_processing_output(unsafe { &*processing }, output_interleaved, frame_count);
        native_host.clear_error();
        K_SUCCESS
    })
}

#[no_mangle]
pub unsafe extern "C" fn Vst3Host_SaveState(
    host: Vst3HostInstanceHandle,
    buffer: *mut u8,
    buffer_size: i32,
) -> i32 {
    with_host(host, |native_host| {
        if buffer_size < 0 {
            return native_host.fail(
                K_ERROR_INVALID_ARGUMENT,
                "State buffer size must not be negative.",
            );
        }
        if native_host.module.is_none() || !native_host.has_selected_effect_class {
            return native_host.fail(K_ERROR_NOT_CONFIGURED, "No VST3 plugin has been loaded.");
        }

        PluginContextFactory::instance().set_plugin_context(native_host.plugin_context_ptr());

        let Some(factory) = native_host.module_factory() else {
            return native_host.fail(
                K_ERROR_NO_FACTORY,
                "VST3 module did not provide a plugin factory.",
            );
        };

        if !ensure_plugin_provider(native_host, &factory) {
            return native_host.fail(
                K_ERROR_PLUGIN_LOAD,
                "VST3 plugin component/controller initialization failed.",
            );
        }

        let (component, controller) = match native_host.plug_provider.as_ref() {
            Some(provider) => (provider.get_component_ptr(), provider.get_controller_ptr()),
            None => (None, None),
        };

        let component_stream = StreamGuard::new(MemoryStreamObj::create_owned());
        let mut has_component_state = false;
        if let Some(component) = &component {
            has_component_state =
                unsafe { component.get_state(component_stream.as_cvoid()) } == K_RESULT_TRUE;
        }

        if !has_component_state {
            // Component state is unavailable (or the component itself is
            // missing); fall back to a controller parameter dump so the
            // plugin settings are still captured for the profile.
            return save_controller_parameter_state(
                native_host,
                controller.as_ref(),
                buffer,
                buffer_size,
            );
        }

        let controller_stream = StreamGuard::new(MemoryStreamObj::create_owned());
        let mut has_controller_state = false;
        if let Some(controller) = &controller {
            if unsafe { controller.get_state(controller_stream.as_cvoid()) } == K_RESULT_TRUE {
                has_controller_state = true;
            }
        }

        let component_size_raw = unsafe { (*component_stream.0).get_size() };
        let controller_size_raw = if has_controller_state {
            unsafe { (*controller_stream.0).get_size() }
        } else {
            0
        };
        if component_size_raw > K_MAX_STATE_BLOB_SIZE || controller_size_raw > K_MAX_STATE_BLOB_SIZE
        {
            return native_host.fail(
                K_ERROR_UNSUPPORTED,
                "VST3 plugin state is too large to persist.",
            );
        }

        let component_size = component_size_raw as u32;
        let controller_size = controller_size_raw as u32;
        let fixed_size = std::mem::size_of::<u32>() * 4;
        let total_size = fixed_size + component_size as usize + controller_size as usize;

        if buffer.is_null() {
            native_host.clear_error();
            return total_size as i32;
        }

        if buffer_size < 0 || (buffer_size as usize) < total_size {
            return native_host.fail(K_ERROR_INVALID_ARGUMENT, "State buffer is too small.");
        }

        let mut cursor = buffer;
        unsafe {
            std::ptr::copy_nonoverlapping(K_STATE_BLOB_MAGIC.to_le_bytes().as_ptr(), cursor, 4);
            cursor = cursor.add(4);
            std::ptr::copy_nonoverlapping(K_STATE_BLOB_VERSION.to_le_bytes().as_ptr(), cursor, 4);
            cursor = cursor.add(4);
            std::ptr::copy_nonoverlapping(component_size.to_le_bytes().as_ptr(), cursor, 4);
            cursor = cursor.add(4);

            if component_size > 0 {
                let data = (*component_stream.0).get_data();
                std::ptr::copy_nonoverlapping(data, cursor, component_size as usize);
                cursor = cursor.add(component_size as usize);
            }

            std::ptr::copy_nonoverlapping(controller_size.to_le_bytes().as_ptr(), cursor, 4);
            cursor = cursor.add(4);
            if controller_size > 0 {
                let data = (*controller_stream.0).get_data();
                std::ptr::copy_nonoverlapping(data, cursor, controller_size as usize);
                cursor = cursor.add(controller_size as usize);
            }
        }
        let _ = cursor;

        native_host.clear_error();
        total_size as i32
    })
}

#[no_mangle]
pub unsafe extern "C" fn Vst3Host_LoadState(
    host: Vst3HostInstanceHandle,
    buffer: *const u8,
    buffer_size: i32,
) -> i32 {
    with_host(host, |native_host| {
        if buffer.is_null() || buffer_size <= 0 {
            return native_host.fail(K_ERROR_INVALID_ARGUMENT, "State buffer is required.");
        }
        if native_host.module.is_none() || !native_host.has_selected_effect_class {
            return native_host.fail(K_ERROR_NOT_CONFIGURED, "No VST3 plugin has been loaded.");
        }

        let total_size = buffer_size as usize;
        if total_size < 12 {
            return native_host.fail(K_ERROR_INVALID_ARGUMENT, "State buffer is too small.");
        }

        let mut cursor = buffer;
        let mut magic: u32 = 0;
        let mut version: u32 = 0;
        let mut component_size: u32 = 0;
        let mut controller_size: u32 = 0;
        unsafe {
            std::ptr::copy_nonoverlapping(cursor, &mut magic as *mut u32 as *mut u8, 4);
            cursor = cursor.add(4);
            std::ptr::copy_nonoverlapping(cursor, &mut version as *mut u32 as *mut u8, 4);
            cursor = cursor.add(4);
            std::ptr::copy_nonoverlapping(cursor, &mut component_size as *mut u32 as *mut u8, 4);
            cursor = cursor.add(4);
        }

        if version != K_STATE_BLOB_VERSION {
            return native_host.fail(
                K_ERROR_INVALID_ARGUMENT,
                "State buffer version is not supported.",
            );
        }

        if magic == K_PARAM_STATE_BLOB_MAGIC || magic == K_LEGACY_PARAM_STATE_BLOB_MAGIC {
            let parameter_count = component_size;
            let pair_size = std::mem::size_of::<u32>() + std::mem::size_of::<f64>();
            if total_size < 12 + parameter_count as usize * pair_size {
                return native_host.fail(K_ERROR_INVALID_ARGUMENT, "State buffer is truncated.");
            }

            PluginContextFactory::instance().set_plugin_context(native_host.plugin_context_ptr());

            let Some(factory) = native_host.module_factory() else {
                return native_host.fail(
                    K_ERROR_NO_FACTORY,
                    "VST3 module did not provide a plugin factory.",
                );
            };

            if !ensure_plugin_provider(native_host, &factory) {
                return native_host.fail(
                    K_ERROR_PLUGIN_LOAD,
                    "VST3 plugin component/controller initialization failed.",
                );
            }

            let controller = native_host
                .plug_provider
                .as_ref()
                .and_then(|provider| provider.get_controller_ptr());
            let Some(controller) = controller else {
                return native_host.fail(
                    K_ERROR_PLUGIN_LOAD,
                    "VST3 edit controller is unavailable for a parameter restore.",
                );
            };

            for _index in 0..parameter_count {
                let mut parameter_id: u32 = 0;
                let mut value: f64 = 0.0;
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        cursor,
                        &mut parameter_id as *mut u32 as *mut u8,
                        4,
                    );
                    cursor = cursor.add(4);
                    std::ptr::copy_nonoverlapping(cursor, &mut value as *mut f64 as *mut u8, 8);
                    cursor = cursor.add(8);
                }

                if parameter_id != 0xFFFFFFFF {
                    unsafe {
                        controller.set_param_normalized(parameter_id, value);
                    }
                    native_host
                        .component_handler
                        .perform_edit_direct(parameter_id, value);
                }
            }

            native_host.clear_error();
            return K_SUCCESS;
        }

        if magic != K_STATE_BLOB_MAGIC && magic != K_LEGACY_STATE_BLOB_MAGIC {
            return native_host.fail(
                K_ERROR_INVALID_ARGUMENT,
                "State buffer is not a recognized VST-Applier state blob.",
            );
        }

        let fixed_size = std::mem::size_of::<u32>() * 4;
        if total_size < fixed_size {
            return native_host.fail(K_ERROR_INVALID_ARGUMENT, "State buffer is too small.");
        }

        if component_size as usize > total_size.wrapping_sub(12).wrapping_sub(4) {
            return native_host.fail(K_ERROR_INVALID_ARGUMENT, "State buffer is truncated.");
        }

        let component_data = cursor;
        cursor = unsafe { cursor.add(component_size as usize) };

        unsafe {
            std::ptr::copy_nonoverlapping(cursor, &mut controller_size as *mut u32 as *mut u8, 4);
            cursor = cursor.add(4);
        }

        if controller_size as usize
            > total_size
                .wrapping_sub(12)
                .wrapping_sub(component_size as usize)
                .wrapping_sub(4)
        {
            return native_host.fail(K_ERROR_INVALID_ARGUMENT, "State buffer is truncated.");
        }

        let controller_data = cursor;

        PluginContextFactory::instance().set_plugin_context(native_host.plugin_context_ptr());

        let Some(factory) = native_host.module_factory() else {
            return native_host.fail(
                K_ERROR_NO_FACTORY,
                "VST3 module did not provide a plugin factory.",
            );
        };

        if !ensure_plugin_provider(native_host, &factory) {
            return native_host.fail(
                K_ERROR_PLUGIN_LOAD,
                "VST3 plugin component/controller initialization failed.",
            );
        }

        let (component, controller) = match native_host.plug_provider.as_ref() {
            Some(provider) => (provider.get_component_ptr(), provider.get_controller_ptr()),
            None => (None, None),
        };

        let Some(component) = component else {
            return native_host.fail(
                K_ERROR_PLUGIN_LOAD,
                "VST3 plugin did not provide an audio component.",
            );
        };

        if component_size > 0 {
            let component_stream = StreamGuard::new(MemoryStreamObj::create_external(
                component_data as *mut u8,
                component_size as usize,
            ));
            if unsafe { component.set_state(component_stream.as_cvoid()) } != K_RESULT_TRUE {
                return native_host.fail(
                    K_ERROR_PLUGIN_LOAD,
                    "VST3 component state restore failed.",
                );
            }

            if let Some(controller) = &controller {
                let component_state_for_controller =
                    StreamGuard::new(MemoryStreamObj::create_external(
                        component_data as *mut u8,
                        component_size as usize,
                    ));
                let _ = unsafe {
                    controller.set_component_state(component_state_for_controller.as_cvoid())
                };
            }
        }

        if let Some(controller) = &controller {
            if controller_size > 0 {
                let controller_stream = StreamGuard::new(MemoryStreamObj::create_external(
                    controller_data as *mut u8,
                    controller_size as usize,
                ));
                if unsafe { controller.set_state(controller_stream.as_cvoid()) } != K_RESULT_TRUE {
                    return native_host.fail(
                        K_ERROR_PLUGIN_LOAD,
                        "VST3 controller state restore failed.",
                    );
                }
            }
        }

        native_host.clear_error();
        K_SUCCESS
    })
}

#[no_mangle]
pub unsafe extern "C" fn Vst3Host_OpenEditor(
    host: Vst3HostInstanceHandle,
    parent_hwnd: *mut c_void,
) -> i32 {
    with_host(host, |native_host| {
        if parent_hwnd.is_null() {
            return native_host.fail(
                K_ERROR_INVALID_ARGUMENT,
                "Parent window handle is required.",
            );
        }
        if native_host.module.is_none() || !native_host.has_selected_effect_class {
            return native_host.fail(K_ERROR_NOT_CONFIGURED, "No VST3 plugin has been loaded.");
        }

        close_editor_state(&mut native_host.editor);
        PluginContextFactory::instance().set_plugin_context(native_host.plugin_context_ptr());

        let Some(factory) = native_host.module_factory() else {
            return native_host.fail(
                K_ERROR_NO_FACTORY,
                "VST3 module did not provide a plugin factory.",
            );
        };

        if !ensure_plugin_provider(native_host, &factory) {
            return native_host.fail(
                K_ERROR_PLUGIN_LOAD,
                "VST3 plugin component/controller initialization failed.",
            );
        }

        let controller = native_host
            .plug_provider
            .as_ref()
            .and_then(|provider| provider.get_controller_ptr());
        let Some(controller) = controller else {
            return native_host.fail(
                K_ERROR_NO_EDITOR,
                "VST3 plugin does not provide an edit controller.",
            );
        };

        let view_ptr = unsafe { controller.create_view(K_VIEW_TYPE_EDITOR.as_ptr() as FIDString) };
        if view_ptr.is_null() {
            return native_host.fail(
                K_ERROR_NO_EDITOR,
                "VST3 edit controller does not provide an editor view.",
            );
        }
        let plug_view: InterfacePtr<IPlugViewObj> =
            unsafe { InterfacePtr::from_owned(view_ptr as *mut IPlugViewObj) };

        if unsafe {
            plug_view.is_platform_type_supported(K_PLATFORM_TYPE_HWND.as_ptr() as FIDString)
        } != K_RESULT_TRUE
        {
            return native_host.fail(
                K_ERROR_UNSUPPORTED,
                "VST3 editor view does not support HWND hosting.",
            );
        }

        let mut plug_frame = PlugFrameObj::create(parent_hwnd);
        plug_frame.set_plug_view(plug_view.as_ptr());

        if unsafe { plug_view.set_frame(PlugFrameObj::as_ptr(&mut plug_frame)) } != K_RESULT_TRUE {
            return native_host.fail(K_ERROR_EDITOR_ATTACH, "VST3 editor setFrame failed.");
        }

        if unsafe { plug_view.attached(parent_hwnd, K_PLATFORM_TYPE_HWND.as_ptr() as FIDString) }
            != K_RESULT_TRUE
        {
            return native_host.fail(K_ERROR_EDITOR_ATTACH, "VST3 editor attached(HWND) failed.");
        }

        let mut editor = Box::new(EditorState {
            plug_view: None,
            plug_frame: None,
            controller: None,
            attached: true,
        });

        let mut view_size: ViewRect = unsafe { std::mem::zeroed() };
        if unsafe { plug_view.get_size(&mut view_size) } == K_RESULT_TRUE {
            plug_frame.resize_parent_to_view_size(&view_size);
            unsafe {
                plug_view.on_size(&mut view_size);
            }
        }

        editor.controller = Some(controller);
        editor.plug_view = Some(plug_view);
        editor.plug_frame = Some(plug_frame);
        native_host.editor = Some(editor);

        native_host.clear_error();
        K_SUCCESS
    })
}

#[no_mangle]
pub unsafe extern "C" fn Vst3Host_CloseEditor(host: Vst3HostInstanceHandle) {
    let native_host = from_handle(host);
    if native_host.is_null() {
        return;
    }
    let _ = guarded(|| {
        close_editor_state(&mut (*native_host).editor);
        (*native_host).clear_error();
    });
}

#[no_mangle]
pub unsafe extern "C" fn Vst3Host_GetLastError(host: Vst3HostInstanceHandle) -> *const u16 {
    let native_host = from_handle(host);
    if native_host.is_null() {
        return NULL_HOST_ERROR.as_ptr();
    }
    match guarded(|| (*native_host).last_error.as_ptr()) {
        Ok(ptr) => ptr,
        Err(_) => NULL_HOST_ERROR.as_ptr(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deliberately invalid pointer read must come back as
    /// `Err(GuardFailure::Hardware)` instead of taking the test process down:
    /// proves the SEH guard is armed.
    #[test]
    fn seh_guard_catches_hardware_fault() {
        let result = guarded(|| unsafe {
            core::ptr::read_volatile(core::mem::align_of::<i32>() as *const i32)
        });
        assert_eq!(result, Err(GuardFailure::Hardware));
        assert_eq!(guarded(|| 21 * 2), Ok(42));
    }

    /// Rust panics must be caught by the same guard.
    #[test]
    fn guard_catches_rust_panic() {
        let result = guarded(|| -> i32 { panic!("intentional test panic") });
        assert_eq!(result, Err(GuardFailure::Panic));
    }
}
