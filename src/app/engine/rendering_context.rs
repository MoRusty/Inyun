use crate::app::engine::rendering_context;
use anyhow::Result;
use ash::ext::debug_utils::Instance as DebugUtils;
use ash::prelude::VkResult;
use ash::vk;
use ash::vk::{ImageView, SwapchainKHR};
use std::collections::HashSet;
use std::ffi::{CStr, CString};
use std::os::raw::c_void;
use std::sync::Arc;
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::Window;
// 3070 queue families, stick with 0 for now, use 1 and 2 late game optimisations
// 0	GRAPHICS + COMPUTE + TRANSFER + SPARSE for graphics
// 1	TRANSFER + SPARSE for pure transfer?
// 2	COMPUTE + TRANSFER + SPARSE for compute and presentation

#[derive(Debug, Clone)]
pub struct QueueFamily {
    pub index: u32,
    pub properties: vk::QueueFamilyProperties,
}

#[derive(Clone)]
pub struct QueueFamilies {
    pub graphics: QueueFamily,
    pub present: QueueFamily,
    pub compute: QueueFamily,
    pub transfer: QueueFamily,
}

#[derive(Debug, Clone)]
pub struct PhysicalDevice {
    pub handle: vk::PhysicalDevice,
    pub properties: vk::PhysicalDeviceProperties,
    pub features: vk::PhysicalDeviceFeatures,
    pub memory_properties: vk::PhysicalDeviceMemoryProperties,
    pub queue_families: Vec<QueueFamily>,
}

pub struct Surface {
    pub handle: vk::SurfaceKHR,
    pub capabilities: vk::SurfaceCapabilitiesKHR,
    pub formats: Vec<vk::SurfaceFormatKHR>,
    pub present_modes: Vec<vk::PresentModeKHR>,
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
type QueueFamilyPicker = fn(Vec<PhysicalDevice>) -> Result<(PhysicalDevice, QueueFamilies)>;
pub struct RenderingContextAttributes {
    //temp used for compatibility check of physical devices
    pub window: Arc<Window>,
    pub queue_family_picker: QueueFamilyPicker,
}

pub struct RenderingContext {
    //store the instance and entry so they don't get dropped, we will need them for creating devices and surfaces
    // store them in the inverse order they were created so they get dropped in the correct order
    // the entry should be dropped after the instance, and the instance should be dropped after the surface (unsafe fn)
    pub queues: Vec<vk::Queue>,
    pub device: ash::Device,
    pub swapchain_extension: ash::khr::swapchain::Device,
    pub queue_family_indices: HashSet<u32>,
    pub queue_families: QueueFamilies,
    pub physical_device: PhysicalDevice,
    pub surface_extension: ash::khr::surface::Instance,
    pub instance: ash::Instance,
    pub entry: ash::Entry,
    // debug utils loader and messenger to receive validation/debug callbacks
    pub debug_utils_loader: DebugUtils,
    pub debug_messenger: vk::DebugUtilsMessengerEXT,
    pub attributes: RenderingContextAttributes,
}

impl RenderingContext {
    pub fn new(attributes: RenderingContextAttributes) -> Result<Self> {
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

            //above loads the Vulkan library and instance but not any extensions,
            // we will need to load the surface extension and create a surface for the window
            let surface_extension = ash::khr::surface::Instance::new(&entry, &instance);
            let dummy_surface = ash_window::create_surface(
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
            let mut physical_devices = instance
                .enumerate_physical_devices()?
                .into_iter()
                .map(|handle| {
                    let properties = instance.get_physical_device_properties(handle);
                    let features = instance.get_physical_device_features(handle);
                    let memory_properties = instance.get_physical_device_memory_properties(handle);

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
                })
                .collect::<Vec<_>>();

            // println!("{:#?}", physical_devices);
            //retain the devices that have surface support (can present to a surface, which is required for rendering to a window)
            physical_devices.retain(|physical_device| {
                surface_extension
                    .get_physical_device_surface_support(physical_device.handle, 0, dummy_surface)
                    .unwrap_or(false)
            });
            //println!("{:#?}", physical_devices);

            //dummy surface to get device compatibility
            surface_extension.destroy_surface(dummy_surface, None);

            let (physical_device, queue_families) =
                (attributes.queue_family_picker)(physical_devices)?;

            //note I'm not using HashSet<u32,S>
            let queue_family_indices: HashSet<u32> = [
                queue_families.graphics.index,
                queue_families.present.index,
                queue_families.compute.index,
                queue_families.transfer.index,
            ]
            .into_iter()
            .collect();

            let queue_create_infos = queue_family_indices
                .iter()
                .copied()
                .map(|index| {
                    vk::DeviceQueueCreateInfo::default()
                        .queue_family_index(index)
                        .queue_priorities(&[1.0]) // single queue with highest priority
                })
                .collect::<Vec<_>>();

            //using dynamic rendering which is now a core feature in Vulkan 1.3 -
            // todo - research the benefits of dynamic rendering vs traditional render passes and framebuffers, and decide if we want to use it for our renderer,
            // but for now just enable the feature and use it in the device create info
            let device = instance.create_device(
                physical_device.handle,
                &vk::DeviceCreateInfo::default()
                    .queue_create_infos(&queue_create_infos)
                    .enabled_extension_names(&[ash::khr::swapchain::NAME.as_ptr()])
                    .push_next(
                        &mut vk::PhysicalDeviceDynamicRenderingFeatures::default()
                            .dynamic_rendering(true),
                    )
                    .push_next(
                        &mut vk::PhysicalDeviceBufferDeviceAddressFeatures::default()
                            .buffer_device_address(true),
                    ),
                None,
            )?;

            let swapchain_extension = ash::khr::swapchain::Device::new(&instance, &device);

            let queues = queue_family_indices
                .iter()
                .copied()
                .map(|index| {
                    device.get_device_queue(index, 0) // get the first queue from each family
                })
                .collect::<Vec<_>>();

            Ok(Self {
                queues,
                device,
                swapchain_extension,
                queue_family_indices,
                queue_families,
                physical_device,
                surface_extension,
                instance,
                entry,
                debug_utils_loader,
                debug_messenger,
                attributes,
            })
        }
    }
    pub fn pick_discrete_gpu(
        physical_devices: Vec<PhysicalDevice>,
    ) -> Result<(PhysicalDevice, QueueFamilies)> {
        // Try to find a discrete GPU first
        let best_device = physical_devices
            .iter() //
            .find(|d| d.properties.device_type == vk::PhysicalDeviceType::DISCRETE_GPU)
            .or_else(|| physical_devices.first()) //
            .ok_or_else(|| anyhow::anyhow!("No physical devices found"))?
            .clone();

        // Helpful for debugging when running elsewhere
        log::debug!("Selected device: {:?}", best_device.properties.device_type);

        // Just use the first queue family that supports graphics
        let graphics_family = best_device
            .queue_families
            .iter()
            .find(|qf| qf.properties.queue_flags.contains(vk::QueueFlags::GRAPHICS))
            .ok_or_else(|| anyhow::anyhow!("No graphics queue family"))?
            .clone();

        // Use the same queue family for everything (simple but works)
        //todo change it so that the best queue family is used for each type of queue (graphics, present, transfer, compute)
        Ok((
            best_device,
            rendering_context::QueueFamilies {
                graphics: graphics_family.clone(),
                present: graphics_family.clone(),
                transfer: graphics_family.clone(),
                compute: graphics_family.clone(),
            },
        ))
    }

