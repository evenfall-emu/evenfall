# @evenfall/gba

The JavaScript/TypeScript SDK for Evenfall's cycle-aware Game Boy Advance core.
`Emulator` runs in a browser Worker; `WasmCore` is the synchronous adapter for
Node and other hosts. The SDK handles no canvas, audio device or persistence.
Build the Rust core with `npm run build:wasm -w @evenfall/gba` before calling
`Emulator.create()` or `WasmCore.create()` without an explicit WASM source.

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
  than `zigba.wasm`, because this binary is built by the Rust core. Explicit
  `wasmUrl` and `WasmCore.create(source)` overrides still work as before.
- The package is private and is not published. For now it is consumed from the
  `web/*` npm workspace. The JS API, entry points, events, options, errors and
  TypeScript interfaces retain their ZiGBA names and behavior; `zigba.js` and
  the internal `zigba-audio` processor name remain unchanged for compatibility.
- The scenario and capture CLIs are under `web/gba/scripts/` instead of ZiGBA's
  root `tools/`; their optional WASM argument and manifest formats are unchanged.

No other public API differences are intended. The SDK exercises the Rust
WASM adapter and compares its replay output with native Rust execution.

## Development

From the repository root, run `npm ci`, `npm run check`,
`npm run build:wasm -w @evenfall/gba`, then `npm test` to exercise the actual
WASM ABI. Tests requiring a missing `web/gba/evenfall-gba.wasm` skip with an
explicit reason; a full test run must build the binary first. The native/WASM
parity test builds the native `session_probe` and compares canonical state,
memory, frame, audio and save hashes after the same bounded schedule.
`npm run build -w @evenfall/gba` compiles bridge sources into the
gitignored `web/gba/bridge/` directory; the `zigba-source` condition exposes
their TypeScript sources to in-repository consumers without compiling first.
The Rust WASM build writes the gitignored binary to
`web/gba/evenfall-gba.wasm`. `prepack` still refuses a missing, debug or
ABI-incompatible binary, and the package remains private.

The Rust wasm32 default allocator is used rather than a custom page allocator:
an isolated wasm32 probe showed near-exact growth for a large allocation and
address reuse across repeated free/allocate cycles. `allocator.test.mjs`
checks these properties on the shipped module with a request just across a
page boundary. The module has a 16 MiB initial memory, a 512 MiB maximum,
an 8 MiB stack and no imports.
ZiGBA's separate page-allocator probe checks its own bitmap, in-place page-run
resize and OOM recovery; those implementation-specific assertions do not apply
to Rust's dlmalloc. The shipped-module test instead checks small and large
allocation reuse, while SDK tests check that rejected uploads leave a live
session and its ROM intact. The scenario and pooled-audio tests use `wasmTest`
to run their original assertions whenever the binary is present.

Scenario and capture CLIs live at `web/gba/scripts/run-scenario.mjs` and
`web/gba/scripts/replay-capture.mjs`. They use the same default binary path and
accept an explicit WASM path as their final argument.
The Zig-only allocator probe and native `zigba-session-probe` are replaced by
the allocator test and Rust `session_probe` parity run.

## License and disclaimer

GNU Affero General Public License, version 3 or later; see [LICENSE](LICENSE).
BIOS images and games are user-supplied. Evenfall is not affiliated with or
endorsed by Nintendo.
