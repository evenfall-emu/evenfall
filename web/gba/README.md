# @evenfall/gba

The JavaScript and TypeScript SDK for Evenfall's Game Boy Advance core.
`Emulator` runs the core in a browser Worker, and `WasmCore` runs it synchronously in Node and other hosts.
The SDK creates no canvas or audio device and stores nothing: the host draws, plays sound and keeps saves.

[`docs/sdk.md`](../../docs/sdk.md) is the full guide, starting with a complete player in a page.
[`docs/design/web-runtime.md`](../../docs/design/web-runtime.md) explains how the Worker, the WebAssembly module and memory fit together.

```js
import { Emulator, CYCLES_PER_FRAME, KEYS } from "@evenfall/gba";

const emulator = await Emulator.create();
const start = await emulator.load(new Uint8Array(await romFile.arrayBuffer()));
await emulator.runUntil(start.cycle + 60n * CYCLES_PER_FRAME, [
  { at: start.cycle + 10n * CYCLES_PER_FRAME, keys: KEYS.start },
]);
const pixels = await emulator.frame(); // Uint16Array, 240x160, 15-bit BGR
const state = await emulator.saveState();
await emulator.dispose();
```

Other entry points: `@evenfall/gba/core`, `@evenfall/gba/devtools/scenario`, `@evenfall/gba/devtools/capture`, `@evenfall/gba/bridge`, `@evenfall/gba/bridge/react` and `@evenfall/gba/audio-worklet`.

## What's in the package

- `emulator.js` and `emulator.d.ts`: `Emulator` and the shared types.
- `core.js`: `WasmCore`.
- `worker.js` and `presentation.js`: the Worker and its buffer pool.
- `devtools/`: scenarios, failure capture and replay.
- `bridge/`: game bridges, compiled from `src/bridge`.
- `audio-worklet.js`: the AudioWorklet processor.
- `evenfall-gba.wasm` and `evenfall-gba-scalar.wasm`: the core, with and without WebAssembly SIMD.
  The default URL picks one with `WebAssembly.validate`, and `core.js` exports the result as `wasmSimd`.
- `scripts/`: the WebAssembly build script, the scenario and replay command-line tools, and package checks.

The package is private and isn't published to npm.
It is used from this repository's `web/*` npm workspace.

## Development

From the repository root:

```sh
npm ci
npm run check
npm run build:wasm -w @evenfall/gba
npm test
```

`npm run build:wasm` writes `web/gba/evenfall-gba.wasm` and its scalar twin, both kept out of Git.
Tests that need the binary are skipped with a message when it is missing, so a full run must build it first.
Among them, the native-against-WebAssembly test builds the native `session_probe` and compares state, memory, frame, audio and save hashes after the same schedule.

`npm run build -w @evenfall/gba` compiles the bridge sources into `web/gba/bridge/`, also kept out of Git.
The `evenfall-source` export condition lets code in this repository use the bridge's TypeScript sources without compiling first.
`prepack` refuses a missing, debug or incompatible binary, although the package isn't published.

The scenario and replay tools are `scripts/run-scenario.mjs` and `scripts/replay-capture.mjs` ([`docs/devtools.md`](../../docs/devtools.md)).
Both use the same default binary and accept a WebAssembly path as their last argument.

## License and disclaimer

GNU Affero General Public License, version 3 or later ([LICENSE](LICENSE)).
BIOS images and games are supplied by the user.
Evenfall is not affiliated with or endorsed by Nintendo.
