//! Hand-rolled VST3 ABI surface, ported 1:1 from the Steinberg VST3 SDK
//! (`third_party/vst3sdk`, v3.7.12) that the C++ host was built against.
//!
//! Every interface declared here exists ONLY to call into (or be called by)
//! plugin binaries, so the `#[repr(C)]` vtable layouts and member order are
//! ABI-critical and must match `pluginterfaces/**` exactly. All vtable
//! functions are `STDMETHODCALLTYPE` (`PLUGIN_API`) => `extern "system"`.
#![allow(non_snake_case, non_camel_case_types, dead_code)]
#![allow(clippy::missing_safety_doc)]

use std::ffi::c_void;

pub type TUID = [u8; 16];
pub type TChar = u16;
/// `FIDString` — `const char8*` in the SDK.
pub type FIDString = *const i8;
/// `TBool` — `uint8` in the SDK.
pub type TBool = u8;

// ---------------------------------------------------------------------------
// tresult values (COM_COMPATIBLE == 1 on Windows, see funknown.h)
// ---------------------------------------------------------------------------

pub const K_RESULT_OK: i32 = 0;
pub const K_RESULT_TRUE: i32 = 0;
pub const K_RESULT_FALSE: i32 = 1;
pub const K_NO_INTERFACE: i32 = 0x80004002u32 as i32;
pub const K_INVALID_ARGUMENT: i32 = 0x80070057u32 as i32;
pub const K_NOT_IMPLEMENTED: i32 = 0x80004001u32 as i32;
pub const K_INTERNAL_ERROR: i32 = 0x80004005u32 as i32;
pub const K_OUT_OF_MEMORY: i32 = 0x8007000Eu32 as i32;

// ---------------------------------------------------------------------------
// UIDs — INLINE_UID with COM_COMPATIBLE == 1 byte layout
// ---------------------------------------------------------------------------

/// Mirrors the SDK `INLINE_UID` macro for `COM_COMPATIBLE == 1`.
pub const fn uid(l1: u32, l2: u32, l3: u32, l4: u32) -> TUID {
    [
        (l1 & 0x000000FF) as u8,
        ((l1 & 0x0000FF00) >> 8) as u8,
        ((l1 & 0x00FF0000) >> 16) as u8,
        ((l1 & 0xFF000000) >> 24) as u8,
        ((l2 & 0x00FF0000) >> 16) as u8,
        ((l2 & 0xFF000000) >> 24) as u8,
        (l2 & 0x000000FF) as u8,
        ((l2 & 0x0000FF00) >> 8) as u8,
        ((l3 & 0xFF000000) >> 24) as u8,
        ((l3 & 0x00FF0000) >> 16) as u8,
        ((l3 & 0x0000FF00) >> 8) as u8,
        (l3 & 0x000000FF) as u8,
        ((l4 & 0xFF000000) >> 24) as u8,
        ((l4 & 0x00FF0000) >> 16) as u8,
        ((l4 & 0x0000FF00) >> 8) as u8,
        (l4 & 0x000000FF) as u8,
    ]
}

/// `FUnknownPrivate::iidEqual` — compares the TUID as two native u64s.
#[inline]
pub fn iid_equal(a: &TUID, b: &TUID) -> bool {
    let pa = a.as_ptr() as *const u64;
    let pb = b.as_ptr() as *const u64;
    unsafe { *pa == *pb && *pa.add(1) == *pb.add(1) }
}

#[inline]
pub unsafe fn iid_equal_ptr(a: *const u8, b: &TUID) -> bool {
    if a.is_null() {
        return false;
    }
    let pa = a as *const u64;
    let pb = b.as_ptr() as *const u64;
    *pa == *pb && *pa.add(1) == *pb.add(1)
}

pub mod iid {
    use super::{uid, TUID};

