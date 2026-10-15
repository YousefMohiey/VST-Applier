//! Host-side COM objects, ported 1:1 from the VST3 SDK hosting classes the
//! C++ host compiled in:
//! `hostclasses.{h,cpp}` (IHostApplication / IMessage / IAttributeList /
//! IPlugInterfaceSupport), `memorystream.{h,cpp}` (IBStream),
//! `parameterchanges.{h,cpp}` (IParameterChanges / IParamValueQueue /
//! ParameterChangeTransfer), `connectionproxy.{h,cpp}` (IConnectionPoint)
//! plus the two ad-hoc objects from `Vst3HostNative.cpp` itself
//! (IComponentHandler and IPlugFrame).
#![allow(non_snake_case, dead_code)]
#![allow(clippy::missing_safety_doc)]

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Mutex;

use windows::Win32::Foundation::{BOOL, HWND};
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, GetMenu, GetWindowLongPtrW, IsWindow, SetWindowPos, GWL_EXSTYLE, GWL_STYLE,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, WS_CHILD,
};

use crate::vst3abi::*;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Mirrors `FUnknownPrivate::atomicAdd` (InterlockedAdd): returns the NEW value.
fn atomic_add(count: &AtomicI32, amount: i32) -> i32 {
    count.fetch_add(amount, Ordering::SeqCst) + amount
}

fn qi_result(obj: *mut *mut c_void, value: *mut c_void) -> i32 {
    unsafe { *obj = value };
    K_RESULT_OK
}

/// The repeated `if (obj == nullptr) return kInvalidArgument; *obj = nullptr;`
/// preamble from the SDK objects.
unsafe fn qi_preamble(obj: *mut *mut c_void) -> bool {
    if obj.is_null() {
        return false;
    }
    *obj = std::ptr::null_mut();
    true
}

// ===========================================================================
// HostComponentHandler (Vst3HostNative.cpp)
// ===========================================================================

#[repr(C)]
pub struct ComponentHandlerObj {
    pub vtbl: *const ComponentHandlerVtbl,
    pub transfer: Mutex<ParameterChangeTransfer>,
}

static COMPONENT_HANDLER_VTBL: ComponentHandlerVtbl = ComponentHandlerVtbl {
    query_interface: ch_query_interface,
    add_ref: ch_add_ref,
    release: ch_release,
    begin_edit: ch_begin_edit,
    perform_edit: ch_perform_edit,
    end_edit: ch_end_edit,
    restart_component: ch_restart_component,
};

impl ComponentHandlerObj {
    pub fn create() -> Box<Self> {
        Box::new(Self {
            vtbl: &COMPONENT_HANDLER_VTBL,
            transfer: Mutex::new(ParameterChangeTransfer::new(1000)),
        })
    }

    pub fn as_ptr(b: &mut Box<Self>) -> *mut c_void {
        b.as_mut() as *mut Self as *mut c_void
    }

    /// `TransferChangesTo`: drains pending edits into `destination`.
    pub fn transfer_changes_to(&self, destination: &mut ParameterChangesObj) {
        let mut guard = self.transfer.lock().unwrap();
        guard.transfer_changes_to(destination);
    }

    /// `ClearPendingChanges`.
    pub fn clear_pending_changes(&self) {
        let mut guard = self.transfer.lock().unwrap();
        guard.remove_changes();
    }

    /// `performEdit`, used directly by `LoadState`'s parameter restore.
    pub fn perform_edit_direct(&self, id: u32, value: f64) {
        let mut guard = self.transfer.lock().unwrap();
        guard.add_change(id, value, 0);
    }
}

unsafe extern "system" fn ch_query_interface(
    this: *mut ComponentHandlerObj,
    iid: *const u8,
    obj: *mut *mut c_void,
) -> i32 {
    if !qi_preamble(obj) {
        return K_INVALID_ARGUMENT;
    }
    if iid_equal_ptr(iid, &iid::I_COMPONENT_HANDLER) || iid_equal_ptr(iid, &iid::F_UNKNOWN) {
        // addRef(): returns a constant, nothing to do.
        return qi_result(obj, this as *mut c_void);
    }
    K_NO_INTERFACE
}

// The C++ handler returns a constant 1000 from both addRef and release.
unsafe extern "system" fn ch_add_ref(_this: *mut ComponentHandlerObj) -> u32 {
    1000
}
unsafe extern "system" fn ch_release(_this: *mut ComponentHandlerObj) -> u32 {
    1000
}

unsafe extern "system" fn ch_begin_edit(_this: *mut ComponentHandlerObj, _id: u32) -> i32 {
    K_RESULT_TRUE
}

unsafe extern "system" fn ch_perform_edit(
    this: *mut ComponentHandlerObj,
    id: u32,
    value: f64,
) -> i32 {
    crate::catch_panic_i32(K_RESULT_TRUE, || {
        let obj = &*this;
        obj.perform_edit_direct(id, value);
        K_RESULT_TRUE
    })
}

unsafe extern "system" fn ch_end_edit(_this: *mut ComponentHandlerObj, _id: u32) -> i32 {
    K_RESULT_TRUE
}

unsafe extern "system" fn ch_restart_component(
    _this: *mut ComponentHandlerObj,
    _flags: i32,
) -> i32 {
    K_RESULT_TRUE
}

// ===========================================================================
// NativePlugFrame (Vst3HostNative.cpp)
// ===========================================================================

#[repr(C)]
pub struct PlugFrameObj {
    pub vtbl: *const PlugFrameVtbl,
    pub parent_hwnd: HWND,
    pub plug_view: *mut IPlugViewObj,
    pub resize_view_in_progress: bool,
}

static PLUG_FRAME_VTBL: PlugFrameVtbl = PlugFrameVtbl {
    query_interface: pf_query_interface,
    add_ref: pf_add_ref,
    release: pf_release,
    resize_view: pf_resize_view,
};

fn same_view_rect(left: &ViewRect, right: &ViewRect) -> bool {
    left.left == right.left
        && left.top == right.top
        && left.right == right.right
        && left.bottom == right.bottom
}

impl PlugFrameObj {
    pub fn create(parent_hwnd: *mut c_void) -> Box<Self> {
        Box::new(Self {
            vtbl: &PLUG_FRAME_VTBL,
            parent_hwnd: HWND(parent_hwnd),
            plug_view: std::ptr::null_mut(),
            resize_view_in_progress: false,
        })
    }

    pub fn as_ptr(b: &mut Box<Self>) -> *mut c_void {
        b.as_mut() as *mut Self as *mut c_void
    }

