//todo: go back over the wait - signal semaphoes. make sure the correct stage mask is set.
mod swapchain;

use anyhow::Result;
use ash::vk;
use ash::vk::ShaderModule;
use gpu_allocator::MemoryLocation;
use gpu_allocator::vulkan::{AllocationScheme, Allocator};
use std::sync::Arc;
use winit::window::Window;

use crate::app::engine::renderer::swapchain::Swapchain;
use crate::app::engine::rendering_context::{
    Image, ImageAttributes, ImageLayoutState, RenderingContext,
};

const SHADERS_DIR: &str = "resources/shaders";

struct Frame {
    command_buffer: vk::CommandBuffer,
    image_available_semaphore: vk::Semaphore,
    in_flight_fence: vk::Fence,
}

pub struct Renderer {
    allocator: Allocator,
    in_flight_frames_count: usize,
    frame_index: usize,
    frames: Vec<Frame>,
    render_finished_semaphores: Vec<vk::Semaphore>,
    command_pool: vk::CommandPool,
    pipeline_layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    swapchain: Swapchain,
    context: Arc<RenderingContext>,
    render_target: Image,
}

fn load_shader_module(context: &RenderingContext, path: &str) -> Result<ShaderModule> {
    let code = std::fs::read(format!("{}/{}", SHADERS_DIR, path))?;
    context.create_shader_module(&code)
}

//alot in this Renderer needs to be improved
//I'm picking device 0, should really find the best one, and also pick the best queue family for each type of queue, not just graphics.
impl Renderer {
    pub fn new(context: Arc<RenderingContext>, window: Arc<Window>) -> Result<Self> {
        let mut swapchain = Swapchain::new(context.clone(), window.clone())?;
        swapchain.create()?;

        let vertex_shader = load_shader_module(context.as_ref(), "vert.spv")?; //&context returns &Arc rather than &RenderingContext
        let fragment_shader = load_shader_module(context.as_ref(), "frag.spv")?;

        let mut allocator = context.create_allocator(Default::default(), Default::default())?;
        //todo: should probably have a render_target for each frame in flight? or all frames? come back to this and figure it out.
        let render_target = context.create_image(
            "render target",
            &mut allocator,
            ImageAttributes {
                location: MemoryLocation::GpuOnly,
                linear: false,
                allocation_scheme: AllocationScheme::GpuAllocatorManaged,
                usage: vk::ImageUsageFlags::TRANSFER_DST
                    | vk::ImageUsageFlags::TRANSFER_SRC
                    | vk::ImageUsageFlags::COLOR_ATTACHMENT
                    | vk::ImageUsageFlags::INPUT_ATTACHMENT,
                format: vk::Format::R16G16B16A16_SFLOAT,
                extent: vk::Extent3D {
                    width: swapchain.extent.width,
                    height: swapchain.extent.height,
                    depth: 1,
                },
            },
        )?;

        unsafe {
            let pipeline_layout = context
                .device
                .create_pipeline_layout(&vk::PipelineLayoutCreateInfo::default(), None)?;

            let pipeline = context.create_graphics_pipeline(
                vertex_shader,
                fragment_shader,
                render_target.attributes.format,
                pipeline_layout,
                Default::default(),
            )?;

            context.device.destroy_shader_module(vertex_shader, None);
            context.device.destroy_shader_module(fragment_shader, None);

            let command_pool = context.device.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(context.queue_families.graphics.index)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            )?;

            let in_flight_frames_count = 2;

            let command_buffers = context.device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(in_flight_frames_count as u32),
            )?;

