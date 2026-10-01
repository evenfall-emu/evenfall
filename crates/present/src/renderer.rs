use evenfall_host::{PixelFormat, VideoFormat};

use crate::{Preset, Settings, Viewport};

/// Draws one system's frames into wgpu textures.
///
/// The renderer keeps the last frame it was given, so a host can redraw it
/// after a resize or a settings change without asking the emulator again.
pub struct Renderer {
    video: VideoFormat,
    pipeline: wgpu::RenderPipeline,
    source: wgpu::Texture,
    params: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// The frame's bytes in upload order, reused for every frame.
    upload: Vec<u8>,
}

impl Renderer {
    /// Builds the pipeline for frames of `video` drawn into textures of
    /// `target` format.
    ///
    /// The frame's colour values are already display-encoded, so `target`
    /// must not be an sRGB format, which would encode them a second time.
    /// Hosts whose surface prefers one draw through a view of
    /// [`wgpu::TextureFormat::remove_srgb_suffix`].
    pub fn new(device: &wgpu::Device, video: VideoFormat, target: wgpu::TextureFormat) -> Self {
        assert!(!target.is_srgb(), "{target:?} would encode display colours twice");
        // `scale.wgsl` decodes BGR555 only. A new pixel format stops this
        // binding from compiling until the shader decodes it too.
        let PixelFormat::Bgr555 = video.pixel;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("evenfall-present scale"),
            source: wgpu::ShaderSource::Wgsl(include_str!("scale.wgsl").into()),
        });
        let source = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("evenfall-present source"),
            size: wgpu::Extent3d {
                width: video.width,
                height: video.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R16Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("evenfall-present params"),
            size: PARAMS_BYTES as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("evenfall-present scale"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(target.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("evenfall-present scale"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &source.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry { binding: 1, resource: params.as_entire_binding() },
            ],
        });
        Self {
            video,
            pipeline,
            source,
            params,
            bind_group,
            upload: Vec::with_capacity(video.width as usize * video.height as usize * 2),
        }
    }

    /// The shape of the frames this renderer draws.
    pub fn video(&self) -> VideoFormat {
        self.video
    }

    /// Uploads a completed frame, row-major, one `u16` per pixel.
    ///
    /// # Panics
    ///
    /// If `pixels` doesn't hold exactly width * height pixels.
    pub fn set_frame(&mut self, queue: &wgpu::Queue, pixels: &[u16]) {
        let (width, height) = (self.video.width, self.video.height);
        assert_eq!(pixels.len(), width as usize * height as usize, "a {width}x{height} frame");
        self.upload.clear();
        self.upload.extend(pixels.iter().flat_map(|pixel| pixel.to_le_bytes()));
        queue.write_texture(
            self.source.as_image_copy(),
            &self.upload,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 2),
                rows_per_image: None,
            },
            self.source.size(),
        );
    }

    /// Records a pass that clears `target` to black and draws the last frame
    /// into the viewport `settings` gives for a `drawable` area (width,
    /// height), which must be the target's size in pixels.
    ///
    /// The settings go through one uniform buffer, so submit `encoder` before
    /// the next call. Returns the viewport, empty when nothing was drawn.
    pub fn draw(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        drawable: (u32, u32),
        settings: Settings,
    ) -> Viewport {
        let viewport =
            Viewport::fit((self.video.width, self.video.height), drawable, settings.integer_scale);
        let preset = match settings.preset {
            Preset::Pixel => 0,
            Preset::VanillaPlus => 1,
        };
        let params = [viewport.x, viewport.y, viewport.width, viewport.height, preset, 0, 0, 0];
        let mut bytes = [0; PARAMS_BYTES];
        for (chunk, value) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(params) {
            *chunk = value.to_le_bytes();
        }
        queue.write_buffer(&self.params, 0, &bytes);

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("evenfall-present scale"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        if !viewport.is_empty() {
            pass.set_viewport(
                viewport.x as f32,
                viewport.y as f32,
                viewport.width as f32,
                viewport.height as f32,
                0.0,
                1.0,
            );
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        viewport
    }
}

/// `Params` in `scale.wgsl`: five `u32`s, padded to 16 bytes.
const PARAMS_BYTES: usize = 32;
