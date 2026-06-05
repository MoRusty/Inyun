use anyhow::Result;
use std::sync::Arc;
use winit::window::Window;

use crate::app::engine::rendering_context::RenderingContext;

pub struct Renderer {
    context: Arc<RenderingContext>,
}
//alot in this Renderer needs to be improved
//I'm picking device 0, should really find the best one, and also pick the best queue family for each type of queue, not just graphics.
impl Renderer {
    pub fn new(context: Arc<RenderingContext>, window: Arc<Window>) -> Result<Self> {
        //moved context creation to Engine, will be a shared resource owned by engine
        //no need to have multiple contexts, and it needs to be created before the renderer so that it can be shared between renderers if needed
        // let context = Context::new(ContextAttributes {
        //     window,
        //     queue_family_picker: Self::pick_discrete_gpu,
        // })?;

        Ok(Self { context })
    }
}
