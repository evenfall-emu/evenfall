//! The presets' definitions, evaluated in f64 one output pixel at a time.
//!
//! The tests check this against hand-derived fixtures and compare the GPU
//! with it on whole frames.

use evenfall_host::VideoFormat;

use crate::{Preset, Settings, Viewport};

/// The RGBA bytes [`crate::Renderer::draw`] should leave in a `drawable`
/// (width, height) target.
pub fn render(
    video: VideoFormat,
    pixels: &[u16],
    drawable: (u32, u32),
    settings: Settings,
) -> Vec<u8> {
    let viewport = Viewport::fit((video.width, video.height), drawable, settings.integer_scale);
    let mut out = [0, 0, 0, 255].repeat(drawable.0 as usize * drawable.1 as usize);
    let texel = |x: i64, y: i64| {
        let x = x.clamp(0, i64::from(video.width) - 1) as usize;
        let y = y.clamp(0, i64::from(video.height) - 1) as usize;
        decode(pixels[y * video.width as usize + x])
    };
    for y in 0..viewport.height {
        for x in 0..viewport.width {
            let rgb = match settings.preset {
                Preset::Pixel => texel(
                    nearest(x, viewport.width, video.width),
                    nearest(y, viewport.height, video.height),
                ),
                Preset::VanillaPlus => {
                    let (x0, x1, wx) = sharp_bilinear(x, viewport.width, video.width);
                    let (y0, y1, wy) = sharp_bilinear(y, viewport.height, video.height);
                    let top = mix(texel(x0, y0), texel(x1, y0), wx);
                    let bottom = mix(texel(x0, y1), texel(x1, y1), wx);
                    mix(top, bottom, wy)
                }
            };
            let at = ((viewport.y + y) * drawable.0 + viewport.x + x) as usize * 4;
            for (byte, channel) in out[at..at + 3].iter_mut().zip(rgb) {
                *byte = (channel * 255.0).round() as u8;
            }
        }
    }
    out
}

/// BGR555 channels as fractions of full scale.
fn decode(pixel: u16) -> [f64; 3] {
    [pixel, pixel >> 5, pixel >> 10].map(|five| f64::from(five & 31) / 31.0)
}

/// The source index under output pixel `p`'s centre, along an axis that
/// scales `source` pixels to `out`.
fn nearest(p: u32, out: u32, source: u32) -> i64 {
    ((f64::from(p) + 0.5) * f64::from(source) / f64::from(out)).floor() as i64
}

/// The two source indices and the weight of the second, along one axis.
///
/// The axis is first enlarged k times with nearest neighbour, k being the
/// largest whole multiple of the scale (at least 1). Output pixel `p`'s
/// centre then interpolates linearly between the enlarged texels' centres,
/// and enlarged texel `i` shows source pixel floor(i / k).
fn sharp_bilinear(p: u32, out: u32, source: u32) -> (i64, i64, f64) {
    let scale = f64::from(out) / f64::from(source);
    let k = scale.floor().max(1.0);
    let f = (f64::from(p) + 0.5) / scale * k - 0.5;
    let base = f.floor();
    let last = i64::from(source) * k as i64 - 1;
    let index = |i: i64| i.clamp(0, last) / k as i64;
    (index(base as i64), index(base as i64 + 1), f - base)
}

fn mix(a: [f64; 3], b: [f64; 3], weight: f64) -> [f64; 3] {
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * weight)
}
