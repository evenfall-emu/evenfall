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
const Assets = struct { rom: []const u8, bios: ?[]const u8 = null, save: ?[]const u8 = null };
const Config = struct {
    boot: []const u8,
    saveType: []const u8,
    rtcEpoch: ?u64 = null,
    solarLevel: ?u8 = null,
    gyroSample: ?u16 = null,
    tilt: ?[2]u16 = null,
    rumble: bool = false,
    audioOutput: bool = true,
};
const Peer = struct {
    assets: Assets,
    config: Config,
    terminalPc: ?u32 = null,
    terminalR12: ?u32 = null,
};
const Link = struct { connection: []const u8, peers: []const Peer };
const Manifest = struct {
    name: []const u8,
    assets: Assets,
    config: Config,
    link: ?Link = null,
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
    if (m.link) |link| {
        if (m.inputs.len != 0 or m.hostActions.len != 0) return error.InvalidScenario;
        return linkedRun(init, a, parent, m, link, slice, end, args[3]);
    }
    const machine = try createMachine(init, a, parent, m.assets, m.config);
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
        const cpu = machine.inspectCpu();
        if (checkpoint == end) try terminal(cpu, m.terminalPc, m.terminalR12);
        try writeObservation(a, w, machine, checkpoint, audio.items);
        audio.clearRetainingCapacity();
    }
}

fn createMachine(init: std.process.Init, a: std.mem.Allocator, parent: []const u8, assets: Assets, config: Config) !*zigba.Machine {
    const rom = try asset(init.io, a, parent, assets.rom, 32 * 1024 * 1024);
    const bios = if (assets.bios) |path| try asset(init.io, a, parent, path, 16 * 1024) else &.{};
    const save = if (assets.save) |path| try asset(init.io, a, parent, path, 128 * 1024) else null;
    const boot = std.meta.stringToEnum(zigba.BootMode, config.boot) orelse return error.InvalidScenario;
    if ((boot == .hle and bios.len != 0) or (boot != .hle and bios.len != 16 * 1024)) return error.InvalidBios;
    const save_type = if (std.mem.eql(u8, config.saveType, "flash64k")) zigba.SaveType.flash_64k else if (std.mem.eql(u8, config.saveType, "flash64k_atmel")) zigba.SaveType.flash_64k_atmel else if (std.mem.eql(u8, config.saveType, "flash128k")) zigba.SaveType.flash_128k else if (std.mem.eql(u8, config.saveType, "eeprom512")) zigba.SaveType.eeprom_512 else if (std.mem.eql(u8, config.saveType, "eeprom8k")) zigba.SaveType.eeprom_8k else std.meta.stringToEnum(zigba.SaveType, config.saveType) orelse return error.InvalidScenario;
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
    if (config.gyroSample != null and config.gyroSample.? > 4095) return error.InvalidScenario;
    if (config.tilt) |tilt| {
        if (tilt[0] > 4095 or tilt[1] > 4095) return error.InvalidScenario;
    }
    const machine = try a.create(zigba.Machine);
    machine.initInPlace(.{
        .rom = rom,
        .bios = bios,
        .immutable_media = true,
        .boot = boot,
        .save_type = save_type,
        .save_data = save,
        .rtc_epoch_seconds = config.rtcEpoch,
        .solar_level = config.solarLevel,
        .gyro_sample = if (config.gyroSample) |v| @intCast(v) else null,
        .tilt_sample = if (config.tilt) |v| .{ .x = @intCast(v[0]), .y = @intCast(v[1]) } else null,
        .rumble = config.rumble,
    });
    machine.setAudioOutput(config.audioOutput);
    return machine;
}

fn terminal(cpu: anytype, pc: ?u32, r12: ?u32) !void {
    if (pc) |want| {
        if (cpu.next_instruction_address != want) {
            std.debug.print("terminal PC: {x}, expected {x}\n", .{ cpu.next_instruction_address, want });
            return error.TerminalPcMismatch;
        }
    }
    if (r12) |want| {
        if (cpu.registers[12] != want) {
            std.debug.print("terminal r12: {x}, expected {x}\n", .{ cpu.registers[12], want });
            return error.TerminalReportMismatch;
        }
    }
}

