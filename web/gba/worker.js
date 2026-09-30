import { WasmCore, CYCLES_PER_FRAME } from "./core.js";
import { PresentationPool } from "./presentation.js";
const presentation = new PresentationPool();
import { GameplayRecording } from "./devtools/journal.js";
import { runScenario, sha1, sha256 } from "./devtools/scenario.js";
import { replayFailureBundleAsync } from "./devtools/capture.js";
let recording, cancellation, build;
const execution = () => recording ?? core;
function recordedConfig() {
  return {
    boot: initial.options.bios ? "bios" : "hle",
    saveType: initial.options.saveType ?? "auto",
    rtc: initial.options.rtc ?? core.inspect().rtcEnabled,
    rtcEpoch: String(initial.options.rtcEpoch ?? 0n),
    sampleRate: initial.options.sampleRate ?? 48000,
  };
}
function beginRecording(options = {}) {
  recording = new GameplayRecording(core, {
    ...options,
    build,
    onCapture: (bundle) => postMessage({ event: "capture", bundle }),
  });
  updatePlaybackOutput();
}

let core;
let playing = false;
let playbackRate = 1;
let busy = false;
let timer;
let pumpEpoch = 0;
const pumpChannel = new MessageChannel();
pumpChannel.port1.onmessage = ({ data }) => {
  if (data === pumpEpoch) pump();
};
function schedulePump(delay) {
  const epoch = pumpEpoch;
  // Run frame work in a message task. Scheduling the next timer from that
  // task resets HTML timer nesting, avoiding its 4 ms floor at high speeds.
  timer = setTimeout(() => pumpChannel.port2.postMessage(epoch), delay);
}
let nextCycle = 0n;
let nextTime = 0;
let pendingFrames = 0;
let lastVideoFrameId;
let pendingWorkMs = 0,
  pendingWorkFrames = 0;
let generation = 0;
let initial;
let diagnosticsIntervalMs = 0;
let diagnosticState,
  diagnosticTime = -Infinity;
const frameMs = Number(CYCLES_PER_FRAME) / 16777.216;
const quantum = CYCLES_PER_FRAME / 4n;

function updatePlaybackOutput() {
  if (core?.loaded) {
    core.setAudioOutput(!playing || playbackRate <= 2);
    core.setVideoInterval(
      playing && playbackRate > 2 ? Math.ceil(playbackRate) : 1,
    );
    core.setRewindSpeed?.(playbackRate);
  }
  // Retain roughly three wall-clock seconds at the requested speed. The
  // recorder's independent operation/byte budgets still bound all history.
  if (recording)
    recording.recorder.snapshotIntervalCycles =
      CYCLES_PER_FRAME *
      BigInt(Math.ceil(60 * (playing ? Math.max(1, playbackRate) : 1)));
}

