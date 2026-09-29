//! GBA scanline rendering and transient caches.

mod backgrounds;
mod history;
mod objects;
mod render;
pub(crate) use history::AffineTransition;
pub mod scanline_memo;
pub mod tile_row_cache;
pub mod timing;

#[cfg(test)]
mod audit_tests;
#[cfg(test)]
mod tests;
