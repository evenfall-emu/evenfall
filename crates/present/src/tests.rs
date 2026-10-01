//! The renderer on a real adapter, against hand-derived fixtures and the f64
//! reference.
//!
//! These tests need a GPU adapter, and a software one such as Mesa's
//! lavapipe is enough. Without one they pass with a note, unless
//! `EVENFALL_REQUIRE_GPU` is set, which makes a missing adapter fail as CI
//! requires.

use std::future::IntoFuture;
use std::pin::pin;
use std::sync::OnceLock;
use std::task::{Context, Poll, Waker};

use evenfall_host::{PixelFormat, VideoFormat};

use crate::{Preset, Renderer, Settings, reference};

const BLACK: [u8; 4] = [0, 0, 0, 255];
/// Grey at 10/31 of full scale, 82.26 in 8 bits.
const GREY: u16 = 10 | 10 << 5 | 10 << 10;
const GREY_RGBA: [u8; 4] = [82, 82, 82, 255];
const WHITE: u16 = 0x7fff;
const WHITE_RGBA: [u8; 4] = [255, 255, 255, 255];

fn video(width: u32, height: u32) -> VideoFormat {
    VideoFormat { width, height, pixel: PixelFormat::Bgr555 }
}

fn settings(preset: Preset, integer_scale: bool) -> Settings {
    Settings { preset, integer_scale }
}

/// Native wgpu resolves adapter and device requests before returning them.
fn resolved<F: IntoFuture>(future: F) -> F::Output {
    let mut future = pin!(future.into_future());
    match future.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("native wgpu requests resolve immediately"),
    }
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

/// One device on the default adapter for every test, honouring wgpu's
/// `WGPU_*` variables.
///
/// One instance for the whole process: the Vulkan loader (1.4.304 here)
/// crashes in `vkSetDebugUtilsObjectNameEXT` when instances on parallel test
/// threads label their pipelines, as wgpu's debug builds do.
fn gpu() -> Option<&'static Gpu> {
    static GPU: OnceLock<Option<Gpu>> = OnceLock::new();
    let gpu = GPU.get_or_init(|| {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = match resolved(instance.request_adapter(&Default::default())) {
            Ok(adapter) => adapter,
            Err(error) => {
                eprintln!("skipped: no GPU adapter ({error}); Mesa's lavapipe is a software one");
                return None;
            }
        };
        let (device, queue) =
            resolved(adapter.request_device(&Default::default())).expect("a device on the adapter");
        Some(Gpu { device, queue })
    });
    assert!(
        gpu.is_some() || std::env::var_os("EVENFALL_REQUIRE_GPU").is_none(),
        "EVENFALL_REQUIRE_GPU is set, but there is no adapter"
    );
    gpu.as_ref()
}

impl Gpu {
    /// Draws `pixels` into a new `drawable`-sized texture and reads back its
    /// RGBA bytes, rows packed.
    fn render(
        &self,
        video: VideoFormat,
        pixels: &[u16],
        drawable: (u32, u32),
        settings: Settings,
    ) -> Vec<u8> {
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut renderer = Renderer::new(&self.device, video, format);
        renderer.set_frame(&self.queue, pixels);
        let (width, height) = drawable;
        let target = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
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
            label: None,
            size: u64::from(row * height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let view = target.create_view(&Default::default());
        renderer.draw(&self.queue, &mut encoder, &view, drawable, settings);
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
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |mapped| mapped.expect("readback mapped"));
        self.device.poll(wgpu::PollType::wait_indefinitely()).expect("the GPU finished");
        let mapped = slice.get_mapped_range().expect("readback range");
        mapped.chunks(row as usize).flat_map(|line| &line[..width as usize * 4]).copied().collect()
    }
}

/// Checks every channel is within `tolerance` of `expected`, reporting the
/// first pixel that isn't.
#[track_caller]
fn assert_close(actual: &[u8], expected: &[u8], width: u32, tolerance: u8, what: &str) {
    assert_eq!(actual.len(), expected.len(), "{what}: size");
    let bad = actual
        .chunks(4)
        .zip(expected.chunks(4))
        .position(|(a, e)| a.iter().zip(e).any(|(a, e)| a.abs_diff(*e) > tolerance));
    if let Some(i) = bad {
        let (x, y) = (i as u32 % width, i as u32 / width);
        panic!(
            "{what}: pixel ({x}, {y}) is {:?}, expected {:?} within {tolerance}",
            &actual[i * 4..][..4],
            &expected[i * 4..][..4],
        );
    }
}

/// Checks the reference exactly and the GPU within `tolerance` against a
/// hand-derived image, one RGBA entry per pixel.
#[track_caller]
fn check_fixture(
    video: VideoFormat,
    pixels: &[u16],
    drawable: (u32, u32),
    settings: Settings,
    expected: &[[u8; 4]],
    tolerance: u8,
) {
    let expected = expected.concat();
    let reference = reference::render(video, pixels, drawable, settings);
    assert_close(&reference, &expected, drawable.0, 0, "reference");
    if let Some(gpu) = gpu() {
        let drawn = gpu.render(video, pixels, drawable, settings);
        assert_close(&drawn, &expected, drawable.0, tolerance, "GPU");
    }
}

