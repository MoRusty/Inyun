use anyhow::Result;
use ash::ext::debug_utils::Instance as DebugUtils;
use ash::vk;
use cstr::cstr;
use gpu_allocator::vulkan::{
    Allocation, AllocationCreateDesc, AllocationScheme, Allocator, AllocatorCreateDesc,
};
use gpu_allocator::{AllocationSizes, AllocatorDebugSettings, MemoryLocation};
use std::cmp::PartialEq;
use std::collections::HashSet;
use std::ffi::{CStr, CString};
use std::io;
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

#[derive(Clone, Copy)]
pub struct ImageLayoutState {
    pub access_mask: vk::AccessFlags2,
    pub layout: vk::ImageLayout,
    pub stage_mask: vk::PipelineStageFlags2,
    pub queue_family: u32,
}

impl Default for ImageLayoutState {
    fn default() -> Self {
        Self {
            access_mask: vk::AccessFlags2::NONE,
            layout: vk::ImageLayout::UNDEFINED,
            stage_mask: vk::PipelineStageFlags2::ALL_COMMANDS,
            queue_family: vk::QUEUE_FAMILY_IGNORED,
        }
    }
}

pub struct Image {
    pub handle: vk::Image,
    pub allocation: Option<Allocation>,
    pub view: vk::ImageView,
    pub layout: ImageLayoutState,
    pub attributes: ImageAttributes,
}

pub struct ImageAttributes {
    pub location: MemoryLocation,
    pub linear: bool,
    pub allocation_scheme: AllocationScheme,
    pub extent: vk::Extent3D,
    pub format: vk::Format,
    pub usage: vk::ImageUsageFlags,
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
                c"<no message>"
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

impl PartialEq<vk::ImageLayout> for ImageLayoutState {
    fn eq(&self, other: &vk::ImageLayout) -> bool {
        self.layout == *other
    }
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

            //todo: look into the other features in vulkan 1.2 and .3
            let device = instance.create_device(
                physical_device.handle,
                &vk::DeviceCreateInfo::default()
                    .queue_create_infos(&queue_create_infos)
                    .enabled_extension_names(&[ash::khr::swapchain::NAME.as_ptr()])
                    .push_next(
                        &mut vk::PhysicalDeviceVulkan12Features::default()
                            .buffer_device_address(true)
                            .descriptor_indexing(true),
                    )
                    .push_next(
                        &mut vk::PhysicalDeviceVulkan13Features::default()
                            .dynamic_rendering(true)
                            .synchronization2(true),
                    ),
                None,
            )?;

            let swapchain_extension = ash::khr::swapchain::Device::new(&instance, &device);

            let queues = queue_family_indices
                .iter()
                .copied()
                .map(|index| {
                    device.get_device_queue2(
                        &vk::DeviceQueueInfo2::default().queue_family_index(index),
                    ) // get the first queue from each family
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
            QueueFamilies {
                graphics: graphics_family.clone(),
                present: graphics_family.clone(),
                transfer: graphics_family.clone(),
                compute: graphics_family.clone(),
            },
        ))
    }

    //get all surface info and store it in struct, saves having the individual functions
    pub fn create_surface(&self, window: &Window) -> Result<Surface> {
        let raw_display_handle = window.display_handle()?.as_raw();
        let raw_window_handle = window.window_handle()?.as_raw();

        unsafe {
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

    pub fn create_shader_module(&self, code: &[u8]) -> Result<vk::ShaderModule> {
        let mut code = io::Cursor::new(code);
        let code = ash::util::read_spv(&mut code)?;
        let create_info = vk::ShaderModuleCreateInfo::default().code(&code);
        let shader_module = unsafe { self.device.create_shader_module(&create_info, None) }?;
        Ok(shader_module)
    }

    pub fn create_graphics_pipeline(
        &self,
        vertex: vk::ShaderModule,
        fragment: vk::ShaderModule,
        swapchain_format: vk::Format,
        pipeline_layout: vk::PipelineLayout,
        pipeline_cache: vk::PipelineCache,
    ) -> Result<vk::Pipeline> {
        //todo - research graphics pipeline creation and how to make it more efficient, currently just creating a basic pipeline with hardcoded values for everything except the shaders and pipeline layout
        let vertex_stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(vertex)
            .name(cstr!("main"));

        let fragment_stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(fragment)
            .name(cstr!("main"));

        let stages = [vertex_stage, fragment_stage];

        let vertex_input_state = vk::PipelineVertexInputStateCreateInfo::default();

        let input_assembly_state = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST);

        // using dynamic viewport and scissor states, so we don't need to specify them here, but we still need to specify the counts
        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);

        let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];