    pub const F_UNKNOWN: TUID = uid(0x00000000, 0x00000000, 0xC0000000, 0x00000046);
    pub const I_PLUGIN_BASE: TUID = uid(0x22888DDB, 0x156E45AE, 0x8358B348, 0x08190625);
    pub const I_COMPONENT: TUID = uid(0xE831FF31, 0xF2D54301, 0x928EBBEE, 0x25697802);
    pub const I_AUDIO_PROCESSOR: TUID = uid(0x42043F99, 0xB7DA453C, 0xA569E79D, 0x9AAEC33D);
    pub const I_EDIT_CONTROLLER: TUID = uid(0xDCD7BBE3, 0x7742448D, 0xA874AACC, 0x979C759E);
    pub const I_COMPONENT_HANDLER: TUID = uid(0x93A0BEA3, 0x0BD045DB, 0x8E890B0C, 0xC1E46AC6);
    pub const I_PLUG_VIEW: TUID = uid(0x5BC32507, 0xD06049EA, 0xA6151B52, 0x2B755B29);
    pub const I_PLUG_FRAME: TUID = uid(0x367FAF01, 0xAFA94693, 0x8D4DA2A0, 0xED0882A3);
    pub const I_HOST_APPLICATION: TUID = uid(0x58E595CC, 0xDB2D4969, 0x8B6AAF8C, 0x36A664E5);
    pub const I_MESSAGE: TUID = uid(0x936F033B, 0xC6C047DB, 0xBB0882F8, 0x13C1E613);
    pub const I_ATTRIBUTE_LIST: TUID = uid(0x1E5F0AEB, 0xCC7F4533, 0xA2544011, 0x38AD5EE4);
    pub const I_BSTREAM: TUID = uid(0xC3BF6EA2, 0x30994752, 0x9B6BF990, 0x1EE33E9B);
    pub const I_PARAMETER_CHANGES: TUID = uid(0xA4779663, 0x0BB64A56, 0xB44384A8, 0x466FEB9D);
    pub const I_PARAM_VALUE_QUEUE: TUID = uid(0x01263A18, 0xED074F6F, 0x98C9D356, 0x4686F9BA);
    pub const I_CONNECTION_POINT: TUID = uid(0x70A4156F, 0x6E6E4026, 0x989148BF, 0xAA60D8D1);
    pub const I_PLUGIN_FACTORY: TUID = uid(0x7A4D811C, 0x52114A1F, 0xAED9D2EE, 0x0B43BF9F);
    pub const I_PLUGIN_FACTORY_2: TUID = uid(0x0007B650, 0xF24B4C0B, 0xA464EDB9, 0xF00B2ABB);
    pub const I_PLUGIN_FACTORY_3: TUID = uid(0x4555A2AB, 0xC1234E57, 0x9B122910, 0x36878931);
    pub const I_PLUG_INTERFACE_SUPPORT: TUID = uid(0x4FB58B9E, 0x9EAA4E0F, 0xAB361C1C, 0xCCB56FEA);
    // PlugInterfaceSupport "minimum set" interfaces (hostclasses.cpp).
    pub const I_UNIT_INFO: TUID = uid(0x3D4BD6B5, 0x913A4FD2, 0xA886E768, 0xA5EB92C1);
    pub const I_UNIT_DATA: TUID = uid(0x6C389611, 0xD391455D, 0xB870B833, 0x94A0EFDD);
    pub const I_PROGRAM_LIST_DATA: TUID = uid(0x8683B01F, 0x7B354F70, 0xA2651DEC, 0x353AF4FF);
    pub const I_MIDI_MAPPING: TUID = uid(0xDF0FF9F7, 0x49B74669, 0xB63AB732, 0x7ADBF5E5);
    pub const I_EDIT_CONTROLLER_2: TUID = uid(0x7F4EFE59, 0xF3204967, 0xAC27A3AE, 0xAFB63038);
}

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

pub const K_VST_AUDIO_EFFECT_CLASS: &[u8] = b"Audio Module Class\0";
pub const K_PLATFORM_TYPE_HWND: &[u8] = b"HWND\0";
pub const K_VIEW_TYPE_EDITOR: &[u8] = b"editor\0";
pub const K_NO_PARAM_ID: u32 = 0xFFFFFFFF;

// enums
pub const K_AUDIO: i32 = 0;
pub const K_INPUT: i32 = 0;
pub const K_OUTPUT: i32 = 1;
pub const K_SAMPLE_32: i32 = 0;
pub const K_REALTIME: i32 = 0;
pub const K_MONO: u64 = 1 << 19; // SpeakerArr::kMono == kSpeakerM
pub const K_STEREO: u64 = 3; // SpeakerArr::kStereo == kSpeakerL | kSpeakerR
pub const K_ALL_CHANNELS_SILENT: u64 = 0xFFFFFFFFFFFFFFFF;

#[repr(C)]
pub struct PFactoryInfo {
    pub vendor: [i8; 64],
    pub url: [i8; 256],
    pub email: [i8; 128],
    pub flags: i32,
}

#[repr(C)]
pub struct PClassInfo {
    pub cid: TUID,
    pub cardinality: i32,
    pub category: [i8; 32],
    pub name: [i8; 64],
}

#[repr(C)]
pub struct PClassInfo2 {
    pub cid: TUID,
    pub cardinality: i32,
    pub category: [i8; 32],
    pub name: [i8; 64],
    pub class_flags: u32,
    pub sub_categories: [i8; 128],
    pub vendor: [i8; 64],
    pub version: [i8; 64],
    pub sdk_version: [i8; 64],
}

#[repr(C)]
pub struct PClassInfoW {
    pub cid: TUID,
    pub cardinality: i32,
    pub category: [i8; 32],
    pub name: [u16; 64],
    pub class_flags: u32,
    pub sub_categories: [i8; 128],
    pub vendor: [u16; 64],
    pub version: [u16; 64],
    pub sdk_version: [u16; 64],
}

#[repr(C)]
pub struct BusInfo {
    pub media_type: i32,
    pub direction: i32,
    pub channel_count: i32,
    pub name: [u16; 128],
    pub bus_type: i32,
    pub flags: u32,
}

#[repr(C)]
pub struct RoutingInfo {
    pub media_type: i32,
    pub bus_index: i32,
    pub channel: i32,
}

#[repr(C)]
pub struct ProcessSetup {
    pub process_mode: i32,
    pub symbolic_sample_size: i32,
    pub max_samples_per_block: i32,
    pub sample_rate: f64,
}

#[repr(C)]
pub struct AudioBusBuffers {
    pub num_channels: i32,
    pub silence_flags: u64,
    pub channel_buffers32: *mut *mut f32,
}

