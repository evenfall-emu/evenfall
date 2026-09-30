import { CHEAT_FORMATS, CYCLES_PER_FRAME, SAVE_TYPES } from "../core.js";
import { FailureRecorder } from "./capture.js";

const U64 = 0xffffffffffffffffn;
// Input assets are identified by SHA-1, as in No-Intro DATs; observed outputs
// (frames, audio, saves, states, memory) are SHA-256 digests.
const hashPattern = /^[a-f0-9]{64}$/;
const assetPattern = /^[a-f0-9]{40}$/;
const reasons = [
  "deadline",
  "instruction_boundary_after_deadline",
  "power_stopped",
  "faulted",
];
function object(x, keys, label) {
  if (!x || typeof x !== "object" || Array.isArray(x))
    throw new TypeError(`${label} must be an object`);
  for (const k of Object.keys(x))
    if (!keys.includes(k)) throw new Error(`Unknown ${label}.${k}`);
}
function integer(x, max, label) {
  if (!Number.isInteger(x) || x < 0 || x > max)
    throw new RangeError(`Invalid ${label}`);
}
function cycle(x, label) {
  if (typeof x !== "string" || !/^(0|[1-9][0-9]*)$/.test(x) || BigInt(x) > U64)
    throw new RangeError(`Invalid ${label}: expected decimal u64 string`);
  return BigInt(x);
}
function hash(x, label) {
  if (!hashPattern.test(x)) throw new Error(`Invalid ${label} SHA-256`);
}
export function jsonSafe(x) {
  return JSON.parse(
    JSON.stringify(x, (_, v) =>
      typeof v === "bigint"
        ? v.toString()
        : ArrayBuffer.isView(v)
          ? Array.from(v)
          : v,
    ),
  );
}
async function digest(algorithm, bytes) {
  const digest = await globalThis.crypto.subtle.digest(algorithm, bytes);
  return Array.from(new Uint8Array(digest), (b) =>
    b.toString(16).padStart(2, "0"),
  ).join("");
}
export const sha256 = (bytes) => digest("SHA-256", bytes);
export const sha1 = (bytes) => digest("SHA-1", bytes);
export function validateScenario(m) {
  object(
    m,
    [
      "version",
      "name",
      "assets",
      "config",
      "inputs",
      "cheats",
      "checkpoints",
      "limits",
    ],
    "scenario",
  );
  if (m.version !== 2) throw new Error("Unsupported scenario version");
  if (typeof m.name !== "string" || !m.name.length)
    throw new Error("Scenario name required");
  object(m.assets, ["rom", "bios", "save", "state"], "assets");
  if (!m.assets.rom) throw new Error("ROM asset required");
  for (const [name, asset] of Object.entries(m.assets)) {
    object(asset, ["path", "sha1"], `assets.${name}`);
    if (!assetPattern.test(asset.sha1))
      throw new Error(`Invalid ${name} SHA-1`);
    if (
      asset.path !== undefined &&
      (typeof asset.path !== "string" || !asset.path)
    )
      throw new Error("Invalid asset path");
  }
  const c = m.config;
  object(c, ["boot", "saveType", "rtc", "rtcEpoch", "sampleRate"], "config");
  if (
    !["hle", "bios"].includes(c.boot) ||
    (c.boot === "bios") !== !!m.assets.bios
  )
    throw new Error("Boot mode and BIOS asset disagree");
  if (!SAVE_TYPES.includes(c.saveType) || typeof c.rtc !== "boolean")
    throw new Error("Explicit saveType and RTC policy required");
  cycle(c.rtcEpoch, "rtcEpoch");
  integer(c.sampleRate, 192000, "sampleRate");
  if (c.sampleRate < 8000) throw new Error("sampleRate must be at least 8000");
  if (BigInt(c.rtcEpoch) > 253402300799n)
    throw new Error("rtcEpoch exceeds supported range");
  object(
    m.limits,
    ["maxCycles", "maxRuns", "chunkCycles", "maxAudioBytes"],
    "limits",
  );
  const end = cycle(m.limits.maxCycles, "maxCycles");
  const chunk = cycle(m.limits.chunkCycles, "chunkCycles");
  if (!chunk || chunk > CYCLES_PER_FRAME / 4n)
    throw new Error("chunkCycles must be 1..70224 to drain audio safely");
  integer(m.limits.maxRuns, 1000000, "maxRuns");
  if (!m.limits.maxRuns) throw new Error("maxRuns must be positive");
  integer(m.limits.maxAudioBytes, 256 * 1024 * 1024, "maxAudioBytes");
  if (!Array.isArray(m.inputs) || m.inputs.length > 1024)
    throw new Error("At most 1024 inputs");
  let prev = 0n;
  for (const e of m.inputs) {
    object(e, ["at", "keys"], "input");
    const at = cycle(e.at, "input.at");
    if (at < prev || at > end)
      throw new Error("Inputs must be ordered and within maxCycles");
    integer(e.keys, 1023, "keys");
    prev = at;
  }
  if (
    m.cheats !== undefined &&
    (!Array.isArray(m.cheats) || m.cheats.length > 1024)
  )
    throw new Error("At most 1024 cheat events");
  prev = 0n;
  for (const e of m.cheats ?? []) {
    object(e, ["at", "cheats"], "cheat event");
    const at = cycle(e.at, "cheat event at");
    if (at < prev || at > end)
      throw new Error("Cheat events must be ordered and within maxCycles");
    prev = at;
    if (!Array.isArray(e.cheats) || e.cheats.length > 256)
      throw new Error("A cheat event holds at most 256 cheats");
    for (const c of e.cheats) {
      object(c, ["code", "format"], "cheat");
      if (typeof c.code !== "string" || !c.code)
        throw new Error("A cheat needs a code string");
      if (
        c.format !== undefined &&
        c.format !== "auto" &&
        !CHEAT_FORMATS.includes(c.format)
      )
        throw new Error("Invalid cheat format");
    }
  }
  if (
    !Array.isArray(m.checkpoints) ||
    !m.checkpoints.length ||
    m.checkpoints.length > 10000
  )
    throw new Error("Require 1..10000 checkpoints");
  prev = 0n;
  const names = new Set();
  for (const p of m.checkpoints) {
    object(p, ["name", "at", "expect", "memory"], "checkpoint");
    const at = cycle(p.at, "checkpoint.at");
    if (at < prev || at > end)
      throw new Error("Checkpoints must be ordered and within maxCycles");
    prev = at;
    if (typeof p.name !== "string" || !p.name || names.has(p.name))
      throw new Error("Checkpoint names must be unique");
    names.add(p.name);
    if (p.expect !== undefined) {
      object(
        p.expect,
        [
          "cycle",
          "reason",
          "cpu",
          "frameSha256",
          "audioSha256",
          "saveSha256",
          "stateSha256",
        ],
        "expect",
      );
      for (const [k, v] of Object.entries(p.expect)) {
        if (k.endsWith("Sha256")) hash(v, k);
        else if (k === "cycle") cycle(v, k);
        else if (k === "reason" && !reasons.includes(v))
          throw new Error("Invalid expected reason");
        else if (k === "cpu") {
          object(v, ["registers", "cpsr", "nextInstructionAddress"], "cpu");
          if (v.registers !== undefined) {
            object(
              v.registers,
              Array.from({ length: 16 }, (_, i) => String(i)),
              "registers",
            );
            for (const n of Object.values(v.registers))
              integer(n, 0xffffffff, "register");
          }
          for (const field of ["cpsr", "nextInstructionAddress"])
            if (v[field] !== undefined) integer(v[field], 0xffffffff, field);
        }
      }
    }
    if (p.memory !== undefined) {
      if (!Array.isArray(p.memory) || p.memory.length > 64)
        throw new Error("At most 64 memory regions");
      const ids = new Set();
      for (const r of p.memory) {
        object(r, ["name", "address", "length", "sha256"], "memory");
        if (typeof r.name !== "string" || !r.name || ids.has(r.name))
          throw new Error("Memory names must be unique");
        ids.add(r.name);
        integer(r.address, 0xffffffff, "address");
        integer(r.length, 1048576, "length");
        if (r.address + r.length > 0x100000000)
          throw new Error("Memory range overflow");
        if (r.sha256 !== undefined) hash(r.sha256, "memory");
      }
    }
  }
  if (m.inputs.some((e) => BigInt(e.at) > prev))
    throw new Error("Input after final checkpoint");
  if ((m.cheats ?? []).some((e) => BigInt(e.at) > prev))
    throw new Error("Cheat event after final checkpoint");
  return m;
}
function pcmBytes(samples) {
  const out = new Uint8Array(samples.length * 2),
    view = new DataView(out.buffer);
  samples.forEach((s, i) => view.setUint16(i * 2, s & 65535, true));
  return out;
}
function concat(parts, length) {
  const out = new Uint8Array(length);
  let offset = 0;
  for (const p of parts) {
    out.set(p, offset);
    offset += p.length;
  }
  return out;
}

