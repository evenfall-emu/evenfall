//! Native ZiGBA trace writer. Build with tools/parity/oracle/build.zig.
const std = @import("std");
const zigba = @import("zigba");
const schema = zigba.parity_schema;

const Input = struct {
    at: []const u8,
    change: struct {
        keys: ?u16 = null,
        solar_level: ?u8 = null,
        gyro_sample: ?u16 = null,
        tilt: ?[2]u16 = null,
    },
};
const HostAction = struct {
    at: []const u8,
    command: struct { joybus_write: ?u32 = null, joybus_read: ?bool = null },
};
const Manifest = struct {
    name: []const u8,
    assets: struct { rom: []const u8, bios: ?[]const u8 = null, save: ?[]const u8 = null },
    config: struct {
        boot: []const u8,
        saveType: []const u8,
        rtcEpoch: ?u64 = null,
        solarLevel: ?u8 = null,
        gyroSample: ?u16 = null,
        tilt: ?[2]u16 = null,
        rumble: bool = false,
    },
    inputs: []const Input = &.{},
    hostActions: []const HostAction = &.{},
    endCycle: []const u8,
    checkpoints: []const []const u8,
    sliceCycles: []const []const u8,
    terminalPc: ?u32 = null,
    terminalR12: ?u32 = null,
};