function stop() {
  playing = false;
  updatePlaybackOutput();
  clearTimeout(timer);
  pumpEpoch++;
}
function clearPresentation() {
  diagnosticState = undefined;
  generation++;
  pendingFrames = 0;
  pendingWorkMs = 0;
  pendingWorkFrames = 0;
  lastVideoFrameId = core?.loaded ? core.videoFrameId() : undefined;
  postMessage({ event: "clear", generation, timeline: core?.timeline ?? null });
}
function sendFrame(
  audio = new Int16Array(),
  started,
  earlierWorkMs = 0,
  workFrames = 1,
) {
  const selected = playing && playbackRate > 2;
  const pixels = selected
    ? core.presentationFrame(presentation.takePixels())
    : core.frame(presentation.takePixels());
  lastVideoFrameId = core.videoFrameId();
  pendingFrames++;
  const now = performance.now();
  if (!diagnosticState || now - diagnosticTime >= diagnosticsIntervalMs) {
    diagnosticState = core.inspect();
    diagnosticTime = now;
  }
  const workMs =
    started === undefined
      ? 0
      : (earlierWorkMs + performance.now() - started) / workFrames;
  postMessage(
    {
      event: "frame",
      pixels,
      audio,
      generation,
      workMs,
      memoryBytes: core.wasm.memory.buffer.byteLength,
      videoFrameId: lastVideoFrameId,
      videoFrameCycle: core.videoFrameCycle(),
      emulatedWorkFrames: workFrames,
      presentationAllocations: presentation.allocations,
      state: diagnosticState,
    },
    [pixels.buffer, audio.buffer],
  );
}
function advanceFrame() {
  const started = performance.now();
  const audible = playbackRate <= 2;
  const audio = audible ? presentation.takeAudio() : null;
  let length = 0;
  let result;
  const slice = audible ? quantum : CYCLES_PER_FRAME;
  while (core.wasm.ef_cycle() < nextCycle) {
    const at = core.wasm.ef_cycle();
    result = execution().runUntil(
      at + slice < nextCycle ? at + slice : nextCycle,
    );
    if (audible)
      length += execution().drainAudio(audio.subarray(length)).length;
    if (result.reason === "faulted" || result.reason === "power_stopped") break;
  }
  if (audible || core.videoFrameId() !== lastVideoFrameId) {
    sendFrame(
      audio ? audio.subarray(0, length) : new Int16Array(),
      started,
      pendingWorkMs,
      pendingWorkFrames + 1,
    );
    pendingWorkMs = 0;
    pendingWorkFrames = 0;
  } else {
    pendingWorkMs += performance.now() - started;
    pendingWorkFrames++;
  }
  return result;
}
function pump() {
  if (!playing) return;
  // A hidden/unresponsive host cannot accumulate unbounded video/audio messages.
  if (pendingFrames >= 2) {
    schedulePump(4);
    return;
  }
  try {
    nextTime += frameMs / playbackRate;
    if (performance.now() - nextTime > 200) nextTime = performance.now();
    // Start the next wait before frame work, yielding for commands after
    // every frame even when the requested speed exceeds core capacity.
    schedulePump(Math.max(0, nextTime - performance.now()));
    nextCycle += CYCLES_PER_FRAME;
    const result = advanceFrame();
    if (result?.reason === "faulted" || result?.reason === "power_stopped") {
      stop();
      postMessage({
        event: "stopped",
        reason: result.reason,
        state: core.inspect(),
      });
      return;
    }
  } catch (error) {
    stop();
    postMessage({ event: "error", message: error.message });
  }
}