            let frames = command_buffers
                .into_iter()
                .map(|command_buffer| {
                    let image_available_semaphore = context
                        .device
                        .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)?;
                    let in_flight_fence = context.device.create_fence(
                        &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                        None,
                    )?;
                    Ok(Frame {
                        command_buffer,
                        image_available_semaphore,
                        in_flight_fence,
                    })
                })
                .collect::<Result<Vec<_>>>()?;

            // Create render_finished_semaphores for each swapchain image
            let mut render_finished_semaphores = Vec::new();
            for _ in 0..swapchain.images.len() {
                let semaphore = context
                    .device
                    .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)?;
                render_finished_semaphores.push(semaphore);
            }

            Ok(Self {
                allocator,
                in_flight_frames_count,
                frame_index: 0,
                frames,
                render_finished_semaphores,
                command_pool,
                pipeline_layout,
                pipeline,
                swapchain,
                context,
                render_target,
            })
        }
    }

    pub fn resize(&mut self) -> Result<()> {
        //self.swapchain.resize()
        self.swapchain.is_dirty = true;
        Ok(())
    }

    pub fn render(&mut self) -> Result<()> {
        let frame = &self.frames[self.frame_index];

        unsafe {
            self.context
                .device
                .wait_for_fences(&[frame.in_flight_fence], true, u64::MAX)?;
            self.context.device.reset_fences(&[frame.in_flight_fence])?;
            self.context
                .device
                .reset_command_buffer(frame.command_buffer, vk::CommandBufferResetFlags::empty())?;

            let image_index = self
                .swapchain
                .acquire_next_image(frame.image_available_semaphore)?;

            self.context.device.begin_command_buffer(
                frame.command_buffer,
                &vk::CommandBufferBeginInfo::default(),
            )?;

            //todo: move these over into a enum or CONST?
            let undefined_image_state = ImageLayoutState {
                layout: vk::ImageLayout::UNDEFINED,
                access_mask: vk::AccessFlags2::NONE,
                stage_mask: vk::PipelineStageFlags2::NONE,
                queue_family: vk::QUEUE_FAMILY_IGNORED,
            };

            let renderable_image_state = ImageLayoutState {
                layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                access_mask: vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
                stage_mask: vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
                queue_family: vk::QUEUE_FAMILY_IGNORED,
            };

            let transfer_image_state = ImageLayoutState {
                layout: vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                access_mask: vk::AccessFlags2::TRANSFER_READ,
                stage_mask: vk::PipelineStageFlags2::TRANSFER,
                queue_family: vk::QUEUE_FAMILY_IGNORED,
            };
            let transfer_dst_image_state = ImageLayoutState {
                layout: vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                access_mask: vk::AccessFlags2::TRANSFER_WRITE,
                stage_mask: vk::PipelineStageFlags2::TRANSFER,
                queue_family: vk::QUEUE_FAMILY_IGNORED,
            };

            let present_image_state = ImageLayoutState {
                layout: vk::ImageLayout::PRESENT_SRC_KHR,
                access_mask: vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
                stage_mask: vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
                queue_family: vk::QUEUE_FAMILY_IGNORED,
            };

            //transition render target image layout from undefined to color attachment
            self.context.transition_image_layout(
                frame.command_buffer,
                self.render_target.handle,
                undefined_image_state,
                renderable_image_state,
            );

            self.context.begin_rendering(
                frame.command_buffer,
                self.render_target.view,
                vk::ClearColorValue {
                    float32: [0.0, 0.0, 0.0, 1.0],
                },
                vk::Rect2D::default().extent(self.swapchain.extent),
            );
            self.draw(frame.command_buffer);

            self.context.device.cmd_end_rendering(frame.command_buffer);

            //transition render target image to transfer source layout
            self.context.transition_image_layout(
                frame.command_buffer,
                self.render_target.handle,
                renderable_image_state,
                transfer_image_state,
            );

            //transition swapchain image layout from undefined to transfer
            self.context.transition_image_layout(
                frame.command_buffer,
                self.swapchain.images[image_index as usize],
                undefined_image_state,
                transfer_dst_image_state,
            );

            //copy the render target image to the swapchain image/ todo: remain to blit and check parameters
            self.context.copy_image(
                frame.command_buffer,
                self.render_target.handle,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                self.swapchain.images[image_index as usize],
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                self.render_target.attributes.extent,
            );

            //transition swapchain image layout from transfer to present
            self.context.transition_image_layout(
                frame.command_buffer,
                self.swapchain.images[image_index as usize],
                transfer_dst_image_state,
                present_image_state,
            );

            self.context
                .device
                .end_command_buffer(frame.command_buffer)?;

            //submit command buffer
            // self.context.device.queue_submit(
            //     self.context.queues[self.context.queue_families.graphics.index as usize],
            //     &[vk::SubmitInfo::default()
            //         .command_buffers(&[frame.command_buffer])
            //         .wait_semaphores(&[frame.image_available_semaphore])
            //         .wait_dst_stage_mask(&[vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT])
            //         .signal_semaphores(&[self.render_finished_semaphores[image_index as usize]])],
            //     frame.in_flight_fence,
            // )?;
            self.context.device.queue_submit2(
                self.context.queues[self.context.queue_families.graphics.index as usize],
                &[vk::SubmitInfo2::default()
                    .command_buffer_infos(&[vk::CommandBufferSubmitInfo::default()
                        .command_buffer(frame.command_buffer)
                        .device_mask(1)])
                    .wait_semaphore_infos(&[vk::SemaphoreSubmitInfo::default()
                        .semaphore(frame.image_available_semaphore)
                        .stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)])
                    .signal_semaphore_infos(&[vk::SemaphoreSubmitInfo::default()
                        .semaphore(self.render_finished_semaphores[image_index as usize])
                        .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)])],
                frame.in_flight_fence,
            )?;

            //present
            self.swapchain.present(
                image_index,
                self.render_finished_semaphores[image_index as usize],
            )?;

            //in flight frames are set to one, will need to cycle through if more than one
            //self.frame_index = (self.frame_index + 1) % self.in_flight_frames_count;

            Ok(())
        }
    }
    pub fn draw(&self, command_buffer: vk::CommandBuffer) {
        //draw here
        unsafe {
            self.context.device.cmd_set_viewport(
                command_buffer,
                0,
                &[vk::Viewport::default()
                    .width(self.swapchain.extent.width as f32)
                    .height(self.swapchain.extent.height as f32)],
            );

            self.context.device.cmd_set_scissor(
                command_buffer,
                0,
                &[vk::Rect2D::default().extent(self.swapchain.extent)],
            );

            self.context.device.cmd_bind_pipeline(
                command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.pipeline,
            );

            self.context.device.cmd_draw(command_buffer, 3, 1, 0, 0);
        }
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            self.context.device.device_wait_idle().unwrap();

            self.context
                .destroy_image(&mut self.render_target, &mut self.allocator)
                .unwrap();

            for &semaphore in &self.render_finished_semaphores {
                self.context.device.destroy_semaphore(semaphore, None);
            }
            for frame in &self.frames {
                self.context
                    .device
                    .destroy_semaphore(frame.image_available_semaphore, None);
                self.context
                    .device
                    .destroy_fence(frame.in_flight_fence, None);
            }
            self.context
                .device
                .destroy_command_pool(self.command_pool, None);
            self.context.device.destroy_pipeline(self.pipeline, None);
            self.context
                .device
                .destroy_pipeline_layout(self.pipeline_layout, None);
        }
    }
}
