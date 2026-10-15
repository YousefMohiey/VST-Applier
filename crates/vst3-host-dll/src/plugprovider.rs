//! `PlugProvider` / `ConnectionProxy` / `PluginContextFactory`, ported 1:1
//! from `public.sdk/source/vst/hosting/plugprovider.{h,cpp}` and
//! `connectionproxy.{h,cpp}` (SDK 3.7.12).
#![allow(non_snake_case, dead_code)]
#![allow(clippy::missing_safety_doc)]

use std::ffi::c_void;
use std::sync::atomic::{AtomicPtr, Ordering};

use crate::module::{ClassInfo, PluginFactory};
use crate::objects::ConnectionProxyObj;
use crate::vst3abi::*;

// ---------------------------------------------------------------------------
// IPluginBase access
// ---------------------------------------------------------------------------

/// `IPluginBase::initialize` — slots 0..2 are FUnknown, slot 3 is
/// `initialize` for every interface derived from `IPluginBase`
/// (IComponent, IEditController, ...), so this works for any of them.
unsafe fn plugin_base_initialize(p: *mut c_void, context: *mut c_void) -> i32 {
    let vtbl = *(p as *mut *const IPluginBaseVtbl);
    ((*vtbl).initialize)(p, context)
}

/// `IPluginBase::terminate`.
unsafe fn plugin_base_terminate(p: *mut c_void) -> i32 {
    let vtbl = *(p as *mut *const IPluginBaseVtbl);
    ((*vtbl).terminate)(p)
}

// ---------------------------------------------------------------------------
// PluginContextFactory
// ---------------------------------------------------------------------------

/// Mirrors `PluginContextFactory::instance ()` — a process-wide singleton
/// holding the FUnknown* context passed to `IPluginBase::initialize`.
pub struct PluginContextFactory;

static PLUGIN_CONTEXT: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

impl PluginContextFactory {
    pub fn instance() -> &'static PluginContextFactory {
        &PluginContextFactory
    }

    pub fn set_plugin_context(&self, obj: *mut c_void) {
        PLUGIN_CONTEXT.store(obj, Ordering::SeqCst);
    }

    pub fn plugin_context(&self) -> *mut c_void {
        PLUGIN_CONTEXT.load(Ordering::SeqCst)
    }
}

// ---------------------------------------------------------------------------
// PlugProvider
// ---------------------------------------------------------------------------

pub struct PlugProvider {
    factory: PluginFactory,
    component: Option<InterfacePtr<IComponentObj>>,
    controller: Option<InterfacePtr<IEditControllerObj>>,
    class_info: ClassInfo,
    component_cp: Option<*mut ConnectionProxyObj>,
    controller_cp: Option<*mut ConnectionProxyObj>,
    plug_is_global: bool,
}

/// Mirrors `PlugProvider::printError` (writes diagnostics to std::cout).
fn print_error(message: String) {
    println!("{message}");
}

impl PlugProvider {
    pub fn new(factory: &PluginFactory, class_info: ClassInfo, plug_is_global: bool) -> Self {
        Self {
            factory: unsafe { factory.clone_ptr() },
            component: None,
            controller: None,
            class_info,
            component_cp: None,
            controller_cp: None,
            plug_is_global,
        }
    }

    pub fn initialize(&mut self) -> bool {
        if self.plug_is_global {
            let context = PluginContextFactory::instance().plugin_context();
            return self.setup_plugin(context);
        }
        true
    }

    /// `IPtr<IComponent> getComponentPtr () const` — addRef'd copy.
    pub fn get_component_ptr(&self) -> Option<InterfacePtr<IComponentObj>> {
        match &self.component {
            Some(c) if !c.is_null() => unsafe { Some(c.cloned()) },
            _ => None,
        }
    }

    /// `IPtr<IEditController> getControllerPtr () const` — addRef'd copy.
    pub fn get_controller_ptr(&self) -> Option<InterfacePtr<IEditControllerObj>> {
        match &self.controller {
            Some(c) if !c.is_null() => unsafe { Some(c.cloned()) },
            _ => None,
        }
    }

