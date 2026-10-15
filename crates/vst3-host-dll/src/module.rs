//! Win32 plugin-module loading, ported 1:1 from
//! `public.sdk/source/vst/hosting/module.h`, `module.cpp` and
//! `module_win32.cpp` (SDK 3.7.12).
#![allow(non_snake_case, dead_code)]
#![allow(clippy::missing_safety_doc)]

use std::ffi::c_void;
use std::path::PathBuf;

use windows::core::{PCSTR, PCWSTR};
use windows::Win32::Foundation::{FreeLibrary, HMODULE};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

use crate::vst3abi::*;

pub const ARCHITECTURE_STRING: &str = "x86_64-win";

/// `PLUGIN_API bool (*)()` entry points (`InitDll`/`ExitDll`).
type ModuleInitFunc = unsafe extern "system" fn() -> u8;
type GetFactoryProc = unsafe extern "system" fn() -> *mut c_void;

// ---------------------------------------------------------------------------
// ClassInfo
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
pub struct ClassInfo {
    pub class_id: TUID,
    pub cardinality: i32,
    pub category: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub sdk_version: String,
    pub sub_categories: Vec<String>,
    pub class_flags: u32,
}

impl ClassInfo {
    fn from_p_class_info(info: &PClassInfo) -> Self {
        Self {
            class_id: info.cid,
            cardinality: info.cardinality,
            category: fixed_cstr_to_string(&info.category),
            name: fixed_cstr_to_string(&info.name),
            ..Default::default()
        }
    }

    fn from_p_class_info2(info: &PClassInfo2) -> Self {
        Self {
            class_id: info.cid,
            cardinality: info.cardinality,
            category: fixed_cstr_to_string(&info.category),
            name: fixed_cstr_to_string(&info.name),
            vendor: fixed_cstr_to_string(&info.vendor),
            version: fixed_cstr_to_string(&info.version),
            sdk_version: fixed_cstr_to_string(&info.sdk_version),
            sub_categories: parse_sub_categories(&fixed_cstr_to_string(&info.sub_categories)),
            class_flags: info.class_flags,
        }
    }

    fn from_p_class_info_w(info: &PClassInfoW) -> Self {
        Self {
            class_id: info.cid,
            cardinality: info.cardinality,
            category: fixed_cstr_to_string(&info.category),
            name: fixed_wstr_to_string(&info.name),
            vendor: fixed_wstr_to_string(&info.vendor),
            version: fixed_wstr_to_string(&info.version),
            sdk_version: fixed_wstr_to_string(&info.sdk_version),
            sub_categories: parse_sub_categories(&fixed_cstr_to_string(&info.sub_categories)),
            class_flags: info.class_flags,
        }
    }

    /// `UID::toString (comFormat = true)`: `%08X%04X%04X` + 8 hex bytes.
    pub fn id_to_string(&self) -> String {
        let d = &self.class_id;
        let data1 = u32::from_le_bytes([d[0], d[1], d[2], d[3]]);
        let data2 = u16::from_le_bytes([d[4], d[5]]);
        let data3 = u16::from_le_bytes([d[6], d[7]]);
        let mut out = format!("{data1:08X}{data2:04X}{data3:04X}");
        for byte in &d[8..16] {
            out.push_str(&format!("{byte:02X}"));
        }
        out
    }
}

fn parse_sub_categories(s: &str) -> Vec<String> {
    s.split('|').map(|item| item.to_string()).collect()
}

// ---------------------------------------------------------------------------
// PluginFactory
// ---------------------------------------------------------------------------

/// Owns one reference to a plugin's `IPluginFactory`.
pub struct PluginFactory {
    raw: *mut IPluginFactoryObj,
}

impl PluginFactory {
    fn null() -> Self {
        Self {
            raw: std::ptr::null_mut(),
        }
    }

    /// Takes ownership of an existing reference (`IPtr` semantics).
    unsafe fn from_owned(raw: *mut IPluginFactoryObj) -> Self {
        Self { raw }
    }

    /// `IPtr` copy semantics (addRef).
    pub unsafe fn clone_ptr(&self) -> Self {
        if !self.raw.is_null() {
            com_add_ref(self.raw);
        }
        Self { raw: self.raw }
    }

    pub fn get(&self) -> *mut IPluginFactoryObj {
        self.raw
    }

    pub fn is_null(&self) -> bool {
        self.raw.is_null()
    }

    pub fn info(&self) -> PFactoryInfo {
        let mut factory_info = PFactoryInfo::default();
        unsafe {
            let _ = (*self.raw).get_factory_info(&mut factory_info);
        }
        factory_info
    }