async function command(method, args) {
  if (method === "init") {
    diagnosticsIntervalMs = args[1]?.diagnosticsIntervalMs ?? 0;
    if (!Number.isFinite(diagnosticsIntervalMs) || diagnosticsIntervalMs < 0)
      throw new RangeError("Invalid diagnostics interval");
    const response = await fetch(args[0]);
    if (!response.ok) throw new Error("WASM fetch failed");
    const bytes = new Uint8Array(await response.arrayBuffer());
    build = { wasmSha256: await sha256(bytes) };
    core = await WasmCore.create(bytes);
    return { abiVersion: 1 };
  }
  if (!core) throw new Error("NotInitialized");
  if (method === "cancel") {
    cancellation?.abort();
    return null;
  }
  // A focus-loss key release can land between manual execution slices.
  // Every other exclusive task (scenario, replay, rewind, load) rejects
  // player mutations.
  if (busy && !(method === "setKeys" && busy === "manual" && recording))
    throw new Error("SessionBusy");
  switch (method) {
    case "scenario": {
      stop();
      busy = true;
      cancellation = new AbortController();
      recording = null;
      try {
        return await runScenario(core, args[0], args[1], {
          build,
          signal: cancellation.signal,
          yieldControl: () => new Promise((resolve) => setTimeout(resolve, 0)),
          allArtifacts: true,
          onProgress: (checkpoint) =>
            postMessage({ event: "progress", checkpoint }),
          onArtifact: (name, bytes) =>
            postMessage({ event: "artifact", name, bytes }, [bytes.buffer]),
        });
      } finally {
        busy = false;
        cancellation = null;
        clearPresentation();
      }
    }
    case "replay": {
      stop();
      busy = true;
      cancellation = new AbortController();
      recording = null;
      try {
        return await replayFailureBundleAsync(core, args[0], {
          identity: {
            manifest: args[0].identity.manifest,
            config: recordedConfig(),
            build,
            biosSha1: initial.options.biosSha1,
          },
          signal: cancellation.signal,
        });
      } finally {
        busy = false;
        cancellation = null;
        clearPresentation();
      }
    }
    case "recording":
      if (args[0] !== undefined) {
        recording.enabled = args[0];
        if (args[0]) recording.start();
      }
      updatePlaybackOutput();
      return recording.status();
    case "segment":
      recording.start();
      updatePlaybackOutput();
      return recording.status();
    case "checkpoint":
      return recording.checkpoint(args[0]);
    case "exportScenario":
      return recording.scenario(args[0]);
    case "capture":
      stop();
      return recording.recorder.capture("manual", { note: args[0] });
    case "rewind": {
      if (playing) throw new Error("PauseBeforeRewind");
      core.requireLoaded();
      const steps = args[0] ?? 1;
      if (!Number.isInteger(steps) || steps < 1 || steps > 1024)
        throw new RangeError("Invalid rewind steps");
      busy = true;
      try {
        const result = core.rewind(steps);
        // The engine replaced the timeline: prior op history cannot replay it.
        if (recording) recording.start();
        updatePlaybackOutput();
        clearPresentation();
        sendFrame();
        return result;
      } finally {
        busy = false;
      }
    }
    case "rewindConfig":
      core.requireLoaded();
      core.setRewindConfig(args[0], args[1], args[2]);
      updatePlaybackOutput();
      return { depth: core.rewindDepth() };
    case "rewindDepth":
      core.requireLoaded();
      return core.rewindDepth();
    case "rewindInfo":
      core.requireLoaded();
      return core.rewindInfo(args[0]);
    case "rewindCheckpoints": {
      core.requireLoaded();
      const live = core.inspect();
      const checkpoints = [];
      const depth = core.rewindDepth();
      for (let index = 0; index < depth; index++) {
        const info = core.rewindInfo(index);
        if (info.frame < live.frames)
          checkpoints.push({ index, steps: 0, ...info });
      }
      for (let index = 0; index < checkpoints.length; index++)
        checkpoints[index].steps = checkpoints.length - index;
      return {
        timeline: core.timeline,
        liveCycle: live.cycle,
        liveFrame: live.frames,
        checkpoints,
      };
    }
    case "rewindFrame":
      core.requireLoaded();
      return core.rewindFrame(args[0]);
    case "exportRom":
      return core.exportRom();
    case "load": {
      stop();
      // Hash before replacing the machine, and hold the session across the
      // await: a command arriving meanwhile would otherwise see the new
      // machine with the old recording, or publish its own and be overwritten.
      busy = true;
      try {
        const biosSha1 = args[1]?.bios ? await sha1(args[1].bios) : undefined;
        const result = core.load(...args);
        initial = { options: { ...args[1], biosSha1 } };
        beginRecording(initial.options);
        clearPresentation();
        sendFrame();
        return result;
      } finally {
        busy = false;
      }
    }
    case "setSpeed": {
      const value = args[0];
      if (!Number.isFinite(value) || value < 0.5 || value > 16)
        throw new RangeError("Speed must be between 0.5 and 16");
      playbackRate = value;
      updatePlaybackOutput();
      clearPresentation();
      // Start a new wall-clock pacing interval; never change machine cycles.
      if (playing) {
        clearTimeout(timer);
        pumpEpoch++;
        nextTime = performance.now() + frameMs / playbackRate;
        schedulePump(frameMs / playbackRate);
      }
      return playbackRate;
    }
    case "play":
      core.requireLoaded();
      if (core.inspect().fault) throw new Error("MachineFaulted");
      if (!playing) {
        playing = true;
        updatePlaybackOutput();
        nextCycle = core.wasm.ef_cycle();
        nextTime = performance.now();
        schedulePump(0);
      }
      return null;
    case "pause":
      stop();
      clearPresentation();
      return core.loaded ? core.inspect() : null;
    case "reset": {
      core.requireLoaded();
      stop();
      const save = core.exportSave();
      const result = core.resetWithSave(save);
      beginRecording({ ...initial.options, save });
      clearPresentation();
      sendFrame();
      return result;
    }
    case "resetWithSave": {
      core.requireLoaded();
      stop();
      const result = core.resetWithSave(args[0]);
      beginRecording({ ...initial.options, save: args[0] });
      clearPresentation();
      sendFrame();
      return result;
    }
    case "runUntil": {
      if (playing) throw new Error("PauseBeforeManualExecution");
      core.requireLoaded();
      const [deadline, events = []] = args;
      const start = core.wasm.ef_cycle();
      if (
        typeof deadline !== "bigint" ||
        deadline < start ||
        deadline > 0xffffffffffffffffn ||
        deadline - start > 600n * CYCLES_PER_FRAME
      )
        throw new RangeError(
          "Manual execution is limited to 600 frames per request",
        );
      if (!Array.isArray(events) || events.length > 1024)
        throw new RangeError("At most 1024 input events per request");
      let previous = start;
      for (const event of events) {
        if (
          typeof event.at !== "bigint" ||
          event.at < previous ||
          event.at > 0xffffffffffffffffn ||
          !Number.isInteger(event.keys) ||
          event.keys < 0 ||
          event.keys > 1023
        )
          throw new RangeError("Invalid input event");
        previous = event.at;
      }
      busy = "manual";
      try {
        let consumed = 0;
        let result = {
          cycle: start,
          frames: core.wasm.ef_frames(),
          reason: "deadline",
        };
        let slices = 0;
        if (deadline === start) {
          result = execution().runUntil(deadline, events);
          consumed = result.inputsConsumed;
        }
        while (core.wasm.ef_cycle() < deadline) {
          const at = core.wasm.ef_cycle();
          result = execution().runUntil(
            at + quantum < deadline ? at + quantum : deadline,
            events.slice(consumed),
            deadline,
          );
          consumed += result.inputsConsumed;
          execution().drainAudio();
          if (result.reason === "faulted" || result.reason === "power_stopped")
            break;
          if (++slices % 4 === 0)
            await new Promise((resolve) => setTimeout(resolve, 0));
        }
        clearPresentation();
        sendFrame();
        return { ...result, inputsConsumed: consumed };
      } finally {
        busy = false;
      }
    }
    case "step":
      if (playing) throw new Error("PauseBeforeManualExecution");
      {
        const result = execution().step();
        execution().drainAudio();
        sendFrame();
        return result;
      }
    case "loadState":
      if (playing) throw new Error("PauseBeforeLoadingState");
      {
        const result = core.loadState(args[0]);
        beginRecording(initial.options);
        clearPresentation();
        sendFrame();
        return result;
      }
    case "loadRewindBundle": {
      if (playing) throw new Error("PauseBeforeLoadingRewindBundle");
      const result = core.loadRewindBundle(args[0]);
      // Drop the old input/operation journal; replay starts from this live
      // position, not from a cold boot or the newest periodic checkpoint.
      if (recording) recording.start();
      updatePlaybackOutput();
      clearPresentation();
      sendFrame();
      return result;
    }
    case "inspect":
      return core.inspect();
    case "readMemory":
      return core.readMemory(...args);
    case "captureMemory":
      return core.loaded ? core.captureMemory() : null;
    case "setKeys":
      return execution().setKeys(args[0]);
    case "saveState":
      return core.saveState();
    case "saveRewindBundle":
      return core.saveRewindBundle();
    case "exportSave":
      return core.exportSave();
    case "frame":
      return core.frame();
    case "dispose":
      stop();
      core.dispose();
      initial = null;
      clearPresentation();
      return null;
    default:
      throw new Error(`UnknownMethod: ${method}`);
  }
}

self.onmessage = async ({ data }) => {
  if (data.recycle) {
    presentation.release(data.recycle);
    return;
  }
  if (data.ack !== undefined) {
    if (data.ack === generation) pendingFrames = Math.max(0, pendingFrames - 1);
    return;
  }
  try {
    const result = await command(data.method, data.args ?? []);
    const transfer = ArrayBuffer.isView(result)
      ? [result.buffer]
      : data.method === "captureMemory" && result
        ? result.regions.map((region) => region.bytes.buffer)
        : [];
    postMessage({ id: data.id, result }, transfer);
  } catch (error) {
    postMessage({ id: data.id, error: error.message });
  }
};
