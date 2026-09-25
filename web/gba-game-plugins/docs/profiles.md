# Pokémon bridge profiles and evidence

This is the evidence behind [`@evenfall/gba-pokemon`](../README.md): the exact builds
it recognizes, where each address and layout came from, and what is still
unverified. The generic bridge contract these profiles implement is in
[`docs/game-bridges.md`](../../../docs/game-bridges.md).

> Not affiliated with or endorsed by Nintendo, Creatures Inc., GAME FREAK inc.
> or The Pokémon Company. See the [disclaimer](../README.md#disclaimer).

## Shared semantic views

`PokemonPartyView` and `PokemonStorageView` are shared semantic structures,
independent of build layouts. Party and stored Pokémon share `PokemonDetails`
(identity, decoded names/text, level and experience, types, abilities, item,
stats, moves, warnings, and small artwork references); only party members add
live HP, status, ailments, and calculated stats. Storage does not invent those
party-only fields. IDs remain in the matched game's namespace.

`abilities: readonly PokemonAbility[] | null` describes equipped slots, not
alternative species abilities. Each entry has `slot`, `kind` (ability/innate/extra),
description and `state` (available/locked/suppressed/unknown), with an optional
reason. Available is not a claim that a conditional effect is firing. Null means
unresolved; an empty array means none. HnS supplies one verified slot. JSX does
not cap the list at four. Current move PP can be null when a storage format does
not retain it; the UI shows a dash rather than zero or invented restored PP.

Game-authored header data uses the SDK's generic
[`GameArenaStatus`](../../../web/src/bridge/context.ts). `gameContext.arena` is
HnS's producer; the shared `ArenaStatus` component does not enumerate weather,
terrain, trainer, or buff keys. New games may supply
their own fields. HnS currently publishes battle kind/opponent category and
unavailable live weather during combat, or explicitly labelled map weather
outside it. Trainer identity remains unavailable. Turn stays null in the battle
contract and is omitted from the UI, not simulated from frame counts.

`definePreparedGame` prepares the shared Gen III catalog and artwork decoder
once from the identity-checked user ROM. Catalog entries and decoded artwork
are cached outside live observations; bridge results carry small serializable
artwork references rather than pixel buffers.

`PokemonResources.linksFactory` optionally supplies pure `pokemon`, `ability`
and `move` URL factories; each accepts its semantic record and returns a URL
or `null`. Factories stay outside live snapshots. The selected-Pokémon inspector
renders a pixel-arrow link only when `pokemon` supplies a URL, including party,
PC, opponent and trainer-preview selections. It opens a separate tab without
an opener. Ability/move links are reserved in the contract but not rendered yet.
Elite Redux's exact-build manifest owns its Nextdex factory: Beta2 uses
`v=2.65beta` and the record's internal `species` ID, not its selection `id` or
National Dex number. Eggs return `null`; builds without factories show no link.

Held-item artwork uses the release ROM's `gItemIconTable` ABI: one 8-byte
record per item, containing pointers to LZ77-compressed 24×24 4bpp tiles (288
decoded bytes) and a compressed BGR555 palette (2–32 decoded bytes, with
omitted trailing colors zero-filled as the game does). Exact release tables
are `0x08907368` for HnS 1.2.1 (398 records, ending `0x08907fd8`) and
`0x08ae4b40` for Elite Redux 2.65 Beta2 (1023 records, ending `0x08ae6b38`).
These roots were found by bounded scanning for pointer-pair runs and then
checked directly in the two identity-pinned ROMs: every tile stream decodes to
288 bytes and every palette stream has an even 2–32-byte decoded length. Unit
coverage verifies tiling, owned buffers, ROM/ID rejection, and item/species
cache isolation. Opt-in exact-ROM tests decode item 1, MASTER BALL / Master
Ball, to a nonempty-alpha 24×24 image in each game and verify that its artwork
reference carries the matched ROM hash. No source-build address is promoted as
release evidence.

## Gen III record codec

The first bridge uses a narrow Gen III codec with verified build-specific
species bounds and egg flags, not a general C parser.

The stock 100-byte codec derives its layout, XOR key, checksum and permutation
facts from [the pinned FireRed structures](https://github.com/pret/pokefirered/blob/c75f352304d529f6ba92d4f74b9cf8b5c3810788/include/pokemon.h)
and [record accessors](https://github.com/pret/pokefirered/blob/c75f352304d529f6ba92d4f74b9cf8b5c3810788/src/pokemon.c).
It computes permutation positions rather than copying the game's dispatch
implementation. Selecting this codec for a build still requires independent
layout verification; there are no stock addresses or supported ROM hashes in it.

Primary-source observations behind starting from named roots (see
[the generic workflow](../../../docs/game-bridges.md#start-from-named-roots-not-a-universal-magic-address)):

- [FireRed](https://github.com/pret/pokefirered/blob/master/src/pokemon.c),
  [Emerald](https://github.com/pret/pokeemerald/blob/master/include/pokemon.h)
  and [Ruby](https://github.com/pret/pokeruby/blob/master/src/pokemon_1.c)
  declare party/count globals. These are useful entry points, not proof that
  an arbitrary fork preserves their type or semantics.
- [FireRed](https://github.com/pret/pokefirered/blob/master/src/load_save.c)
  and [Emerald](https://github.com/pret/pokeemerald/blob/master/src/load_save.c)
  expose active save/storage pointer globals; [Ruby's save code](https://github.com/pret/pokeruby/blob/master/src/load_save.c)
  uses direct save-block globals. Do not read a backing buffer instead of the
  currently active pointer merely because both symbols exist.
- [Emerald's Game Freak header](https://github.com/pret/pokeemerald/blob/master/src/rom_header_gf.c)
  and [FireRed's header](https://github.com/pret/pokefirered/blob/master/src/rom_header_gf.c)
  contain useful table pointers, save sizes and selected field offsets. They
  are not complete symbol/type information or a universal Gen III contract;
  Ruby does not provide that same header. They do not replace record codecs.
- [FireRed](https://github.com/pret/pokefirered/blob/master/Makefile),
  [Emerald](https://github.com/pret/pokeemerald/blob/master/Makefile) and
  [Ruby](https://github.com/pret/pokeruby/blob/master/Makefile) expose linker/build
  artifacts. Debug-info coverage depends on compiler and flags. ELF does not
  guarantee complete DWARF; map/symbol files do not describe C member layouts.
  Validate optional debug types or use compiler-assisted target `sizeof` and
  `offsetof` extraction plus reviewed bitfield facts. Never guess layout from
  a host compiler, and never silently substitute a different ROM build.

## Heart & Soul 1.2.1 profile evidence

This profile is for the **1.2.1 release ROM**, not arbitrary builds of
the moving `1.2` branch. Source is the
[Release-v1.2.1 commit](https://github.com/PokemonHnS-Development/pokemonHnS/commit/4aa8fa894db4c890b3d1100abd6aab50f44c22b1).
Raw ROM size: 33,554,432 bytes. SHA-256:
`7c4f90c8b68b64d4639a37306d5302a11cd4d7c44005114ec2faa2b28b210c3d`.
Its CRC32 `96a8425b` matches the official release UPS output checksum; the
UPS SHA-256 is `c8e70f448b481d2980266ca8f021aa8b3c462f4e582cf80228ced4636c6154eb`.
No ROM, save, patch or game-source assets are committed here.

The release does not publish a compiler identity or ELF/map. The source prescribes
devkitARM r65. A local ARM GCC 16.2 layout check using the source ABI flags
(`-mthumb -mthumb-interwork -mabi=apcs-gnu -mtune=arm7tdmi -march=armv4t`)
corroborates the 12-byte Misc substructure, 100-byte party record and egg bit 31;
it is **not a matched rebuild**, and its linker addresses are not used.
Reviewed [record declarations](https://github.com/PokemonHnS-Development/pokemonHnS/blob/4aa8fa894db4c890b3d1100abd6aab50f44c22b1/include/pokemon.h)
and [species constants](https://github.com/PokemonHnS-Development/pokemonHnS/blob/4aa8fa894db4c890b3d1100abd6aab50f44c22b1/include/constants/species.h)
give these differences from stock: five-bit Poké Ball field moves `isEgg` from
bit 30 to bit 31; populated species end at Kleavor (455), reserved ids are
456–461, and the egg display sentinel is 462. Encryption/checksum/permutation
and party status/level/HP fields retain the shared layout.

Address evidence comes from the **identity-checked release binary**:

- `gPlayerPartyCount`: `0x0201d9e9` (u8).
- `gPlayerParty`: `0x0201d9ec` (six 100-byte records).
- At guest `0x080b983c`, the routine corresponding to
  [`CheckRelicanthWailord`](https://github.com/PokemonHnS-Development/pokemonHnS/blob/4aa8fa894db4c890b3d1100abd6aab50f44c22b1/src/braille_puzzles.c#L93)
  checks Wailord (314) first and Relicanth (381) last, using the party/count
  literals at ROM offsets `0xb9880`/`0xb9884` and stride 100.
- Its call to guest `0x0819060c` matches
  [`CalculatePlayerPartyCount`](https://github.com/PokemonHnS-Development/pokemonHnS/blob/4aa8fa894db4c890b3d1100abd6aab50f44c22b1/src/pokemon.c#L8102):
  zero the count, scan species, increment until empty/capacity. Count/party
  literals are at ROM offsets `0x190644`/`0x190648`; the stride instruction is
  at guest `0x08190610`. Literal occurrence counts alone are not this evidence.

The expanded release profile also has independently verified ROM tables and
target strides. Names are at `0x08744798` (462 × 11), species info at
`0x0893bd40` (490 × 40), move names at `0x08745b74` (368 × 13), moves at
`0x08937998` (368 × 12), abilities at `0x086e1e90` (82 × 13), types at
`0x086e152c` (19 × 7), nature pointers at `0x08965f48` (25 × 4), items at
`0x088fe910` (398 × 44), and experience at `0x0893b0a0` (6 × 404). Verified
graphics roots are the icon table/index/palettes at
`0x08952b3c`/`0x089532e4`/`0x089534d0`, front pictures at `0x08732678`, and
normal/shiny/modern-shiny palettes at
`0x0872a28c`/`0x0872b1dc`/`0x0872c12c`. These addresses describe assets in the
identity-checked ROM only; they are not provenance for redistributing assets.

Runtime root verification found `gSaveBlock1Ptr`, `gSaveBlock2Ptr`, and
`gPokemonStoragePtr` at `0x03003740`, `0x03003744`, and `0x03003748`.
`SaveBlock1` is `0x3db8` bytes and its relevant settings begin at `+0x3d9d`.
`PokemonStorage` is `0x8d34` bytes: current box at `+0`, records at `+4`, 15
boxes of 30 records with record stride `0x50` and box stride `0x960`, names at
`+0x8ca4` with stride 9, and wallpapers at `+0x8d2b`. This deliberately rejects
stale source comments that place boxes/names/wallpapers at `+1`, `+0x8344`, and
`+0x83c2`. Party records are `0x64`; the fork's secure Misc layout places egg
at word-1 bit 31 and ability slot at word-2 bit 0, rather than stock comments.

HnS randomized party/storage types and abilities and its base-stat equalizer are
detected but not resolved: semantic fields become unavailable/null with warnings
rather than silently using stock values. Active battlers instead publish their
authoritative live type and ability fields. Battle turn count and live weather
have no verified release symbols yet, so they remain null with warnings;
map-configured weather is separate and available through `gameContext`. Spinda
spot-rendered and Deoxys tile-processed front portraits are unsupported. The
implementation uses the authoritative release `species.h` Deoxys ID **410**;
the static-ROM artifact's text says 386 and is a known source discrepancy, not
the bridge value.

Reproduce with a user-supplied, hash-checked raw ROM:

```sh
sha256sum "$ROM"
arm-none-eabi-objdump -D -b binary -m arm -M force-thumb \
  --adjust-vma=0x08000000 --start-address=0x080b983c --stop-address=0x080b9888 "$ROM"
arm-none-eabi-objdump -D -b binary -m arm -M force-thumb \
  --adjust-vma=0x08000000 --start-address=0x0819060c --stop-address=0x0819066c "$ROM"
```

Local smoke evidence: `game.state` SHA-256
`0b008f9170f4a5d4189b2202ff41880d9a29558bdddb9933145829862c0d2354`,
loaded with HLE, RTC enabled/epoch 0 and the above ROM, decodes to a **valid
empty party** at cycle 1,413,468,675. The supplied save also exposes no Pokémon
menu before acquiring a starter. At that stage, occupied records were only
fixture-tested. With ReleaseSafe WASM SHA-256
`4ae4a4e0a0bedd527046be02b3336d106c161e991c2b78d1db155473abdef5ce`, two cores
restored from this state and advanced for 60 runs of `currentCycle + 280896`
with no new input: capture every third run versus no observer produced identical
per-run drained audio, final video and serialized state (SHA-256
`96657be8d8df1e3c816f7dbddbd0a5a2dc9f23e12012c7af59f8f5b63ed4c267`).
The raw report was kept locally and is not published. This is a
bounded non-interference check, not a full-game or performance qualification.

A newer live state, `game2.state`, has SHA-256
`71817992f9053560808493c79c3e65b5acba2b4499375cc52d76806b2107fb5f` and was
captured at cycle 54,445,509,893. Independent inspection established the party
as TOTODILE (internal species 158), level 10, HP 29/33; PIDGEY (16), level 3,
HP 16/16; and HOPPIP (187), level 2, HP 14/14. With the same ReleaseSafe WASM
identity above, full rich decoding produces those values, 15 named empty PC
boxes, and inactive battle state. Totodile has MODEST nature, TORRENT ability,
SCRATCH (32/35 PP), LEER (30/30), and WATER GUN (25/25); Pidgey exposes TACKLE
and KEEN EYE, and Hoppip exposes SPLASH and CHLOROPHYLL. All three icons and
front portraits decode from the user ROM. The optional
[`hns-live.spec.ts`](../tests/browser/hns-live.spec.ts) exercises
the unmodified production player, exact fixture hashes, imported state, names,
HP, artwork dimensions, move categories, click/keyboard selection, reload
invalidation, and desktop/mobile rendering. It is not full-game qualification.

The supplied wild-battle state, `battle.state`, has SHA-256
`5515f43a8119632270b2f64a447917fd826bc7cf16655f3276bbb0f3c4d42e54` and was
captured at cycle 55,823,304,776. The same identity-checked ROM and current WASM
decode it as a single wild battle on Route 29: player slot 0 is TOTODILE, level
10, HP 26/33; opponent slot 0 is HOPPIP, level 3, HP 16/16. The live battlers
carry neutral stages and modified stats 33/18/20/16/17/17 for Totodile and
16/8/8/7/8/9 for Hoppip. Live moves and PP are SCRATCH 30/35, LEER 30/30,
WATER GUN 25/25, and SPLASH 40/40. Both parties retain ROM-backed semantic
details and artwork references.

The same verified ROM catalog now publishes move priority from signed byte
`gBattleMoves + 7` (for example QUICK ATTACK is `+1`) and item descriptions
through the `gItems + 20` ROM pointer. Description reads are EOS-terminated and
bounded to 256 bytes; unsupported control text is returned as unavailable
rather than displayed as fabricated prose. The MASTER BALL pointer independently
decodes to the three source lines ending “without fail.”

Ability and move descriptions also come from the identity-checked ROM:
`gAbilityDescriptionPointers` at `0x086e22bc` (82 × 4, indexed by ability ID)
and `gMoveDescriptionPointers` at `0x089658c8` (367 × 4, indexed by move ID − 1;
move 0 has no description). Unique encoded-string anchors located the pointer
tables; every pointer's terminated bytes were then independently compared to
the pinned source's
[abilities](https://github.com/PokemonHnS-Development/pokemonHnS/blob/4aa8fa894db4c890b3d1100abd6aab50f44c22b1/src/data/text/abilities.h)
and [move descriptions](https://github.com/PokemonHnS-Development/pokemonHnS/blob/4aa8fa894db4c890b3d1100abd6aab50f44c22b1/src/data/text/move_descriptions.h),
using its charmap (including apostrophe `0xb4`, newline `0xfe`, EOS `0xff`).
All 82 ability and 367 move entries match; maximum lengths including EOS are
28 and 60 bytes respectively. Profile bounds are 32 and 64 bytes. No modern
rebuild addresses are used. Shared `DescribedValue` adds optional description
and warnings to abilities/moves/items; unknown control codes or unterminated
text are unavailable, not partially fabricated descriptions. The UI opens these
on hover, keyboard focus or tap, with move power/category/priority from the
same catalog. Browser synthetic overlays change capture data only, never RAM.

Release-binary literals and the captured structures verify these runtime roots:

- `gMain`: `0x03001574`; packed `inBattle` is byte `+0x439`, bit 1, at
  `0x030019ad`. It is 1 in `battle.state` and 0 in `game2.state`. The previous
  candidate `0x03001ded` is executable IWRAM and was rejected.
- `gBattleTypeFlags`: `0x020003a0`; `gBattlersCount`: `0x02001420`;
  `gBattlerPartyIndexes`: `0x02001422` (four u16 values); `gBattleMons`:
  `0x0200143c` (four `0x58`-byte records); `gAbsentBattlerFlags`:
  `0x020015c8`.
- `gEnemyParty`: `0x0201de9c` (six `0x64`-byte records). The active battle
  records' personality/trainer ids exactly link to player/enemy party slot 0;
  inactive `game2.state` deliberately demonstrates that the battle records are
  stale unless the verified `gMain.inBattle` bit is set.
- `gMapHeader`: `0x0200c220`; its section/weather bytes are `+0x14`/`+0x16`.
  The active `gSaveBlock1Ptr` record supplies position `(22, 13)` and location
  `(group 0, map 11)`, matching pinned map metadata for Route 29, section
  `0x58`, configured weather 0 (None).

The active bridge validates counts, occupied records, checksums, party indexes,
battler identity linkage, species/move bounds, HP/level, and stage range before
publication. Synthetic mutations cover asymmetric live stats/stages, stale
identity linkage, and invalid stages. Extraction evidence and inspected frames
were kept locally and are not published.

### Heart & Soul checks

The first stage tested stock party basics against synthetic encrypted records
covering all 24 permutations, eggs, fainting, capacity and corruption.
HnS-specific tests distinguish egg bit 31 from stock bit 30 and populated
species through 455 from reserved/sentinel values. Synthetic encrypted records
exercise the real HnS decoder, host and hook, including fainting, eggs,
checksum failure and recovery. The player selects the dashboard for the exact
HnS hash; `/debug.html#/hud` remains a diagnostic override.
`usePokemonArtwork(resources)` provides bounded cached image URLs outside
snapshots. The real `game2.state` browser test corroborates the occupied team.
Observation cost is not measured, and there is no performance or complete
gameplay compatibility claim.

## Elite Redux 2.65: one layout family, exact-build manifests

[`elite-redux.ts`](../src/elite-redux.ts) registers `gameParty`,
`gameBattle`, `gameStorage`, and `gameContext`. `createEliteReduxGame(builds)`
accepts compatible `EliteReduxBuild` manifests: layout ID, exact raw-ROM hash,
source provenance, typed RAM symbols, catalog roots/counts and artwork roots.
Resource preparation selects the same hash as live decoding. To support a
nearby revision, verify its manifest and append it; do not duplicate decoders or
React components. Beta1 is **not** currently registered. A changed struct ABI
needs an explicitly revised layout family, not relaxed hash matching.

The player connects these outputs to the same `PokemonDashboard`, roster,
inspector, move list, artwork hook and tooltips used by HnS. Only the connector
knows the game definition. [`redux-record.ts`](../src/redux-record.ts)
owns packed RAM decoding; [`redux-catalog.ts`](../src/redux-catalog.ts)
adapts ROM tables to shared Pokémon metadata. Redux records are unencrypted and
have no Gen III checksum. Its box/party/battle sizes are 0x34/0x4c/0x68,
respectively; using stock Emerald/HnS codecs is incorrect.

### Release evidence

- Source [06da575972cfed20d471f8bb5b90ea9e4736b16a](https://github.com/Elite-Redux/eliteredux-source/tree/06da575972cfed20d471f8bb5b90ea9e4736b16a),
  config submodule `3073176bb3a5abf7a1fe52c23e61bcdf580e4685`; release label
  v2.65 Beta2, game version 1048.
- ZIP SHA-256 `8c8ac8c6587abc85d1f7472df3adcf847f4d5eef99797a79ab6a14b614a7a432`.
- Raw ROM: 33,554,432 bytes, SHA-256
  `d86e6ac909e4abdef9159ed20b75a684a9b2ee2fe874f388122e00a5749d2d37`.
- `redux.state` SHA-256
  `bf403592558c7a6cdc0deb493f8ab26f8796dfec829f0a2031fcbbc44af66d23`,
  capture cycle 12,852,115,585. Trainer battle: Frigibax Lv20 62/62 vs
  Mudkip Lv20 58/58. Player roster Frigibax/Klink/Aron/Goomy; opponent
  Mudkip/Goomy. Frigibax has Mountaineer, Thermal Exchange, Heatproof and
  Freezing Point, read from live battle slots.
- Local evidence (not published): a layout report, target header probes,
  disassembly, and captured RAM. ARM GCC 16.2 header
  probes used `-mthumb -mthumb-interwork -mabi=apcs-gnu -march=armv4t
  -fshort-enums`; generated enum headers came from the pinned config. Missing
  libc headers required scratch stubs. This was **not** a matched ROM rebuild.
  Release disassembly overrides compiler-sensitive packing: ability metadata
  is at +0xb8 (not modern GCC's +0xb6), neutralizing gas at timer +0x11.

Verified RAM roots: player count/party 0x02039905/0x02039908; enemy party
0x02039ad0; main/inBattle 0x03003420/0x03003859 bit1; battle flags 0x0201c3f4;
battler count/indexes/records 0x0201c484/0x0201c488/0x0201c50c;
absent mask 0x0201c6db; SaveBlock2/storage pointer roots
0x03003410/0x03003414. `gEnemyPartyCount` is zero in the supplied trainer
battle; occupied enemy slots are determined from their packed species fields.
Storage is 26 boxes × 30 slots, with 0x34-byte records and names at +0x9e74.

ROM header pointers corroborate species names 0x089c16c8 (pointer table),
species data 0x08bf5b0c (stride44), move names 0x089c7a24 (stride13), moves
0x08bebfac (stride24), and items 0x08ab9674 (stride40). Abilities are at
0x08fbc4d8 (stride192); name/short/expanded description pointers are +0/+4/+8.
Other roots and table bounds are in the typed manifest. Text is bounded and
unrecognized control codes remain warnings, never silently invented prose.

Player innate thresholds are Easy/Ace 0/0/0, Elite 1/17/24 and Hell 10/17/24;
enemies bypass those level locks. Release `CanDisableInnates` (0x081b085c)
and `getInnateDisableLevel` (0x081b0878) verify difficulty at SaveBlock2 +0xc89
high nibble. RandomizeInnate/Ability/Type (0x081ae08c/0x081ae100/0x081ae174)
verify +0xc8a bits3/5/7. The supplied save is Easy with these randomizers off.
The battle reader uses the actual four slots
plus up to three extra Hell abilities. Persistent suppression follows verified
`IsSuppressed` (0x080da638) and `DoesBattlerHaveAbilityShield` (0x080d2c18):
Gastro Acid is statuses3 bit16 at 0x0201c7ac; gas is 0x0201cb69 bit0;
unsuppressable/blocks-suppression metadata are word +0xb8 bits11/13.
Ability-based shielding checks four normal slots, while item-based shielding
uses hold effect62 and is blocked by Embargo (statuses3 bit17). Enigma Berry's
runtime hold effect is read separately. Contextual Mold Breaker, conditional
activation and every damage modifier are not presented as a global enabled flag.

Battle records retain pre-stage `stats.actual` and publish separate
`stageAdjustedStats`, used by the active inspector's Total column. HnS and Redux
use the standard stage ratios: multiply by `(2 + stage) / 2` for nonnegative
stages, or `2 / (2 - stage)` for negative stages, then floor; HP is unchanged.
Redux's pinned [ratio table](https://github.com/Elite-Redux/eliteredux-source/blob/06da575972cfed20d471f8bb5b90ea9e4736b16a/src/pokemon.c#L172-L186)
and [stat calculation](https://github.com/Elite-Redux/eliteredux-source/blob/06da575972cfed20d471f8bb5b90ea9e4736b16a/src/battle_util.c#L7309-L7315)
keep raw fields and stage multiplication separate. These totals exclude
contextual ability/item/weather/move modifiers and critical-hit/Unaware stage
bypass; they are not universally effective damage stats.

The supplied `redux-stage-buff.state` (SHA-256
`d092f7bd0f323b1720321f16b72291753748c03fba63454fe8d19455ec7b472d`, cycle
18,089,983,297) retains Mudkip's raw Attack/Defense 41/31 and has stages +1/+1,
giving totals 61/46 after integer flooring. Max HP 58 and Speed 29 remain
unchanged. `ZIGBA_REDUX_STAGE_STATE` enables the browser regression that loads
this state, checks both totals, and restores the original state's neutral stages.

Limits: randomized party types/abilities remain unresolved (null plus warning),
but active battlers use live values. Rare/legendary shiny artwork remains
unavailable instead of using a wrong palette. Turn, live weather, trainer name
and map name are not verified. No mutation API or all-game compatibility claim.

Verification: `elite-redux.test.ts` covers packed asymmetric fields, slot/lock
boundaries, compatible build selection and optional exact-ROM/state tests.
Supply `ZIGBA_REDUX_ROM` and `ZIGBA_REDUX_STATE` to run private fixture tests
and the shared console Chromium test; nothing private is bundled. The browser
checks real four-ability rendering, descriptions via focus/touch, native-aspect
full-width canvas, desktop/mobile overflow, and selected ROM artwork.

### Trainer previews resolve before reaching the shared UI

`PokemonTrainerSearch` returns `PokemonTrainerPreview[]`: stable encounter ID,
source/internal ID, name, bridge-authored variant, class, typed portrait,
average level, warnings, and an ordinary `PokemonPartyMember[]`. The inspector
has no difficulty/scaling/EV rules. Unknown generated personality, trainer ID,
gender, friendship and experience remain absent/unavailable. Starting HP is
max HP; known IVs, applied EVs and calculated stats use the ordinary fields.

Redux's `gameTrainerContext` reads difficulty, EV enablement, highest nonempty
non-egg level across all six slots (including fainted Pokémon), and the third
badge from one captured memory observation. The ROM resource
`searchTrainers(query, context)` resolves the selected difficulty's party,
signed level offsets, Hell's post-badge +5 and level cap100, Easy/disabled EVs,
nature, Iron Pill's zero Speed IV, Shedinja-family HP, PP Max, and Hell extra
abilities. Context is per session; the connector invalidates search only when
relevant values change, not every frame. The ROM generator already supplies
fallback Elite/Hell party pointers. No game memory is written or executed by
the resolver.

Verified beta2 roots: trainers `0x089a2ffc`, 932 entries of `0x38`; trainer
Pokémon **stride `0x1c`**, not `0x1a` (two trailing APCS padding bytes); class
names `0x089be5b4`, 73 × 13 bytes; portraits `0x08963d64` and palettes
`0x08964074`, 98 × 8 bytes; Hell skills `0x0850da44`, 932 × 24 bytes.
SaveBlock1 pointer `0x0300340c` leads to badge03 at `+0x1ae1` bit3.
SaveBlock2's EV toggle is `+0xc8c` bit2. Evidence and exact input identities
were recorded locally and are not published.

Roxanne's initial and four rematch records have explicit singles/doubles and
legends labels. The Redux bridge groups results alphabetically by name, then
uses build-authored encounter order, with internal ID as the final tie-breaker.
May and Brendan follow Route103 → Rustboro → Route110 → Meteor Falls (partner)
→ Route119 → Lilycove / Route103 rematch, with the player's starter choice
shown for every variant. The pinned source's `data/maps/*/scripts.pory` routes
these encounters: May IDs63/70/87 and Brendan104/113/122 are Meteor Falls
**partner** teams, despite their low IDs; Route103 rematches reuse Lilycove's
records, and Route104 can use the Rustboro records. Sorting by ID or scaled
level would not give encounter progression. Unverified duplicate variants keep
conservative encounter-number labels and ID order after verified variants;
the shared UI does not infer or re-sort chronology.

This is an encounter catalog, **not an eligibility list**: it does not claim a
rematch is unlocked. Procedural OLDPLAYER (ID2000) and
special facility/secret-base parties are outside this ordinary trainer catalog.
Hell's contextual battle rules are noted, not folded into starting totals.

Only a ready, inactive battle shows search. Live battles and unavailable/error
observations take precedence; timeline replacement clears preview selection.
Dropdown and selected team use shared ROM artwork, roster, inspector and
tooltips. Keyboard selection, Escape, click/touch dismissal, same-name
variants, context refresh and mobile viewport bounds are exercised in browser
tests. The real-ROM browser fixture masks the battle-active bit **in a captured
copy**, because the supplied Redux state is in combat; it does not alter the
emulator or claim to be an actual overworld save. Real-ROM tests additionally
check Easy/Ace totals, Hell abilities, portraits, signed scaling boundaries,
EV enablement and fainted/egg selection. No user ROM or state is bundled.

### pokeemerald-expansion is not a universal binary ABI

Inspected upstream [4c680433909c7fb2219cc9e755f05cdd081d4778](https://github.com/rh-hideout/pokeemerald-expansion/tree/4c680433909c7fb2219cc9e755f05cdd081d4778)
(1.17.1 metadata). Its GF public header and append-only `RHHEXP` header expose
ROM table pointers, counts, version and selected save offsets. They do **not**
provide all live RAM globals or a versioned struct-layout ABI. Configuration
changes layouts; current parties use `gParties`/`gPartiesCount`, and save blocks
relocate. A safe next implementation is a shared expansion decoder family plus
an ELF/symbol-generated manifest bound to the final ROM hash, optionally seeded
from those public ROM headers. This can support many hacks with small manifests;
it cannot honestly auto-enable every expansion hack from its title/header alone.
Manifest generation remains future work, not an implemented universal bridge.