    fn vendor_string(info: &PFactoryInfo) -> String {
        fixed_cstr_to_string(&info.vendor)
    }

    /// `PluginFactory::classInfos()`: IPluginFactory3 -> 2 -> 1 fallback per
    /// class, with the vendor backfilled from the factory info.
    pub fn class_infos(&self) -> Vec<ClassInfo> {
        let count = unsafe { (*self.raw).count_classes() };
        let count = count.max(0) as usize;

        let mut factory_info: Option<PFactoryInfo> = None;
        let mut classes: Vec<ClassInfo> = Vec::with_capacity(count);

        // U::cast<IPluginFactory3> / U::cast<IPluginFactory2> — owned refs.
        let f3 = unsafe { com_query_interface(self.raw, &iid::I_PLUGIN_FACTORY_3) };
        let f2 = unsafe { com_query_interface(self.raw, &iid::I_PLUGIN_FACTORY_2) };

        for i in 0..count {
            let mut class_info: Option<ClassInfo> = None;

            if let Some(f3_ptr) = f3 {
                let obj = f3_ptr as *mut IPluginFactoryObj;
                let vtbl = unsafe { (*obj).vtbl as *const IPluginFactory3Vtbl };
                let mut ci3 = PClassInfoW {
                    cid: [0; 16],
                    cardinality: 0,
                    category: [0; 32],
                    name: [0; 64],
                    class_flags: 0,
                    sub_categories: [0; 128],
                    vendor: [0; 64],
                    version: [0; 64],
                    sdk_version: [0; 64],
                };
                if unsafe { ((*vtbl).get_class_info_unicode)(f3_ptr, i as i32, &mut ci3) }
                    == K_RESULT_TRUE
                {
                    class_info = Some(ClassInfo::from_p_class_info_w(&ci3));
                }
            }

            if class_info.is_none() {
                if let Some(f2_ptr) = f2 {
                    let obj = f2_ptr as *mut IPluginFactoryObj;
                    let vtbl = unsafe { (*obj).vtbl as *const IPluginFactory2Vtbl };
                    let mut ci2 = PClassInfo2 {
                        cid: [0; 16],
                        cardinality: 0,
                        category: [0; 32],
                        name: [0; 64],
                        class_flags: 0,
                        sub_categories: [0; 128],
                        vendor: [0; 64],
                        version: [0; 64],
                        sdk_version: [0; 64],
                    };
                    if unsafe { ((*vtbl).get_class_info2)(f2_ptr, i as i32, &mut ci2) }
                        == K_RESULT_TRUE
                    {
                        class_info = Some(ClassInfo::from_p_class_info2(&ci2));
                    }
                }
            }

            if class_info.is_none() {
                let mut ci = PClassInfo {
                    cid: [0; 16],
                    cardinality: 0,
                    category: [0; 32],
                    name: [0; 64],
                };
                if unsafe { (*self.raw).get_class_info(i as i32, &mut ci) } == K_RESULT_TRUE {
                    class_info = Some(ClassInfo::from_p_class_info(&ci));
                }
            }

            // The C++ calls `classes.back()` unconditionally here (UB when all
            // three getClassInfo calls failed); we skip such entries instead.
            let Some(mut class_info) = class_info else {
                continue;
            };

            if class_info.vendor.is_empty() {
                if factory_info.is_none() {
                    factory_info = Some(self.info());
                }
                if let Some(info) = &factory_info {
                    class_info.vendor = Self::vendor_string(info);
                }
            }

            classes.push(class_info);
        }

        unsafe {
            if let Some(f3_ptr) = f3 {
                com_release(f3_ptr as *mut c_void);
            }
            if let Some(f2_ptr) = f2 {
                com_release(f2_ptr as *mut c_void);
            }
        }

        classes
    }

    /// `PluginFactory::createInstance<T>`: returns an owned reference.
    pub unsafe fn create_instance<T>(&self, class_id: &TUID, interface_iid: &TUID) -> Option<*mut T> {
        let mut obj: *mut c_void = std::ptr::null_mut();
        if (*self.raw).create_instance(class_id.as_ptr(), interface_iid.as_ptr(), &mut obj)
            == K_RESULT_TRUE
        {
            Some(obj as *mut T)
        } else {
            None
        }
    }
}

impl Drop for PluginFactory {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe {
                com_release(self.raw);
            }
            self.raw = std::ptr::null_mut();
        }
    }
}

// ---------------------------------------------------------------------------
// Module (Win32Module)
// ---------------------------------------------------------------------------

pub struct Module {
    pub factory: PluginFactory,
    hmodule: HMODULE,
}

fn wide(path: &std::path::Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain([0]).collect()
}

