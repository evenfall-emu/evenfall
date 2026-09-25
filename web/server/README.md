# ZiGBA catalog and account server

This package is the backend of the hosted ZiGBA player: a metadata-only game
catalog, optional Discord and magic-link accounts, and private per-account
cloud backups of game progress. The player works without it; sign-in and
cloud backups report themselves unavailable when their bindings or secrets
are missing.

The server stores game, release-version, base-ROM identity, source-provenance
and optional external image-URL metadata, authentication records, and
compressed save states and battery saves. It does **not** host ROMs, BIOS
images, patches or image files. Signing in does not grant catalog management
rights.

The Worker uses [Hono](https://hono.dev/), Drizzle's Cloudflare D1 adapter,
[Better Auth](https://www.better-auth.com/) 1.7.5 and an R2 bucket. One
Worker serves the whole site: `/api/*` and the player's page URLs
(`/library`, `/library/<game ID>`, `/player`) run through this package
first, the pages answering with the player's `index.html`, and every other
path is served from the player's Static Assets in `.dist/cloudflare` (see
[`frontend/README.md`](../frontend/README.md#pages-and-navigation)).

| Path | Contents |
|---|---|
| [`src/index.ts`](src/index.ts) | Worker entry, catalog routes, management guard, auth mounting |
| [`src/catalog.ts`](src/catalog.ts), [`src/schema.ts`](src/schema.ts), [`src/seed.ts`](src/seed.ts) | Catalog queries, Drizzle schema, seed data |
| [`src/auth.ts`](src/auth.ts), [`src/auth-schema.ts`](src/auth-schema.ts) | Better Auth configuration and generated tables |
| [`src/cloud-saves.ts`](src/cloud-saves.ts) | Cloud progress, quick slots, account deletion and the cleanup cron |
| [`migrations/`](migrations/) | Generated D1 migrations |
| [`wrangler.jsonc`](wrangler.jsonc) | Local-development Wrangler configuration |

## Data model

- `games`: stable ID, unique internal name, display name, optional summary,
  HTTP(S) image URL and franchise.
- `game_tags`: a game's tags, unique per game ignoring case, in display order.
- `versions`: stable ID, owning game, unique per-game internal name, globally
  unique exact **playable output ROM SHA-1**, display label, optional
  required base, and optional source repository and revision, release date
  (`YYYY-MM-DD`), release notes and hosted patch. A patch is an HTTP(S) URL,
  a format (`bps`, `ups` or `ips`) and a size in bytes, and needs a required
  base; the base SHA-1 in and the output SHA-1 out verify it, so it carries
  no digest of its own. A game can have many versions.
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

| Method | Route | Response |
|---|---|---|
| `GET` | `/api/games` | `{ games: GameSummary[] }`, including `versionCount`, `tags` and `cloudPlaythroughs` |
| `GET` | `/api/games/:id` | `{ game }`, including `tags`, `cloudPlaythroughs`, versions and their required bases |
| `GET` | `/api/versions` | `{ versions }` with required-base metadata, `patch` and `saveCompatibleFrom` |
| `GET` | `/api/versions/:id` | `{ version }` |
| `GET` | `/api/bases` | `{ bases }`, each with its `sha1` and `crc32` |
| `GET` | `/api/bases/:id` | `{ base }` |

Management is deliberately small: `PUT` and `DELETE` at
`/api/admin/games/:id`, `/api/admin/versions/:id` and `/api/admin/bases/:id`,
plus `POST /api/admin/seed`. `PUT` is a validated, full-record upsert that
also replaces a game's `tags` or a version's `saveCompatibleFrom`, atomically.
Bodies are strict JSON; unknown fields, malformed IDs, non-HTTP(S) URLs, bad
hashes and dates, a patch without a base, save compatibility with the same
version or another game's, and broken foreign-key relations are rejected.

`cloudPlaythroughs` counts the cloud-backup playthroughs, across all
accounts, whose ROM SHA-1 is one of the game's version outputs. It is the
catalog's measure of popularity and identifies no account.

Every management request requires `Authorization: Bearer <ADMIN_SECRET>`.
Management fails closed with `503` when the Worker secret is absent. A
request carrying an `Origin` other than the request URL's origin is rejected
even with a valid token; requests without `Origin` remain usable from
command-line tooling. No mutable catalog route is public. Seeding is
explicit: ordinary reads and Worker startup never seed.

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
  `ZiGBA <EMAIL_FROM>` through the Cloudflare Email Sending binding, which is
  restricted to that sender. Without both the `EMAIL` binding and
  `EMAIL_FROM`, email sign-in returns `503 email_unavailable`.
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

| Header | Meaning |
|---|---|
| `X-Zigba-Revision` | UUID of this revision |
| `X-Zigba-Captured-At` | Capture time, epoch milliseconds |
| `X-Zigba-Digest` | SHA-256 of the compressed body |
| `X-Zigba-Title` | Optional private game label, `encodeURIComponent` text of at most 160 code points without control characters |
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
- Each account may make 120 `PUT` attempts and upload 20 MiB of compressed
  bodies per hour, counting retries and quick slots. The windows are fixed,
  start with the first request, and are consumed by rejected work too.

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
```

`server/wrangler.jsonc` is a development-only configuration. Its D1 database
ID and R2 bucket name are placeholders: `wrangler dev` and `--local`
migrations simulate both under the ignored `server/.wrangler/` and never
contact a deployed database or bucket. Never run that configuration with
`--remote`; deployment uses the root `wrangler.jsonc`.

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
`server/.wrangler/tmp/email/*/email-text/`. Open the newest file and paste
its link into the same browser within five minutes. No real inbox is
contacted and no Cloudflare email quota is used. Keep `remote: true` off the
email binding for routine tests, and do not share captured links.

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
quotas and account erasure. Only the external services are mocked: no real
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

[`src/seed.ts`](src/seed.ts) is explicit and idempotent. It seeds two games
and the exact releases that have registered profiles in
[`@evenfall/gba-pokemon`](../games/pokemon/README.md):

| Version | Output ROM SHA-1 | Source revision |
|---|---|---|
| Heart & Soul 1.2.1 | `a4be81f8330d7f223a3ab847c3da822fe6f8a2e0` | `PokemonHnS-Development/pokemonHnS@4aa8fa894db4c890b3d1100abd6aab50f44c22b1` |
| Elite Redux 2.65 Beta2 (1048) | `f1323126bc043f3ce931bcc670957930559eb515` | `Elite-Redux/eliteredux-source@06da575972cfed20d471f8bb5b90ea9e4736b16a` |

Both require the `pokemon-emerald-us` base, No-Intro "Pokemon - Emerald
Version (USA, Europe)": SHA-1 `f3ae088181bf583e55daf962a92bb46f4f1d07b7`,
CRC32 `1f1c08fb`.

These are raw playable ROM identities, not release ZIP or patch hashes. No
distributable catalog images exist for them, so both `imageUrl` values are
null.

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

Catalog management stays disabled unless the `ADMIN_SECRET` Worker secret is
deliberately set, never as a Wrangler variable or a checked-in file:

```sh
npx wrangler secret put ADMIN_SECRET --config wrangler.jsonc
```

An authorized operator may then call `POST /api/admin/seed` once; seeding is
not part of a deployment.

The D1 location hint and the EU jurisdiction of the R2 bucket do not make
processing EU-only: Workers, D1, email and logs are not covered by the bucket
jurisdiction, so do not promise EU-only processing from this configuration.
The official instance's data practices are described in
[`PRIVACY.md`](../PRIVACY.md).

### Self-hosting

Replace every instance-specific value in the root `wrangler.jsonc` and the
build environment before the first deploy:

| Value | Where | Notes |
|---|---|---|
| Worker name | `name` | Any unused Worker name in your account. |
| Domain | `routes[].pattern` | A custom domain on a zone in your account. Keep `workers_dev` and `preview_urls` off. |
| Auth origin | `vars.BETTER_AUTH_URL` | Exactly `https://<your domain>`: an HTTPS origin with no path or trailing slash. |
| Discord client ID | `vars.DISCORD_CLIENT_ID` | Your Discord application; register `https://<your domain>/api/auth/callback/discord`. |
| Email sender | `vars.EMAIL_FROM`, `send_email[].allowed_sender_addresses` | An address on a domain onboarded to Cloudflare Email Sending. Remove both to disable email sign-in. |
| D1 database | `d1_databases[].database_name`, `database_id` | Create with `wrangler d1 create <name>`; update the name in the `db:migrate:remote` script if it differs. |
| R2 bucket | `r2_buckets[].bucket_name` | Create with `wrangler r2 bucket create <name> --jurisdiction eu`; keep public access off. |
| Analytics | `VITE_POSTHOG_KEY`, `VITE_POSTHOG_HOST`, `VITE_POSTHOG_HOSTNAME` | Build-time; `tools/build-cloudflare.sh` defaults to the official instance. Export your own or empty values. |
| Privacy notice | `PRIVACY.md`, `VITE_PRIVACY_URL` | Published as `/privacy.txt`. Replace it with your notice, or point `VITE_PRIVACY_URL` elsewhere. |

Then, with Wrangler authenticated against your account:

1. Set the required secrets `BETTER_AUTH_SECRET` (at least 32 random bytes,
   for example `openssl rand -hex 32`) and `DISCORD_CLIENT_SECRET` with
   `npx wrangler secret put <NAME> --config wrangler.jsonc`.
2. Apply all migrations with `npm run db:migrate:remote -w server`.
3. Build and deploy with the two commands above.

Keep the five-minute cron trigger: it finishes account deletions and removes
late uploads (see [account deletion](#account-deletion)).

## Open questions

- Individual cloud playthroughs cannot be deleted; only whole-account
  deletion removes cloud data.
- Tombstone retention and the sweep cadence are coupled; there is no policy
  for retiring tombstones.