#[repr(C)]
pub struct ProcessData {
    pub process_mode: i32,
    pub symbolic_sample_size: i32,
    pub num_samples: i32,
    pub num_inputs: i32,
    pub num_outputs: i32,
    pub inputs: *mut AudioBusBuffers,
    pub outputs: *mut AudioBusBuffers,
    pub input_parameter_changes: *mut c_void,
    pub output_parameter_changes: *mut c_void,
    pub input_events: *mut c_void,
    pub output_events: *mut c_void,
    pub process_context: *mut ProcessContext,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ViewRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FrameRate {
    pub frames_per_second: u32,
    pub flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Chord {
    pub key_note: u8,
    pub root_note: u8,
    pub chord_mask: i16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ProcessContext {
    pub state: u32,
    pub sample_rate: f64,
    pub project_time_samples: i64,
    pub system_time: i64,
    pub continuous_time_samples: i64,
    pub project_time_music: f64,
    pub bar_position_music: f64,
    pub cycle_start_music: f64,
    pub cycle_end_music: f64,
    pub tempo: f64,
    pub time_sig_numerator: i32,
    pub time_sig_denominator: i32,
    pub chord: Chord,
    pub smpte_offset_subframes: i32,
    pub frame_rate: FrameRate,
    pub samples_to_next_clock: i32,
}

impl Default for ProcessContext {
    fn default() -> Self {
        // Zero-initialised, exactly like the C++ `{}` aggregate.
        unsafe { std::mem::zeroed() }
    }
}

#[repr(C)]
pub struct ParameterInfo {
    pub id: u32,
    pub title: [u16; 128],
    pub short_title: [u16; 128],
    pub units: [u16; 128],
    pub step_count: i32,
    pub default_normalized_value: f64,
    pub unit_id: i32,
    pub flags: i32,
}

impl Default for ParameterInfo {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

impl Default for PFactoryInfo {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

// ---------------------------------------------------------------------------
// Generic FUnknown helpers (the first three slots of every interface)
// ---------------------------------------------------------------------------

#[inline]
unsafe fn vtbl_ptr<T>(p: *mut T) -> *const usize {
    debug_assert!(!p.is_null());
    *(p as *mut *const usize)
}

pub unsafe fn com_query_interface<T>(p: *mut T, iid: &TUID) -> Option<*mut c_void> {
    let vtbl = vtbl_ptr(p);
    let f: unsafe extern "system" fn(*mut c_void, *const u8, *mut *mut c_void) -> i32 =
        std::mem::transmute(*vtbl);
    let mut out: *mut c_void = std::ptr::null_mut();
    if f(p as *mut c_void, iid.as_ptr(), &mut out) == K_RESULT_OK {
        Some(out)
    } else {
        None
    }
}

pub unsafe fn com_add_ref<T>(p: *mut T) -> u32 {
    let vtbl = vtbl_ptr(p);
    let f: unsafe extern "system" fn(*mut c_void) -> u32 = std::mem::transmute(*vtbl.add(1));
    f(p as *mut c_void)
}

pub unsafe fn com_release<T>(p: *mut T) -> u32 {
    let vtbl = vtbl_ptr(p);
    let f: unsafe extern "system" fn(*mut c_void) -> u32 = std::mem::transmute(*vtbl.add(2));
    f(p as *mut c_void)
}

/// Owning smart pointer, the Rust equivalent of the SDK's `IPtr<T>`:
/// holds one reference and releases it on drop.
pub struct InterfacePtr<T> {
    ptr: *mut T,
}

impl<T> InterfacePtr<T> {
    /// Takes ownership of a reference returned by a `createInstance`/query
    /// result (count already incremented by the producer).
    pub unsafe fn from_owned(ptr: *mut T) -> Self {
        Self { ptr }
    }

    #[inline]
    pub fn as_ptr(&self) -> *mut T {
        self.ptr
    }

    #[inline]
    pub fn is_null(&self) -> bool {
        self.ptr.is_null()
    }

    /// `IPtr` copy semantics: takes an extra reference.
    pub unsafe fn cloned(&self) -> Self {
        if !self.ptr.is_null() {
            com_add_ref(self.ptr);
        }
        Self { ptr: self.ptr }
    }

    /// `FUnknownPtr<T>` style conversion: queryInterface (returns an owned ref).
    pub unsafe fn query<U>(&self, iid: &TUID) -> Option<InterfacePtr<U>> {
        if self.ptr.is_null() {
            return None;
        }
        com_query_interface(self.ptr, iid).map(|p| InterfacePtr::from_owned(p as *mut U))
    }
}

impl<T> Drop for InterfacePtr<T> {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe {
                com_release(self.ptr);
            }
            self.ptr = std::ptr::null_mut();
        }
    }
}

/// Lets callers invoke the interface methods directly on the owning pointer
/// (`component.get_bus_count (...)`) while the pointer manages the refcount.
impl<T> std::ops::Deref for InterfacePtr<T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.ptr }
    }
}

// ---------------------------------------------------------------------------
// Plugin-side interfaces (implemented by the plugin binaries)
// ---------------------------------------------------------------------------


#[repr(C)]
pub struct IPluginBaseVtbl {
    pub query_interface:
        unsafe extern "system" fn(*mut c_void, *const u8, *mut *mut c_void) -> i32,
    pub add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub initialize: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub terminate: unsafe extern "system" fn(*mut c_void) -> i32,
}

#[repr(C)]
pub struct IComponentObj {
    pub vtbl: *const IComponentVtbl,
}

