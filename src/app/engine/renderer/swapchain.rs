use crate::app::engine::rendering_context::{RenderingContext, Surface};
use anyhow::Result;
use ash::vk;
use std::sync::Arc;
use winit::window::Window;

pub struct Swapchain {
    pub desired_image_count: u32,
    pub format: vk::Format,
    pub extent: vk::Extent2D,
    pub image_views: Vec<vk::ImageView>,
    pub images: Vec<vk::Image>,
    handle: vk::SwapchainKHR,
    surface: Surface,
    window: Arc<Window>,
    context: Arc<RenderingContext>,
    pub is_dirty: bool,
}

impl Swapchain {
    pub fn new(context: Arc<RenderingContext>, window: Arc<Window>) -> Result<Self> {
        let surface = unsafe { context.create_surface(window.as_ref())? };

        let (format, color_space) = surface
            .formats
            .iter()
            .find(|f| {
                f.format == vk::Format::B8G8R8_SRGB
                    && f.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
            })
            .or_else(|| surface.formats.first())
            .map(|f| (f.format, f.color_space))
            .ok_or_else(|| anyhow::anyhow!("No supported surface formats"))?;

        //i'm using wayland, sizes can be 0 when created
        let extent = if surface.capabilities.current_extent.width != u32::MAX
            && surface.capabilities.current_extent.width != 0
        {
            surface.capabilities.current_extent
        } else {
            let size = window.inner_size();
            vk::Extent2D {
                width: size.width.max(1),
                height: size.height.max(1),
            }
        };
        let desired_image_count = (surface.capabilities.min_image_count + 1).clamp(
            surface.capabilities.max_image_count,
            if surface.capabilities.max_image_count == 0 {
                u32::MAX
            } else {
                surface.capabilities.max_image_count
            },
        );

        Ok(Self {
            desired_image_count,
            format,
            extent,
            image_views: vec![],
            images: vec![],
            handle: Default::default(),
            surface,
            window,
            context,
            is_dirty: true,
        })
    }

    fn recreate_swapchain(&mut self, old_swapchain: vk::SwapchainKHR) -> Result<()> {
        let new_handle = unsafe {
            self.context.swapchain_extension.create_swapchain(
                &vk::SwapchainCreateInfoKHR::default()
                    .surface(self.surface.handle)
                    .min_image_count(self.desired_image_count)
                    .image_format(self.format)
                    .image_color_space(vk::ColorSpaceKHR::SRGB_NONLINEAR)
                    .image_extent(self.extent)
                    .image_array_layers(1)
                    .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
                    .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
                    .pre_transform(self.surface.capabilities.current_transform)
                    .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
                    .present_mode(vk::PresentModeKHR::FIFO)
                    .clipped(true)
                    .old_swapchain(old_swapchain),
                None,
            )?
        };

        // Cleanup old if it existed
        if old_swapchain != vk::SwapchainKHR::null() {
            unsafe {
                self.context.device.device_wait_idle()?;
            }
            for image_view in self.image_views.drain(..) {
                unsafe {
                    self.context.device.destroy_image_view(image_view, None);
                }
            }
            unsafe {
                self.context
                    .swapchain_extension
                    .destroy_swapchain(old_swapchain, None);
            }
        }

        // Set up new
        self.handle = new_handle;
        self.images = unsafe {
            self.context
                .swapchain_extension
                .get_swapchain_images(new_handle)?
        };
        for image in &self.images {
            self.image_views.push(self.context.create_image_view(
                *image,
                self.format,
                vk::ImageAspectFlags::COLOR,
            )?);
        }
        Ok(())
    }

    pub fn create(&mut self) -> Result<()> {
        self.recreate_swapchain(vk::SwapchainKHR::null())
    }

    //todo - does this crash if it was resized to 0 manually by the user? probably, need to handle that case better, maybe just ignore resize events if the size is 0,
    // and then try to recreate the swapchain on the next render loop iteration, and if it still fails, then just keep ignoring resize events until it succeeds?
    pub fn resize(&mut self) -> Result<()> {
        let size = self.window.inner_size();
        self.extent = vk::Extent2D {
            width: size.width,
            height: size.height,
        };
        let old = self.handle;
        self.recreate_swapchain(old)
    }

    pub fn acquire_next_image(&mut self, image_available_semaphore: vk::Semaphore) -> Result<u32> {
        let (image_index, is_suboptimal) = unsafe {
            self.context.swapchain_extension.acquire_next_image(
                self.handle,
                u64::MAX,
                image_available_semaphore,
                vk::Fence::null(),
            )?
        };
        if is_suboptimal {
            self.is_dirty = true;
        }
        Ok(image_index)
    }

    pub fn present(
        &mut self,
        image_index: u32,
        render_finished_semaphore: vk::Semaphore,
    ) -> Result<()> {
        let is_suboptimal = unsafe {
            self.context.swapchain_extension.queue_present(
                self.context.queues[self.context.queue_families.present.index as usize],
                &vk::PresentInfoKHR::default()
                    .wait_semaphores(&[render_finished_semaphore])
                    .swapchains(&[self.handle])
                    .image_indices(&[image_index]), // idiomatic version of std::slice::from_ref(&image_index)
            )
        }?;
        if is_suboptimal {
            self.is_dirty = true;
        }
        Ok(())
    }
}

impl Drop for Swapchain {
    fn drop(&mut self) {
        unsafe {
            // Destroy image views
            for &image_view in &self.image_views {
                self.context.device.destroy_image_view(image_view, None);
            }
            // Destroy the swapchain itself
            if self.handle != vk::SwapchainKHR::null() {
                self.context
                    .swapchain_extension
                    .destroy_swapchain(self.handle, None);
            }
            // Destroy the surface (it's owned by this swapchain, created per window)
            self.context
                .surface_extension
                .destroy_surface(self.surface.handle, None);
        }
    }
}
