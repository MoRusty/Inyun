mod swapchain;

use anyhow::Result;
use ash::vk;
use ash::vk::ShaderModule;
use std::path::Path;
use std::sync::Arc;
use winit::window::Window;

use crate::app::engine::renderer::swapchain::Swapchain;
use crate::app::engine::rendering_context::RenderingContext;

const SHADERS_DIR: &str = "resources/shaders";
pub struct Renderer {
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
            Ok(Self {
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
