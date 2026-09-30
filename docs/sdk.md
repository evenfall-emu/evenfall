# JavaScript and TypeScript SDK

`@evenfall/gba` is Evenfall's Rust WebAssembly GBA core with a JavaScript
interface. It runs the same deterministic emulator as the
native build, in a browser Worker or synchronously in Node. It has no runtime
dependencies and creates no canvas or audio device. The package is private and
consumed from this repository's `web/*` npm workspace, not published to npm.

The SDK is experimental and versioned 0.x: minor releases can change the
interface (see [Versioning](#versioning)).

- [Install](#install) · [Entry points](#entry-points)
- [Use with a bundler](#use-with-a-bundler) ·
  [Without a bundler](#without-a-bundler) ·
  [Node and headless use](#node-and-headless-use)
- [The `Emulator` API](#the-emulator-api) · [Events](#events) ·
  [Execution and failure contract](#execution-and-failure-contract)
- [Speed and presentation](#speed-and-presentation) · [Audio](#audio) ·
  [Rewind and rewind bundles](#rewind-and-rewind-bundles) ·
  [Persistence](#persistence)
- [Recording and developer tools](#recording-and-developer-tools) ·
  [Game bridges](#game-bridges) · [The WASM ABI](#the-wasm-abi) ·
  [Versioning](#versioning)
- [Building from source](#building-from-source) · [Limits](#limits)

## Install

From the repository root, run `npm ci` and build the Rust WASM core with
`npm run build:wasm -w @evenfall/gba`. The private workspace package includes
ES modules, TypeScript declarations and two builds of the core (gitignored):
`evenfall-gba.wasm`, which uses WebAssembly SIMD, and `evenfall-gba-scalar.wasm`
for engines without it.
Browsers need module Workers and WebAssembly with BigInt integration; serve
pages over HTTPS or `localhost`. Cross-origin isolation is not required.
Headless use needs Node 24 or later.

Evenfall includes no BIOS images or games. Your application supplies ROM bytes,
for example from a file the user opens; the SDK never uploads them.

The SDK is licensed under the
[GNU Affero General Public License, version 3 or later](../LICENSE). An
application or service that includes it, whether distributed or offered over a
network, must be released under the same license with its source code. For
other licensing terms, contact
[contact@evenfall-emu.org](mailto:contact@evenfall-emu.org).

### Entry points

| Import                                   | Contents                                                                                                                                              |
| ---------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| `@evenfall/gba`                          | `Emulator` (asynchronous, one Worker per session), `KEYS`, `CYCLES_PER_FRAME`, `SAVE_TYPES`, and the shared types                                     |
| `@evenfall/gba/core`                     | `WasmCore`: the synchronous single-instance adapter for Node and Workers                                                                              |
| `@evenfall/gba/devtools/scenario`        | `runScenario`, `validateScenario`, `sha1`, `sha256`, `jsonSafe`                                                                                       |
| `@evenfall/gba/devtools/capture`         | `FailureRecorder`, `validateFailureBundle`, `replayFailureBundle`, `replayFailureBundleAsync`                                                         |
| `@evenfall/gba/bridge`                   | typed read-only game bridges: `defineFamily`, `createGame`, `defineGame`, `resolveGame`, `decodeGame`, `BridgeHost`, `createMemoryReader` and layouts |
| `@evenfall/gba/bridge/react`             | `BridgeProvider`, `useViews`, `useBridge`, `useBridgeResources` (React 18+, an optional peer dependency)                                              |
| `@evenfall/gba/audio-worklet`            | the `zigba-audio` AudioWorklet processor, loaded by URL                                                                                               |
| `@evenfall/gba/evenfall-gba.wasm`        | the WebAssembly binary, built with SIMD                                                                                                               |
| `@evenfall/gba/evenfall-gba-scalar.wasm` | the WebAssembly binary without SIMD, for engines that lack it                                                                                         |

Every JavaScript entry point has declarations. The sections below summarize
the interface; [`zigba.d.ts`](../web/gba/zigba.d.ts),
[`core.d.ts`](../web/gba/core.d.ts) and the
[devtools declarations](../web/gba/devtools/) are the reference.

## Use with a bundler

```ts
import { CYCLES_PER_FRAME, Emulator, KEYS } from "@evenfall/gba";

const emulator = await Emulator.create();
const rom = new Uint8Array(await file.arrayBuffer());
const start = await emulator.load(rom);
// Run two emulated seconds, pressing Start at frame 10 for five frames.
await emulator.runUntil(start.cycle + 120n * CYCLES_PER_FRAME, [
  { at: start.cycle + 10n * CYCLES_PER_FRAME, keys: KEYS.start },
  { at: start.cycle + 15n * CYCLES_PER_FRAME, keys: 0 },
]);
const pixels = await emulator.frame(); // 240×160 BGR555
await emulator.dispose();
```

`Emulator.create()` starts the package's `worker.js` with
`new Worker(new URL("./worker.js", import.meta.url), { type: "module" })` and
passes it `new URL("./evenfall-gba.wasm", import.meta.url)`, or
`new URL("./evenfall-gba-scalar.wasm", import.meta.url)` when
`WebAssembly.validate` reports no SIMD support. Bundlers that understand these
standard forms emit the worker and both modules with your application and
rewrite the URLs; no configuration or copying is needed.

The Vite development server pre-bundles dependencies into `node_modules/.vite`,
where those relative URLs do not resolve, so exclude the SDK from pre-bundling:

```js
// vite.config.js
import { defineConfig } from "vite";
export default defineConfig({ optimizeDeps: { exclude: ["@evenfall/gba"] } });
```

Other bundlers that handle `new URL(..., import.meta.url)` and module Workers,
such as webpack 5, should work the same way but are untested. If yours does
not, copy `worker.js`, its imports (`core.js`, `presentation.js`, `devtools/`)
and `evenfall-gba.wasm` (plus `evenfall-gba-scalar.wasm` if you support
engines without WebAssembly SIMD) to your static files and pass their URLs:

```ts
const emulator = await Emulator.create({
  workerUrl: new URL("/zigba/worker.js", location.href),
  wasmUrl: new URL("/zigba/evenfall-gba.wasm", location.href),
});
```

Module Workers must be same-origin with the page. Loading the SDK from a CDN
therefore also needs a same-origin `workerUrl`; self-hosting the package
directory is simpler.

## Without a bundler

The package is plain ES modules. Serve `web/gba/` in a checkout after building
the WASM core and import it by path or through an import map:

```html
<script type="importmap">
  { "imports": { "@evenfall/gba": "/vendor/zigba/zigba.js" } }
</script>
<script type="module">
  import { Emulator } from "@evenfall/gba";
  const emulator = await Emulator.create();
</script>
```

The default worker and WASM URLs are relative to `zigba.js`, so they resolve
inside the served directory. Serve `.wasm` as `application/wasm` so browsers
can compile it while downloading.

## Node and headless use

`@evenfall/gba/core` exports `WasmCore`, a synchronous adapter over one WASM
instance. It is what the Worker uses internally, and it suits test harnesses,
CLI tools and servers. Without an argument, `WasmCore.create()` reads the
packaged `evenfall-gba.wasm` from disk in Node and fetches it elsewhere; it also
accepts bytes or a URL.

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
  const pixels = core.frame(); // Uint16Array(38400), BGR555
  const audio = core.drainAudio(); // interleaved stereo Int16
} finally {
  core.dispose();
}
```

`WasmCore` methods run to completion on the calling thread and throw on invalid
input; drive one instance from one caller at a time. Before `load`, methods
that need a machine throw `NoSession`; `dispose()` releases the machine, after
which `load` may be called again. `Emulator` needs a browser module Worker; in
Node, use `WasmCore` directly or inside your own `worker_threads` worker.
Besides the methods `Emulator` forwards, `WasmCore` has `currentCycle()` and
`hasFault()` as cheap probes, `setVideoInterval()` and `presentationFrame()`
for frame skipping, and `setAudioOutput()`. The
[developer tools](#recording-and-developer-tools) wrap `WasmCore`.

## The `Emulator` API

Each `Emulator` owns one Worker and one WASM instance; sessions share no
machine state. Every method returns a promise, and calls are serialized through
the session's command queue. Returned byte, pixel and sample arrays are owned
copies, never live views of WASM memory. Cycles are unsigned 64-bit `bigint`
values: do not convert long-running counters to `number`.

| Group                   | Methods                                                                                                                                                             |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Lifecycle               | `Emulator.create(options?)`, `load(image, options?)`, `dispose()`, `terminate(message?)`                                                                            |
| Realtime play           | `play()`, `pause()`, `setSpeed(multiplier)`, `setKeys(mask)`                                                                                                        |
| Manual execution        | `runUntil(deadline, inputs?)`, `step()`                                                                                                                             |
| Observation             | `inspect()`, `frame()`, `readMemory(address, length)`, `captureMemory()`, `exportRom()`                                                                             |
| Saves and state         | `exportSave()`, `reset()`, `resetWithSave(save)`, `saveState()`, `loadState(bytes)`                                                                                 |
| Rewind                  | `setRewindConfig(...)`, `rewind(steps?)`, `rewindDepth()`, `rewindCheckpoints()`, `rewindFrame(index)`, `saveRewindBundle()`, `loadRewindBundle(bytes)`             |
| Recording and scenarios | `recording(enabled?)`, `newSegment()`, `checkpoint(name)`, `capture(note?)`, `exportScenario(name?)`, `runScenario(manifest, assets)`, `replay(bundle)`, `cancel()` |
| Presentation buffers    | `recyclePresentation(pixels?, audio?)`                                                                                                                              |

`Emulator.create(options)` accepts `diagnosticsIntervalMs` (default 0: attach
a fresh CPU snapshot to every frame event; larger values cache it), and
`workerUrl` / `wasmUrl` overrides.

`load(image, options)` accepts a raw ROM or a ZIP holding exactly one `.gba`
entry (at most 40 MiB for the image, 32 MiB for the ROM inside) and resolves
to a `Snapshot`. The previous game, if any, is replaced; the new one starts
paused. Options:

| Option       | Meaning                                                                      |
| ------------ | ---------------------------------------------------------------------------- |
| `bios`       | a 16 KiB BIOS image; omitted means the original HLE BIOS                     |
| `save`       | raw cartridge save bytes, sized for the save type                            |
| `saveType`   | `"auto"` (default detection) or an explicit type from `SAVE_TYPES`           |
| `zipped`     | force ZIP handling on or off; detected from the `PK` signature by default    |
| `rtc`        | force the cartridge clock on or off; detected by default                     |
| `rtcEpoch`   | RTC start time in Unix seconds as `bigint`; default `0n` for reproducibility |
| `sampleRate` | host audio rate for the resampler, 8,000–192,000 Hz; default 48,000          |

Save types, their sizes and the detection order are the same as for the native
frontends; see [`cli.md`](cli.md). `reset()` reboots the same ROM and BIOS
while keeping the current save memory; `resetWithSave(save)` reboots with
replacement save bytes.

A `Snapshot` reports `cycle`, `frames` (completed frames), the 16 `registers`,
`cpsr`, `nextInstructionAddress`, `romHash` (SHA-1 of the ROM, after ZIP
extraction), `rtcEnabled`, the resolved `saveType` and `fault` (a diagnostic
string, or `null`).

`KEYS` maps button names (`a`, `b`, `select`, `start`, `right`, `left`, `up`,
`down`, `r`, `l`) to bits; combine them into a mask for `setKeys` or timed
inputs. `CYCLES_PER_FRAME` is 280,896 cycles of the 16,777,216 Hz master
clock.

`captureMemory()` returns owned EWRAM and IWRAM copies from one synchronous
worker turn, with `romHash`, `cycle`, `timeline` and `sequence`, or `null`
before a game is loaded. It neither advances execution nor reads MMIO or save
devices. Load, reset, state load and rewind replace the `timeline`; pause and
speed changes do not.

## Events

`Emulator` is an `EventTarget`; `EmulatorEventMap` types the listeners.

| Event                             | `detail`                                                                                                                                                                                  |
| --------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `frame`                           | `pixels` (240×160 BGR555), interleaved stereo `audio`, `state`, `workMs`, `emulatedWorkFrames`, `videoFrameId`, `videoFrameCycle`, `memoryBytes`, `presentationAllocations`, `generation` |
| `clear`                           | queued presentation is stale (after pause, speed changes, loads, resets, state loads and rewinds); discard queued audio. Carries the current `timeline`                                   |
| `stopped`                         | realtime play stopped on STOP or a compatibility fault; carries `reason` and `state`                                                                                                      |
| `error`                           | an adapter or worker failure message                                                                                                                                                      |
| `close`                           | the session ended (`dispose`, `terminate` or a worker crash)                                                                                                                              |
| `capture`, `progress`, `artifact` | failure bundles, scenario checkpoint progress and scenario artifacts                                                                                                                      |

`play()` emits `frame` events at the emulated rate. The worker keeps at most
two unacknowledged frames in flight, so a hidden or slow page cannot build up a
backlog. `workMs` is worker emulation and resampling time averaged over
`emulatedWorkFrames`, not total presentation latency. Audio values use the
core's DAC scale; divide by 512 for `[-1, 1]` floats, as the bundled worklet
does. The SDK never creates a canvas or audio device.
After drawing, `recyclePresentation(pixels, audio)` returns the buffers to a
small pool and detaches them; it is optional.

## Execution and failure contract

- A manual request accepts at most 600 emulated frames and 1,024 input events.
  Memory reads are capped at 1 MiB. The command queue holds 128 requests.
- Inputs use absolute cycles, must be nondecreasing and cannot predate current
  machine time. Same-cycle events keep their order. Results report
  `inputsConsumed`.
- The worker runs a long request in slices and yields between them. An input
  that falls inside an instruction crossing a slice boundary still takes
  effect in that instruction; the request's final deadline remains the cutoff
  for consuming future inputs.
- Instruction execution is atomic: `runUntil` can return
  `instruction_boundary_after_deadline` with a cycle beyond the request. The
  bound is emulated time, **not a hard wall-time guarantee** for an unusually
  expensive instruction or HLE call.
- `step()` distinguishes `executed`, `halted`, `power_stopped` and `faulted`.
  Bounded running advances eligible devices while HALTed; STOP freezes time.
- Invalid ROM, BIOS, configuration or snapshot imports leave the previous
  machine intact. Guest compatibility faults are sticky diagnostics:
  `play()` on a faulted machine fails, and snapshot writes from it fail.
- `terminate()` aborts even an unresponsive session immediately.
- The API is for trusted application code. It does not sandbox untrusted
  JavaScript.

Failed calls reject with an `Error` whose message names the cause:

| Message                                                                          | Cause                                                     |
| -------------------------------------------------------------------------------- | --------------------------------------------------------- |
| `NoSession`                                                                      | the call needs a loaded game                              |
| `SessionBusy`                                                                    | a manual run, scenario or replay is already in progress   |
| `PauseBeforeManualExecution`                                                     | `runUntil` or `step` while playing                        |
| `PauseBeforeRewind`, `PauseBeforeLoadingState`, `PauseBeforeLoadingRewindBundle` | `rewind`, `loadState` or `loadRewindBundle` while playing |
| `MachineFaulted`                                                                 | `play()` or a snapshot write after a compatibility fault  |
| `CommandQueueFull`                                                               | 128 requests are already pending                          |
| `SessionDisposed`                                                                | the session was disposed or terminated                    |

Load and state errors carry the core's error name, for example `NoGbaInZip`,
`SaveTypeConflict` or `SaveStateRomMismatch`.

## Speed and presentation

`setSpeed(multiplier)` sets realtime pacing from 0.5× to 16× (default 1×),
limited by host throughput. It does not change `runUntil`, emulated cycles,
scenario manifests or snapshots. Above 2×, the runtime stops producing host
audio while the sound hardware keeps running, and it renders one complete image
per `ceil(speed)` emulated frames; `frame` events then carry only selected
images, identified by `videoFrameId` and `videoFrameCycle`. Returning to 2× or
below, or pausing, restores audio. Hosts should discard audio above 2×,
including already queued messages. Explicit `saveState()`, `frame()` and
failure captures reconstruct exact-cycle pixels without advancing the machine.
[`research/selective-video.md`](research/selective-video.md) explains how
skipped frames stay exact.

## Audio

`@evenfall/gba/audio-worklet` registers an AudioWorklet processor named
`zigba-audio`: a bounded stereo ring buffer with linear-interpolated rate
changes. Load it by URL, then post each frame's samples:

```ts
// Vite: "?url&no-inline" keeps the worklet a separate file.
import workletUrl from "@evenfall/gba/audio-worklet?url&no-inline";

const context = new AudioContext();
await context.audioWorklet.addModule(workletUrl);
const node = new AudioWorkletNode(context, "zigba-audio", {
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

Messages to the processor: `{ samples: Int16Array }` (interleaved stereo,
optionally with `recycle: true` to get the buffer back as `{ recycled }`),
`{ speed }` (0.5–8) and `{ clear: true }`. It starts playing once about 50 ms
of audio is queued, and it drops its queue rather than play stale audio when
more arrives than it holds. It reports `{ queued, underruns, overruns }` about
once a second. Load the emulator with `sampleRate: context.sampleRate` so no
host resampling is needed.

## Rewind and rewind bundles

Rewind is off until `setRewindConfig()` enables it. The defaults are a 15 MiB
budget (at most 64 MiB; 0 disables rewind), one capture per 60 frames at 1×
and anchored delta compression. Older history thins out as the budget fills,
keeping recent checkpoints denser. Pass a budget, an interval (1–65,535
frames) or `"off" | "fast" | "delta"` compression to change them.
Reconfiguring discards history.

While paused, `rewindDepth()` reports available history and `rewind(steps)`
(1–1,024) restores an earlier checkpoint, publishes the restored frame and
replaces the timeline. `rewindCheckpoints()` lists the available points oldest
first, with exact frame and cycle timestamps and the `steps` value that
restores each. `rewindFrame(index)` decodes one retained checkpoint's
framebuffer without changing the live machine, audio queue, timeline or
history. Indices are valid only while the returned timeline is current;
discard stale asynchronous results after a load, reset, rewind or
reconfiguration.

`saveRewindBundle()` exports the exact live state plus all retained history and
its policy as owned bytes, without advancing time. `loadRewindBundle(bytes)`
requires paused playback and the same ROM and BIOS already loaded. It validates
the whole bundle before replacing state or history, and failure preserves both.
Success replaces the timeline, emits `clear` and a restored frame, and starts a
new recording segment. `saveState()` / `loadState()` remain state-only, and
`loadState()` clears rewind history.

```ts
await emulator.pause();
const outgoing = await emulator.saveRewindBundle();
await emulator.rewind(10);
// Later: return to the original position and its rewind points.
await emulator.loadRewindBundle(outgoing);
```

Bundles contain no ROM or BIOS, are versioned separately from portable states,
and can reach 68 MiB. Import leaves your `Uint8Array` unchanged. WASM memory
starts at 16 MiB and can grow to 512 MiB for fragmentation headroom; neither
number is the rewind budget. Avoid transferring whole histories per frame.
[`design/rewind.md`](design/rewind.md) describes retention, compression and
the bundle format.

## Persistence

The SDK keeps nothing between sessions. Storing saves, states, rewind bundles
and settings is the host's job.

- **Raw saves** (`exportSave`, the `save` load option, `resetWithSave`) are the
  cartridge's save memory. They interoperate with the native frontends' `.sav`
  files. When the medium's type is known, imports may have trailing bytes;
  only the cartridge bytes are used. Exports do not contain RTC state, and
  imported RTC trailers are ignored, so the clock restarts from `rtcEpoch`
  at every load.
- **Snapshots** (`saveState`) are portable, versioned states (format 6) that
  preserve RTC and peripheral state. They require the exact ROM and BIOS; see
  [`design/save-states.md`](design/save-states.md).
- **Rewind bundles** add history to a snapshot, as above.
- The RTC epoch defaults to zero for reproducible runs. Pass the current time
  as `rtcEpoch` for realtime clocks; recordings and scenarios keep the resolved
  value.

## Recording and developer tools

Every `Emulator` session records its gameplay by default: timed inputs for
`exportScenario()`, and bounded snapshots and operation history for
`capture()`. A guest fault during play captures a failure bundle automatically
and emits it as a `capture` event. Each load, reset and state load starts a
fresh recording with recording enabled; a rewind or rewind-bundle load starts
a new segment. `recording(false)` turns recording off until the next load,
reset or state load, and `newSegment()` starts a new segment from the current
state. `runScenario()`
and `replay()` run in the session's worker, replace its machine state and end
its recording until the next load, reset or state load; `cancel()` stops
them.

`@evenfall/gba/devtools/scenario` runs deterministic scenario manifests
(inputs, checkpoints and expected hashes) against a `WasmCore`.
`@evenfall/gba/devtools/capture` records bounded failure evidence around a core
and replays failure bundles with identity checks.

```js
import { WasmCore } from "@evenfall/gba/core";
import { runScenario } from "@evenfall/gba/devtools/scenario";

const core = await WasmCore.create();
const result = await runScenario(core, manifest, { rom: romBytes });
console.log(result.status, result.failures);
core.dispose();
```

[`devtools.md`](devtools.md) documents the manifest and bundle formats, the
command-line runner, session recording and the capture budgets.

## Game bridges

`@evenfall/gba/bridge` turns captured memory into typed, read-only game data. A
game definition pairs exact ROM builds (by SHA-1) with typed symbols and
pure `decode` functions; `BridgeHost` samples a session with `captureMemory()`
at most 20 times a second while observed and shares decoded results with
subscribers. `@evenfall/gba/bridge/react` adds `BridgeProvider` and the
`useViews(game)` and `useBridge(game, key)` hooks. Most games are written as a
family (`defineFamily`): sources, ROM resources and views for an overlay, with
one JSON manifest per build (`createGame`); the lower-level `defineGame` below
takes symbols and decoders directly.

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

[`game-bridges.md`](game-bridges.md) describes the contract, capture
coherence and authoring workflow.

## The WASM ABI

`evenfall-gba.wasm` exports `zb_*` functions. They
are an **internal, versioned ABI** between the binary and this package's
JavaScript, not a stable public interface: functions can change in any release.
`zb_version()` identifies the ABI, and `WasmCore` rejects a binary whose version
it does not speak ("Unsupported WASM ABI"). Always use the
`evenfall-gba.wasm` built for this package version, and build on the JavaScript
API instead of the exports. The Rust WASM build strips DWARF. The
[web runtime design](design/web-runtime.md) describes the Worker, module memory
and native/WASM parity boundary.

## Versioning

The package follows semantic versioning with 0.x rules: until 1.0, a minor
release (0.x.0) may change the public API, and a patch release does not.
The npm version, the WASM ABI version and the save-state format version are
independent.

## Building from source

In a checkout, build and check the private workspace package with the pinned
Rust toolchain and Node 24:

```sh
npm ci
npm run build:wasm -w @evenfall/gba
npm run check
npm test
```

The Rust build writes `web/gba/evenfall-gba.wasm` (gitignored).
`npm run build -w @evenfall/gba` compiles the bridge's TypeScript
(`web/gba/src/bridge`) into `web/gba/bridge/`. The package's `zigba-source`
condition exposes bridge sources to in-repository consumers. Its `prepack`
checks that the WASM is present, stripped and ABI-compatible, but the package
remains private and is not published.

[`testing.md`](testing.md) lists the SDK test suites, including the
native/WASM parity check that can compare locally supplied ROMs.

## Limits

- Only buttons are exposed. Solar, gyro and tilt cartridges run with a fixed
  sensor reading (solar level 128, gyro sample `0x6C0`, tilt `0x3A0`,
  `0x3A0`), and rumble output is not reported.
- Each session emulates one GBA; the SDK has no link-cable API.
- Raw saves do not carry RTC state (see [Persistence](#persistence)).
- `runUntil` bounds emulated time, not wall time.
- The SDK is not a sandbox for untrusted JavaScript or untrusted bridge code.
