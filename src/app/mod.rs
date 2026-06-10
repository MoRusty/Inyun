use crate::app::engine::Engine;
use winit::event_loop::ActiveEventLoop;
use winit::{application::ApplicationHandler, window::WindowAttributes};

mod engine;

#[derive(Default)]
pub struct App {
    engine: Option<Engine>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.engine = Some(Engine::new(event_loop).unwrap());
        if let Some(engine) = self.engine.as_mut() {
            let secondary_window = engine
                .create_window(
                    event_loop,
                    WindowAttributes::default().with_title("Secondary window"),
                )
                .unwrap();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        if let Some(engine) = self.engine.as_mut() {
            engine.window_event(event_loop, window_id, event);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // request redraw
        if let Some(engine) = self.engine.as_mut() {
            engine.request_redraw();
        }
    }

    fn suspended(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.engine = None;
    }
}
