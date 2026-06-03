use anyhow::Result;
use ash::ext::debug_utils::Instance as DebugUtils;
use ash::vk;
use std::ffi::{CStr, CString};
use std::os::raw::c_void;
use std::sync::Arc;
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::Window;

pub struct Context {
    //store the instance and entry so they dont get dropped, we will need them for creating devices and surfaces
    // store them in the inverse order they were created so they get dropped in the correct order
    // the entry should be dropped after the instance, and the instance should be dropped after the surface (unsafe fn)
    surface: vk::SurfaceKHR,
    surface_extension: ash::khr::surface::Instance,
    instance: ash::Instance,
    entry: ash::Entry,
    // debug utils loader and messenger to receive validation/debug callbacks
    debug_utils_loader: DebugUtils,
    debug_messenger: vk::DebugUtilsMessengerEXT,
    window: Arc<Window>,
}

unsafe extern "system" fn vulkan_debug_callback(
    message_severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    message_types: vk::DebugUtilsMessageTypeFlagsEXT,
    p_callback_data: *const vk::DebugUtilsMessengerCallbackDataEXT,
    _p_user_data: *mut c_void,
) -> vk::Bool32 {
    if !p_callback_data.is_null() {
        let callback_data = unsafe { &*p_callback_data };
        let message = unsafe {
            if callback_data.p_message.is_null() {
                CStr::from_bytes_with_nul(b"<no message>\0").unwrap()
            } else {
                CStr::from_ptr(callback_data.p_message)
            }
        };
        eprintln!(
            "[VULKAN] {:?} {:?}: {}",
            message_severity,
            message_types,
            message.to_string_lossy()
        );
    }
    vk::FALSE
}

impl Context {
    pub fn new(window: Arc<Window>) -> Result<Self> {
        unsafe {
            let entry = ash::Entry::load()?;

            let raw_display_handle = window.display_handle()?.as_raw();
            let raw_window_handle = window.window_handle()?.as_raw();

            //enabling validation layers
            let validation_layer_names = [CString::new("VK_LAYER_KHRONOS_validation")?];
            let enabled_layer_names = validation_layer_names
                .iter()
                .map(|name| name.as_ptr())
                .collect::<Vec<_>>();

            // required extensions for window surface + debug utils
            let mut extension_names =
                ash_window::enumerate_required_extensions(raw_display_handle)?.to_vec();
            // push the debug utils extension name (static CStr from ash)
            extension_names.push(vk::EXT_DEBUG_UTILS_NAME.as_ptr());

            // Debug messenger create info (we'll create the messenger after instance creation)
            let debug_create_info = vk::DebugUtilsMessengerCreateInfoEXT::default()
                .message_severity(
                    vk::DebugUtilsMessageSeverityFlagsEXT::WARNING
                        | vk::DebugUtilsMessageSeverityFlagsEXT::ERROR
                        | vk::DebugUtilsMessageSeverityFlagsEXT::INFO,
                )
                .message_type(
                    vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                        | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                        | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
                )
                .pfn_user_callback(Some(vulkan_debug_callback));

            let instance = entry.create_instance(
                &vk::InstanceCreateInfo::default()
                    .application_info(
                        &vk::ApplicationInfo::default().api_version(vk::API_VERSION_1_3),
                    )
                    .enabled_extension_names(&extension_names)
                    .enabled_layer_names(&enabled_layer_names),
                None,
            )?;

            // create DebugUtils loader and the messenger now that instance exists
            let debug_utils_loader = DebugUtils::new(&entry, &instance);
            let debug_messenger =
                debug_utils_loader.create_debug_utils_messenger(&debug_create_info, None)?;

            //above loads the vulkan library and instance but not any extensions,
            // we will need to load the surface extension and create a surface for the window
            let surface_extension = ash::khr::surface::Instance::new(&entry, &instance);
            let surface = ash_window::create_surface(
                &entry,
                &instance,
                raw_display_handle,
                raw_window_handle,
                None,
            )?;

            let physical_devices = instance.enumerate_physical_devices()?;

            dbg!(physical_devices);

            Ok(Self {
                surface,
                surface_extension,
                instance,
                entry,
                debug_utils_loader,
                debug_messenger,
                window,
            })
        }
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        unsafe {
            // destroy messenger before destroying instance
            self.debug_utils_loader
                .destroy_debug_utils_messenger(self.debug_messenger, None);

            self.surface_extension.destroy_surface(self.surface, None);
            self.instance.destroy_instance(None);
        }
    }
}
