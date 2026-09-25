# ZiGBA Player and Dev / Debug

A local TypeScript/React application built with Vite, Effect 3 and IndexedDB
through `idb`. Gameplay needs no account or backend. The deployed Worker also
hosts a metadata-only [Hono/Drizzle/D1 catalog API](../server/README.md) with
optional Discord and magic-link login. ROMs stay user-supplied and local.
Signed-in players get optional private cloud progress backup, including named
playthroughs, default-on cloud backup, Reset and account deletion.

## Build and run

From the repository root (Zig 0.16 and Node 24+):

```sh
npm ci                 # one install for every npm workspace
zig build web
python3 tools/serve-web.py
```

Open `http://127.0.0.1:8765/` for the new player. It has its own React entry
point and stylesheet, not the previous application's pages or control deck.
The home page introduces ZiGBA and is the default destination; its footer
carries the trademark/non-affiliation notice and, when
[configured](#analytics), a privacy-notice link. Clicking the
ZiGBA wordmark returns home without unloading the current game; Player returns
to the same session and resumes it only when it was playing before navigation.
Opening a ROM from Home switches to Player and starts it automatically.
Home, Player and Library share the Library-style navigation header. Playback
actions live in a separate Player toolbar below it. After reloading, Home's
**Resume adventure** offers the last played local game's selected playthrough,
with its name and checkpoint time. It checks that run's cloud copy before loading;
if no game is installed, it can offer a cloud-only run. In-memory rewind history
is not the resume store.
The old interface is isolated at `/debug.html`, with diagnostic HUD at
`/debug.html#/hud` and Lab at `/debug.html#/lab`. Dev / Debug remains accessible
through **More → Dev / Debug player**, not the main navigation. These tabs
share storage, not an emulator session.

The deployable static directory is `zig-out/web`; Vite writes it through the
`zig-out/player` intermediate. The Zig install prefix is respected. Serve over
localhost or HTTPS. For frontend development:

```sh
zig build web-sdk      # builds web/zigba.wasm for the SDK workspace
npm run dev -w frontend
```

The player consumes the [JavaScript SDK](../docs/sdk.md) as the `@evenfall/gba`
workspace package, like any other application: Vite bundles its worker,
audio worklet and WASM as hashed files under `assets/`. Pokémon panels come
from the separate [`@evenfall/gba-pokemon`](../games/pokemon/README.md) package
through the [game-plugin registry](#game-plugins).

For local login, also run the Wrangler backend on port 8787, with its local
migrations and ignored secrets configured as described in the
[server setup](../server/README.md#local-development-and-tests). Open exactly
`http://127.0.0.1:5173`; Vite proxies `/api` to Wrangler. The static Python
server cannot handle login. Local magic links are captured in Wrangler's email
simulator rather than delivered to real inboxes.

**Account** is available on Home and in the shared Player/Library header.
Opening it pauses gameplay and persists progress before Discord can navigate
away; closing it resumes a previously running game. Authentication remains
optional, and unavailable sign-in does not prevent local play. Signing in does
not grant catalog editing or upload ROMs or library contents.

## Offline play and cloud progress

Production builds install a service worker that caches one complete version of
the player, SDK and WASM: every file of the Vite build, including the hashed
SDK worker, worklet and WASM and the game plugins' lazily loaded layouts. After its first successful installation, the player
can reload offline and resume an imported local game. API responses, credentials
and user media are never stored in that cache. A new build waits for existing
tabs to close before activation. Development Vite pages do not register it.
Browser storage denial/eviction and closing before installation finishes remain
limitations; offline play is not a guarantee against losing browser data.

Cloud backups default on when signed in. **Account → Disable cloud backups**
persists an opt-out for this account on this device, including across reloads.
Local resume persists every five seconds and on pause. Cloud uploads reuse that
record; they do not capture or queue each rewind checkpoint. They run at most
every two minutes per playthrough/save kind and stop after five minutes without game
input. Held controls count; animation and reconnecting do not. Battery saves
only become pending when their bytes change. Both are compressed with native
browser gzip; experimental zstd/delta results are not the shipped codec.

Hiding the tab pauses play, commits local progress, then immediately attempts
the ordinary cloud drain. `pagehide` repeats this best-effort attempt; neither
event bypasses the two-minute interval or idle cutoff. Browser shutdown/freezing
can interrupt the work, so use **Back up now** and wait for confirmation before
switching devices. Window blur alone releases controls without pausing play.

IndexedDB v6 stores account/run-bound pending/acknowledged revisions, input time,
in-flight identity and retry deadlines alongside existing save data. Offline
hours coalesce into the latest record; no separate snapshot payload or backlog
is retained. The page drains eligible work when connected, not while closed.
Expired login pauses network sync without stopping local persistence. Signing
out or changing accounts never transfers old jobs to another account.

**More → Saves & options → Cloud progress** offers an acknowledged **Back up
now**, cloud resume/in-game-save restore, and an explicit replacement action.
Another device must supply the same playable ROM locally. Conditional remote writes stop
on conflicting progress rather than silently choosing the newest clock time.
Restoring is explicit and prompts before replacing local progress; export local
progress first to keep both versions of the same run. Cloud storage retains one
current resume, battery save and state per [quick slot](#quick-slot-sync) for each
playthrough, not snapshot history. BIOS portability/content
review is [deferred](../docs/research/snapshot-compression.md#deferred-bios-and-content-review).

On sign-in, returning Home, or reconnecting, Home reads the account's paginated
backup metadata without downloading snapshots. Home offers the last played
local playthrough, not the newest upload from an unrelated run. Only if nothing
is installed does it offer the latest cloud run. The Library lists local and
cloud-only playthroughs for each game, including their available checkpoint
and upload times. The Player's playthrough selector checks the chosen run too.

Loading a playthrough checks that run's cloud version. If the cloud has changed
since this device's last acknowledged version, or the versions cannot be
compared, a single dialog offers the cloud checkpoint or that run's local
progress. A known unchanged cloud version does not interrupt local play, even
when the local checkpoint is newer; timestamps are for orientation, not conflict
resolution. Explicit **More → Restore cloud resume** opens that same dialog even
when the versions match. Without a stored ROM it asks for the exact `.gba` or `.zip`;
no ROM bytes reach the server. An isolated emulator
validates the ROM hash and checkpoint before changing the active player. Account
changes, incompatible snapshots, and writer-lock failures cannot silently replace
a stored checkpoint with a boot state. If the cloud changes again after the
choice, the restore is rejected. A successful restore persists locally,
starts playback, and can subsequently resume offline. A different playthrough
is stored separately; replacing the same run requires choosing the cloud copy. Keep a
separate export to preserve divergent versions of the same run; the two local generations are recovery
slots, not permanent history. Server/network failures leave local play available.

Use the Player's **Playthrough** selector (with **New playthrough** at the end),
the pencil to rename, and the trash icon to delete a local playthrough after
confirmation. Deletion removes that run's local saves, checkpoints, quick slots
and pending uploads, but retains any confirmed cloud backup. Deleting the last
local run starts a fresh empty one. Battery saves, resume snapshots,
quick slots, writer locks and outbox jobs all include that run's ID. ROM bytes
remain stored once per exact hash. The v5 → v6 migration moves existing progress
into `default` (Playthrough 1), including pending cloud work and quick slots,
in one transaction. New runs are independent UUIDs and can be created offline.
Cloud admits 100 playthroughs across all games per account. A 101st stays local
with a clear sync error; existing cloud runs can still update. The quota applies
on first upload, not local creation. See the [API contract](../server/README.md#private-cloud-progress)
for legacy quota accounting and failed-upload reservations.

**Account → Delete my account** has one confirmation and removes authentication
and cloud progress while keeping device-local games and saves. The server returns
202 after deleting authentication; R2 cleanup continues automatically. An opaque
account-ID/timestamp tombstone is retained to catch late uploads with recurring
sweeps. Cloud erasure is eventually consistent and depends on the cleanup cron;
it is not an instant-erasure promise. Individual cloud-run deletion is not included.

Snapshots must remain exact-ROM-specific; battery-save forwarding across ROM
releases needs an explicit compatibility policy rather than an automatic match.

### Quick slot sync

Quick slots 1–10 stay local-first: saving commits the slot to IndexedDB at once,
offline or not. With cloud backups enabled for this account and device, the
same transaction replaces that slot's single pending outbox revision, and the
save immediately attempts the ordinary drain; there is no timer capture. Repeat
saves of one slot upload at most every 30 seconds, and failures use the shared
bounded backoff, so quick slots stay within the server's upload budgets. Offline
saves wait for reconnection. With backups off, nothing is queued.

Each slot's outbox entry records the last acknowledged revision and cloud ETag,
the common base. Opening a playthrough, signing in, enabling backups,
reconnecting and returning to the tab list that run's slots
([API](../server/README.md#cloud-quick-slots)), then decide per slot:

- only this device changed: upload with `If-Match` on the base, or
  `If-None-Match` for a new slot;
- only the cloud changed, for example a save on another device: silently
  replace the local slot after verifying its digest and state magic;
- both changed: keep both untouched and ask once in **Quick slots changed**,
  preselecting the newer save time per slot. **Decide later** leaves both copies;
  **More → Quick slots** keeps a **Resolve** reminder without asking again until
  the cloud copy changes again. The chosen copy replaces the other.

Loading a slot never waits for the network. A new device downloads a run's
cloud slots when that playthrough is opened or restored. A failed conditional
upload is recorded as a divergence, never retried as an overwrite. Slots saved
while signed out or with backups off upload on the next sync, unless the cloud
already has that slot, which is a divergence. A slot last synced for another
account is never uploaded or retagged automatically; the old account's pending
job stays with that account. **Replace cloud with local progress** covers resume
and in-game saves only. Only slots 1–10 sync, even if an account's server limit
is higher; a lower limit keeps higher slots local with a sync message.

## Analytics

Analytics are opt-in at build time and off by default: `zig build web`, the
Vite dev server and tests send nothing. A production build enables anonymous,
cookieless Home/Player/Library pageviews through PostHog only when these
variables are set in the build environment:

| Variable                | Meaning                                                                       |
| ----------------------- | ----------------------------------------------------------------------------- |
| `VITE_POSTHOG_KEY`      | PostHog project key (a public ingestion key, not a secret).                   |
| `VITE_POSTHOG_HOST`     | PostHog ingestion host, for example `https://eu.i.posthog.com`.               |
| `VITE_POSTHOG_HOSTNAME` | Optional. Send only from pages served on exactly this hostname (recommended). |
| `VITE_PRIVACY_URL`      | Optional. Shows a **Privacy notice** link on Home and in the Account dialog.  |

`tools/build-cloudflare.sh` supplies the official `zigba.ursid.ai` values, and
publishes the repository's [`PRIVACY.md`](../PRIVACY.md) as `/privacy.txt`;
export your own values (or empty ones) when deploying elsewhere.

Debug pages, accounts, ROMs, gameplay, clicks, errors and recordings are never
tracked. URL queries, fragments, referrers and arbitrary event properties are
removed before sending. The hostname and browser user agent are sent so PostHog
can hash cookieless events with the request IP; PostHog does not persist an
analytics cookie or browser identifier.

## Cloudflare deployment

The official instance, [zigba.ursid.ai](https://zigba.ursid.ai), is one Worker
using **Static Assets plus the catalog API**. `/api/*` runs through Hono first;
other paths use Static Assets. WASM, the emulator Worker, audio and IndexedDB
still run in the visitor's browser. Cloud saves use a private EU-jurisdiction
R2 bucket with public access disabled. No ROM storage or private test fixtures
are included. See the [save API](../server/README.md#private-cloud-progress),
[deployment and self-hosting](../server/README.md#deployment) and the hosted
instance's [privacy notice](../PRIVACY.md).

A Git-connected Cloudflare Workers Builds project can use these settings:

| Setting           | Value                                                                                |
| ----------------- | ------------------------------------------------------------------------------------ |
| Production branch | `main` (no preview-branch deployments)                                               |
| Root directory    | `/`                                                                                  |
| Build command     | `bash tools/build-cloudflare.sh`                                                     |
| Deploy command    | `npm run deploy -w frontend`                                                         |
| Build variables   | `NODE_VERSION=24.5.0`, `SKIP_DEPENDENCY_INSTALL=true`, `WRANGLER_SEND_METRICS=false` |

The build installs locked npm dependencies, runs server and frontend tests and
type-checking, and compiles **ReleaseSafe WASM with Zig 0.16.0**. On Cloudflare's
Linux x86-64 runner, the script downloads Zig and verifies its pinned SHA-256.
It builds into a fresh temporary prefix, rejects game media/archive files,
and publishes only `.dist/cloudflare`. It does not reuse `zig-out/web`, run
the research environment setup, or require a manually uploaded WASM binary.

To build and preview the same deployment locally, from the repository root:

```sh
bash tools/build-cloudflare.sh
npm run preview:deploy -w frontend
```

To deploy manually after that build, authenticate Wrangler with your Cloudflare
account (`npx wrangler login`, or a scoped
`CLOUDFLARE_API_TOKEN` plus `CLOUDFLARE_ACCOUNT_ID`), then run
`npm run deploy -w frontend`. Keep tokens out of the repository. A
Git-connected build uses Cloudflare's managed build token; it does not need a
GitHub Actions secret.

Schema changes require the explicit pre-deploy migration step documented in
the [server guide](../server/README.md#schema-changes); ordinary builds do not
mutate the database or re-seed it. Catalog management stays disabled unless
the `ADMIN_SECRET` Worker secret is intentionally configured.

`wrangler.jsonc` owns the custom domain and disables workers.dev/preview URLs.
Missing paths return 404 rather than the Home page. HTML and the service worker
retain Cloudflare's revalidation policy; Vite's hashed assets, including the
SDK worker and WASM, use immutable caching. Cross-origin isolation is not required. To roll back, use
the Worker's deployment history in Cloudflare or redeploy a known-good commit.

Browser checks can target a local Wrangler preview or a deployment:

```sh
ZIGBA_TEST_URL=https://your-deployment.example npm run test:browser -w frontend -- \
  home-navigation.spec.ts library.spec.ts
```

Those cases use tiny generated test programs and browser-local storage, not
private ROMs or a server-side upload endpoint. Browser libraries and saves are
origin-scoped: existing localhost data does not automatically transfer to
another domain. Export important saves before switching origins.

## Player controls

Open or drag a `.gba` or single-ROM ZIP onto the player. The native importer
normalizes the ROM; its raw SHA-256 identifies the game and save lease. The
toolbar's Library opens a full-page collection: list and details on desktop,
list → details on mobile. Search and All games / Installed / Not installed
share one list, sorted by Recently played by default (unplayed titles follow
alphabetically). Mobile Back preserves search, filter, scroll and focus.
Browsing pauses the current session and blocks gameplay input. Player returns
without resetting it and resumes only if it was running; Continue explicitly
resumes the loaded game. Unknown ROMs use a generic player without Pokémon panels.

The production collection contains local imports and discovered cloud-only games;
neither is a public catalog. Square logos can
be supplied through `LibraryPage` metadata; missing or broken images use title
initials. No catalog, patch installation, or activity backend is connected yet.
`/library-preview.html` on the Vite dev server is a labelled, in-memory preview
of installed, base-ready, and missing-base states, including the compact file
drop area and mobile native picker. It is not a production build entry, never
stores selected files, and reports unavailable actions explicitly. Community
new-run counts stay absent unless supplied by a genuine activity service.

- Opening a ROM automatically starts playback from its battery save. Library
  **Continue** restores a durable checkpoint when no matching session is loaded;
  **Play** uses the battery save when no checkpoint exists. The separate Dev /
  Debug entry deliberately starts paused.
- Space toggles play/pause, R restores one checkpoint, and Shift+R opens the
  rewind timeline. The toolbar gamepad and screen `?` open **How to play**, with
  illustrated keycaps and a touch-control toggle. Help pauses play and resumes
  it on close if it was running before opening the dialog.
- Arrows move; X/Z are A/B; Enter is Start; Backspace is Select; A/S are L/R.
  Standard gamepads and captured touch pointers are supported. Typing in
  fields and open dialogs does not drive the game. Moving into a field or
  losing window focus releases buttons; hiding the page pauses.
- The toolbar has a **Back up now** cloud-upload shortcut (available with a game
  open and cloud backups enabled), fullscreen, sound and playback speed. Hover
  or focus the sound button for a volume slider; volume is saved on this device
  and mute does not reset it. Game keys work without canvas focus, while text
  fields, dialogs and handled widget keys retain their normal behavior. Hold
  Tab while playing for 16×; release to restore speed. Shift+Tab navigates
  normally. Speed affects wall-clock pacing, not guest cycles.
  Audio follows the selected rate with corresponding pitch changes.
- **Reset** restarts emulation using the current playthrough's in-game battery
  save, preserves whether play was running or paused, and clears rewind history.
  It does not create a new run or erase saved progress.
- R restores the newest checkpoint at least one emulated second earlier,
  skipping more recent snapshots so repeated taps keep moving back. A 300 ms
  debounce and held/repeated-key protection remain. If no checkpoint is old
  enough, R leaves the game unchanged. Shift+R can still select any available
  checkpoint, including recent ones, with exact pixels and emulated-time offsets.
  Preview is read-only: cancel preserves live state and resumes only if play was
  running; commit restores the selected checkpoint, clears stale audio/input,
  persists save data and follows the same resume rule. History resets on load,
  reset or rewind reconfiguration. Transient notifications dismiss after two
  seconds, including when playback resumes.
- Shift+1–9 saves a quick slot and Ctrl+1–9 loads one; 0 is slot 10. The footer
  buttons use slot 1. Slots are playthrough-specific, persist across reloads, do
  not replace battery saves, and [sync](#quick-slot-sync) when cloud backups are on.
  The More menu shows each slot's save time and sync state, plus state/battery
  import/export.
- Drop a `.state` or use **More → Import state** to restore a ZiGBA snapshot.
  Importing a battery `.sav` reboots the ROM with that save. Save/state imports
  differ: state imports stay paused for inspection; battery imports start play.

The screen fills its layout column at the native 3:2 aspect ratio. Side panels
stop growing at 460px so wider desktops give extra space to the player; stacked
and generic layouts have no fixed player width cap. Fullscreen fits both axes
without stretching or cropping the image.
Frames and audio bypass React. The pixel font is self-hosted VT323 under the
SIL Open Font License, included beside the font in `src/assets/`.

## Game plugins

Game-specific panels are optional packages registered in
[`src/games.ts`](src/games.ts). A plugin contributes headless, exact-build game
definitions from [`@evenfall/gba/bridge`](../docs/game-bridges.md) plus two kinds
of React layouts, keyed by game id: **dashboards** for the main player and
**HUDs** for `/debug.html#/hud`. Each page loads its kind (with the plugin's
styles) before its first render, so a layout never swaps in under a running
game. `resolveGame` then selects a layout by raw-ROM SHA-256, never by
filename; ambiguous matches are errors, and unknown ROMs keep the generic
layout. A layout receives the self-contained player as `children`, plus
`playing`, and reads live data with `useBridge` inside the player's
`BridgeProvider`. `main.tsx` and `debug-main.tsx` never import a game package.

The default build installs [`@evenfall/gba-pokemon`](../games/pokemon/README.md),
which adds the Heart & Soul 1.2.1 and Elite Redux 2.65 Beta2 dashboards and
the Heart & Soul HUD; its README describes those panels and their limits. To
build without it, remove its import and `installed` entry from `games.ts` and
its dependency from `package.json`. To add a game, export an object of the
same `GamePlugin` shape from another workspace package and append it to
`installed`.

## Dev / Debug retains the diagnostic tools

The independent `debug-main.tsx` / `debug.css` entry retains the former
Player/HUD/Library/Lab pages: boot options (HLE/BIOS, save type, RTC), library
rename/delete, screenshots, performance overlay, failure recording,
checkpoints, scenario export and isolated replay. These tools are not rendered
under the new player. Its HUD route shows a plugin's HUD layout for a matching
ROM, or the first installed HUD as a diagnostic override. The page also exposes
`window.zigbaDebug` (`Emulator` and `acquireSaveLock`) for console use and
browser tests.

Recording defaults to three snapshots, 16 MiB of snapshot payload, 4 MiB of
estimated history and 4,096 operations. Guest faults retain evidence; when
recording is disabled, missing replay history is explicit. Scenario input
journals freeze an exportable prefix at 1,024 inputs; a new segment starts from
a portable state. Lab workers never write the player's battery save. Recorded
baselines establish regressions, not hardware correctness. Snapshot restore
resets the host resampler; exact continuous audio checks require cold boot.

Existing CLI formats remain unchanged:

```sh
node tools/run-scenario.mjs path/to/scenario.json /tmp/zigba-result
node tools/replay-capture.mjs path/to/failure.json path/to/scenario.json
```

## Storage and ownership

The `player.ts`, `services.ts`, `library.ts`, `storage.js` and `media.ts`
libraries own session lifecycle, workers, input/audio, serialized persistence
and browser save leases. Both interfaces use `zigba-local`; save ownership requires the
`zigba-save-playthrough:["<raw ROM SHA-256>","<playthrough ID>"]` lock. Quick slots use the
assets store with keys `quick-state:["<ROM hash>","<playthrough ID>",<1–10>]`.
Values are `{data, savedAt, cloudRevision?, accountId?}`; older bare state bytes
still load and are treated as never synced. Their outbox jobs use kind
`quick/<slot>` in the existing `cloud` store, so no IndexedDB upgrade is needed.

Unchanged battery bytes are not rewritten. Autosave runs every five seconds and
on pause, storing both battery data and an exact-ROM machine checkpoint with
the boot/media options required to restore it. IndexedDB's `resumes` store retains
the latest and previous generation per run; `games.resumeSavedAt` projects the
active playthrough's availability
without loading snapshot blobs. Quick slots and in-memory rewind are separate.
If the latest checkpoint is corrupt or incompatible, resume tries the previous
generation once. If neither restores, progress stays preserved and autosave and
playback are blocked until retry succeeds or the user explicitly chooses
**Start from in-game save**. Read-only tabs may resume but cannot persist progress.

Browser termination before transaction completion or storage eviction
can lose progress. Export your saves; an unavailable-storage indicator is not a
successful save receipt. Opening another tab does not transfer unsaved state.

## Verify

```sh
npm run check -w frontend
npm test -w frontend
zig build web
ZIGBA_TEST_PORT=8890 npm run test:browser -w frontend
```

`zig build test-frontend` and `zig build test-browser` run these together with
the [`@evenfall/gba-pokemon`](../games/pokemon/README.md#tests) suites. The browser
suite uses system Chromium when available, or
`PLAYWRIGHT_CHROMIUM_EXECUTABLE`. Otherwise install it with
`npx playwright install chromium`. The suite runs the new console, isolated
developer UI, and the `/validation.html` SDK harness.

Some unit and browser tests skip unless `ZIGBA_*` environment variables point
at ROMs and states you supply locally; they verify fixture hashes before use.
No user asset is committed. These checks do not qualify retail-game compatibility
or physical controllers/mobile devices. Broader core/SDK checks remain documented
in [web runtime](../docs/adr/web-runtime.md).
