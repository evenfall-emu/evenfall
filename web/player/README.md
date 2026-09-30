# @evenfall/player

Evenfall's web player: a game library and GBA player that run in the browser, built on [`@evenfall/gba`](../gba/README.md).
[`docs/player.md`](../../docs/player.md) describes what it does and how it works, and [`docs/server.md`](../../docs/server.md) the Worker behind the hosted instance.

From the repository root:

```sh
npm ci
npm run build:wasm -w @evenfall/gba
npm run dev -w @evenfall/player            # http://127.0.0.1:5173
npm run build -w @evenfall/player          # web/player/dist
npm run test:browser -w @evenfall/player   # Chromium, against web/player/dist
```

The unit tests run with `vp test --project player`.

## Layout

- `src/`: the application. `main.tsx` holds the pages, `player.ts` the session, `library.ts` local storage, `cloud.ts` cloud sync, `games.ts` the game-plugin registry.
- `src/storage-names.ts` and `src/storage-migration.ts`: the browser storage names, and the one-time move from an earlier player's names.
- `src/state-format.ts`: tells current save states from those of the previous core.
- `offline.ts`: the Vite plugin that writes the service worker.
- `tests/`: Vitest unit tests, and `tests/browser/` the Playwright suite.
- `scripts/`: the local static server the browser tests use, the brand-icon renderer and the landing-art tools.

## License

GNU Affero General Public License, version 3 or later.
The bundled fonts keep their own licenses ([third-party notices](../../THIRD_PARTY_NOTICES.md)).
