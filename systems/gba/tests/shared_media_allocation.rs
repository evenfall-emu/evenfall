//! A large ROM stays in the same allocation through reset and bundle import.
use evenfall_gba::machine::Machine;
use evenfall_gba::session::{Config, RewindConfig, Session};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};

const ROM_BYTES: usize = 32 * 1024 * 1024;
static MEASURING: AtomicBool = AtomicBool::new(false);
static TOTAL_BYTES: AtomicUsize = AtomicUsize::new(0);
static LARGEST_ALLOCATION: AtomicUsize = AtomicUsize::new(0);

struct CountingAllocator;

fn record(size: usize) {
    if MEASURING.load(Relaxed) {
        TOTAL_BYTES.fetch_add(size, Relaxed);
        LARGEST_ALLOCATION.fetch_max(size, Relaxed);
    }
}

// SAFETY: every operation delegates to System with the original pointer and
// layout; measurement never changes allocations or deallocations.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        // SAFETY: System receives the layout supplied by the caller.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        // SAFETY: System receives the layout supplied by the caller.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record(new_size);
        // SAFETY: System receives the original pointer/layout and requested size.
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: System receives the original pointer/layout.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn measured(f: impl FnOnce()) -> (usize, usize) {
    TOTAL_BYTES.store(0, Relaxed);
    LARGEST_ALLOCATION.store(0, Relaxed);
    MEASURING.store(true, Relaxed);
    f();
    MEASURING.store(false, Relaxed);
    (TOTAL_BYTES.load(Relaxed), LARGEST_ALLOCATION.load(Relaxed))
}

#[test]
fn large_rom_reset_and_multi_entry_bundle_import_do_not_allocate_media() {
    let mut rom = vec![0; ROM_BYTES];
    rom[..4].copy_from_slice(&[0xfe, 0xff, 0xff, 0xea]); // ARM: b .
    let address = rom.as_ptr();
    let mut session = Session::create_from_owned_buffer(
        Config {
            rewind: RewindConfig { interval_frames: 1, ..RewindConfig::default() },
            ..Config::default()
        },
        rom,
        ROM_BYTES,
    )
    .unwrap();
    assert_eq!(session.rom().as_ptr(), address);
    assert_eq!(session.rom().as_ptr(), session.machine.bus.rom().as_ptr());

    for _ in 0..4 {
        session.advance(session.machine.now() + Machine::CYCLES_PER_FRAME).unwrap();
    }
    let depth = session.rewind_depth();
    assert!(depth >= 3, "exercise multiple validated checkpoints");
    let cycle = session.machine.now();
    let bundle = session.write_rewind_bundle().unwrap();

    let (total, largest) = measured(|| session.reset_with_save(&[]).unwrap());
    assert_eq!(session.rom().as_ptr(), address);
    assert_eq!(session.rom().as_ptr(), session.machine.bus.rom().as_ptr());
    assert!(largest < ROM_BYTES / 4, "reset allocated {largest} bytes at once");
    assert!(total < 8 * 1024 * 1024, "reset allocated {total} bytes total");

    let (total, largest) = measured(|| session.load_rewind_bundle_owned(bundle).unwrap());
    assert_eq!(session.machine.now(), cycle);
    assert_eq!(session.rewind_depth(), depth);
    assert_eq!(session.rom().as_ptr(), session.machine.bus.rom().as_ptr());
    assert!(largest < ROM_BYTES / 4, "bundle import allocated {largest} bytes at once");
    assert!(total < 8 * 1024 * 1024, "bundle import allocated {total} bytes total");
}