#[repr(C)]
pub struct IComponentVtbl {
    pub query_interface:
        unsafe extern "system" fn(*mut c_void, *const u8, *mut *mut c_void) -> i32,
    pub add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub initialize: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub terminate: unsafe extern "system" fn(*mut c_void) -> i32,
    pub get_controller_class_id: unsafe extern "system" fn(*mut c_void, *mut u8) -> i32,
    pub set_io_mode: unsafe extern "system" fn(*mut c_void, i32) -> i32,
    pub get_bus_count: unsafe extern "system" fn(*mut c_void, i32, i32) -> i32,
    pub get_bus_info: unsafe extern "system" fn(*mut c_void, i32, i32, i32, *mut BusInfo) -> i32,
    pub get_routing_info:
        unsafe extern "system" fn(*mut c_void, *mut RoutingInfo, *mut RoutingInfo) -> i32,
    pub activate_bus: unsafe extern "system" fn(*mut c_void, i32, i32, i32, TBool) -> i32,
    pub set_active: unsafe extern "system" fn(*mut c_void, TBool) -> i32,
    pub set_state: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub get_state: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
}

impl IComponentObj {
    #[inline]
    unsafe fn v(&self) -> &'static IComponentVtbl {
        &*self.vtbl
    }

    pub unsafe fn initialize(&self, context: *mut c_void) -> i32 {
        (self.v().initialize)(self as *const Self as *mut c_void, context)
    }
    pub unsafe fn terminate(&self) -> i32 {
        (self.v().terminate)(self as *const Self as *mut c_void)
    }
    pub unsafe fn get_controller_class_id(&self, cid: *mut u8) -> i32 {
        (self.v().get_controller_class_id)(self as *const Self as *mut c_void, cid)
    }
    pub unsafe fn get_bus_count(&self, media_type: i32, dir: i32) -> i32 {
        (self.v().get_bus_count)(self as *const Self as *mut c_void, media_type, dir)
    }
    pub unsafe fn activate_bus(&self, media_type: i32, dir: i32, index: i32, state: TBool) -> i32 {
        (self.v().activate_bus)(
            self as *const Self as *mut c_void,
            media_type,
            dir,
            index,
            state,
        )
    }
    pub unsafe fn set_active(&self, state: TBool) -> i32 {
        (self.v().set_active)(self as *const Self as *mut c_void, state)
    }
    pub unsafe fn set_state(&self, stream: *mut c_void) -> i32 {
        (self.v().set_state)(self as *const Self as *mut c_void, stream)
    }
    pub unsafe fn get_state(&self, stream: *mut c_void) -> i32 {
        (self.v().get_state)(self as *const Self as *mut c_void, stream)
    }
}

#[repr(C)]
pub struct IAudioProcessorObj {
    pub vtbl: *const IAudioProcessorVtbl,
}

#[repr(C)]
pub struct IAudioProcessorVtbl {
    pub query_interface:
        unsafe extern "system" fn(*mut c_void, *const u8, *mut *mut c_void) -> i32,
    pub add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub set_bus_arrangements:
        unsafe extern "system" fn(*mut c_void, *mut u64, i32, *mut u64, i32) -> i32,
    pub get_bus_arrangement: unsafe extern "system" fn(*mut c_void, i32, i32, *mut u64) -> i32,
    pub can_process_sample_size: unsafe extern "system" fn(*mut c_void, i32) -> i32,
    pub get_latency_samples: unsafe extern "system" fn(*mut c_void) -> u32,
    pub setup_processing: unsafe extern "system" fn(*mut c_void, *mut ProcessSetup) -> i32,
    pub set_processing: unsafe extern "system" fn(*mut c_void, TBool) -> i32,
    pub process: unsafe extern "system" fn(*mut c_void, *mut ProcessData) -> i32,
    pub get_tail_samples: unsafe extern "system" fn(*mut c_void) -> u32,
}

impl IAudioProcessorObj {
    #[inline]
    unsafe fn v(&self) -> &'static IAudioProcessorVtbl {
        &*self.vtbl
    }

    pub unsafe fn set_bus_arrangements(
        &self,
        inputs: *mut u64,
        num_ins: i32,
        outputs: *mut u64,
        num_outs: i32,
    ) -> i32 {
        (self.v().set_bus_arrangements)(
            self as *const Self as *mut c_void,
            inputs,
            num_ins,
            outputs,
            num_outs,
        )
    }
    pub unsafe fn get_bus_arrangement(&self, dir: i32, index: i32, arrangement: *mut u64) -> i32 {
        (self.v().get_bus_arrangement)(self as *const Self as *mut c_void, dir, index, arrangement)
    }
    pub unsafe fn can_process_sample_size(&self, size: i32) -> i32 {
        (self.v().can_process_sample_size)(self as *const Self as *mut c_void, size)
    }
    pub unsafe fn setup_processing(&self, setup: *mut ProcessSetup) -> i32 {
        (self.v().setup_processing)(self as *const Self as *mut c_void, setup)
    }
    pub unsafe fn set_processing(&self, state: TBool) -> i32 {
        (self.v().set_processing)(self as *const Self as *mut c_void, state)
    }
    pub unsafe fn process(&self, data: *mut ProcessData) -> i32 {
        (self.v().process)(self as *const Self as *mut c_void, data)
    }
}

#[repr(C)]
pub struct IEditControllerObj {
    pub vtbl: *const IEditControllerVtbl,
}