// Owns initialization and the entire deterministic session. Assets are raw bytes,
// never paths/URLs; the CLI is the only layer with filesystem authority.
export async function runScenario(core, manifest, assets, options = {}) {
  validateScenario(manifest);
  const m = jsonSafe(manifest);
  for (const name of Object.keys(assets))
    if (!(name in m.assets)) throw new Error(`Undeclared ${name} asset`);
  for (const [name, descriptor] of Object.entries(m.assets)) {
    if (!(assets[name] instanceof Uint8Array))
      throw new Error(`Missing ${name} bytes`);
    if ((await sha1(assets[name])) !== descriptor.sha1)
      throw new Error(`${name} SHA-1 mismatch`);
  }
  const result = {
    version: 2,
    name: m.name,
    build: options.build ?? null,
    manifest: m,
    status: "passed",
    checkpoints: [],
    runs: 0,
    inputsConsumed: 0,
    failures: [],
    artifacts: [],
  };
  let recorder,
    audioParts = [],
    audioLength = 0,
    totalAudio = 0;
  async function artifact(name, bytes) {
    const owned = bytes.slice();
    result.artifacts.push({
      name,
      size: owned.length,
      sha256: await sha256(owned),
    });
    if (options.onArtifact) await options.onArtifact(name, owned);
  }
  function fail(message) {
    result.failures.push(message);
    result.status = "failed";
  }
  try {
    core.load(assets.rom, {
      zipped: false,
      bios: assets.bios,
      save: assets.save,
      saveType: m.config.saveType,
      rtc: m.config.rtc,
      rtcEpoch: BigInt(m.config.rtcEpoch),
      sampleRate: m.config.sampleRate,
    });
    if (assets.state) core.loadState(assets.state);
    const initial = core.inspect();
    if (initial.romHash !== m.assets.rom.sha1)
      throw new Error("Loaded ROM identity mismatch");
    if (
      m.inputs.some((e) => BigInt(e.at) < initial.cycle) ||
      (m.cheats ?? []).some((e) => BigInt(e.at) < initial.cycle) ||
      m.checkpoints.some((p) => BigInt(p.at) < initial.cycle)
    )
      throw new Error("Scenario event predates initial state");
    result.initial = jsonSafe(initial);
    recorder =
      options.capture === false
        ? null
        : new FailureRecorder(core, {
            ...options.capture,
            identity: { manifest: m, build: result.build },
          });
    const execution = recorder ?? core;
    const inputs = m.inputs.map((e) => ({ at: BigInt(e.at), keys: e.keys }));
    // A cheat event applies at the first stop at or after its cycle: runs
    // end there, so a recorded event lands on the cycle it was made at.
    const cheatEvents = (m.cheats ?? []).map((e) => ({
      at: BigInt(e.at),
      cheats: e.cheats,
    }));
    let cheatCursor = 0;
    const applyDueCheats = () => {
      while (
        cheatCursor < cheatEvents.length &&
        cheatEvents[cheatCursor].at <= core.inspect().cycle
      )
        execution.setCheats(cheatEvents[cheatCursor++].cheats);
    };
    let cursor = 0,
      lastReason = "deadline";
    for (const checkpoint of m.checkpoints) {
      const target = BigInt(checkpoint.at);
      do {
        if (options.signal?.aborted) {
          result.status = "cancelled";
          return result;
        }
        if (options.yieldControl && result.runs % 4 === 0)
          await options.yieldControl();
        if (options.signal?.aborted) {
          result.status = "cancelled";
          return result;
        }
        applyDueCheats();
        const before = core.inspect().cycle;
        if (
          before >= target &&
          !(cursor < inputs.length && inputs[cursor].at <= target)
        )
          break;
        if (++result.runs > m.limits.maxRuns)
          throw new Error("Scenario run budget exceeded");
        let deadline =
          before + BigInt(m.limits.chunkCycles) < target
            ? before + BigInt(m.limits.chunkCycles)
            : target;
        const nextCheat = cheatEvents[cheatCursor]?.at;
        if (
          nextCheat !== undefined &&
          nextCheat > before &&
          nextCheat < deadline
        )
          deadline = nextCheat;
        const run = execution.runUntil(
          deadline,
          inputs.slice(cursor),
          BigInt(m.checkpoints.at(-1).at),
        );
        cursor += run.inputsConsumed;
        result.inputsConsumed = cursor;
        lastReason = run.reason;
        const audio = pcmBytes(execution.drainAudio());
        totalAudio += audio.length;
        if (totalAudio > m.limits.maxAudioBytes)
          throw new Error("Scenario audio budget exceeded");
        audioParts.push(audio);
        audioLength += audio.length;
        if (run.reason === "faulted" || run.reason === "power_stopped") break;
        if (run.cycle <= before && run.inputsConsumed === 0)
          throw new Error("Execution made no progress");
      } while (core.inspect().cycle < target);
      if (lastReason !== "faulted") applyDueCheats();
      const state = core.inspect();
      const audio = concat(audioParts, audioLength);
      audioParts = [];
      audioLength = 0;
      const frame = pcmBytes(core.frame()),
        save = core.exportSave();
      const observed = {
        name: checkpoint.name,
        requestedCycle: checkpoint.at,
        cycle: state.cycle.toString(),
        reason: lastReason,
        fault: state.fault,
        cpu: jsonSafe({
          registers: state.registers,
          cpsr: state.cpsr,
          nextInstructionAddress: state.nextInstructionAddress,
        }),
        frameSha256: await sha256(frame),
        audioSha256: await sha256(audio),
        audioBytes: audio.length,
        saveSha256: await sha256(save),
        memory: [],
      };
      const expected = checkpoint.expect ?? {};
      if (!state.fault) observed.stateSha256 = await sha256(core.saveState());
      for (const [key, value] of Object.entries(expected)) {
        if (key === "cpu") {
          for (const [field, n] of Object.entries(value)) {
            if (field === "registers") {
              for (const [reg, v] of Object.entries(n))
                if (state.registers[reg] !== v)
                  fail(
                    `${checkpoint.name}: r${reg} expected ${v}, got ${state.registers[reg]}`,
                  );
            } else if (state[field] !== n)
              fail(
                `${checkpoint.name}: ${field} expected ${n}, got ${state[field]}`,
              );
          }
        } else if (observed[key] !== value)
          fail(
            `${checkpoint.name}: ${key} expected ${value}, got ${observed[key]}`,
          );
      }
      const regions = [];
      for (const r of checkpoint.memory ?? []) {
        const data = core.readMemory(r.address, r.length);
        const actual = await sha256(data);
        observed.memory.push({ ...r, sha256: actual });
        regions.push(data);
        if (r.sha256 && r.sha256 !== actual)
          fail(`${checkpoint.name}: memory ${r.name} hash mismatch`);
      }
      if (
        ["power_stopped", "faulted"].includes(lastReason) &&
        expected.reason !== lastReason
      )
        fail(
          `${checkpoint.name}: unexpected ${lastReason}${state.fault ? ` (${state.fault})` : ""}`,
        );
      result.checkpoints.push(observed);
      if (options.onProgress) options.onProgress(jsonSafe(observed));
      if (options.allArtifacts) {
        await artifact(
          `checkpoint-${result.checkpoints.length}.bgr555le`,
          frame,
        );
        await artifact(`checkpoint-${result.checkpoints.length}.s16le`, audio);
      }
      if (result.status === "failed") {
        await artifact("frame.bgr555le", frame);
        await artifact("audio.s16le", audio);
        await artifact("battery.sav", save);
        for (let i = 0; i < regions.length; i++)
          await artifact(`memory-${i}.bin`, regions[i]);
        break;
      }
      if (lastReason === "faulted" || lastReason === "power_stopped") {
        if (checkpoint !== m.checkpoints.at(-1) || cursor !== inputs.length)
          fail("Terminal result before remaining checkpoints or inputs");
        break;
      }
    }
  } catch (error) {
    fail(error.message ?? String(error));
  }
  if (result.status === "failed" && recorder) {
    try {
      result.capture =
        recorder.lastCapture ??
        recorder.capture("scenario_failure", { failures: result.failures });
    } catch (error) {
      result.captureError = error.message;
    }
  }
  return result;
}
