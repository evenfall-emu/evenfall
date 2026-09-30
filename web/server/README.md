# @evenfall/server

The Cloudflare Worker behind the hosted Evenfall player: the game catalog, accounts, cloud backups, state shares and patch uploads.
[`docs/server.md`](../../docs/server.md) describes its API, storage, local development and deployment.

From the repository root:

```sh
npm ci
npm run db:migrate:local -w @evenfall/server
npm run dev -w @evenfall/server            # Wrangler on http://127.0.0.1:8787
vp test --project server
```

`wrangler.jsonc` here is for local development only.
The deployed Worker is configured in [`web/cloudflare/`](../cloudflare/).

## Layout

- `src/`: the Worker. `index.ts` is the entry and catalog, `auth.ts` accounts, `cloud-saves.ts` cloud progress and shares.
- `migrations/`: generated D1 migrations.
- `test/`: Vitest tests under Miniflare.
- `scripts/catalog/`: No-Intro lookups, the romhack seed and patch publication ([README](scripts/catalog/README.md)).
- `scripts/generate-email-brand.mjs`: regenerates the sign-in email's inline logo.
