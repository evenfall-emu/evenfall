use std::sync::Arc;

use evenfall_host::VideoFormat;
use evenfall_present::{Renderer, Settings};
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

/// The window's GPU surface, drawn by the shared presenter.
pub struct Screen {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    /// The format the presenter draws in: the surface's, without sRGB.
    view_format: wgpu::TextureFormat,
    renderer: Renderer,
    pub settings: Settings,
}

impl Screen {
    pub fn new(
        event_loop: &ActiveEventLoop,
        window: Arc<Window>,
        video: VideoFormat,
        settings: Settings,
    ) -> Result<Self, String> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(
                Box::new(event_loop.owned_display_handle()),
            ));
        let surface =
            instance.create_surface(Arc::clone(&window)).map_err(|error| error.to_string())?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .map_err(|error| error.to_string())?;
        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))
            .map_err(|error| error.to_string())?;

        let capabilities = surface.get_capabilities(&adapter);
        // Frames hold display-encoded values, so draw through a plain view
        // even when the surface itself is sRGB.
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(|format| !format.is_srgb())
            .or_else(|| capabilities.formats.first().copied())
            .ok_or("the window's surface offers no format")?;
        let view_format = format.remove_srgb_suffix();
        let present_mode = if capabilities.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else {
            wgpu::PresentMode::Fifo
        };
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or("the window's surface doesn't suit this adapter")?;
        config.format = format;
        config.present_mode = present_mode;
        config.view_formats = if view_format == format { vec![] } else { vec![view_format] };
        surface.configure(&device, &config);
        let renderer = Renderer::new(&device, video, view_format);
        let info = adapter.get_info();
        eprintln!("screen: {} through {:?}, {format:?}, {present_mode:?}", info.name, info.backend);
        Ok(Self { window, surface, device, queue, config, view_format, renderer, settings })
    }

    pub fn window(&self) -> &Window {
        &self.window
    }

    /// Follows the window's size in physical pixels.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// Draws `frame`. A surface that changed is reconfigured, and that frame
    /// is skipped.
    pub fn draw(&mut self, frame: &[u16]) {
        let texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Validation => return,
        };
        self.renderer.set_frame(&self.queue, frame);
        let view = texture.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.view_format),
            ..Default::default()
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let drawable = (self.config.width, self.config.height);
        self.renderer.draw(&self.queue, &mut encoder, &view, drawable, self.settings);
        self.queue.submit([encoder.finish()]);
        self.window.pre_present_notify();
        self.queue.present(texture);
    }
}
