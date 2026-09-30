# Server

The server is the Cloudflare Worker behind the hosted [player](player.md): a metadata-only game catalog, optional accounts, private cloud backups of game progress, and shareable save-state links.
The player works without it, and sign-in and cloud backups report themselves unavailable when the server's bindings or secrets are missing.
It never serves ROMs, BIOS images or game images, and signing in grants no catalog rights.

One Worker serves the whole site.
`/api/*` and the player's page URLs (`/library`, `/library/<game ID>`, `/manage`, `/player`, `/share/state/<ID>`) run through it first, and the pages answer with the player's `index.html`.
Every other path is a static file from the player's build.
Patches are served straight from an R2 bucket's custom domain ([Patch hosting](#patch-hosting)).

The source is [`web/server/`](../web/server/), the `@evenfall/server` workspace.
It uses [Hono](https://hono.dev/), Drizzle's D1 adapter, [Better Auth](https://www.better-auth.com/) 1.7.5, a D1 database and R2 buckets.

| Path                                                                                                                                         | Contents                                                                         |
| -------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------- |
| [`src/index.ts`](../web/server/src/index.ts)                                                                                                 | Worker entry, catalog routes, management guard, auth mounting                    |
| [`src/catalog.ts`](../web/server/src/catalog.ts), [`src/schema.ts`](../web/server/src/schema.ts), [`src/seed.ts`](../web/server/src/seed.ts) | Catalog queries, Drizzle schema, seed data                                       |
| [`src/auth.ts`](../web/server/src/auth.ts), [`src/auth-schema.ts`](../web/server/src/auth-schema.ts)                                         | Better Auth configuration and generated tables                                   |
| [`src/cloud-saves.ts`](../web/server/src/cloud-saves.ts)                                                                                     | Cloud progress, quick slots, state shares, account deletion and the cleanup cron |
| [`migrations/`](../web/server/migrations/)                                                                                                   | Generated D1 migrations                                                          |
| [`wrangler.jsonc`](../web/server/wrangler.jsonc)                                                                                             | Local development only                                                           |
| [`scripts/catalog/`](../web/server/scripts/catalog/README.md)                                                                                | No-Intro lookups, the romhack seed and patch publication                         |
| [`web/cloudflare/`](../web/cloudflare/)                                                                                                      | The deployment configuration, the build script, headers and privacy notice       |

## Data model

- `games`: a stable ID, a unique internal name, a display name, an optional short `summary`, a longer `description`, an HTTP(S) image URL and a franchise.
- `game_tags`: a game's tags, unique per game ignoring case, in display order.
- `versions`: a game's releases.
  Each has a stable ID, a unique per-game internal name, a globally unique **playable output ROM SHA-1**, a label, an optional required base, the creator's project page, a source revision, a release date, release notes and a hosted patch.
  A patch is an HTTP(S) URL, a format (`bps`, `ups` or `ips`) and its exact size, and needs a required base.
  The base SHA-1 in and the output SHA-1 out verify it, so it carries no digest of its own.
- Each version also has a visibility: `featured` (reserved) and `normal` (reviewed) appear by default, `experimental` needs the Library's opt-in, and `hidden` is absent from public reads.
  A management write without one gets `experimental`.
- `version_save_compatibility`: for a version, the earlier versions of the same game whose in-game saves are verified to load in it.
- `bases`: a stable ID, a unique internal name, a label, and the unique **SHA-1** and indexed **CRC32** of one No-Intro dump.
  BPS and UPS footers name their source ROM by CRC32, and the SHA-1 decides between dumps that share one.

Foreign keys relate versions to games and bases, tags to games, and save compatibility to versions, and deleting a game or version removes its rows there.
Hash formats (40 lowercase hex digits for SHA-1, 8 for CRC32) are checked at the HTTP boundary and in SQLite.
Games are identified by SHA-1 throughout, as No-Intro does, so a catalog hash compares directly with a player's ROM hash and a DAT entry.

## Catalog API

Responses are JSON, except successful deletes (`204`).
IDs use lowercase letters and digits with `-` or `.` separators that cannot lead, trail or repeat.

| Method | Route               | Response                                                                             |
| ------ | ------------------- | ------------------------------------------------------------------------------------ |
| `GET`  | `/api/games`        | `{ games: GameSummary[] }`, including `versionCount`, `tags` and `cloudPlaythroughs` |
| `GET`  | `/api/games/:id`    | `{ game }`, including `tags`, `cloudPlaythroughs`, versions and their required bases |
| `GET`  | `/api/versions`     | `{ versions }` with required-base metadata, `patch` and `saveCompatibleFrom`         |
| `GET`  | `/api/versions/:id` | `{ version }`                                                                        |
| `GET`  | `/api/bases`        | `{ bases }`, each with its `sha1` and `crc32`                                        |
| `GET`  | `/api/bases/:id`    | `{ base }`                                                                           |

Public reads include only releases with a hosted patch that are not `hidden`, including direct ID reads.
A game without such a release answers `404` by ID and is absent from the list.
Experimental releases stay in the public API for the player to show after opt-in: the setting is not access control.
`cloudPlaythroughs` counts cloud-backup playthroughs, across accounts, of the game's public releases, and identifies no account.

The admin API has the same `/api/admin/games`, `/api/admin/versions` and `/api/admin/bases` collections, which also list hidden, patchless and release-less records.
`PUT /:id` creates or replaces a record, `PATCH /:id` edits only the supplied fields, and `DELETE /:id` removes one.
Bodies are strict JSON: unknown fields, malformed IDs, non-HTTP(S) URLs, bad hashes and dates, a patch without a base and broken relations are rejected.
Deleting a game cascades to its versions, and their patch objects stay in R2 because content-addressed keys may be shared.

`PUT /api/admin/versions/:id/patch` attaches or replaces a version's patch.
The body is the raw patch (`Content-Type: application/octet-stream`, `X-Patch-Format: bps`, `ups` or `ips`), at most 40 MiB.
The Worker stores it as `<sha1>.<format>` in the `PATCHES` bucket, with `Cache-Control: public, max-age=31536000, immutable`, and sets the version's patch URL, format and size.
It checks the format header, not that the patch applies to the base or produces `outputSha1`, so verify both with your own base before publishing.
Keep a new version `hidden` until then, because `experimental` is publicly readable.

```sh
curl -X PUT "$ORIGIN/api/admin/versions/<id>/patch" \
  -H "Authorization: Bearer $ADMIN_SECRET" \
  -H 'Content-Type: application/octet-stream' -H 'X-Patch-Format: ups' \
  --data-binary @verified.ups
curl -X PATCH "$ORIGIN/api/admin/versions/<id>" \
  -H "Authorization: Bearer $ADMIN_SECRET" \
  -H 'Content-Type: application/json' -d '{"visibility":"experimental"}'
```

Every management request needs `Authorization: Bearer <ADMIN_SECRET>` or a session whose account has `account_limits.admin = 1` ([Including the ROM](#including-the-rom)).
Revoking the flag or starting account deletion removes access at once.
Management fails closed with `503` when neither is configured.
Session writes need the exact configured `Origin`, and bearer writes, for command-line tools, accept none but reject a foreign one.
Seeding is explicit: reads and Worker startup never seed.

`GET /api/me` returns the signed-in account's `{ id, admin }`, which only informs the player's interface.

## Accounts

Better Auth handles `/api/auth/*` on the same origin, with its tables in D1 beside the catalog.

- **Discord** requests only `identify email` and requires a verified email.
  Accounts with the same verified email link, and linking different emails is disabled.
- **Magic links** expire after five minutes, are stored hashed and work once, even under concurrent verification.
  Mail goes out as `Evenfall <EMAIL_FROM>` through the Cloudflare Email Sending binding, as plain text and branded HTML.
  The logo travels as an inline PNG attachment ([`src/email-brand.ts`](../web/server/src/email-brand.ts)), never a remote image or a tracking link.
  Without both the `EMAIL` binding and `EMAIL_FROM`, email sign-in answers `503 email_unavailable`.
- **Rate limits** live in D1: three magic-link requests per IP per minute, and per recipient three per 15 minutes and ten per day.
  They are fixed windows that start with the first request.
  A throttled recipient gets the same answer as a delivered one.
- **Sessions** last seven days, refresh after one day, and use host-only `HttpOnly`, `SameSite=Lax` cookies, `Secure` over HTTPS.
  Sign-out revokes the database session.
- **Origins.** Unsafe requests need the exact configured origin, and callback destinations are checked before any email is sent.

Configuration fails closed.
`BETTER_AUTH_SECRET` needs at least 32 characters.
`BETTER_AUTH_URL` must be an exact HTTPS origin, with no path or trailing slash, or the local origin `http://127.0.0.1:5173`.
Anything else disables sign-in and cloud saves with `503`.

## Cloud progress

Every cloud request below needs a current session and an account header that names the same account.
The cloud protocol's headers all start with `X-Evenfall-`, and the tables below give only the rest of each name.
Earlier players sent the same headers with an `X-Zigba-` prefix, and their tabs and offline copies keep running for a while after a deploy.
Until they are gone, the Worker accepts either prefix on a request, preferring `X-Evenfall-`, and sends both on every response.
The player sends and reads only `X-Evenfall-`.
R2 custom-metadata keys never carried the prefix, so no stored object depends on it.
Responses are `Cache-Control: no-store`.

### Save objects

`GET`, `HEAD` and `PUT /api/saves/:hash/:playthroughId/:kind` address one backup.
`hash` is the raw playable ROM SHA-1, `playthroughId` a UUID or the reserved `default`, and `kind` is `resume` (a save state) or `battery` (an in-game save).
The server derives the R2 key `<user>/<hash>/<playthroughId>/<kind>` from the authenticated user, so a client can never name another account's object.
For `default`, an older object at `<user>/<hash>/<kind>` is read and replaced in place.

`PUT` needs the exact configured `Origin`, `Content-Type: application/octet-stream` and a gzip body without an HTTP `Content-Encoding`:

| Header             | Meaning                                                                                                        |
| ------------------ | -------------------------------------------------------------------------------------------------------------- |
| `Account`          | The session's account ID                                                                                       |
| `Revision`         | UUID of this revision                                                                                          |
| `Captured-At`      | Capture time, epoch milliseconds                                                                               |
| `Digest`           | SHA-256 of the compressed body                                                                                 |
| `Title`            | Optional private game label: `encodeURIComponent` text of at most 160 code points, without control characters  |
| `Playthrough-Name` | Optional playthrough name, encoded the same way, defaulting to `Playthrough` (`First adventure` for `default`) |

Create with `If-None-Match: *` and replace only with a known `If-Match` ETag.
An atomic R2 conditional write rejects a stale version with `412`.
Retrying the same revision with the same digest is idempotent, and reusing a revision with other bytes is `409 revision_conflict`.
`GET` and `HEAD` return the metadata headers with `ETag` and `Uploaded-At`, and `PUT` returns `{etag, revision, uploadedAt}`.
The title stays private to the account.

A decoded `resume` or quick-slot state must start with `EVENFALL`, the magic of the core's [save states](design/save-states.md), or the upload fails with `400 invalid_resume` or `400 invalid_quick`.
The check screens the format and does not decode the state.
Objects are never checked again after upload, so states saved by the previous core (`ZIGBAST1`) still list, download, delete and erase like any other object.
The player decides what it can load ([States from the previous core](player.md#states-from-the-previous-core)).

### Discovery

`GET /api/saves?cursor=...` lists the account's resume backups as `{saves, cursor, usage: {playthroughs, limit: 100}}`.
Each entry has the ROM `hash`, `playthroughId`, private `playthroughName`, a `title`, `capturedAt` and R2's `uploadedAt`.
A title is the catalog's name for the exact ROM, else the private upload title, else a hash prefix.
A page lists at most 100 R2 objects and filters out battery saves, so it can be empty and still carry a cursor.
Clients read every page before choosing the latest upload, because R2 key order is not upload order.

### Playthrough quota

An account can register 100 playthroughs across all games.
D1 atomically admits the first valid `PUT` for each `(account, hash, playthroughId)` while the account is under the limit.
A playthrough's resume, battery and quick-slot objects share its registration, and a registered playthrough can always update.
The 101st answers `409 {error: {code: "playthrough_limit"}}`.

`DELETE /api/saves/:hash/:playthroughId` needs a cookie session, the account header and an exact same-origin `Origin`.
It removes the playthrough's resume, battery and quick-slot objects, including the older `default` keys, and frees its registration.
It answers `204` even when nothing was there, and other playthroughs and accounts are untouched.

### Quick slots

`GET`, `HEAD` and `PUT /api/saves/:hash/:playthroughId/quick/:slot` store one state per slot under `<user>/<hash>/<playthroughId>/quick/<slot>`.
Everything about them works as for `resume`, and they cost no extra playthrough quota.
`GET /api/saves/:hash/:playthroughId/quick` returns `{slots: [{slot, revision, digest, capturedAt, etag, uploadedAt}], limit}`, without state bodies.

Slots are numbered `1..limit`, with a default limit of 10 (`QUICK_SLOT_DEFAULT` in [`src/cloud-saves.ts`](../web/server/src/cloud-saves.ts)).
An operator can override one account in D1, from 0 (no quick-slot uploads) to 99:

```sql
INSERT INTO account_limits (user_id, quick_slots) VALUES ('<user id>', 20)
  ON CONFLICT(user_id) DO UPDATE SET quick_slots = excluded.quick_slots;
```

A `PUT` beyond the limit answers `409 {error: {code: "quick_slot_limit", limit}}` before spending upload budget.
Lowering a limit keeps higher slots readable, but they cannot be replaced.

### Limits

- Compressed bodies, and decoded resume and quick-slot states, are limited to 2 MiB.
- Battery saves must be a supported save size: 512 B, 8, 32, 64 or 128 KiB.
- Each account may make 120 upload attempts and upload 20 MiB of compressed bodies per hour, counting retries, quick slots and shares.
  The windows are fixed and count rejected work too.

### State shares

A signed-in account can publish one save state as a link anyone can open, without an account, for 24 hours ([Sharing a state](player.md#sharing-a-state)).

`POST /api/shares` needs what a `resume` upload needs, plus `Rom` (the raw playable ROM SHA-1) and a required `Playthrough-Name`.
It answers `201 {id, url, canIncludeRom}`, where `id` is 128 random bits as 22 base64url characters and `url` is `<configured origin>/share/state/<id>`.
Every request creates a new share, which spends upload budget but no playthrough quota.

`GET /api/shares/:id` is public.
It returns the compressed state with `Rom`, `Digest`, URI-encoded `Title` and `Playthrough-Name`, and `Rom-Included: 1` when the ROM can be downloaded too.
It never identifies the account.
Unknown and malformed IDs, expired shares and shares whose account is being deleted answer `404 share_not_found`.

Shares are stored at `share/state/<id>` in the `SAVES` bucket, outside every account prefix, with the owner in private object metadata.
The Worker never checks a share's age: an R2 lifecycle rule on `share/` deletes them ([Self-hosting](#self-hosting)).
R2 applies such rules lazily, so a one-day rule keeps a link working for at least 24 hours, and usually less than 48.
An account can create at most 100 shares per 24 hours (`SHARE_LIMIT`), and past that `POST` answers like the exhausted upload budget, `429 save_rate_limited`.

#### Including the ROM

An admin account can attach the game's ROM to its own share, so the link starts on a device without the game.
The operator grants admin in D1, and there is no HTTP route for it:

```sql
INSERT INTO account_limits (user_id, admin)
  SELECT id, 1 FROM user WHERE email = '<email>'
  ON CONFLICT(user_id) DO UPDATE SET admin = 1;
```

The flag also grants catalog management, so grant it only to trusted operators.
Setting it back to 0 withdraws every ROM the account attached at once.

`PUT /api/shares/:id/rom` takes a gzip ROM of at most 33 MiB, with a `Digest`, for a share the account created.
Other accounts get `403 rom_sharing_forbidden`.
The Worker neither decodes nor hashes the ROM, so readers check its SHA-1 against the share's `Rom`, as the player does.
`GET /api/shares/:id/rom` is public and returns the ROM while the share is readable and its account is still an admin.
Included ROMs are stored at `share/rom/<id>`, under the same lifecycle rule.

### Account deletion

`DELETE /api/account` needs a cookie session, the account header and the exact configured `Origin`.

1. It durably inserts an `account_deletions` tombstone with the opaque user ID, which closes new cloud admissions for the account.
2. It removes the user with its sessions, OAuth accounts, pending email links, rate limits, playthrough registrations and limits.
   It then answers `202 {accountDeleted: true, cloudDeletion: "pending"}`, or a retryable `503 deletion_pending` if this step fails.
3. An immediate best-effort sweep deletes every R2 key under the account prefix, whatever its layout.

Shares sit outside the account prefix: the tombstone hides them from step 1, and the lifecycle rule deletes them.
A five-minute cron trigger repairs a crash between steps 1 and 2 and retries R2 failures.
It sweeps each tombstone every five minutes for the first hour and daily afterwards.
Tombstones are never retired, because an R2 `PUT` issued before deletion can land later, and the next sweep removes it.
R2 and D1 are not transactional together, so cloud erasure is eventually consistent, and device-local data is untouched.

## Patch hosting

Hosted patches live in the EU `evenfall-patches` R2 bucket, served read-only by its custom domain, `patches.evenfall-emu.org`, with the HTTP metadata stored on each object.
Keys are content-addressed (`<sha1>.bps`, `.ups` or `.ips`), and every upload stores `Cache-Control: public, max-age=31536000, immutable`.
The main Worker binds the same bucket only to store admin uploads, and never serves patches in a deployment.
[`scripts/catalog/README.md`](../web/server/scripts/catalog/README.md) describes the seed and patch publication.

The player downloads a patch with a plain cross-origin `GET` from its own origin, without credentials or custom headers, so the browser sends no preflight.
The bucket's CORS policy must allow that request:

| Field           | Value                                                                              |
| --------------- | ---------------------------------------------------------------------------------- |
| Allowed origins | `https://play.evenfall-emu.org`, and `http://127.0.0.1:5173` for local development |
| Allowed methods | `GET`, and `HEAD` for tools that check an object without reading it                |
| Allowed headers | none                                                                               |
| Exposed headers | none                                                                               |

## Local development

From the repository root:

```sh
npm ci
npm run db:migrate:local -w @evenfall/server
npm run dev -w @evenfall/server           # Wrangler on http://127.0.0.1:8787
npm run dev -w @evenfall/player           # the player on http://127.0.0.1:5173
```

[`web/server/wrangler.jsonc`](../web/server/wrangler.jsonc) is for local development only.
Its D1 ID and bucket name are placeholders, and Wrangler simulates both under the ignored `web/server/.wrangler/`.
Never run it with `--remote`.
Without the bucket's custom domain, the local Worker serves patches itself at `/api/dev/patches/<key>`, from the local bucket.
Only the development configuration enables that route, with `LOCAL_PATCHES`, and its `PATCHES_URL` points at it through Vite's `/api` proxy, so a patch uploaded on Manage installs in the local Library.
Catalog releases seeded from `romhacks.json` still point at the public domain.

Put local secrets in the ignored `web/server/.dev.vars`, never in a `VITE_` variable, and restart Wrangler after changing them:

```dotenv
BETTER_AUTH_SECRET=<independent local value, e.g. from openssl rand -hex 32>
DISCORD_CLIENT_SECRET=<your Discord application secret>
ADMIN_SECRET=<development-only catalog management secret>
```

Load the catalog seed with the development secret:

```sh
curl -X POST http://127.0.0.1:8787/api/admin/seed -H 'Authorization: Bearer <development-secret>'
```

To sign in, open exactly `http://127.0.0.1:5173`, where Vite proxies `/api` to Wrangler.
Email is simulated: Wrangler prints capture paths under `web/server/.wrangler/tmp/email/`, and the text version's link signs in within five minutes.
Captures hold a live sign-in credential, so don't share them.
Discord sign-in needs your own Discord application, with its client ID in `wrangler.jsonc` and `http://127.0.0.1:5173/api/auth/callback/discord` registered as a redirect.

`node web/server/scripts/generate-email-brand.mjs` regenerates the email's inline PNG from the checked-in logo, with the workspace's Playwright.

### Tests

`vp test --project server` runs the Worker under workerd with disposable Miniflare D1 databases and the generated migrations.
The tests cover catalog reads and writes, seeding, validation, auth configuration, cookies, single-use links, rate-limit windows, email failures, Discord linking, cloud saves, quotas, shares, account erasure and objects saved by the previous core.
Only external services are mocked: no mail is sent and Discord is never contacted.

## Schema changes

After changing [`src/schema.ts`](../web/server/src/schema.ts), generate and review a migration with `npm run db:generate -w @evenfall/server`.
For a Better Auth upgrade, regenerate its schema first, inside `web/server/`:

```sh
npx --yes auth@1.7.5 generate --config ./auth-schema.config.ts \
  --adapter drizzle --dialect sqlite --output ./src/auth-schema.ts --yes
npm run db:generate
```

Never edit a migration that a deployed database has applied: add the next one.
Remote migrations are a separate, explicit step, never part of a build or deploy:

```sh
npm run db:migrate:remote -w @evenfall/server
```

Apply them, review the result, and only then deploy code that depends on them.
Additive migrations keep the deployed Worker valid between the two steps.

## Seed data

[`src/seed.ts`](../web/server/src/seed.ts) is explicit and idempotent.
It includes only patch-backed releases from the metadata-only [`romhacks.json`](../web/server/src/romhacks.json).
Of the releases with profiles in [`@evenfall/gba-game-plugins`](../web/gba-game-plugins/README.md), it includes Elite Redux 2.65 Beta2 (1048), output SHA-1 `f1323126bc043f3ce931bcc670957930559eb515`, from `Elite-Redux/eliteredux-source@06da575972cfed20d471f8bb5b90ea9e4736b16a`.
It requires the `pokemon-emerald-us` base, the No-Intro "Pokemon - Emerald Version (USA, Europe)" dump: SHA-1 `f3ae088181bf583e55daf962a92bb46f4f1d07b7`, CRC32 `1f1c08fb`.
Heart & Soul has no verified hosted patch and is not seeded.
Running `POST /api/admin/seed` again overwrites the seeded IDs, and admin changes to other records live in D1.

## Deployment

The official instance, [play.evenfall-emu.org](https://play.evenfall-emu.org), is one Worker, `zigba`, configured in [`web/cloudflare/wrangler.jsonc`](../web/cloudflare/wrangler.jsonc): the player's static files, the API and its cron.
Patches come from the bucket's custom domain ([Patch hosting](#patch-hosting)).

The configuration holds no account ID: Wrangler takes it from `CLOUDFLARE_ACCOUNT_ID` or `wrangler login`, and non-interactive deploys use a scoped `CLOUDFLARE_API_TOKEN`.
[`build.sh`](../web/cloudflare/build.sh) builds the main Worker's static files into `.dist/cloudflare`:

1. When rustup is missing, it installs a pinned rustup, checked against its SHA-256, and `rust-toolchain.toml` then selects the pinned compiler and the WebAssembly target.
2. It runs `npm ci`, builds both WebAssembly cores, and runs `vp check` and the player, plugin and server tests.
3. It builds the player with the official instance's [analytics settings](player.md#analytics) into a fresh temporary directory.
4. It refuses to publish ROMs, saves, states, BIOS dumps and archives, and checks that the SDK Worker and both hashed WebAssembly cores are there.
5. It adds [`headers`](../web/cloudflare/headers) as `_headers`, the license files, and [`PRIVACY.md`](../web/cloudflare/PRIVACY.md) as `/privacy.txt`, then moves the result into place.

It never migrates or seeds a database.
HTML and the service worker keep Cloudflare's default revalidation, and Vite's hashed assets are cached as immutable.
Cross-origin isolation is not required.

To build, preview and deploy by hand from the repository root:

```sh
bash web/cloudflare/build.sh
npx wrangler dev --config web/cloudflare/wrangler.jsonc          # local preview of the deployment
npx wrangler deploy --config web/cloudflare/wrangler.jsonc
```

To roll back, use a Worker's deployment history in Cloudflare or redeploy a known-good commit.
Browser data is origin-scoped, so libraries and saves on one domain don't move to another.

### Workers Builds

Workers Builds builds and deploys the Worker from Git, on Ubuntu 24.04 with Node.js and without Rust, which `build.sh` installs.
Its dashboard settings:

| Setting           | Value                                                                                 |
| ----------------- | ------------------------------------------------------------------------------------- |
| Production branch | `main`                                                                                |
| Root directory    | `/`                                                                                   |
| Build command     | `bash web/cloudflare/build.sh`                                                        |
| Deploy command    | `npx wrangler deploy --config web/cloudflare/wrangler.jsonc`                          |
| Build variables   | `NODE_VERSION=24.18.0`, `SKIP_DEPENDENCY_INSTALL=true`, `WRANGLER_SEND_METRICS=false` |

`SKIP_DEPENDENCY_INSTALL` skips Workers Builds' own install, because `build.sh` runs `npm ci` itself.
The build watch paths are `crates/*`, `systems/*`, `web/gba/*`, `web/player/*`, `web/gba-game-plugins/*`, `web/server/*`, `web/cloudflare/*`, `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `package.json`, `package-lock.json` and `vite.config.ts`, with no exclusions.
The dashboard's Worker name must match the `name` in the configuration, or Workers Builds refuses to deploy.

### Self-hosting

Replace every instance-specific value before the first deploy:

| Value             | Where                                                            | Notes                                                                                                        |
| ----------------- | ---------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| Worker name       | `name`                                                           | Any unused Worker name in your account.                                                                      |
| Domain            | `routes[].pattern`                                               | A custom domain on a zone in your account. Keep `workers_dev` and `preview_urls` off.                        |
| Auth origin       | `vars.BETTER_AUTH_URL`                                           | Exactly `https://<your domain>`.                                                                             |
| Discord client ID | `vars.DISCORD_CLIENT_ID`                                         | Your Discord application, with `https://<your domain>/api/auth/callback/discord` registered.                 |
| Email sender      | `vars.EMAIL_FROM`, `send_email[].allowed_sender_addresses`       | An address on a domain onboarded to Cloudflare Email Sending. Remove both to disable email sign-in.          |
| D1 database       | `d1_databases[]`                                                 | Create with `wrangler d1 create <name>`, and update the name in the `db:migrate:remote` script.              |
| R2 bucket         | `r2_buckets[]` for `SAVES`                                       | Create with `wrangler r2 bucket create <name> --jurisdiction eu`, with public access off.                    |
| Patch bucket      | the `PATCHES` binding, `vars.PATCHES_URL`                        | A bucket with a public custom domain and the [CORS policy](#patch-hosting) for your origin, and that domain. |
| Analytics         | `VITE_POSTHOG_KEY`, `VITE_POSTHOG_HOST`, `VITE_POSTHOG_HOSTNAME` | Exported before `build.sh`, which otherwise uses the official instance's. Empty values disable analytics.    |
| Privacy notice    | `web/cloudflare/PRIVACY.md`, `VITE_PRIVACY_URL`                  | Published as `/privacy.txt`. Replace it, or point `VITE_PRIVACY_URL` elsewhere.                              |

Then, with Wrangler authenticated against your account:

1. Set the secrets `BETTER_AUTH_SECRET` (at least 32 random bytes) and `DISCORD_CLIENT_SECRET` with `npx wrangler secret put <NAME> --config web/cloudflare/wrangler.jsonc`.
   `ADMIN_SECRET` is optional and enables bearer-token catalog management.
2. Apply the migrations with `npm run db:migrate:remote -w @evenfall/server`.
3. Add the lifecycle rule that deletes shares:

   ```sh
   npx wrangler r2 bucket lifecycle add <bucket> share-expiry share/ --expire-days 1 --jurisdiction eu
   ```

4. Build and deploy as above.

Keep the five-minute cron trigger: it finishes account deletions and removes late uploads.
The D1 location hint and the bucket's EU jurisdiction don't make processing EU-only, because Workers, D1, email and logs are outside the bucket's jurisdiction.

### Deployed identifiers

Some names are part of what is deployed, and keep an earlier spelling.
Renaming one needs a migration of the resource or the data behind it, not an edit:

- the Worker name `zigba`, which Workers Builds matches against the dashboard,
- the D1 database `zigba-catalog`, and the R2 bucket `zigba-saves` with its key layout,
- the `X-Zigba-` header prefix, accepted and sent [during the transition](#cloud-progress) only,
- the legacy browser storage names, read once by the [move to the current names](player.md#storage-names-and-the-move-from-an-earlier-player).

## Known limits

- Tombstone retention and the sweep cadence are coupled, and there is no policy for retiring tombstones.
- A state share cannot be revoked before it expires, except by deleting the account.
- The upload check reads only a state's magic, so a damaged `EVENFALL` state is caught only when a player loads it.
