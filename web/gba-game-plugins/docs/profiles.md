# Pokémon bridge profiles and evidence

This is the evidence behind [`@evenfall/gba-pokemon`](../README.md): the exact
builds it recognizes, where each address and layout comes from, and what
remains unverified. The generic bridge contract these profiles implement is
in [`docs/game-bridges.md`](../../../docs/game-bridges.md).

> Not affiliated with or endorsed by Nintendo, Creatures Inc., GAME FREAK inc.
> or The Pokémon Company. See the [disclaimer](../README.md#disclaimer).

## Evidence rules

- A profile applies to one exact build, identified by the SHA-1 of the raw
  ROM. Titles, header codes and nearby revisions never match.
- Addresses come from the identity-checked release binary: disassembly of
  the routine that uses a global, literal pools, ROM header pointers, and
  captured RAM from states of that build. Source links point at the pinned
  commit of the release. Addresses from rebuilding the source with a
  different compiler are not used, because they do not describe the release.
- Target-compiler `sizeof`/`offsetof` probes may corroborate a struct layout;
  release disassembly overrides them where packing is compiler-sensitive.
- A value that cannot be verified is published as unavailable or `null` with
  a warning, never as a guessed or stock value.
- ROM addresses describe assets in a user's own ROM. They are not provenance
  for redistributing any asset, and nothing decoded from a ROM is committed.

## Shared semantic views

`PokemonPartyView` and `PokemonStorageView` are shared semantic structures,
independent of build layouts. Party and stored Pokémon share `PokemonDetails`
(identity, decoded names and text, level and experience, types, abilities,
item, stats, moves, warnings, and small artwork references); only party
members add live HP, status, ailments and calculated stats. Storage does not
invent those party-only fields. IDs stay in the matched game's namespace.

`abilities: readonly PokemonAbility[] | null` describes equipped slots, not
alternative species abilities. Each entry has `slot`, `kind` (ability,
innate or extra), description and `state` (available, locked, suppressed or
unknown), with an optional reason. Available does not claim that a
conditional effect is firing. `null` means unresolved; an empty array means
none. Heart & Soul supplies one verified slot; the UI does not cap the list
at four. Current move PP can be `null` when a storage format does not retain
it, and the UI shows a dash rather than zero or an invented value.

Game-authored header data uses the SDK's generic
[`GameArenaStatus`](../../../web/src/bridge/context.ts), produced by each
game's `context` view. The shared `ArenaStatus` component does not enumerate
weather, terrain, trainer or buff keys, so a game may supply its own fields.
Battle turn stays `null` in the battle contract and is omitted from the UI
rather than simulated from frame counts.

Each family's `resources` prepares the Gen III catalog and artwork decoder
once from the identity-checked user ROM. Catalog entries and decoded artwork
are cached outside live observations; bridge results carry small serializable
artwork references rather than pixel buffers.

`PokemonResources.linksFactory` optionally supplies pure `pokemon`, `ability`
and `move` URL factories; each accepts its semantic record and returns a URL
or `null`. Factories stay outside live snapshots. The selected-Pokémon
inspector renders a link only when `pokemon` returns a URL, for party, PC,
opponent and trainer-preview selections, and opens it in a new tab without
an opener. The inspector does not render ability or move links.

### Held-item artwork

Held-item artwork uses the release ROM's `gItemIconTable` layout: one 8-byte
record per item, holding pointers to LZ77-compressed 24x24 4bpp tiles (288
decoded bytes) and a compressed BGR555 palette (2-32 decoded bytes, with
omitted trailing colors zero-filled as the game does).

| Build                  | Table        | Records | End          |
| ---------------------- | ------------ | ------- | ------------ |
| Heart & Soul 1.2.1     | `0x08907368` | 398     | `0x08907fd8` |
| Elite Redux 2.65 Beta2 | `0x08ae4b40` | 1023    | `0x08ae6b38` |
| Elite Redux 2.65 Beta3 | `0x08ae4b0c` | 1023    | `0x08ae6b04` |

The roots were located by scanning for runs of pointer pairs and checked in
each identity-pinned ROM: every tile stream decodes to 288 bytes and every
palette stream to an even length of 2-32 bytes.

## Gen III record codec

The stock 100-byte codec derives its layout, XOR key, checksum and
substructure permutation from
[the FireRed structures](https://github.com/pret/pokefirered/blob/c75f352304d529f6ba92d4f74b9cf8b5c3810788/include/pokemon.h)
and [record accessors](https://github.com/pret/pokefirered/blob/c75f352304d529f6ba92d4f74b9cf8b5c3810788/src/pokemon.c)
at a pinned commit. It computes permutation positions rather than copying
the game's dispatch code. The codec is narrow: build-specific species bounds
and egg flags are parameters, and it contains no addresses or ROM hashes.
Selecting it for a build still requires independent layout verification.

### Named roots in the decompilations

Profiles start from named globals rather than a universal magic address (see
[`docs/game-bridges.md`](../../../docs/game-bridges.md)). What the pret
decompilations offer, and its limits:

- [FireRed](https://github.com/pret/pokefirered/blob/master/src/pokemon.c),
  [Emerald](https://github.com/pret/pokeemerald/blob/master/include/pokemon.h)
  and [Ruby](https://github.com/pret/pokeruby/blob/master/src/pokemon_1.c)
  declare party and count globals. These are entry points, not proof that a
  fork preserves their type or meaning.
- [FireRed](https://github.com/pret/pokefirered/blob/master/src/load_save.c)
  and [Emerald](https://github.com/pret/pokeemerald/blob/master/src/load_save.c)
  expose active save and storage pointer globals;
  [Ruby](https://github.com/pret/pokeruby/blob/master/src/load_save.c) uses
  direct save-block globals. Read through the active pointer, not a backing
  buffer that happens to have a symbol.
- [Emerald's Game Freak header](https://github.com/pret/pokeemerald/blob/master/src/rom_header_gf.c)
  and [FireRed's](https://github.com/pret/pokefirered/blob/master/src/rom_header_gf.c)
  contain table pointers, save sizes and selected field offsets. They are not
  complete symbol or type information, Ruby has no such header, and they do
  not replace record codecs.
- The [FireRed](https://github.com/pret/pokefirered/blob/master/Makefile),
  [Emerald](https://github.com/pret/pokeemerald/blob/master/Makefile) and
  [Ruby](https://github.com/pret/pokeruby/blob/master/Makefile) builds produce
  linker artifacts, but debug-info coverage depends on compiler and flags:
  ELF does not guarantee complete DWARF, and map or symbol files do not
  describe C member layouts. Validate debug types or use target-compiler
  `sizeof`/`offsetof` extraction plus reviewed bitfield facts; never infer a
  layout from a host compiler or silently substitute a different build.

## Heart & Soul 1.2.1

### Build identity

The profile covers the **1.2.1 release ROM**, not arbitrary builds of the
`1.2` branch. Source: the
[Release-v1.2.1 commit](https://github.com/PokemonHnS-Development/pokemonHnS/commit/4aa8fa894db4c890b3d1100abd6aab50f44c22b1).

| Property          | Value                                                              |
| ----------------- | ------------------------------------------------------------------ |
| Raw ROM size      | 33,554,432 bytes                                                   |
| Raw ROM SHA-1     | `a4be81f8330d7f223a3ab847c3da822fe6f8a2e0`                         |
| Raw ROM CRC32     | `96a8425b`, matching the official UPS patch's output checksum      |
| UPS patch SHA-256 | `c8e70f448b481d2980266ca8f021aa8b3c462f4e582cf80228ced4636c6154eb` |

The **1.2.0 release ROM** has no committed manifest: the player locates it
with the 1.2.1 locators. Source: the
[Release-v1.2.0 commit](https://github.com/PokemonHnS-Development/pokemonHnS/commit/f54ee7e82d856fa43e5d714c5689304ce92f6f67).

| Property      | Value                                                                                     |
| ------------- | ----------------------------------------------------------------------------------------- |
| Raw ROM SHA-1 | `b8a4b491ca67a76f079a8e7c19fd5512a2147dd2`                                                |
| Raw ROM CRC32 | `cc143d14`, matching the official 1.2.0 UPS patch's output checksum                       |
| Manifest      | Located at runtime from the 1.2.1 locators; relocation from 1.2.1 agrees on every address |
| To review     | `rom.catalog.moveDescriptions.count`, inferred from a bounds check                        |

Its RAM layout equals 1.2.1's, and every ROM table sits `0x180` bytes lower.

### Record layout

The release publishes no compiler identity, ELF or map; the source
prescribes devkitARM r65. The
[record declarations](https://github.com/PokemonHnS-Development/pokemonHnS/blob/4aa8fa894db4c890b3d1100abd6aab50f44c22b1/include/pokemon.h)
and [species constants](https://github.com/PokemonHnS-Development/pokemonHnS/blob/4aa8fa894db4c890b3d1100abd6aab50f44c22b1/include/constants/species.h)
differ from stock in three ways:

- a five-bit Poké Ball field moves `isEgg` from bit 30 to bit 31 of Misc
  word 1; the ability slot is word 2 bit 0;
- populated species end at Kleavor (455); IDs 456-461 are reserved and 462
  is the egg display sentinel;
- the Deoxys species ID is **410**, as in the release `species.h`. A static
  ROM text artifact says 386; the bridge follows the header.

Encryption, checksum, permutation and the party status, level and HP fields
keep the shared layout; party records are `0x64` bytes. An ARM GCC 16.2
layout probe with the source ABI flags (`-mthumb -mthumb-interwork
-mabi=apcs-gnu -mtune=arm7tdmi -march=armv4t`) corroborates the 12-byte Misc
substructure, the 100-byte party record and egg bit 31. It is not a matched
rebuild, and its linker addresses are not used.

### Party roots

| Global              | Address      | Notes                |
| ------------------- | ------------ | -------------------- |
| `gPlayerPartyCount` | `0x0201d9e9` | u8                   |
| `gPlayerParty`      | `0x0201d9ec` | six 100-byte records |

Evidence from the release binary:

- At guest `0x080b983c`, the routine corresponding to
  [`CheckRelicanthWailord`](https://github.com/PokemonHnS-Development/pokemonHnS/blob/4aa8fa894db4c890b3d1100abd6aab50f44c22b1/src/braille_puzzles.c#L93)
  checks Wailord (314) first and Relicanth (381) last, using the party and
  count literals at ROM offsets `0xb9880`/`0xb9884` and stride 100.
- Its call to guest `0x0819060c` matches
  [`CalculatePlayerPartyCount`](https://github.com/PokemonHnS-Development/pokemonHnS/blob/4aa8fa894db4c890b3d1100abd6aab50f44c22b1/src/pokemon.c#L8102):
  zero the count, scan species, increment until empty or full. The count and
  party literals are at ROM offsets `0x190644`/`0x190648`, and the stride
  instruction is at guest `0x08190610`.

Occurrence counts of a literal alone are not evidence of a root. To
reproduce with a user-supplied, hash-checked raw ROM:

```sh
sha1sum "$ROM"
arm-none-eabi-objdump -D -b binary -m arm -M force-thumb \
  --adjust-vma=0x08000000 --start-address=0x080b983c --stop-address=0x080b9888 "$ROM"
arm-none-eabi-objdump -D -b binary -m arm -M force-thumb \
  --adjust-vma=0x08000000 --start-address=0x0819060c --stop-address=0x0819066c "$ROM"
```

### ROM tables

| Table                                  | Address                                    | Count x stride          |
| -------------------------------------- | ------------------------------------------ | ----------------------- |
| Species names                          | `0x08744798`                               | 462 x 11                |
| Species info                           | `0x0893bd40`                               | 490 x 40                |
| Move names                             | `0x08745b74`                               | 368 x 13                |
| Moves (`gBattleMoves`)                 | `0x08937998`                               | 368 x 12                |
| Ability names                          | `0x086e1e90`                               | 82 x 13                 |
| Type names                             | `0x086e152c`                               | 19 x 7                  |
| Nature name pointers                   | `0x08965f48`                               | 25 x 4                  |
| Items (`gItems`)                       | `0x088fe910`                               | 398 x 44                |
| Experience                             | `0x0893b0a0`                               | 6 x 404                 |
| `gAbilityDescriptionPointers`          | `0x086e22bc`                               | 82 x 4, by ability ID   |
| `gMoveDescriptionPointers`             | `0x089658c8`                               | 367 x 4, by move ID - 1 |
| Icon table / index / palettes          | `0x08952b3c` / `0x089532e4` / `0x089534d0` |                         |
| Front pictures                         | `0x08732678`                               |                         |
| Normal / shiny / modern-shiny palettes | `0x0872a28c` / `0x0872b1dc` / `0x0872c12c` |                         |

Move priority is the signed byte at `gBattleMoves + 7` (QUICK ATTACK is
`+1`). Item descriptions come through the pointer at `gItems + 20`; the
MASTER BALL pointer decodes to the three source lines ending "without fail."

The description pointer tables were located by unique encoded-string
anchors. Every entry's terminated bytes match the pinned source's
[abilities](https://github.com/PokemonHnS-Development/pokemonHnS/blob/4aa8fa894db4c890b3d1100abd6aab50f44c22b1/src/data/text/abilities.h)
and [move descriptions](https://github.com/PokemonHnS-Development/pokemonHnS/blob/4aa8fa894db4c890b3d1100abd6aab50f44c22b1/src/data/text/move_descriptions.h)
under the game's charmap (apostrophe `0xb4`, newline `0xfe`, EOS `0xff`): all
82 ability and 367 move entries. Move 0 has no description. The longest
entries including EOS are 28 and 60 bytes; the profile bounds reads at 32 and
64 bytes, and item descriptions at 256. A description with an unknown
control code or no terminator is unavailable, never partially decoded.

### Save and storage roots

| Root                 | Address      |
| -------------------- | ------------ |
| `gSaveBlock1Ptr`     | `0x03003740` |
| `gSaveBlock2Ptr`     | `0x03003744` |
| `gPokemonStoragePtr` | `0x03003748` |

`SaveBlock1` is `0x3db8` bytes; the relevant settings begin at `+0x3d9d`.
`PokemonStorage` is `0x8d34` bytes: current box at `+0`, records at `+4` (15
boxes of 30 records, record stride `0x50`, box stride `0x960`), box names at
`+0x8ca4` with stride 9, and wallpapers at `+0x8d2b`. Source comments that
place boxes, names and wallpapers at `+1`, `+0x8344` and `+0x83c2` are stale
and do not match the release.

### Battle and map roots

Release-binary literals and captured structures verify these roots:

| Root                   | Address      | Notes                                                                                                                     |
| ---------------------- | ------------ | ------------------------------------------------------------------------------------------------------------------------- |
| `gMain`                | `0x03001574` | packed `inBattle` is byte `+0x439` bit 1, at `0x030019ad`                                                                 |
| `gBattleTypeFlags`     | `0x020003a0` |                                                                                                                           |
| `gBattlersCount`       | `0x02001420` |                                                                                                                           |
| `gBattlerPartyIndexes` | `0x02001424` | four u16 values; code indexes it from a literal of this address and multiplies the result by the `0x64` party record size |
| `gBattleMons`          | `0x0200143c` | four `0x58`-byte records                                                                                                  |
| `gAbsentBattlerFlags`  | `0x020015c8` |                                                                                                                           |
| `gEnemyParty`          | `0x0201de9c` | six `0x64`-byte records                                                                                                   |
| `gMapHeader`           | `0x0200c220` | section and weather bytes at `+0x14`/`+0x16`                                                                              |

`inBattle` is 1 in the battle fixture and 0 in the overworld fixture. In the
battle fixture, the personality and trainer IDs of the active battle records
link exactly to player and enemy party slot 0. The overworld fixture shows
that battle records are stale outside battle, so the bridge reads them only
while the `inBattle` bit is set. The captured `gSaveBlock1Ptr` record gives
position (22, 13) and location (group 0, map 11), matching the pinned map
metadata for Route 29: section `0x58`, configured weather 0 (None).

Before publishing, the battle bridge validates counts, occupied records,
checksums, party indexes, battler identity linkage, species and move bounds,
HP and level, and stage range.

### Unresolved values

- HnS's party randomizers for types and abilities and its base-stat
  equalizer are detected but not resolved: those party and storage fields
  are `null` with warnings. Active battlers publish their live type and
  ability fields instead.
- Battle turn count and live battle weather have no verified release
  symbols; they are `null` with warnings. Map-configured weather is
  available separately through the `context` view, labelled as map weather.
- Trainer identity in battle is unavailable; the `context` view publishes the
  battle kind and opponent category.
- Spinda's spot-rendered and Deoxys's tile-processed front portraits are
  unsupported.

## Elite Redux 2.65 Beta2

### Build identity

| Property            | Value                                                                                                                                                                                                     |
| ------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Source              | [`06da575972cfed20d471f8bb5b90ea9e4736b16a`](https://github.com/Elite-Redux/eliteredux-source/tree/06da575972cfed20d471f8bb5b90ea9e4736b16a), config submodule `3073176bb3a5abf7a1fe52c23e61bcdf580e4685` |
| Release             | v2.65 Beta2, game version 1048                                                                                                                                                                            |
| Release ZIP SHA-256 | `8c8ac8c6587abc85d1f7472df3adcf847f4d5eef99797a79ab6a14b614a7a432`                                                                                                                                        |
| Raw ROM size        | 33,554,432 bytes                                                                                                                                                                                          |
| Raw ROM SHA-1       | `f1323126bc043f3ce931bcc670957930559eb515`                                                                                                                                                                |

### Layout family and build manifests

[`elite-redux.ts`](../src/elite-redux.ts) defines `eliteReduxFamily`, which
meets the Gen III views (`party`, `storage`, `battle`, `context`, `media`) with
its own sources and adds `trainers`. Each build is a manifest
(`ManifestOf<typeof eliteReduxFamily>`): exact raw-ROM hash, source
provenance, an address for each RAM source, and catalog, trainer and artwork
roots and counts. `createGame(eliteReduxFamily, manifests)` registers them, and
resource preparation selects the same hash as live decoding. The player
locates nearby revisions with the family's locators; to review one, write its
manifest with `tools/bridge.mjs locate --manifest`, verify it and append it.
Do not duplicate decoders or React components. A changed struct layout needs
its own sources or views, not relaxed hash matching; a ROM record whose size
differs between builds, such as the ability record, is a manifest value
(`abilityStride`, `abilityFlagsOffset`). Beta1 has not been checked;
[Beta3](#elite-redux-265-beta3) has its own manifest.

### Debug build

| Property        | Value                                                                                             |
| --------------- | ------------------------------------------------------------------------------------------------- |
| Build           | v2.65 Beta2 (1048) debug build                                                                    |
| Raw ROM SHA-1   | `299165fb56e19ac8afe5d66f83dd5bada823d58c`                                                        |
| Raw ROM SHA-256 | `4f5bdd1dbcc013d876120b4a09c7e9116513d2a80f28a49591c44eb0e5bf0346`                                |
| Manifest        | Located at runtime from the release locators; relocation from the release agrees on every address |

The debug build uses about 46 KB more ROM than the release. Its data tables
sit at five offsets from the release's (+0x5330, +0x55dc, +0xb50c, +0xb534 and
+0xb5d0) and match them word for word, pointers compared by target. In RAM,
`playerCount`, `playerParty` and `enemyParty` move by +0x14 and the other
sources stay put; each move is backed by the literal pools that load it.
`battleActive` and `neutralizingGas` have no literal of their own and follow
their neighbours, and `enigmaBerries` has 14 of 17 pools in agreement. The
build passes the family's probes and a cold-boot smoke run; no save state of
it has been checked, so live RAM decoding is unverified.

### Locators

[`elite-redux.locators.ts`](../src/elite-redux.locators.ts) and
[`heart-n-soul.locators.ts`](../src/heart-n-soul.locators.ts) are generated
from the verified release ROMs with `tools/bridge.mjs signatures`. They cover
every RAM source and every ROM address in the manifests; tables of name,
description and icon pointers are also reached through their targets.
Fields that no literal names directly, such as `battleActive` in both games
and Redux's `neutralizingGas`, are found through the code that computes them
(`thumbAccess`). Every count has value locators except Heart & Soul's
species-table size (490), which no code uses; Heart & Soul's
`moveDescriptions` count comes from a bounds check against the last ID and is
inferred.

Each family names its reviewed build as `locatedFrom`, the template for
builds located at runtime. On the Redux debug build, the release locators
find every RAM source, ROM table and count without the release ROM, and the
1.2.1 Heart & Soul locators do the same for 1.2.0, leaving only the inferred
`moveDescriptions` count to review. Relocation from the reviewed builds
agrees on every address; the private `located-builds` test checks both. `tools/bridge.mjs detect` scores both Redux builds as Elite
Redux and the Heart & Soul release as Heart & Soul, although all three carry
Emerald's game code.

The player connects these outputs to the same `PokemonDashboard`, roster,
inspector, move list, artwork hook and tooltips used by Heart & Soul; only
the connector knows the game definition.
[`redux-record.ts`](../src/redux-record.ts) owns packed RAM decoding and
[`redux-catalog.ts`](../src/redux-catalog.ts) adapts ROM tables to the shared
metadata.

`eliteReduxLinks` builds Nextdex links: Beta2 uses
`v=2.65beta` and the record's internal `species` ID, not its selection `id`
or National Dex number. Eggs return `null`.

### Record layout

Redux records are unencrypted and have no Gen III checksum. Box, party and
battle records are `0x34`, `0x4c` and `0x68` bytes, so the stock Emerald and
Heart & Soul codecs do not apply.

Struct offsets were corroborated by ARM GCC 16.2 target probes
(`-mthumb -mthumb-interwork -mabi=apcs-gnu -march=armv4t -fshort-enums`)
over enum headers generated from the pinned config. This is not a matched
rebuild, and release disassembly overrides compiler-sensitive packing:
ability metadata is at `+0xb8` (not `+0xb6`), and the neutralizing-gas
timer is at `+0x11`.

### RAM roots

| Root                              | Address                                    |
| --------------------------------- | ------------------------------------------ |
| Player party count / party        | `0x02039905` / `0x02039908`                |
| Enemy party                       | `0x02039ad0`                               |
| `gMain` / `inBattle`              | `0x03003420` / `0x03003859` bit 1          |
| Battle type flags                 | `0x0201c3f4`                               |
| Battler count / indexes / records | `0x0201c484` / `0x0201c488` / `0x0201c50c` |
| Absent-battler mask               | `0x0201c6db`                               |
| `SaveBlock1` pointer              | `0x0300340c`                               |
| `SaveBlock2` / storage pointers   | `0x03003410` / `0x03003414`                |

`gEnemyPartyCount` is zero during trainer battles, so occupied enemy slots
are determined from their packed species fields. Storage is 26 boxes of 30
slots, with `0x34`-byte records and names at `+0x9e74`.

### ROM tables

ROM header pointers corroborate these roots; the remaining roots and all
table bounds are in the typed manifest.

| Table                         | Address      | Stride                                                               |
| ----------------------------- | ------------ | -------------------------------------------------------------------- |
| Species names (pointer table) | `0x089c16c8` |                                                                      |
| Species data                  | `0x08bf5b0c` | 44                                                                   |
| Move names                    | `0x089c7a24` | 13                                                                   |
| Moves                         | `0x08bebfac` | 24                                                                   |
| Items                         | `0x08ab9674` | 40                                                                   |
| Abilities                     | `0x08fbc4d8` | 192; name, short and expanded description pointers at `+0`/`+4`/`+8` |

Text reads are bounded, and unrecognized control codes produce warnings
rather than invented prose.

### Innates, randomizers and suppression

| Setting                              | Location                         | Verified by                                                                                   |
| ------------------------------------ | -------------------------------- | --------------------------------------------------------------------------------------------- |
| Difficulty                           | `SaveBlock2 +0xc89`, high nibble | `CanDisableInnates` (`0x081b085c`), `getInnateDisableLevel` (`0x081b0878`)                    |
| Innate, ability and type randomizers | `SaveBlock2 +0xc8a` bits 3, 5, 7 | `RandomizeInnate`/`RandomizeAbility`/`RandomizeType` (`0x081ae08c`/`0x081ae100`/`0x081ae174`) |
| EV toggle                            | `SaveBlock2 +0xc8c` bit 2        |                                                                                               |
| Third badge                          | `SaveBlock1 +0x1ae1` bit 3       |                                                                                               |

Player innate slots unlock at levels 0/0/0 on Easy and Ace, 1/17/24 on Elite
and 10/17/24 on Hell; enemies bypass those level locks. The battle reader
uses the four actual slots plus up to three extra Hell abilities.

Persistent suppression follows `IsSuppressed` (`0x080da638`) and
`DoesBattlerHaveAbilityShield` (`0x080d2c18`):

- Gastro Acid is `statuses3` bit 16 at `0x0201c7ac`; neutralizing gas is
  `0x0201cb69` bit 0;
- the unsuppressable and blocks-suppression flags are bits 11 and 13 of the
  ability metadata word at `+0xb8`;
- ability-based shielding checks the four normal slots; item-based shielding
  uses hold effect 62 and is blocked by Embargo (`statuses3` bit 17); Enigma
  Berry's runtime hold effect is read separately.

Contextual effects such as Mold Breaker, conditional activation and damage
modifiers are not presented as a global enabled flag.

### Stage-adjusted stats

Battle records keep the pre-stage `stats.actual` and publish a separate
`stageAdjustedStats`, used by the inspector's Total column in battle. Both
games use the standard stage ratios: multiply by `(2 + stage) / 2` for
nonnegative stages or `2 / (2 - stage)` for negative stages, then floor; HP
is unchanged. Redux's pinned
[ratio table](https://github.com/Elite-Redux/eliteredux-source/blob/06da575972cfed20d471f8bb5b90ea9e4736b16a/src/pokemon.c#L172-L186)
and [stat calculation](https://github.com/Elite-Redux/eliteredux-source/blob/06da575972cfed20d471f8bb5b90ea9e4736b16a/src/battle_util.c#L7309-L7315)
keep raw fields and stage multiplication separate. The totals exclude
contextual ability, item, weather and move modifiers and the critical-hit and
Unaware stage bypasses, so they are not effective damage stats.

### Trainer previews

`PokemonTrainerSearch` returns `PokemonTrainerPreview[]`: a stable encounter
ID, source and internal IDs, name, bridge-authored variant, class, typed
portrait, average level, warnings and an ordinary `PokemonPartyMember[]`. The
shared inspector has no difficulty, scaling or EV rules. Generated
personality, trainer ID, gender, friendship and experience stay unavailable;
starting HP is max HP, and known IVs, applied EVs and calculated stats use
the ordinary fields.

Redux's `reduxTrainerContext` reads, from one captured observation, the
difficulty, EV enablement, the highest level among nonempty non-egg party
slots (fainted Pokémon included) and the third badge. The `trainers` view binds the ROM resource
`searchTrainers(query, context)` to it, which resolves the selected difficulty's
party, signed level offsets, Hell's +5 after the third badge with a level cap
of 100, Easy or disabled EVs, nature, Iron Pill's zero Speed IV,
Shedinja-family HP, PP Max and Hell extra abilities. It uses the ROM's own
fallback Elite and Hell party pointers. Context is per session, search is
invalidated only when a relevant value changes, and the resolver never writes
or executes game memory.

| Table                | Address                     | Count x stride                                         |
| -------------------- | --------------------------- | ------------------------------------------------------ |
| Trainers             | `0x089a2ffc`                | 932 x `0x38`                                           |
| Trainer Pokémon      | per trainer                 | stride `0x1c` (two trailing padding bytes, not `0x1a`) |
| Class names          | `0x089be5b4`                | 73 x 13                                                |
| Portraits / palettes | `0x08963d64` / `0x08964074` | 98 x 8 each                                            |
| Hell skills          | `0x0850da44`                | 932 x 24                                               |

Results group alphabetically by name, then follow build-authored encounter
order, with internal ID as the final tie-breaker. The pinned source's
`data/maps/*/scripts.pory` defines that order:

- Roxanne's initial and four rematch records carry explicit singles, doubles
  and legends labels.
- May and Brendan follow Route 103, Rustboro, Route 110, Meteor Falls
  (partner), Route 119, then Lilycove and the Route 103 rematch, with the
  player's starter choice shown for every variant. May IDs 63/70/87 and
  Brendan IDs 104/113/122 are Meteor Falls partner teams despite their low
  IDs. Route 103 rematches reuse Lilycove's records, and Route 104 can use
  the Rustboro records.
- Duplicate variants without verified routing keep conservative
  encounter-number labels, in ID order after the verified variants. The
  shared UI does not infer or re-sort chronology.

Sorting by ID or by scaled level would not give encounter progression. This
is an encounter catalog, not an eligibility list: it does not claim a
rematch is unlocked. The procedural OLDPLAYER party (ID 2000) and special
facility and secret-base parties are outside it, and Hell's contextual
battle rules are noted rather than folded into starting totals.

Search appears only during a ready, inactive battle state. Live battles and
unavailable or failed observations take precedence, and a timeline
replacement clears the preview selection.

### Unresolved values

- Randomized party types and abilities are `null` with a warning; active
  battlers use live values.
- Front artwork with a short normal or shiny palette is available only when
  its pixels use colors present in that palette. If a pixel needs an omitted
  color, the artwork stays unavailable: the game copies stale
  decompression-buffer contents for omitted entries, not zeroes or additional
  colors from the ROM.
- Rare and legendary shiny artwork is unavailable rather than drawn with a
  wrong palette.
- Turn, live weather, trainer name and map name are not verified.

## Elite Redux 2.65 Beta3

### Build identity

| Property      | Value                                                                                                                                                                                                                                                      |
| ------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Release       | v2.65 Beta3 (2.65.3b); the ROM's own version strings read `v2.65 Beta2.1`                                                                                                                                                                                  |
| Source        | An unpublished revision of the `upcoming` branch that includes [`156dd545`](https://github.com/Elite-Redux/eliteredux-source/commit/156dd545602c457b154f3c6a5adaf0b44adf5bf8) (`Ability.onModifyTargetFlag`) and still uses the separate Hell-skills table |
| Raw ROM size  | 33,554,432 bytes                                                                                                                                                                                                                                           |
| Raw ROM SHA-1 | `5b38a681b6924ef3e0bb9f6cfbf22426c4c4e523`                                                                                                                                                                                                                 |
| Manifest      | `eliteReduxBeta3`, reviewed                                                                                                                                                                                                                                |

### Differences from Beta2

RAM is unchanged: every source sits at Beta2's address. The ROM tables move:

| Table                                        | Address                                    | Count x stride                    |
| -------------------------------------------- | ------------------------------------------ | --------------------------------- |
| Species names / data                         | `0x089c1614` / `0x08bf5b84`                | 2902                              |
| Move names / moves / descriptions            | `0x089c7970` / `0x08bec024` / `0x08ec12b8` | 1032                              |
| Abilities                                    | `0x08fc6de4`                               | 1034 x 196                        |
| Items                                        | `0x08ab9640`                               | 1023 x 40                         |
| Types / natures / experience                 | `0x08517af4` / `0x08ec2398` / `0x08bf4ee4` |                                   |
| Trainers / classes / Hell skills             | `0x089a2f48` / `0x089be500` / `0x0850e4c4` | 932 x `0x38` / 73 x 13 / 932 x 24 |
| Icons / icon palette indices / icon palettes | `0x08e1fb28` / `0x08e255d8` / `0x08e263d0` | 2902 / 2902 / 7                   |
| Front pictures / palettes / shiny palettes   | `0x08955b38` / `0x0857d07c` / `0x08582b2c` |                                   |
| Trainer pictures / palettes                  | `0x08963bf0` / `0x08963f00`                | 98                                |

The ability record grows from 192 to 196 bytes: `Ability` gains the
`onModifyTargetFlag` handler pointer ahead of its bitfields, so the
suppression flags word moves from `+0xb8` to `+0xbc`. Name pointers repeat
every 196 bytes across all 1034 records. At `+0xbc`, bit 11 is set on exactly
the abilities the source marks `.unsuppressable`, apart from display names
(both As One riders read "As One", Dual Shadow reads "Two-Faced"); read at
`+0xb8` or `+0xc0`, no ability matches.

The other records the family reads keep their layout. `BattleMove.target`
becomes `MoveTarget`, declared `: u16`; the `BattlePokemon` status fields
become enums declared `: u32`; `SaveBlock2` takes its boss-rush bit from
filler; renumbered event flags start at 3099, above the badges. Beta2's
encounter labels still apply: each labelled trainer ID names the same
trainer, and no map script that battles Roxanne, May or Brendan changes
between Beta2's revision and the battle-skill rework, which Beta3 predates.

Values the Beta2 locators do not find, and their evidence:

- `playerParty` and `enemyParty`, `0x02039908` and `0x02039ad0`: 639 and 227
  literal-pool words name them, and the party decodes from a Beta3 state.
- Both item counts, 1023: item 1022 (Waterium Z) is the last named record,
  and the item icon table keeps its extra final entry.
- `classCount`, 73: the two immediate locators read 73 and the literal
  locator reads 0; class 72 (Skier) is the last name.
- `iconTable`, `0x08e1fb28`: the Game Freak ROM header's icon pointer
  (`0x08000138`) names it, four literal pools load it, it starts 2902 x 8
  bytes before `iconPaletteIndices`, the same spacing as in Beta2, and its
  entries point to 0x400-byte icon sheets. Beta2's address lies inside this
  table, so the artwork probe alone accepts it.

The header's other front-picture, palette, icon, species-name and move-name
pointers (`0x08000128`-`0x08000148`) also equal the manifest's.

### Locators on Beta3

`tools/bridge.mjs detect` scores Beta3 0.50 as Elite Redux, below the 0.75
the player requires: the first locators of `playerParty`, `enemyParty` and
`settings` miss. Past detection, the Beta2 locators leave the six values
above unresolved and the probes reject the ability records, so the player
cannot locate Beta3 from Beta2. Every value they do resolve equals the
reviewed manifest's. Some Beta2 code patterns begin with the end of the
preceding literal pool, a ROM address that moves with every build:
`enemyParty`'s first pattern starts with Beta2's trainer-table pointer.

## pokeemerald-expansion

Upstream
[`4c680433909c7fb2219cc9e755f05cdd081d4778`](https://github.com/rh-hideout/pokeemerald-expansion/tree/4c680433909c7fb2219cc9e755f05cdd081d4778)
(1.17.1 metadata) is not a universal binary ABI. Its Game Freak header and
append-only `RHHEXP` header expose ROM table pointers, counts, a version and
selected save offsets. They do **not** provide all live RAM globals or a
versioned struct layout. Configuration changes layouts, parties use
`gParties`/`gPartiesCount`, and save blocks relocate. Hacks built on it
therefore cannot be enabled from their title or header alone.

## Test fixtures

Tests that need real games skip unless these variables point at
user-supplied files, and they check each file's hash before use: SHA-1 for
ROMs, SHA-256 for states. None of these files is committed.

| Variable                  | File                                       | Hash                                                               | Decodes to                                                                                                                                                                                                                                                        |
| ------------------------- | ------------------------------------------ | ------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ZIGBA_HNS_ROM`           | Heart & Soul 1.2.1 raw ROM                 | `a4be81f8330d7f223a3ab847c3da822fe6f8a2e0`                         |                                                                                                                                                                                                                                                                   |
| `ZIGBA_HNS_120_ROM`       | Heart & Soul 1.2.0 raw ROM                 | `b8a4b491ca67a76f079a8e7c19fd5512a2147dd2`                         | Located at runtime; compared with relocation from `ZIGBA_HNS_ROM`                                                                                                                                                                                                 |
| `ZIGBA_HNS_STATE`         | `game2.state`                              | `71817992f9053560808493c79c3e65b5acba2b4499375cc52d76806b2107fb5f` | Overworld, no battle, 15 named empty PC boxes. Party: TOTODILE (species 158) Lv10 HP 29/33, MODEST, TORRENT, SCRATCH 32/35, LEER 30/30, WATER GUN 25/25; PIDGEY (16) Lv3 HP 16/16 with TACKLE and KEEN EYE; HOPPIP (187) Lv2 HP 14/14 with SPLASH and CHLOROPHYLL |
| `ZIGBA_HNS_BATTLE`        | `battle.state`                             | `5515f43a8119632270b2f64a447917fd826bc7cf16655f3276bbb0f3c4d42e54` | Single wild battle on Route 29: TOTODILE Lv10 HP 26/33 against HOPPIP Lv3 HP 16/16, neutral stages, stats 33/18/20/16/17/17 and 16/8/8/7/8/9; SCRATCH 30/35, LEER 30/30, WATER GUN 25/25 and SPLASH 40/40                                                         |
| `ZIGBA_REDUX_ROM`         | Elite Redux 2.65 Beta2 raw ROM             | `f1323126bc043f3ce931bcc670957930559eb515`                         |                                                                                                                                                                                                                                                                   |
| `ZIGBA_REDUX_DEBUG_ROM`   | Elite Redux 2.65 Beta2 debug build raw ROM | `299165fb56e19ac8afe5d66f83dd5bada823d58c`                         | Located at runtime; compared with relocation from `ZIGBA_REDUX_ROM`                                                                                                                                                                                               |
| `ZIGBA_REDUX_STATE`       | `redux.state`                              | `bf403592558c7a6cdc0deb493f8ab26f8796dfec829f0a2031fcbbc44af66d23` | Trainer battle on Easy with randomizers off: Frigibax Lv20 62/62 against Mudkip Lv20 58/58; rosters Frigibax/Klink/Aron/Goomy and Mudkip/Goomy; Frigibax has Mountaineer, Thermal Exchange, Heatproof and Freezing Point from live battle slots                   |
| `ZIGBA_REDUX_STAGE_STATE` | `redux-stage-buff.state`                   | `d092f7bd0f323b1720321f16b72291753748c03fba63454fe8d19455ec7b472d` | Mudkip at Attack/Defense +1/+1: raw 41/31, totals 61/46; max HP 58 and Speed 29 unchanged                                                                                                                                                                         |
| `ZIGBA_REDUX_BETA3_ROM`   | Elite Redux 2.65 Beta3 raw ROM             | `5b38a681b6924ef3e0bb9f6cfbf22426c4c4e523`                         | Probes pass; the Beta2 locators give no value that differs from the manifest                                                                                                                                                                                      |
| `ZIGBA_REDUX_BETA3_STATE` | a Beta3 state (raw, not gzipped)           | `74a207b19b09a228cef36f5122d7f229945d8693192eaf1b75796a347445c359` | Overworld on Ace. Party: Zoroark, Gimmighoul, Flygon, Dipplin, Coalossal and Pawmot, all Lv40 at full HP; Zoroark 110/110 with Opportunist and innates Illusion, Exploit Weakness and Ambush. 26 boxes hold 232 Pokémon, the first a Drilbur Lv25                 |

Coverage by suite:

- Unit tests use synthetic encrypted records covering all 24 substructure
  permutations, eggs, fainting, capacity, checksum failure and recovery, and
  the Heart & Soul differences (egg bit 31, species through 455 versus
  reserved and sentinel values). Synthetic mutations cover asymmetric live
  stats and stages, stale identity linkage and invalid stages.
  [`elite-redux.test.ts`](../tests/elite-redux.test.ts) covers packed
  asymmetric fields, slot and lock boundaries and build selection.
- With the fixtures, unit tests decode the two Heart & Soul states and
  `redux.state`, and decode item 1 (MASTER BALL / Master Ball) in each game
  to a 24x24 image with nonempty alpha whose artwork reference carries the
  matched ROM hash.
- The Heart & Soul party test in
  [`console.spec.ts`](../tests/browser/console.spec.ts) runs the main player
  with `game2.state`: names, HP, artwork dimensions, move categories, click
  and keyboard selection, invalidation on reload, and desktop and mobile
  layouts.
- The shared console browser test with the Redux fixtures checks four-ability
  rendering, descriptions on focus and touch, the native-aspect canvas,
  desktop and mobile overflow, ROM artwork, Easy and Ace totals, Hell
  abilities, portraits, signed scaling boundaries, EV enablement and fainted
  or egg selection. `ZIGBA_REDUX_STAGE_STATE` adds a regression that loads
  the buffed state, checks both totals and restores neutral stages.
- The trainer-search browser tests exercise keyboard selection, Escape,
  click and touch dismissal, same-name variants, context refresh and mobile
  viewport bounds. Because `redux.state` is in combat, the real-ROM fixture
  masks the battle-active bit in a captured copy of memory; it does not alter
  the emulator.
- With the Beta3 fixtures, unit tests check the ability stride and flags
  against named abilities and decode the state's party, PC and context, and a
  browser test opens Beta3 as a reviewed build and shows Zoroark's innates.
  A synthetic test reads ability records at the Beta3 stride where a 192-byte
  stride or the Beta2 flags offset reads other bytes.

These checks do not measure observation cost and make no claim of complete
gameplay compatibility.

## Open questions

- Heart & Soul battle turn count and live weather: no verified release
  symbols.
- Elite Redux turn, live weather, trainer name and map name: not verified.
- Ability and move links: part of the `linksFactory` contract, not rendered.
- pokeemerald-expansion hacks: a shared decoder family plus an ELF- or
  symbol-generated manifest bound to the final ROM hash, optionally seeded
  from the public ROM headers, could support many hacks with small manifests.
  No generator exists.
