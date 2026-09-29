# Deterministic developer tools

Evenfall's developer tools turn a bug report into a reproducible artifact. They
have three parts:

- **Scenarios**: a JSON manifest of assets, timed inputs and checkpoints with
  expected hashes, run deterministically from a cold boot.
- **Failure capture**: a bounded recorder that keeps recent snapshots and
  operations around a running core and packages them as a failure bundle when
  something goes wrong.
- **Replay**: restoring a bundle's newest snapshot and replaying its
  operations, with identity and result checks at every step.

The tools are JavaScript modules over the ordinary synchronous `WasmCore`
interface of the [`@evenfall/gba`](sdk.md) package. They use the same Rust
WASM binary as the SDK, need no special instrumentation, and do not change the
hardware model, so a reproduction runs on the binary that showed the problem.
The command-line tools own filesystem access and reports; the modules own
orchestration and bounded capture; `Machine` owns emulation. For
instruction-level traces, use the native runner's `--trace` option
([`cli.md`](cli.md)).

Every `Emulator` session also records gameplay by default (see
[Recording in `Emulator` sessions](#recording-in-emulator-sessions)). A
session exports its recording as a scenario and captures failures, and a
separate `Emulator` runs scenarios and replays captures in its own worker
without touching another session. Exported files run with the command-line
tools below.

## Quick start

From the repository root, generate and run a scenario for a project-owned
eight-byte ARM program. No game or BIOS is needed:

```sh
npm ci
npm run build:wasm -w @evenfall/gba
node web/gba/scripts/run-scenario.mjs --example /tmp/evenfall-example
node web/gba/scripts/run-scenario.mjs /tmp/evenfall-example/scenario.json /tmp/evenfall-result
```

`--example [directory]` writes `loop.gba` and `scenario.json` into the
directory (default `scenario-example`). The second command runs the manifest
and writes `result.json` into the output directory, which must not exist yet
so earlier evidence is never overwritten.

```text
node web/gba/scripts/run-scenario.mjs MANIFEST [OUTPUT_DIR] [WASM_PATH]
node web/gba/scripts/run-scenario.mjs --example [DIRECTORY]
```

`OUTPUT_DIR` defaults to `scenario-result-<timestamp>` and `WASM_PATH` to
`web/gba/evenfall-gba.wasm`; the WASM file's SHA-256 is recorded as the build
identity. The runner prints a one-line JSON summary and exits with:

| Status | Meaning                                                                |
| ------ | ---------------------------------------------------------------------- |
| 0      | Every checkpoint passed.                                               |
| 1      | The scenario failed or was cancelled.                                  |
| 2      | Invalid manifest, missing or mismatched asset, or another setup error. |

These codes suit CI and `git bisect run`; a bisect wrapper should map status 2
to bisect's skip code, 125.

## Scenario manifests

The generated example is a complete version-2 manifest:

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

Unknown fields are rejected, so a misspelled expectation cannot silently
become a passing test. Cycles are absolute unsigned 64-bit values written as
decimal strings. Input assets are identified by SHA-1; observed outputs
(frames, audio, saves, states and memory) by SHA-256.

Version-1 manifests identified assets by SHA-256. The SDK validates version-2
manifests; ZiGBA's `upgrade-scenario.mjs` CLI is not part of this workspace.

| Field         | Contract                                                                                                                                                                                                                                                                                                                    |
| ------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `assets`      | `rom` (required), and optionally `bios`, `save` (initial battery save) and `state` (a save state loaded after boot). Each has the `sha1` of its raw bytes, as No-Intro DATs identify dumps, and, for the command-line tools, a `path` relative to the manifest. The ROM must be a raw image; ZIP archives are not accepted. |
| `config`      | Explicit `boot` (`hle`, or `bios` when a BIOS asset is present), `saveType` (one of `SAVE_TYPES`, including `auto`), `rtc` (boolean), `rtcEpoch` (Unix seconds up to 253402300799) and output `sampleRate` (8,000–192,000).                                                                                                 |
| `inputs`      | Up to 1,024 ordered `{at, keys}` entries; `keys` is the button mask from `KEYS`. Same-cycle entries keep their order. No input may follow the final checkpoint.                                                                                                                                                             |
| `checkpoints` | 1–10,000 ordered entries, each with a unique `name`, a cycle `at`, an optional `expect` and up to 64 named `memory` regions of at most 1 MiB. Checkpoints may share a cycle.                                                                                                                                                |
| `expect`      | Any of: exact `cycle` and `reason`; sparse CPU values (`registers` by index, `cpsr`, `nextInstructionAddress`); SHA-256 values `frameSha256`, `audioSha256`, `saveSha256` and `stateSha256`. A memory region carries its own optional `sha256`.                                                                             |
| `limits`      | Absolute `maxCycles` (every input and checkpoint must fall within it), `maxRuns` (1–1,000,000 execution slices), `chunkCycles` (1–70,224 cycles per slice) and cumulative `maxAudioBytes` (up to 256 MiB). These bound emulated work, not wall time.                                                                        |

## Scenario results

Every checkpoint records the actual stopping cycle and reason, any fault, the
CPU registers, the SHA-256 of the frame, audio and battery save, the hashes of
requested memory regions, and a machine-state hash when the machine has not
faulted.

- Frames are 240×160 native BGR555, 16-bit little-endian.
- Audio is interleaved signed 16-bit little-endian stereo at the configured
  rate. Its hash covers the samples drained since the previous checkpoint, or
  since the start.
- The runner advances in slices of `chunkCycles` and drains audio after each.
  An instruction that crosses a checkpoint can consume inputs up to the final
  checkpoint; the checkpoint's actual cycle says when its observation was
  made.
- A checkpoint without expectations still fails on an unexpected fault or
  STOP. To expect one, set `expect.reason` on the final checkpoint, with no
  inputs left unconsumed.

On the first failing checkpoint, the runner stops and writes `frame.bgr555le`,
`audio.s16le`, `battery.sav` and `memory-N.bin` for each requested region. A
failed `result.json` also contains a `capture` failure bundle, unless capture
was disabled.

Review the source of an expected hash before accepting it. A hash recorded from
the current implementation is a regression baseline, not an independent
oracle, and neither a passing scenario nor a successful replay establishes
agreement with real hardware.

## Replaying a captured failure

```sh
node web/gba/scripts/replay-capture.mjs /tmp/evenfall-result/result.json /tmp/evenfall-example/scenario.json
```

```text
node web/gba/scripts/replay-capture.mjs RESULT_OR_BUNDLE_JSON MANIFEST_JSON [WASM_PATH]
```

The first argument is a failed `result.json` or an extracted bundle. Supply
the original manifest and its assets, and optionally the original WASM (default
`web/gba/evenfall-gba.wasm`). The tool verifies asset and build identities,
restores the newest retained snapshot, replays the recorded operations, and
compares the captured CPU state, frame, save and machine state. A successful
reproduction prints `status: "reproduced"` and exits 0; an identity mismatch,
replay divergence or missing snapshot exits 2.

Replay reproduces the captured failure state, not a passing assertion. To test
a new emulator revision, run the cold-boot scenario again instead of bypassing
the build-identity check.

## Programmatic use

The modules run in Node or in a dedicated browser Worker, on a `WasmCore` that
no other code drives at the same time. In a checkout, relative imports from
`web/gba/` work as well as the package names.

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
        /* retain locally */
      },
      capture: { maxSnapshots: 3, maxHistoryBytes: 4 * 1024 * 1024 },
    },
  );
} finally {
  core.dispose();
}
```

`runScenario(core, manifest, assets, options)` loads the assets into the core
and owns it for the call. Supply every declared asset as bytes under its asset
name, and no undeclared ones. Validation and asset-identity errors reject
before anything is loaded; execution and checkpoint failures resolve with
`status: "failed"`. Options:

| Option         | Meaning                                                                                                 |
| -------------- | ------------------------------------------------------------------------------------------------------- |
| `build`        | Build identity recorded in the result and in failure bundles.                                           |
| `capture`      | Recorder limits (see [Capture budgets](#capture-budgets)), or `false` to skip capture and its overhead. |
| `signal`       | An `AbortSignal`; aborting resolves with `status: "cancelled"`.                                         |
| `yieldControl` | An async callback awaited every few slices, so a long scenario can yield to its host.                   |
| `onProgress`   | Called with each checkpoint's observations.                                                             |
| `onArtifact`   | Receives owned artifact bytes.                                                                          |
| `allArtifacts` | Also emit `checkpoint-N.bgr555le` and `checkpoint-N.s16le` for every checkpoint.                        |

For interactive diagnostic scripts, wrap an already loaded core in a
`FailureRecorder`:

```js
import { FailureRecorder } from "@evenfall/gba/devtools/capture";

