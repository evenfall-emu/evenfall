# Golden Sun bridge profile and evidence

This is the evidence behind the Golden Sun battle dashboard in [`@evenfall/gba-game-plugins`](../README.md): the builds it recognizes, where each address and record field comes from, and what remains unverified.
The generic bridge contract is in [`docs/game-bridges.md`](../../../docs/game-bridges.md).

> Not affiliated with or endorsed by Nintendo or Camelot Software Planning.
> See the [disclaimer](../README.md#disclaimer).

The bridge reads only work RAM.
Names, stats, elements, djinn and statuses are all live in the game's unit records, so the family reads no ROM tables and decodes no text.

## Build identity

The reviewed build is the user-supplied Golden Age 1 ROM hack, a binary patch of Golden Sun (USA, Europe).
Its raw ROM is 16 MiB with SHA-1 `1864e0f1ae44e07f10c2d0ac826e6104c69ca5e9`, and its header still reads `Golden_Sun_AAGSE01`.
The hack patches the retail code in place, so the retail engine's RAM layout holds.
Its rebalanced content shows in the data, such as the enemy "Orc Brute" and renamed djinn, not in the layout.

Golden Sun: The Lost Age (USA, Europe), raw SHA-1 `b500663220cb9bf56b9f8e8c0c544f1d6fa3a824` and header code `AGFE`, has no manifest.
The player locates it ([below](#the-lost-age)).

## RAM roots

Every root was found in the patched binary's code and checked against live states.

| Source       | Address      | Evidence                                                                                                                                  |
| ------------ | ------------ | ----------------------------------------------------------------------------------------------------------------------------------------- |
| `party`      | `0x02000500` | `getUnit(id)` at `0x08077394` returns `0x02000500 + id * 0x14C` for ids 0-7                                                               |
| `enemies`    | `0x03001F28` | the same `getUnit` reads this pointer for ids `0x80`-`0x85` and returns `pointer + (id - 0x80) * 0x14C`; it is null outside battle        |
| `partyFlags` | `0x02000040` | the first byte of the story flags; bits 0-7 are the characters who have joined, `0x0F` for Isaac, Garet, Ivan and Mia in the battle state |
| `partyOrder` | `0x02000438` | one byte per member in party order, `00 01 02 03` in the battle state; one routine loads it by literal and three index it from a base     |
| `coins`      | `0x02000250` | code adds to the word at `0x10` from the `0x02000240` block; it reads 60,612 in the battle state and 60,896 after the battle's reward     |

The enemy block holds six records, from the pointer to the next allocation at `pointer + 0x7C8`.
Unused slots are zero, and defeated enemies keep their record at 0 HP.

The IWRAM words around `0x03001F28` are the engine's table of allocation pointers, at the same offsets as The Lost Age's table at `0x03000000` on the [Golden Sun Data Wiki's RAM map](https://gsdata.fandom.com/wiki/RAM_Map).
The bridge reads only the enemy pointer.

### The Lost Age

The Lost Age compiles the same routines to different code, and two of its roots move by 0x20 with the save block.

| Source       | Address      | Evidence                                                                           |
| ------------ | ------------ | ---------------------------------------------------------------------------------- |
| `party`      | `0x02000520` | `getUnit(id)` at `0x08016CA4`, the same routine as Golden Sun's                    |
| `enemies`    | `0x030000D8` | the same `getUnit`, which builds the address from immediates rather than a literal |
| `partyFlags` | `0x02000040` | `0x20` with Jenna alone in the opening and `0x30` once Felix is in the save        |
| `partyOrder` | `0x02000458` | `05` with Jenna alone                                                              |
| `coins`      | `0x02000250` | 200, as the game's own menu shows                                                  |

The opening's records match the game's status screen: Jenna Lv5 with 65 HP and 43 PP.
The enemy pointer and the battle statuses have not been read in a live Lost Age battle.

## Unit record

Party members and enemies share one 0x14C-byte record.

| Offset          | Field                                                                         | Evidence                                                                |
| --------------- | ----------------------------------------------------------------------------- | ----------------------------------------------------------------------- |
| `0x00`          | name, printable ASCII, NUL-terminated, 15 bytes                               | "Isaac", "Orc Brute1"                                                   |
| `0x0F`          | level                                                                         | 21 for Isaac, as on screen                                              |
| `0x34`-`0x3A`   | max HP, max PP, HP, PP (16-bit)                                               | the battle HUD's values for all four adepts                             |
| `0x3C`-`0x42`   | Attack, Defense, Agility (16-bit), Luck (8-bit), current values               | the djinn menu shows Isaac's Attack as 400 and 316 after a class change |
| `0x48`          | power and resistance per element, 16-bit pairs: Venus, Mercury, Mars, Jupiter | each adept's own element is its highest pair                            |
| `0xF8`, `0x108` | djinn held and djinn Set per element, 32-bit masks, same element order        | unleashing Isaac's Mars djinni clears one bit of the Set mask only      |

Battle statuses follow at `0x130`.
The live states and forced bytes confirmed these:

| Offset          | Status                                   | Observed                                                 |
| --------------- | ---------------------------------------- | -------------------------------------------------------- |
| `0x131`         | 1 poison, 2 venom                        | forcing 1 printed "The poison does 40 damage to Isaac!"  |
| `0x132`/`0x133` | Attack change: turns, then signed stages | Rally set 7 and 2, and raised Attack from 248 to 310     |
| `0x138`         | delusion turns                           | "Orc Brute2 is wrapped in delusion!" set 7               |
| `0x13B`         | stun turns                               | forcing it printed "Isaac is paralyzed and cannot move!" |
| `0x13C`         | sleep turns                              | forcing it printed "Isaac is asleep!"                    |

The rest come from The Lost Age's record on the wiki and its published cheat codes, whose confirmed offsets above match this game's:

- `0x130` curse, `0x134`/`0x135` Defense change, `0x136`/`0x137` Resist change,
- `0x13D` Psynergy seal, `0x13E` regeneration and `0x13F` reflect turns,
- `0x140` haunt and `0x141` the countdown to being downed.

The Attack value at `0x3C` already includes the change, so the dashboard shows it as it is.
A stage is 12.5% of the stat.

## Derived values

- **Turn order** sorts living units by Agility.
  The game adds a random factor each round, so the dashboard labels it an estimate.
- **Weak and resistant elements** compare each resistance with the mean of the unit's four.
  A resistance 15 or more below the mean is a weakness, and 15 or more above is a resistance.
  This is an interface convention, not ROM data.
- **Summons ready** counts the party's djinn held but not Set.
  It matches the in-game summon counter in the tested states.

## Locators

[`golden-sun.locators.ts`](../src/golden-sun.locators.ts) holds two or three code signatures per root and per game, generated with the SDK's `codeSignatures` and `accessSignatures` from Golden Age 1 and The Lost Age.
Each game's signatures find nothing in the other game, so on either ROM only its own locators answer, and they agree.
No code loads the coins' address itself, so its locators follow a base and an offset.
On both ROMs each root is found by every one of its game's locators, none of them inferred.
The family's detector requires the header game code `AGSE` or `AGFE`, and its two markers accept either game's `getUnit`.

## Unresolved values

- The byte at `0x13A` printed "Isaac regains consciousness!" when forced, and it is not shown.
- The byte at `0x142` is 15 for Garet in the battle state, and its meaning is unknown.
- Djinn recovering after a summon are not told apart from those on Standby.
- The round number and the game's actual action order are not read.

## ROM catalogs

The bridge reads none of these yet.
In Golden Age 1 they would show an enemy's abilities with their names, descriptions, PP cost, power and element:

- `getEnemy(id)` at `0x080773D8` returns `0x08080EC8 + (id - 8) * 0x54`, where `id` is the 16-bit value at `0x128` in the enemy's RAM record (`0x54` for Orc Brute).
  The entry holds the base stats, the items at `0x28`, and eight 16-bit ability IDs at `0x38`.
  Orc Brute's are Slice twice, Rally, War Cry and Attack four times, the moves it used in battle.
- `getAbility(id)` at `0x08078B9C` masks `id` with `0x3FFF`, treats 520 or more as 0, and returns `0x0807EE58 + id * 16`.
  Byte 2 is the element, 4 for none, and byte 9 is the PP cost: Katana Strike costs 15, Shuriken 8 and Punji 7, as the battle menu shows.
  Bytes 10-11 are the power, and bytes 0, 1, 3 and 8 are the target, the damage type, the effect and the range, whose values are not mapped.
- The hack stores its text uncompressed.
  Its ARM text routine reads 32-bit offsets at `0x08800000` into strings at `0x0880A7F0`.
  Ability names are text `819 + id` and descriptions `1338 + id`, so Rally's description reads "Boost an ally's Attack."
  Enemy-only abilities have the description "?".

Retail Golden Sun and The Lost Age compress their text, so their names and descriptions need a decoder of the game's compressed text.

## Test fixtures

Tests that need the game skip unless these variables point at user-supplied files, and they check each file's hash before use.
None of these files is committed.

| Variable                       | File                                                              | Hash (SHA-1 ROM, SHA-256 states)                                   | Decodes to                                                                                |
| ------------------------------ | ----------------------------------------------------------------- | ------------------------------------------------------------------ | ----------------------------------------------------------------------------------------- |
| `EVENFALL_GOLDEN_SUN_ROM`      | Golden Age 1 raw ROM                                              | `1864e0f1ae44e07f10c2d0ac826e6104c69ca5e9`                         | Every root located                                                                        |
| `EVENFALL_GOLDEN_SUN_BATTLE`   | the shared battle state                                           | `d1206cddf38d88d72b9f0690ba50d2b3ed70d73d618cc9aad42ee544214670c1` | Isaac Lv21 400/400, Garet, Ivan and Mia against two Orc Brutes Lv21 226/226; coins 60,612 |
| `EVENFALL_GOLDEN_SUN_BUFF`     | the same battle after Orc Brute2's Rally                          | `3dcb61f486a002cf5f1c385363deb73d982f15d754672084087eab59e13923e2` | Orc Brute2 Attack 310, up 2 stages for 7 turns                                            |
| `EVENFALL_GOLDEN_SUN_DELUSION` | the same battle after Isaac unleashes Blanket and Mist Blast hits | `7946eff1ae3c687666c17804d0a41936355af0be5247589a9d0d06d4d1fd835e` | Orc Brute2 deluded for 7 turns; one of Isaac's three Mars djinn on Standby                |
| `EVENFALL_GOLDEN_SUN_FIELD`    | the field after winning the battle                                | `4659517551017f8bb9ceacd58c33364a6b9debc02312aeb0b071462f6320706c` | No battle; coins 60,896                                                                   |