fn writeObservation(a: std.mem.Allocator, w: *std.Io.Writer, machine: *zigba.Machine, checkpoint: u64, audio: []const u8) !void {
    try number(w, u64, checkpoint);
    try number(w, u64, machine.now());
    try number(w, u64, machine.completedFrames());
    const cpu = machine.inspectCpu();
    for (cpu.registers) |register| try number(w, u32, register);
    try number(w, u32, cpu.next_instruction_address);
    try number(w, u32, cpu.cpsr);
    try number(w, u32, machine.audioSampleRate());
    const frame = try a.alloc(u8, 240 * 160 * 2);
    defer a.free(frame);
    for (machine.framebuffer(), 0..) |pixel, i| std.mem.writeInt(u16, frame[i * 2 ..][0..2], pixel, .little);
    try blob(w, frame);
    try blob(w, audio);
    try blob(w, machine.saveData());
    const state = try machine.writeStateAlloc(a);
    defer a.free(state);
    try blob(w, state);
}

fn linkedRun(init: std.process.Init, a: std.mem.Allocator, parent: []const u8, m: Manifest, link: Link, slice: u64, end: u64, output: []const u8) !void {
    const count = link.peers.len + 1;
    if (count < 2 or count > 4) return error.InvalidScenario;
    const connection = std.meta.stringToEnum(zigba.SioConnection, link.connection) orelse return error.InvalidScenario;
    if (connection == .cable_2p and count != 2) return error.InvalidScenario;
    const machines = try a.alloc(*zigba.Machine, count);
    machines[0] = try createMachine(init, a, parent, m.assets, m.config);
    for (link.peers, 1..) |peer, index| machines[index] = try createMachine(init, a, parent, peer.assets, peer.config);

    // One standard EFPAR01 file per machine. File names encode cable order;
    // no version change or machine-agnostic trace interpretation is needed.
    const stem = if (std.mem.endsWith(u8, output, ".trace")) output[0 .. output.len - 6] else output;
    const files = try a.alloc(std.Io.File, count);
    const buffers = try a.alloc([65536]u8, count);
    const streams = try a.alloc(std.Io.File.Writer, count);
    const audios = try a.alloc(std.ArrayList(u8), count);
    for (0..count) |index| {
        const path = try std.fmt.allocPrint(a, "{s}-machine-{d}.trace", .{ stem, index });
        files[index] = try std.Io.Dir.cwd().createFile(init.io, path, .{ .truncate = true });
        streams[index] = files[index].writerStreaming(init.io, &buffers[index]);
        try streams[index].interface.writeAll("EFPAR01\x00");
        try number(&streams[index].interface, u32, @intCast(m.checkpoints.len));
        audios[index] = .empty;
    }
    defer for (files) |file| file.close(init.io);

    var samples: [4096]zigba.StereoSample = undefined;
    var previous: u64 = 0;
    for (m.checkpoints) |requested| {
        const checkpoint = try std.fmt.parseInt(u64, requested, 10);
        if (checkpoint <= previous or checkpoint > end) return error.InvalidScenario;
        previous = checkpoint;
        while (true) {
            var earliest: u64 = std.math.maxInt(u64);
            for (machines) |machine| earliest = @min(earliest, machine.now());
            if (earliest >= checkpoint) break;
            const target = @max(earliest, @min(earliest +| slice, checkpoint));
            const result = try zigba.runLinkedUntil(machines, target, connection);
            switch (result.reason) {
                .deadline => {},
                .power_stopped => return error.PowerStopped,
                .faulted => |fault| {
                    std.debug.print("machine {d} faulted at {x}\n", .{ fault.machine_index, fault.fault.instruction_address });
                    return error.GuestFaulted;
                },
            }
            for (machines, audios) |machine, *audio| {
                while (true) {
                    const n = machine.drainAudioSamples(&samples);
                    for (samples[0..n]) |s| {
                        var pair: [4]u8 = undefined;
                        std.mem.writeInt(u16, pair[0..2], @bitCast(s.left), .little);
                        std.mem.writeInt(u16, pair[2..4], @bitCast(s.right), .little);
                        try audio.appendSlice(a, &pair);
                    }
                    if (n < samples.len) break;
                }
            }
            if (result.earliest_cycle == earliest and target > earliest) return error.NoProgress;
        }
        for (machines, audios, streams, 0..) |machine, *audio, *stream, index| {
            if (checkpoint == end) {
                const pc = if (index == 0) m.terminalPc else link.peers[index - 1].terminalPc;
                const r12 = if (index == 0) m.terminalR12 else link.peers[index - 1].terminalR12;
                terminal(machine.inspectCpu(), pc, r12) catch |err| {
                    std.debug.print("machine {d}: ", .{index});
                    return err;
                };
            }
            try writeObservation(a, &stream.interface, machine, checkpoint, audio.items);
            audio.clearRetainingCapacity();
        }
    }
    for (streams) |*stream| try stream.interface.flush();
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
