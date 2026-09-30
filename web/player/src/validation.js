// Browser-native SDK integration test. Optional fixtures are served only when
// named explicitly on tools/serve-web.py's command line; no game is bundled here.
import { Emulator, CYCLES_PER_FRAME as F } from "@evenfall/gba";
import audioWorkletUrl from "@evenfall/gba/audio-worklet?url&no-inline";
import { acquireSaveLock } from "./storage.js";
// Browser tests drive the bundled SDK and the save lock through these hooks.
Object.assign(window, { zigbaValidation: { Emulator, acquireSaveLock } });
const results = [];
const output = document.getElementById("result");
function assert(value, message) {
  if (!value) throw new Error(message);
}
function record(message) {
  results.push(message);
  output.textContent = results.join("\n");
}
const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
async function until(predicate, label) {
  for (let i = 0; i < 1000; i++) {
    if (predicate()) return;
    await wait(20);
  }
  throw new Error(`Timeout: ${label}`);
}
const sha = async (bytes) =>
  [...new Uint8Array(await crypto.subtle.digest("SHA-256", bytes))]
    .map((x) => x.toString(16).padStart(2, "0"))
    .join("");
let emulator;
// SDK tests need the bundled hooks, not another run of this whole suite.
if (new URL(location.href).searchParams.has("sdk-only")) {
  output.textContent = "READY";
} else
  try {
    emulator = await Emulator.create();
    const errors = [];
    window.addEventListener("error", (e) => errors.push(e.message));
    window.addEventListener("unhandledrejection", (e) =>
      errors.push(String(e.reason)),
    );
    emulator.addEventListener("error", (e) => errors.push(e.detail.message));
    // Return presentation buffers to the worker, as a player does.
    emulator.addEventListener("frame", ({ detail }) =>
      emulator.recyclePresentation(detail.pixels, detail.audio),
    );
    const fixture = new URL(location.href).searchParams.get("fixture");
    const image = fixture
      ? new Uint8Array(
          await fetch(`/__fixtures__/${encodeURIComponent(fixture)}`).then(
            (r) => {
              if (!r.ok) throw new Error("Fixture unavailable");
              return r.arrayBuffer();
            },
          ),
        )
      : new Uint8Array([0xfe, 0xff, 0xff, 0xea]);
    const zipped = image[0] === 0x50 && image[1] === 0x4b;
    let state = await emulator.load(image, { zipped });
    assert(
      state.cycle === 0n && !state.fault,
      state.fault ?? "ROM did not load at reset",
    );
    record(
      `PASS ROM load${zipped ? " from ZIP" : ""}: ${state.romHash}, save=${state.saveType}`,
    );
    const start = state.cycle;
    const dueNow = await emulator.runUntil(start, [{ at: start, keys: 4 }]);
    assert(
      dueNow.inputsConsumed === 1 && dueNow.cycle === start,
      "Due-now input was not applied without advancing time",
    );
    await emulator.setKeys(0);
    const memory = await emulator.readMemory(0x08000000, 16);
    assert(
      memory.length === 16 && (await emulator.inspect()).cycle === start,
      "Inspection changed emulated time",
    );
    await emulator.runUntil(start + 120n * F, [
      { at: start + F, keys: 8 },
      { at: start + 2n * F, keys: 0 },
    ]);
    state = await emulator.inspect();
    assert(
      state.cycle >= 120n * F && !state.fault,
      state.fault ?? "No execution",
    );
    record(
      `PASS worker execution and timed input: ${state.frames} frames, cycle=${state.cycle}`,
    );
    const snapshot = await emulator.saveState();
    const savedHash = await sha(snapshot);
    await emulator.runUntil(state.cycle + 2n * F);
    await emulator.loadState(snapshot);
    assert(
      (await sha(await emulator.saveState())) === savedHash,
      "Snapshot did not restore exactly",
    );
    const corrupt = snapshot.slice();
    corrupt[0] ^= 1;
    let rejected = false;
    try {
      await emulator.loadState(corrupt);
    } catch {
      rejected = true;
    }
    assert(
      rejected && (await sha(await emulator.saveState())) === savedHash,
      "Invalid snapshot changed state",
    );
    record("PASS transactional snapshot restore and invalid-state recovery");
    const second = await Emulator.create();
    await second.load(new Uint8Array([0xfe, 0xff, 0xff, 0xea]));
    assert((await second.inspect()).cycle === 0n, "Workers share state");
    await second.dispose();
    record("PASS isolated session lifecycle");
    const bytes = await emulator.exportSave();
    if (bytes.length) {
      const { SaveStore } = await import("./storage.js");
      const db = await SaveStore.open();
      // A separate fixture key cannot replace the user's title save.
      const key = `validation-${state.romHash}`;
      await db.put(key, bytes);
      assert(
        (await sha((await db.get(key)).data)) === (await sha(bytes)),
        "IndexedDB save mismatch",
      );
      record(`PASS IndexedDB save round trip: ${bytes.length} bytes`);
    }
    // Verify the actual worklet renders incoming stereo samples, without relying
    // on speakers or making a subjective claim about sound quality.
    const audio = new OfflineAudioContext(2, 4096, 48000);
    // Initialize the offline render thread, suspended before its first sample.
    // This also works on browsers that defer the worklet thread until rendering.
    const suspended = audio.suspend(0);
    const rendering = audio.startRendering();
    await suspended;
    await Promise.race([
      audio.audioWorklet.addModule(audioWorkletUrl),
      wait(10000).then(() => {
        throw new Error("Offline AudioWorklet initialization timed out");
      }),
    ]);
    const samples = new Int16Array(8192);
    for (let i = 0; i < 4096; i++) {
      samples[i * 2] = 100;
      samples[i * 2 + 1] = -100;
    }
    const node = new AudioWorkletNode(audio, "zigba-audio", {
      numberOfInputs: 0,
      outputChannelCount: [2],
      processorOptions: { samples },
    });
    node.connect(audio.destination);
    await audio.resume();
    const rendered = await rendering;
    assert(
      rendered.getChannelData(0).some((x) => x > 0) &&
        rendered.getChannelData(1).some((x) => x < 0),
      "AudioWorklet did not render stereo PCM",
    );
    record("PASS AudioWorklet stereo rendering");
    const keyinput = async () =>
      new DataView((await emulator.readMemory(0x04000130, 2)).buffer).getUint16(
        0,
        true,
      );
    await emulator.setKeys(1);
    assert(((await keyinput()) & 1) === 0, "A was not applied to KEYINPUT");
    await emulator.setKeys(0);
    assert(((await keyinput()) & 1) === 1, "Released A remained held");
    record("PASS key input and release through KEYINPUT");
    if (fixture && new URL(location.href).searchParams.has("replay")) {
      const inputs = [
        { at: 1250n * F, keys: 8 },
        { at: 1255n * F, keys: 0 },
        { at: 1400n * F, keys: 1 },
        { at: 1405n * F, keys: 0 },
      ];
      let consumed = 0;
      for (const target of [600n, 1200n, 1800n, 2200n]) {
        const result = await emulator.runUntil(
          target * F,
          inputs.slice(consumed),
        );
        consumed += result.inputsConsumed;
        assert(
          result.reason !== "faulted" && result.reason !== "power_stopped",
          (await emulator.inspect()).fault ?? result.reason,
        );
      }
      const end = await emulator.inspect();
      record(
        `PASS 2200-frame replay: cycle=${end.cycle}, PC=${end.nextInstructionAddress.toString(16)}, inputs=${consumed}, framebuffer=${await sha((await emulator.frame()).buffer)}`,
      );
    }
    // Realtime play and pause in the worker, paced by the browser's clock.
    const old = (await emulator.inspect()).cycle;
    const costs = [];
    let frames = 0;
    const measure = ({ detail }) => {
      frames++;
      if (detail.workMs > 0) costs.push(detail.workMs);
    };
    emulator.addEventListener("frame", measure);
    await emulator.play();
    await until(() => frames > 0, "play");
    await wait(2000);
    await emulator.pause();
    const paused = await emulator.inspect();
    assert(paused.cycle > old, "Realtime playback did not advance");
    emulator.removeEventListener("frame", measure);
    await wait(100);
    assert(
      (await emulator.inspect()).cycle === paused.cycle,
      "Pause did not stop execution",
    );
    costs.sort((a, b) => a - b);
    record(
      `PASS realtime play/pause controls: ${frames} frames, worker emulation/resampling p95=${costs[Math.floor(costs.length * 0.95)]?.toFixed(2)} ms`,
    );
    assert(!errors.length, errors.join("\n"));
    record(
      `PASS browser integration complete; framebuffer SHA-256=${await sha((await emulator.frame()).buffer)}`,
    );
    document.title = "ZiGBA browser validation · PASS";
    output.dataset.result = "pass";
  } catch (error) {
    record(`FAIL ${error.stack ?? error}`);
    document.title = "ZiGBA browser validation · FAIL";
    output.dataset.result = "fail";
  } finally {
    await emulator?.dispose().catch(() => {});
    await fetch(`/__validation_result__?token=${window.validationToken}`, {
      method: "POST",
      body: output.textContent,
    });
  }
