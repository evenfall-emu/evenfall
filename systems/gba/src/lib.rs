//! A deterministic, cycle-aware Game Boy Advance core, ported from ZiGBA.
//!
//! `Machine` is the host-facing hardware seam and `Bus` owns time.

// Module declarations, grouped by the part of the port that owns them. Add a
// module directly under its group's marker so parallel work merges cleanly.

// [devices]

// [apu]

// [video]

// [bus]

// [cpu]
pub mod cpu;

// [hle]

// [machine]
pub mod machine;