    //get all surface info and store it in struct, saves having the individual functions
    pub unsafe fn create_surface(&self, window: &Window) -> Result<Surface> {
        let raw_display_handle = window.display_handle()?.as_raw();
        let raw_window_handle = window.window_handle()?.as_raw();

        let handle = ash_window::create_surface(
            &self.entry,
            &self.instance,
            raw_display_handle,
            raw_window_handle,
            None,
        )?;

        let capabilities = self
            .surface_extension
            .get_physical_device_surface_capabilities(self.physical_device.handle, handle)?;

        let formats = self
            .surface_extension
            .get_physical_device_surface_formats(self.physical_device.handle, handle)?;

        let present_modes = self
            .surface_extension
            .get_physical_device_surface_present_modes(self.physical_device.handle, handle)?;

        Ok(Surface {
            handle,
            capabilities,
            formats,
            present_modes,
        })
    }

    //todo come back to this
    pub fn create_image_view(
        &self,
        image: vk::Image,
        format: vk::Format,
        aspect_mask: vk::ImageAspectFlags,
    ) -> Result<vk::ImageView> {
        let image_view = unsafe {
            self.device.create_image_view(
                &vk::ImageViewCreateInfo::default()
                    .image(image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(format)
                    .subresource_range(
                        vk::ImageSubresourceRange::default()
                            .aspect_mask(aspect_mask)
                            .base_mip_level(0)
                            .level_count(1)
                            .base_array_layer(0)
                            .layer_count(1),
                    ),
                None,
            )
        }?;
        Ok(image_view)
    }
}

impl Drop for RenderingContext {
    fn drop(&mut self) {
        unsafe {
            // destroy messenger before destroying instance
            self.debug_utils_loader
                .destroy_debug_utils_messenger(self.debug_messenger, None);
            //destroy device before surface and instance
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}
