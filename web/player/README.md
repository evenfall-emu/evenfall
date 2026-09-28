# ZiGBA player

The browser player for ZiGBA: a TypeScript and React application built with
Vite, Tailwind CSS 4, Effect 3 and IndexedDB (through `idb`). Games run
entirely in the visitor's browser, and ROMs stay user-supplied and local.
Gameplay needs no account or backend. When the
[catalog and account server](../server/README.md) is available, signed-in
players also get private cloud backups of their progress.

The application has two entry points:

| Page | Entry | Purpose |
|---|---|---|
| `/` | `index.html`, `src/main.tsx` | The player: Home, Player and Library |
| `/validation.html` | `validation.html`, `src/validation.js` | SDK validation harness used by browser tests |

The player consumes the [JavaScript SDK](../docs/sdk.md) as the
`@evenfall/gba` workspace package, like any other application; Vite bundles
the SDK's worker, audio worklet and WASM as hashed files under `assets/`.
Pokémon panels come from the separate
[`@evenfall/gba-pokemon`](../games/pokemon/README.md) package through the
[game-plugin registry](#game-plugins). The runtime architecture (worker
session, pacing, audio, rewind) is described in
[`docs/design/web-runtime.md`](../docs/design/web-runtime.md).

## Styling

Tailwind CSS 4 runs through its Vite plugin. [`src/styles.css`](src/styles.css)
imports Tailwind and defines the Evenfall design tokens in its `@theme` block:
surface, boundary, text, brand (mint and peach) and feedback colors, control
and panel radii, popover and dialog shadows, and two faces. Each token
generates utilities such as `bg-surface`, `text-muted`, `border-line`,
`rounded-panel` and `shadow-dialog`; components style themselves with these
utilities rather than raw values. Shared building blocks live in
[`src/ui/`](src/ui/): stroke icons and pixel-art icons (`PixelIcon`, drawn
at whole-number scales), button and popup classes, and a dialog and selects
built on Base UI.

Text uses Press Start 2P (`font-sans`), self-hosted under the SIL Open Font
License in `src/assets/fonts/`. It draws every glyph on an 8-pixel em and has
one weight, so hierarchy comes from size and color rather than bold. The
text sizes name how many screen pixels one font pixel covers: `text-2x`
(16px), `text-3x`, `text-4x` and `text-5x` keep every font pixel square and
sharp, and `text-1x5` (12px) is the one half step, for secondary text.

The stylesheets that predate Tailwind (`console.css`, `account.css` and
`rewind-timeline.css`) are imported by `styles.css` into a
`legacy` cascade layer, above Tailwind's base styles and below its
utilities. Their global element rules skip any subtree marked `data-ui`,
which every Tailwind-built root carries, including popups rendered into a
portal. `console.css` also gives the page the browser's default line height
and its cream text color in place of Tailwind's `1.5` and `text-content`, so
legacy pages and game-plugin dashboards keep the metrics they were laid out
on; the outermost `data-ui` roots set Tailwind's two values again. Because
the legacy stylesheets set a fluid root font size, `styles.css` restates
Tailwind's spacing, type, container and radius scales in pixels.

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
over localhost or HTTPS, from the site root: asset URLs are absolute so they
resolve from nested page URLs. `tools/serve-web.py` is a loopback-only
static server (`--port`, `--directory`) that answers the player's page URLs
with `index.html`.

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

Each page has a URL, handled by TanStack Router, so a refresh or a shared
link lands on it again:

