use crate::app::engine::renderer::Renderer;
use crate::app::engine::rendering_context::{
    Image, ImageAttributes, ImageLayoutState, RenderingContext,
};
use anyhow::Result;
use ash::vk;
use gpu_allocator::vulkan::AllocationScheme;
use image::*;
use std::sync::Arc;

pub struct ImageRenderer {
    context: Arc<RenderingContext>,
    renderer: Renderer,
    command_pool: vk::CommandPool,
    command_buffer: vk::CommandBuffer,
    clear_color: vk::ClearColorValue,
    fence: vk::Fence,
    image: Image,
}

impl ImageRenderer {
    pub fn new(
        context: Arc<RenderingContext>,
        clear_color: vk::ClearColorValue,
        resolution: (u32, u32),
        format: vk::Format,
    ) -> Result<Self> {
        unsafe {
            let command_pool = context.device.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(context.queue_families.graphics.index),
                None,
            )?;

            let command_buffer = context.device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )?[0];

            let mut renderer = Renderer::new(context.clone(), resolution, format)?;

            let fence = context
                .device
                .create_fence(&vk::FenceCreateInfo::default(), None)?;

            let image = context.create_image(
                "image",
                &mut renderer.allocator,
                ImageAttributes {
                    extent: vk::Extent3D {
                        width: resolution.0,
                        height: resolution.1,
                        depth: 1,
                    },
                    format,
                    linear: true,
                    usage: vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED,
                    location: gpu_allocator::MemoryLocation::CpuToGpu,
                    allocation_scheme: AllocationScheme::GpuAllocatorManaged,
                },
            )?;

            Ok(Self {
                context,
                renderer,
                command_pool,
                command_buffer,
                clear_color,
                fence,
                image,
            })
        }
    }

    pub fn render(&mut self) -> Result<()> {
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
        unsafe {
            self.context
                .device
                .reset_command_pool(self.command_pool, vk::CommandPoolResetFlags::empty())?;

            self.context.device.begin_command_buffer(
                self.command_buffer,
                &vk::CommandBufferBeginInfo::default(),
            )?;

            self.renderer
                .render(self.command_buffer, self.clear_color)?;

            //transition image layout from undefined to transfer
            self.context.transition_image_layout(
                self.command_buffer,
                self.image.handle,
                undefined_image_state,
                transfer_dst_image_state,
            );

            //transition render target image to transfer source layout
            self.context.transition_image_layout(
                self.command_buffer,
                self.renderer.render_target.handle,
                self.renderer.render_target.layout,
                transfer_src_image_state,
            );

            self.context.blit_image(
                self.command_buffer,
                self.renderer.render_target.handle,
                transfer_src_image_state.layout,
                self.image.handle,
                transfer_dst_image_state.layout,
                self.renderer.render_target.attributes.extent,
            );

            self.context
                .device
                .end_command_buffer(self.command_buffer)?;

            self.context.device.queue_submit2(
                self.context.queues[self.context.queue_families.graphics.index as usize],
                &[vk::SubmitInfo2::default().command_buffer_infos(&[
                    vk::CommandBufferSubmitInfo::default()
                        .command_buffer(self.command_buffer)
                        .device_mask(1),
                ])],
                self.fence,
            )?;

            self.context
                .device
                .wait_for_fences(&[self.fence], true, u64::MAX)?;
            self.context.device.reset_fences(&[self.fence])?;

            let data = self
                .image
                .allocation
                .as_ref()
                .unwrap()
                .mapped_slice()
                .unwrap();

            // Query the actual row pitch - LINEAR tiling may add padding between rows
            let subresource_layout = self.context.device.get_image_subresource_layout(
                self.image.handle,
                vk::ImageSubresource::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .mip_level(0)
                    .array_layer(0),
            );

            let width = self.image.attributes.extent.width as usize;
            let height = self.image.attributes.extent.height as usize;
            let row_pitch = subresource_layout.row_pitch as usize;
            let tight_row = width * 4; // R8G8B8A8 = 4 bytes per pixel
            let base = subresource_layout.offset as usize;

            // Strip any row padding so the image crate gets tightly packed rows
            let packed: Vec<u8> = if row_pitch != tight_row {
                let mut buf = Vec::with_capacity(tight_row * height);
                for row in 0..height {
                    let start = base + row * row_pitch;
                    buf.extend_from_slice(&data[start..start + tight_row]);
                }
                buf
            } else {
                data[base..base + tight_row * height].to_vec()
            };

            //save using image crate
            save_buffer(
                "image.png",
                &packed,
                self.image.attributes.extent.width,
                self.image.attributes.extent.height,
                ColorType::Rgba8,
            )?;
        }
        Ok(())
    }
}

impl Drop for ImageRenderer {
    fn drop(&mut self) {
        unsafe {
            self.context.device.device_wait_idle().unwrap();
            self.context
                .destroy_image(&mut self.image, &mut self.renderer.allocator)
                .unwrap();
            self.context.device.destroy_fence(self.fence, None);
            self.context
                .device
                .destroy_command_pool(self.command_pool, None);
        }
    }
}
