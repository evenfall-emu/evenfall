//! A deterministic, cycle-aware Game Boy Advance core, ported from ZiGBA.
//!
//! `Machine` is the host-facing hardware seam and `Bus` owns time.

// Module declarations, grouped by the part of the port that owns them. Add a
// module directly under its group's marker so parallel work merges cleanly.

// [devices]
pub mod cartridge;
pub mod dma;
#[cfg(not(target_arch = "wasm32"))]
pub mod host;
pub mod multiboot;
pub mod rom_image;
pub mod timer;
pub mod types;

// [apu]
pub mod apu;

// [video]
pub mod video;

// [bus]
pub mod bus;

// [cpu]
pub mod cpu;

// [hle]
pub mod hle;

// [machine]
