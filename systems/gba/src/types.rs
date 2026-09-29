//! Fundamental hardware-domain types shared by core modules.

pub type Cycle = u64;

/// One complete GBA video frame: 228 scanlines at 1232 master cycles each.
pub const CYCLES_PER_FRAME: Cycle = 1232 * 228;
