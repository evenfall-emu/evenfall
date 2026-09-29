//! GBA scanline rendering and transient caches.

mod backgrounds;
mod compose;
mod history;
// SIMD intrinsics: the one measured use of `unsafe` in the core.
#[allow(unsafe_code)]
mod lanes;
mod objects;
mod render;
pub(crate) use compose::LineLayers;
pub(crate) use history::AffineTransition;
pub mod scanline_memo;
pub mod tile_row_cache;
pub mod timing;

#[cfg(test)]
mod tests;
