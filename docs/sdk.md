# JavaScript and TypeScript SDK

`@evenfall/gba` puts Evenfall's GBA core in a web page or a Node program.
In a browser, the `Emulator` class runs the core in a Web Worker and sends the page frames and audio as events.
In Node, `WasmCore` runs it synchronously, which suits tests, tools and servers.
Both run the same Rust core, compiled to WebAssembly, that the native build runs.

The SDK only emulates.
It creates no canvas or audio device, stores nothing, and has no runtime dependencies.
Your page draws the frames, plays the sound and keeps the saves.
[`design/web-runtime.md`](design/web-runtime.md) explains how the pieces fit together inside.

The package is private, used from this repository's `web/*` npm workspace, and not published to npm.
It is experimental and versioned 0.x, so a minor release can change the interface ([Versioning](#versioning)).

## Playing a game in a page

```ts
import { Emulator, KEYS } from "@evenfall/gba";

const canvas = document.querySelector("canvas")!; // 240x160
const context = canvas.getContext("2d")!;
const image = context.createImageData(240, 160);

const emulator = await Emulator.create();
await emulator.load(new Uint8Array(await file.arrayBuffer()));

emulator.addEventListener("frame", ({ detail }) => {
  // Each pixel is 15-bit BGR: red in bits 0-4, green in 5-9, blue in 10-14.
  for (let i = 0; i < detail.pixels.length; i++) {
    const p = detail.pixels[i];
    image.data[i * 4 + 0] = (p & 31) << 3;
    image.data[i * 4 + 1] = ((p >> 5) & 31) << 3;
    image.data[i * 4 + 2] = ((p >> 10) & 31) << 3;
    image.data[i * 4 + 3] = 255;
  }
  context.putImageData(image, 0, 0);
});

const held = new Set<number>();
const keyMap: Record<string, number> = {
  KeyX: KEYS.a,
  KeyZ: KEYS.b,
  Enter: KEYS.start,
  Backspace: KEYS.select,
  ArrowUp: KEYS.up,
  ArrowDown: KEYS.down,
  ArrowLeft: KEYS.left,
  ArrowRight: KEYS.right,
  KeyA: KEYS.l,
  KeyS: KEYS.r,
};
const sendKeys = () => emulator.setKeys([...held].reduce((m, k) => m | k, 0));
addEventListener("keydown", (e) => {
  if (keyMap[e.code]) {
    held.add(keyMap[e.code]);
    sendKeys();
  }
});
addEventListener("keyup", (e) => {
  if (keyMap[e.code]) {
    held.delete(keyMap[e.code]);
    sendKeys();
  }
});

await emulator.play();
```

That is a complete, silent player.
[Audio](#audio) adds sound, and [Persistence](#persistence) covers keeping the game's save between visits.

Games and BIOS images aren't included.
Your page supplies ROM bytes, for example from a file the player opens, and the SDK never uploads them anywhere.

## Installing

From the repository root, run `npm ci`, then build the core with `npm run build:wasm -w @evenfall/gba`.
The package contains ES modules, TypeScript declarations and two builds of the core, both kept out of Git:

- `evenfall-gba.wasm`, which uses WebAssembly SIMD,
- `evenfall-gba-scalar.wasm`, for browsers without SIMD.

Browsers need module Workers and WebAssembly with BigInt support.
Serve pages over HTTPS or from `localhost`.
Cross-origin isolation isn't needed.
Node needs version 24 or later.

The SDK is licensed under the [GNU Affero General Public License, version 3 or later](../LICENSE).
An application or service that includes it, whether distributed or offered over a network, must be released under the same license with its source code.
For other terms, contact [contact@evenfall-emu.org](mailto:contact@evenfall-emu.org).

### Entry points

| Import                                   | Contents                                                                                                            |
| ---------------------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| `@evenfall/gba`                          | `Emulator` (asynchronous, one Worker per session), `KEYS`, `CYCLES_PER_FRAME`, `SAVE_TYPES`, `CHEAT_FORMATS`, types |
| `@evenfall/gba/core`                     | `WasmCore`, the synchronous adapter for Node and Workers                                                            |
| `@evenfall/gba/devtools/scenario`        | `runScenario`, `validateScenario`, `sha1`, `sha256`, `jsonSafe`                                                     |
| `@evenfall/gba/devtools/capture`         | `FailureRecorder`, `validateFailureBundle`, `replayFailureBundle`, `replayFailureBundleAsync`                       |
| `@evenfall/gba/bridge`                   | typed read-only views of game memory ([Game bridges](#game-bridges))                                                |
| `@evenfall/gba/bridge/react`             | `BridgeProvider`, `useViews`, `useBridge`, `useBridgeResources` (React 18+, optional)                               |
| `@evenfall/gba/audio-worklet`            | the `evenfall-audio` AudioWorklet processor, loaded by URL                                                          |
| `@evenfall/gba/evenfall-gba.wasm`        | the core, built with SIMD                                                                                           |
| `@evenfall/gba/evenfall-gba-scalar.wasm` | the core, built without SIMD                                                                                        |

Every JavaScript entry point has type declarations.
This page summarizes the interface, and [`emulator.d.ts`](../web/gba/emulator.d.ts), [`core.d.ts`](../web/gba/core.d.ts) and the [devtools declarations](../web/gba/devtools/) are the reference.

### With a bundler

`Emulator.create()` starts the package's Worker with `new Worker(new URL("./worker.js", import.meta.url), { type: "module" })`.
It passes the Worker `new URL("./evenfall-gba.wasm", import.meta.url)`, or the scalar build when `WebAssembly.validate` reports no SIMD.
Bundlers that understand these standard forms ship the Worker and both modules with your application and fix the URLs, with nothing to configure.

Vite's development server pre-bundles dependencies into `node_modules/.vite`, where those relative URLs don't resolve, so leave the SDK out of pre-bundling:

```js
// vite.config.js
import { defineConfig } from "vite";
export default defineConfig({ optimizeDeps: { exclude: ["@evenfall/gba"] } });
```

Other bundlers that handle `new URL(..., import.meta.url)` and module Workers, such as webpack 5, should work the same way but are untested.
If yours doesn't, copy `worker.js`, its imports (`core.js`, `presentation.js`, `devtools/`) and the `.wasm` files to your static files, and pass their URLs:

```ts
const emulator = await Emulator.create({
  workerUrl: new URL("/evenfall/worker.js", location.href),
  wasmUrl: new URL("/evenfall/evenfall-gba.wasm", location.href),
});
```

A module Worker must come from the same origin as the page, so loading the SDK from a CDN also needs a same-origin `workerUrl`.
Hosting the package directory yourself is simpler.

### Without a bundler

The package is plain ES modules.
Serve `web/gba/` from a checkout after building the core, and import it by path or through an import map:

```html
<script type="importmap">
  { "imports": { "@evenfall/gba": "/vendor/evenfall/emulator.js" } }
</script>
<script type="module">
  import { Emulator } from "@evenfall/gba";
  const emulator = await Emulator.create();
</script>
```

The default Worker and WebAssembly URLs are relative to `emulator.js`, so they resolve inside the served directory.
Serve `.wasm` files as `application/wasm` so browsers can compile them while downloading.

## Node and headless use

`@evenfall/gba/core` exports `WasmCore`, a synchronous wrapper around one WebAssembly instance.
The Worker uses it internally, and it suits test harnesses, command-line tools and servers.
Without an argument, `WasmCore.create()` reads the packaged `evenfall-gba.wasm` from disk in Node and fetches it elsewhere.
It also accepts bytes or a URL.

```js
import { CYCLES_PER_FRAME, WasmCore } from "@evenfall/gba/core";

const core = await WasmCore.create();
try {
  // mov r0, #42; b .
  const start = core.load(
    new Uint8Array([42, 0, 160, 227, 254, 255, 255, 234]),
  );
  const result = core.runUntil(start.cycle + CYCLES_PER_FRAME);
  console.log(result.reason, core.inspect().registers[0]); // 42
  const pixels = core.frame(); // Uint16Array(38400), 15-bit BGR
  const audio = core.drainAudio(); // interleaved stereo Int16
} finally {
  core.dispose();
}
```

`WasmCore` methods run to completion on the calling thread and throw on invalid input.
Drive one instance from one caller at a time.
Before `load`, methods that need a game throw `NoSession`.
`dispose()` releases the game, after which `load` can be called again.

`Emulator` needs a browser module Worker.
In Node, use `WasmCore` directly or inside your own `worker_threads` worker.
Besides everything `Emulator` offers, `WasmCore` has `currentCycle()` and `hasFault()` as cheap checks, `setVideoInterval()` and `presentationFrame()` for skipping frames, and `setAudioOutput()`.
The [developer tools](#recording-and-developer-tools) are built on `WasmCore`.

## The `Emulator` API

Each `Emulator` owns one Worker and one WebAssembly instance, and sessions share nothing.
Every method returns a promise, and calls run in order through the session's queue.
Returned bytes, pixels and samples are always copies you own, never views into WebAssembly memory.
Cycles are unsigned 64-bit `bigint` values, so don't convert long-running counters to `number`.

| Group                   | Methods                                                                                                                                                             |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Lifecycle               | `Emulator.create(options?)`, `load(image, options?)`, `dispose()`, `terminate(message?)`                                                                            |
| Playing                 | `play()`, `pause()`, `setSpeed(multiplier)`, `setKeys(mask)`                                                                                                        |
| Running by hand         | `runUntil(deadline, inputs?)`, `step()`                                                                                                                             |
| Cheats                  | `setCheats(cheats)`, `cheats()`, `parseCheat(code, format?)`                                                                                                        |
| Looking inside          | `inspect()`, `frame()`, `readMemory(address, length)`, `captureMemory()`, `exportRom()`                                                                             |
| Saves and states        | `exportSave()`, `reset()`, `resetWithSave(save)`, `saveState()`, `loadState(bytes)`                                                                                 |
| Rewind                  | `setRewindConfig(...)`, `rewind(steps?)`, `rewindDepth()`, `rewindCheckpoints()`, `rewindFrame(index)`, `saveRewindBundle()`, `loadRewindBundle(bytes)`             |
| Recording and scenarios | `recording(enabled?)`, `newSegment()`, `checkpoint(name)`, `capture(note?)`, `exportScenario(name?)`, `runScenario(manifest, assets)`, `replay(bundle)`, `cancel()` |
| Buffers                 | `recyclePresentation(pixels?, audio?)`                                                                                                                              |

`Emulator.create(options)` accepts `workerUrl` and `wasmUrl`, and `diagnosticsIntervalMs`.
The last one sets how often a frame event carries a fresh CPU snapshot: the default, 0, means every frame, and larger values reuse a cached one.

### Loading a game

`load(image, options)` takes a raw ROM, or a ZIP holding exactly one `.gba` file (at most 40 MiB for the file, 32 MiB for the ROM inside), and resolves to a `Snapshot`.
It replaces any game already loaded, and the new one starts paused.

| Option       | Meaning                                                                   |
| ------------ | ------------------------------------------------------------------------- |
| `bios`       | a 16 KiB BIOS image, or omit it for Evenfall's own BIOS                   |
| `save`       | the game's save bytes, sized for its save type                            |
| `saveType`   | `"auto"` (detect it, the default) or a type from `SAVE_TYPES`             |
| `zipped`     | force ZIP handling on or off, detected from the `PK` signature by default |
| `rtc`        | force the cartridge clock on or off, detected by default                  |
| `rtcEpoch`   | the clock's start time in Unix seconds, as a `bigint`, default `0n`       |
| `sampleRate` | the audio output rate, 8,000 to 192,000 Hz, default 48,000                |

Save types and how they are detected are described in [`cartridge.md`](gba/hardware/cartridge.md#save-type-detection).
`reset()` restarts the same ROM and BIOS and keeps the current save.
`resetWithSave(save)` restarts with other save bytes.

A `Snapshot` reports `cycle`, `frames` (completed frames), the 16 `registers`, `cpsr`, `nextInstructionAddress`, `romHash` (the ROM's SHA-1, after unzipping), `rtcEnabled`, the detected `saveType`, and `fault` (a message, or `null`).

`KEYS` maps the button names (`a`, `b`, `select`, `start`, `right`, `left`, `up`, `down`, `r`, `l`) to bits.
Combine them into a mask for `setKeys` or for timed inputs.
`CYCLES_PER_FRAME` is 280,896 cycles of the 16,777,216 Hz clock.

### Running by hand

`runUntil` runs a paused game to a given cycle, pressing buttons at exact cycles along the way:

```ts
import { CYCLES_PER_FRAME, KEYS } from "@evenfall/gba";

const start = await emulator.load(rom);
// Run two emulated seconds, pressing Start at frame 10 for five frames.
await emulator.runUntil(start.cycle + 120n * CYCLES_PER_FRAME, [
  { at: start.cycle + 10n * CYCLES_PER_FRAME, keys: KEYS.start },
  { at: start.cycle + 15n * CYCLES_PER_FRAME, keys: 0 },
]);
const pixels = await emulator.frame();
```

This is how tests and tools drive the emulator, and the same inputs always give the same result.

### Reading memory

`captureMemory()` returns copies of EWRAM and IWRAM taken in one step of the Worker, with the `romHash`, `cycle`, `timeline` and a `sequence` number, or `null` before a game is loaded.
It doesn't advance the game or touch I/O registers or the save chip.
Loading, resetting, loading a state and rewinding start a new `timeline`, while pausing and changing speed don't.
[Game bridges](#game-bridges) are built on it.

## Events

`Emulator` is an `EventTarget`, and `EmulatorEventMap` types the listeners.

- **`frame`** carries `pixels` (240x160, 15-bit BGR) and interleaved stereo `audio`, plus `state`, `workMs`, `emulatedWorkFrames`, `videoFrameId`, `videoFrameCycle`, `memoryBytes`, `presentationAllocations` and `generation`.
- **`clear`** says that frames and audio already queued are stale, after a pause, a speed change, a load, a reset, a state load or a rewind.
  Drop queued audio.
  It carries the current `timeline`.
- **`stopped`** says play stopped on STOP or a fault, with the `reason` and `state`.
- **`error`** carries a message about a failure in the adapter or Worker.
- **`close`** says the session ended, through `dispose`, `terminate` or a Worker crash.
- **`capture`**, **`progress`** and **`artifact`** carry failure bundles, scenario progress and scenario files.

`play()` sends `frame` events at the emulated rate.
At most two frames wait for the page at a time, so a hidden or slow page can't build up a backlog.
`workMs` is the Worker's emulation and resampling time averaged over `emulatedWorkFrames`, not the delay until the frame is shown.
After drawing, `recyclePresentation(pixels, audio)` hands the buffers back to a small pool so they can be reused.
It is optional.

## How calls behave

- A request to run by hand covers at most 600 emulated frames and 1,024 inputs.
  Memory reads are limited to 1 MiB, and the queue holds 128 requests.
- Inputs use absolute cycles, in non-decreasing order, and can't be in the game's past.
  Inputs at the same cycle keep their order, and results report `inputsConsumed`.
- The Worker runs a long request in slices and yields between them.
  An input that falls inside an instruction crossing a slice boundary still takes effect in that instruction.
- An instruction can't be split, so `runUntil` can return `instruction_boundary_after_deadline` at a cycle past the one asked for.
  The limit is emulated time, not a guarantee on wall-clock time for an unusually long instruction or BIOS call.
- `step()` returns `executed`, `halted`, `power_stopped` or `faulted`.
  While halted, running advances the devices, and STOP freezes time.
- An invalid ROM, BIOS, setting or state leaves the previous game untouched.
  A guest fault sticks: `play()` fails on a faulted game, and so does saving a state from it.
- `terminate()` stops even a session that has stopped responding, at once.
- The API is for trusted code.
  It doesn't sandbox untrusted JavaScript.

Failed calls reject with an `Error` whose message names the cause:

| Message                                                                          | Cause                                                             |
| -------------------------------------------------------------------------------- | ----------------------------------------------------------------- |
| `NoSession`                                                                      | the call needs a loaded game                                      |
| `SessionBusy`                                                                    | a load, manual run, rewind, scenario or replay is already running |
| `PauseBeforeManualExecution`                                                     | `runUntil` or `step` while playing                                |
| `PauseBeforeRewind`, `PauseBeforeLoadingState`, `PauseBeforeLoadingRewindBundle` | `rewind`, `loadState` or `loadRewindBundle` while playing         |
| `MachineFaulted`                                                                 | `play()` or saving a state after a fault                          |
| `CommandQueueFull`                                                               | 128 requests are already waiting                                  |
| `SessionDisposed`                                                                | the session was disposed or terminated                            |

Load and state errors carry the core's error name, for example `NoGbaInZip`, `SaveTypeConflict` or `SaveStateRomMismatch`.

## Cheats

`setCheats(cheats)` turns on a list of cheat codes and resolves to the active list in canonical form.
Each entry is `{ code, format? }`, and the result is a list of `{ format, code }`.
`format` defaults to `"auto"`, which detects it, or names one of `CHEAT_FORMATS`:

- `raw`: one address and value per line, like `02025E90:0F423F00`
- `gameshark` and `gameshark-decrypted`: GameShark Advance and Action Replay v1/v2 codes, encrypted or raw
- `action-replay` and `action-replay-decrypted`: Action Replay v3 codes, also sold as GameShark v3, encrypted or raw
- `codebreaker`: CodeBreaker and GameShark SP codes

```ts
await emulator.setCheats([
  { code: "D8BAE4D9 4864DCE5\nA86CDBA5 19BA49B3" }, // Emerald master (AR v3)
  { code: "C051CCF6 975E8DA1" }, // Infinite money
  { format: "codebreaker", code: "82005274 0001" },
]);
```

An empty list turns every cheat off.
`cheats()` returns the active list.
`parseCheat(code, format?)` detects and normalizes one code without touching the session, which suits a cheat editor.
The SDK keeps no names and no disabled cheats: your page owns its cheat list and passes the enabled ones.

A change takes effect at the next VBlank start.
Cheats then run at the first instruction boundary at or after every VBlank start, so results don't depend on speed or how the worker slices time.
`setCheats` works while playing.
An invalid code rejects with `InvalidCheat: cheat N: line L: ...`.
ROM patch codes and GameShark v1/v2 `DEADFACE` codes are not supported.
When several formats read a code plausibly, detection fails and asks you to name one.

The active cheats are part of the machine state.
`saveState()` includes them.
`loadState()`, `rewind()` and rewind bundles restore the cheats their state was saved with, and a state saved without cheats turns them off.
Call `cheats()` after these to refresh what your page shows.
`reset()` keeps the cheats, and `load()` starts without any.
Recordings note each change at its cycle, and exported scenarios list them in an optional `cheats` field.
[`design/cheats.md`](design/cheats.md) describes the formats, timing and evidence.

## Speed and fast-forward

`setSpeed(multiplier)` sets how fast play runs, from 0.5x to 16x (default 1x), as fast as the host can manage.
It doesn't change `runUntil`, emulated cycles, scenarios or states.

Above 2x, the runtime stops producing audio, while the sound hardware keeps running, and draws one picture every `ceil(speed)` frames.
`frame` events then carry only those pictures, identified by `videoFrameId` and `videoFrameCycle`.
Dropping to 2x or pausing brings audio back.
Drop any audio you had queued when going above 2x.
`saveState()`, `frame()` and failure captures still get the exact current picture ([selective video](research/selective-video.md)).

## Audio

`@evenfall/gba/audio-worklet` registers an AudioWorklet processor named `evenfall-audio`, a bounded stereo buffer that smooths speed changes.
Load it by URL, then send it each frame's samples:

```ts
// Vite: "?url&no-inline" keeps the worklet a separate file.
import workletUrl from "@evenfall/gba/audio-worklet?url&no-inline";

const context = new AudioContext();
await context.audioWorklet.addModule(workletUrl);
const node = new AudioWorkletNode(context, "evenfall-audio", {
  numberOfInputs: 0,
  outputChannelCount: [2],
});
node.connect(context.destination);
emulator.addEventListener("frame", ({ detail }) =>
  node.port.postMessage({ samples: detail.audio }),
);
emulator.addEventListener("clear", () =>
  node.port.postMessage({ clear: true }),
);
```

Load the game with `sampleRate: context.sampleRate`, so no second resampling is needed.

The processor accepts:

- `{ samples: Int16Array }`, interleaved stereo.
  Add `recycle: true` to get the buffer back as `{ recycled }`.
- `{ speed }`, from 0.5 to 8.
- `{ clear: true }`.

It starts playing once about 50 ms of audio is queued.
When more arrives than it holds, it drops its queue instead of playing old sound.
About once a second it reports `{ queued, underruns, overruns }`.
Sample values are on the sound chip's scale, so divide by 512 to get floats in `[-1, 1]`, as the worklet does.

## Rewind

Rewind is off until `setRewindConfig()` turns it on.
The defaults are a 15 MiB budget (at most 64 MiB, and 0 turns rewind off), one checkpoint per 60 frames at 1x, and delta compression.
As the budget fills, older history thins out while recent checkpoints stay dense.
Pass a budget, an interval (1 to 65,535 frames) or a compression (`"off"`, `"fast"` or `"delta"`) to change them.
Changing the settings clears history.
[`design/rewind.md`](design/rewind.md) explains how checkpoints are kept and compressed, and which settings to change.

While paused:

- `rewindDepth()` says how much history there is.
- `rewind(steps)` (1 to 1,024) goes back to an earlier checkpoint, publishes its frame and starts a new timeline.
- `rewindCheckpoints()` lists the checkpoints oldest first, with their frame, cycle, and the `steps` value that restores each.
- `rewindFrame(index)` decodes one checkpoint's picture without changing the game, the audio, the timeline or the history.
  It is meant for showing a timeline of previews.

Indices are only valid for the timeline they came with, so discard results that arrive after a load, reset, rewind or settings change.

A rewind bundle saves the whole session: the exact current state, every checkpoint and the settings.

```ts
await emulator.pause();
const outgoing = await emulator.saveRewindBundle();
await emulator.rewind(10);
// Later: return to where we were, with all its rewind points.
await emulator.loadRewindBundle(outgoing);
```

`saveRewindBundle()` doesn't advance time.
`loadRewindBundle(bytes)` needs the game paused and the same ROM and BIOS loaded.
It checks the whole bundle before replacing anything, and a failure leaves both the game and the history as they were.
Success starts a new timeline, sends `clear` and the restored frame, and starts a new recording segment.
`saveState()` and `loadState()` handle the current state only, and `loadState()` clears rewind history.

Bundles contain no ROM or BIOS, and can reach 68 MiB.
Loading one doesn't change your `Uint8Array`.
WebAssembly memory starts at 16 MiB and can grow to 512 MiB, and neither number is the rewind budget.

## Persistence

The SDK keeps nothing between sessions.
Storing saves, states, rewind bundles and settings is up to your page.

- **The game's save** (`exportSave`, the `save` load option, `resetWithSave`) is the contents of the cartridge's save chip.
  It works with the native runner's `.sav` files.
  Once the save type is known, imported files may be longer than needed, and only the chip's bytes are used.
  Exports don't include the clock, so the clock restarts from `rtcEpoch` at every load.
- **Save states** (`saveState`) capture the whole console, clock and sensors included, but only load with the exact same ROM and BIOS ([`design/save-states.md`](design/save-states.md)).
- **Rewind bundles** add rewind history to a state, as above.
- The clock starts at time zero by default, so runs are reproducible.
  Pass the current time as `rtcEpoch` for a real clock.
  Recordings and scenarios keep the value that was used.

## Recording and developer tools

Every `Emulator` session records play by default: timed inputs for `exportScenario()`, and bounded snapshots and history for `capture()`.
A guest fault during play captures a failure bundle automatically and sends it as a `capture` event.

- Loading, resetting and loading a state start a fresh recording, with recording on.
  Rewinding or loading a rewind bundle starts a new segment.
- `recording(false)` turns recording off until the next load, reset or state load, and `newSegment()` starts a new segment from the current state.
- `runScenario()` and `replay()` run in the session's Worker, replace its game state, and end its recording until the next load, reset or state load.
  `cancel()` stops them.

The devtools modules also work directly on a `WasmCore`:

```js
import { WasmCore } from "@evenfall/gba/core";
import { runScenario } from "@evenfall/gba/devtools/scenario";

const core = await WasmCore.create();
const result = await runScenario(core, manifest, { rom: romBytes });
console.log(result.status, result.failures);
core.dispose();
```

[`devtools.md`](devtools.md) describes the scenario and bundle formats, the command-line tools, session recording and the capture budgets.

## Game bridges

`@evenfall/gba/bridge` turns captured memory into typed, read-only game data, such as a party list or the current map, for a page to display.
A game definition pairs exact ROM builds (by SHA-1) with typed memory locations and pure `decode` functions.
`BridgeHost` samples a session with `captureMemory()` at most 20 times a second while something is watching, and shares decoded results with subscribers.
`@evenfall/gba/bridge/react` adds `BridgeProvider` and the `useViews(game)` and `useBridge(game, key)` hooks.

```ts
import { BridgeHost, defineGame, ref, u8 } from "@evenfall/gba/bridge";

const game = defineGame({
  id: "example",
  profiles: [{ romHash, source, symbols: { lives: ref(0x02000010, u8) } }],
  bridges: {
    lives: {
      id: "lives",
      decode: ({ memory, symbols }) => ({
        status: "ready",
        data: memory.read(symbols.lives),
      }),
    },
  },
});
const host = new BridgeHost(emulator);
const store = host.observe(game);
store.subscribe(() => console.log(store.getSnapshot().lives), "lives");
```

Most real games are written as a family (`defineFamily`) with one JSON manifest per ROM build (`createGame`), instead of the lower-level `defineGame` above.
[`game-bridges.md`](game-bridges.md) explains how.

## The WebAssembly interface

`evenfall-gba.wasm` exports `ef_*` functions.
They are an internal interface between the binary and this package's JavaScript, and can change in any release.
`ef_version()` identifies it, and `WasmCore` refuses a binary it doesn't understand ("Unsupported WASM ABI").
Always use the `evenfall-gba.wasm` built with this package version, and build on the JavaScript API.
The binary carries no debug information.
[`design/web-runtime.md`](design/web-runtime.md) describes the interface, memory and Worker.

## Versioning

The package follows semantic versioning with the 0.x rules: until 1.0, a minor release (0.x.0) may change the API, and a patch release doesn't.
The package version, the WebAssembly interface version and the save-state format version are independent.

## Building from source

In a checkout, with the pinned Rust toolchain and Node 24:

```sh
npm ci
npm run build:wasm -w @evenfall/gba
npm run check
npm test
```

The Rust build writes `web/gba/evenfall-gba.wasm`, kept out of Git.
`npm run build -w @evenfall/gba` compiles the bridge's TypeScript (`web/gba/src/bridge`) into `web/gba/bridge/`.
The package's `evenfall-source` export condition gives code in this repository the bridge's sources.
Its `prepack` step checks that the WebAssembly file is present, stripped and compatible, although the package isn't published.

[`testing.md`](testing.md) lists the SDK's tests, including the native-against-WebAssembly comparison that can run on your own ROMs.

## Known limits

- Only buttons are exposed.
  Light, gyro and tilt cartridges run with fixed sensor values (light level 128, gyro `0x6C0`, tilt `0x3A0`, `0x3A0`), and rumble isn't reported.
- Each session emulates one GBA, and the SDK has no link cable.
- The game's save doesn't include the clock ([Persistence](#persistence)).
- `runUntil` limits emulated time, not wall-clock time.
- The SDK doesn't sandbox untrusted JavaScript or untrusted bridge code.
