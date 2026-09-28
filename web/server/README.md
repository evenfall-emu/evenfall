# ZiGBA catalog and account server

This package is the backend of the hosted ZiGBA player: a metadata-only game
catalog, optional Discord and magic-link accounts, and private per-account
cloud backups of game progress. The player works without it; sign-in and
cloud backups report themselves unavailable when their bindings or secrets
are missing.

The server stores game, release-version, base-ROM identity, source-provenance
and optional external image-URL metadata, authentication records, and
compressed save states and battery saves. It does **not** serve ROMs, BIOS
images, patches or image files: public patch URLs refer to the separate
`zigba-patches` Worker on `patches.evenfall-emu.org`, backed by the
`evenfall-patches` R2 bucket. The catalog Worker uploads patches to that
bucket, but never serves them. Signing in alone does not grant catalog
management rights.

The Worker uses [Hono](https://hono.dev/), Drizzle's Cloudflare D1 adapter,
[Better Auth](https://www.better-auth.com/) 1.7.5 and an R2 bucket. One
Worker serves the whole site: `/api/*` and the player's page URLs
(`/library`, `/library/<game ID>`, `/player`, `/share/state/<ID>`) run
through this package
first, the pages answering with the player's `index.html`, and every other
path is served from the player's Static Assets in `.dist/cloudflare` (see
[`frontend/README.md`](../frontend/README.md#pages-and-navigation)).

| Path                                                                                               | Contents                                                                         |
| -------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------- |
| [`src/index.ts`](src/index.ts)                                                                     | Worker entry, catalog routes, management guard, auth mounting                    |
| [`src/catalog.ts`](src/catalog.ts), [`src/schema.ts`](src/schema.ts), [`src/seed.ts`](src/seed.ts) | Catalog queries, Drizzle schema, seed data                                       |
| [`src/auth.ts`](src/auth.ts), [`src/auth-schema.ts`](src/auth-schema.ts)                           | Better Auth configuration and generated tables                                   |
| [`src/cloud-saves.ts`](src/cloud-saves.ts)                                                         | Cloud progress, quick slots, state shares, account deletion and the cleanup cron |
| [`migrations/`](migrations/)                                                                       | Generated D1 migrations                                                          |
| [`wrangler.jsonc`](wrangler.jsonc)                                                                 | Local-development Wrangler configuration                                         |

## Data model

- `games`: stable ID, unique internal name, display name, optional short
  `summary` hook, longer `description`, HTTP(S) image URL and franchise.
- `game_tags`: a game's tags, unique per game ignoring case, in display order.
- `versions`: stable ID, owning game, unique per-game internal name, globally
  unique exact **playable output ROM SHA-1**, display label, optional
  required base, creator project or release page, source revision, release
  date (`YYYY-MM-DD`), release notes and hosted patch. A patch is an HTTP(S) URL,
  a format (`bps`, `ups` or `ips`) and its exact size in bytes, and needs a
  required base; the base SHA-1 in and the output SHA-1 out verify it, so it
  carries no digest of its own. The player downloads patches from the
  browser, so their host must allow cross-origin `GET`. A game can have many
  versions. Each version also has a visibility level: `featured` (reserved for
  future placement) and `normal` (reviewed) appear in the player by default;
  `experimental` requires a device-side Library opt-in; `hidden` is absent
  from public catalog reads. An omitted level on a management write defaults
  to `experimental`.
- `version_save_compatibility`: for a version, the earlier versions of the
  same game whose in-game saves are verified to load in it.
- `bases`: stable ID, unique internal name, display label, and the unique
  **SHA-1** and indexed **CRC32** of one No-Intro dump, as its DAT lists them.
  BPS and UPS patch footers name their source ROM by CRC32; the SHA-1 decides
  between dumps that share a CRC32.

Foreign keys relate versions to games and required bases, tags to games,
and save compatibility to versions; deleting a game or version removes its
rows there. Hash formatting
(40 lowercase hex digits for SHA-1, 8 for CRC32) is checked both at the HTTP
boundary and in SQLite. Game assets are identified by SHA-1 throughout ZiGBA,
matching No-Intro, so a catalog hash compares directly with a player's ROM
hash and with a DAT entry.

## Catalog API

All responses are JSON except successful deletes (`204`). IDs use lowercase
letters and digits with `-` or `.` separators; separators cannot lead, trail
or repeat.

| Method | Route               | Response                                                                             |
| ------ | ------------------- | ------------------------------------------------------------------------------------ |
| `GET`  | `/api/games`        | `{ games: GameSummary[] }`, including `versionCount`, `tags` and `cloudPlaythroughs` |
| `GET`  | `/api/games/:id`    | `{ game }`, including `tags`, `cloudPlaythroughs`, versions and their required bases |
| `GET`  | `/api/versions`     | `{ versions }` with required-base metadata, `patch` and `saveCompatibleFrom`         |
| `GET`  | `/api/versions/:id` | `{ version }`                                                                        |
| `GET`  | `/api/bases`        | `{ bases }`, each with its `sha1` and `crc32`                                        |
| `GET`  | `/api/bases/:id`    | `{ base }`                                                                           |

Public game and version reads include only releases with a hosted patch and
exclude `hidden`, including direct ID reads and save-compatibility IDs. A
game with no qualifying release returns `404` by ID and is absent from the
list. Experimental releases remain in the public API for the player to show
after opt-in; the setting is not access control. Management writes can still
hold patchless or hidden records.

The admin API has the same `/api/admin/games`, `/api/admin/versions` and
`/api/admin/bases` collections. `GET` lists every record, including hidden,
patchless and zero-version games; `GET /:id` returns one, including all
versions of a game. `PUT /:id` creates or fully replaces a record;
`PATCH /:id` edits only supplied fields and returns `404` for a missing
record; `DELETE /:id` removes it (`204`). `PUT` and `PATCH` also replace a
game's `tags` or a version's `saveCompatibleFrom` when supplied. Bodies are
strict JSON; unknown fields, malformed IDs, non-HTTP(S) URLs, bad hashes and
dates, a patch without a base, save compatibility with the same version or
another game's, and broken foreign-key relations are rejected. Deleting a
game cascades to its versions; deleted versions' patch objects remain in R2
because content-addressed keys may be shared.

`PUT /api/admin/versions/:id/patch` attaches or replaces the patch for an
existing version with a required base. Send raw bytes (`Content-Type:
application/octet-stream`) and `X-Patch-Format: bps`, `ups` or `ips`. The
body must have the matching format header and be at most 40 MiB. The Worker
computes its SHA-1, stores `<sha1>.<format>` in the `PATCHES` bucket, and sets
the version's patch URL, format and size from the uploaded bytes. A repeat
upload is safe. The public patch CDN remains read-only. The upload checks the
header, **not** that the patch applies to the declared base or produces
`outputSha1`: verify those with your legally supplied base before publishing.
The player checks both the base SHA-1 and patched output SHA-1 on install.
Keep new versions `hidden` until the patch has been independently verified;
`experimental` is still publicly readable.

For another Elite Redux release, `PUT` its metadata with `gameId:
"elite-redux"`, a new ID, exact playable `outputSha1`, `requiredBaseId:
"pokemon-emerald-us"`, `patch: null` and `visibility: "hidden"`, along with
the other required fields in [`versionInput`](src/types.ts). Upload the
verified patch, then `PATCH` the version's visibility. The catalog does not
register bridge manifests: runtime recognition still needs its own reviewed
profile or successful locator pass. Admin changes live in D1, not in seed
JSON; running `POST /api/admin/seed` again overwrites seeded IDs.

For an existing hidden version with verified output metadata, a bearer-token
operator can upload and publish its patch like this (replace the origin, ID
and file with the intended release):

```sh
curl -X PUT "$ORIGIN/api/admin/versions/<id>/patch" \
  -H "Authorization: Bearer $ADMIN_SECRET" \
  -H 'Content-Type: application/octet-stream' -H 'X-Patch-Format: ups' \
  --data-binary @verified.ups
curl -X PATCH "$ORIGIN/api/admin/versions/<id>" \
  -H "Authorization: Bearer $ADMIN_SECRET" \
  -H 'Content-Type: application/json' -d '{"visibility":"experimental"}'
```

`cloudPlaythroughs` counts the cloud-backup playthroughs, across all
accounts, whose ROM SHA-1 is one of the game's public version outputs. It is the
catalog's measure of popularity and identifies no account.

Every management request requires either `Authorization: Bearer
<ADMIN_SECRET>` or a current Better Auth session whose account has
`account_limits.admin = 1` (see [including the ROM](#including-the-rom)).
Revoking the flag or starting account deletion removes access immediately.
Management fails closed with `503` when neither auth nor the operator secret
is configured. Session-based writes require the exact configured `Origin`;
bearer-token writes accept no `Origin` for command-line tooling, but reject a
foreign one. Admin responses use `Cache-Control: no-store`. No mutable catalog
route is public. Seeding is explicit: ordinary reads and Worker startup never
seed.

## Accounts

Better Auth handles `/api/auth/*` on the same origin. `user`, `account`,
`session`, `verification` and `rate_limit` are generated Drizzle tables in
D1, separate from catalog metadata.

- **Discord** requests only `identify email` and requires a verified email
  address. Accounts with the same verified email can link; linking accounts
  with different emails is disabled. OAuth tokens are encrypted with the auth
  secret.
- **Magic links** expire after five minutes, are stored hashed in D1, and are
  single-use, including under concurrent verification. Mail is sent as
  `Evenfall <EMAIL_FROM>` with subject `Sign in to Evenfall Player` through the
  Cloudflare Email Sending binding, which is restricted to that sender.
  The message includes plain text and branded HTML with a sign-in button and
  copyable link. It uses inline styles, presentation tables and system fonts.
  The canonical Evenfall logo and wordmark travel as an inline PNG attachment
  referenced by Content-ID, not a remote image or unsupported SVG. The PNG
  includes a navy background so its cream wordmark stays legible if the
  client changes the surrounding colors. Clients that suppress attachments
  still have the image's alt text, live heading, button and plain-text version.
  The URL is HTML-escaped, never shortened or wrapped in tracking links.
  Without both the `EMAIL` binding and `EMAIL_FROM`, email sign-in returns
  `503 email_unavailable`.
- **Rate limits** are stored in D1: three magic-link requests per IP per
  minute, and per recipient (normalized case-insensitively) three per 15
  minutes and ten per day, enforced atomically. These are fixed windows that
  begin with the first request, not sliding windows, and a suppressed request
  can still consume the other window's budget. Recipient throttling and
  delivery failures return the same generic success.
- **Sessions** last seven days, refresh after one day, and use host-only
  `HttpOnly`, `SameSite=Lax` cookies, `Secure` over HTTPS. There is no
  client-side session cache; sign-out revokes the database session.
- **Origins.** Unsafe requests require the exact configured origin, and
  callback destinations are checked before any email is sent. Auth responses
  carry `Cache-Control: no-store` and `Referrer-Policy: no-referrer`; cloud
  save responses are `no-store` too.

Configuration fails closed. `BETTER_AUTH_SECRET` must contain at least 32
characters; generate at least 32 random bytes. `BETTER_AUTH_URL` must be an
exact HTTPS origin (no path or trailing slash) or the local origin
`http://127.0.0.1:5173`. Anything else, including an unset value, disables
sign-in and cloud saves with `503`. Each instance trusts only its own
configured origin.

## Cloud progress

### Save objects

`GET`, `HEAD` and `PUT /api/saves/:hash/:playthroughId/:kind` address one
backup: `hash` is the raw playable ROM SHA-1, `playthroughId` a UUID or the
reserved `default`, and `kind` either `resume` (a save state) or `battery` (an
in-game save). [Quick slots](#quick-slots) use `.../quick/:slot`.

- Better Auth verifies the session on every request, and every request must
  send `X-Zigba-Account` matching it. The server derives the R2 key
  `<user>/<hash>/<playthroughId>/<kind>` from the authenticated user ID, so a
  client can never name another account's object.
- For `default`, an existing object at `<user>/<hash>/<kind>` (without the
  playthrough segment) is read and replaced in place; otherwise the
  four-segment key is used.
- There are no public bucket URLs: keep `r2.dev` access disabled and attach
  no custom domain to the bucket.

`PUT` requires the exact configured `Origin`, `Content-Type:
application/octet-stream`, and gzip bytes sent without an HTTP
`Content-Encoding`. Headers identify the upload:

| Header                     | Meaning                                                                                                         |
| -------------------------- | --------------------------------------------------------------------------------------------------------------- |
| `X-Zigba-Revision`         | UUID of this revision                                                                                           |
| `X-Zigba-Captured-At`      | Capture time, epoch milliseconds                                                                                |
| `X-Zigba-Digest`           | SHA-256 of the compressed body                                                                                  |
| `X-Zigba-Title`            | Optional private game label, `encodeURIComponent` text of at most 160 code points without control characters    |
| `X-Zigba-Playthrough-Name` | Optional playthrough name, same encoding and limit; defaults to `Playthrough` (`First adventure` for `default`) |

Create with `If-None-Match: *`; replace only with a known quoted `If-Match`
ETag. An atomic R2 conditional write rejects stale versions with `412`.
Retrying the same revision with the same digest is idempotent; reusing a
revision with different bytes is `409 revision_conflict`. `GET` and `HEAD`
return the metadata headers plus `ETag` and `X-Zigba-Uploaded-At`, the
server-confirmed upload time; `PUT` returns `{etag, revision, uploadedAt}`.
Replacing an object without a title keeps its stored title. The title is
private to the account and never exposed through the catalog.

### Discovery

`GET /api/saves?cursor=...` lists the account's resume backups, with the same
authentication, account header and `no-store` rules. It returns
`{saves, cursor, usage: {playthroughs, limit: 100}}`. Each entry has the ROM
`hash`, `playthroughId`, private `playthroughName`, a `title`, `capturedAt`
in epoch milliseconds and ISO `uploadedAt` from R2. Titles come from the
catalog's exact ROM hashes first, then the private upload title, then a hash
prefix.

Each page lists at most 100 R2 objects and filters out battery saves, so a
page can contain no resumes and still return a continuation cursor. Clients
must read every page before choosing the latest upload, because R2 key order
is not upload order. No snapshot bodies, ROM bytes, bucket URLs or other
accounts' keys are returned.

### Playthrough quota

An account can register 100 playthroughs across all games. D1 atomically
admits the first valid `PUT` for each `(account, hash, playthroughId)` while
the account is under the limit; resume, battery and quick-slot objects of one
run share its registration, and updates to a registered run always work. The
101st run receives `409 {error: {code: "playthrough_limit", message}}`. A
validated attempt can register a run even if its conditional R2 write then
fails. Runs that never upload consume nothing, and device-local saves are
independent of sync errors.

`usage.playthroughs` counts registered runs, including battery-only ones. An
object stored at the three-segment `default` key counts only after a `PUT`
through the playthrough route registers it, so `usage` can undercount on
instances that hold such objects.

`DELETE /api/saves/:hash/:playthroughId` requires a current cookie session,
a matching `X-Zigba-Account`, and an exact same-origin `Origin`. It removes
the run's resume, battery and quick-slot objects, including legacy resume and
battery keys for `default`, then frees its D1 playthrough registration. It
returns `204` even if the backup is already absent. Other runs and accounts
are untouched. This removes the current cloud copy, not local copies on other
devices; later uploads from those devices can create a backup again.

### Quick slots

`GET`, `HEAD` and `PUT /api/saves/:hash/:playthroughId/quick/:slot` store one
save state per numbered slot under `<user>/<hash>/<playthroughId>/quick/<slot>`.
Authentication, `X-Zigba-Account`, `Origin`, gzip bodies, metadata headers,
conditional writes, idempotent retries, `409 revision_conflict`, `412` and the
upload budgets are exactly those of `resume`. Decoded bytes must start with
the state magic and fit 2 MiB; otherwise the `PUT` fails with
`400 invalid_quick` or `413 decoded_too_large`. Quick slots never consume
additional playthrough quota, and `GET /api/saves` does not list them.

`GET /api/saves/:hash/:playthroughId/quick` returns
`{slots: [{slot, revision, digest, capturedAt, etag, uploadedAt}], limit}` for
the authenticated account only, sorted by slot and without state bodies.
Clients use it to discover a run's slots on a new device and to notice a slot
replaced elsewhere. Invalid hashes, IDs or slot numbers (`0`, leading zeros,
more than two digits) are `400 invalid_save_key`.

Slots are numbered `1..limit` per playthrough. The default limit is 10
(`QUICK_SLOT_DEFAULT` in [`src/cloud-saves.ts`](src/cloud-saves.ts)). An
operator can override one account in D1:

```sql
INSERT INTO account_limits (user_id, quick_slots) VALUES ('<user id>', 20)
  ON CONFLICT(user_id) DO UPDATE SET quick_slots = excluded.quick_slots;
```

`quick_slots` accepts 0-99 (0 disables quick-slot uploads); `NULL` or no row
uses the default. There is no HTTP route for this. A `PUT` beyond the limit
returns `409 {error: {code: "quick_slot_limit", message, limit}}` before
consuming upload budget. Lowering a limit keeps higher existing slots
readable and listed, but they cannot be replaced.

### Limits

- Compressed bodies, and decoded resume and quick-slot states, are limited to
  2 MiB. Decoded states must start with the ZiGBA state magic; this screens
  the format but does not validate the whole state or its content.
- Battery saves must have a supported save size (512 B, 8, 32, 64 or
  128 KiB).
- Each account may make 120 upload attempts and upload 20 MiB of compressed
  bodies per hour, counting retries, quick slots and
  [state shares](#state-shares). The windows are fixed, start with the first
  request, and are consumed by rejected work too.

### State shares

A signed-in account can publish one save state as a link that anyone can
open, without an account, for 24 hours. The player's side is described in
[`frontend/README.md`](../frontend/README.md#sharing-a-state).

`POST /api/shares` requires the session, a matching `X-Zigba-Account`, the
exact configured `Origin` and a gzip state body, exactly as a `resume`
upload does, plus:

| Header                     | Meaning                                          |
| -------------------------- | ------------------------------------------------ |
| `X-Zigba-Rom`              | Raw playable ROM SHA-1                           |
| `X-Zigba-Digest`           | SHA-256 of the compressed body                   |
| `X-Zigba-Playthrough-Name` | Required playthrough name, encoded like a save's |
| `X-Zigba-Title`            | Optional game label, encoded like a save's       |

It returns `201 {id, url, canIncludeRom}`. `id` is 128 random bits as 22
base64url characters and `url` is `<configured origin>/share/state/<id>`.
`canIncludeRom` is true for [admin accounts](#including-the-rom). Every
request creates a new share, even for the same bytes. Shares consume the
account's [upload budget](#limits) but no playthrough quota.

`GET /api/shares/:id` is public and `no-store`. It returns the compressed
body with `X-Zigba-Rom`, `X-Zigba-Digest` and URI-encoded `X-Zigba-Title`
and `X-Zigba-Playthrough-Name`, plus `X-Zigba-Rom-Included: 1` when the ROM
can be downloaded with it. The title is
resolved like a listing's: the catalog's name for an exact release first,
then the uploaded label, then a hash prefix. The response never identifies
the account. Unknown, malformed and deleted IDs, and shares whose account is
being deleted, all return `404 share_not_found`.

#### Including the ROM

An admin account can attach the game's ROM to its own share, so the link
starts on a device without the game. No other account can. The operator
grants admin in D1; there is no HTTP route for it:

```sql
INSERT INTO account_limits (user_id, admin)
  SELECT id, 1 FROM user WHERE email = '<email>'
  ON CONFLICT(user_id) DO UPDATE SET admin = 1;
```

Setting `admin = 0` withdraws every ROM the account attached at once:
downloads return `404` and shares stop announcing them. It also withdraws
catalog management access; grant the flag only to trusted operators.

`PUT /api/shares/:id/rom` requires the same session, `X-Zigba-Account`,
`Origin` and content type as a share, an `X-Zigba-Digest` of the compressed
body, and a share the account created. Non-admins get
`403 rom_sharing_forbidden`; another account's, expired or unknown share is
`404 share_not_found`. The gzip body may be at most 33 MiB (a GBA ROM is at
most 32 MiB), else `413 compressed_too_large`. It costs one upload attempt
but no upload bytes. R2 checks the digest (`400 digest_mismatch`); the
Worker neither decodes nor hashes the ROM, so readers must check its SHA-1
against the share's `X-Zigba-Rom`, as the player does. It returns `204`, and
a repeated `PUT` replaces the ROM.

`GET /api/shares/:id/rom` is public, `no-store` and never identifies the
account. It returns the compressed ROM with `X-Zigba-Digest` while its share
is readable and its account is still an admin, otherwise
`404 rom_not_found`.

Shares are stored at `share/state/<id>` in the `SAVES` bucket, and included
ROMs at `share/rom/<id>`, outside every account prefix, with the owner's ID
in private object metadata. The Worker never checks a share's age: an R2
lifecycle rule on the `share/` prefix deletes both (see
[self-hosting](#self-hosting)). R2 applies such rules lazily,
typically within a day after the age limit, so a 1-day rule keeps a link
working for at least 24 hours, the lifetime the player promises, and usually
less than 48.

To prevent abuse, an account can create at most 100 shares per fixed 24-hour
window (`SHARE_LIMIT` in [`src/cloud-saves.ts`](src/cloud-saves.ts)), counted
only for valid states. Past it, `POST` answers exactly like the exhausted
upload budget, `429 save_rate_limited`, so the player shows no separate
limit. It bounds new links rather than live ones; with the lifecycle rule's
lag, an account holds at most a few hundred.

### Account deletion

`DELETE /api/account` requires a current cookie session, a matching
`X-Zigba-Account` and the exact configured `Origin`.

1. It durably inserts an `account_deletions` tombstone (the opaque user ID
   and timestamps; no email or title), which closes new cloud admissions for
   that account.
2. It removes the user with its sessions and OAuth accounts, pending email
   links, account-scoped rate limits, playthrough registrations and
   `account_limits` row, and returns
   `202 {accountDeleted: true, cloudDeletion: "pending"}`. If this step
   fails, the route returns a retryable `503 deletion_pending` while the
   tombstone keeps the account gated.
3. An immediate best-effort sweep deletes every R2 key under the account
   prefix, across pages, including quick slots and any other layout.

[State shares](#state-shares) and their included ROMs are outside the
account prefix. The tombstone hides them from step 1 onwards, and the
`share/` lifecycle rule deletes them within about two days of their upload.

The five-minute cron trigger repairs a crash between steps 1 and 2 and
retries R2 failures. It sweeps each tombstone every five minutes for the
first hour and daily afterwards. Tombstones are never retired, even after an
empty sweep, because an R2 `PUT` issued before deletion can complete later;
the next sweep removes it. Review the cron cadence and retention together
before changing either. New Better Auth users receive fresh IDs, never an
erased prefix.

R2 and D1 are not transactional together, so cloud erasure is eventually
consistent: the `202` does not claim that all R2 data is gone, and if the
cron stops, pending objects remain until it runs again. Device-local data is
untouched.

The player's side of these contracts (default-on backups, the opt-out,
offline coalescing and quick-slot reconciliation) is described in
[`frontend/README.md`](../frontend/README.md).

## Local development

From the repository root:

```sh
npm ci
npm run check -w server
npm test -w server
npm run db:migrate:local -w server
npm run dev -w server          # Wrangler on http://127.0.0.1:8787
npx wrangler dev --config server/patches.wrangler.jsonc --port 8788 # optional local patch CDN
```

`server/wrangler.jsonc` is a development-only configuration. Its D1 database
ID and R2 bucket name are placeholders: `wrangler dev` and `--local`
migrations simulate both under the ignored `server/.wrangler/` and never
contact a deployed database or bucket. Never run that configuration with
`--remote`; deployment uses the root `wrangler.jsonc`. The local `PATCHES_URL`
points to port 8788; the patch CDN must use the same local R2 persistence
directory to read back uploaded patches. Neither local Worker uses the
deployed bucket unless explicitly run with `--remote`.

For login, open exactly `http://127.0.0.1:5173`, not `localhost`, port 8787
or the static Python server. Vite proxies `/api` to Wrangler on port 8787, so
cookies and OAuth callbacks stay same-origin. Start Vite in a second terminal
with `npm run dev -w frontend`, after `zig build web-sdk` if the SDK's WASM
has not been built.

Put local secrets in the ignored `server/.dev.vars`, never in a `VITE_`
variable, and restart Wrangler after changing it. Never reuse a local auth
secret in production.

```dotenv
BETTER_AUTH_SECRET=<independent local value, e.g. from openssl rand -hex 32>
DISCORD_CLIENT_SECRET=<your Discord application secret>
ADMIN_SECRET=<development-only catalog management secret>
```

To test Discord sign-in, use your own Discord application: set its client ID
as `DISCORD_CLIENT_ID` in `server/wrangler.jsonc`, and register this redirect
in the Discord Developer Portal under OAuth2 > Redirects:

```text
http://127.0.0.1:5173/api/auth/callback/discord
```

Open **Account** on Home or in the Player/Library header. Discord uses the
real provider, so completing its consent screen is a manual test. Email is
simulated: Wrangler prints capture paths, and text messages appear under
`server/.wrangler/tmp/email/*/email-text/`; HTML messages appear under
`server/.wrangler/tmp/email/*/email-html/`, with the inline PNG captured under
`server/.wrangler/tmp/email/*/email-attachment/`. A standalone HTML capture
does not resolve `cid:` references: map `cid:evenfall-logo` to its matching
attachment when previewing in a browser. Paste the text version's link into
the same browser within five minutes. Captures contain a live sign-in
credential: do not share them or use them for public screenshots. No real
inbox is contacted and no Cloudflare email quota is used. Keep `remote: true`
off the email binding for routine tests.

The small base64 PNG in [`src/email-brand.ts`](src/email-brand.ts) is bundled
with the Worker, so deployment does not depend on the gitignored brand kit.
To regenerate it from the checked-in SVG using the workspace's Playwright,
run from the repository root:

```sh
node tools/generate-email-brand.mjs
# Or use an installed browser:
CHROMIUM_EXECUTABLE_PATH=/usr/bin/chromium node tools/generate-email-brand.mjs
```

The template avoids remote images, font downloads and tracking, but cannot
guarantee inbox placement. For spam reports, inspect the received message's
authentication results for SPF, DKIM and DMARC alignment with the configured
sender domain, as well as Cloudflare's sending-domain status and reputation.
Branding is not a substitute for domain authentication. Actual Gmail, Apple
Mail and Outlook rendering and inbox placement require inbox checks; a local
browser preview does not emulate their sanitizers or Outlook's Word renderer.

To load the catalog seed locally:

```sh
curl -X POST http://127.0.0.1:8787/api/admin/seed \
  -H 'Authorization: Bearer <development-secret>'
```

### Tests

The tests use disposable Miniflare D1 databases with the generated
migrations applied, and check native Workers responses under workerd. They
cover catalog reads and writes, idempotent seeding, validation, missing
bindings, auth configuration, cookies, concurrent single-use verification,
expiry, sign-out, origin and redirect rejection, concurrent limits and window
boundaries, email failures, Discord linking and OAuth state, cloud saves,
quotas, state shares and account erasure. Only the external services are mocked: no real
mail is sent and Discord is never contacted. Tests write no database or
credentials into the checkout.

Vitest and Drizzle Kit are development tools and are not bundled into the
Worker; do not expose their UI or development servers on public interfaces.

## Schema changes

After changing [`src/schema.ts`](src/schema.ts), generate and review a
migration:

```sh
npm run db:generate -w server
```

For a Better Auth or plugin upgrade, regenerate its schema first from the
binding-free generator configuration, inside `server/`:

```sh
npx --yes auth@1.7.5 generate --config ./auth-schema.config.ts \
  --adapter drizzle --dialect sqlite --output ./src/auth-schema.ts --yes
npm run db:generate
```

Never edit a migration that has been applied to a deployed database; add the
next one. Remote migrations are a separate, explicit step, never part of the
build or deploy:

```sh
npm run db:migrate:remote -w server
```

Apply them, review the result, and only then deploy code that depends on
them. Prefer additive migrations, so the deployed Worker stays valid between
the two steps.

## Seed data

[`src/seed.ts`](src/seed.ts) is explicit and idempotent. It includes only
patch-backed collected releases from the metadata-only
[`romhacks.json`](src/romhacks.json). Elite Redux keeps its existing catalog
identity and source revision. Of the releases with registered profiles in
[`@evenfall/gba-pokemon`](../games/pokemon/README.md), the catalog includes:

| Version                       | Output ROM SHA-1                           | Source revision                                                          |
| ----------------------------- | ------------------------------------------ | ------------------------------------------------------------------------ |
| Elite Redux 2.65 Beta2 (1048) | `f1323126bc043f3ce931bcc670957930559eb515` | `Elite-Redux/eliteredux-source@06da575972cfed20d471f8bb5b90ea9e4736b16a` |

It requires the `pokemon-emerald-us` base, No-Intro "Pokemon - Emerald
Version (USA, Europe)": SHA-1 `f3ae088181bf583e55daf962a92bb46f4f1d07b7`,
CRC32 `1f1c08fb`.

These are raw playable ROM identities, not release ZIP or patch hashes. The
romhack seed and CDN publication procedure are in
[`tools/catalog/README.md`](../tools/catalog/README.md#romhack-releases-and-patch-cdn).
Heart & Soul has no verified hosted patch and is not seeded. Existing
patchless rows in a database remain stored but are not returned publicly.
No distributable catalog images exist for these profiles, so their `imageUrl`
values are null.

## Deployment

The root [`wrangler.jsonc`](../wrangler.jsonc) describes the official hosted
instance. It contains no Cloudflare account ID: Wrangler takes the account
from `CLOUDFLARE_ACCOUNT_ID` (or from `wrangler login` when you have a single
account), and non-interactive deploys use a scoped `CLOUDFLARE_API_TOKEN`.
Keep tokens out of the repository.

```sh
bash tools/build-cloudflare.sh   # tests, then a ReleaseSafe build into .dist/cloudflare
npm run deploy -w frontend       # wrangler deploy --config ../wrangler.jsonc
```

The build script runs `npm ci`, type-checking and tests for the workspaces,
compiles the WASM core with a pinned Zig, and publishes only
`.dist/cloudflare`; see [`frontend/README.md`](../frontend/README.md). It
never migrates or seeds a database. A Git-connected Cloudflare Workers Builds
project can run the same two commands as its build and deploy steps.

For command-line bearer-token management, set the optional `ADMIN_SECRET`
Worker secret, never a Wrangler variable or a checked-in file:

```sh
npx wrangler secret put ADMIN_SECRET --config wrangler.jsonc
```

An authorized operator may call `POST /api/admin/seed` after the patch CDN is
populated and verified; seeding is not part of a deployment. The separate
patch Worker handles the patch-domain route. The catalog Worker also binds
the EU `evenfall-patches` bucket for admin uploads; its private `SAVES` bucket
is unchanged.

The D1 location hint and the EU jurisdiction of the R2 bucket do not make
processing EU-only: Workers, D1, email and logs are not covered by the bucket
jurisdiction, so do not promise EU-only processing from this configuration.
The official instance's data practices are described in
[`PRIVACY.md`](../PRIVACY.md).

### Self-hosting

Replace every instance-specific value in the root `wrangler.jsonc` and the
build environment before the first deploy:

| Value             | Where                                                            | Notes                                                                                                                         |
| ----------------- | ---------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| Worker name       | `name`                                                           | Any unused Worker name in your account.                                                                                       |
| Domain            | `routes[].pattern`                                               | A custom domain on a zone in your account. Keep `workers_dev` and `preview_urls` off.                                         |
| Auth origin       | `vars.BETTER_AUTH_URL`                                           | Exactly `https://<your domain>`: an HTTPS origin with no path or trailing slash.                                              |
| Discord client ID | `vars.DISCORD_CLIENT_ID`                                         | Your Discord application; register `https://<your domain>/api/auth/callback/discord`.                                         |
| Email sender      | `vars.EMAIL_FROM`, `send_email[].allowed_sender_addresses`       | An address on a domain onboarded to Cloudflare Email Sending. Remove both to disable email sign-in.                           |
| D1 database       | `d1_databases[].database_name`, `database_id`                    | Create with `wrangler d1 create <name>`; update the name in the `db:migrate:remote` script if it differs.                     |
| R2 bucket         | `r2_buckets[].bucket_name`                                       | Create with `wrangler r2 bucket create <name> --jurisdiction eu`; keep public access off. Add the share lifecycle rule below. |
| Patch bucket/CDN  | `PATCHES` R2 binding and `vars.PATCHES_URL`                      | Bind the patch CDN bucket for uploads and point the URL at its read-only HTTPS Worker origin.                                 |
| Analytics         | `VITE_POSTHOG_KEY`, `VITE_POSTHOG_HOST`, `VITE_POSTHOG_HOSTNAME` | Build-time; `tools/build-cloudflare.sh` defaults to the official instance. Export your own or empty values.                   |
| Privacy notice    | `PRIVACY.md`, `VITE_PRIVACY_URL`                                 | Published as `/privacy.txt`. Replace it with your notice, or point `VITE_PRIVACY_URL` elsewhere.                              |

Then, with Wrangler authenticated against your account:

1. Set the required secrets `BETTER_AUTH_SECRET` (at least 32 random bytes,
   for example `openssl rand -hex 32`) and `DISCORD_CLIENT_SECRET` with
   `npx wrangler secret put <NAME> --config wrangler.jsonc`.
2. Apply all migrations with `npm run db:migrate:remote -w server`.
3. Add the lifecycle rule that deletes [state shares](#state-shares):

   ```sh
   npx wrangler r2 bucket lifecycle add <bucket> share-expiry share/ \
     --expire-days 1 --jurisdiction eu
   ```

4. Build and deploy with the two commands above.

Keep the five-minute cron trigger: it finishes account deletions and removes
late uploads (see [account deletion](#account-deletion)).

## Open questions

- Tombstone retention and the sweep cadence are coupled; there is no policy
  for retiring tombstones.
- A state share cannot be revoked before it expires, except by deleting the
  account.