pub fn main(init: std.process.Init) !void {
    const a = init.arena.allocator();
    const args = try init.minimal.args.toSlice(a);
    if (args.len == 3 and std.mem.eql(u8, args[1], "schema")) {
        const bytes = try schema.bytes(a);
        try writeFile(init.io, args[2], bytes);
        var digest: [32]u8 = undefined;
        std.crypto.hash.Blake3.hash(bytes, &digest, .{});
        std.debug.print("schema v6 BLAKE3: {s}\n", .{std.fmt.bytesToHex(digest, .lower)});
        return;
    }
    if (args.len != 4) return error.Usage; // SCENARIO SLICE_INDEX OUTPUT
    const manifest_bytes = try std.Io.Dir.cwd().readFileAlloc(init.io, args[1], a, .limited(2 * 1024 * 1024));
    const parsed = try std.json.parseFromSlice(Manifest, a, manifest_bytes, .{ .ignore_unknown_fields = false });
    const m = parsed.value;
    const schedule = try std.fmt.parseInt(usize, args[2], 10);
    if (schedule >= m.sliceCycles.len or m.checkpoints.len == 0 or m.checkpoints.len > 10_000) return error.InvalidScenario;
    const slice = try std.fmt.parseInt(u64, m.sliceCycles[schedule], 10);
    const end = try std.fmt.parseInt(u64, m.endCycle, 10);
    if (slice == 0 or try std.fmt.parseInt(u64, m.checkpoints[m.checkpoints.len - 1], 10) != end) return error.InvalidScenario;
    const parent = std.fs.path.dirname(args[1]) orelse ".";
    const rom = try asset(init.io, a, parent, m.assets.rom, 32 * 1024 * 1024);
    const bios = if (m.assets.bios) |path| try asset(init.io, a, parent, path, 16 * 1024) else &.{};
    const save = if (m.assets.save) |path| try asset(init.io, a, parent, path, 128 * 1024) else null;
    const boot = std.meta.stringToEnum(zigba.BootMode, m.config.boot) orelse return error.InvalidScenario;
    if ((boot == .hle and bios.len != 0) or (boot != .hle and bios.len != 16 * 1024)) return error.InvalidBios;
    const save_type = if (std.mem.eql(u8, m.config.saveType, "flash64k")) zigba.SaveType.flash_64k else if (std.mem.eql(u8, m.config.saveType, "flash64k_atmel")) zigba.SaveType.flash_64k_atmel else if (std.mem.eql(u8, m.config.saveType, "flash128k")) zigba.SaveType.flash_128k else if (std.mem.eql(u8, m.config.saveType, "eeprom512")) zigba.SaveType.eeprom_512 else if (std.mem.eql(u8, m.config.saveType, "eeprom8k")) zigba.SaveType.eeprom_8k else std.meta.stringToEnum(zigba.SaveType, m.config.saveType) orelse return error.InvalidScenario;
    if (save) |data| {
        const required: usize = switch (save_type) {
            .auto => return error.SaveTypeRequired,
            .none => 0,
            .sram => 32 * 1024,
            .flash_64k, .flash_64k_atmel => 64 * 1024,
            .flash_128k => 128 * 1024,
            .eeprom_512 => 512,
            .eeprom_8k => 8 * 1024,
        };
        if (data.len != required) return error.InvalidSaveSize;
    }
    if (m.config.gyroSample != null and m.config.gyroSample.? > 4095) return error.InvalidScenario;
    if (m.config.tilt) |tilt| {
        if (tilt[0] > 4095 or tilt[1] > 4095) return error.InvalidScenario;
    }
    const machine = try init.gpa.create(zigba.Machine);
    defer init.gpa.destroy(machine);
    machine.initInPlace(.{
        .rom = rom,
        .bios = bios,
        .immutable_media = true,
        .boot = boot,
        .save_type = save_type,
        .save_data = save,
        .rtc_epoch_seconds = m.config.rtcEpoch,
        .solar_level = m.config.solarLevel,
        .gyro_sample = if (m.config.gyroSample) |v| @intCast(v) else null,
        .tilt_sample = if (m.config.tilt) |v| .{ .x = @intCast(v[0]), .y = @intCast(v[1]) } else null,
        .rumble = m.config.rumble,
    });
    const inputs = try a.alloc(zigba.TimedInput, m.inputs.len);
    var last_input: u64 = 0;
    for (m.inputs, inputs) |item, *out| {
        const at = try std.fmt.parseInt(u64, item.at, 10);
        if (at < last_input or at > end) return error.InvalidScenario;
        last_input = at;
        const c = item.change;
        const change: zigba.TimedInput.Change = if (c.keys) |v| blk: {
            if (v > 1023) return error.InvalidScenario;
            break :blk .{ .keys = @bitCast(@as(u10, @intCast(v))) };
        } else if (c.solar_level) |v| .{ .solar_level = v } else if (c.gyro_sample) |v| blk: {
            if (v > 4095) return error.InvalidScenario;
            break :blk .{ .gyro_sample = @intCast(v) };
        } else if (c.tilt) |v| blk: {
            if (v[0] > 4095 or v[1] > 4095) return error.InvalidScenario;
            break :blk .{ .tilt = .{ .x = @intCast(v[0]), .y = @intCast(v[1]) } };
        } else return error.InvalidScenario;
        out.* = .{ .at = at, .change = change };
    }

    var file = try std.Io.Dir.cwd().createFile(init.io, args[3], .{ .truncate = true });
    defer file.close(init.io);
    var buffer: [65536]u8 = undefined;
    var stream = file.writerStreaming(init.io, &buffer);
    const w = &stream.interface;
    defer w.flush() catch {};
    try w.writeAll("EFPAR01\x00");
    try number(w, u32, @intCast(m.checkpoints.len));
    var pending: []const zigba.TimedInput = inputs;
    var next_action: usize = 0;
    var previous: u64 = 0;
    var audio: std.ArrayList(u8) = .empty;
    var samples: [4096]zigba.StereoSample = undefined;
    for (m.checkpoints) |requested| {
        const checkpoint = try std.fmt.parseInt(u64, requested, 10);
        if (checkpoint <= previous or checkpoint > end) return error.InvalidScenario;
        previous = checkpoint;
        while (next_action < m.hostActions.len and try std.fmt.parseInt(u64, m.hostActions[next_action].at, 10) <= machine.now()) : (next_action += 1) {
            try hostAction(machine, m.hostActions[next_action]);
        }
        while (machine.now() < checkpoint or (pending.len > 0 and pending[0].at <= checkpoint) or (next_action < m.hostActions.len and try std.fmt.parseInt(u64, m.hostActions[next_action].at, 10) <= checkpoint)) {
            const before = machine.now();
            const action_at = if (next_action < m.hostActions.len) try std.fmt.parseInt(u64, m.hostActions[next_action].at, 10) else std.math.maxInt(u64);
            const target = @max(before, @min(@min(before +| slice, checkpoint), action_at));
            const result = try machine.runSliceWithInput(target, checkpoint, pending);
            pending = pending[result.inputs_consumed..];
            switch (result.reason) {
                .faulted => return error.GuestFaulted,
                .power_stopped => return error.PowerStopped,
                else => {},
            }
            while (true) {
                const count = machine.drainAudioSamples(&samples);
                for (samples[0..count]) |s| {
                    var pair: [4]u8 = undefined;
                    std.mem.writeInt(u16, pair[0..2], @bitCast(s.left), .little);
                    std.mem.writeInt(u16, pair[2..4], @bitCast(s.right), .little);
                    try audio.appendSlice(a, &pair);
                }
                if (count < samples.len) break;
            }
            while (next_action < m.hostActions.len and try std.fmt.parseInt(u64, m.hostActions[next_action].at, 10) <= machine.now() and try std.fmt.parseInt(u64, m.hostActions[next_action].at, 10) <= checkpoint) : (next_action += 1) {
                try hostAction(machine, m.hostActions[next_action]);
            }
            if (machine.now() == before and result.inputs_consumed == 0 and target > before) return error.NoProgress;
        }
        try number(w, u64, checkpoint);
        try number(w, u64, machine.now());
        try number(w, u64, machine.completedFrames());
        const cpu = machine.inspectCpu();
        if (checkpoint == end) {
            if (m.terminalPc) |pc| {
                if (cpu.next_instruction_address != pc) {
                    std.debug.print("terminal PC: {x}, expected {x}\n", .{ cpu.next_instruction_address, pc });
                    return error.TerminalPcMismatch;
                }
            }
            if (m.terminalR12) |r12| {
                if (cpu.registers[12] != r12) {
                    std.debug.print("terminal r12: {x}, expected {x}\n", .{ cpu.registers[12], r12 });
                    return error.TerminalReportMismatch;
                }
            }
        }
        for (cpu.registers) |register| try number(w, u32, register);
        try number(w, u32, cpu.next_instruction_address);
        try number(w, u32, cpu.cpsr);
        try number(w, u32, machine.audioSampleRate());
        const frame = try a.alloc(u8, 240 * 160 * 2);
        for (machine.framebuffer(), 0..) |pixel, i| std.mem.writeInt(u16, frame[i * 2 ..][0..2], pixel, .little);
        try blob(w, frame);
        try blob(w, audio.items);
        audio.clearRetainingCapacity();
        try blob(w, machine.saveData());
        const state = try machine.writeStateAlloc(a);
        try blob(w, state);
        a.free(state);
    }
}

