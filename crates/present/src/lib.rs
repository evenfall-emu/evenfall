//! Host-side presentation of a system's completed frames.
//!
//! A system hands its host raw frames at native resolution
//! ([`evenfall_host::VideoFormat`]). This crate draws them into a host's GPU
//! texture with [`wgpu`]: it decodes the pixel format, fits the picture to the
//! drawable area at the source's aspect ratio, and scales it with a [`Preset`].
//! It only reads frames, so presentation cannot change emulation, recordings
//! or save states.
//!
//! Every preset filters with shader arithmetic on loaded texels instead of the
//! GPU's texture sampler. Vulkan guarantees samplers only 4 bits of sub-texel
//! precision, so sampled results differ between GPUs, while shader arithmetic
//! stays within one 8-bit step of the f64 definition the tests compare with.
//!
//! The crate enables no wgpu backend. Each host build enables the ones it
//! ships: `webgpu` in a browser, Vulkan, Metal, DX12 or GLES natively.

mod renderer;
mod viewport;

#[cfg(test)]
mod reference;
#[cfg(test)]
mod tests;

pub use renderer::Renderer;
pub use viewport::Viewport;

/// How source pixels become output pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Preset {
    /// Nearest neighbour: every output pixel shows exactly one source pixel.
    /// At fractional scales some source pixels come out one output pixel
    /// wider or taller than their neighbours.
    #[default]
    Pixel,
    /// Sharp bilinear: nearest neighbour up to the largest whole multiple
    /// that fits, then bilinear to the final size. At whole multiples it is
    /// identical to [`Preset::Pixel`]. Between them every source pixel keeps
    /// the same size, and blending stays within 1/k of a source pixel of each
    /// edge, k being that multiple: at most two output pixels.
    VanillaPlus,
}

/// What to draw and how to place it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub preset: Preset,
    /// Scale only by whole multiples and leave a border around the picture.
    /// A drawable area smaller than the source falls back to fitting.
    pub integer_scale: bool,
}