        let dynamic_state_info =
            vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);

        let rasterization_state = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::CLOCKWISE)
            .line_width(1.0);

        let multisample_state = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);

        let color_blend_attachment = vk::PipelineColorBlendAttachmentState {
            blend_enable: vk::FALSE,
            src_color_blend_factor: vk::BlendFactor::ONE,
            dst_color_blend_factor: vk::BlendFactor::ZERO,
            color_blend_op: vk::BlendOp::ADD,
            src_alpha_blend_factor: vk::BlendFactor::ONE,
            dst_alpha_blend_factor: vk::BlendFactor::ZERO,
            alpha_blend_op: vk::BlendOp::ADD,
            color_write_mask: vk::ColorComponentFlags::RGBA,
        };

        let binding = [color_blend_attachment];
        let color_blend_state = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .logic_op(vk::LogicOp::COPY)
            .attachments(&binding);

        let color_attachment_formats = [swapchain_format];

        //dynamic
        let mut rendering_info = vk::PipelineRenderingCreateInfo::default()
            .color_attachment_formats(&color_attachment_formats);

        let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vertex_input_state)
            .input_assembly_state(&input_assembly_state)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterization_state)
            .multisample_state(&multisample_state)
            .color_blend_state(&color_blend_state)
            .layout(pipeline_layout)
            .render_pass(vk::RenderPass::null()) // using dynamic rendering, so no render pass
            .dynamic_state(&dynamic_state_info)
            .push_next(&mut rendering_info);

        let pipeline = unsafe {
            self.device
                .create_graphics_pipelines(pipeline_cache, &[pipeline_info], None)
        }
        .map_err(|(_, err)| err)?
        .into_iter()
        .next()
        .unwrap();
        Ok(pipeline)
    }

    pub fn transition_image_layout(
        &self,
        command_buffer: vk::CommandBuffer,
        image: vk::Image, //todo: change this to Image struct rather than vk
        old_layout: ImageLayoutState,
        new_layout: ImageLayoutState,
    ) {
        unsafe {
            let aspect_mask = if new_layout == vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL {
                vk::ImageAspectFlags::DEPTH | vk::ImageAspectFlags::STENCIL
            } else {
                vk::ImageAspectFlags::COLOR
            };

            //use cmd_pipeline_barrier2 instead
            self.device.cmd_pipeline_barrier2(
                command_buffer,
                &vk::DependencyInfo::default().image_memory_barriers(&[
                    vk::ImageMemoryBarrier2::default()
                        .src_stage_mask(old_layout.stage_mask)
                        .dst_stage_mask(new_layout.stage_mask)
                        .src_access_mask(old_layout.access_mask)
                        .dst_access_mask(new_layout.access_mask)
                        .old_layout(old_layout.layout)
                        .new_layout(new_layout.layout)
                        .src_queue_family_index(old_layout.queue_family)
                        .dst_queue_family_index(new_layout.queue_family)
                        .image(image)
                        .subresource_range(
                            vk::ImageSubresourceRange::default()
                                .aspect_mask(aspect_mask)
                                .base_mip_level(0)
                                .level_count(1)
                                .base_array_layer(0)
                                .layer_count(1),
                        ),
                ]),
            );
        }
    }

    pub fn begin_rendering(
        &self,
        command_buffer: vk::CommandBuffer,
        image_view: vk::ImageView,
        clear_color: vk::ClearColorValue,
        render_area: vk::Rect2D,
    ) {
        unsafe {
            self.device.cmd_begin_rendering(
                command_buffer,
                &vk::RenderingInfo::default()
                    .layer_count(1)
                    .color_attachments(&[vk::RenderingAttachmentInfo::default()
                        .image_view(image_view)
                        .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                        .clear_value(vk::ClearValue { color: clear_color })
                        .load_op(vk::AttachmentLoadOp::CLEAR)
                        .store_op(vk::AttachmentStoreOp::STORE)])
                    .render_area(render_area),
            );
        }
    }

    pub fn create_allocator(
        &self,
        debug_settings: AllocatorDebugSettings,
        allocation_sizes: AllocationSizes,
    ) -> Result<Allocator> {
        Ok(Allocator::new(&AllocatorCreateDesc {
            instance: self.instance.clone(),
            device: self.device.clone(),
            physical_device: self.physical_device.handle,
            debug_settings,
            buffer_device_address: true, // Ideally, check the BufferDeviceAddressFeatures struct.
            allocation_sizes,
        })?)
    }

    pub fn create_image(
        //move this to a struct except of name and allocator?
        &self,
        name: &str,
        allocator: &mut Allocator,
        attributes: ImageAttributes,
    ) -> Result<Image> {
        let image_create_info = unsafe {
            self.device.create_image(
                &vk::ImageCreateInfo::default()
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(attributes.format)
                    .extent(attributes.extent)
                    .mip_levels(1)
                    .array_layers(1)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .initial_layout(vk::ImageLayout::UNDEFINED)
                    .tiling(if attributes.linear {
                        vk::ImageTiling::LINEAR
                    } else {
                        vk::ImageTiling::OPTIMAL
                    })
                    .usage(attributes.usage)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )
        }?;

        let requirements = unsafe { self.device.get_image_memory_requirements(image_create_info) };

        let allocation = allocator.allocate(&AllocationCreateDesc {
            name,
            requirements,
            location: attributes.location,
            linear: attributes.linear,
            allocation_scheme: attributes.allocation_scheme,
        })?;

        unsafe {
            self.device.bind_image_memory(
                image_create_info,
                allocation.memory(),
                allocation.offset(),
            )?;
        }

        let view = self.create_image_view(
            image_create_info,
            attributes.format,
            vk::ImageAspectFlags::COLOR,
        )?;

        Ok(Image {
            handle: image_create_info,
            allocation: Some(allocation), //changed to an Option to allow memory aliasing in the future
            view,
            layout: ImageLayoutState {
                access_mask: vk::AccessFlags2::NONE,
                layout: vk::ImageLayout::UNDEFINED,
                stage_mask: vk::PipelineStageFlags2::NONE,
                queue_family: 0,
            },
            attributes,
        })
    }

    pub fn destroy_image(&self, image: &mut Image, allocator: &mut Allocator) -> Result<()> {
        unsafe {
            self.device.destroy_image_view(image.view, None);
            if let Some(allocation) = image.allocation.take() {
                allocator.free(allocation)?;
            }
            self.device.destroy_image(image.handle, None);
        }
        Ok(())
    }

    //not in use but might come handy later
    #[allow(dead_code)]
    pub fn clear_image(
        &self,
        command_buffer: vk::CommandBuffer,
        image: Image,
        clear_color: [f32; 4],
        layout: Option<vk::ImageLayout>, //Optional layout
    ) {
        let image_layout = layout.unwrap_or(vk::ImageLayout::GENERAL);

        unsafe {
            self.device.cmd_clear_color_image(
                command_buffer,
                image.handle,
                image_layout, // Use the provided layout
                &vk::ClearColorValue {
                    float32: clear_color,
                },
                &[vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(0)
                    .level_count(1)
                    .base_array_layer(0)
                    .layer_count(1)],
            )
        }
    }

    pub fn blit_image(
        //todo: when i change things to use the Image struct, i will need to change this fn to do the same
        &self,
        command_buffer: vk::CommandBuffer,
        src_image: vk::Image,
        src_layout: vk::ImageLayout,
        dst_image: vk::Image,
        dst_layout: vk::ImageLayout,
        extent: vk::Extent3D,
    ) {
        unsafe {
            // self.device.cmd_copy_image(
            //     command_buffer,
            //     src_image,
            //     src_layout,
            //     dst_image,
            //     dst_layout,
            //     &[vk::ImageCopy::default()
            //         .src_subresource(
            //             vk::ImageSubresourceLayers::default()
            //                 .aspect_mask(vk::ImageAspectFlags::COLOR)
            //                 .layer_count(1),
            //         )
            //         .dst_subresource(
            //             vk::ImageSubresourceLayers::default()
            //                 .aspect_mask(vk::ImageAspectFlags::COLOR)
            //                 .layer_count(1),
            //         )
            //         .extent(extent)],
            // )
            self.device.cmd_blit_image2(
                command_buffer,
                &vk::BlitImageInfo2::default()
                    .src_image(src_image)
                    .src_image_layout(src_layout)
                    .dst_image(dst_image)
                    .dst_image_layout(dst_layout)
                    .regions(&[vk::ImageBlit2::default()
                        .src_subresource(
                            vk::ImageSubresourceLayers::default()
                                .aspect_mask(vk::ImageAspectFlags::COLOR)
                                .layer_count(1),
                        )
                        .dst_subresource(
                            vk::ImageSubresourceLayers::default()
                                .aspect_mask(vk::ImageAspectFlags::COLOR)
                                .layer_count(1),
                        )
                        .src_offsets([
                            vk::Offset3D { x: 0, y: 0, z: 0 },
                            vk::Offset3D {
                                x: extent.width as i32,
                                y: extent.height as i32,
                                z: extent.depth as i32,
                            },
                        ])
                        .dst_offsets([
                            vk::Offset3D { x: 0, y: 0, z: 0 },
                            vk::Offset3D {
                                x: extent.width as i32,
                                y: extent.height as i32,
                                z: extent.depth as i32,
                            },
                        ])]),
            );
        }
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
