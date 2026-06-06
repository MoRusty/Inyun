//libraries i plan to use, winit, ash, tracing, anyhow, gpu allocator, tokio?, pallete?, rapier physics?, nalgerbra?
//todo: add a way to specify which gpu to use, and also a way to specify which queue family to use for each type of queue (graphics, compute, transfer)
//todo: change all the pub functions if its internal crate use only to pub(crate) or private, only expose what needs to be exposed
//todo: change 2d to 3d

use winit::event_loop::EventLoop;

use crate::app::App;
use anyhow::Result;
mod app;

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let mut app = App::default();

    let event_loop = EventLoop::new()?;
    event_loop.run_app(&mut app)?;

    Ok(())
}

//Structs
//an example of a renderitem for batch rendering
// RenderItem{
//     mesh id,
//     material id,
//     transform
// } Hashmap for (mesh,material)
// (mesh_1, mat_1) → [obj1, obj2, obj3...]
// (mesh_2, mat_5) → [obj9, obj10...]