#[repr(C)]
pub struct IEditControllerVtbl {
    pub query_interface:
        unsafe extern "system" fn(*mut c_void, *const u8, *mut *mut c_void) -> i32,
    pub add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub initialize: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub terminate: unsafe extern "system" fn(*mut c_void) -> i32,
    pub set_component_state: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub set_state: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub get_state: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub get_parameter_count: unsafe extern "system" fn(*mut c_void) -> i32,
    pub get_parameter_info:
        unsafe extern "system" fn(*mut c_void, i32, *mut ParameterInfo) -> i32,
    pub get_param_string_by_value:
        unsafe extern "system" fn(*mut c_void, u32, f64, *mut u16) -> i32,
    pub get_param_value_by_string:
        unsafe extern "system" fn(*mut c_void, u32, *const u16, *mut f64) -> i32,
    pub normalized_param_to_plain: unsafe extern "system" fn(*mut c_void, u32, f64) -> f64,
    pub plain_param_to_normalized: unsafe extern "system" fn(*mut c_void, u32, f64) -> f64,
    pub get_param_normalized: unsafe extern "system" fn(*mut c_void, u32) -> f64,
    pub set_param_normalized: unsafe extern "system" fn(*mut c_void, u32, f64) -> i32,
    pub set_component_handler: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub create_view: unsafe extern "system" fn(*mut c_void, FIDString) -> *mut c_void,
}

impl IEditControllerObj {
    #[inline]
    unsafe fn v(&self) -> &'static IEditControllerVtbl {
        &*self.vtbl
    }

    pub unsafe fn initialize(&self, context: *mut c_void) -> i32 {
        (self.v().initialize)(self as *const Self as *mut c_void, context)
    }
    pub unsafe fn terminate(&self) -> i32 {
        (self.v().terminate)(self as *const Self as *mut c_void)
    }
    pub unsafe fn set_component_state(&self, stream: *mut c_void) -> i32 {
        (self.v().set_component_state)(self as *const Self as *mut c_void, stream)
    }
    pub unsafe fn set_state(&self, stream: *mut c_void) -> i32 {
        (self.v().set_state)(self as *const Self as *mut c_void, stream)
    }
    pub unsafe fn get_state(&self, stream: *mut c_void) -> i32 {
        (self.v().get_state)(self as *const Self as *mut c_void, stream)
    }
    pub unsafe fn get_parameter_count(&self) -> i32 {
        (self.v().get_parameter_count)(self as *const Self as *mut c_void)
    }
    pub unsafe fn get_parameter_info(&self, index: i32, info: *mut ParameterInfo) -> i32 {
        (self.v().get_parameter_info)(self as *const Self as *mut c_void, index, info)
    }
    pub unsafe fn get_param_normalized(&self, id: u32) -> f64 {
        (self.v().get_param_normalized)(self as *const Self as *mut c_void, id)
    }
    pub unsafe fn set_param_normalized(&self, id: u32, value: f64) -> i32 {
        (self.v().set_param_normalized)(self as *const Self as *mut c_void, id, value)
    }
    pub unsafe fn set_component_handler(&self, handler: *mut c_void) -> i32 {
        (self.v().set_component_handler)(self as *const Self as *mut c_void, handler)
    }
    pub unsafe fn create_view(&self, name: FIDString) -> *mut c_void {
        (self.v().create_view)(self as *const Self as *mut c_void, name)
    }
}

#[repr(C)]
pub struct IPlugViewObj {
    pub vtbl: *const IPlugViewVtbl,
}

#[repr(C)]
pub struct IPlugViewVtbl {
    pub query_interface:
        unsafe extern "system" fn(*mut c_void, *const u8, *mut *mut c_void) -> i32,
    pub add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub is_platform_type_supported: unsafe extern "system" fn(*mut c_void, FIDString) -> i32,
    pub attached: unsafe extern "system" fn(*mut c_void, *mut c_void, FIDString) -> i32,
    pub removed: unsafe extern "system" fn(*mut c_void) -> i32,
    pub on_wheel: unsafe extern "system" fn(*mut c_void, f32) -> i32,
    pub on_key_down: unsafe extern "system" fn(*mut c_void, u16, i16, i16) -> i32,
    pub on_key_up: unsafe extern "system" fn(*mut c_void, u16, i16, i16) -> i32,
    pub get_size: unsafe extern "system" fn(*mut c_void, *mut ViewRect) -> i32,
    pub on_size: unsafe extern "system" fn(*mut c_void, *mut ViewRect) -> i32,
    pub on_focus: unsafe extern "system" fn(*mut c_void, TBool) -> i32,
    pub set_frame: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub can_resize: unsafe extern "system" fn(*mut c_void) -> i32,
    pub check_size_constraint: unsafe extern "system" fn(*mut c_void, *mut ViewRect) -> i32,
}

impl IPlugViewObj {
    #[inline]
    unsafe fn v(&self) -> &'static IPlugViewVtbl {
        &*self.vtbl
    }

    pub unsafe fn is_platform_type_supported(&self, platform: FIDString) -> i32 {
        (self.v().is_platform_type_supported)(self as *const Self as *mut c_void, platform)
    }
    pub unsafe fn attached(&self, parent: *mut c_void, platform: FIDString) -> i32 {
        (self.v().attached)(self as *const Self as *mut c_void, parent, platform)
    }
    pub unsafe fn removed(&self) -> i32 {
        (self.v().removed)(self as *const Self as *mut c_void)
    }
    pub unsafe fn get_size(&self, size: *mut ViewRect) -> i32 {
        (self.v().get_size)(self as *const Self as *mut c_void, size)
    }
    pub unsafe fn on_size(&self, size: *mut ViewRect) -> i32 {
        (self.v().on_size)(self as *const Self as *mut c_void, size)
    }
    pub unsafe fn set_frame(&self, frame: *mut c_void) -> i32 {
        (self.v().set_frame)(self as *const Self as *mut c_void, frame)
    }
}

