# ZiGBA player

The browser player for ZiGBA: a TypeScript and React application built with
Vite, Effect 3 and IndexedDB (through `idb`). Games run entirely in the
visitor's browser, and ROMs stay user-supplied and local. Gameplay needs no
account or backend. When the [catalog and account server](../server/README.md)
is available, signed-in players also get private cloud backups of their
progress.

The application has three entry points:

| Page | Entry | Purpose |
|---|---|---|
| `/` | `index.html`, `src/main.tsx` | The player: Home, Player and Library |
| `/debug.html` | `debug.html`, `src/debug-main.tsx` | Dev / Debug: diagnostic player, HUD and Lab |
| `/validation.html` | `validation.html` | SDK validation harness used by browser tests |

The player consumes the [JavaScript SDK](../docs/sdk.md) as the
`@evenfall/gba` workspace package, like any other application; Vite bundles
the SDK's worker, audio worklet and WASM as hashed files under `assets/`.
Pokémon panels come from the separate
[`@evenfall/gba-pokemon`](../games/pokemon/README.md) package through the
[game-plugin registry](#game-plugins). The runtime architecture (worker
session, pacing, audio, rewind) is described in
[`docs/design/web-runtime.md`](../docs/design/web-runtime.md).

## Build and run

From the repository root, with Zig 0.16 and Node 24+:

```sh
npm ci                     # one install for every npm workspace
zig build web
python3 tools/serve-web.py # http://127.0.0.1:8765/
```

`zig build web` builds the ReleaseSafe WASM, runs `npm run build -w frontend`
into `zig-out/player`, and installs the result as `zig-out/web`, the
deployable static directory; the Zig install prefix is respected. Serve it
over localhost or HTTPS. `tools/serve-web.py` is a loopback-only static
server (`--port`, `--directory`).

For frontend development with hot reload:

```sh
zig build web-sdk          # builds web/zigba.wasm for the SDK workspace
npm run dev -w frontend    # http://127.0.0.1:5173
```