fn hostAction(machine: *zigba.Machine, action: HostAction) !void {
    const command: zigba.JoyBusCommand = if (action.command.joybus_write) |payload| .{ .write_to_gba = payload } else if (action.command.joybus_read == true) .read_from_gba else return error.InvalidHostAction;
    try machine.bus.scheduleJoyBus(command, machine.now() + 1000);
}

fn asset(io: std.Io, allocator: std.mem.Allocator, parent: []const u8, path: []const u8, limit: usize) ![]u8 {
    const full = try std.fs.path.join(allocator, &.{ parent, path });
    // Zig's .limited(n) rejects files whose size is *equal* to n.
    return std.Io.Dir.cwd().readFileAlloc(io, full, allocator, .limited(limit + 1));
}
fn writeFile(io: std.Io, path: []const u8, bytes: []const u8) !void {
    var file = try std.Io.Dir.cwd().createFile(io, path, .{ .truncate = true });
    defer file.close(io);
    var storage: [4096]u8 = undefined;
    var stream = file.writerStreaming(io, &storage);
    try stream.interface.writeAll(bytes);
    try stream.interface.flush();
}
fn number(w: *std.Io.Writer, comptime T: type, value: T) !void {
    var bytes: [@sizeOf(T)]u8 = undefined;
    std.mem.writeInt(T, &bytes, value, .little);
    try w.writeAll(&bytes);
}
fn blob(w: *std.Io.Writer, bytes: []const u8) !void {
    try number(w, u32, @intCast(bytes.len));
    var digest: [32]u8 = undefined;
    std.crypto.hash.Blake3.hash(bytes, &digest, .{});
    try w.writeAll(&digest);
    try w.writeAll(bytes);
}
