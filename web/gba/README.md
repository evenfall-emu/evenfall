# @evenfall/gba

The JavaScript/TypeScript SDK for Evenfall's cycle-aware Game Boy Advance core.
`Emulator` runs in a browser Worker; `WasmCore` is the synchronous adapter for
Node and other hosts. The SDK handles no canvas, audio device or persistence.
The Rust WebAssembly core is not built yet; until phase 2 supplies
`evenfall-gba.wasm`, calls that instantiate it cannot run.

```js
import { Emulator, CYCLES_PER_FRAME, KEYS } from "@evenfall/gba";

const emulator = await Emulator.create();
const start = await emulator.load(new Uint8Array(await romFile.arrayBuffer()));
await emulator.runUntil(start.cycle + 60n * CYCLES_PER_FRAME, [
  { at: start.cycle + 10n * CYCLES_PER_FRAME, keys: KEYS.start },
]);
const pixels = await emulator.frame(); // Uint16Array, 240×160 BGR555
const state = await emulator.saveState();
await emulator.dispose();
```

Other entry points: `@evenfall/gba/core`, `@evenfall/gba/devtools/scenario`,
`@evenfall/gba/devtools/capture`, `@evenfall/gba/bridge`,
`@evenfall/gba/bridge/react` and `@evenfall/gba/audio-worklet`.

## Differences from ZiGBA's `@evenfall/gba`

- The default WASM asset and exported asset path are `evenfall-gba.wasm` rather
  than `zigba.wasm`, because this binary will be built by the Rust core. Explicit
  `wasmUrl` and `WasmCore.create(source)` overrides still work as before.
- The package is private and is not published. For now it is consumed from the
  `web/*` npm workspace. The JS API, entry points, events, options, errors and
  TypeScript interfaces retain their ZiGBA names and behavior; `zigba.js` and
  the internal `zigba-audio` processor name remain unchanged for compatibility.
- The scenario and capture CLIs are under `web/gba/scripts/` instead of ZiGBA's
  root `tools/`; their optional WASM argument and manifest formats are unchanged.

No other public API differences are intended. The exact core behavior and
binary parity will be verified when the Rust WASM adapter exists.

## Development

From the repository root, `npm ci`, `npm run check` and `npm test` check the
SDK. Tests requiring a missing `web/gba/evenfall-gba.wasm` skip with an explicit
reason. `npm run build -w @evenfall/gba` compiles bridge sources into the
gitignored `web/gba/bridge/` directory; the `zigba-source` condition exposes
their TypeScript sources to in-repository consumers without compiling first.
The eventual Rust WASM build will write the gitignored binary to
`web/gba/evenfall-gba.wasm`. `prepack` still refuses a missing, debug or
ABI-incompatible binary, and the package remains private.

Scenario and capture CLIs live at `web/gba/scripts/run-scenario.mjs` and
`web/gba/scripts/replay-capture.mjs`. They use the same default binary path and
accept an explicit WASM path as their final argument.
The Zig-only allocator probe and native `zigba-session-probe` comparison are
not part of this JS port; Rust allocator and native/WASM parity tests belong to
the phase-2 ABI crate.

## License and disclaimer

GNU Affero General Public License, version 3 or later; see [LICENSE](LICENSE).
BIOS images and games are user-supplied. Evenfall is not affiliated with or
endorsed by Nintendo.