Sign-in needs the Wrangler backend on port 8787, with its local migrations
and ignored secrets configured as described in the
[server's local development guide](../server/README.md#local-development).
Open exactly `http://127.0.0.1:5173`: Vite proxies `/api` to Wrangler there,
and the static Python server cannot handle login. Local magic links are
captured by Wrangler's email simulator rather than delivered.

## Pages and navigation

Home is the default destination. It introduces ZiGBA and, after a reload,
offers **Resume adventure** for the last played local game's selected
playthrough, with its name and checkpoint time; if no game is installed, it
can offer a cloud-only run. Its footer carries the trademark and
non-affiliation notice and, when [configured](#analytics), a privacy-notice
link. Opening a ROM from Home switches to Player and starts it.

Home, Player and Library share one navigation header; playback actions live
in a separate Player toolbar below it. Clicking the ZiGBA wordmark returns
Home without unloading the current game. Returning to Player resumes the
session only if it was playing before navigation.

**Account** is available on Home and in the header. Opening it pauses play
and persists progress before Discord can navigate away; closing it resumes a
game that was running. Sign-in is optional, and an unavailable server never
blocks local play. Signing in does not upload ROMs or library contents and
grants no catalog rights.

Dev / Debug lives at `/debug.html`, with its HUD at `/debug.html#/hud` and
the Lab at `/debug.html#/lab`, and is reached through **More → Dev / Debug
player** rather than the main navigation. It shares browser storage with the
player, not an emulator session.

## Player controls

Open or drag a `.gba` file or a single-ROM ZIP onto the player. The native
importer normalizes the ROM; its raw SHA-256 identifies the game and its save
lock. Opening a ROM starts playback from its battery save. Unknown ROMs use
the generic player without game panels.

| Key | Action |
|---|---|
| Arrows | D-pad |
| X / Z | A / B |
| Enter / Backspace | Start / Select |
| A / S | L / R |
| Space | Play or pause |
| Tab (hold) | Run at 16x while held; Shift+Tab still moves focus |
| R | Restore the newest checkpoint at least one emulated second old |
| Shift+R | Open the rewind timeline |
| Shift+1-9, Shift+0 | Save quick slot 1-9, 10 |
| Ctrl+1-9, Ctrl+0 | Load quick slot 1-9, 10 |

Standard gamepads and captured touch pointers are supported. Game keys work
without canvas focus, but typing in fields, open dialogs and widgets that
handle a key keep their normal behavior. Moving into a field or losing window
focus releases buttons; hiding the page pauses. The toolbar's gamepad button
and the screen's `?` open **How to play**, with illustrated keycaps and a
touch-control toggle; help pauses play and resumes on close if it was
running.

- **Toolbar.** **Back up now** (with a game open and cloud backups on),
  fullscreen, sound and playback speed. Hover or focus the sound button for a
  volume slider; volume is stored on the device and mute does not reset it.
  Speed changes wall-clock pacing, not guest cycles, and audio follows the
  rate with a matching pitch change.
- **Reset** restarts emulation from the current playthrough's battery save,
  keeps the play or pause state and clears rewind history. It does not create
  a run or erase saved progress.
- **Rewind.** Repeated R taps keep moving back, because each skips snapshots
  newer than one emulated second; a 300 ms debounce and held-key protection
  apply, and with no old-enough checkpoint R does nothing. Shift+R can select
  any checkpoint, including recent ones, and previews it with exact pixels
  and its emulated-time offset. Preview is read-only: cancel keeps the live
  state, and commit restores the checkpoint, clears stale audio and input,
  and persists save data. Both resume only if play was running. History
  resets on load, reset or rewind reconfiguration. See
  [`docs/design/rewind.md`](../docs/design/rewind.md).
- **Quick slots.** The footer buttons use slot 1. Slots belong to one
  playthrough, persist across reloads, never replace battery saves, and
  [sync](#quick-slot-sync) when cloud backups are on. The More menu shows each
  slot's save time and sync state, plus state and battery import and export.
- **Imports.** Drop a `.state` file or use **More → Import state** to restore
  a ZiGBA save state; it stays paused for inspection. Importing a battery
  `.sav` reboots the ROM with that save and starts play.

Transient notifications dismiss after two seconds. The screen fills its
column at the native 3:2 aspect ratio; game side panels stop growing at
460 px so wide desktops give the extra space to the screen, and fullscreen
fits both axes without stretching or cropping. Frames and audio bypass
React. The pixel font, VT323, is self-hosted under the SIL Open Font
License, with its license beside it in `src/assets/`.

### Library

The toolbar's Library opens a full-page collection: list and details side by
side on desktop, list then details on mobile. Search and the All games /
Installed / Not installed filters share one list, sorted by Recently played
by default, with unplayed titles following alphabetically. Mobile Back
preserves search, filter, scroll and focus. Browsing pauses the current
session and blocks gameplay input; Player returns without resetting it, and
**Continue** explicitly resumes the loaded game. Library **Continue**
restores a durable checkpoint when no matching session is loaded; **Play**
uses the battery save when no checkpoint exists.

The collection holds local imports and cloud-only games discovered from the
account; neither is a public catalog. Square logos can be supplied through
`LibraryPage` metadata, and missing or broken images fall back to title
initials. The Library does not query the server catalog, install patches or
show community activity. On the Vite dev server, `/library-preview.html` is a
labelled in-memory preview of installed, base-ready and missing-base states,
including the compact drop area and the mobile file picker. It is not part of
production builds and never stores the files it is given.

### Playthroughs

Each game can have several independent playthroughs. Use the Player's
**Playthrough** selector (with **New playthrough** at the end), the pencil to
rename, and the trash icon to delete a local playthrough after confirmation.
Deletion removes that run's local saves, checkpoints, quick slots and pending
uploads but keeps any confirmed cloud backup; deleting the last local run
starts a fresh empty one. New runs get UUIDs and can be created offline.

## Local saves and resume

All progress is stored in the `zigba-local` IndexedDB database (version 6).
Battery saves, resume checkpoints, quick slots, save locks and cloud outbox
jobs all include the playthrough ID; ROM bytes are stored once per exact
hash. Opening an older database upgrades it in one version transaction,
moving existing progress into playthrough `default` ("Playthrough 1"),
including pending cloud work and quick slots.

- **Autosave** runs every five seconds and on pause. It stores the battery
  data and an exact-ROM machine checkpoint with the boot and media options
  needed to restore it. Unchanged battery bytes are not rewritten.
- **Resume checkpoints** live in the `resumes` store, which keeps the latest
  and previous generation per run; `games.resumeSavedAt` records the active
  playthrough's availability without loading snapshot blobs. If the latest
  checkpoint is corrupt or incompatible, resume tries the previous one. If
  neither restores, progress is preserved, and autosave and playback stay
  blocked until a retry succeeds or the user chooses **Start from in-game
  save**.
- **Quick slots** use the `assets` store with keys
  `quick-state:["<ROM hash>","<playthrough ID>",<1-10>]` and values
  `{data, savedAt, cloudRevision?, accountId?}`; a bare byte array is also
  accepted and treated as never synced. Their outbox jobs use kind
  `quick/<slot>` in the `cloud` store.
- **Save ownership.** A tab must hold the Web Lock
  `zigba-save-playthrough:["<raw ROM SHA-256>","<playthrough ID>"]` to write
  a run's progress. Read-only tabs may resume but cannot persist, and
  opening another tab does not transfer unsaved state.

In-memory rewind history is separate from all of these and is not a resume
store. Snapshots are specific to one exact ROM; carrying battery saves
across ROM releases would need an explicit compatibility policy.

Closing the browser before a transaction completes, or storage eviction, can
lose progress, and an unavailable-storage indicator is not a save receipt.
Export important saves.

## Cloud backups

Cloud backups are on by default when signed in. **Account → Disable cloud
backups** stores an opt-out for that account on this device. Uploads carry
the resume checkpoint and battery save of each playthrough, compressed with
the browser's native gzip; they never include ROM bytes or a user-supplied
BIOS. Cloud storage keeps one current resume, battery save and state per
[quick slot](#quick-slot-sync) for each playthrough, not a history of
snapshots. The server contract is in
[`server/README.md`](../server/README.md#cloud-progress).

### When uploads happen

Cloud uploads reuse the autosave record; they do not capture or queue each
rewind checkpoint. They run at most every two minutes per playthrough and
save kind, and stop after five minutes without game input (held controls
count; animation and reconnecting do not). A battery save becomes pending
only when its bytes change. Offline hours coalesce into the latest record,
with no backlog; the page drains eligible work while it is open and
connected. An expired login pauses sync without stopping local persistence.
Signing out or switching accounts never moves pending jobs to another
account.

Hiding the tab pauses play, commits local progress and immediately attempts
the ordinary upload; `pagehide` repeats the attempt. Neither bypasses the
two-minute interval or the idle cutoff, and browser shutdown can interrupt
them, so use **Back up now** and wait for confirmation before switching
devices. Window blur alone releases controls without pausing.

### Restoring and conflicts

**More → Saves & options → Cloud progress** offers an acknowledged **Back up
now**, restore of the cloud resume or in-game save, and an explicit
**Replace cloud with local progress** (resume and in-game saves only).
Another device must supply the same ROM locally.

On sign-in, returning Home or reconnecting, Home reads the account's backup
metadata page by page without downloading snapshots. It offers the last
played local playthrough, not the newest upload from an unrelated run, and
offers the latest cloud run only when nothing is installed. The Library lists
local and cloud-only playthroughs for each game with their checkpoint and
upload times.

Loading a playthrough checks that run's cloud version. If the cloud changed
since this device's last acknowledged version, or the versions cannot be
compared, one dialog offers the cloud checkpoint or the local progress. An
unchanged cloud version never interrupts local play, even when the local
checkpoint is newer: timestamps orient the user but do not resolve
conflicts. **More → Restore cloud resume** opens the same dialog on demand.
Without a stored ROM it asks for the exact `.gba` or `.zip`, and no ROM bytes
reach the server.

An isolated emulator validates the ROM hash and checkpoint before the active
player changes. Account changes, incompatible snapshots and lock failures
cannot silently replace a stored checkpoint with a boot state, and a restore
is rejected if the cloud changes again after the choice. A successful
restore persists locally and starts playback. Conditional writes stop on
conflicting progress rather than choosing the newest clock time; to keep
both versions of a run, export the local progress first. The two local resume
generations are recovery slots, not permanent history. Server or network
failures leave local play available.

The server admits 100 playthroughs across all games per account, counted on
first upload rather than local creation. A 101st run stays local with a sync
error, while existing cloud runs keep updating.

**Account → Delete my account** asks once, then removes authentication and
cloud progress while keeping device-local games and saves. The server
answers once authentication is deleted and removes cloud objects
asynchronously, so cloud erasure is eventually consistent rather than
instant.

### Quick slot sync

Quick slots are local-first: saving commits the slot to IndexedDB at once,
online or not. With cloud backups on for this account and device, the same
transaction replaces that slot's single pending outbox revision and
immediately attempts the ordinary upload; there is no timer capture. Repeat
saves of one slot upload at most every 30 seconds, and failures share the
bounded retry backoff, which keeps quick slots within the server's upload
budgets. With backups off, nothing is queued.

Each slot's outbox entry records the last acknowledged revision and cloud
ETag, the common base. Opening a playthrough, signing in, enabling backups,
reconnecting and returning to the tab list that run's cloud slots
([API](../server/README.md#quick-slots)) and decide per slot:

- only this device changed: upload with `If-Match` on the base, or
  `If-None-Match` for a new slot;
- only the cloud changed, for example after a save on another device:
  replace the local slot silently after verifying its digest and state
  magic;
- both changed: keep both and ask once in **Quick slots changed**,
  preselecting the newer save time per slot. **Decide later** leaves both
  copies, and **More → Quick slots** shows a **Resolve** reminder without
  asking again until the cloud copy changes. The chosen copy replaces the
  other.

Loading a slot never waits for the network. A new device downloads a run's
cloud slots when that playthrough is opened or restored. A failed
conditional upload is recorded as a divergence, never retried as an
overwrite. Slots saved while signed out or with backups off upload on the
next sync unless the cloud already has that slot, which is a divergence. A
slot last synced for another account is never uploaded or retagged. Only
slots 1-10 sync, even if the account's server limit is higher; a lower limit
keeps the higher slots local with a sync message.

## Offline play

Production builds install a service worker that caches one complete version
of the player: every file of the Vite build, including the hashed SDK
worker, worklet and WASM and the game plugins' lazily loaded layouts. After
its first successful installation, the player can reload offline and resume
an imported local game. API responses, credentials and user media are never
cached. A new build waits for existing tabs to close before it activates, so
a running worker never loads an incompatible WASM binary. The Vite dev server
does not register the service worker. Offline play does not protect against
browser storage denial or eviction, or against closing the page before the
installation finishes.

## Game plugins

Game-specific panels are optional packages registered in
[`src/games.ts`](src/games.ts). A plugin (`GamePlugin`) contributes headless,
exact-build game definitions from
[`@evenfall/gba/bridge`](../docs/game-bridges.md) plus two kinds of React
layouts, keyed by game ID: **dashboards** for the player and **HUDs** for
`/debug.html#/hud`. Each page loads its kind of layout, with the plugin's
styles, before its first render, so a layout never swaps in under a running
game. `resolveGame` selects a layout by raw-ROM SHA-256, never by filename;
ambiguous matches are errors, and unknown ROMs keep the generic layout. A
layout receives the self-contained player as `children` plus `playing`, and
reads live data with `useBridge` inside the player's `BridgeProvider`.
`main.tsx` and `debug-main.tsx` never import a game package.

The default build installs
[`@evenfall/gba-pokemon`](../games/pokemon/README.md), which adds the
Heart & Soul 1.2.1 and Elite Redux 2.65 Beta2 dashboards and the Heart & Soul
HUD; its README describes those panels and their limits. To build without
it, remove its import and `installed` entry from `games.ts` and its
dependency from `package.json`. To add a game, export an object of the same
`GamePlugin` shape from another workspace package and append it to
`installed`.

## Dev / Debug

The `debug-main.tsx` and `debug.css` entry holds the diagnostic tools that
the player does not render: boot options (HLE or an external BIOS, save
type, RTC), library rename and delete, screenshots, a performance overlay,
failure recording, checkpoints, scenario export and isolated replay. Unlike
the player, it opens games paused. Its HUD route shows a plugin's HUD for a
matching ROM, or the first installed HUD as a diagnostic override. The page
exposes `window.zigbaDebug` (`Emulator` and `acquireSaveLock`) for console
use and browser tests.

Failure recording keeps by default three snapshots, 16 MiB of snapshot
payload, 4 MiB of estimated history and 4,096 operations. Guest faults retain
evidence; when recording is off, missing replay history is reported
explicitly. Scenario input journals freeze an exportable prefix at 1,024
inputs, and a new segment starts from a portable state. Lab workers never
write the player's battery save. Snapshot restore resets the host resampler,
so exact continuous-audio checks require a cold boot. Recorded baselines
detect regressions; they do not establish hardware correctness.

Exported scenarios and failure captures run from the command line:

```sh
node tools/run-scenario.mjs path/to/scenario.json /tmp/zigba-result
node tools/replay-capture.mjs path/to/failure.json path/to/scenario.json
```

The formats are described in [`docs/devtools.md`](../docs/devtools.md).

## Analytics

Analytics are opt-in at build time and off by default: `zig build web`, the
Vite dev server and tests send nothing. A production build sends anonymous,
cookieless Home, Player and Library pageviews to PostHog only when these
variables are set in the build environment:

| Variable | Meaning |
|---|---|
| `VITE_POSTHOG_KEY` | PostHog project key (a public ingestion key, not a secret). |
| `VITE_POSTHOG_HOST` | PostHog ingestion host, for example `https://eu.i.posthog.com`. |
| `VITE_POSTHOG_HOSTNAME` | Optional. Send only from pages served on exactly this hostname (recommended). |
| `VITE_PRIVACY_URL` | Optional. Shows a **Privacy notice** link on Home and in the Account dialog. |

`tools/build-cloudflare.sh` supplies the official instance's values and
publishes the repository's [`PRIVACY.md`](../PRIVACY.md) as `/privacy.txt`;
export your own values, or empty ones, when deploying elsewhere.

Debug pages, accounts, ROMs, gameplay, clicks, errors and recordings are
never tracked. URL queries, fragments, referrers and other event properties
are removed before sending. The page host and browser user agent are sent so
PostHog can hash cookieless events with the request IP; PostHog stores no
analytics cookie or browser identifier.

## Cloudflare deployment

The official instance, [zigba.ursid.ai](https://zigba.ursid.ai), is one
Worker using Static Assets plus the catalog API: `/api/*` runs through Hono
first, and other paths are static files. WASM, the emulator worker, audio and
IndexedDB run in the visitor's browser. Cloud saves use a private
EU-jurisdiction R2 bucket with public access disabled. No ROMs or test
fixtures are deployed. See the server's
[deployment and self-hosting guide](../server/README.md#deployment) and the
hosted instance's [privacy notice](../PRIVACY.md).

A Git-connected Cloudflare Workers Builds project can use these settings:

| Setting | Value |
|---|---|
| Production branch | `main` (no preview-branch deployments) |
| Root directory | `/` |
| Build command | `bash tools/build-cloudflare.sh` |
| Deploy command | `npm run deploy -w frontend` |
| Build variables | `NODE_VERSION=24.5.0`, `SKIP_DEPENDENCY_INSTALL=true`, `WRANGLER_SEND_METRICS=false` |

[`tools/build-cloudflare.sh`](../tools/build-cloudflare.sh) must run from the
repository root. It installs locked npm dependencies, type-checks and tests
the server, SDK bridge, Pokémon package and player, and builds the player
with ReleaseSafe WASM and Zig 0.16.0. On a Linux x86-64 runner without that
Zig, it downloads Zig and verifies its pinned SHA-256. It builds into a fresh
temporary prefix, refuses to publish game media or archive files, adds
`_headers` and license files, and publishes only `.dist/cloudflare`. It does
not reuse `zig-out/web` or need a manually uploaded WASM binary.

To build and preview the same deployment locally:

```sh
bash tools/build-cloudflare.sh
npm run preview:deploy -w frontend
```

To deploy by hand after that build, authenticate Wrangler with your
Cloudflare account (`npx wrangler login`, or a scoped `CLOUDFLARE_API_TOKEN`
plus `CLOUDFLARE_ACCOUNT_ID`), then run `npm run deploy -w frontend`. Keep
tokens out of the repository; a Git-connected build uses Cloudflare's managed
build token and needs no CI secret.

Schema changes require the explicit migration step in the
[server guide](../server/README.md#schema-changes); builds never migrate or
seed the database. Catalog management stays disabled unless the
`ADMIN_SECRET` Worker secret is deliberately configured.

The root `wrangler.jsonc` owns the custom domain and disables `workers.dev`
and preview URLs. Missing paths return 404 rather than the Home page. HTML
and the service worker keep Cloudflare's default revalidation, while Vite's
hashed assets, including the SDK worker and WASM, are cached as immutable
([`tools/cloudflare-headers`](../tools/cloudflare-headers)). Cross-origin
isolation is not required. To roll back, use the Worker's deployment history
in Cloudflare or redeploy a known-good commit.

Browser data is origin-scoped: libraries and saves on `localhost` do not
transfer to another domain, so export important saves before switching
origins.

## Tests

```sh
npm run check -w frontend
npm test -w frontend
zig build web
ZIGBA_TEST_PORT=8890 npm run test:browser -w frontend
```

`zig build test-frontend` and `zig build test-browser` run these together with
the [`@evenfall/gba-pokemon`](../games/pokemon/README.md#tests) suites. The
browser suite serves `zig-out/web` on `ZIGBA_TEST_PORT` (default 8765) and
covers the player, the Dev / Debug UI and the `/validation.html` SDK harness.
It uses `PLAYWRIGHT_CHROMIUM_EXECUTABLE` or `/usr/bin/chromium` when present;
otherwise install a browser with `npx playwright install chromium`.

Browser checks can also target a local Wrangler preview or a deployment,
where they use tiny generated test programs and browser-local storage:

```sh
ZIGBA_TEST_URL=https://your-deployment.example npm run test:browser -w frontend -- \
  home-navigation.spec.ts library.spec.ts
```

Some unit and browser tests skip unless `ZIGBA_*` variables point at ROMs
and states you supply; they verify fixture hashes before use, and no user
asset is committed. These checks do not establish retail-game compatibility
or cover physical controllers and mobile devices. Broader core and SDK checks
are listed in [`docs/testing.md`](../docs/testing.md).

## Open questions

- External-BIOS portability of cloud snapshots, and the content and legal
  implications of memory snapshots, need review before cloud payloads widen
  beyond states and battery saves.
- Individual cloud playthroughs cannot be deleted; account deletion is the
  only way to remove cloud data.
- Battery-save forwarding between releases of the same game has no
  compatibility policy.
