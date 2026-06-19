//todo: go back over the wait - signal semaphoes. make sure the correct stage mask is set.
pub(crate) mod image_renderer;
mod swapchain;
pub mod window_renderer;

use anyhow::Result;
use ash::vk;
use ash::vk::{ClearColorValue, CommandBuffer, ShaderModule};
use gpu_allocator::MemoryLocation;
use gpu_allocator::vulkan::{AllocationScheme, Allocator};
use std::sync::Arc;

use crate::app::engine::renderer::swapchain::Swapchain;
use crate::app::engine::rendering_context::{
    Image, ImageAttributes, ImageLayoutState, RenderingContext,
};

const SHADERS_DIR: &str = "resources/shaders";

pub struct Renderer {
    allocator: Allocator,
    pipeline_layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    context: Arc<RenderingContext>,
    //todo: using a shared render target, should eventually have one per in-flight frame
    // ignore until i start using and post processing that requires previous frame data
    // Vec<Image>
    render_target: Image,
}

fn load_shader_module(context: &RenderingContext, path: &str) -> Result<ShaderModule> {
    let code = std::fs::read(format!("{}/{}", SHADERS_DIR, path))?;
    context.create_shader_module(&code)
}

fn create_render_target(
    context: &RenderingContext,
    allocator: &mut Allocator,
    resolution: (u32, u32),
    format: vk::Format,
) -> Result<Image> {
    context.create_image(
        "render target",
        allocator,
        ImageAttributes {
            location: MemoryLocation::GpuOnly,
            linear: false,
            allocation_scheme: AllocationScheme::GpuAllocatorManaged,
            usage: vk::ImageUsageFlags::TRANSFER_DST
                | vk::ImageUsageFlags::TRANSFER_SRC
                | vk::ImageUsageFlags::COLOR_ATTACHMENT
                | vk::ImageUsageFlags::INPUT_ATTACHMENT,
            format,
            extent: vk::Extent3D {
                width: resolution.0,
                height: resolution.1,
                depth: 1,
            },
        },
    )
}

//alot in this Renderer needs to be improved
//I'm picking device 0, should really find the best one, and also pick the best queue family for each type of queue, not just graphics.
impl Renderer {
    pub fn new(
        context: Arc<RenderingContext>,
        resolution: (u32, u32),
        format: vk::Format,
    ) -> Result<Self> {
        let vertex_shader = load_shader_module(context.as_ref(), "vert.spv")?; //&context returns &Arc rather than &RenderingContext
        let fragment_shader = load_shader_module(context.as_ref(), "frag.spv")?;

        let mut allocator = context.create_allocator(Default::default(), Default::default())?;
        //todo: should probably have a render_target for each frame in flight? or all frames? come back to this and figure it out.
        let render_target =
            create_render_target(context.as_ref(), &mut allocator, resolution, format)?;

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

            Ok(Self {
                allocator,
                pipeline_layout,
                pipeline,
                context,
                render_target,
            })
        }
    }

    pub fn resize(&mut self, resolution: (u32, u32)) -> Result<()> {
        self.context
            .destroy_image(&mut self.render_target, &mut self.allocator)?;
        self.render_target = create_render_target(
            self.context.as_ref(),
            &mut self.allocator,
            resolution,
            self.render_target.attributes.format,
        )?;
        Ok(())
    }

    pub fn render(
        &mut self,
        command_buffer: CommandBuffer,
        clear_color: ClearColorValue,
    ) -> Result<()> {
        unsafe {
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

            //transition render target image layout from undefined to color attachment
            self.context.transition_image_layout(
                command_buffer,
                self.render_target.handle,
                undefined_image_state,
                renderable_image_state,
            );

            //set the image layout to one used, context should match current layout of image
            self.render_target.layout = renderable_image_state;

            self.context.begin_rendering(
                command_buffer,
                self.render_target.view,
                clear_color,
                vk::Rect2D::default().extent(
                    vk::Extent2D::default()
                        .width(self.render_target.attributes.extent.width)
                        .height(self.render_target.attributes.extent.height),
                ),
            );
            self.draw(command_buffer);

            self.context.device.cmd_end_rendering(command_buffer);

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
                    .width(self.render_target.attributes.extent.width as f32)
                    .height(self.render_target.attributes.extent.height as f32)],
            );

            self.context.device.cmd_set_scissor(
                command_buffer,
                0,
                &[vk::Rect2D::default().extent(
                    vk::Extent2D::default()
                        .width(self.render_target.attributes.extent.width)
                        .height(self.render_target.attributes.extent.height),
                )],
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

            self.context.device.destroy_pipeline(self.pipeline, None);
            self.context
                .device
                .destroy_pipeline_layout(self.pipeline_layout, None);
        }
    }
}
