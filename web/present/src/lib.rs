//! The browser host of [`evenfall_present`]: draws a system's frames into a
//! canvas through WebGPU.
//!
//! This module is separate from the emulator core's WebAssembly module.
//! wgpu reaches WebGPU through wasm-bindgen, so this module imports from
//! JavaScript, while the core's module imports nothing. `present.js` wraps it
//! and loads it only in browsers that expose WebGPU.

#![cfg(target_arch = "wasm32")]

use std::sync::{Arc, Mutex};

use evenfall_host::{PixelFormat, VideoFormat};
use evenfall_present::{Preset, Renderer, Settings};
use wasm_bindgen::prelude::*;

/// One canvas drawn through one WebGPU device.
#[wasm_bindgen]
pub struct Presenter {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
    settings: Settings,
    /// Why the device can no longer draw: it was lost, or it reported an
    /// error nothing caught.
    failure: Arc<Mutex<Option<String>>>,
}

#[wasm_bindgen]
impl Presenter {
    /// Draws `source_width` x `source_height` frames into `canvas`, whose
    /// drawing buffer is `width` x `height` pixels.
    ///
    /// The canvas gets its WebGPU context only once an adapter and a device
    /// exist. Failing before that leaves it free for a 2D context.
    pub async fn create(
        canvas: web_sys::HtmlCanvasElement,
        source_width: u32,
        source_height: u32,
        width: u32,
        height: u32,
    ) -> Result<Presenter, JsError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::BROWSER_WEBGPU,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await?;
        let (device, queue) = adapter.request_device(&Default::default()).await?;
        let failure = Arc::new(Mutex::new(None));
        let lost = Arc::clone(&failure);
        device.set_device_lost_callback(move |reason, message| {
            record(&lost, format!("WebGPU device lost ({reason:?}): {message}"));
        });
        let uncaptured = Arc::clone(&failure);
        device.on_uncaptured_error(Arc::new(move |error| {
            record(&uncaptured, format!("WebGPU error: {error}"));
        }));

        let surface = instance.create_surface(wgpu::SurfaceTarget::Canvas(canvas))?;
        let mut config = surface
            .get_default_config(&adapter, width.max(1), height.max(1))
            .ok_or_else(|| JsError::new("the canvas can't show this adapter's output"))?;
        // Frames hold display-encoded values, so the canvas must not encode
        // them again. WebGPU canvases offer no sRGB formats anyway.
        config.format = config.format.remove_srgb_suffix();
        if surface
            .get_capabilities(&adapter)
            .alpha_modes
            .contains(&wgpu::CompositeAlphaMode::Opaque)
        {
            config.alpha_mode = wgpu::CompositeAlphaMode::Opaque;
        }
        surface.configure(&device, &config);

