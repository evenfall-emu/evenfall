//! Regenerate the companion fixture with Zig 0.16 from the repository root:
//! zig test --dep session --dep zigba -Mroot=crates/state/tests/fixtures/generate-zigba-rewind-bundle.zig --dep zigba -Msession=refs/zigba/src/session.zig -Mzigba=refs/zigba/src/root.zig --test-filter 'write a real ZiGBA v6 session rewind bundle'
const std = @import("std");
const zigba = @import("zigba");
const Session = @import("session").Session;

test "write a real ZiGBA v6 session rewind bundle" {
    const session = try Session.create(std.testing.allocator, .{
        // ARM branch-to-self, a synthetic four-byte ROM.
        .image = &.{ 0xfe, 0xff, 0xff, 0xea },
        .rewind = .{ .interval_frames = 1 },
    });
    defer session.destroy();
    session.machine.bus.dispcnt = 0;
    for (0..3) |_| _ = try session.advance(session.machine.now() + zigba.cycles_per_frame);
    try std.testing.expectEqual(@as(usize, 3), session.rewindDepth());
    const bytes = try session.writeRewindBundle();
    defer std.testing.allocator.free(bytes);
    const live = try session.writeState();
    defer std.testing.allocator.free(live);
    var digest: [32]u8 = undefined;
    std.crypto.hash.Blake3.hash(live, &digest, .{});
    std.debug.print("live: {s}\n", .{std.fmt.bytesToHex(digest, .lower)});
    for (0..session.rewindDepth()) |index| {
        const state = try session.rewind_history.decodeAlloc(std.testing.allocator, index);
        defer std.testing.allocator.free(state);
        std.crypto.hash.Blake3.hash(state, &digest, .{});
        std.debug.print("checkpoint {d}: {s}\n", .{ index, std.fmt.bytesToHex(digest, .lower) });
    }
    try std.Io.Dir.cwd().writeFile(std.testing.io, .{
        .sub_path = "crates/state/tests/fixtures/zigba-rewind-bundle.bin",
        .data = bytes,
    });
}
