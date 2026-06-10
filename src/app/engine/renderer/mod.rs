mod swapchain;

use anyhow::Result;
use ash::vk;
use ash::vk::{Rect2D, ShaderModule};
use std::path::Path;
use std::sync::Arc;
use winit::window::Window;

use crate::app::engine::renderer::swapchain::Swapchain;
use crate::app::engine::rendering_context::{ImageLayoutState, RenderingContext};

const SHADERS_DIR: &str = "resources/shaders";

struct Frame {
    command_buffer: vk::CommandBuffer,
    image_available_semaphore: vk::Semaphore,
    render_finished_semaphore: vk::Semaphore,
    in_flight_fence: vk::Fence,
}

pub struct Renderer {
    in_flight_frames_count: usize,
    frame_index: usize,
    frames: Vec<Frame>,
    command_pool: vk::CommandPool,
    pipeline_layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    swapchain: Swapchain,
    context: Arc<RenderingContext>,
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

        unsafe {
            let pipeline_layout = context
                .device
                .create_pipeline_layout(&vk::PipelineLayoutCreateInfo::default(), None)?;

            let pipeline = context.create_graphics_pipeline(
                vertex_shader,
                fragment_shader,
                swapchain.extent,
                swapchain.format,
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

            let in_flight_frames_count = 1;

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
                    let render_finished_semaphore = context
                        .device
                        .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)?;
                    let in_flight_fence = context.device.create_fence(
                        &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                        None,
                    )?;
                    Ok(Frame {
                        command_buffer,
                        image_available_semaphore,
                        render_finished_semaphore,
                        in_flight_fence,
                    })
                })
                .collect::<Result<Vec<_>>>()?;

            Ok(Self {
                in_flight_frames_count,
                frame_index: 0,
                frames,
                command_pool,
                pipeline_layout,
                pipeline,
                swapchain,
                context,
            })
        }
    }

    pub fn resize(&mut self) -> Result<()> {
        self.swapchain.resize()
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

            let renderable_image_state = ImageLayoutState {
                layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                access_mask: vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
                stage_mask: vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                queue_family: vk::QUEUE_FAMILY_IGNORED,
            };

            //transition image layout from undefined to color attachment
            self.context.transition_image_layout(
                frame.command_buffer,
                self.swapchain.images[image_index as usize],
                ImageLayoutState {
                    layout: vk::ImageLayout::UNDEFINED,
                    access_mask: vk::AccessFlags::empty(),
                    stage_mask: vk::PipelineStageFlags::TOP_OF_PIPE,
                    queue_family: vk::QUEUE_FAMILY_IGNORED,
                },
                renderable_image_state,
                vk::ImageAspectFlags::COLOR,
            );

            //transition image layout from color attachment to present
            self.context.transition_image_layout(
                frame.command_buffer,
                self.swapchain.images[image_index as usize],
                renderable_image_state,
                ImageLayoutState {
                    layout: vk::ImageLayout::PRESENT_SRC_KHR,
                    access_mask: vk::AccessFlags::empty(),
                    stage_mask: vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                    queue_family: vk::QUEUE_FAMILY_IGNORED,
                },
                vk::ImageAspectFlags::COLOR,
            );

            //draw here
            self.context.begin_rendering(
                frame.command_buffer,
                self.swapchain.image_views[self.frame_index],
                vk::ClearColorValue {
                    float32: [0.0, 0.0, 0.0, 1.0],
                },
                vk::Rect2D::default().extent(self.swapchain.extent),
            );

            self.context.device.cmd_bind_pipeline(
                frame.command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.pipeline,
            );

            self.context
                .device
                .cmd_draw(frame.command_buffer, 3, 1, 0, 0);

            self.context.device.cmd_end_rendering(frame.command_buffer);

            self.context
                .device
                .end_command_buffer(frame.command_buffer)?;

            //submit command buffer
            self.context.device.queue_submit(
                self.context.queues[self.context.queue_families.graphics.index as usize],
                &[vk::SubmitInfo::default()
                    .command_buffers(&[frame.command_buffer])
                    .wait_semaphores(&[frame.image_available_semaphore])
                    .wait_dst_stage_mask(&[vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT])
                    .signal_semaphores(&[frame.render_finished_semaphore])],
                frame.in_flight_fence,
            )?;

            //present
            self.swapchain
                .present(image_index, frame.render_finished_semaphore)?;

            //in flight frames are set to one, will need to cycle through if more than one
            //self.frame_index = (self.frame_index + 1) % self.in_flight_frames_count;

            Ok(())
        }
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            self.context.device.destroy_pipeline(self.pipeline, None);
            self.context
                .device
                .destroy_pipeline_layout(self.pipeline_layout, None);
        }
    }
}
