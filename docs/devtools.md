# Developer tools

Emulator bugs are hard to report because they are hard to reproduce: "the game froze somewhere in the second dungeon" is not something another developer can run.
Evenfall is deterministic, so a bug can be turned into a file that reproduces it exactly, on any machine.
The developer tools do that, in three parts:

- **Scenarios.** A JSON file listing the ROM, the button presses with their exact cycles, and checkpoints with expected hashes.
  It runs from a cold boot and says which checkpoint failed.
- **Failure capture.** A recorder that keeps recent snapshots and operations around a running game, and packages them into a failure bundle when something goes wrong.
- **Replay.** Restoring a bundle's newest snapshot and replaying its operations, checking identities and results at every step.

The tools are JavaScript modules on top of the SDK's synchronous `WasmCore` ([`sdk.md`](sdk.md)).
They use the same WebAssembly binary as the SDK, with no special build, and don't touch the hardware model, so a reproduction runs on the exact binary that showed the problem.
The command-line tools handle files and reports, the modules handle running and capturing, and `Machine` does the emulation.
For a trace of individual instructions, use the native runner's `--trace` option ([`cli.md`](cli.md)).

Every `Emulator` session also records play by default ([below](#recording-in-emulator-sessions)).
A session can export its recording as a scenario and captures failures on its own.
A separate `Emulator` runs scenarios and replays captures in its own Worker, without touching another session.

## Quick start

From the repository root, generate and run a scenario for a tiny test program that ships with the tools.
No game or BIOS is needed:

```sh
npm ci
npm run build:wasm -w @evenfall/gba
node web/gba/scripts/run-scenario.mjs --example /tmp/evenfall-example
node web/gba/scripts/run-scenario.mjs /tmp/evenfall-example/scenario.json /tmp/evenfall-result
```

`--example [directory]` writes `loop.gba` and `scenario.json` into the directory (default `scenario-example`).
The second command runs the scenario and writes `result.json` into the output directory.
That directory must not exist yet, so earlier results are never overwritten.

```text
node web/gba/scripts/run-scenario.mjs MANIFEST [OUTPUT_DIR] [WASM_PATH]
node web/gba/scripts/run-scenario.mjs --example [DIRECTORY]
```

`OUTPUT_DIR` defaults to `scenario-result-<timestamp>`, and `WASM_PATH` to `web/gba/evenfall-gba.wasm`.
The WebAssembly file's SHA-256 is recorded as the build identity.
The runner prints a one-line JSON summary and exits with:

| Status | Meaning                                                          |
| ------ | ---------------------------------------------------------------- |
| 0      | every checkpoint passed                                          |
| 1      | the scenario failed or was cancelled                             |
| 2      | invalid manifest, missing or wrong asset, or another setup error |

These codes work in CI and with `git bisect run`, where a wrapper should turn status 2 into bisect's "skip" code, 125.

## Writing a scenario

The generated example is a complete manifest:

```json
{
  "version": 2,
  "name": "ARM constant loop",
  "assets": {
    "rom": {
      "path": "loop.gba",
      "sha1": "c9cecd1aec9e3585c00ebcbbdf7acfe5ff71c2b3"
    }
  },
  "config": {
    "boot": "hle",
    "saveType": "none",
    "rtc": false,
    "rtcEpoch": "0",
    "sampleRate": 48000
  },
  "limits": {
    "maxCycles": "280896",
    "maxRuns": 100,
    "chunkCycles": "16384",
    "maxAudioBytes": 1048576
  },
  "inputs": [
    { "at": "100", "keys": 1 },
    { "at": "200", "keys": 0 }
  ],
  "checkpoints": [
    {
      "name": "loop",
      "at": "280896",
      "expect": { "cpu": { "registers": { "0": 42 } } },
      "memory": [
        {
          "name": "instructions",
          "address": 134217728,
          "length": 8,
          "sha256": "88579b2e7e76d3eaa9eb547c49c58255d0af9ff6efb31b6f0edefe52882ec2d0"
        }
      ]
    }
  ]
}
```

Unknown fields are rejected, so a misspelled expectation can't silently turn into a passing test.
Cycles are absolute 64-bit values, written as decimal strings.
Inputs (ROM, BIOS, saves) are identified by SHA-1, the way No-Intro catalogs identify ROM dumps, and outputs (frames, audio, saves, states, memory) by SHA-256.
Only version 2 is accepted.

The fields:

- **`assets`**: the `rom` (required), and optionally a `bios`, a `save` (the starting battery save) and a `state` (a save state loaded after boot).
  Each has the `sha1` of its bytes and, for the command-line tools, a `path` relative to the manifest.
  The ROM must be a raw image, not a ZIP.
- **`config`**: `boot` (`hle`, or `bios` when a BIOS asset is present), `saveType` (one of `SAVE_TYPES`, including `auto`), `rtc` (on or off), `rtcEpoch` (Unix seconds, up to 253402300799) and the audio `sampleRate` (8,000 to 192,000).
  All of them must be given, so a scenario never depends on a default.
- **`inputs`**: up to 1,024 `{at, keys}` entries in cycle order, where `keys` is a button mask built from `KEYS`.
  Entries at the same cycle keep their order.
  No input may come after the last checkpoint.
- **`checkpoints`**: 1 to 10,000 entries in order, each with a unique `name`, a cycle `at`, an optional `expect`, and up to 64 named `memory` regions of at most 1 MiB.
  Checkpoints may share a cycle.
- **`expect`**: any of an exact `cycle` and `reason`, some CPU values (`registers` by number, `cpsr`, `nextInstructionAddress`), and the hashes `frameSha256`, `audioSha256`, `saveSha256` and `stateSha256`.
  A memory region carries its own optional `sha256`.
- **`limits`**: `maxCycles` (every input and checkpoint must fall within it), `maxRuns` (1 to 1,000,000 slices), `chunkCycles` (1 to 70,224 cycles per slice) and the total `maxAudioBytes` (up to 256 MiB).
  They bound emulated work, not wall-clock time, so a scenario fails the same way on a fast and a slow machine.

## What a result contains

Every checkpoint records the actual stopping cycle and reason, any fault, the CPU registers, the SHA-256 of the frame, the audio and the battery save, the hashes of the requested memory regions, and a hash of the whole machine state when it hasn't faulted.

- Frames are 240x160 pixels of 15-bit BGR color, 16-bit little-endian.
- Audio is interleaved signed 16-bit little-endian stereo at the configured rate.
  Its hash covers the samples produced since the previous checkpoint, or since the start.
- The runner advances in slices of `chunkCycles` and collects audio after each.
  An instruction can run past a checkpoint's cycle, so the checkpoint records the cycle at which it was actually observed.
- A checkpoint with no expectations still fails on an unexpected fault or STOP.
  To expect one, set `expect.reason` on the last checkpoint, with no inputs left.

At the first failing checkpoint, the runner stops and writes `frame.bgr555le`, `audio.s16le`, `battery.sav` and a `memory-N.bin` per requested region.
A failed `result.json` also contains a `capture` failure bundle, unless capture was turned off.

Check where an expected hash came from before trusting it.
A hash recorded from the current code only guards against regressions.
A passing scenario, or a successful replay, doesn't show that Evenfall matches real hardware.

## Replaying a captured failure

```sh
node web/gba/scripts/replay-capture.mjs /tmp/evenfall-result/result.json /tmp/evenfall-example/scenario.json
```

```text
node web/gba/scripts/replay-capture.mjs RESULT_OR_BUNDLE_JSON MANIFEST_JSON [WASM_PATH]
```

The first argument is a failed `result.json` or a bundle saved on its own.
Pass the original manifest with its assets, and optionally the original WebAssembly file (default `web/gba/evenfall-gba.wasm`).
The tool checks the asset and build identities, restores the newest snapshot, replays the recorded operations, and compares the CPU state, frame, save and machine state with the captured ones.
A successful reproduction prints `status: "reproduced"` and exits 0.
A mismatched identity, a replay that diverges, or a missing snapshot exits 2.

Replay reproduces the failure on the build that captured it.
To test whether a newer build fixed the bug, run the cold-boot scenario again instead of bypassing the build check.

## Using the tools from code

The modules run in Node or in a dedicated browser Worker, on a `WasmCore` that nothing else drives at the same time.
In a checkout, relative imports from `web/gba/` work as well as the package names.

```js
import { WasmCore } from "@evenfall/gba/core";
import { runScenario } from "@evenfall/gba/devtools/scenario";

const core = await WasmCore.create(wasmBytes);
try {
  const result = await runScenario(
    core,
    manifest,
    { rom: romBytes },
    {
      build: { wasmSha256 },
      onArtifact(name, ownedBytes) {
        /* keep it */
      },
      capture: { maxSnapshots: 3, maxHistoryBytes: 4 * 1024 * 1024 },
    },
  );
} finally {
  core.dispose();
}
```

`runScenario(core, manifest, assets, options)` loads the assets and owns the core for the call.
Pass every declared asset as bytes, under its asset name, and nothing else.
Invalid manifests and wrong assets reject before anything loads, while failed checkpoints resolve with `status: "failed"`.

| Option         | Meaning                                                                                        |
| -------------- | ---------------------------------------------------------------------------------------------- |
| `build`        | the build identity recorded in the result and in failure bundles                               |
| `capture`      | recorder limits ([Capture budgets](#capture-budgets)), or `false` to skip capture and its cost |
| `signal`       | an `AbortSignal`, and aborting resolves with `status: "cancelled"`                             |
| `yieldControl` | an async callback awaited every few slices, so a long scenario lets its host breathe           |
| `onProgress`   | called with each checkpoint's observations                                                     |
| `onArtifact`   | receives the bytes of each artifact, which the callee owns                                     |
| `allArtifacts` | also emit `checkpoint-N.bgr555le` and `checkpoint-N.s16le` for every checkpoint                |

For an interactive diagnostic script, wrap an already loaded core in a `FailureRecorder`:

```js
import { FailureRecorder } from "@evenfall/gba/devtools/capture";

const recording = new FailureRecorder(core, {
  identity: { build: { wasmSha256 }, config: bootConfiguration },
  onCapture(bundle) {
    /* keep it */
  },
});
recording.runUntil(1000n, [{ at: 100n, keys: 1 }]);
recording.drainAudio();
const bundle = recording.capture("manual", { note: "First incorrect scene" });
// recording.assert(condition, "Unexpected register", { expected, actual });
```

Send `runUntil`, `step`, `setKeys` and `drainAudio` through the recorder.
It passes read-only calls (inspection, frames, memory, saves) straight through.
A call that changes state without going through the recorder can't be replayed, so create a new recorder after loading a ROM or state directly.
Faults and thrown errors are captured automatically, and `assert` captures before it throws.
The original error is thrown again even if `onCapture` fails.

`replayFailureBundle(core, bundle, { identity })` replays into a separately loaded core, and `replayFailureBundleAsync` does the same with cancellation and periodic yields.
Every identity field recorded in the bundle must match the one supplied.
These functions can't hash the WebAssembly file themselves, so build the recorder with the full build and boot identity you will supply at replay.
`validateFailureBundle` checks the fields a viewer reads before it shows an untrusted bundle.

## Recording in `Emulator` sessions

An `Emulator` session sends everything it runs through a gameplay recording ([`journal.js`](../web/gba/devtools/journal.js)), which combines a `FailureRecorder` with a log of inputs:

- Loading a ROM, resetting and loading a state each start a fresh recording.
  Rewinding or loading a rewind bundle starts a new segment from the restored state.
- The log keeps the timed inputs of the current segment.
  `exportScenario(name?)` turns it into a manifest and its assets, starting from a cold boot or from the segment's first state, and ending with an `End` checkpoint.
  The log stops growing at 1,024 inputs, so only that first stretch can be exported.
- A guest fault captures a failure bundle and sends it as a `capture` event.
  `capture(note?)` pauses and captures on demand, and `checkpoint(name)` adds a named checkpoint to the export.
- `recording(false)` turns recording off.
  Anything run while it is off leaves a gap: older snapshots are dropped, a capture says replay is unavailable, and `exportScenario()` fails until `recording(true)` or `newSegment()` starts a new segment from the current state.
- During play, a snapshot is taken about every 60 frames times the playback speed, so the history covers about the same stretch of real time at any speed.

## Capture budgets

| Limit                                        | Default   |
| -------------------------------------------- | --------- |
| Snapshots kept (`maxSnapshots`)              | 3         |
| Raw snapshot bytes (`maxSnapshotBytes`)      | 16 MiB    |
| Operations kept (`maxOperations`)            | 4,096     |
| Serialized history bytes (`maxHistoryBytes`) | 4 MiB     |
| Snapshot interval (`snapshotIntervalCycles`) | 60 frames |

Before the history overflows, the recorder takes a fresh snapshot and starts the history again from it, so a later fault still has a starting point.
These extra snapshots are counted in `history.budgetSnapshots`, and very small budgets make them frequent.
A single operation or snapshot bigger than its budget makes replay unavailable.

A capture also holds copies of the history, the encoded snapshots, the frame, the save, and one final state when the machine hasn't faulted.
Exporting as Base64 and JSON takes more memory on top of the budgets.
Before relying on a bundle, check `history.replayable`, the dropped counts and `snapshotError`.

## Rules for reliable reproductions

- **Use emulated cycles.** Record the cycle each run actually stopped at.
  An instruction can't be split, so it can run past a requested deadline.
  The core reports how many inputs a run consumed, not the instruction at which each took effect, so a recorded input lies somewhere within its run and shouldn't be presented as an exact time.
- **Keep the cold-boot version.** Keep the manifest, the starting battery save and the inputs next to any snapshot shortcut.
  Snapshots are tied to the state format, the ROM and the BIOS, so a core change can make them unusable, and a cold boot also runs the start-up code a snapshot skips.
- **Snapshots come before the failure.** A faulted machine can't be saved, so capture keeps an earlier snapshot.
  The history either holds every operation since that snapshot or says replay is unavailable.
- **Results are copies.** Reports hold copied observations, never views into WebAssembly memory that could change.
- **Host audio restarts.** Snapshots don't include the host resampler, and loading one resets it.
  Comparing resampled audio exactly needs cold-boot runs with the same output rate and collection schedule.
  Frame, CPU, memory, save and state comparisons are unaffected.

## Privacy

ROM and BIOS files are passed separately and never stored in a bundle.
A bundle's snapshots and memory regions can still contain game data and save progress, so leaving out the ROM doesn't make a bundle anonymous or safe to publish.
The tools write local files and upload nothing.

## How it is checked

Build the WebAssembly binary with `npm run build:wasm -w @evenfall/gba`, then `npm test` (Node 24 or later) runs the scenario runner, failure capture and command-line replay against the real core.
Without the binary, the WebAssembly tests are skipped.
[`testing.md`](testing.md) lists the other test suites.

## Known limits

- Scenarios take raw ROM images only, not ZIP archives.
- A scenario holds at most 1,024 inputs, and a session recording stops collecting inputs at the same count.
- Replay needs a kept snapshot.
  A bundle whose history has a gap is evidence, not a reproduction.
- The tools compare with recorded results, not with hardware.
