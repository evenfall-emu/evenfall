# ZiGBA catalog and account server

This package hosts the metadata-only catalog API and optional Discord/magic-link
accounts for ZiGBA. It stores game, release-version, base-ROM identity,
source-provenance and optional external image URL metadata, authentication
records, and private per-account cloud progress backups.
It does **not** provide a ROM/BIOS/patch or image-blob hosting API. Signing in
does not grant catalog management rights.

The Worker uses [Hono](https://hono.dev/) and Drizzle's Cloudflare D1 adapter.
The root Worker serves `/api/*` through this package before Static Assets and
continues to serve the UI from `.dist/cloudflare` for all other paths.

## Data model

- `games`: stable ID, unique internal name, display name, optional summary and
  HTTP(S) image URL.
- `versions`: stable ID, owning game, unique per-game internal name, globally
  unique exact **playable output ROM SHA-256**, display label, optional required
  base, and optional source repository/revision. A game can have many versions.
- `bases`: stable ID, unique internal name, display label, and nullable unique
  exact **playable base ROM SHA-256**.

Foreign keys relate versions to games and required bases. Lowercase 64-digit
SHA-256 formatting is checked both at the HTTP boundary and in SQLite.

The seeded `Pokémon Emerald (US)` base deliberately has `sha256: null`. The
repository establishes the two supported patched-output hashes but does not
establish an exact playable Emerald base-ROM SHA-256. Guessing one, copying a
ZIP hash, or treating a label as proof would create false install matches. API
responses therefore report this base as `verified: false`; clients must never
match an install against a null base hash.

## HTTP API

All responses are JSON except successful deletes (`204`). IDs use lowercase
letters/digits with `-` or `.` separators; separators cannot lead, trail, or
repeat.

| Method | Route               | Contract                                                |
| ------ | ------------------- | ------------------------------------------------------- |
| `GET`  | `/api/games`        | `{ games: GameSummary[] }`, including `versionCount`    |
| `GET`  | `/api/games/:id`    | `{ game }`, including versions and their required bases |
| `GET`  | `/api/versions`     | `{ versions }` with required-base metadata              |
| `GET`  | `/api/versions/:id` | `{ version }`                                           |
| `GET`  | `/api/bases`        | `{ bases }`, each with derived `verified`               |
| `GET`  | `/api/bases/:id`    | `{ base }`                                              |

Management is intentionally small: `PUT` and `DELETE` at
`/api/admin/games/:id`, `/api/admin/versions/:id`, and
`/api/admin/bases/:id`, plus `POST /api/admin/seed`. `PUT` is a validated,
full-record upsert. Bodies are strict JSON; unknown fields, malformed IDs,
non-HTTP(S) URLs, bad hashes, and broken foreign-key relations are rejected.

Every management request requires `Authorization: Bearer <ADMIN_SECRET>`.
Management fails closed with `503` when the Worker secret is absent, and any
request carrying an `Origin` other than the request URL's origin is rejected
even when the bearer token is valid. Requests without `Origin` remain usable by
trusted command-line release tooling. No mutable catalog route is public.

## Account authentication

Better Auth 1.7.5 handles `/api/auth/*` through the existing same-origin Worker.
`user`, `account`, `session`, `verification`, and `rate_limit` are generated
Drizzle/D1 tables, separate from catalog metadata.

- Discord requests only `identify email` and requires a verified email address.
  Verified same-email accounts can link; different-email automatic linking is
  disabled. OAuth tokens are encrypted with the auth secret.
- Magic links expire after five minutes, are hashed in D1, and are single-use,
  including concurrent verification. Mail comes from `ZiGBA <EMAIL_FROM>`
  through the native Cloudflare Email Sending binding, restricted to that sender.
  Email sign-in returns `503 email_unavailable` without both the `EMAIL`
  binding and `EMAIL_FROM`.
- Database-backed request limits are three per IP per minute. Atomic recipient
  limits are three per 15 minutes and ten per day, normalized case-insensitively.
  These are fixed windows beginning with the first request, not sliding windows;
  suppressed requests can consume the other window's request budget.
  Recipient throttling and delivery failures return the same generic success.
- Sessions last seven days, refresh after one day, and use host-only `HttpOnly`,
  `SameSite=Lax` cookies (`Secure` in production). No client-side session cache
  is enabled; sign-out revokes the database session.
- Unsafe requests require the exact configured Origin. Callback destinations
  are checked before email delivery. All auth responses use `Cache-Control:
  no-store` and `Referrer-Policy: no-referrer`.

Secrets fail closed. `BETTER_AUTH_SECRET` must contain at least 32 characters;
generate at least 32 random bytes. `BETTER_AUTH_URL` must be an exact HTTPS
origin (no path or trailing slash) or the fixed local origin
`http://127.0.0.1:5173`; anything else, including an unset value, disables
sign-in and cloud saves with `503`. Each instance trusts only its own
configured origin.

## Private cloud progress

`GET`, `HEAD`, and `PUT /api/saves/:hash/:playthroughId/:kind` use the raw playable
ROM SHA-256, a UUID playthrough ID (or reserved `default`), and `resume` or
`battery`. [Quick slots](#cloud-quick-slots) use `.../quick/:slot` instead.
Better Auth verifies every session; every request must include
`X-Zigba-Account` matching that session, and the server derives the
private R2 key from the authenticated user ID. All responses are `no-store`.
There are no public bucket URLs: keep `r2.dev` access disabled and attach no
custom domain to the bucket. The deployment config binds `SAVES` to a bucket
with jurisdiction `eu`; local Wrangler simulates storage. EU R2 storage does
not establish EU-only processing for Workers, D1, email or logs.

`GET /api/saves?cursor=...` provides account-private resume discovery.
It returns `{saves, cursor, usage: {playthroughs, limit: 100}}`; usage counts
registered playthroughs, including battery-only runs. Each entry contains the
exact ROM `hash`, `playthroughId`, private `playthroughName`, catalog-resolved
`title` (hash-prefix fallback), `capturedAt` in epoch milliseconds, and ISO
`uploadedAt` from R2. The same authentication,
account header and `no-store` rules apply to this exact route. Each page lists
at most 100 R2 objects, filters out battery saves, and may therefore contain no
resumes while still returning a continuation cursor. Clients must consume all
pages before selecting the latest upload: R2 key order is not upload order.
No snapshot bodies, ROM bytes, bucket URLs or other accounts' keys are returned.
Legacy three-segment R2 backups appear as `default` with name `First adventure`;
GET/HEAD and new-format PUT to that ID use the legacy object when it exists.
Old three-segment HTTP PUT URLs are no longer accepted. A legacy run claims a
quota slot on its first new-format PUT, not on discovery. Thus `usage` can
underreport legacy-only backups until written; old backups are not proactively
backfilled. Installations with legacy data should account for this gap
before asserting an absolute 100-run historical cap.
Titles come from exact catalog ROM hashes, then optional private upload
metadata, then a hash prefix for older unknown ROMs.

PUT requires the exact configured Origin, `application/octet-stream`, and gzip
bytes without an HTTP `Content-Encoding`. Headers identify a UUID revision,
capture timestamp and compressed SHA-256 (`X-Zigba-Revision`,
`X-Zigba-Captured-At`, `X-Zigba-Digest`). Create with `If-None-Match: *`; replace
only with a known quoted `If-Match` ETag. An atomic R2 conditional write rejects
stale versions with 412. Same-revision/same-digest retries are idempotent;
revision reuse with different bytes is 409. GET/HEAD return those metadata
headers plus `ETag` and `X-Zigba-Uploaded-At` (the server-confirmed upload time);
PUT returns `{etag, revision, uploadedAt}`.

New clients also send optional `X-Zigba-Title`: `encodeURIComponent` of a private
game label, limited to 160 Unicode code points with no control characters. It
is stored alongside the checkpoint, never exposed through the public catalog.
Older clients may omit it; replacing an object without a title preserves its
existing title. This field does not change the revision/ETag conflict contract.

`X-Zigba-Playthrough-Name` is optional `encodeURIComponent` text, at most 160
Unicode code points with no control characters. The server uses `Playthrough`
when absent (`First adventure` for `default`). D1 atomically admits the first
valid PUT for each `(account, hash, playthroughId)` while the account has fewer
than 100 registered runs. Resume and battery for the same ID share one slot;
updates to a registered run work at 100. The 101st run receives HTTP 409
`{error: {code: "playthrough_limit", message: ...}}`. A validated attempt can
reserve a slot even if its conditional R2 write later fails; offline-only runs
consume no cloud slot. Device-local saves remain independent of sync errors.

`DELETE /api/account` requires a current cookie session, matching
`X-Zigba-Account`, and the exact configured `Origin`. It first durably inserts
an independent `account_deletions` tombstone (opaque user ID and timestamps,
no email or title), closing new cloud admissions, then removes the user and
cascading sessions/OAuth accounts, pending email links, account-scoped rate
limits, playthrough records and `account_limits`. Success is HTTP 202
`{accountDeleted:true,cloudDeletion:"pending"}`. The response does **not** claim
that all R2 data has already been deleted; device-local data is untouched.

An immediate best-effort sweep deletes every R2 key under the account prefix,
including quick slots and old/unknown layouts, across pages. A five-minute cron
repairs a crash after the tombstone but before auth removal, and retries R2
failures. Jobs are swept every five minutes for the first hour and daily later.
Even empty sweeps never retire tombstones: a previously issued R2 PUT can
finish after an empty sweep, even if its Worker then dies. The next scheduled
sweep removes it. Tombstones are retained indefinitely for this reason; an
operational review is required before changing retention or cron cadence.
If auth removal fails before the 202, the route returns retryable HTTP 503
`deletion_pending` while the tombstone remains gated and cron continues work.
Newly created Better Auth users receive fresh IDs, never the erased prefix.
R2/D1 are not transactional, so cloud erasure is eventually consistent, not
instantaneous; if cron stops running, pending blobs can remain until repaired.

Compressed bodies and decoded resume/quick states are limited to 2 MiB. Their
bytes must start with the state magic; this is format screening, not a complete
state or copyright validator. Battery saves must use a supported size, up to
128 KiB. The existing D1 rate-limit table enforces 120 PUT attempts and 20 MiB
of received compressed bodies per account/hour, including retries and quick
slots. These are fixed windows starting at the first request, not sliding
windows. Invalid/rejected work can consume budgets. Apply
`0002_slim_blazing_skull.sql` before deploying this Worker;
the migration adds the retained tombstones and quota registry. There is no individual
cloud-run deletion yet.

The [player](../frontend/README.md#offline-play-and-cloud-progress) defaults cloud
backups on for signed-in accounts, supports a device/account opt-out, and
coalesces offline work; it also owns
[quick-slot reconciliation](../frontend/README.md#quick-slot-sync). Local saves
and source-of-truth resume snapshots remain independent of network success.
New keys allow multiple runs per ROM, not cross-release save forwarding.

### Cloud quick slots

`GET`, `HEAD`, and `PUT /api/saves/:hash/:playthroughId/quick/:slot` store one
state per numbered slot under R2 key `<user>/<hash>/<playthroughId>/quick/<slot>`
(never the legacy three-segment namespace). Authentication, `X-Zigba-Account`,
Origin, gzip bodies, metadata headers, conditional writes, idempotent retries,
`409 revision_conflict`, `412` and the upload budgets are exactly those of
`resume`. Decoded bytes must start with the state magic and fit 2 MiB;
otherwise the PUT fails with `400 invalid_quick` or `413 decoded_too_large`.
The first quick PUT of a run claims that run's single registry entry, like a
resume or battery PUT. All kinds of one run share it, so quick slots never
consume additional playthrough quota. `GET /api/saves` does not list them.

`GET /api/saves/:hash/:playthroughId/quick` returns
`{slots: [{slot, revision, digest, capturedAt, etag, uploadedAt}], limit}` for
the authenticated account only, sorted by slot and without state bodies.
Clients use it to discover a run's slots on a new device and to notice a slot
replaced elsewhere. Invalid hashes, IDs or slot numbers (`0`, leading zeros,
more than two digits) are `400 invalid_save_key`.

Slots are numbered `1..limit` per playthrough. The default limit, 10, is the
`QUICK_SLOT_DEFAULT` constant in `src/cloud-saves.ts`; changing it is a code
release. An operator can override one account in D1:

```sql
INSERT INTO account_limits (user_id, quick_slots) VALUES ('<user id>', 20)
  ON CONFLICT(user_id) DO UPDATE SET quick_slots = excluded.quick_slots;
```

`quick_slots` accepts 0-99 (0 disables quick-slot uploads); `NULL` or no row
uses the default. There is no HTTP management route. A PUT beyond the limit
returns HTTP 409 `{error: {code: "quick_slot_limit", message, limit}}` before
consuming upload budget. Lowering a limit keeps existing higher slots readable
and listed, but they can no longer be replaced. Account deletion removes the
override row and the sweep removes every quick-slot object.

Migration `0003_quick_slot_limits.sql` adds `account_limits`. It is additive;
apply it before deploying a Worker with these routes, because quota checks and
account deletion read that table.

## Local development and tests

From the repository root:

```sh
npm ci
npm run check -w server
npm test -w server
npm run db:migrate:local -w server
npm run dev -w server
```

`server/wrangler.jsonc` is a development-only configuration. Its D1 database ID
and R2 bucket name are placeholders: `wrangler dev` and `--local` migrations
simulate both under the ignored `server/.wrangler/` and never contact a
deployed database or bucket. Do not run that configuration with `--remote`;
deployment uses the root `wrangler.jsonc`.

For login, use **exactly `http://127.0.0.1:5173`**, not `localhost`, port 8787,
or the static Python server. Vite proxies `/api` to Wrangler on port 8787, so
the browser's cookies and OAuth callbacks stay same-origin. Start Vite in a
second terminal with `npm run dev -w frontend`; build the SDK first if
needed (`zig build web-sdk`).

To test Discord sign-in locally, use your own Discord application: set its
client ID as `DISCORD_CLIENT_ID` in `server/wrangler.jsonc` and, in Discord's
Developer Portal under OAuth2 > Redirects, register:

```text
http://127.0.0.1:5173/api/auth/callback/discord
```

Put local secrets in the ignored `server/.dev.vars`, never a `VITE_` variable:

```dotenv
BETTER_AUTH_SECRET=<independent local value generated with openssl rand -hex 32>
DISCORD_CLIENT_SECRET=<your Discord application secret>
```

Restart Wrangler after changing `.dev.vars`. Never reuse a local auth secret in
production.

Open **Account** on Home or in the Player/Library header. Discord uses the real
provider; completing its consent screen is a manual test. Email is **simulated**
by default: Wrangler prints capture paths, and text messages appear beneath
`server/.wrangler/tmp/email/*/email-text/`. Open the newest text file and paste
its link into the same browser within five minutes. No real inbox is contacted
and no Cloudflare email quota is consumed. Do not commit or share those links.
Keep `remote: true` off this binding for routine local tests.

Put a development-only `ADMIN_SECRET=...` in `server/.dev.vars`; it is ignored
by git. Then seed explicitly (ordinary reads and Worker startup never seed):

```sh
curl -X POST http://localhost:8787/api/admin/seed \
  -H 'Authorization: Bearer <development-secret>'
```

Tests use disposable Miniflare D1 databases and apply generated SQL migrations.
They exercise catalog reads/writes, idempotent seeding, validation, missing
bindings, auth configuration, cookies, concurrent single-use verification,
expiry, sign-out, origin/redirect rejection, concurrent limits and window
boundaries, email failures, Discord linking and OAuth state, cloud saves,
quotas and account erasure. Native Workers responses are checked under
workerd, not only Node's Request/Response implementation. Tests do not send
real mail or contact Discord; only those external services are mocked. They do
not write a database or credentials into the checkout. Miniflare is pinned to
the version bundled by Wrangler; its official v4-options converter keeps the
test setup small. npm may report advisories in Vitest and Drizzle Kit's
development tooling; the bundled Worker does not include them. Do not expose
their UI/development servers on public interfaces.

## Schema changes

After changing `src/schema.ts`, generate and review a migration:

```sh
npm run db:generate -w server
```

For a Better Auth/plugin upgrade, regenerate its schema first from the
binding-free generator configuration (run inside `server/`):

```sh
npx --yes auth@1.7.5 generate --config ./auth-schema.config.ts \
  --adapter drizzle --dialect sqlite --output ./src/auth-schema.ts --yes
npm run db:generate
```

Do not edit a migration after it has been applied to a deployed database; add
the next migration. Remote migrations are a separate, explicit step, never part
of the ordinary build or deploy:

```sh
npm run db:migrate:remote -w server
```

Apply them, review the result, and only then deploy code that depends on them.
Prefer backward-compatible (additive) migrations so the currently deployed
Worker stays valid between the two steps.

## Seed provenance

`src/seed.ts` is explicit and idempotent. It seeds two games and only the exact
locally registered supported releases:

| Version                       | Output ROM SHA-256                                                 | Source revision                                                              |
| ----------------------------- | ------------------------------------------------------------------ | ---------------------------------------------------------------------------- |
| Heart & Soul 1.2.1            | `7c4f90c8b68b64d4639a37306d5302a11cd4d7c44005114ec2faa2b28b210c3d` | `PokemonHnS-Development/pokemonHnS@4aa8fa894db4c890b3d1100abd6aab50f44c22b1` |
| Elite Redux 2.65 Beta2 (1048) | `d86e6ac909e4abdef9159ed20b75a684a9b2ee2fe874f388122e00a5749d2d37` | `Elite-Redux/eliteredux-source@06da575972cfed20d471f8bb5b90ea9e4736b16a`     |

These are raw playable ROM output identities from the registered profiles, not
release ZIP or patch hashes. No distributable catalog images were established,
so both seeded `imageUrl` values are null.

## Deployment

One Worker serves the whole site: `/api/*` runs this package first, and every
other path is served from the player's Static Assets. The root `wrangler.jsonc`
describes the official `zigba.ursid.ai` instance. It deliberately contains no
Cloudflare account ID: Wrangler takes it from `CLOUDFLARE_ACCOUNT_ID` (or from
`wrangler login` when you have a single account), and non-interactive deploys
use a scoped `CLOUDFLARE_API_TOKEN`. Keep tokens out of the repository.

```sh
bash tools/build-cloudflare.sh     # tests, then a ReleaseSafe build into .dist/cloudflare
npm run deploy -w frontend   # wrangler deploy --config ../wrangler.jsonc
```

The build script runs `npm ci`, type-checking and tests for every workspace,
compiles the WASM core with a pinned Zig, and publishes only `.dist/cloudflare`
(see the [player deployment notes](../frontend/README.md#cloudflare-deployment)).
It never migrates or seeds a database. A Git-connected Cloudflare Workers
Builds project can run the same two commands as its build and deploy steps.

Catalog management stays disabled unless the `ADMIN_SECRET` Worker secret is
intentionally configured (never as a Wrangler var or a checked-in file):

```sh
npx wrangler secret put ADMIN_SECRET --config wrangler.jsonc
```

An authorized operator may then run `POST /api/admin/seed` once; it is not part
of every deployment and does not run on public requests.

The D1 location hint and EU R2 jurisdiction are not an EU-only residency
guarantee for D1, Workers, email processing or logs; do not promise EU-only
processing from this configuration alone. The official instance's user-facing
data practices are described in [`PRIVACY.md`](../PRIVACY.md).

### Self-hosting

To run your own instance, replace every instance-specific value before the
first deploy:

| Value             | Where                                                            | Notes                                                                                                    |
| ----------------- | ---------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| Worker name       | `name` in `wrangler.jsonc`                                       | Any unused Worker name in your account.                                                                  |
| Domain            | `routes[].pattern`                                               | A custom domain on a zone in your account. Keep `workers_dev` and `preview_urls` off.                    |
| Auth origin       | `vars.BETTER_AUTH_URL`                                           | Exactly `https://<your domain>`: an HTTPS origin with no path or trailing slash.                         |
| Discord client ID | `vars.DISCORD_CLIENT_ID`                                         | Your Discord application; register `https://<your domain>/api/auth/callback/discord`.                    |
| Email sender      | `vars.EMAIL_FROM`, `send_email[].allowed_sender_addresses`       | An address on a domain onboarded to Cloudflare Email Sending. Remove both to disable email sign-in.      |
| D1 database       | `d1_databases[].database_name`, `database_id`                    | Create with `wrangler d1 create <name>`; update the name in `db:migrate:remote` if it differs.           |
| R2 bucket         | `r2_buckets[].bucket_name`                                       | Create with `wrangler r2 bucket create <name> --jurisdiction eu`; keep public access off.                |
| Analytics         | `VITE_POSTHOG_KEY`, `VITE_POSTHOG_HOST`, `VITE_POSTHOG_HOSTNAME` | Build-time; `tools/build-cloudflare.sh` defaults to the official instance. Export yours or empty values. |
| Privacy notice    | `PRIVACY.md`, `VITE_PRIVACY_URL`                                 | Published as `/privacy.txt`. Replace it with your notice, or point `VITE_PRIVACY_URL` elsewhere.         |

Then, with Wrangler authenticated against your account:

1. Set the required secrets `BETTER_AUTH_SECRET` (at least 32 random bytes, for
   example `openssl rand -hex 32`) and `DISCORD_CLIENT_SECRET` with
   `npx wrangler secret put <NAME> --config wrangler.jsonc`.
2. Apply all migrations with `npm run db:migrate:remote -w server`.
3. Build and deploy with the two commands above.

Keep the five-minute cron trigger: it finishes account deletions and removes
late uploads (see [private cloud progress](#private-cloud-progress)). The player
works without the API; sign-in and cloud backups report themselves unavailable
when their bindings or secrets are missing.
