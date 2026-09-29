//! A deterministic, cycle-aware Game Boy Advance core, ported from ZiGBA.
//!
//! `Machine` is the host-facing hardware seam and `Bus` owns time.

// Module declarations, grouped by the part of the port that owns them. Add a
// module directly under its group's marker so parallel work merges cleanly.

// [devices]

// [apu]

// [video]
pub mod video;

// [bus]
pub mod bus;

// [cpu]
pub mod cpu;

// [hle]
pub mod hle;

// [machine]