    /// `setupPlugin (FUnknown* hostContext)`.
    fn setup_plugin(&mut self, host_context: *mut c_void) -> bool {
        let mut res = false;
        let mut is_single_component = false;

        let component_raw: Option<*mut IComponentObj> = unsafe {
            self.factory
                .create_instance::<IComponentObj>(&self.class_info.class_id, &iid::I_COMPONENT)
        };

        if let Some(component_ptr) = component_raw {
            self.component = Some(unsafe { InterfacePtr::from_owned(component_ptr) });

            // if (auto plugBase = U::cast<IPluginBase> (component))
            let plug_base = unsafe { com_query_interface(component_ptr, &iid::I_PLUGIN_BASE) };
            match plug_base {
                Some(base_ptr) => {
                    res = unsafe { plugin_base_initialize(base_ptr, host_context) } == K_RESULT_OK;
                    unsafe {
                        com_release(base_ptr as *mut c_void);
                    }
                    if !res {
                        print_error(format!(
                            "Failed to initialize component of {}!",
                            self.class_info.name
                        ));
                        return false;
                    }
                }
                None => {
                    print_error(format!(
                        "Failed to get IPluginBase from component of {}!",
                        self.class_info.name
                    ));
                    return false;
                }
            }

            // try to create the controller part from the component
            if let Some(controller_ptr) = unsafe {
                com_query_interface(component_ptr, &iid::I_EDIT_CONTROLLER)
            } {
                self.controller =
                    Some(unsafe { InterfacePtr::from_owned(controller_ptr as *mut IEditControllerObj) });
                is_single_component = true;
            } else {
                let mut controller_cid: TUID = [0; 16];
                let got_cid = unsafe { (*component_ptr).get_controller_class_id(controller_cid.as_mut_ptr()) };
                if got_cid == K_RESULT_TRUE {
                    let controller_raw = unsafe {
                        self.factory.create_instance::<IEditControllerObj>(
                            &controller_cid,
                            &iid::I_EDIT_CONTROLLER,
                        )
                    };
                    if let Some(ctrl_ptr) = controller_raw {
                        self.controller = Some(unsafe { InterfacePtr::from_owned(ctrl_ptr) });

                        let ctrl_base =
                            unsafe { com_query_interface(ctrl_ptr, &iid::I_PLUGIN_BASE) };
                        match ctrl_base {
                            Some(base_ptr) => {
                                res = unsafe { plugin_base_initialize(base_ptr, host_context) }
                                    == K_RESULT_OK;
                                unsafe {
                                    com_release(base_ptr as *mut c_void);
                                }
                                if !res {
                                    print_error(format!(
                                        "Failed to initialize controller of {}!",
                                        self.class_info.name
                                    ));
                                }
                            }
                            None => {
                                print_error(format!(
                                    "Failed to get IPluginBase from controller of {}!",
                                    self.class_info.name
                                ));
                                return false;
                            }
                        }
                    }
                } else {
                    print_error(format!(
                        "Component does not provide a required controller class ID [{}]!",
                        self.class_info.name
                    ));
                }
            }

            if !res {
                self.component = None;
                self.controller = None;
            }
        } else {
            print_error(format!(
                "Failed to create component instance of {}!",
                self.class_info.name
            ));
        }

        if res && !is_single_component {
            return self.connect_components();
        }

        res
    }

    /// `connectComponents ()`.
    fn connect_components(&mut self) -> bool {
        let (Some(component), Some(controller)) = (&self.component, &self.controller) else {
            return false;
        };

        let comp_icp = unsafe { com_query_interface(component.as_ptr(), &iid::I_CONNECTION_POINT) };
        let contr_icp = unsafe { com_query_interface(controller.as_ptr(), &iid::I_CONNECTION_POINT) };
        let (Some(comp_icp), Some(contr_icp)) = (comp_icp, contr_icp) else {
            return false;
        };

        // componentCP = owned (new ConnectionProxy (compICP));
        // controllerCP = owned (new ConnectionProxy (contrICP));
        // (assigned to the members BEFORE connecting, as in the C++)
        self.component_cp = Some(ConnectionProxyObj::create(comp_icp));
        self.controller_cp = Some(ConnectionProxyObj::create(contr_icp));

        // Drop the local U::cast references; the proxies now share them.
        unsafe {
            com_release(comp_icp as *mut c_void);
            com_release(contr_icp as *mut c_void);
        }

        let component_cp = self.component_cp.unwrap();
        let controller_cp = self.controller_cp.unwrap();

        let tres = unsafe { (*component_cp).connect_external(contr_icp) };
        if tres != K_RESULT_TRUE {
            print_error(format!(
                "Failed to connect the component with the controller with result code '{tres}'!"
            ));
            return false;
        }
        let tres = unsafe { (*controller_cp).connect_external(comp_icp) };
        if tres != K_RESULT_TRUE {
            print_error(format!(
                "Failed to connect the controller with the component with result code '{tres}'!"
            ));
            return false;
        }
        true
    }

    /// `disconnectComponents ()`.
    fn disconnect_components(&mut self) -> bool {
        let (Some(component_cp), Some(controller_cp)) = (self.component_cp, self.controller_cp)
        else {
            return false;
        };

        let res = unsafe { (*component_cp).disconnect_current() }
            & unsafe { (*controller_cp).disconnect_current() };

        unsafe {
            com_release(component_cp as *mut c_void);
            com_release(controller_cp as *mut c_void);
        }
        self.component_cp = None;
        self.controller_cp = None;

        res
    }

    /// `terminatePlugin ()`.
    fn terminate_plugin(&mut self) {
        self.disconnect_components();

        let mut controller_is_component = false;
        if let Some(component) = &self.component {
            if let Some(qi) =
                unsafe { com_query_interface(component.as_ptr(), &iid::I_EDIT_CONTROLLER) }
            {
                controller_is_component = true;
                unsafe {
                    com_release(qi as *mut c_void);
                }
            }

            let plug_base =
                unsafe { com_query_interface(component.as_ptr(), &iid::I_PLUGIN_BASE) };
            match plug_base {
                Some(base_ptr) => {
                    let _ = unsafe { plugin_base_terminate(base_ptr) };
                    unsafe {
                        com_release(base_ptr as *mut c_void);
                    }
                }
                None => {
                    print_error(format!(
                        "Failed to get IPluginBase from component of {}!",
                        self.class_info.name
                    ));
                }
            }
        }

        if let Some(controller) = &self.controller {
            if !controller_is_component {
                let plug_base =
                    unsafe { com_query_interface(controller.as_ptr(), &iid::I_PLUGIN_BASE) };
                match plug_base {
                    Some(base_ptr) => {
                        let _ = unsafe { plugin_base_terminate(base_ptr) };
                        unsafe {
                            com_release(base_ptr as *mut c_void);
                        }
                    }
                    None => {
                        print_error(format!(
                            "Failed to get IPluginBase from controller of {}!",
                            self.class_info.name
                        ));
                    }
                }
            }
        }

        self.component = None;
        self.controller = None;
    }
}

impl Drop for PlugProvider {
    fn drop(&mut self) {
        // ~PlugProvider -> terminatePlugin ().
        self.terminate_plugin();
    }
}
