const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});
    // The Rust CLI copies pinned src to this scratch tree and places
    // schema.zig alongside it; no source in refs/zigba is changed.
    const zigba = b.createModule(.{ .root_source_file = .{ .cwd_relative = b.pathFromRoot("../../../target/parity/zigba/src/root.zig") }, .target = target, .optimize = optimize });
    const exe = b.addExecutable(.{
        .name = "evenfall-zigba-oracle",
        .root_module = b.createModule(.{ .root_source_file = b.path("main.zig"), .target = target, .optimize = optimize, .imports = &.{
            .{ .name = "zigba", .module = zigba },
        } }),
    });
    b.installArtifact(exe);
}
