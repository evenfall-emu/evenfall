# Game bridges

A game bridge reads a running game's memory and turns it into typed data a page can show: the player's party, the current map, a score.
It is how a companion view, a stream overlay or a tracker knows what is happening in the game without the game's help.

Reading another program's memory is fragile.
Addresses differ between versions of a game, a ROM hack moves things around, and a game can be halfway through updating a record when you look.
Bridges deal with this in three ways:

- **Exact builds.** A bridge is written for exact ROM builds, identified by their SHA-1, so its addresses and layouts are known to be right.
- **Consistent snapshots.** All decoding reads one copy of memory taken at a single instant, never live memory that changes as it reads.
- **Types all the way.** Memory layouts, decoders and React hooks share TypeScript types, so a decoder that returns the wrong shape doesn't compile.

The code ships in the SDK as `@evenfall/gba/bridge`, with React bindings in `@evenfall/gba/bridge/react` ([`sdk.md`](sdk.md#game-bridges)).
Applications write their own game definitions and supply their own ROMs.

## How it works

```text
 Emulator (Worker)         BridgeHost (page)              React
┌──────────────────┐      ┌────────────────────────┐     ┌──────────────────┐
│ captureMemory(): │─────▶│ match the ROM hash to  │     │ useBridge(game,  │
│ EWRAM + IWRAM at │ copy │ a build of the game    │     │   "party")       │
│ one instruction  │      │                        │     │                  │
│ boundary         │      │ decode each watched    │────▶│ re-render with   │
└──────────────────┘      │ view from the copy     │     │ typed data       │
  at most 20 times        └────────────────────────┘     └──────────────────┘
  a second, while
  something watches
```

Decoding runs synchronously on the page, on copied memory, outside the emulator's Worker.
That needs no extra Worker and no way to send functions between threads.
Game formats never enter the Rust core.

A few rules keep bridges simple and safe:

- **Trusted code.** Bridges are TypeScript modules bundled with the application.
  There are no downloadable plugins, sandbox, scripting console or registry.
- **Read-only.** A bridge can't write memory or send commands to the game.
- **Clear ownership.** A game definition owns its builds and its decoders.
  The host owns choosing the game, capturing, scheduling and subscriptions.
  Components only render.

| Source                                             | Contents                                                                                                  |
| -------------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| [`family.ts`](../web/gba/src/bridge/family.ts)     | families, manifests, the `ram` and `ramArray` sources, views, and the registry of builds found at runtime |
| [`locate.ts`](../web/gba/src/bridge/locate.ts)     | locators, signatures, detection, and finding a build's addresses                                          |
| [`contract.ts`](../web/gba/src/bridge/contract.ts) | `defineGame`, `defineBridge`, `definePreparedGame`, `resolveGame`, `decodeGame` and the result types      |
| [`memory.ts`](../web/gba/src/bridge/memory.ts)     | `createMemoryReader` and the memory layouts                                                               |
| [`host.ts`](../web/gba/src/bridge/host.ts)         | `BridgeHost`, which captures and publishes                                                                |
| [`react.ts`](../web/gba/src/bridge/react.ts)       | `BridgeProvider`, `useViews`, `useBridge`, `useBridgeResources`                                           |
| [`context.ts`](../web/gba/src/bridge/context.ts)   | the `GameArenaStatus` and `GameStatusField` display types                                                 |

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

`romHash` is the lowercase hexadecimal SHA-1 of the raw ROM, and `source` says how that ROM was built ([Game definitions](#game-definitions)).
Define games once at module level, not inside a component: the host caches its stores by object identity.

## Game families

Real games are usually written as a family.
A family says what the game reads, what it prepares from the ROM, and what it gives the overlay, without any addresses.
Each exact build then gets a small JSON manifest with its addresses.
This matters because one game often exists as several builds (regions, revisions, ROM hacks) that share their logic but not their addresses.

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

The parts of a family:

- **`Views`** is the overlay's input, one entry per view.
  It belongs to the family, and nothing about it is shared between games.
- **`sources`** is what the family reads: a `ram(layout)` value or a `ramArray(layout, length)`, with optional locators.
  A source has no address, and the manifest supplies it.
- **`resources`** are built once per ROM from the manifest's `rom` data, such as catalogs, artwork decoders and search indexes.
- **`views`** has one pure function per view.
  It reads `symbols` (each source at the build's address), `memory`, `sample` and `resources`, and returns a decode result.

`defineFamily<Views, Rom>()` names the overlay's input and the manifest's ROM data, and infers everything else.
A view that returns the wrong shape, a missing required view, and a record layout that doesn't fit the views reading it are all type errors.
Views may return functions, such as a search bound to the ROM's data, and the SDK passes them through as they are.

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

A replaced source may decode to a different record type, as long as every view that reads it still type-checks.

### Manifests

A manifest describes one exact build as plain JSON:

- `family` and `romHash`,
- `source`, how the ROM was built,
- an address for every source,
- the `rom` data the resources read,
- `provenance`: `reviewed`, `relocated` from a `parent` build, or `located` ([below](#recognizing-builds-at-runtime)).

`ManifestOf<typeof family>` gives its type.
`createGame(family, manifests)` checks each manifest (the family id, the SHA-1 format, exactly one address per source) and returns an ordinary game definition, so the host, `useBridge` and `decodeGame` work with it unchanged.
Each view becomes a bridge with the same key.
`checkManifest` runs the same checks on a manifest parsed from JSON.

### Overlays

`useViews(game)` returns every view of a game at once, for an overlay component.
An optional view the family leaves out is absent, and its type includes `undefined`, so the overlay has to check before reading it.
`useBridge(game, key)` subscribes to a single view.

## Finding addresses in a new build

A manifest per build works for builds someone has reviewed.
Locators let a family also recognize builds nobody has reviewed, such as a new ROM hack, by finding its addresses in the ROM itself.

### Locators

A locator finds a value in a ROM: an address, or a constant such as a table's length.
A RAM source lists locators in its `locate` field (`locateWith(sources, locators)` attaches a generated module), and a family's `romLocators` lists them for the numbers in its ROM data, keyed by path (`"catalog.species"`, `"catalog.speciesCount"`).
Those paths are checked against the ROM data's type, and `createGame` rejects a path a manifest doesn't have.

Locators compose.
Each returns candidate values, and each one's `id` is the TypeScript that builds it, so a generated module reads as what it does:

- **`thumbLiteral(pattern, ldr)`** returns the word a Thumb `ldr rN, [pc, #imm]` in the matched code loads from its literal pool: a RAM global, a ROM table or a large constant.
- **`thumbImmediate(pattern, at)`** returns the 8-bit immediate of a Thumb `mov`, `cmp`, `add` or `sub` in the matched code, shifted when a `lsls` of the same register follows, which is how compilers build larger constants.
- **`thumbAccess(pattern, at)`** returns the address a Thumb load or store in the matched code accesses, worked out from the instructions before it.
  An unknown index drops out, leaving the start of an array, so it finds fields and arrays no literal names.
- **`content(pattern)`** returns where data matches: exact bytes, `??` wildcards, and `ptr` for a word that is a ROM pointer.
- **`references(l)`** returns where a pointer to one of `l`'s results is stored.
- **`follow(l)`** returns the pointer stored at each of `l`'s results.
- **`offset(l, delta)`** returns each of `l`'s results plus `delta`.
- **`inferred(l)`** returns `l`, marked as resting on an assumed layout.

A game package can add its own locators for patterns specific to its game and combine them with these.

### Signatures

Signatures generate locators from a reviewed build:

- `codeSignatures` finds code that loads the address and masks literal offsets and branch targets, so the signature usually survives code moving around it.
- `accessSignatures` finds code that computes the address (`thumbAccess`), when no code loads it directly.
- When neither exists, `codeSignatures` falls back to code that loads a base just below the address, `inferred(offset(thumbLiteral(…), 1))`, which assumes the layout next to that base.
- `dataSignatures` matches a table's first bytes, with its pointers written as `ptr`, grown until the match is unique.
- `pointerSignatures` reaches a table made only of pointers through the data an entry points to: `offset(references(content(…)), -4 * entry)`.
- `valueSignatures` finds a constant through code that uses it, as a literal or an immediate.
  Code that uses a table's last index (a bounds check) gives `inferred(offset(…, 1))`, ranked after exact signatures.

Immediates are wildcards in these patterns, so a changed count is read rather than missed.

A value is found when every locator that finds something agrees on one value.
If code or data changed, its signature simply stops matching, which lowers the support for the value instead of giving a wrong one.
A value found only by inferred locators is flagged for review.

### Detection and probes

`detectFamily(family, rom)` checks cheaply whether a ROM looks like one of a family's builds, before locating everything.
It compares the header's game code with the family's `detect.gameCodes`, then runs a few locators (the family's `detect.markers`, or one locator from each of its first six sources).
The score is the share of those that find something.

A family can also list `probes`: named checks of a build's ROM data that throw when its resources don't decode as expected, for example well-known catalog entries.

### Recognizing builds at runtime

Only reviewed manifests are committed.
A family whose locators were generated from a reviewed build names it in `locatedFrom`, and `createGame` uses that build's manifest as the template for builds it has never seen.
It rejects a `locatedFrom` that isn't one of its reviewed manifests.

`locateGame(game, rom, options)` tries to recognize a ROM as a build of `game`:

1. `detectFamily` must score at least 0.75, unless `known` says the caller already knows the game, for example from a catalog.
   `onDetected` is called next, so a host only reports "recognized" when that is likely.
2. Every locator runs, with a pause between them (`yieldControl`, a macrotask by default) so the page stays responsive.
   `signal` aborts.
3. The build is rejected if any value's locators disagree, if an address isn't found, if the manifest checks fail or if a probe throws.
   Values no locator covers, such as strides and labels, come from the template.

The result is one of:

- `located`, with the new manifest (provenance `located`, the template as `parent`), the locator `version`, and a `review` list of values found only by inferred locators or kept from the template,
- `not-this-game`, with the detection result,
- `rejected`, with the reason.

`registerBuild(game, manifest)` adds an accepted build, after the same checks as `createGame`, so `resolveGame`, the host and the hooks treat it like a reviewed one.
`unregisterBuild` removes it again, and never removes a reviewed build.
`buildProvenance(game, romHash)` says whether a build is `reviewed`, `located` or unknown.
`locatable(game)` returns the family, the template and a `version` that changes whenever a locator or the template changes, so a cache of located manifests keyed by it never serves results from older locators.

## Game definitions

Underneath families, a game is an `id`, a non-empty list of `profiles` and a dictionary of `bridges`.
`defineGame` builds one directly.

Each profile is one exact build:

- `romHash`: the raw ROM's SHA-1, 40 lowercase hexadecimal digits.
- `source`: `repository`, `revision`, `variant`, `compiler` and `options`, saying where the build came from and how it was compiled.
- `symbols`: typed memory references for that build, by name.

The profile that matches the loaded ROM supplies every bridge's `symbols`.
All profiles of a game must have the same symbol names with compatible types.
A fork with different layouts uses different references under the same names, and a fork with different features gets its own game definition.
Keep profile exports precisely typed, not widened to `SymbolTable`, so the compiler checks each symbol.

A bridge has an `id` and a synchronous `decode` function, independent of any ROM.
Dictionary keys must equal bridge ids.
Ids only need to be unique within their game, so two games can share a bridge.
`defineGame` rejects invalid or duplicate hashes, an empty profile or bridge list, and a key that differs from its bridge's id.
It can't check that a hash or layout is true.

Types flow from the definition to the hook without casts or duplicated declarations:

- Inline decoders in `defineGame` infer their symbols from its profiles.
- A reusable `defineBridge` decoder declares `BridgeContext<RequiredSymbols>`, and adding it to a game that lacks those symbols is a type error.
- `useBridge(game, key)` checks `key` against that game's bridges and infers that decoder's data type, unions included.

### Prepared ROM resources

`definePreparedGame(prepare)` sets a resource type before the game's decoders are inferred.
The host calls `prepare({ rom, romHash })` once per loaded ROM, after exporting the ROM and checking its SHA-1, to build shared resources such as catalogs or artwork decoders.
`rom` is a `MemoryReader` over the cartridge image at `0x08000000`.
Decoders receive the result as `context.resources`, and `useBridgeResources(game, key)` gives it to components.

Resources are ROM data that can be cached, never live RAM or cached pointers.
Bridge results should carry small references to them rather than pixel buffers.
ROM tables and images come from the user's ROM, and nothing derived from a ROM (names, graphics, patches, saves, game source) is checked into this project.

## Decode results and states

`decode(context)` receives:

| Field       | Meaning                                                      |
| ----------- | ------------------------------------------------------------ |
| `memory`    | a `MemoryReader` over one captured snapshot                  |
| `symbols`   | the matched profile's symbols                                |
| `sample`    | the snapshot's `romHash`, `timeline`, `sequence` and `cycle` |
| `resources` | the prepared ROM resources, or `undefined`                   |

It returns `{ status: "ready", data }` or `{ status: "unavailable", reason }`.
Use `unavailable` for the normal moments with nothing to show: booting, scene transitions, records that aren't valid yet.
A valid empty collection is `ready` with an empty array.

What a hook or store reports:

| Status        | Meaning                                                                               |
| ------------- | ------------------------------------------------------------------------------------- |
| `idle`        | no game loaded, or nothing subscribed to this key                                     |
| `unsupported` | no profile matches the loaded ROM                                                     |
| `loading`     | waiting for the first snapshot                                                        |
| `unavailable` | the decoder had nothing valid for this snapshot, with a `reason`                      |
| `ready`       | decoded `data` for this snapshot                                                      |
| `error`       | the decoder threw or the capture failed, with a `message` and, if known, the snapshot |

Only `ready` has `data`, and `ready` and `unavailable` also carry their `sample`.
Data is deeply read-only in TypeScript.
Return fresh plain records, arrays, primitives and functions, not classes, `Map`s or typed arrays.
`decodeGame` enforces this at runtime: it rejects non-plain objects, then copies and deep-freezes records and arrays, without freezing the decoder's own scratch data.
Functions pass through as they are, so a view should return the same function while its inputs don't change.
None of this proves a decoder is pure or that the game's bytes are valid.

Games can show header data with `GameArenaStatus`: `{ title, fields: readonly GameStatusField[] }`.
Each field has a stable `id`, a `label`, and either a string `value` or `value: null` with a `reason`.
There is no fixed vocabulary (weather, terrain, buffs), and a missing value says why it is missing instead of pretending to be neutral.

## Reading memory

A `MemoryRef<T>` pairs an address with a `MemoryLayout<T>`: a byte length and a synchronous decoder.
`memory.read(ref)` returns a `T`.
The reader also has little-endian `u8`, `u16` and `u32` reads, which needn't be aligned, and `bytes(address, length)`, which returns a copy.

| Constructor                       | Result                                                          |
| --------------------------------- | --------------------------------------------------------------- |
| `ref(address, layout)`            | a reference to one value                                        |
| `u8`                              | a one-byte unsigned layout                                      |
| `array(address, element, length)` | a bounded array reference, at most 1 MiB                        |
| `at(arrayRef, index)`             | a checked reference to one element, without decoding the others |
| `pointer(layout)`                 | a 4-byte pointer layout that decodes to `MemoryRef<T> \| null`  |

Custom layouts express anything else: a record type, a field at an explicit offset, a bitfield.
`createMemoryReader(regions)` copies the regions it is given, and rejects reads that overlap regions, fall outside them, or go out of range.
Each read or array must fit in one region, and is limited to 1 MiB.
The reader captures nothing itself, and doesn't restrict addresses to RAM or ROM.

### Following pointers

A pointer layout returns a reference, not the value it points to.
Follow it explicitly:

```ts
const saveRef = memory.read(symbols.gSaveBlock1Ptr);
if (saveRef === null) {
  return { status: "unavailable", reason: "Save is not initialized" };
}
const save = memory.read(saveRef);
```

The compiler requires the null check and knows the fields of what it points to.
A null pointer decodes to `null`, and a pointer that isn't 4-byte aligned throws.
Every step reads the same snapshot.
Don't keep resolved pointers between snapshots, because games move structures such as save blocks around.

## Writing a bridge for a new game

There is no single base address from which every global follows.
Offsets are reliable inside a known layout, but independent globals need symbols.
Start from named roots:

1. Choose a decompilation revision and build configuration, or build a source-available fork with its symbols.
   Keep the ROM locally.
2. Match its ROM hash to a reviewed profile.
   For a new build, tie that build's symbols and layouts to its hash, and record the compiler, options and source revision.
   Don't apply a modern rebuild's symbols to a retail ROM without checking that they describe that exact file.
3. Start from a known root, such as a party array or a save-block pointer, and reach fields, elements and pointer targets through that build's typed layouts.
4. Reuse a family's shared record and text readers where they exist, pick the fields you need in `decode`, and render them with `useBridge`.
5. Test with synthetic records and snapshots captured locally, including boot, scene transitions, invalid pointers and loading a state.

## The host

`new BridgeHost(emulator)` serves one `Emulator` session.
`host.observe(game)` returns that game's store, created once per game object and shared by every consumer.
A store's `subscribe(listener, key?)` watches one bridge, or all of them when the key is left out.
Hooks always subscribe to their key.
Only watched keys are decoded, and the others report `idle`.

- **Consistent snapshots.** The host calls `captureMemory()`, which copies all of EWRAM (256 KiB) and IWRAM (32 KiB) in one step of the Worker, at one instruction boundary, with the ROM hash, cycle, timeline and sequence number.
  It doesn't advance the game or read I/O registers, the save chip or the ROM.
  Separate `readMemory()` calls while the game plays don't give a consistent snapshot.
- **How often.** `frame` and `clear` events ask for a capture.
  Requests merge into at most one capture in flight, started at least 50 ms after the previous one, so at most 20 a second.
  Nothing polls while paused, since pausing, stepping, loading, resetting and loading a state all send events that refresh the data.
  With no subscribers, the host stops listening.
- **Choosing the game.** Each snapshot is matched by ROM hash to exactly one watched game and profile.
  Two matches are an error, and none reports `unsupported`.
- **Timelines.** Loading, resetting, loading a state or rewinding starts a new timeline.
  The host then resets its stores to `loading` and ignores late replies from the old timeline or with an older sequence number.
  Speed changes keep the timeline.
- **Failures.** A decoder that throws gives `error` for its own key, without stopping play or the other bridges.
  A failed capture gives `error` for every watched key, instead of keeping stale data.
  The host never presents an old snapshot as current.
- **ROM resources.** For a game with `prepare`, the host exports the ROM once per ROM hash, checks its SHA-1, and caches the reader while watched.
  Hooks never get writable ROM bytes.
- **Cost.** Watching adds nothing to the emulated CPU's memory accesses.
  The cost is the copy and the decoders.

A snapshot is taken between two instructions, but it can still catch a game halfway through updating or decrypting a record, so decoders must check what they read.
Sampling may miss short-lived events, and doesn't pair data with a particular displayed frame.

`resolveGame(games, romHash)` and `decodeGame(game, sample, memory, resources?, keys?)` are the pieces the host is built from, available to other hosts and to tests.
`resolveGame` returns the one game with a matching profile, or `null`, and throws on duplicate game ids or several matches.
`decodeGame` runs every bridge, or the given keys, against one snapshot, which the caller must capture consistently.

## How it is checked

```sh
npm ci
npm run check
npm run build:wasm -w @evenfall/gba
npm test
```

`npm run check` type-checks the bridge code, and `npm test` runs the SDK's Node tests, skipping those that need the WebAssembly binary if it is missing.
Game-specific decoders belong to applications, which should test them against synthetic records and captured snapshots.
A synthetic test shows a decoder reads its format correctly, not that it works in the game.
[`testing.md`](testing.md) lists the other tests.

## Known limits

- Finding addresses relies on Thumb code loading RAM addresses from literal pools.
  A source no code names directly, such as a field reached as an offset from another global, is inferred from its neighbors or needs a review.
  Only a save state of the new build checks RAM sources live.
- Values no locator covers, such as strides, labels and counts no code uses distinctively, come from the template manifest.
  A new build that changes them needs a review, though probes catch some changes.
- Nothing imports a build's ELF, map file or debug information into typed layouts.
- Detection only says a ROM looks like a family's builds.
  A located build is accepted on its locators and probes, which check ROM data, so a wrong RAM address that still decodes without errors isn't caught.
- The built-in layouts are `u8`, arrays and nullable pointers.
  Bitfields, enums, signed values and strings need custom layouts.
- Errors carry only a message, not the symbol, field path, address or expected length.
  The `error` state has `address` and `path` fields that `decodeGame` doesn't fill yet.
- Each read is bounded, but a decoder has no overall budget, and can still loop or allocate without limit.
  None of the checks is a sandbox.
- Decoding cost hasn't been measured.
  Moving decoding to a Worker would need a measurement that justifies it.
- There is no inspector for browsing memory roots and pointers.
- Bridges are read-only.
  Writing game memory would need paused, atomic edits that fit with recording and rewind, which the design doesn't provide.
