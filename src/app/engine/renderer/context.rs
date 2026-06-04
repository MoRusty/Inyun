use anyhow::Result;
use ash::ext::debug_utils::Instance as DebugUtils;
use ash::vk;
use std::ffi::{CStr, CString};
use std::os::raw::c_void;
use std::sync::Arc;
use tracing::span::Attributes;
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::Window;

// 3070 queue families, stick with 0 for now, use 1 and 2 late game optimisations
// 0	GRAPHICS + COMPUTE + TRANSFER + SPARSE for graphics
// 1	TRANSFER + SPARSE for pure transfer?
// 2	COMPUTE + TRANSFER + SPARSE for compute and presentation

#[derive(Debug)]
pub struct QueueFamily {
    pub index: u32,
    pub properties: vk::QueueFamilyProperties,
}

pub struct QueueFamilies{
    pub graphics: QueueFamily,
    pub present: QueueFamily,
    pub compute: QueueFamily,
    pub transfer: QueueFamily,
}

#[derive(Debug)]
pub struct PhysicalDevice {
    pub handle: vk::PhysicalDevice,
    pub properties: vk::PhysicalDeviceProperties,
    pub features: vk::PhysicalDeviceFeatures,
    pub memory_properties: vk::PhysicalDeviceMemoryProperties,
    pub queue_families: Vec<QueueFamily>,
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

//can use dynamic callback instead of static, research this topic more later,
// but for now just use a static callback function
type QueueFamilyPicker = fn(Vec<PhysicalDevice>) -> Result<(PhysicalDevice, QueueFamily)>;
struct ContextAttributes {
    window: Arc<Window>,
    queue_family_picker: QueueFamilyPicker,
}


pub struct Context {
    //store the instance and entry so they dont get dropped, we will need them for creating devices and surfaces
    // store them in the inverse order they were created so they get dropped in the correct order
    // the entry should be dropped after the instance, and the instance should be dropped after the surface (unsafe fn)
    pub surface: vk::SurfaceKHR,
    pub surface_extension: ash::khr::surface::Instance,
    pub instance: ash::Instance,
    pub entry: ash::Entry,
    // debug utils loader and messenger to receive validation/debug callbacks
    pub debug_utils_loader: DebugUtils,
    pub debug_messenger: vk::DebugUtilsMessengerEXT,
    pub attributes: ContextAttributes,
}


impl Context {
    pub fn new(attributes: ContextAttributes) -> Result<Self> {
        unsafe {
            let entry = ash::Entry::load()?;

            let raw_display_handle = attributes.window.display_handle()?.as_raw();
            let raw_window_handle = attributes.window.window_handle()?.as_raw();

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


            //dbg!(physical_devices);
            //todo - filter physical devices based on features and properties, for now just
            // take the first one and create a logical device from it,
            // but we will need to check for presentation support and other features later on
            let mut physical_devices = instance.enumerate_physical_devices()?.into_iter().map(|handle| {
                let properties = instance.get_physical_device_properties(handle);
                let features = instance.get_physical_device_features(handle);
                let memory_properties =
                    instance.get_physical_device_memory_properties(handle);

                let queue_family_properties =
                    instance.get_physical_device_queue_family_properties(handle);

                let queue_families = queue_family_properties
                    .into_iter()
                    .enumerate()
                    .map(|(index, properties)| QueueFamily {
                        index: index as u32,
                        properties, //same properties for all queue families, just different flags and counts
                    })
                    .collect::<Vec<_>>();

                PhysicalDevice {
                    handle,
                    properties,
                    features,
                    memory_properties,
                    queue_families,
                }
            }).collect::<Vec<_>>();

           // println!("{:#?}", physical_devices);
            //retain the devices that have surface support (can present to a surface, which is required for rendering to a window)
            physical_devices.retain(|physical_device| {
               surface_extension.get_physical_device_surface_support(physical_device.handle,0, surface).unwrap_or(false)
            });
            //println!("{:#?}", physical_devices);

            let (physical_device, queue_family) = (attributes.queue_family_picker)(physical_devices)?;



            Ok(Self {
                surface,
                surface_extension,
                instance,
                entry,
                debug_utils_loader,
                debug_messenger,
                attributes,
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