#[repr(C)]
pub struct IConnectionPointObj {
    pub vtbl: *const IConnectionPointVtbl,
}

#[repr(C)]
pub struct IConnectionPointVtbl {
    pub query_interface:
        unsafe extern "system" fn(*mut c_void, *const u8, *mut *mut c_void) -> i32,
    pub add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub connect: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub disconnect: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub notify: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
}

impl IConnectionPointObj {
    #[inline]
    unsafe fn v(&self) -> &'static IConnectionPointVtbl {
        &*self.vtbl
    }
    pub unsafe fn connect(&self, other: *mut c_void) -> i32 {
        (self.v().connect)(self as *const Self as *mut c_void, other)
    }
    pub unsafe fn disconnect(&self, other: *mut c_void) -> i32 {
        (self.v().disconnect)(self as *const Self as *mut c_void, other)
    }
}

#[repr(C)]
pub struct IPluginFactoryObj {
    pub vtbl: *const IPluginFactoryVtbl,
}

#[repr(C)]
pub struct IPluginFactoryVtbl {
    pub query_interface:
        unsafe extern "system" fn(*mut c_void, *const u8, *mut *mut c_void) -> i32,
    pub add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub get_factory_info: unsafe extern "system" fn(*mut c_void, *mut PFactoryInfo) -> i32,
    pub count_classes: unsafe extern "system" fn(*mut c_void) -> i32,
    pub get_class_info: unsafe extern "system" fn(*mut c_void, i32, *mut PClassInfo) -> i32,
    pub create_instance:
        unsafe extern "system" fn(*mut c_void, FIDString, FIDString, *mut *mut c_void) -> i32,
}

#[repr(C)]
pub struct IPluginFactory2Vtbl {
    pub query_interface:
        unsafe extern "system" fn(*mut c_void, *const u8, *mut *mut c_void) -> i32,
    pub add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub get_factory_info: unsafe extern "system" fn(*mut c_void, *mut PFactoryInfo) -> i32,
    pub count_classes: unsafe extern "system" fn(*mut c_void) -> i32,
    pub get_class_info: unsafe extern "system" fn(*mut c_void, i32, *mut PClassInfo) -> i32,
    pub create_instance:
        unsafe extern "system" fn(*mut c_void, FIDString, FIDString, *mut *mut c_void) -> i32,
    pub get_class_info2: unsafe extern "system" fn(*mut c_void, i32, *mut PClassInfo2) -> i32,
}

#[repr(C)]
pub struct IPluginFactory3Vtbl {
    pub query_interface:
        unsafe extern "system" fn(*mut c_void, *const u8, *mut *mut c_void) -> i32,
    pub add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub get_factory_info: unsafe extern "system" fn(*mut c_void, *mut PFactoryInfo) -> i32,
    pub count_classes: unsafe extern "system" fn(*mut c_void) -> i32,
    pub get_class_info: unsafe extern "system" fn(*mut c_void, i32, *mut PClassInfo) -> i32,
    pub create_instance:
        unsafe extern "system" fn(*mut c_void, FIDString, FIDString, *mut *mut c_void) -> i32,
    pub get_class_info2: unsafe extern "system" fn(*mut c_void, i32, *mut PClassInfo2) -> i32,
    pub get_class_info_unicode:
        unsafe extern "system" fn(*mut c_void, i32, *mut PClassInfoW) -> i32,
    pub set_host_context: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
}

impl IPluginFactoryObj {
    #[inline]
    unsafe fn v(&self) -> &'static IPluginFactoryVtbl {
        &*self.vtbl
    }

    pub unsafe fn get_factory_info(&self, info: *mut PFactoryInfo) -> i32 {
        (self.v().get_factory_info)(self as *const Self as *mut c_void, info)
    }
    pub unsafe fn count_classes(&self) -> i32 {
        (self.v().count_classes)(self as *const Self as *mut c_void)
    }
    pub unsafe fn get_class_info(&self, index: i32, info: *mut PClassInfo) -> i32 {
        (self.v().get_class_info)(self as *const Self as *mut c_void, index, info)
    }
    pub unsafe fn create_instance(
        &self,
        cid: *const u8,
        iid: *const u8,
        obj: *mut *mut c_void,
    ) -> i32 {
        (self.v().create_instance)(self as *const Self as *mut c_void, cid as FIDString, iid as FIDString, obj)
    }
}

impl IPluginFactory2Vtbl {
    #[inline]
    pub unsafe fn get_class_info2(
        &self,
        this: *mut c_void,
        index: i32,
        info: *mut PClassInfo2,
    ) -> i32 {
        (self.get_class_info2)(this, index, info)
    }
}

impl IPluginFactory3Vtbl {
    #[inline]
    pub unsafe fn get_class_info_unicode(
        &self,
        this: *mut c_void,
        index: i32,
        info: *mut PClassInfoW,
    ) -> i32 {
        (self.get_class_info_unicode)(this, index, info)
    }
}

#[repr(C)]
pub struct IParameterChangesObj {
    pub vtbl: *const IParameterChangesVtbl,
}

#[repr(C)]
pub struct IParameterChangesVtbl {
    pub query_interface:
        unsafe extern "system" fn(*mut c_void, *const u8, *mut *mut c_void) -> i32,
    pub add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub get_parameter_count: unsafe extern "system" fn(*mut c_void) -> i32,
    pub get_parameter_data: unsafe extern "system" fn(*mut c_void, i32) -> *mut c_void,
    pub add_parameter_data: unsafe extern "system" fn(*mut c_void, *const u32, *mut i32) -> *mut c_void,
}