fn format_load_library_error(path: &std::path::Path) -> String {
    // Windows message text for GetLastError, mirroring
    // `Win32Module::getLastError`: either the formatted system message or,
    // when formatting fails, the plain error number.
    let last_error = unsafe { windows::Win32::Foundation::GetLastError() };
    let message = format_system_error(last_error.0);
    match message {
        Some(msg) => format!(
            "LoadLibraryW failed for path {}: {}",
            path.to_string_lossy(),
            msg.trim_end()
        ),
        None => format!(
            "LoadLibraryW failed with error number: {} for path {}",
            last_error.0,
            path.to_string_lossy()
        ),
    }
}

fn format_system_error(code: u32) -> Option<String> {
    let mut buffer = [0u16; 1024];
    // MAKELANGID(LANG_NEUTRAL, SUBLANG_DEFAULT)
    let language_id = 0x400u32;
    let len = unsafe {
        windows::Win32::System::Diagnostics::Debug::FormatMessageW(
            windows::Win32::System::Diagnostics::Debug::FORMAT_MESSAGE_FROM_SYSTEM
                | windows::Win32::System::Diagnostics::Debug::FORMAT_MESSAGE_IGNORE_INSERTS,
            None,
            code,
            language_id,
            windows::core::PWSTR(buffer.as_mut_ptr()),
            buffer.len() as u32,
            None,
        )
    };
    if len == 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&buffer[..len as usize]))
}

impl Module {
    /// `Module::create (path, errorDescription)` for Win32.
    pub fn create(path: &str) -> Result<Module, String> {
        let path_buf = PathBuf::from(path);

        let hmodule = if path_buf.is_dir() {
            // try as package (bundle)
            Self::load_as_package(&path_buf)?
        } else {
            // try old definition without package
            Self::load_as_dll(&path_buf)?
        };

        let get_factory = unsafe { GetProcAddress(hmodule, PCSTR(c"GetPluginFactory".as_ptr() as *const u8)) };
        let factory_proc: GetFactoryProc = match get_factory {
            Some(f) => unsafe { std::mem::transmute(f) },
            None => {
                unsafe {
                    let _ = FreeLibrary(hmodule);
                }
                return Err(
                    "The dll does not export the required 'GetPluginFactory' function".to_string(),
                );
            }
        };

        // InitDll is optional
        if let Some(init_ptr) =
            unsafe { GetProcAddress(hmodule, PCSTR(c"InitDll".as_ptr() as *const u8)) }
        {
            let init_dll: ModuleInitFunc = unsafe { std::mem::transmute(init_ptr) };
            if unsafe { init_dll() } == 0 {
                unsafe {
                    let _ = FreeLibrary(hmodule);
                }
                return Err("Calling 'InitDll' failed".to_string());
            }
        }

        let factory_ptr = unsafe { factory_proc() } as *mut IPluginFactoryObj;
        if factory_ptr.is_null() {
            unsafe {
                let _ = FreeLibrary(hmodule);
            }
            return Err("Calling 'GetPluginFactory' returned nullptr".to_string());
        }

        Ok(Module {
            factory: unsafe { PluginFactory::from_owned(factory_ptr) },
            hmodule,
        })
    }

    fn load_as_package(p: &std::path::Path) -> Result<HMODULE, String> {
        let filename = p.file_name().unwrap_or_default();
        let inner = p
            .join("Contents")
            .join(ARCHITECTURE_STRING)
            .join(filename);
        let inner_wide = wide(&inner);
        match unsafe { LoadLibraryW(PCWSTR(inner_wide.as_ptr())) } {
            Ok(instance) => Ok(instance),
            Err(_) => Err(format_load_library_error(&inner)),
        }
    }

    fn load_as_dll(p: &std::path::Path) -> Result<HMODULE, String> {
        let path_wide = wide(p);
        match unsafe { LoadLibraryW(PCWSTR(path_wide.as_ptr())) } {
            Ok(instance) => Ok(instance),
            Err(_) => Err(format_load_library_error(p)),
        }
    }
}

impl Drop for Module {
    fn drop(&mut self) {
        // ~Win32Module: factory reset first, then optional ExitDll, then
        // FreeLibrary.
        let factory = std::mem::replace(&mut self.factory, PluginFactory::null());
        drop(factory);

        if !self.hmodule.is_invalid() {
            if let Some(exit_ptr) =
                unsafe { GetProcAddress(self.hmodule, PCSTR(c"ExitDll".as_ptr() as *const u8)) }
            {
                let exit_dll: ModuleInitFunc = unsafe { std::mem::transmute(exit_ptr) };
                let _ = unsafe { exit_dll() };
            }
            unsafe {
                let _ = FreeLibrary(self.hmodule);
            }
        }
    }
}
