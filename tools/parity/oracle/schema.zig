//! Copied beside ZiGBA's unmodified src/ in target/parity/zigba/src. These
//! are exactly the writeSchema calls in Machine.stateSchemaDigest (v6).
const std = @import("std");
const codec = @import("state_codec.zig");
const Cpu = @import("cpu.zig").Cpu;
const Bus = @import("bus.zig").Bus;
const Cycle = @import("types.zig").Cycle;

pub fn bytes(allocator: std.mem.Allocator) ![]u8 {
    var output = std.Io.Writer.Allocating.init(allocator);
    defer output.deinit();
    var sink: codec.Sink = .{ .writer = &output.writer };
    try codec.writeSchemaName(&sink, "zigba.state-codec.le-minimal.v1");
    try codec.writeSchemaName(&sink, "zigba.machine.payload.v6");
    try codec.writeSchema(&sink, Cycle);
    try Cpu.writeStateSchema(&sink);
    try Bus.writeStateSchema(&sink);
    return output.toOwnedSlice();
}