const recording = new FailureRecorder(core, {
  identity: { build: { wasmSha256 }, config: bootConfiguration },
  onCapture(bundle) {
    /* retain locally */
  },
});
recording.runUntil(1000n, [{ at: 100n, keys: 1 }]);
recording.drainAudio();
const bundle = recording.capture("manual", { note: "First incorrect scene" });
// recording.assert(condition, "Unexpected register", { expected, actual });
```

Route `runUntil`, `step`, `setKeys` and `drainAudio` through the recorder; it
forwards read-only inspection, frame, memory and save access. A state-changing
call that bypasses the recorder cannot be replayed, so create a new recorder
after loading a ROM or state directly. Faults and thrown execution errors
capture automatically, and `assert` captures and then throws. The original
execution error is rethrown even if `onCapture` fails.

`replayFailureBundle(core, bundle, { identity })` replays into a separately
loaded core, and `replayFailureBundleAsync` does the same with cancellation
and periodic yields. Every identity key recorded in the bundle must match the
supplied identity. Unlike the command-line tool, these functions cannot derive
a WASM hash, so construct the recorder with the complete build and boot
identity you will supply at replay. `validateFailureBundle` checks the fields a
viewer reads before it renders an untrusted bundle.

## Recording in `Emulator` sessions

An `Emulator` session routes its execution through a gameplay recording
([`web/gba/devtools/journal.js`](../web/gba/devtools/journal.js)), which combines a
`FailureRecorder` with an input journal:

- Each load, reset and state load starts a fresh recording. A rewind or
  rewind-bundle load starts a new segment from the restored state.
- The journal keeps the timed inputs of the current segment.
  `exportScenario(name?)` turns it into a manifest and its assets, starting
  from a cold boot or from the segment's initial state, and ending with an
  `End` checkpoint. The journal stops growing at 1,024 inputs, which freezes
  the exportable prefix.
- A guest fault captures a failure bundle and emits it as a `capture` event;
  `capture(note?)` pauses and captures on demand, and `checkpoint(name)` adds a
  named checkpoint to the export.
- `recording(false)` turns recording off. Execution while it is off leaves a
  gap: older snapshots are dropped, capture reports that replay is
  unavailable, and `exportScenario()` fails until `recording(true)` or
  `newSegment()` starts a new segment from the current state.
- During realtime play the periodic snapshot interval is about 60 frames times
  the playback speed, so the retained history covers roughly the same
  wall-clock span at every speed.

## Capture budgets

| Limit                                                 | Default   |
| ----------------------------------------------------- | --------- |
| Retained snapshots (`maxSnapshots`)                   | 3         |
| Raw snapshot bytes (`maxSnapshotBytes`)               | 16 MiB    |
| Retained operations (`maxOperations`)                 | 4,096     |
| Serialized history bytes (`maxHistoryBytes`)          | 4 MiB     |
| Periodic snapshot interval (`snapshotIntervalCycles`) | 60 frames |

Before ordinary history overflows, the recorder takes a fresh snapshot and
rebases its history on it, so a later fault still has an anchor. These extra
snapshots are counted in `history.budgetSnapshots`; tiny budgets increase
snapshot overhead. A single operation or snapshot larger than its budget makes
replay unavailable.

A capture also holds copies of the retained history, the encoded snapshots,
the frame, the save and, when the machine has not faulted, one final state.
Base64 and JSON export add further memory beyond the ring budgets. Before
relying on a bundle, check `history.replayable`, the dropped counts and
`snapshotError`.

## Reproduction rules

- Use emulated cycle timestamps and keep actual stopping cycles. Instructions
  are atomic and can cross a requested deadline. The core reports how many
  inputs a run consumed, not the instruction at which each applied: a recorded
  input lies somewhere within its run's cycle interval and must not be
  presented as an exact observed time.
- Keep the cold-boot manifest, initial battery save and timed inputs alongside
  any snapshot shortcut. Snapshots are tied to their state format and to the
  ROM and BIOS, so a core change can make them unusable, and a cold boot also
  exercises the initialization that a snapshot skips.
- Capture keeps a snapshot from before the failure, because a faulted machine
  cannot write one. Bounded history either keeps every operation since its
  replay anchor or reports that replay is unavailable.
- Reports contain copied observations, never views into mutable WASM memory.
- Snapshots do not include the host audio resampler, and loading one resets
  its phase. Exact resampled-audio comparisons need cold-boot runs with the
  same output rate and drain schedule; a snapshot continuation does not
  continue the original host audio stream exactly. Frame, CPU, memory, save
  and machine-state comparisons are unaffected.

## Privacy

ROM and BIOS files are supplied separately and never embedded in a bundle. A
bundle's snapshots and memory regions can still contain guest program data
and save progress, so leaving out the ROM and BIOS does not make a bundle
anonymous or safe to publish automatically. The tools write local files and
upload nothing.

## Testing

Build the Rust WASM binary with `npm run build:wasm -w @evenfall/gba`, then
`npm test` runs the scenario runner, failure capture and command-line replay
against the real core; it needs Node 24 or later. Without the binary, WASM
tests skip.
[`testing.md`](testing.md) lists the other suites.

## Limits

- Scenarios take raw ROM images only, not ZIP archives.
- A scenario holds at most 1,024 inputs, and an exported session recording
  stops collecting inputs at the same count.
- Replay needs a retained snapshot; a bundle whose history had a gap is
  evidence, not a reproduction.
- The tools compare against recorded results, not against hardware.