#[repr(C)]
pub struct IParamValueQueueVtbl {
    pub query_interface:
        unsafe extern "system" fn(*mut c_void, *const u8, *mut *mut c_void) -> i32,
    pub add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub get_parameter_id: unsafe extern "system" fn(*mut c_void) -> u32,
    pub get_point_count: unsafe extern "system" fn(*mut c_void) -> i32,
    pub get_point: unsafe extern "system" fn(*mut c_void, i32, *mut i32, *mut f64) -> i32,
    pub add_point: unsafe extern "system" fn(*mut c_void, i32, f64, *mut i32) -> i32,
}

// ---------------------------------------------------------------------------
// Host-side interfaces (implemented by this DLL, called by plugins)
// ---------------------------------------------------------------------------

/// `IComponentHandler` vtable, implemented by `HostComponentHandler`.
#[repr(C)]
pub struct ComponentHandlerVtbl {
    pub query_interface:
        unsafe extern "system" fn(*mut ComponentHandlerObj, *const u8, *mut *mut c_void) -> i32,
    pub add_ref: unsafe extern "system" fn(*mut ComponentHandlerObj) -> u32,
    pub release: unsafe extern "system" fn(*mut ComponentHandlerObj) -> u32,
    pub begin_edit: unsafe extern "system" fn(*mut ComponentHandlerObj, u32) -> i32,
    pub perform_edit: unsafe extern "system" fn(*mut ComponentHandlerObj, u32, f64) -> i32,
    pub end_edit: unsafe extern "system" fn(*mut ComponentHandlerObj, u32) -> i32,
    pub restart_component: unsafe extern "system" fn(*mut ComponentHandlerObj, i32) -> i32,
}

/// `IPlugFrame` vtable, implemented by `NativePlugFrame`.
#[repr(C)]
pub struct PlugFrameVtbl {
    pub query_interface:
        unsafe extern "system" fn(*mut PlugFrameObj, *const u8, *mut *mut c_void) -> i32,
    pub add_ref: unsafe extern "system" fn(*mut PlugFrameObj) -> u32,
    pub release: unsafe extern "system" fn(*mut PlugFrameObj) -> u32,
    pub resize_view:
        unsafe extern "system" fn(*mut PlugFrameObj, *mut c_void, *mut ViewRect) -> i32,
}

/// `IHostApplication` vtable, implemented by `HostApplicationObj`.
#[repr(C)]
pub struct HostApplicationVtbl {
    pub query_interface:
        unsafe extern "system" fn(*mut HostApplicationObj, *const u8, *mut *mut c_void) -> i32,
    pub add_ref: unsafe extern "system" fn(*mut HostApplicationObj) -> u32,
    pub release: unsafe extern "system" fn(*mut HostApplicationObj) -> u32,
    pub get_name: unsafe extern "system" fn(*mut HostApplicationObj, *mut u16) -> i32,
    pub create_instance:
        unsafe extern "system" fn(*mut HostApplicationObj, *const u8, *const u8, *mut *mut c_void) -> i32,
}

/// `IPlugInterfaceSupport` vtable, implemented by `PlugInterfaceSupportObj`.
#[repr(C)]
pub struct PlugInterfaceSupportVtbl {
    pub query_interface:
        unsafe extern "system" fn(*mut PlugInterfaceSupportObj, *const u8, *mut *mut c_void) -> i32,
    pub add_ref: unsafe extern "system" fn(*mut PlugInterfaceSupportObj) -> u32,
    pub release: unsafe extern "system" fn(*mut PlugInterfaceSupportObj) -> u32,
    pub is_plug_interface_supported:
        unsafe extern "system" fn(*mut PlugInterfaceSupportObj, *const u8) -> i32,
}

/// `IMessage` vtable, implemented by `HostMessageObj`.
#[repr(C)]
pub struct HostMessageVtbl {
    pub query_interface:
        unsafe extern "system" fn(*mut HostMessageObj, *const u8, *mut *mut c_void) -> i32,
    pub add_ref: unsafe extern "system" fn(*mut HostMessageObj) -> u32,
    pub release: unsafe extern "system" fn(*mut HostMessageObj) -> u32,
    pub get_message_id: unsafe extern "system" fn(*mut HostMessageObj) -> FIDString,
    pub set_message_id: unsafe extern "system" fn(*mut HostMessageObj, FIDString),
    pub get_attributes: unsafe extern "system" fn(*mut HostMessageObj) -> *mut c_void,
}

/// `IAttributeList` vtable, implemented by `HostAttributeListObj`.
#[repr(C)]
pub struct HostAttributeListVtbl {
    pub query_interface:
        unsafe extern "system" fn(*mut HostAttributeListObj, *const u8, *mut *mut c_void) -> i32,
    pub add_ref: unsafe extern "system" fn(*mut HostAttributeListObj) -> u32,
    pub release: unsafe extern "system" fn(*mut HostAttributeListObj) -> u32,
    pub set_int: unsafe extern "system" fn(*mut HostAttributeListObj, FIDString, i64) -> i32,
    pub get_int:
        unsafe extern "system" fn(*mut HostAttributeListObj, FIDString, *mut i64) -> i32,
    pub set_float:
        unsafe extern "system" fn(*mut HostAttributeListObj, FIDString, f64) -> i32,
    pub get_float:
        unsafe extern "system" fn(*mut HostAttributeListObj, FIDString, *mut f64) -> i32,
    pub set_string:
        unsafe extern "system" fn(*mut HostAttributeListObj, FIDString, *const u16) -> i32,
    pub get_string:
        unsafe extern "system" fn(*mut HostAttributeListObj, FIDString, *mut u16, u32) -> i32,
    pub set_binary:
        unsafe extern "system" fn(*mut HostAttributeListObj, FIDString, *const c_void, u32) -> i32,
    pub get_binary:
        unsafe extern "system" fn(*mut HostAttributeListObj, FIDString, *mut *const c_void, *mut u32) -> i32,
}

