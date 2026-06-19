mod renderer;
mod rendering_context;

use anyhow::Result;
use ash::vk;
use std::{collections::HashMap, sync::Arc};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowAttributes, WindowId};

use crate::app::engine::renderer::window_renderer;
use crate::app::engine::rendering_context::RenderingContext;
pub struct Engine {
    renderers: HashMap<WindowId, window_renderer::WindowRenderer>,
    windows: HashMap<WindowId, Arc<Window>>,
    primary_window_id: WindowId,
    rendering_context: Arc<RenderingContext>,
}

impl Engine {
    pub fn new(event_loop: &ActiveEventLoop) -> Result<Self> {
        let primary_window = Arc::new(event_loop.create_window(Default::default())?);
        let primary_window_id = primary_window.id();
        let windows = HashMap::from([(primary_window_id, primary_window.clone())]);

        let rendering_context = Arc::new(RenderingContext::new(
            rendering_context::RenderingContextAttributes {
                window: primary_window.clone(),
                queue_family_picker: RenderingContext::pick_discrete_gpu,
            },
        )?);

        let renderers = windows
            .iter()
            .map(|(id, window)| {
                let renderer = window_renderer::WindowRenderer::new(
                    rendering_context.clone(),
                    window.clone(),
                    2,
                    vk::Format::R16G16B16A16_SFLOAT,
                    vk::ClearColorValue {
                        float32: [0.0, 0.0, 0.0, 0.0],
                    },
                )
                .unwrap();
                (*id, renderer)
            })
            .collect::<HashMap<_, _>>();

        Ok(Self {
            renderers,
            windows,
            primary_window_id,
            rendering_context,
        })
    }
    pub fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: winit::event::WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                if window_id == self.primary_window_id {
                    event_loop.exit();
                } else {
                    self.renderers.remove(&window_id);
                    self.windows.remove(&window_id);
                }
            }
            WindowEvent::Resized(_) => {
                if let Some(renderer) = self.renderers.get_mut(&window_id) {
                    renderer.resize()
                }
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(renderer) = self.renderers.get_mut(&window_id) {
                    renderer.resize()
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(renderer) = self.renderers.get_mut(&window_id) {
                    renderer.render().unwrap()
                }
            }

            _ => {}
        }
    }

    pub fn create_window(
        &mut self,
        event_loop: &ActiveEventLoop,
        attributes: WindowAttributes,
    ) -> Result<WindowId> {
        let window = Arc::new(event_loop.create_window(attributes)?);
        let window_id = window.id();
        self.windows.insert(window_id, window.clone());
        let renderer = window_renderer::WindowRenderer::new(
            self.rendering_context.clone(),
            window.clone(),
            2,
            vk::Format::R16G16B16A16_SFLOAT,
            vk::ClearColorValue {
                float32: [0.0, 0.0, 0.0, 0.0],
            },
        )?;
        self.renderers.insert(window_id, renderer);
        Ok(window_id)
    }

    pub fn request_redraw(&mut self) {
        for window in self.windows.values() {
            window.request_redraw();
        }
    }
}
