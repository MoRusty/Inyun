mod context;

use anyhow::Result;
use ash::vk;
use std::sync::Arc;
use winit::window::Window;

use crate::app::engine::renderer::context::{
    Context, ContextAttributes, PhysicalDevice, QueueFamilies,
};

pub struct Renderer {
    context: Context,
}
//alot in this Renderer needs to be improved
//I'm picking device 0, should really find the best one, and also pick the best queue family for each type of queue, not just graphics.
impl Renderer {
    fn pick_discrete_gpu(
        physical_devices: Vec<PhysicalDevice>,
    ) -> Result<(PhysicalDevice, QueueFamilies)> {
        // Try to find a discrete GPU first
        let best_device = physical_devices
            .iter() //
            .find(|d| d.properties.device_type == vk::PhysicalDeviceType::DISCRETE_GPU)
            .or_else(|| physical_devices.first()) //
            .ok_or_else(|| anyhow::anyhow!("No physical devices found"))?
            .clone();

        // Helpful for debugging when running elsewhere
        log::debug!("Selected device: {:?}", best_device.properties.device_type);

        // Just use the first queue family that supports graphics
        let graphics_family = best_device
            .queue_families
            .iter()
            .find(|qf| qf.properties.queue_flags.contains(vk::QueueFlags::GRAPHICS))
            .ok_or_else(|| anyhow::anyhow!("No graphics queue family"))?
            .clone();

        // Use the same queue family for everything (simple but works)
        //todo change it so that the best queue family is used for each type of queue (graphics, present, transfer, compute)
        Ok((
            best_device,
            QueueFamilies {
                graphics: graphics_family.clone(),
                present: graphics_family.clone(),
                transfer: graphics_family.clone(),
                compute: graphics_family.clone(),
            },
        ))
    }

    pub fn new(window: Arc<Window>) -> Result<Self> {
        let context = Context::new(ContextAttributes {
            window,
            queue_family_picker: Self::pick_discrete_gpu,
        })?;

        Ok(Self { context })
    }
}
