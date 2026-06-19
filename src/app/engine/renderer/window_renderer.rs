use crate::app::engine::renderer::Renderer;
use crate::app::engine::renderer::swapchain::Swapchain;
use crate::app::engine::rendering_context::{ImageLayoutState, RenderingContext};
use anyhow::Result;
use ash::vk;
use std::sync::Arc;
use winit::window::Window;

pub struct WindowRenderer {
    pub renderer: Renderer,
    in_flight_frames_count: usize,
    frame_index: usize,
    frames: Vec<Frame>,
    command_pool: vk::CommandPool,
    swapchain: Swapchain,
    context: Arc<RenderingContext>,
    clear_color: vk::ClearColorValue,
    render_finished_semaphores: Vec<vk::Semaphore>,
}

struct Frame {
    command_buffer: vk::CommandBuffer,
    image_available_semaphore: vk::Semaphore,
    in_flight_fence: vk::Fence,
}

impl WindowRenderer {
    pub fn new(
        context: Arc<RenderingContext>,
        window: Arc<Window>,
        in_flight_frames_count: usize,
        format: vk::Format,
        clear_color: vk::ClearColorValue,
    ) -> Result<Self> {
        let mut swapchain = Swapchain::new(context.clone(), window.clone())?;
        swapchain.create()?;

        unsafe {
            let command_pool = context.device.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(context.queue_families.graphics.index)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            )?;

            let command_buffers = context.device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(in_flight_frames_count as u32),
            )?;

            let mut frames = Vec::with_capacity(command_buffers.len());

            for &command_buffer in command_buffers.iter() {
                let image_available_semaphore = context
                    .device
                    .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)?;
                let in_flight_fence = context.device.create_fence(
                    &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                    None,
                )?;

                frames.push(Frame {
                    command_buffer,
                    image_available_semaphore,
                    in_flight_fence,
                });
            }

            let mut render_finished_semaphores = Vec::with_capacity(swapchain.images.len());
            for _ in 0..swapchain.images.len() {
                let sem = context
                    .device
                    .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)?;
                render_finished_semaphores.push(sem);
            }

            let renderer = Renderer::new(
                context.clone(),
                (swapchain.extent.width, swapchain.extent.height),
                format,
            )?;

            Ok(Self {
                renderer,
                in_flight_frames_count,
                frame_index: 0,
                frames,
                clear_color,
                command_pool,
                swapchain,
                context,
                render_finished_semaphores,
            })
        }
    }

    pub fn resize(&mut self) {
        self.swapchain.is_dirty = true;
    }

    pub fn render(&mut self) -> Result<()> {
        let frame = &self.frames[self.frame_index];

        //todo: move these over into a enum or CONST?
        let undefined_image_state = ImageLayoutState {
            layout: vk::ImageLayout::UNDEFINED,
            access_mask: vk::AccessFlags2::NONE,
            stage_mask: vk::PipelineStageFlags2::NONE,
            queue_family: vk::QUEUE_FAMILY_IGNORED,
        };

        let transfer_src_image_state = ImageLayoutState {
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

        unsafe {
            self.context
                .device
                .wait_for_fences(&[frame.in_flight_fence], true, u64::MAX)?;
            self.context.device.reset_fences(&[frame.in_flight_fence])?;
            self.context
                .device
                .reset_command_buffer(frame.command_buffer, vk::CommandBufferResetFlags::empty())?;

            if self.swapchain.is_dirty {
                self.swapchain.resize()?;
                self.renderer
                    .resize((self.swapchain.extent.width, self.swapchain.extent.height))?;
                self.swapchain.is_dirty = false;
            }

            let image_index = self
                .swapchain
                .acquire_next_image(frame.image_available_semaphore)?;

            self.context.device.begin_command_buffer(
                frame.command_buffer,
                &vk::CommandBufferBeginInfo::default(),
            )?;

            self.renderer
                .render(frame.command_buffer, self.clear_color)?;

            //transition render target image to transfer source layout
            self.context.transition_image_layout(
                frame.command_buffer,
                self.renderer.render_target.handle,
                self.renderer.render_target.layout,
                transfer_src_image_state,
            );

            //transition swapchain image layout from undefined to transfer
            self.context.transition_image_layout(
                frame.command_buffer,
                self.swapchain.images[image_index as usize],
                undefined_image_state,
                transfer_dst_image_state,
            );

            //copy the render target image to the swapchain image
            //todo: i'm storing layout in the image struct but not really using it for anything? i'm manually putting in the "old" layout here,
            // but maybe I should be using the stored layout instead? or maybe I should just remove the stored layout since it's not really being used for anything?
            // when i did use the stored one i'm getting a validation so i'm not even updating it correctly.
            self.context.blit_image(
                frame.command_buffer,
                self.renderer.render_target.handle,
                transfer_src_image_state.layout,
                self.swapchain.images[image_index as usize],
                transfer_dst_image_state.layout,
                self.renderer.render_target.attributes.extent,
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

            self.frame_index = (self.frame_index + 1) % self.in_flight_frames_count;

            Ok(())
        }
    }
}

impl Drop for WindowRenderer {
    fn drop(&mut self) {
        unsafe {
            self.context.device.device_wait_idle().unwrap();

            for frame in &self.frames {
                self.context
                    .device
                    .destroy_semaphore(frame.image_available_semaphore, None);
                self.context
                    .device
                    .destroy_fence(frame.in_flight_fence, None);
            }

            for sem in &self.render_finished_semaphores {
                self.context.device.destroy_semaphore(*sem, None);
            }

            self.context
                .device
                .destroy_command_pool(self.command_pool, None);
        }
    }
}
