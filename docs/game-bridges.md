# Game bridges: typed, read-only game data

A game bridge turns a coherent capture of emulated memory into typed data a
user interface can render: a party list, a map name, a score. The generic
contract ships in the SDK as `@evenfall/gba/bridge`, with React bindings in
`@evenfall/gba/bridge/react` (see [`sdk.md`](sdk.md#game-bridges)).
Applications supply their own game-specific definitions and user-supplied ROMs.

| Source                                             | Contents                                                                                                                                                                                                                                      |
| -------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [`family.ts`](../web/gba/src/bridge/family.ts)     | `defineFamily`, `createGame`, the `ram` and `ramArray` sources, views, manifests, `locateWith`, and the runtime build registry (`locatable`, `registerBuild`, `unregisterBuild`, `buildProvenance`)                                           |
| [`locate.ts`](../web/gba/src/bridge/locate.ts)     | Locators (`thumbLiteral`, `thumbImmediate`, `thumbAccess`, `content`, `references`, `follow`, `offset`, `inferred`), signatures, `detectFamily`, `locateGame`, `locateBuild`, `manifestFromLocated`, `createOffsetMap` and `relocateManifest` |
| [`contract.ts`](../web/gba/src/bridge/contract.ts) | `defineGame`, `defineBridge`, `definePreparedGame`, `resolveGame`, `decodeGame` and the result types                                                                                                                                          |
| [`memory.ts`](../web/gba/src/bridge/memory.ts)     | `createMemoryReader` and the layout constructors                                                                                                                                                                                              |
| [`host.ts`](../web/gba/src/bridge/host.ts)         | `BridgeHost`, the live capture and subscription host                                                                                                                                                                                          |
| [`react.ts`](../web/gba/src/bridge/react.ts)       | `BridgeProvider`, `useViews`, `useBridge`, `useBridgeResources`                                                                                                                                                                               |
| [`context.ts`](../web/gba/src/bridge/context.ts)   | `GameArenaStatus` and `GameStatusField` display types                                                                                                                                                                                         |

## Scope

- **Trusted code.** Bridges are TypeScript or TSX modules bundled with an
  application. There are no downloadable plugins, sandbox, runtime JSX
  compiler, scripting console or registry service. Bridge code is trusted,
  not sandboxed.
- **Exact builds.** Bridges target decompiled games and source-available
  forks, whose symbols and layouts can be verified. Support is per exact built
  ROM, identified by SHA-1 as in No-Intro DATs, not per title or header game
  code. A reviewed build has a committed manifest; a family with locators can
  also recognize other builds at runtime, which are marked as not reviewed.
- **Read-only.** The contract has no writes, patch placeholders, commands or
  mutation permissions.
- **Separation.** A game definition owns its build profiles and a typed
  collection of bridges. The host owns selection, capture, scheduling and
  subscriptions; bridges decode a shared observation; React components render
  typed data. Game formats never enter the Rust emulator core.
- **On the page.** Decoding runs synchronously on the page, on copied
  observations, outside the emulator worker. This needs no extra worker and no
  serialization of functions or layouts.

## Quick start

```tsx
import { BridgeProvider, useBridge } from "@evenfall/gba/bridge/react";
import { BridgeHost, defineGame, ref, u8 } from "@evenfall/gba/bridge";

const game = defineGame({
  id: "example/game",
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

function Lives() {
  const state = useBridge(game, "lives");
  return <p>{state.status === "ready" ? state.data : state.status}</p>;
}

// <BridgeProvider host={new BridgeHost(emulator)}><Lives /></BridgeProvider>
```

`romHash` is the lowercase hexadecimal SHA-1 of the raw ROM, and `source`
describes how that ROM was built (see [Game definitions](#game-definitions)).
Define game objects once at module scope, not inside component renders: the
host caches its stores by object identity.

## Game families

Most games are written as a family: an object that says what the game reads,
what it prepares from the ROM, and what it gives its overlay.

```ts
interface MyViews {
  party: PartyView;
  enemy?: PartyView; // optional: a family may leave it out
}

export const myFamily = defineFamily<MyViews, MyRom>()({
  id: "example/family",
  sources: {
    partyCount: ram(u8, { locate: [partyCountLocator] }),
    party: ramArray(monRecord, 6),
  },
  resources: ({ rom, romHash, config }: RomContext<MyRom>) => ({
    names: createNameTable(rom, config.speciesNames),
  }),
  views: {
    party: ({ memory, symbols, resources }) => ({ status: "ready", data: … }),
  },
});
```

| Part        | Meaning                                                                                                                                                  |
| ----------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Views`     | The overlay's input: one entry per view. It belongs to the family, and nothing about it is shared between games.                                         |
| `sources`   | What the family reads: a `ram(layout)` value or a `ramArray(layout, length)`, with optional locators. A source has no address; the manifest supplies it. |
| `resources` | Built once per ROM from the manifest's `rom` data, such as catalogs, artwork decoders and search indexes.                                                |
| `views`     | One pure function per view. It reads `symbols` (each source at the build's address), `memory`, `sample` and `resources`, and returns a decode result.    |

`defineFamily<Views, Rom>()` names the overlay's input and the manifest's
ROM data; everything else is inferred from the object. A view that returns the
wrong shape, a missing required view, and a codec whose record does not fit
the views that read it are type errors. Views may return functions, such as a
search bound to the ROM's trainer index and the live difficulty; the SDK passes
functions through by reference.

A game specializes a family by spreading it and replacing what differs:

```ts
export const myHack = defineFamily<MyViews, MyRom>()({
  ...myFamily,
  id: "example/hack",
  sources: {
    ...myFamily.sources,
    party: { ...myFamily.sources.party, layout: hackRecord },
  },
});
```

A replaced source may decode to a different record type, as long as every
view that reads it still type-checks.

### Manifests

A manifest describes one exact build as plain JSON: `family`, `romHash`,
`source`, an address for every source, the `rom` data the resources read, and
`provenance` (`reviewed`, `relocated` from a `parent` build, or `located`).
`ManifestOf<typeof family>` gives its type. `createGame(family, manifests)`
checks each manifest (family id, SHA-1 format, one address per source and no
unknown ones) and returns an ordinary game definition, so the host, `useBridge`
and `decodeGame` work unchanged; each view becomes a bridge with the same key.
`checkManifest` runs the same checks on a manifest parsed from JSON.

### Overlays

`useViews(game)` returns every view state of a game at once, for an overlay
component. An optional view that a family leaves out is absent, and its type
includes `undefined`, so an overlay checks before reading it.
`useBridge(game, key)` still subscribes to a single view.

### Locators, detection and probes

Locators find a value in a ROM without a manifest: an address, or a constant
such as a table's count. A RAM source lists them in its `locate` field
(`locateWith(sources, locators)` attaches a generated module), and a family's
`romLocators` lists them for the numbers in its ROM data, keyed by path
(`"catalog.species"`, `"catalog.speciesCount"`); those paths are checked
against the ROM data type, and `createGame` rejects a path a manifest's ROM
data does not have.

Locators compose. Each returns candidate values, and each one's `id` is the
TypeScript that builds it, so a generated module reads as what it does:

| Locator                       | Returns                                                                                                                                                                                                                                                      |
| ----------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `thumbLiteral(pattern, ldr)`  | The word a Thumb `ldr rN, [pc, #imm]` in the masked code loads from its literal pool: a RAM global, a ROM table or a large constant.                                                                                                                         |
| `thumbImmediate(pattern, at)` | The 8-bit immediate of a Thumb `mov`, `cmp`, `add` or `sub` in the masked code, shifted when a `lsls` of the same register follows (how compilers build larger constants): a count or other constant.                                                        |
| `thumbAccess(pattern, at)`    | The address a Thumb load or store in the masked code accesses, worked out from the instructions before it (literal loads, immediates, adds, shifts); an unknown index drops out, leaving an array's start. It finds fields and arrays that no literal names. |
| `content(pattern)`            | Where data matches: exact bytes, `??` wildcards and `ptr` for a word that is a ROM pointer.                                                                                                                                                                  |
| `references(l)`               | Where a pointer to one of `l`'s results is stored.                                                                                                                                                                                                           |
| `follow(l)`                   | The pointer stored at each of `l`'s results.                                                                                                                                                                                                                 |
| `offset(l, delta)`            | Each of `l`'s results plus `delta`.                                                                                                                                                                                                                          |
| `inferred(l)`                 | `l`, marked as resting on an assumed layout.                                                                                                                                                                                                                 |

A game package can add its own base locators for domain-specific byte patterns
and compose them with `offset`, `references` and the built-in locators.

Signatures generate locators from a verified build. `codeSignatures` masks
literal offsets and branch targets, so a signature usually survives code
moving around it. When no code loads an address itself, `accessSignatures`
signs code that computes it (`thumbAccess`); only when neither exists does
`codeSignatures` fall back to code that loads a base just below the address,
`inferred(offset(thumbLiteral(…), 1))`, which assumes the layout next to that
base. `dataSignatures` signs a table's first
bytes with its pointers written as `ptr`, grown until unique.
`pointerSignatures` reaches a table made only of pointers through the data
an entry points to: `offset(references(content(…)), -4 * entry)`.
`valueSignatures` signs a constant through code that uses it, as a literal or
an immediate (including `movs` then `lsls`); code that uses the last ID of a
table (a bounds check) gives `inferred(offset(…, 1))`, ranked after exact
signatures. Immediates are wildcards in these patterns, so a
changed count is read rather than missed.

A value resolves when every locator that finds something agrees on one value.
Code or data that changed loses its signature, which lowers the value's
support rather than giving a wrong value. A value resolved only by inferred
locators is reported for review.

`detectFamily(family, rom)` checks cheaply whether a ROM looks like one of a
family's builds, before locating it fully: the cartridge header's game code
against the family's `detect.gameCodes`, then a few of its locators (its
`detect.markers`, or one locator from each of its first six sources) must
find something. The score is the share of markers that do.

A family may also list `probes`: named checks of a build's ROM data that
throw when its resources do not decode as expected, such as well-known
catalog entries or a trainer search.

### Recognizing builds at runtime

Only reviewed manifests are committed. A family whose locators were generated
from a reviewed build names it in `locatedFrom` (the generated module exports
the value), and `createGame` uses that build's manifest as the template for
builds it has never seen; it rejects a `locatedFrom` that is not one of its
reviewed manifests.

`locateGame(game, rom, options)` tries to recognize a ROM as a build of
`game`:

1. `detectFamily` must score at least 0.75, unless `known` says the caller
   already knows the game (a catalog entry). `onDetected` is called next, so a
   host can report recognition only when that is likely.
2. Every locator runs, with a pause between them (`yieldControl`, a macrotask
   by default) so the page stays responsive; `signal` aborts.
3. The build is rejected if any value's locators disagree, if an address is
   not located, if the manifest checks fail or if a probe throws. Values
   locators do not cover, such as strides and labels, come from the template.

The outcome is `located`, with the manifest (provenance `located`, the
template as `parent`), the locator `version` and a `review` list of values
found only by inferred locators or kept from the template; `not-this-game`,
with the detection; or `rejected`, with the reason.
`registerBuild(game, manifest)` adds an accepted build, after the same checks
as `createGame`, so `resolveGame`, the host and the hooks treat it like a
reviewed one. `unregisterBuild` removes it again and never removes a reviewed
build; `buildProvenance(game, romHash)` says whether a build is `reviewed`,
`located` or unknown. `locatable(game)` returns the family, the template and a
`version` that changes whenever a locator or the template changes, so a cache
of located manifests keyed by it never serves results from older locators.

## Game definitions

A game has an `id`, a nonempty `profiles` list and a `bridges` dictionary.

Each profile is one exact build:

| Field     | Meaning                                                                                                           |
| --------- | ----------------------------------------------------------------------------------------------------------------- |
| `romHash` | SHA-1 of the raw ROM, 40 lowercase hex digits.                                                                    |
| `source`  | `repository`, `revision`, `variant`, `compiler` and `options`: where the build came from and how it was compiled. |
| `symbols` | Typed memory references for that build, keyed by name.                                                            |

The profile matching the loaded ROM supplies every bridge's `symbols`. All
profiles of one game must expose the same symbol names with compatible types.
A fork with different layouts supplies different references and codecs under
the same names; a fork with different bridge capabilities gets its own game
definition. Keep profile exports precisely typed rather than widening them to
`SymbolTable`, so the compiler can check each symbol.

A bridge has an `id` and a synchronous `decode` function, independent of any
ROM identity. Dictionary keys must equal bridge ids. Ids are scoped to their
game, so two games can reuse one bridge definition. `defineGame` rejects
invalid or duplicate ROM hashes, an empty profile or bridge list, and a key
that differs from its bridge's id. These checks cannot prove that a claimed
hash or layout is truthful.

Types flow from the definition to the hook without casts, explicit output
generics, global id-to-data maps or duplicated view-model declarations:

- Inline decoders in `defineGame` infer their symbols from its profiles.
- A reusable `defineBridge` decoder annotates
  `BridgeContext<RequiredSymbols>`, and registering it in a game that lacks
  those symbols is a type error.
- `useBridge(game, key)` checks `key` against that game's bridges and infers
  only that decoder's data. Decoders keep their whole return type, including
  unions of different ready-state shapes.

### Prepared ROM resources

`definePreparedGame(prepare)` fixes a resource type before the game's decoders
are inferred. The host calls `prepare({ rom, romHash })` once per loaded ROM,
after exporting the ROM from the session and checking its SHA-1, to build
shared resources such as catalogs or artwork decoders. `rom` is a
`MemoryReader` over the cartridge image at `0x08000000`. Decoders receive the
result as `context.resources`, and `useBridgeResources(game, key)` returns it
to components. Resources are cacheable ROM metadata, never live RAM or a
pointer cache, and bridge results should carry small serializable references
to them rather than pixel buffers. ROM tables and images remain user-supplied:
no ROM-derived names, graphics, patches, saves or game source are checked into
the project.

## Decode results and states

`decode(context)` receives:

| Field       | Meaning                                                       |
| ----------- | ------------------------------------------------------------- |
| `memory`    | A `MemoryReader` over one captured sample.                    |
| `symbols`   | The matched profile's symbols.                                |
| `sample`    | `romHash`, `timeline`, `sequence` and `cycle` of the capture. |
| `resources` | Prepared ROM resources, or `undefined`.                       |

It returns `{ status: "ready", data }` or
`{ status: "unavailable", reason }`. Use `unavailable` for normal boot,
transition and invalid-record states. A valid empty collection is `ready` with
an empty array.

Hook and store states distinguish:

| Status        | Meaning                                                                              |
| ------------- | ------------------------------------------------------------------------------------ |
| `idle`        | No game loaded, or no subscriber for this key.                                       |
| `unsupported` | No profile matches the loaded ROM.                                                   |
| `loading`     | Waiting for the first capture.                                                       |
| `unavailable` | The decoder reported no valid data for this sample, with a `reason`.                 |
| `ready`       | Decoded `data` for this sample.                                                      |
| `error`       | The decoder threw, or capture failed; carries `message` and, when known, the sample. |

Only `ready` has `data`; `ready` and `unavailable` also carry their
`sample`. Data is deeply read-only in TypeScript.
Publish fresh plain records, arrays, primitives and functions, not classes,
`Map`s or typed arrays. `decodeGame` enforces this at runtime: it rejects
non-plain objects, then copies and recursively freezes records and arrays
without freezing the decoder's own scratch storage. Functions pass through by
reference, so a view should return the same function while its inputs are
unchanged. These checks do not prove that a decoder is pure or that the game's
bytes are valid.

Game-authored header data can use `GameArenaStatus`:
`{ title, fields: readonly GameStatusField[] }`. Each field has a stable `id`,
a display `label`, and either a string `value` or `value: null` with a
required `reason`. No fixed vocabulary (weather, terrain, buffs) is imposed,
and unknown values explain their absence instead of claiming a neutral state.

## Reading memory

`MemoryRef<T>` pairs a guest address with a `MemoryLayout<T>`: a byte length
and a synchronous decoder. `memory.read(ref)` returns `T`. The reader also
offers little-endian `u8`, `u16` and `u32` reads, which need not be aligned,
and `bytes(address, length)`, which returns an owned copy.

| Constructor                       | Result                                                           |
| --------------------------------- | ---------------------------------------------------------------- |
| `ref(address, layout)`            | A reference to one value.                                        |
| `u8`                              | A one-byte unsigned layout.                                      |
| `array(address, element, length)` | A bounded array reference, at most 1 MiB.                        |
| `at(arrayRef, index)`             | A checked reference to one element, without decoding the others. |
| `pointer(layout)`                 | A 4-byte pointer layout that decodes to `MemoryRef<T> \| null`.  |

Custom layouts express anything else: a record type, a field at an explicit
offset, a bitfield. `createMemoryReader(regions)` copies its input regions and
rejects overlapping, uncaptured or out-of-range reads. Each read or array must
fit in one captured region and is limited to 1 MiB. The reader does not
capture anything itself or restrict addresses to RAM or ROM; the caller
supplies the regions.

### Following pointers

A pointer layout returns a reference, not the pointee. Follow it explicitly:

```ts
const saveRef = memory.read(symbols.gSaveBlock1Ptr);
if (saveRef === null) {
  return { status: "unavailable", reason: "Save is not initialized" };
}
const save = memory.read(saveRef);
```

The compiler requires the null check and knows the pointee's fields. A null
pointer decodes to `null`; a pointer that is not 4-byte aligned throws. Every
hop reads the same sample and never requests newer memory. Do not keep
resolved pointers between samples, because games move structures such as save
blocks; a direct global is simply read again. Following a pointer is data
navigation, never CPU control flow.

## Authoring workflow

There is no single base address from which every global follows. Offsets are
reliable inside a verified layout; independent globals need symbols or
documented metadata. Start from named roots:

1. Choose a supported decompilation revision and build configuration, or build
   a source-available fork with its symbols. Supply the ROM locally.
2. Match its raw ROM hash to a reviewed profile. For a new build, bind that
   artifact's symbols and target layout facts to its hash, and record the
   compiler, options and source revision in the profile. Do not apply a
   modern rebuild's symbols to a retail ROM without verifying that they
   describe that exact artifact.
3. Begin at a known root, a data symbol such as a party array or a save-block
   pointer, and derive fields, elements and pointer targets with that build's
   typed layouts.
4. Reuse shared record and text readers where a game family has them, select
   the fields you need in `decode`, and render them with `useBridge`.
5. Test against synthetic records and locally captured samples, including
   boot, scene transitions, invalid pointers and state replacement.

## The host

`new BridgeHost(emulator)` serves one `Emulator` session.
`host.observe(game)` returns that game's store, created once per game object
and shared by every consumer. A store's `subscribe(listener, key?)` observes
one bridge, or the whole collection when the key is omitted; hooks always
subscribe to their key. Only observed keys are decoded, and unobserved keys
report `idle`.

Capture works as follows:

- **Coherent samples.** The host calls `captureMemory()`, which copies all of
  EWRAM (256 KiB) and IWRAM (32 KiB) in one synchronous worker turn, at one
  instruction boundary, with the ROM hash, actual cycle, timeline and sequence
  number. It does not advance execution or read MMIO, save devices or ROM.
  Separate awaited `readMemory()` calls are not a coherent sample while the
  game plays.
- **Cadence.** `frame` and `clear` events request a capture. Requests coalesce
  into at most one capture in flight, started at least 50 ms after the
  previous one, so at most 20 samples a second. Nothing polls while paused;
  pause, step, load, reset and state load each produce events that refresh the
  data. With no subscribers, the host stops listening.
- **Selection.** Each sample is matched by ROM hash to exactly one observed
  game and profile. More than one match is an error; no match reports
  `unsupported`.
- **Identity.** A load, reset, state load or rewind replaces the session's
  timeline. The host then resets its stores to `loading` and discards any late
  reply from the old timeline or with an older sequence number. Speed changes
  keep the timeline, and a frame event's presentation `generation` is not this
  identity.
- **Failures.** A decoder that throws yields `error` for its own key without
  stopping playback or the other bridges. A failed capture reports `error` for
  all observed keys rather than keeping the previous data. The host never
  republishes an old sample as current.
- **ROM resources.** For a game with `prepare`, the host exports the ROM once
  per ROM hash, checks its SHA-1 and caches the reader while observed. Hooks
  never receive mutable ROM bytes.
- **Cost.** Observation adds nothing to the CPU's memory-access path; its cost
  is the capture copy and the decoders.

A capture is consistent at an instruction boundary, but it can still catch a
game halfway through updating or decrypting a record, so decoders must
validate what they read. Sampling does not promise to see every transient
event or to pair data with a specific displayed frame.

`resolveGame(games, romHash)` and `decodeGame(game, sample, memory,
resources?, keys?)` are the pieces the host uses, available for other hosts
and for tests. `resolveGame` returns the one game with a matching profile, or
`null`, and throws on duplicate game ids or multiple matches. `decodeGame`
dispatches every bridge (or the given keys) against one sample; callers must
supply a coherent sample themselves.

## Testing a bridge

```sh
npm ci
npm run check
npm run build:wasm -w @evenfall/gba
npm test
```

`npm run check` type-checks the SDK bridge, and `npm test` runs the SDK's Node
tests; tests requiring WASM skip if the binary is missing. Game-specific
decoders belong to applications, which should test against synthetic records
and captured samples. Synthetic codec tests show that a decoder reads its
format correctly; they are not a gameplay compatibility claim. See
[`testing.md`](testing.md) for the other test layers.

## Limits

- Relocation and signatures rely on Thumb code loading RAM addresses from
  literal pools. A source that no code names directly (a field reached as an
  offset from another global) is inferred from its neighbours or needs a hand
  review, and only a save state of the new build checks RAM sources live.
- Values no locator covers, such as strides, labels and counts that no code
  uses distinctively, come from the template manifest; a new build that
  changes them needs a review (the probes catch some changes). There is no
  generator that imports a build's ELF, map or debug information into typed
  layouts.
- Detection only says a ROM looks like a family's builds. A located build is
  accepted on its locators and probes, which check ROM data; a wrong RAM
  address that still decodes without errors is not caught.
- The built-in layouts are `u8`, arrays and nullable pointers. Bitfields,
  enums, signed values and bounded strings need custom layouts.
- Errors carry only a message, not the symbol, field path, address or
  expected length; the `error` state reserves `address` and `path` fields that
  `decodeGame` does not fill.
- Reads and arrays are bounded individually, but a decoder has no aggregate
  budget. A decoder can still loop or allocate without bound; none
  of the checks is a sandbox.
- Decoding cost is not measured. Decoding runs on the page; moving it to a
  dedicated worker would need a measurement that justifies it.
- There is no inspector for browsing roots and pointer targets.
- Bridges are read-only. Writing guest memory would need paused, atomic edits
  integrated with session recording and rewind, which the design does not
  provide. The session owns execution.