    pub fn set_plug_view(&mut self, plug_view: *mut IPlugViewObj) {
        self.plug_view = plug_view;
    }

    pub fn resize_parent_to_view_size(&self, view_size: &ViewRect) {
        unsafe {
            if !IsWindow(self.parent_hwnd).as_bool() {
                return;
            }

            let width = std::cmp::max(1, view_size.right - view_size.left);
            let height = std::cmp::max(1, view_size.bottom - view_size.top);
            let style = GetWindowLongPtrW(self.parent_hwnd, GWL_STYLE) as u32;
            let style_flags = windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(style);
            let null_hwnd = HWND(std::ptr::null_mut());

            if (style & WS_CHILD.0) == WS_CHILD.0 {
                let _ = SetWindowPos(
                    self.parent_hwnd,
                    null_hwnd,
                    0,
                    0,
                    width,
                    height,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
                return;
            }

            let mut window_rect = windows::Win32::Foundation::RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            };
            let extended_style =
                GetWindowLongPtrW(self.parent_hwnd, GWL_EXSTYLE) as u32;
            let has_menu = GetMenu(self.parent_hwnd).0 != std::ptr::null_mut();

            if AdjustWindowRectEx(
                &mut window_rect,
                style_flags,
                BOOL::from(has_menu),
                windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE(extended_style),
            )
            .is_err()
            {
                window_rect = windows::Win32::Foundation::RECT {
                    left: 0,
                    top: 0,
                    right: width,
                    bottom: height,
                };
            }

            let _ = SetWindowPos(
                self.parent_hwnd,
                null_hwnd,
                0,
                0,
                window_rect.right - window_rect.left,
                window_rect.bottom - window_rect.top,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}

unsafe extern "system" fn pf_query_interface(
    this: *mut PlugFrameObj,
    iid: *const u8,
    obj: *mut *mut c_void,
) -> i32 {
    if !qi_preamble(obj) {
        return K_INVALID_ARGUMENT;
    }
    if iid_equal_ptr(iid, &iid::I_PLUG_FRAME) || iid_equal_ptr(iid, &iid::F_UNKNOWN) {
        return qi_result(obj, this as *mut c_void);
    }
    K_NO_INTERFACE
}

unsafe extern "system" fn pf_add_ref(_this: *mut PlugFrameObj) -> u32 {
    1000
}
unsafe extern "system" fn pf_release(_this: *mut PlugFrameObj) -> u32 {
    1000
}

unsafe extern "system" fn pf_resize_view(
    this: *mut PlugFrameObj,
    view: *mut c_void,
    new_size: *mut ViewRect,
) -> i32 {
    crate::catch_panic_i32(K_INVALID_ARGUMENT, || {
        let frame = &mut *this;
        if view.is_null() || new_size.is_null() || view != frame.plug_view as *mut c_void {
            return K_INVALID_ARGUMENT;
        }
        if frame.resize_view_in_progress {
            return K_RESULT_FALSE;
        }

        let plug_view = &*frame.plug_view;
        let mut current_size: ViewRect = std::mem::zeroed();
        if plug_view.get_size(&mut current_size) == K_RESULT_TRUE
            && same_view_rect(&current_size, &*new_size)
        {
            return K_RESULT_TRUE;
        }

        frame.resize_view_in_progress = true;
        frame.resize_parent_to_view_size(&*new_size);
        let result = plug_view.on_size(new_size);
        frame.resize_view_in_progress = false;
        if result == K_RESULT_TRUE {
            K_RESULT_TRUE
        } else {
            result
        }
    })
}

// ===========================================================================
// PlugInterfaceSupport (pluginterfacesupport.{h,cpp})
// ===========================================================================

#[repr(C)]
pub struct PlugInterfaceSupportObj {
    pub vtbl: *const PlugInterfaceSupportVtbl,
    pub ref_count: AtomicI32,
    pub supported: Vec<TUID>,
}

static PLUG_INTERFACE_SUPPORT_VTBL: PlugInterfaceSupportVtbl = PlugInterfaceSupportVtbl {
    query_interface: pis_query_interface,
    add_ref: pis_add_ref,
    release: pis_release,
    is_plug_interface_supported: pis_is_plug_interface_supported,
};

impl PlugInterfaceSupportObj {
    /// Returns the raw pointer with a reference count of 1 (FUNKNOWN_CTOR).
    pub fn create() -> *mut Self {
        let obj = Box::new(Self {
            vtbl: &PLUG_INTERFACE_SUPPORT_VTBL,
            ref_count: AtomicI32::new(1),
            supported: vec![
                iid::I_COMPONENT,
                iid::I_AUDIO_PROCESSOR,
                iid::I_EDIT_CONTROLLER,
                iid::I_CONNECTION_POINT,
                iid::I_UNIT_INFO,
                iid::I_UNIT_DATA,
                iid::I_PROGRAM_LIST_DATA,
                iid::I_MIDI_MAPPING,
                iid::I_EDIT_CONTROLLER_2,
            ],
        });
        Box::into_raw(obj)
    }

    pub fn is_supported(&self, iid: *const u8) -> i32 {
        if iid.is_null() {
            return K_RESULT_FALSE;
        }
        let needle: TUID = unsafe { *(iid as *const TUID) };
        for entry in &self.supported {
            if iid_equal(entry, &needle) {
                return K_RESULT_TRUE;
            }
        }
        K_RESULT_FALSE
    }
}

unsafe extern "system" fn pis_query_interface(
    this: *mut PlugInterfaceSupportObj,
    iid: *const u8,
    obj: *mut *mut c_void,
) -> i32 {
    if !qi_preamble(obj) {
        return K_INVALID_ARGUMENT;
    }
    // IMPLEMENT_QUERYINTERFACE: FUnknown first, then the interface itself.
    if iid_equal_ptr(iid, &iid::F_UNKNOWN) || iid_equal_ptr(iid, &iid::I_PLUG_INTERFACE_SUPPORT) {
        atomic_add(&(*this).ref_count, 1);
        return qi_result(obj, this as *mut c_void);
    }
    K_NO_INTERFACE
}

unsafe extern "system" fn pis_add_ref(this: *mut PlugInterfaceSupportObj) -> u32 {
    atomic_add(&(*this).ref_count, 1) as u32
}

unsafe extern "system" fn pis_release(this: *mut PlugInterfaceSupportObj) -> u32 {
    let count = atomic_add(&(*this).ref_count, -1);
    if count == 0 {
        drop(Box::from_raw(this));
        return 0;
    }
    count as u32
}

unsafe extern "system" fn pis_is_plug_interface_supported(
    this: *mut PlugInterfaceSupportObj,
    iid: *const u8,
) -> i32 {
    (*this).is_supported(iid)
}

// ===========================================================================
// HostAttributeList (hostclasses.{h,cpp})
// ===========================================================================

enum Attribute {
    Integer(i64),
    Float(f64),
    String(Vec<u16>),
    Binary(Vec<u8>),
}

#[repr(C)]
pub struct HostAttributeListObj {
    pub vtbl: *const HostAttributeListVtbl,
    pub ref_count: AtomicI32,
    list: HashMap<String, Attribute>,
}

static HOST_ATTRIBUTE_LIST_VTBL: HostAttributeListVtbl = HostAttributeListVtbl {
    query_interface: hal_query_interface,
    add_ref: hal_add_ref,
    release: hal_release,
    set_int: hal_set_int,
    get_int: hal_get_int,
    set_float: hal_set_float,
    get_float: hal_get_float,
    set_string: hal_set_string,
    get_string: hal_get_string,
    set_binary: hal_set_binary,
    get_binary: hal_get_binary,
};

impl HostAttributeListObj {
    /// `HostAttributeList::make()` — new instance with refcount 1.
    pub fn create() -> *mut Self {
        Box::into_raw(Box::new(Self {
            vtbl: &HOST_ATTRIBUTE_LIST_VTBL,
            ref_count: AtomicI32::new(1),
            list: HashMap::new(),
        }))
    }
}

fn cstr_to_key(aid: FIDString) -> String {
    unsafe { std::ffi::CStr::from_ptr(aid).to_string_lossy().into_owned() }
}

fn wide_len_plus_one(s: *const u16) -> usize {
    let mut n = 0usize;
    unsafe {
        while *s.add(n) != 0 {
            n += 1;
        }
    }
    n + 1
}

unsafe extern "system" fn hal_query_interface(
    this: *mut HostAttributeListObj,
    iid: *const u8,
    obj: *mut *mut c_void,
) -> i32 {
    if !qi_preamble(obj) {
        return K_INVALID_ARGUMENT;
    }
    if iid_equal_ptr(iid, &iid::F_UNKNOWN) || iid_equal_ptr(iid, &iid::I_ATTRIBUTE_LIST) {
        atomic_add(&(*this).ref_count, 1);
        return qi_result(obj, this as *mut c_void);
    }
    K_NO_INTERFACE
}

unsafe extern "system" fn hal_add_ref(this: *mut HostAttributeListObj) -> u32 {
    atomic_add(&(*this).ref_count, 1) as u32
}

unsafe extern "system" fn hal_release(this: *mut HostAttributeListObj) -> u32 {
    let count = atomic_add(&(*this).ref_count, -1);
    if count == 0 {
        drop(Box::from_raw(this));
        return 0;
    }
    count as u32
}

unsafe extern "system" fn hal_set_int(
    this: *mut HostAttributeListObj,
    aid: FIDString,
    value: i64,
) -> i32 {
    if aid.is_null() {
        return K_INVALID_ARGUMENT;
    }
    (*this)
        .list
        .insert(cstr_to_key(aid), Attribute::Integer(value));
    K_RESULT_TRUE
}

unsafe extern "system" fn hal_get_int(
    this: *mut HostAttributeListObj,
    aid: FIDString,
    value: *mut i64,
) -> i32 {
    if aid.is_null() {
        return K_INVALID_ARGUMENT;
    }
    match (*this).list.get(&cstr_to_key(aid)) {
        Some(Attribute::Integer(v)) => {
            *value = *v;
            K_RESULT_TRUE
        }
        _ => K_RESULT_FALSE,
    }
}

unsafe extern "system" fn hal_set_float(
    this: *mut HostAttributeListObj,
    aid: FIDString,
    value: f64,
) -> i32 {
    if aid.is_null() {
        return K_INVALID_ARGUMENT;
    }
    (*this)
        .list
        .insert(cstr_to_key(aid), Attribute::Float(value));
    K_RESULT_TRUE
}

unsafe extern "system" fn hal_get_float(
    this: *mut HostAttributeListObj,
    aid: FIDString,
    value: *mut f64,
) -> i32 {
    if aid.is_null() {
        return K_INVALID_ARGUMENT;
    }
    match (*this).list.get(&cstr_to_key(aid)) {
        Some(Attribute::Float(v)) => {
            *value = *v;
            K_RESULT_TRUE
        }
        _ => K_RESULT_FALSE,
    }
}

unsafe extern "system" fn hal_set_string(
    this: *mut HostAttributeListObj,
    aid: FIDString,
    string: *const u16,
) -> i32 {
    if aid.is_null() {
        return K_INVALID_ARGUMENT;
    }
    // + 1 for the null-terminate
    let length = wide_len_plus_one(string);
    let slice = std::slice::from_raw_parts(string, length);
    (*this)
        .list
        .insert(cstr_to_key(aid), Attribute::String(slice.to_vec()));
    K_RESULT_TRUE
}

unsafe extern "system" fn hal_get_string(
    this: *mut HostAttributeListObj,
    aid: FIDString,
    string: *mut u16,
    size_in_bytes: u32,
) -> i32 {
    if aid.is_null() {
        return K_INVALID_ARGUMENT;
    }
    match (*this).list.get(&cstr_to_key(aid)) {
        Some(Attribute::String(v)) => {
            let bytes = std::cmp::min(v.len() * 2, size_in_bytes as usize);
            std::ptr::copy_nonoverlapping(v.as_ptr() as *const u8, string as *mut u8, bytes);
            K_RESULT_TRUE
        }
        _ => K_RESULT_FALSE,
    }
}

unsafe extern "system" fn hal_set_binary(
    this: *mut HostAttributeListObj,
    aid: FIDString,
    data: *const c_void,
    size_in_bytes: u32,
) -> i32 {
    if aid.is_null() {
        return K_INVALID_ARGUMENT;
    }
    let value = if data.is_null() || size_in_bytes == 0 {
        Vec::new()
    } else {
        std::slice::from_raw_parts(data as *const u8, size_in_bytes as usize).to_vec()
    };
    (*this)
        .list
        .insert(cstr_to_key(aid), Attribute::Binary(value));
    K_RESULT_TRUE
}

unsafe extern "system" fn hal_get_binary(
    this: *mut HostAttributeListObj,
    aid: FIDString,
    data: *mut *const c_void,
    size_in_bytes: *mut u32,
) -> i32 {
    if aid.is_null() {
        return K_INVALID_ARGUMENT;
    }
    match (*this).list.get(&cstr_to_key(aid)) {
        Some(Attribute::Binary(v)) => {
            *data = v.as_ptr() as *const c_void;
            *size_in_bytes = v.len() as u32;
            K_RESULT_TRUE
        }
        _ => {
            *size_in_bytes = 0;
            K_RESULT_FALSE
        }
    }
}

// ===========================================================================
// HostMessage (hostclasses.{h,cpp})
// ===========================================================================

#[repr(C)]
pub struct HostMessageObj {
    pub vtbl: *const HostMessageVtbl,
    pub ref_count: AtomicI32,
    message_id: Vec<i8>,
    attribute_list: *mut HostAttributeListObj,
}

static HOST_MESSAGE_VTBL: HostMessageVtbl = HostMessageVtbl {
    query_interface: hm_query_interface,
    add_ref: hm_add_ref,
    release: hm_release,
    get_message_id: hm_get_message_id,
    set_message_id: hm_set_message_id,
    get_attributes: hm_get_attributes,
};

impl HostMessageObj {
    pub fn create() -> *mut Self {
        Box::into_raw(Box::new(Self {
            vtbl: &HOST_MESSAGE_VTBL,
            ref_count: AtomicI32::new(1),
            message_id: Vec::new(),
            attribute_list: std::ptr::null_mut(),
        }))
    }
}

impl Drop for HostMessageObj {
    fn drop(&mut self) {
        // ~HostMessage: setMessageID (nullptr) + IPtr<IAttributeList> release.
        self.message_id.clear();
        if !self.attribute_list.is_null() {
            unsafe {
                com_release(self.attribute_list);
            }
            self.attribute_list = std::ptr::null_mut();
        }
    }
}

unsafe extern "system" fn hm_query_interface(
    this: *mut HostMessageObj,
    iid: *const u8,
    obj: *mut *mut c_void,
) -> i32 {
    if !qi_preamble(obj) {
        return K_INVALID_ARGUMENT;
    }
    if iid_equal_ptr(iid, &iid::F_UNKNOWN) || iid_equal_ptr(iid, &iid::I_MESSAGE) {
        atomic_add(&(*this).ref_count, 1);
        return qi_result(obj, this as *mut c_void);
    }
    K_NO_INTERFACE
}

unsafe extern "system" fn hm_add_ref(this: *mut HostMessageObj) -> u32 {
    atomic_add(&(*this).ref_count, 1) as u32
}

unsafe extern "system" fn hm_release(this: *mut HostMessageObj) -> u32 {
    let count = atomic_add(&(*this).ref_count, -1);
    if count == 0 {
        drop(Box::from_raw(this));
        return 0;
    }
    count as u32
}

unsafe extern "system" fn hm_get_message_id(this: *mut HostMessageObj) -> FIDString {
    let id = &(*this).message_id;
    if id.is_empty() {
        std::ptr::null()
    } else {
        id.as_ptr()
    }
}

unsafe extern "system" fn hm_set_message_id(this: *mut HostMessageObj, mid: FIDString) {
    let msg = &mut *this;
    msg.message_id.clear();
    if !mid.is_null() {
        let len = std::ffi::CStr::from_ptr(mid).to_bytes().len() + 1;
        let slice = std::slice::from_raw_parts(mid, len);
        msg.message_id = slice.to_vec();
    }
}

unsafe extern "system" fn hm_get_attributes(this: *mut HostMessageObj) -> *mut c_void {
    let msg = &mut *this;
    if msg.attribute_list.is_null() {
        msg.attribute_list = HostAttributeListObj::create();
    }
    msg.attribute_list as *mut c_void
}

// ===========================================================================
// HostApplication (hostclasses.{h,cpp})
// ===========================================================================

#[repr(C)]
pub struct HostApplicationObj {
    pub vtbl: *const HostApplicationVtbl,
    pub plug_interface_support: *mut PlugInterfaceSupportObj,
}

static HOST_APPLICATION_VTBL: HostApplicationVtbl = HostApplicationVtbl {
    query_interface: ha_query_interface,
    add_ref: ha_add_ref,
    release: ha_release,
    get_name: ha_get_name,
    create_instance: ha_create_instance,
};

const HOST_APPLICATION_NAME: &str = "My VST3 HostApplication";

impl HostApplicationObj {
    pub fn create() -> Box<Self> {
        Box::new(Self {
            vtbl: &HOST_APPLICATION_VTBL,
            plug_interface_support: PlugInterfaceSupportObj::create(),
        })
    }

    pub fn as_ptr(b: &mut Box<Self>) -> *mut c_void {
        b.as_mut() as *mut Self as *mut c_void
    }
}

impl Drop for HostApplicationObj {
    fn drop(&mut self) {
        // ~HostApplication: IPtr<PlugInterfaceSupport> release.
        if !self.plug_interface_support.is_null() {
            unsafe {
                com_release(self.plug_interface_support);
            }
            self.plug_interface_support = std::ptr::null_mut();
        }
    }
}

unsafe extern "system" fn ha_query_interface(
    this: *mut HostApplicationObj,
    iid: *const u8,
    obj: *mut *mut c_void,
) -> i32 {
    if !qi_preamble(obj) {
        return K_INVALID_ARGUMENT;
    }
    if iid_equal_ptr(iid, &iid::F_UNKNOWN) || iid_equal_ptr(iid, &iid::I_HOST_APPLICATION) {
        // addRef() returns a constant 1 in the C++ implementation.
        return qi_result(obj, this as *mut c_void);
    }

    let support = (*this).plug_interface_support;
    if !support.is_null() {
        let vtbl = (*support).vtbl;
        let f = (*vtbl).query_interface;
        if f(support, iid, obj) == K_RESULT_TRUE {
            return K_RESULT_OK;
        }
    }

    *obj = std::ptr::null_mut();
    K_RESULT_FALSE
}

unsafe extern "system" fn ha_add_ref(_this: *mut HostApplicationObj) -> u32 {
    1
}
unsafe extern "system" fn ha_release(_this: *mut HostApplicationObj) -> u32 {
    1
}

unsafe extern "system" fn ha_get_name(this: *mut HostApplicationObj, name: *mut u16) -> i32 {
    let _ = this;
    if name.is_null() {
        return K_INTERNAL_ERROR;
    }
    let units: Vec<u16> = HOST_APPLICATION_NAME.encode_utf16().collect();
    if units.len() + 1 > 128 {
        return K_INTERNAL_ERROR;
    }
    let out = std::slice::from_raw_parts_mut(name, 128);
    out[..units.len()].copy_from_slice(&units);
    out[units.len()] = 0;
    K_RESULT_TRUE
}

unsafe extern "system" fn ha_create_instance(
    _this: *mut HostApplicationObj,
    cid: *const u8,
    iid: *const u8,
    obj: *mut *mut c_void,
) -> i32 {
    if iid_equal_ptr(cid, &iid::I_MESSAGE) && iid_equal_ptr(iid, &iid::I_MESSAGE) {
        *obj = HostMessageObj::create() as *mut c_void;
        return K_RESULT_TRUE;
    }
    if iid_equal_ptr(cid, &iid::I_ATTRIBUTE_LIST) && iid_equal_ptr(iid, &iid::I_ATTRIBUTE_LIST) {
        *obj = HostAttributeListObj::create() as *mut c_void;
        return K_RESULT_TRUE;
    }
    *obj = std::ptr::null_mut();
    K_RESULT_FALSE
}

// ===========================================================================
// MemoryStream (memorystream.{h,cpp})
// ===========================================================================

enum StreamMemory {
    /// Default constructor: owns a growing heap buffer.
    Owned(Vec<u8>),
    /// `MemoryStream (void* data, TSize length)`: reuses external memory.
    External { ptr: *mut u8, len: usize },
}

#[repr(C)]
pub struct MemoryStreamObj {
    pub vtbl: *const MemoryStreamVtbl,
    pub ref_count: AtomicI32,
    memory: StreamMemory,
    memory_size: usize,
    size: usize,
    cursor: i64,
    allocation_error: bool,
}

static MEMORY_STREAM_VTBL: MemoryStreamVtbl = MemoryStreamVtbl {
    query_interface: ms_query_interface,
    add_ref: ms_add_ref,
    release: ms_release,
    read: ms_read,
    write: ms_write,
    seek: ms_seek,
    tell: ms_tell,
};

const K_MEM_GROW_AMOUNT: usize = 4096;

impl MemoryStreamObj {
    /// Returns a raw pointer with refcount 1. The host releases it when done.
    pub fn create_owned() -> *mut Self {
        Box::into_raw(Box::new(Self {
            vtbl: &MEMORY_STREAM_VTBL,
            ref_count: AtomicI32::new(1),
            memory: StreamMemory::Owned(Vec::new()),
            memory_size: 0,
            size: 0,
            cursor: 0,
            allocation_error: false,
        }))
    }

    /// `MemoryStream (void* memory, TSize memorySize)` — no ownership.
    pub fn create_external(data: *mut u8, len: usize) -> *mut Self {
        Box::into_raw(Box::new(Self {
            vtbl: &MEMORY_STREAM_VTBL,
            ref_count: AtomicI32::new(1),
            memory: StreamMemory::External { ptr: data, len },
            memory_size: len,
            size: len,
            cursor: 0,
            allocation_error: false,
        }))
    }

    pub fn get_size(&self) -> u64 {
        self.size as u64
    }

    pub fn get_data(&self) -> *mut u8 {
        match &self.memory {
            StreamMemory::Owned(v) => {
                if v.is_empty() {
                    std::ptr::null_mut()
                } else {
                    v.as_ptr() as *mut u8
                }
            }
            StreamMemory::External { ptr, .. } => *ptr,
        }
    }

    fn set_size(&mut self, s: usize) {
        if s == 0 {
            if let StreamMemory::Owned(_) = self.memory {
                self.memory = StreamMemory::Owned(Vec::new());
            }
            self.memory_size = 0;
            self.size = 0;
            self.cursor = 0;
            return;
        }

        let new_memory_size =
            (((std::cmp::max(self.memory_size, s) - 1) / K_MEM_GROW_AMOUNT) + 1) * K_MEM_GROW_AMOUNT;
        if new_memory_size == self.memory_size {
            self.size = s;
            return;
        }

        match &mut self.memory {
            StreamMemory::External { .. } => {
                self.allocation_error = true;
                return;
            }
            StreamMemory::Owned(buf) => {
                buf.resize(new_memory_size, 0);
                self.memory_size = new_memory_size;
                self.size = s;
            }
        }
    }
}

impl Drop for MemoryStreamObj {
    fn drop(&mut self) {
        // BUFFER free happens via the Rust value; nothing else to do.
    }
}

unsafe extern "system" fn ms_query_interface(
    this: *mut MemoryStreamObj,
    iid: *const u8,
    obj: *mut *mut c_void,
) -> i32 {
    if !qi_preamble(obj) {
        return K_INVALID_ARGUMENT;
    }
    // IMPLEMENT_FUNKNOWN_METHODS (MemoryStream, IBStream, IBStream::iid):
    // FUnknown::iid and IBStream::iid only.
    if iid_equal_ptr(iid, &iid::F_UNKNOWN) || iid_equal_ptr(iid, &iid::I_BSTREAM) {
        atomic_add(&(*this).ref_count, 1);
        return qi_result(obj, this as *mut c_void);
    }
    K_NO_INTERFACE
}

unsafe extern "system" fn ms_add_ref(this: *mut MemoryStreamObj) -> u32 {
    atomic_add(&(*this).ref_count, 1) as u32
}

unsafe extern "system" fn ms_release(this: *mut MemoryStreamObj) -> u32 {
    let count = atomic_add(&(*this).ref_count, -1);
    if count == 0 {
        drop(Box::from_raw(this));
        return 0;
    }
    count as u32
}

unsafe extern "system" fn ms_read(
    this: *mut MemoryStreamObj,
    data: *mut c_void,
    num_bytes: i32,
    num_bytes_read: *mut i32,
) -> i32 {
    crate::catch_panic_i32(K_INTERNAL_ERROR, || {
        let stream = &mut *this;
        let mut num_bytes = num_bytes;

        let base: *mut u8 = match &stream.memory {
            StreamMemory::Owned(v) => {
                if v.is_empty() {
                    std::ptr::null_mut()
                } else {
                    v.as_ptr() as *mut u8
                }
            }
            StreamMemory::External { ptr, .. } => *ptr,
        };

        if base.is_null() {
            if stream.allocation_error {
                return K_OUT_OF_MEMORY;
            }
            num_bytes = 0;
        } else {
            // Does read exceed size?
            if stream.cursor + num_bytes as i64 > stream.size as i64 {
                let max_bytes = stream.size as i64 - stream.cursor;
                if max_bytes <= 0 {
                    stream.cursor = stream.size as i64;
                    num_bytes = 0;
                } else {
                    num_bytes = max_bytes as i32;
                }
            }

            if num_bytes != 0 {
                std::ptr::copy_nonoverlapping(
                    base.offset(stream.cursor as isize),
                    data as *mut u8,
                    num_bytes as usize,
                );
                stream.cursor += num_bytes as i64;
            }
        }

        if !num_bytes_read.is_null() {
            *num_bytes_read = num_bytes;
        }
        K_RESULT_TRUE
    })
}

unsafe extern "system" fn ms_write(
    this: *mut MemoryStreamObj,
    buffer: *mut c_void,
    num_bytes: i32,
    num_bytes_written: *mut i32,
) -> i32 {
    crate::catch_panic_i32(K_INTERNAL_ERROR, || {
        let stream = &mut *this;
        if stream.allocation_error {
            return K_OUT_OF_MEMORY;
        }
        if buffer.is_null() {
            return K_INVALID_ARGUMENT;
        }

        let mut num_bytes = num_bytes;
        let required_size = stream.cursor + num_bytes as i64;
        if required_size > stream.size as i64 {
            if required_size > stream.memory_size as i64 {
                stream.set_size(required_size as usize);
            } else {
                stream.size = required_size as usize;
            }
        }

        let base: *mut u8 = match &stream.memory {
            StreamMemory::Owned(v) => {
                if v.is_empty() {
                    std::ptr::null_mut()
                } else {
                    v.as_ptr() as *mut u8
                }
            }
            StreamMemory::External { ptr, .. } => *ptr,
        };

        if !base.is_null() && stream.cursor >= 0 && num_bytes > 0 {
            // SAFETY note: the C++ copies unconditionally here even when a
            // failed set_size() left the stream smaller; we clamp to the
            // allocated block instead of overflowing it.
            let available = (stream.memory_size as i64 - stream.cursor).max(0) as usize;
            let to_copy = (num_bytes as usize).min(available);
            if to_copy == 0 {
                num_bytes = 0;
            } else {
                std::ptr::copy_nonoverlapping(
                    buffer as *const u8,
                    base.offset(stream.cursor as isize),
                    to_copy,
                );
                stream.cursor += to_copy as i64;
                if to_copy < num_bytes as usize {
                    num_bytes = to_copy as i32;
                }
            }
        } else {
            num_bytes = 0;
        }

        if !num_bytes_written.is_null() {
            *num_bytes_written = num_bytes;
        }
        K_RESULT_TRUE
    })
}

unsafe extern "system" fn ms_seek(
    this: *mut MemoryStreamObj,
    pos: i64,
    mode: i32,
    result: *mut i64,
) -> i32 {
    let stream = &mut *this;
    match mode {
        0 => stream.cursor = pos,             // kIBSeekSet
        1 => stream.cursor += pos,            // kIBSeekCur
        2 => stream.cursor = stream.size as i64 + pos, // kIBSeekEnd
        _ => {}
    }

    if let StreamMemory::External { .. } = stream.memory {
        if stream.cursor > stream.memory_size as i64 {
            stream.cursor = stream.memory_size as i64;
        }
    }

    if !result.is_null() {
        *result = stream.cursor;
    }
    K_RESULT_TRUE
}

unsafe extern "system" fn ms_tell(this: *mut MemoryStreamObj, pos: *mut i64) -> i32 {
    if pos.is_null() {
        return K_INVALID_ARGUMENT;
    }
    *pos = (*this).cursor;
    K_RESULT_TRUE
}

// ===========================================================================
// ParameterValueQueue / ParameterChanges / ParameterChangeTransfer
// (parameterchanges.{h,cpp})
// ===========================================================================

#[repr(C)]
pub struct ParameterValueQueueObj {
    pub vtbl: *const ParameterValueQueueVtbl,
    pub ref_count: AtomicI32,
    param_id: u32,
    values: Vec<(f64, i32)>, // (value, sampleOffset)
}

static PARAM_VALUE_QUEUE_VTBL: ParameterValueQueueVtbl = ParameterValueQueueVtbl {
    query_interface: pvq_query_interface,
    add_ref: pvq_add_ref,
    release: pvq_release,
    get_parameter_id: pvq_get_parameter_id,
    get_point_count: pvq_get_point_count,
    get_point: pvq_get_point,
    add_point: pvq_add_point,
};

impl ParameterValueQueueObj {
    pub fn create(param_id: u32) -> *mut Self {
        Box::into_raw(Box::new(Self {
            vtbl: &PARAM_VALUE_QUEUE_VTBL,
            ref_count: AtomicI32::new(1),
            param_id,
            values: Vec::with_capacity(5), // kQueueReservedPoints
        }))
    }
}

unsafe extern "system" fn pvq_query_interface(
    this: *mut ParameterValueQueueObj,
    iid: *const u8,
    obj: *mut *mut c_void,
) -> i32 {
    if !qi_preamble(obj) {
        return K_INVALID_ARGUMENT;
    }
    if iid_equal_ptr(iid, &iid::F_UNKNOWN) || iid_equal_ptr(iid, &iid::I_PARAM_VALUE_QUEUE) {
        atomic_add(&(*this).ref_count, 1);
        return qi_result(obj, this as *mut c_void);
    }
    K_NO_INTERFACE
}

unsafe extern "system" fn pvq_add_ref(this: *mut ParameterValueQueueObj) -> u32 {
    atomic_add(&(*this).ref_count, 1) as u32
}

unsafe extern "system" fn pvq_release(this: *mut ParameterValueQueueObj) -> u32 {
    let count = atomic_add(&(*this).ref_count, -1);
    if count == 0 {
        drop(Box::from_raw(this));
        return 0;
    }
    count as u32
}

unsafe extern "system" fn pvq_get_parameter_id(this: *mut ParameterValueQueueObj) -> u32 {
    (*this).param_id
}

unsafe extern "system" fn pvq_get_point_count(this: *mut ParameterValueQueueObj) -> i32 {
    (*this).values.len() as i32
}

unsafe extern "system" fn pvq_get_point(
    this: *mut ParameterValueQueueObj,
    index: i32,
    sample_offset: *mut i32,
    value: *mut f64,
) -> i32 {
    let queue = &*this;
    if index >= 0 && (index as usize) < queue.values.len() {
        let (v, offset) = queue.values[index as usize];
        *sample_offset = offset;
        *value = v;
        return K_RESULT_TRUE;
    }
    K_RESULT_FALSE
}

unsafe extern "system" fn pvq_add_point(
    this: *mut ParameterValueQueueObj,
    sample_offset: i32,
    value: f64,
    index: *mut i32,
) -> i32 {
    let queue = &mut *this;
    let mut dest_index = queue.values.len();

    for i in 0..queue.values.len() {
        if queue.values[i].1 == sample_offset {
            queue.values[i].0 = value;
            *index = i as i32;
            return K_RESULT_TRUE;
        }
        if queue.values[i].1 > sample_offset {
            dest_index = i;
            break;
        }
    }

    if dest_index == queue.values.len() {
        queue.values.push((value, sample_offset));
    } else {
        queue.values.insert(dest_index, (value, sample_offset));
    }
    *index = dest_index as i32;
    K_RESULT_TRUE
}

#[repr(C)]
pub struct ParameterChangesObj {
    pub vtbl: *const ParameterChangesVtbl,
    pub ref_count: AtomicI32,
    queues: Vec<*mut ParameterValueQueueObj>,
    used_queue_count: i32,
}

static PARAMETER_CHANGES_VTBL: ParameterChangesVtbl = ParameterChangesVtbl {
    query_interface: pc_query_interface,
    add_ref: pc_add_ref,
    release: pc_release,
    get_parameter_count: pc_get_parameter_count,
    get_parameter_data: pc_get_parameter_data,
    add_parameter_data: pc_add_parameter_data,
};

impl ParameterChangesObj {
    /// `ParameterChanges (int32 maxParameters = 0)` with refcount 1.
    pub fn create(max_parameters: i32) -> *mut Self {
        let obj = Box::into_raw(Box::new(Self {
            vtbl: &PARAMETER_CHANGES_VTBL,
            ref_count: AtomicI32::new(1),
            queues: Vec::new(),
            used_queue_count: 0,
        }));
        unsafe {
            (*obj).set_max_parameters(max_parameters);
        }
        obj
    }

    pub fn set_max_parameters(&mut self, max_parameters: i32) {
        if max_parameters < 0 {
            return;
        }
        while (self.queues.len() as i32) < max_parameters {
            self.queues.push(ParameterValueQueueObj::create(K_NO_PARAM_ID));
        }
        while (self.queues.len() as i32) > max_parameters {
            if let Some(q) = self.queues.pop() {
                unsafe {
                    com_release(q);
                }
            }
        }
        if self.used_queue_count > max_parameters {
            self.used_queue_count = max_parameters;
        }
    }

    /// `clearQueue()`.
    pub fn clear_queue(&mut self) {
        self.used_queue_count = 0;
    }

    /// Raw `addParameterData` for internal use.
    pub fn add_parameter_data(&mut self, pid: u32) -> (*mut ParameterValueQueueObj, i32) {
        for i in 0..self.used_queue_count {
            let q = self.queues[i as usize];
            unsafe {
                if (*q).param_id == pid {
                    return (q, i);
                }
            }
        }

        let value_queue: *mut ParameterValueQueueObj =
            if (self.used_queue_count as usize) < self.queues.len() {
                let q = self.queues[self.used_queue_count as usize];
                unsafe {
                    (*q).param_id = pid;
                    (*q).values.clear();
                }
                q
            } else {
                let q = ParameterValueQueueObj::create(pid);
                self.queues.push(q);
                q
            };

        let index = self.used_queue_count;
        self.used_queue_count += 1;
        (value_queue, index)
    }
}

impl Drop for ParameterChangesObj {
    fn drop(&mut self) {
        for q in self.queues.drain(..) {
            unsafe {
                com_release(q);
            }
        }
    }
}

unsafe extern "system" fn pc_query_interface(
    this: *mut ParameterChangesObj,
    iid: *const u8,
    obj: *mut *mut c_void,
) -> i32 {
    if !qi_preamble(obj) {
        return K_INVALID_ARGUMENT;
    }
    if iid_equal_ptr(iid, &iid::F_UNKNOWN) || iid_equal_ptr(iid, &iid::I_PARAMETER_CHANGES) {
        atomic_add(&(*this).ref_count, 1);
        return qi_result(obj, this as *mut c_void);
    }
    K_NO_INTERFACE
}

unsafe extern "system" fn pc_add_ref(this: *mut ParameterChangesObj) -> u32 {
    atomic_add(&(*this).ref_count, 1) as u32
}

unsafe extern "system" fn pc_release(this: *mut ParameterChangesObj) -> u32 {
    let count = atomic_add(&(*this).ref_count, -1);
    if count == 0 {
        drop(Box::from_raw(this));
        return 0;
    }
    count as u32
}

unsafe extern "system" fn pc_get_parameter_count(this: *mut ParameterChangesObj) -> i32 {
    (*this).used_queue_count
}

unsafe extern "system" fn pc_get_parameter_data(
    this: *mut ParameterChangesObj,
    index: i32,
) -> *mut ParameterValueQueueObj {
    let changes = &*this;
    if index >= 0 && index < changes.used_queue_count {
        changes.queues[index as usize]
    } else {
        std::ptr::null_mut()
    }
}

unsafe extern "system" fn pc_add_parameter_data(
    this: *mut ParameterChangesObj,
    pid: *const u32,
    index: *mut i32,
) -> *mut ParameterValueQueueObj {
    let (queue, i) = (*this).add_parameter_data(*pid);
    *index = i;
    queue
}

/// Ring buffer for transferring parameter changes from a writer to a read
/// thread — a direct port of `ParameterChangeTransfer`.
pub struct ParameterChangeTransfer {
    changes: Vec<(u32, f64, i32)>, // (id, value, sampleOffset)
    read_index: i32,
    write_index: i32,
}

impl ParameterChangeTransfer {
    pub fn new(max_parameters: i32) -> Self {
        let mut transfer = Self {
            changes: Vec::new(),
            read_index: 0,
            write_index: 0,
        };
        transfer.set_max_parameters(max_parameters);
        transfer
    }

    pub fn set_max_parameters(&mut self, max_parameters: i32) {
        // reserve memory for twice the amount of all parameters
        let new_size = (max_parameters * 2).max(0) as usize;
        if self.changes.len() != new_size {
            self.changes = vec![(0u32, 0.0f64, 0i32); new_size];
        }
    }

    pub fn add_change(&mut self, pid: u32, value: f64, sample_offset: i32) {
        if self.changes.is_empty() {
            return;
        }
        let size = self.changes.len() as i32;
        self.changes[self.write_index as usize] = (pid, value, sample_offset);

        let mut new_write_index = self.write_index + 1;
        if new_write_index >= size {
            new_write_index = 0;
        }
        if self.read_index != new_write_index {
            self.write_index = new_write_index;
        }
    }

    pub fn get_next_change(&mut self) -> Option<(u32, f64, i32)> {
        if self.changes.is_empty() {
            return None;
        }
        let current_write_index = self.write_index;
        if self.read_index != current_write_index {
            let change = self.changes[self.read_index as usize];
            let mut new_read_index = self.read_index + 1;
            if new_read_index >= self.changes.len() as i32 {
                new_read_index = 0;
            }
            self.read_index = new_read_index;
            return Some(change);
        }
        None
    }

    pub fn transfer_changes_to(&mut self, dest: &mut ParameterChangesObj) {
        while let Some((pid, value, sample_offset)) = self.get_next_change() {
            let (queue, _index) = dest.add_parameter_data(pid);
            unsafe {
                let mut point_index: i32 = 0;
                let vtbl = (*queue).vtbl;
                ((*vtbl).add_point)(queue, sample_offset, value, &mut point_index);
            }
        }
    }

    pub fn transfer_changes_from(&mut self, source: &ParameterChangesObj) {
        for i in 0..source.used_queue_count {
            let queue = source.queues[i as usize];
            unsafe {
                let vtbl = (*queue).vtbl;
                let point_count = ((*vtbl).get_point_count)(queue);
                for j in 0..point_count {
                    let mut sample_offset = 0i32;
                    let mut value = 0f64;
                    if ((*vtbl).get_point)(queue, j, &mut sample_offset, &mut value) == K_RESULT_TRUE
                    {
                        self.add_change((*queue).param_id, value, sample_offset);
                    }
                }
            }
        }
    }

    pub fn remove_changes(&mut self) {
        self.write_index = self.read_index;
    }
}

// ===========================================================================
// ConnectionProxy (connectionproxy.{h,cpp})
// ===========================================================================

#[repr(C)]
pub struct ConnectionProxyObj {
    pub vtbl: *const ConnectionProxyVtbl,
    pub ref_count: AtomicI32,
    thread_id: std::thread::ThreadId,
    src_connection: *mut c_void,
    dst_connection: *mut c_void,
}

static CONNECTION_PROXY_VTBL: ConnectionProxyVtbl = ConnectionProxyVtbl {
    query_interface: cp_query_interface,
    add_ref: cp_add_ref,
    release: cp_release,
    connect: cp_connect,
    disconnect: cp_disconnect,
    notify: cp_notify,
};

impl ConnectionProxyObj {
    /// `ConnectionProxy (IConnectionPoint* src)` — shares (addRefs) `src`.
    pub fn create(src: *mut c_void) -> *mut Self {
        unsafe {
            if !src.is_null() {
                com_add_ref(src as *mut IConnectionPointObj);
            }
        }
        Box::into_raw(Box::new(Self {
            vtbl: &CONNECTION_PROXY_VTBL,
            ref_count: AtomicI32::new(1),
            thread_id: std::thread::current().id(),
            src_connection: src,
            dst_connection: std::ptr::null_mut(),
        }))
    }

    /// `bool disconnect ()` — disconnects from the current destination.
    pub fn disconnect_current(&mut self) -> bool {
        let dst = self.dst_connection;
        self.disconnect(dst) == K_RESULT_TRUE
    }

    pub fn connect_external(&mut self, other: *mut c_void) -> i32 {
        if other.is_null() {
            return K_INVALID_ARGUMENT;
        }
        if !self.dst_connection.is_null() {
            return K_RESULT_FALSE;
        }

        self.dst_connection = other;
        unsafe {
            com_add_ref(other as *mut IConnectionPointObj);
        }

        let res = unsafe {
            let src = self.src_connection as *mut IConnectionPointObj;
            ((*(*src).vtbl).connect)(src as *mut c_void, self as *mut Self as *mut c_void)
        };
        if res != K_RESULT_TRUE {
            unsafe {
                com_release(self.dst_connection as *mut IConnectionPointObj);
            }
            self.dst_connection = std::ptr::null_mut();
        }
        res
    }

    fn disconnect(&mut self, other: *mut c_void) -> i32 {
        if other.is_null() {
            return K_INVALID_ARGUMENT;
        }
        if other == self.dst_connection {
            if !self.src_connection.is_null() {
                unsafe {
                    let src = self.src_connection as *mut IConnectionPointObj;
                    ((*(*src).vtbl).disconnect)(src as *mut c_void, self as *mut Self as *mut c_void);
                }
            }
            unsafe {
                com_release(self.dst_connection as *mut IConnectionPointObj);
            }
            self.dst_connection = std::ptr::null_mut();
            return K_RESULT_TRUE;
        }
        K_INVALID_ARGUMENT
    }
}

impl Drop for ConnectionProxyObj {
    fn drop(&mut self) {
        unsafe {
            if !self.dst_connection.is_null() {
                com_release(self.dst_connection as *mut IConnectionPointObj);
                self.dst_connection = std::ptr::null_mut();
            }
            if !self.src_connection.is_null() {
                com_release(self.src_connection as *mut IConnectionPointObj);
                self.src_connection = std::ptr::null_mut();
            }
        }
    }
}

unsafe extern "system" fn cp_query_interface(
    this: *mut ConnectionProxyObj,
    iid: *const u8,
    obj: *mut *mut c_void,
) -> i32 {
    if !qi_preamble(obj) {
        return K_INVALID_ARGUMENT;
    }
    if iid_equal_ptr(iid, &iid::F_UNKNOWN) || iid_equal_ptr(iid, &iid::I_CONNECTION_POINT) {
        atomic_add(&(*this).ref_count, 1);
        return qi_result(obj, this as *mut c_void);
    }
    K_NO_INTERFACE
}

unsafe extern "system" fn cp_add_ref(this: *mut ConnectionProxyObj) -> u32 {
    atomic_add(&(*this).ref_count, 1) as u32
}

unsafe extern "system" fn cp_release(this: *mut ConnectionProxyObj) -> u32 {
    let count = atomic_add(&(*this).ref_count, -1);
    if count == 0 {
        drop(Box::from_raw(this));
        return 0;
    }
    count as u32
}

unsafe extern "system" fn cp_connect(this: *mut ConnectionProxyObj, other: *mut c_void) -> i32 {
    (*this).connect_external(other)
}

unsafe extern "system" fn cp_disconnect(this: *mut ConnectionProxyObj, other: *mut c_void) -> i32 {
    (*this).disconnect(other)
}

unsafe extern "system" fn cp_notify(this: *mut ConnectionProxyObj, message: *mut c_void) -> i32 {
    let proxy = &*this;
    if !proxy.dst_connection.is_null() {
        // ThreadChecker: pass the message through only on the thread that
        // created the proxy (Windows GetCurrentThreadId semantics).
        if std::thread::current().id() == proxy.thread_id {
            let dst = proxy.dst_connection as *mut IConnectionPointObj;
            return ((*(*dst).vtbl).notify)(dst as *mut c_void, message);
        }
    }
    K_RESULT_FALSE
}