        let video =
            VideoFormat { width: source_width, height: source_height, pixel: PixelFormat::Bgr555 };
        let renderer = Renderer::new(&device, video, config.format);
        Ok(Presenter {
            device,
            queue,
            surface,
            config,
            renderer,
            settings: Settings::default(),
            failure,
        })
    }

    /// Uploads a completed frame, row-major, one 15-bit BGR pixel per `u16`.
    #[wasm_bindgen(js_name = setFrame)]
    pub fn set_frame(&mut self, pixels: &[u16]) -> Result<(), JsError> {
        let expected = self.renderer.video().width as usize * self.renderer.video().height as usize;
        if pixels.len() != expected {
            return Err(JsError::new(&format!(
                "a frame has {expected} pixels, not {}",
                pixels.len()
            )));
        }
        self.renderer.set_frame(&self.queue, pixels);
        Ok(())
    }

    /// Chooses the preset by name (`pixel` or `vanilla-plus`) and whether to
    /// scale only by whole multiples.
    pub fn configure(&mut self, preset: &str, integer_scale: bool) -> Result<(), JsError> {
        let preset = match preset {
            "pixel" => Preset::Pixel,
            "vanilla-plus" => Preset::VanillaPlus,
            other => return Err(JsError::new(&format!("unknown preset {other:?}"))),
        };
        self.settings = Settings { preset, integer_scale };
        Ok(())
    }

    /// Follows the canvas's drawing buffer, in device pixels.
    pub fn resize(&mut self, width: u32, height: u32) {
        let (width, height) = (width.max(1), height.max(1));
        if (width, height) != (self.config.width, self.config.height) {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);
        }
    }

    /// Draws the last frame. Returns false when the canvas had no texture to
    /// draw into this time, and fails once the device can no longer draw.
    pub fn draw(&mut self) -> Result<bool, JsError> {
        if let Some(failure) = self.failure.lock().ok().and_then(|failure| failure.clone()) {
            return Err(JsError::new(&failure));
        }
        let texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(false);
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return Ok(false);
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(JsError::new("the canvas rejected its configuration"));
            }
        };
        let view = texture.texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let drawable = (self.config.width, self.config.height);
        self.renderer.draw(&self.queue, &mut encoder, &view, drawable, self.settings);
        self.queue.submit([encoder.finish()]);
        self.queue.present(texture);
        Ok(true)
    }

    /// Draws the last frame as [`Presenter::draw`] would, into an offscreen
    /// texture of the canvas's size and format, and resolves to
    /// `{ width, height, rgba }`, the drawn pixels as RGBA bytes.
    ///
    /// Browsers don't let pages read a WebGPU canvas back reliably, so this
    /// is how a page or a test sees what the screen shows.
    pub fn capture(&self) -> Result<js_sys::Promise, JsError> {
        let (width, height) = (self.config.width, self.config.height);
        let format = self.config.format;
        let swap_red_blue = match format {
            wgpu::TextureFormat::Rgba8Unorm => false,
            wgpu::TextureFormat::Bgra8Unorm => true,
            other => return Err(JsError::new(&format!("can't capture {other:?}"))),
        };
        let target = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("evenfall-present capture"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let row = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("evenfall-present capture"),
            size: u64::from(row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let view = target.create_view(&Default::default());
        self.renderer.draw(&self.queue, &mut encoder, &view, (width, height), self.settings);
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: None,
                },
            },
            target.size(),
        );
        self.queue.submit([encoder.finish()]);
        let mapped = js_sys::Promise::new(&mut |resolve, reject| {
            readback.slice(..).map_async(wgpu::MapMode::Read, move |result| {
                let _ = match result {
                    Ok(()) => resolve.call0(&JsValue::NULL),
                    Err(error) => reject.call1(&JsValue::NULL, &error.to_string().into()),
                };
            });
        });
        Ok(wasm_bindgen_futures::future_to_promise(async move {
            wasm_bindgen_futures::JsFuture::from(mapped).await?;
            let data = readback
                .slice(..)
                .get_mapped_range()
                .map_err(|error| JsValue::from_str(&error.to_string()))?;
            let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
            for line in data.chunks(row as usize) {
                for &[first, green, third, alpha] in line[..width as usize * 4].as_chunks::<4>().0 {
                    let (red, blue) = if swap_red_blue { (third, first) } else { (first, third) };
                    rgba.extend([red, green, blue, alpha]);
                }
            }
            let result = js_sys::Object::new();
            js_sys::Reflect::set(&result, &"width".into(), &width.into())?;
            js_sys::Reflect::set(&result, &"height".into(), &height.into())?;
            js_sys::Reflect::set(&result, &"rgba".into(), &js_sys::Uint8Array::from(&rgba[..]))?;
            Ok(result.into())
        }))
    }

    /// Releases the device. The canvas keeps its WebGPU context.
    pub fn destroy(self) {
        self.device.destroy();
    }
}

/// Keeps the first failure, which is the one that explains the rest.
fn record(failure: &Mutex<Option<String>>, message: String) {
    if let Ok(mut failure) = failure.lock() {
        failure.get_or_insert(message);
    }
}
