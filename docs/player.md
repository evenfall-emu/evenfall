# Player

The player is Evenfall's web application: a game library and a GBA player that run entirely in the visitor's browser.
It is built on the [`@evenfall/gba` SDK](sdk.md), like any other page would be, and adds what a person playing games needs: a library, several playthroughs per game, autosave, quick slots, rewind, touch controls, offline use and optional cloud backups.
ROMs stay user-supplied and on the device, and gameplay needs no account or server.
When the [server](server.md) is available, a signed-in player also gets private cloud backups and shareable save-state links, and anyone can send a bug report.

The source is [`web/player/`](../web/player/), the `@evenfall/player` workspace: TypeScript and React, built with Vite, styled with Tailwind CSS 4, with Effect 3 for services and IndexedDB (through `idb`) for storage.
Vite bundles the SDK's Worker, audio worklet and both WebAssembly cores as hashed files under `assets/`.
Game-specific panels come from plugins, such as [`@evenfall/gba-game-plugins`](../web/gba-game-plugins/README.md) ([Game plugins](#game-plugins)).
[`design/web-runtime.md`](design/web-runtime.md) describes the Worker session, pacing, audio and rewind underneath.

The build has two pages:

| Page               | Entry                                                                                                      | Purpose                                         |
| ------------------ | ---------------------------------------------------------------------------------------------------------- | ----------------------------------------------- |
| `/`                | [`index.html`](../web/player/index.html), [`src/main.tsx`](../web/player/src/main.tsx)                     | Home, Library, Player and Manage                |
| `/validation.html` | [`validation.html`](../web/player/validation.html), [`src/validation.js`](../web/player/src/validation.js) | The SDK validation harness of the browser tests |

## Build and run

From the repository root, with Node.js 24.11 or later and the Rust toolchain that `rust-toolchain.toml` pins:

```sh
npm ci
npm run build:wasm -w @evenfall/gba     # the two WebAssembly cores
npm run dev -w @evenfall/player         # http://127.0.0.1:5173, with hot reload
```

The development server proxies `/api` to the [local server](server.md#local-development) on port 8787.
Without it, everything but sign-in, cloud backups, shares, bug reports and the catalog works.
Open exactly `http://127.0.0.1:5173` to sign in, so cookies and OAuth callbacks stay on one origin.

A production build writes the static site into `web/player/dist`, or into `EVENFALL_PLAYER_DIR` when set:

```sh
npm run build -w @evenfall/player
python3 web/player/scripts/serve-web.py --directory web/player/dist   # http://127.0.0.1:8765/
```

Asset URLs are absolute, so serve the build from a site root, over localhost or HTTPS.
[`scripts/serve-web.py`](../web/player/scripts/serve-web.py) is a loopback-only static server that answers the player's page URLs with `index.html`, as the Worker does.
The Cloudflare deployment builds the same way through [`web/cloudflare/build.sh`](../web/cloudflare/build.sh) ([Deployment](server.md#deployment)).

## Pages and navigation

Each page has a URL, handled by TanStack Router, so a refresh or a shared link lands on it again:

| URL                  | Page                                              |
| -------------------- | ------------------------------------------------- |
| `/`                  | Home                                              |
| `/library`           | Library                                           |
| `/library/<game ID>` | Library, with that game's details                 |
| `/manage`            | [Manage](#managing-the-catalog)                   |
| `/manage/<game ID>`  | Manage, with that game open                       |
| `/player`            | Player                                            |
| `/share/state/<ID>`  | Home, starting a [shared state](#sharing-a-state) |

A fresh page load has no running game, so `/player` opens the Library instead.
The browser's Back and Forward buttons move between pages like the tabs.
Other paths are 404s, and servers answer only these URLs with `index.html`.

Home is a pixel-art fresco that scrolls from a coastal sunset down to a seabed, with the product story in the water between ([`src/assets/landing`](../web/player/src/assets/landing/README.md)).
Its main action, **Open player**, becomes **Resume adventure** after a reload, for the last played local game's selected playthrough.
With no game installed, it can offer a cloud-only playthrough instead.
Its footer carries the trademark notice and, when [configured](#analytics), a privacy-notice link.

Library, Player and Manage share one header with a tab for each, the logo and **Account**.
The Manage tab appears only for accounts that may manage the catalog.
Playback actions live in a separate Player toolbar below the header.
The logo returns Home without unloading the current game, and returning to Player resumes the session only if it was playing before.
On narrow screens the tabs move to a second row, and on phones the Player drops the header and toolbar for its [phone layout](#phones).

**Account** pauses play and persists progress before a Discord sign-in can navigate away.
Sign-in is optional, an unavailable server never blocks local play, and signing in uploads no ROM or library contents.

## Player controls

Open or drag a `.gba` file or a single-ROM ZIP onto the player.
The SDK's importer normalizes the ROM, and its raw SHA-1 identifies the game, its saves and its save lock.
Opening a ROM starts playback from its battery save.
A ROM that no plugin recognizes uses the generic player without game panels.

| Key                | Action                                                         |
| ------------------ | -------------------------------------------------------------- |
| Arrows             | D-pad                                                          |
| X / Z              | A / B                                                          |
| Enter / Backspace  | Start / Select                                                 |
| A / S              | L / R                                                          |
| Space              | Play or pause                                                  |
| Tab (hold)         | Run at 16x while held (Shift+Tab still moves focus)            |
| R                  | Restore the newest checkpoint at least one emulated second old |
| Shift+R            | Open the rewind timeline                                       |
| Shift+1-9, Shift+0 | Save quick slot 1-9, 10                                        |
| Ctrl+1-9, Ctrl+0   | Load quick slot 1-9, 10                                        |

Standard gamepads and touch pointers work too.
Game keys work without canvas focus, but fields, open dialogs and widgets that handle a key keep their normal behavior.
Moving into a field or losing window focus releases the buttons, and hiding the page pauses.
**How to play**, from the toolbar's gamepad button or the screen's `?`, shows the keys and toggles the touch controls.

- **Toolbar.** [Report a bug](#reporting-a-bug), [Share state](#sharing-a-state), **Back up now** (with cloud backups on), fullscreen, sound and speed.
  Volume is stored on the device, and mute does not reset it.
  Speed changes wall-clock pacing, not guest cycles, and the audio follows with a matching pitch.
- **Reset** restarts emulation from the current playthrough's battery save and clears rewind history.
  It creates no playthrough and erases no progress.
- **Rewind.** Each tap of R skips the checkpoints newer than one emulated second, so repeated taps keep moving back.
  Shift+R opens the timeline, which previews any checkpoint with exact pixels.
  Cancel keeps the live state, and commit restores the checkpoint and persists it.
  History resets on load, reset or a rewind reconfiguration ([`design/rewind.md`](design/rewind.md)), and is [stored](#rewind-history) with the resume checkpoint, so it survives a reload.
- **Quick slots** belong to one playthrough, persist across reloads, never replace battery saves, and [sync](#quick-slot-sync) when cloud backups are on.
  The More menu shows each slot's save time and sync state.
- **Imports.** Dropping a `.state` file, or **More → Import state**, restores an Evenfall save state and stays paused for inspection.
  Importing a battery `.sav` reboots the ROM with that save and starts play.

### Narrower screens

A game layout's side columns take about a quarter of the page each, between 300 and 460 px.
On a landscape page narrower than 17:10 that is not a phone, such as a 3:2 laptop, two side columns would squeeze the screen.
There the panels move into one column of tabs beside the screen ([`src/docked.tsx`](../web/player/src/docked.tsx)).
The game chooses which tab opens, such as the battle while one runs, and a tab picked by hand stays open until the game's choice changes.

### Phones

A portrait page up to 650 px wide, or a landscape page up to 500 px tall, gets the phone layout ([`src/handheld.tsx`](../web/player/src/handheld.tsx)).
Portrait stacks a bar, the screen at full width, the game's one-line HUD and the on-screen controls.
Landscape gives the screen the full height and puts the controls beside it.
A game layout's panels open in a sheet below the screen, or a drawer beside it in landscape, so the game stays visible.

On-screen controls start shown on touch screens unless a gamepad is connected.
The D-pad follows its finger even off the pad, giving each direction 60° and each diagonal 30°, and a finger slides between buttons ([`src/touch-model.ts`](../web/player/src/touch-model.ts)).
Every finger is tracked on its own, and presses vibrate where the browser supports it.

## Library

The Library is the game collection: a filterable list beside the selected game's details.
Selecting a game changes the URL to `/library/<game ID>`.
Search, **Show**, **Franchise**, **Sort** and a tag picker combine.
Search ignores case, accents and punctuation, so "poke" finds "Pokémon".
**Show** offers All, Installed and Ready, where the base ROM is on this device and the release's patch is hosted, so the game plays at once.
**Sort** orders by Played (the default), Recently updated or Popularity, which counts cloud playthroughs across accounts.

**Library settings → Show experimental romhacks** is off by default and remembered on the device.
Featured and reviewed releases appear by default, and experimental ones only with this opt-in.
Hidden and patchless releases never appear in the public catalog.
A ROM or cloud playthrough whose release the catalog omits stays available as an unlisted game of its own, so no progress disappears.
A catalog base keeps the catalog's name, and any other ROM takes its No-Intro dump name from [`no-intro-gba.json`](../web/player/src/pages/library/no-intro-gba.json), or else the title it was stored with.

A game's details show the project page, the latest release's notes and each playthrough's release, last play time and cloud state.
**Update…** copies one playthrough's in-game save to a new playthrough on the latest release.
If that release's ROM is not installed, it first uses **Install** when the hosted patch and its base ROM are available.
Otherwise it asks for the release ROM.
Unless the catalog declares the saves compatible, it asks the player to acknowledge the risk first.
Playing a run whose cloud copy changed since this device last synced asks which progress to continue.

**Choose file…** reads a `.gba` or single-game ZIP in an isolated Worker and stores it only when its SHA-1 is one of the game's releases or the base its latest patch needs.
**Install** downloads the latest release's hosted patch and applies it to the base on this device ([`src/patch.ts`](../web/player/src/patch.ts) reads BPS, UPS and IPS).
It stores the result only when its SHA-1 is the release's.
BPS and UPS patches also check their own base, output and patch CRC32s.

The catalog comes from `GET /api/games` and `GET /api/versions` ([`use-catalog.ts`](../web/player/src/pages/library/use-catalog.ts)).
The last response is kept in `localStorage`, so the Library shows it at once and while offline.
[`model.ts`](../web/player/src/pages/library/model.ts) joins it with the device's games, playthroughs and cloud outbox, and the account's cloud checkpoints.

### Managing the catalog

Manage edits the hosted catalog through the server's [admin API](server.md#catalog-api).
Its tab, and **Edit** on a catalog game, appear when `GET /api/me` reports an admin account, and the server still authorizes every request.
The page is its own chunk, [`src/pages/manage`](../web/player/src/pages/manage/manage-page.tsx), which players never download.

The page lists every game, including hidden, patchless and release-less ones, with what each lacks (patch, cover image, release date, source link).
**Details** edits a game and sends only the changed fields.
**Releases** lists a game's releases, and each one's visibility saves as soon as it changes.

**Add release** checks a patch on this device before anything is written.
It finds the base ROM in the Library by SHA-1, applies the patch, and names the release by the SHA-1 of the ROM it builds.
It refuses a patch for another base and an output another release already has.
The release is saved without a patch, Experimental by default, and the patch is uploaded to it.
A release whose upload failed stays invisible to players, and adding it again retries.

## Playthroughs

Each game can have several independent playthroughs.
The Player's **Playthrough** selector creates and switches them, the pencil renames one, and the trash icon deletes one after confirmation.
New playthroughs get UUIDs and can be created offline.

Deleting a playthrough removes its local saves, checkpoints, rewind history, quick slots and pending uploads.
When online and signed in, it also deletes the account's cloud copy, and that deletion must succeed before local progress goes.
Deleting the last local playthrough starts a fresh empty one.

## Local saves and resume

All progress lives in one IndexedDB database on the device, `evenfall-local`, at version 2.
Every key starts with the game's raw-ROM SHA-1, and each save, checkpoint, quick slot, lock and outbox job also names the playthrough.
ROM bytes are stored once per exact hash.

- **Autosave** runs every five seconds and on pause.
  It stores the battery save and a resume checkpoint: a save state with the boot options needed to restore it.
  Unchanged battery bytes are not rewritten.
- **Resume checkpoints** keep the latest and a previous generation per playthrough, each a record of its own with its state gzip-compressed.
  Every autosave replaces the latest generation, and the one it replaces becomes the previous generation once the previous one is a minute old, so the previous generation is 5 to 65 seconds older than the latest.
  That way an autosave writes one compressed state, not two raw ones.
  If the latest is corrupt, resume tries the previous one.
  If neither restores, progress is kept, and autosave stays blocked until a retry succeeds or the player chooses **Start from in-game save**.
- **Quick slots** are stored as `quick-state:["<ROM SHA-1>","<playthrough ID>",<1-10>]` in the `assets` store.
- **Save ownership.** A tab writes a playthrough's progress only while it holds that playthrough's Web Lock.
  Other tabs may resume it but cannot persist.
- **Recognized builds** cache a located manifest per game, ROM and locator version in the `builds` store ([Game plugins](#game-plugins)).

A save state belongs to one exact ROM.

### Rewind history

Each autosave also stores the rewind history that matches its resume checkpoint, so resuming after a reload, an update or a switch of playthrough brings back its checkpoints.
Memory keeps about one checkpoint per second of play, and storage keeps about one per ten seconds: a checkpoint is stored only when it is at least ten captures after the previous stored one, and a stored one stays until the history drops it.
After a reload, rewinding steps back in ten-second steps until new checkpoints arrive.
Writing the whole history every five seconds would rewrite up to its 15 MiB budget each time, so the player stores it in parts ([`sdk.md`](sdk.md#storing-history-as-it-grows)):

- The resume checkpoint's latest generation carries the history's index, a few dozen bytes per checkpoint.
- Each compressed checkpoint and anchor is a record of its own in the `rewind` store, keyed by playthrough and content.
  An autosave writes only the records that are new and deletes those the history dropped, in the resume checkpoint's transaction.

Storing one checkpoint in ten writes about 0.5 to 3 KiB a second of history during play, beside a compressed resume checkpoint of 3 to 15 KiB a second.
Only the latest generation has a history: a resume that falls back to the previous generation, or a history that fails to load, starts without one.
Each playthrough keeps its own history, up to the budget, until it is deleted.

### Write statistics

The player counts what each autosave writes, in memory only.
In the browser console, `await evenfallWrites.report()` prints the totals and rates per part (state, previous generation, battery save, rewind index and payloads), what the earlier layout of two raw states per save would have written, and an estimate of an input log at 10 bytes per button change.
It also counts autosaves that wrote nothing, by reason (`unchanged` while paused, `no game`, `no save lock` when another tab owns the playthrough, `resume blocked`, `busy`), and failed ones with the last error.
`evenfallWrites.samples` holds one entry per autosave, and `evenfallWrites.reset()` starts over.
The counts are the bytes handed to IndexedDB, before the browser's own encoding and storage overhead.

### States from the previous core

Players that ran an earlier core saved states that start with `ZIGBAST1`.
The current core writes `EVENFALL` version 7 states ([`design/save-states.md`](design/save-states.md)) and cannot load the old ones.
Battery saves are raw cartridge memory and carry over unchanged.
The player recognizes an old state by its first eight bytes ([`src/state-format.ts`](../web/player/src/state-format.ts)), says it came from an older version, and never deletes it:

- **A resume checkpoint** starts the playthrough from its battery save.
  The old state moves to a `legacy` record of its own, written once, and stays there until the playthrough is deleted.
  It is never uploaded.
- **A quick slot or state file** is refused with an explanation, and the slot keeps its bytes.
  An old cloud quick slot still downloads and stays like any other.
- **A cloud resume checkpoint** restores the playthrough's cloud battery save instead, and the old state stays in the cloud until an upload replaces it.
- **A shared link** reports that its state came from an older version.

Closing the browser before a transaction completes, or storage eviction, can lose progress.
Export important saves.

## Cloud backups

Cloud backups are on by default when signed in, and **Account → Disable cloud backups** stores an opt-out for that account on this device.
Uploads carry each playthrough's resume checkpoint and battery save, gzip-compressed by the browser.
They never include ROM bytes or a BIOS.
The cloud keeps one current resume, battery save and state per [quick slot](#quick-slot-sync) for each playthrough, not a history.
The server's side is in [`server.md`](server.md#cloud-progress).

### When uploads happen

Uploads reuse the autosave record rather than capturing states of their own.
They run at most every two minutes per playthrough and save kind, and stop after five minutes without game input.
A battery save becomes pending only when its bytes change.
Offline hours coalesce into the latest record, with no backlog.
Signing out or switching accounts never moves pending jobs to another account.

Hiding the tab pauses play, commits local progress and attempts the ordinary upload, as does `pagehide`.
Neither bypasses the interval or the idle cutoff, so use **Back up now** and wait for its confirmation before switching devices.

### Restoring and conflicts

**More → Saves & options → Cloud progress** offers **Back up now**, restoring the cloud resume or in-game save, and an explicit **Replace cloud with local progress**.
Another device must supply the same ROM locally.

On sign-in, returning Home or reconnecting, Home reads the account's backup list without downloading any state.
It offers the last played local playthrough, and the latest cloud playthrough only when nothing is installed.

Loading a playthrough checks its cloud version.
If the cloud changed since this device's last acknowledged version, one dialog offers the cloud checkpoint or the local progress.
An unchanged cloud version never interrupts local play, even when the local checkpoint is newer: timestamps orient the player but never resolve conflicts.
Without a stored ROM, a restore asks for the exact `.gba` or `.zip`, and no ROM bytes reach the server.

An isolated emulator validates the ROM hash and checkpoint before the active player changes.
Account changes, incompatible states and lock failures cannot silently replace a stored checkpoint with a boot state.
Conditional writes stop on conflicting progress rather than choosing the newest clock.

The server admits 100 playthroughs per account, counted on first upload.
A 101st playthrough stays local with a sync message, while existing ones keep updating.
**Account → Delete my account** removes authentication and cloud progress and keeps the device's games and saves.

### Quick slot sync

Quick slots are local-first: saving one commits it to IndexedDB at once, online or not.
With cloud backups on, the same transaction queues the slot's upload and attempts it immediately.
Repeated saves of one slot upload at most every 30 seconds.

Each slot's outbox entry records the last acknowledged revision and cloud ETag, the common base.
Opening a playthrough, signing in, enabling backups, reconnecting and returning to the tab list that playthrough's cloud slots, and decide per slot:

- Only this device changed: upload, conditional on the base.
- Only the cloud changed: replace the local slot silently, after checking its digest.
- Both changed: keep both and ask once in **Quick slots changed**, preselecting the newer save.
  **Decide later** keeps both and shows a reminder in the More menu.

Loading a slot never waits for the network.
A failed conditional upload is recorded as a divergence, never retried as an overwrite.
Only slots 1-10 sync, and a slot last synced for another account is never uploaded.

## Sharing a state

**Share state** pauses the game and uploads its exact current state, which includes the in-game save, as a link anyone can open for at least 24 hours.
Creating a link needs a signed-in account, not cloud backups.
The upload carries the playthrough name, the game's title and the ROM's SHA-1, never the ROM itself.
For an [admin account](server.md#including-the-rom), the dialog also offers **Include the game**, which uploads the stored ROM with the link.

Opening `/share/state/<ID>` needs no account.
The player downloads the state and checks its digest.
It uses the shared ROM if the device has it, builds the release from its base and hosted patch if it can, or downloads a ROM the sharer included.
It validates the state against the ROM in an isolated emulator, then starts it as a new playthrough under the shared name, never replacing an existing one.
It asks first only for a missing ROM, or a new name when a playthrough of the game already has the shared one.

## Reporting a bug

**Report a bug**, the beetle at the start of the toolbar, sends a message to the maintainers with the game as it stands ([`src/bug-report.tsx`](../web/player/src/bug-report.tsx)).
It needs no account, and on phones it sits in the Player menu with the rest of the toolbar.

Opening the dialog pauses the game, so the report describes the moment the reporter saw the problem, and closing it resumes play.
A report holds:

- the reporter's message, of at most 2000 characters,
- with a game open, unless the reporter unticks it, the exact current state (gzip-compressed, as for a share) and the battery save, with the ROM's SHA-1 and the game's title, never the ROM itself,
- for a signed-in reporter who ticks **Include my account**, the account ID, which is otherwise left out along with the session cookie,
- the player build from `VITE_BUILD_COMMIT`, which `build.sh` sets to the deployed commit.

The [server](server.md#bug-reports) stores the files and notifies the maintainers through PostHog, and the dialog says so when the build has a PostHog key.
A failed send keeps the message, so the reporter can try again.
The state already contains the in-game save, and the separate `.sav` is there so that other emulators can load it directly.

## Offline play and updates

The player has a web-app manifest, so it installs as an app (in Android Chrome, **Add to home screen → Install**).
Production builds install a service worker ([`offline.ts`](../web/player/offline.ts)) that caches one complete version of the player, including the hashed SDK Worker, worklet and WebAssembly and the plugins' lazily loaded layouts.
After its first installation, the player reloads offline on any of its page URLs.
API responses, credentials and user media are never cached, and the development server registers no service worker.

The player looks for a new version on load, when it returns to the foreground, when the browser comes back online and every hour ([`src/app-update.ts`](../web/player/src/app-update.ts)).
A new version installs next to the running one.
An idle tab that nobody has touched, or a hidden one, reloads into it at once, at most once a minute.
Otherwise the tab offers **Update now**, which saves the running game's checkpoint and reloads.
A tab still on an older version keeps loading that version's hashed Worker, WebAssembly and chunks from its cache, never an incompatible binary.
Older caches are deleted once a page of the new version is the only open tab.

## Game plugins

Game-specific panels are optional packages registered in [`src/games.ts`](../web/player/src/games.ts).
A plugin (`GamePlugin`) contributes exact-build game definitions from [`@evenfall/gba/bridge`](game-bridges.md) and React dashboards, layouts keyed by game ID.
The player loads every plugin's dashboards before its first render, so a layout never swaps in under a running game.
`resolveGame` selects a layout by raw-ROM SHA-1, never by file name.
A layout receives the player as `children` and reads live game data with `useBridge`.
On phones and narrower screens it receives `handheld` or `docked` instead, and returns its panels for the player to place.
`main.tsx` never imports a game package.

A ROM that matches no reviewed build goes through `gameRegistry.recognize`: the `builds` cache, else each installed game's [locators](game-bridges.md#recognizing-builds-at-runtime), run while the game plays.
A recognized build gets the game's layout with `build.provenance` set to `located`.
A located build whose views keep failing is forgotten, and the generic layout returns.

The default build installs [`@evenfall/gba-game-plugins`](../web/gba-game-plugins/README.md), with the Heart & Soul, Elite Redux and Fire Red Upgrade (Pokémon Unbound) dashboards.
To build without it, remove its import and `installed` entry from `games.ts` and its dependency from `package.json`.
To add a game, export another `GamePlugin` from a workspace package and append it to `installed`.

## Analytics

Analytics are off unless a build sets them: the development server, tests and local builds send nothing.
A configured production build sends anonymous, cookieless Home, Player and Library pageviews and the share events `state_share_created` and `state_share_opened` to PostHog:

| Variable                | Effect                                                                         |
| ----------------------- | ------------------------------------------------------------------------------ |
| `VITE_POSTHOG_KEY`      | The PostHog project key, a public ingestion key.                               |
| `VITE_POSTHOG_HOST`     | The ingestion host, such as `https://eu.i.posthog.com`.                        |
| `VITE_POSTHOG_HOSTNAME` | Sends only from pages served on exactly this hostname, so previews stay quiet. |
| `VITE_PRIVACY_URL`      | Shows a **Privacy notice** link on Home and in the Account dialog.             |

[`web/cloudflare/build.sh`](../web/cloudflare/build.sh) supplies the official instance's values and publishes [`PRIVACY.md`](../web/cloudflare/PRIVACY.md) as `/privacy.txt`.
Export other values, or empty ones, to deploy elsewhere.
Accounts, ROMs, gameplay, clicks, errors and share-link pages are never tracked, and URL queries, fragments and referrers are removed before sending.

## Storage names and the move from an earlier player

[`src/storage-names.ts`](../web/player/src/storage-names.ts) names everything the player keeps in the browser: the IndexedDB database, the volume and update-reload storage keys, the offline cache prefix, and the save, cloud-sync and migration Web Locks.
Earlier players used other names for the same things, listed there as `LEGACY`.
The first load of a current player moves what an earlier one stored ([`src/storage-migration.ts`](../web/player/src/storage-migration.ts)), and nothing is lost on the way:

- **Fencing.** The player first opens the legacy database at version 9, above the version 8 that earlier players open.
  An earlier tab closes its connection when that happens, and from then on its own opens fail, so it can neither write to the legacy database nor recreate it empty.
  Its later saves fail visibly, telling the player to export them.
  An earlier tab that keeps its connection open blocks the move with a message, and nothing moves until it closes.
- **Moving.** Records move store by store, written to `evenfall-local` and then deleted from the legacy database.
  ROMs and other large values move one at a time, so the storage quota never has to hold the library twice.
  An interrupted move, for example by a full quota, leaves each record in exactly one of the two databases and finishes on the next load.
  A Web Lock keeps two current tabs from moving at once.
- **After the move** the legacy database stays, fenced and empty.
  A legacy library from version 6 or earlier, which keyed games by SHA-256, starts empty, as it did before.
- The volume moves once from its legacy key, and the service worker prunes offline caches under both the current and the legacy prefix.
  The update-reload key only limits reloads within one tab session, so it simply changed name.

## Tests

The unit tests run with the rest of the repository's Vitest projects, and the browser suite runs the built player in Chromium:

```sh
vp test --project player
npm run build -w @evenfall/player
npm run test:browser -w @evenfall/player
```

The browser suite serves `web/player/dist` on `EVENFALL_TEST_PORT` (default 8765).
It covers the player and the `/validation.html` harness, whose `window.evenfallValidation` hooks also serve the SDK and save-lock tests.
It uses `PLAYWRIGHT_CHROMIUM_EXECUTABLE` or `/usr/bin/chromium` when present, and otherwise Playwright's own Chromium (`npx playwright install chromium`).
Setting `EVENFALL_TEST_URL` points the suite at a running deployment instead.

Some tests skip unless `EVENFALL_*` variables point at ROMs and states you supply, and they check the fixtures' hashes first.
No user asset is committed.

## Known limits

- Whether cloud states restore on a device with a different external BIOS is untested.
- Battery saves have no compatibility policy between releases of the same game, beyond what the catalog declares.
- States from the previous core stay unloadable: the player keeps them but cannot convert them.
- Rewind history stays on the device: cloud backups and shared states carry none, so a playthrough restored from them starts without history.
- A bug report carries the state after the problem, not the inputs that led to it, because the player records no input log.
