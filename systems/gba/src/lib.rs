//! A deterministic, cycle-aware Game Boy Advance core.
//!
//! `Machine` is the host-facing hardware seam and `Bus` owns time.

/// Clones the listed fields of `$source` into the same fields of `$target`
/// with `clone_from`, which reuses the target's boxes and vectors. The
/// pattern names every field of `Self`: a field added to the type does not
/// compile until it is listed, either copied or among the `keep` fields.
macro_rules! clone_fields_from {
    ($target:expr, $source:expr, [$($field:ident),* $(,)?] $(, keep [$($kept:ident),* $(,)?])?) => {{
        let Self { $($field,)* $($($kept: _,)*)? } = $source;
        $($target.$field.clone_from($field);)*
    }};
}

/// `Clone` for a struct whose `clone_from` clones field by field, so copying
/// into an existing value keeps its allocations.
macro_rules! impl_clone_fields {
    ($type:ty { $($field:ident),* $(,)? }) => {
        impl Clone for $type {
            fn clone(&self) -> Self {
                let Self { $($field),* } = self;
                Self { $($field: $field.clone()),* }
            }

            fn clone_from(&mut self, source: &Self) {
                clone_fields_from!(self, source, [$($field),*]);
            }
        }
    };
}

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
pub mod machine;
pub mod session;
pub mod video_recovery;