/// Red sits in bits 0-4 and blue in 10-14, and bit 15 changes nothing.
/// Each 5-bit channel c becomes round(c * 255 / 31), the conversion the web
/// player's Canvas 2D path uses, so 16 becomes 132.
#[test]
fn decodes_bgr555_and_ignores_bit_15() {
    check_fixture(
        video(4, 1),
        &[0x001f, 0x03e0, 0xfc00, 0x4210],
        (4, 1),
        settings(Preset::Pixel, false),
        &[[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [132, 132, 132, 255]],
        0,
    );
}

/// Two pixels stretched to three columns: the centres at 0.5, 1.5 and 2.5
/// land on source positions 0.33, 1.0 and 1.67, so the middle column takes
/// the second pixel. Sampling at the left edges instead would give grey,
/// grey, white.
#[test]
fn nearest_takes_the_pixel_under_each_centre() {
    let row = [GREY_RGBA, WHITE_RGBA, WHITE_RGBA];
    check_fixture(
        video(2, 1),
        &[GREY, WHITE],
        (3, 2),
        settings(Preset::Pixel, false),
        &[row, row].concat(),
        0,
    );
}

/// Two pixels stretched 2.5x: the 2x enlargement is grey, grey, white,
/// white, and only the middle column falls between grey and white texel
/// centres, at the halfway point (20.5 / 31 = 168.6). Plain bilinear would
/// blend three columns, and Pixel blends none.
#[test]
fn vanilla_plus_blends_only_the_column_across_an_edge() {
    let source = video(2, 1);
    let blend = [169, 169, 169, 255];
    let row = [GREY_RGBA, GREY_RGBA, blend, WHITE_RGBA, WHITE_RGBA];
    check_fixture(
        source,
        &[GREY, WHITE],
        (5, 3),
        settings(Preset::VanillaPlus, false),
        &[row, row, row].concat(),
        1,
    );
    let row = [GREY_RGBA, GREY_RGBA, WHITE_RGBA, WHITE_RGBA, WHITE_RGBA];
    check_fixture(
        source,
        &[GREY, WHITE],
        (5, 3),
        settings(Preset::Pixel, false),
        &[row, row, row].concat(),
        0,
    );
}

/// A 2x2 source in a 7x4 area scales 2x into columns 1 to 4, leaving one
/// black column on the left and two on the right.
#[test]
fn integer_scale_centres_the_picture_on_black() {
    let (r, g, b, w) = ([255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], WHITE_RGBA);
    let top = [BLACK, r, r, g, g, BLACK, BLACK];
    let bottom = [BLACK, b, b, w, w, BLACK, BLACK];
    for preset in [Preset::Pixel, Preset::VanillaPlus] {
        check_fixture(
            video(2, 2),
            &[0x001f, 0x03e0, 0x7c00, WHITE],
            (7, 4),
            settings(preset, true),
            &[top, top, bottom, bottom].concat(),
            0,
        );
    }
}

/// A GBA-sized frame with every kind of detail the presets must keep: random
/// colours with one-pixel features, a one-pixel checkerboard dither and
/// bit 15 set at random.
fn busy_frame() -> Vec<u16> {
    let mut state = 0x2545_f491_u32;
    (0..240 * 160)
        .map(|i| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let (x, y) = (i % 240, i / 240);
            if x < 48 && y < 32 {
                if (x + y) % 2 == 0 { WHITE } else { 0 }
            } else {
                (state >> 16) as u16
            }
        })
        .collect()
}

const SIZES: [(u32, u32); 7] =
    [(240, 160), (720, 480), (367, 245), (1000, 700), (1366, 768), (393, 852), (2560, 1600)];

/// Whole GBA frames at 1x, an exact multiple, fractional fits, wide and tall
/// areas and a HiDPI size: Pixel matches the reference exactly, and
/// VanillaPlus within one 8-bit step, which leaves room for f32 arithmetic.
#[test]
fn gpu_matches_reference_on_whole_frames() {
    let Some(gpu) = gpu() else { return };
    let frame = busy_frame();
    for preset in [Preset::Pixel, Preset::VanillaPlus] {
        let tolerance = u8::from(preset == Preset::VanillaPlus);
        for integer_scale in [false, true] {
            for drawable in SIZES {
                let settings = settings(preset, integer_scale);
                let drawn = gpu.render(video(240, 160), &frame, drawable, settings);
                let expected = reference::render(video(240, 160), &frame, drawable, settings);
                let what = format!("{preset:?} {drawable:?} integer {integer_scale}");
                assert_close(&drawn, &expected, drawable.0, tolerance, &what);
            }
        }
    }
}

/// At whole multiples sharp bilinear has nothing to blend, so VanillaPlus
/// draws the same bytes as Pixel.
#[test]
fn vanilla_plus_is_pixel_at_whole_multiples() {
    let Some(gpu) = gpu() else { return };
    let frame = busy_frame();
    for (drawable, integer_scale) in [((720, 480), false), ((1000, 700), true), ((393, 852), true)]
    {
        let pixel =
            gpu.render(video(240, 160), &frame, drawable, settings(Preset::Pixel, integer_scale));
        let vanilla = gpu.render(
            video(240, 160),
            &frame,
            drawable,
            settings(Preset::VanillaPlus, integer_scale),
        );
        assert_close(&vanilla, &pixel, drawable.0, 0, &format!("{drawable:?}"));
    }
}