| URL | Page |
|---|---|
| `/` | Home |
| `/library` | Library |
| `/library/<game ID>` | Library, with that game's details |
| `/player` | Player |
| `/share/state/<ID>` | Home, starting a [shared state](#sharing-a-state) |

A fresh page load has no running game, so `/player` opens the Library
instead. The browser's Back and Forward buttons move
between pages like the tabs. Other paths are 404s, and servers answer only
these URLs with the player's `index.html`.

Home is the default destination: a pixel-art fresco that scrolls from a
coastal sunset down to a seabed, with the product story in the water
between. Its artwork and rendering are described in
[`src/assets/landing`](src/assets/landing/README.md). Its main action,
**Open player**, becomes **Resume adventure** after a reload for the last
played local game's selected playthrough, shown with its name and checkpoint
time; if no game is installed, it can offer a cloud-only run. **Features** in
its header scrolls down to the product story, smoothly unless reduced motion
is preferred. Its footer carries the trademark and non-affiliation notice
and, when [configured](#analytics), a privacy-notice link. Opening a ROM from
Home switches to Player and starts it.

Library and Player share one header with a tab for each, the evenfall logo
and **Account**; Home has its own with **Features**, **Library** and
**Account**, which on phones shows only Library and Account, as icons.
Playback actions live in a separate Player toolbar below the header. The
logo returns Home without unloading the current game. Returning to Player
resumes the session only if it was playing before navigation, including
through the browser's Back and Forward buttons. On narrow screens the tabs
move to a second row beside an Account button, where the Library's details
replace them with **Back to Library**.

**Account** is available on Home and in the header. Opening it pauses play
and persists progress before Discord can navigate away; closing it resumes a
game that was running. Sign-in is optional, and an unavailable server never
blocks local play. Signing in does not upload ROMs or library contents and
grants no catalog rights.

## Player controls

Open or drag a `.gba` file or a single-ROM ZIP onto the player. The native
importer normalizes the ROM; its raw SHA-1 identifies the game and its save
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

- **Toolbar.** [**Share state**](#sharing-a-state), **Back up now** (with a
  game open and cloud backups on), fullscreen, sound and playback speed.
  Hover or focus the sound button for a volume slider; volume is stored on
  the device and mute does not reset it.
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

The header's Library tab, or **Library** in Home's header, opens the game
collection: a filterable list beside the selected game's details. Selecting a
game changes the URL to `/library/<game ID>`; filtering the list keeps that
game's details. On narrow screens the details replace the list, and Back or
Escape returns to it with its scroll position and focus. Search, **Show**,
**Franchise**, **Sort** and a tag picker combine; a game must carry every
selected tag. **Show** offers All, Installed and Ready, where the base ROM is
on this device and the release's patch is hosted, so the game plays at once.
**Sort** orders by Played (the default, with unplayed titles following by
name), Recently updated (latest release date, undated releases last) or
Popularity (cloud playthroughs across accounts).
**Library settings → Show experimental romhacks** is off by default and
remembered on this device. Featured and reviewed normal releases appear by
default; experimental releases carry a label and appear only with this opt-in.
Hidden and patchless releases do not appear in the public catalog. An already
installed ROM or cloud playthrough whose release is omitted remains available
as an unlisted, user-owned game instead of losing its progress.
Each row shows whether the game is installed, ready, or needs a ROM from the
user. ROMs on this device and cloud playthroughs the catalog does not list
as a release appear as games of their own. A catalog base keeps the catalog's
name for it; any other ROM takes its No-Intro dump name from
[`no-intro-gba.json`](src/pages/library/no-intro-gba.json), a SHA-1 to name
table the Library loads as its own chunk, and otherwise the title it was
stored with. Games without catalog artwork show their title initials.

Details show the project page, the latest release's notes, an update offer
when a playthrough is on an older release, and each playthrough's release,
last play time and cloud state. **Update…** copies one playthrough's
in-game save to a new playthrough on the latest release, under the same name,
and plays it; unless the catalog declares the saves compatible, it requires
acknowledging that first. When the latest release's ROM is not on this
device, **Update…** asks for it first. Playing a run whose cloud copy changed
since this device last synced asks which progress to continue. A playthrough's
menu renames it, exports its in-game save, or deletes it after confirmation,
as the Player's controls do; cloud-only runs have no menu.

**Choose file…** reads a `.gba` or single-game ZIP in an isolated Worker and
stores it, without playing it, only when its SHA-1 is one of the game's
releases or the base ROM its latest patch needs. No playthrough exists until
the first play. **Install** downloads the latest release's hosted patch,
applies it to the base ROM on this device
([`src/patch.ts`](src/patch.ts) reads BPS, UPS and IPS), and stores the result
only when its SHA-1 is the release's; **Play** on a ready game installs it,
then plays. BPS and UPS patches also check their own base, output and patch
CRC32s. **Add game** opens a ROM in the Player. Browsing pauses the current
session and blocks gameplay input; Player returns without resetting it.

The catalog comes from `GET /api/games` and `GET /api/versions`
([`use-catalog.ts`](src/pages/library/use-catalog.ts)); the last response is
kept in `localStorage`, so the Library shows it at once and while offline.
The device side is the local database's games, playthroughs and cloud outbox,
and the account's cloud checkpoints;
[`model.ts`](src/pages/library/model.ts) joins both into what the page shows.
Browser tests serve their own catalog through Playwright routes, and unit
tests use [`tests/library-fixtures.ts`](tests/library-fixtures.ts).

### Playthroughs

Each game can have several independent playthroughs. Use the Player's
**Playthrough** selector (with **New playthrough** at the end), the pencil to
rename, and the trash icon to delete a local playthrough after confirmation.
Deletion removes that run's local saves, checkpoints, quick slots and pending
uploads but keeps any confirmed cloud backup; deleting the last local run
starts a fresh empty one. New runs get UUIDs and can be created offline.

## Local saves and resume

All progress is stored in the `zigba-local` IndexedDB database (version 8).
Every key starts with the game's raw-ROM SHA-1. Battery saves, resume
checkpoints, quick slots, save locks and cloud outbox jobs all include the
playthrough ID; ROM bytes are stored once per exact hash. Databases from
version 6 and earlier keyed games by SHA-256; opening one deletes its contents
and starts an empty library. Version 8 adds the `builds` store and keeps
everything else.

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
  `zigba-save-playthrough:["<raw ROM SHA-1>","<playthrough ID>"]` to write
  a run's progress. Read-only tabs may resume but cannot persist, and
  opening another tab does not transfer unsaved state.

- **Recognized builds** live in the `builds` store under
  `<game ID>:<raw ROM SHA-1>:<locator version>`, each a located manifest and
  its review list (see [Game plugins](#game-plugins)). They are a cache: a
  new locator version misses it and locates the build again.

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

### Sharing a state

**Share state** in the Player toolbar pauses the game and uploads its exact
current save state, which includes the in-game save, as a link that anyone
can open for at least 24 hours. The dialog shows the link with **Copy link**,
and closing it resumes play if the game was running. Creating a
link needs a signed-in account, not cloud backups; signed out, the dialog
offers **Sign in** and uploads nothing. The upload carries the playthrough
name, the game's title and the ROM's SHA-1, never the ROM itself. The
server's side is in
[`server/README.md`](../server/README.md#state-shares).

For an [admin account](../server/README.md#including-the-rom), the dialog
also offers **Include the game**, which uploads the stored ROM, gzip
compressed, with that link. Other accounts never see it, and the server
refuses them.

Opening `/share/state/<ID>` needs no account. The player downloads the state,
checks its digest and waits for the catalog request to settle, so it knows
whether the shared ROM is a catalog release. It uses the shared ROM if this
device has it. Otherwise, for a release whose base ROM is here, it builds the
release with the hosted patch, as **Install** does; failing that, it
downloads a ROM the sharer included and stores it once its SHA-1 matches the
state's. It validates the state
against the ROM in an isolated emulator before the player changes, then
starts the state at once as a new playthrough under the shared name, never
replacing an existing one. It asks first only when it needs something:

- a ROM, if this device has neither the shared ROM nor the release's base
  and the link does not include it. The prompt accepts either one and
  patches a base locally. The chosen file
  is stored as **Choose file** stores it, so it stays in the Library even if
  the state or patch then fails, and it is never uploaded;
- a new name, if a playthrough of this game already has the shared name,
  ignoring case and surrounding spaces. Cloud-only runs count.

Starting replaces the link in the browser history with `/player`, so Back
does not start it again. An expired or unknown link says so and offers Home.

## Offline play

Production builds install a service worker that caches one complete version of
the player: every file of the Vite build, including the hashed SDK worker,
worklet and WASM and the game plugins' lazily loaded layouts. After its first
successful installation, the player can reload offline, on any of its page
URLs, and resume an imported local game. API responses, credentials and user
media are never cached. The Vite dev server does not register the service
worker. Offline play does not protect against browser storage denial or
eviction, or against closing the page before the installation finishes.

### Updates

The player looks for a new version when a page loads, when it returns to the
foreground, when the browser comes back online and every hour
([`src/app-update.ts`](src/app-update.ts)). A new version installs next to the
running one. A tab with no game loaded, nothing changing and no dialog open
reloads into it at once if the user has not touched the page yet or the tab is
hidden; automatic reloads in one tab are at least a minute apart. Otherwise the
tab shows _A new version of Evenfall is ready_: **Update now** saves the
running game's resume checkpoint and reloads, and the notice can be dismissed.
Once one tab switches to the new version, every other tab follows the same
rule. A tab still running an older version keeps loading that version's hashed
Worker, WASM and chunks from its cache, never an incompatible binary; older
versions' caches are deleted when a page of the new version starts as the only
open tab, or when the new version activates with no tab open.

## Game plugins

Game-specific panels are optional packages registered in
[`src/games.ts`](src/games.ts). A plugin (`GamePlugin`) contributes headless,
exact-build game definitions from
[`@evenfall/gba/bridge`](../docs/game-bridges.md) plus React **dashboards**,
layouts keyed by game ID. The player loads every plugin's dashboards, with
their styles, before its first render, so a layout never swaps in under a
running game. `resolveGame` selects a layout by raw-ROM SHA-1, never by
filename; ambiguous matches are errors. A layout receives the self-contained
player as `children` plus `playing` and `build`, and reads live data with
`useBridge` inside the player's `BridgeProvider`. `main.tsx` never imports a
game package.

A ROM that matches no reviewed build is recognized with
`gameRegistry.recognize`: from the `builds` cache, else by running the
[locators](../docs/game-bridges.md#recognizing-builds-at-runtime) of each
installed game that has them, on the page while the game plays. The player
shows "Recognizing game…" once a game's detection matches, then the game's
layout with `build.provenance` set to `located`; unrecognized ROMs keep the
generic layout. A located build whose views keep failing is forgotten
(`gameRegistry.forget`) and the generic layout returns.

The default build installs
[`@evenfall/gba-pokemon`](../games/pokemon/README.md), which adds the
Heart & Soul and Elite Redux dashboards; its README
describes those panels and their limits. To build without it, remove its
import and `installed` entry from `games.ts` and its dependency from
`package.json`. To add a game, export an object of the same `GamePlugin` shape
from another workspace package and append it to `installed`.

## Developer tools

The player has no scenario or failure-capture interface. The SDK's `Emulator`
records gameplay, marks checkpoints, exports scenarios and captures failures,
and a separate `Emulator` runs a scenario or replays a capture without
touching a player's session. The same formats run from the command line:

```sh
node tools/run-scenario.mjs path/to/scenario.json /tmp/zigba-result
node tools/replay-capture.mjs path/to/failure.json path/to/scenario.json
```

The formats are described in [`docs/devtools.md`](../docs/devtools.md).

## Analytics

Analytics are opt-in at build time and off by default: `zig build web`, the
Vite dev server and tests send nothing. A production build sends anonymous,
cookieless Home, Player and Library pageviews, and the share events
`state_share_created` and `state_share_opened`, to PostHog only when these
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

Accounts, ROMs, gameplay, clicks, errors, recordings and share-link pages
are never tracked. URL queries, fragments, referrers and other event properties
are removed before sending; share events carry no URL at all, so neither a
link nor its game can reach PostHog. The page host and browser user agent are sent so
PostHog can hash cookieless events with the request IP; PostHog stores no
analytics cookie or browser identifier.

## Cloudflare deployment

The official instance,
[play.evenfall-emu.org](https://play.evenfall-emu.org), is one Worker using
Static Assets plus the catalog API: `/api/*` runs through Hono first, and
other paths are static files. WASM, the emulator worker, audio and
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
and preview URLs. It sends the player's page URLs (`/library`,
`/library/<game ID>`, `/player`) to the Worker, which answers them with the
player's `index.html`; other missing paths return 404 rather than the Home
page. HTML
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
covers the player and the `/validation.html` SDK harness, whose
`window.zigbaValidation` hooks (`Emulator` and `acquireSaveLock`) also serve
the SDK and save-lock tests.
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