/// `IBStream` vtable, implemented by `MemoryStreamObj`.
#[repr(C)]
pub struct MemoryStreamVtbl {
    pub query_interface:
        unsafe extern "system" fn(*mut MemoryStreamObj, *const u8, *mut *mut c_void) -> i32,
    pub add_ref: unsafe extern "system" fn(*mut MemoryStreamObj) -> u32,
    pub release: unsafe extern "system" fn(*mut MemoryStreamObj) -> u32,
    pub read:
        unsafe extern "system" fn(*mut MemoryStreamObj, *mut c_void, i32, *mut i32) -> i32,
    pub write:
        unsafe extern "system" fn(*mut MemoryStreamObj, *mut c_void, i32, *mut i32) -> i32,
    pub seek: unsafe extern "system" fn(*mut MemoryStreamObj, i64, i32, *mut i64) -> i32,
    pub tell: unsafe extern "system" fn(*mut MemoryStreamObj, *mut i64) -> i32,
}

/// `IParameterChanges` / `IParamValueQueue` vtable, implemented by
/// `ParameterChangesObj` / `ParameterValueQueueObj`.
#[repr(C)]
pub struct ParameterChangesVtbl {
    pub query_interface:
        unsafe extern "system" fn(*mut ParameterChangesObj, *const u8, *mut *mut c_void) -> i32,
    pub add_ref: unsafe extern "system" fn(*mut ParameterChangesObj) -> u32,
    pub release: unsafe extern "system" fn(*mut ParameterChangesObj) -> u32,
    pub get_parameter_count: unsafe extern "system" fn(*mut ParameterChangesObj) -> i32,
    pub get_parameter_data:
        unsafe extern "system" fn(*mut ParameterChangesObj, i32) -> *mut ParameterValueQueueObj,
    pub add_parameter_data:
        unsafe extern "system" fn(*mut ParameterChangesObj, *const u32, *mut i32) -> *mut ParameterValueQueueObj,
}

#[repr(C)]
pub struct ParameterValueQueueVtbl {
    pub query_interface:
        unsafe extern "system" fn(*mut ParameterValueQueueObj, *const u8, *mut *mut c_void) -> i32,
    pub add_ref: unsafe extern "system" fn(*mut ParameterValueQueueObj) -> u32,
    pub release: unsafe extern "system" fn(*mut ParameterValueQueueObj) -> u32,
    pub get_parameter_id: unsafe extern "system" fn(*mut ParameterValueQueueObj) -> u32,
    pub get_point_count: unsafe extern "system" fn(*mut ParameterValueQueueObj) -> i32,
    pub get_point:
        unsafe extern "system" fn(*mut ParameterValueQueueObj, i32, *mut i32, *mut f64) -> i32,
    pub add_point:
        unsafe extern "system" fn(*mut ParameterValueQueueObj, i32, f64, *mut i32) -> i32,
}

/// `IConnectionPoint` vtable, implemented by `ConnectionProxyObj`.
#[repr(C)]
pub struct ConnectionProxyVtbl {
    pub query_interface:
        unsafe extern "system" fn(*mut ConnectionProxyObj, *const u8, *mut *mut c_void) -> i32,
    pub add_ref: unsafe extern "system" fn(*mut ConnectionProxyObj) -> u32,
    pub release: unsafe extern "system" fn(*mut ConnectionProxyObj) -> u32,
    pub connect: unsafe extern "system" fn(*mut ConnectionProxyObj, *mut c_void) -> i32,
    pub disconnect: unsafe extern "system" fn(*mut ConnectionProxyObj, *mut c_void) -> i32,
    pub notify: unsafe extern "system" fn(*mut ConnectionProxyObj, *mut c_void) -> i32,
}

// Forward declarations of host object structs (defined in `objects`).
use crate::objects::{
    ComponentHandlerObj, ConnectionProxyObj, HostApplicationObj, HostAttributeListObj,
    HostMessageObj, MemoryStreamObj, ParameterChangesObj, ParameterValueQueueObj, PlugFrameObj,
    PlugInterfaceSupportObj,
};

// ---------------------------------------------------------------------------
// Strings
// ---------------------------------------------------------------------------

/// Reads a NUL-terminated fixed-size `char8` array (SDK `StringConvert::convert
/// (char8*, size)`), like `strncpy` semantics: conversion stops at the first NUL
/// within `size` bytes.
pub fn fixed_cstr_to_string(field: &[i8]) -> String {
    let bytes: Vec<u8> = field
        .iter()
        .take_while(|c| **c != 0)
        .map(|c| *c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Reads a NUL-terminated fixed-size `char16` array (SDK
/// `StringConvert::convert (char16*, size)`).
pub fn fixed_wstr_to_string(field: &[u16]) -> String {
    let units: Vec<u16> = field.iter().take_while(|c| **c != 0).copied().collect();
    String::from_utf16_lossy(&units)
}
