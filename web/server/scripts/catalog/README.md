# Catalog tools

## No-Intro GBA DAT

[`no-intro.mjs`](no-intro.mjs) resolves a Game Boy Advance ROM's CRC32 or
SHA-1 to its No-Intro dump. Romhack patches in BPS or UPS name their source ROM
by CRC32 only; the DAT turns that CRC32 into the dump's name, serial, size and
SHA-1, from which a catalog base for the patch can be recorded.

```sh
node tools/catalog/no-intro.mjs lookup --crc32 1f1c08fb
node tools/catalog/no-intro.mjs lookup --sha1 <40 hex digits>
node tools/catalog/no-intro.mjs lookup --name emerald  # substring, any case
node tools/catalog/no-intro.mjs latest                 # newest upstream DAT
node tools/catalog/no-intro.mjs names                  # the player's name table
```

`lookup` prints JSON `{ commit, version, matches }` and exits 0 when something
matches, 1 when nothing does, and 2 on a usage, download or verification
error. A CRC32 can match several dumps; a SHA-1 matches at most one.

The module also exports the pieces for other tools: `loadIndex()` downloads
and indexes the pinned DAT, `parseDat()` parses clrmamepro text into
`{ name, region, serial, size, crc32, md5, sha1 }` records with lowercase
hashes, `findByCrc32()`, `findBySha1()` and `findByName()` query the index,
and `toBaseRow()` gives the `{ sha1, crc32, displayName, serial }` fields of a
catalog base. The parser rejects a malformed entry instead of skipping it.

### Source and pinning

The DAT is libretro-database's copy of the No-Intro GBA DAT, fetched from
`raw.githubusercontent.com` at the commit in `DAT_COMMIT` and checked against
the SHA-256 of the file bytes in `DAT_SHA256`. A mismatch is an error; bytes
that fail verification are never cached. The verified file is cached at
`artifacts/no-intro/<commit>.dat` and reused while its digest matches. The DAT
itself is never committed.

`names` writes the pinned DAT's SHA-1 to dump-name table to the player's
[`no-intro-gba.json`](../../frontend/src/pages/library/no-intro-gba.json)
(or to `--out FILE`), which is committed. The Library titles ROMs the catalog
does not list with it. The file records the DAT's source URL, version and
license, and lists one dump per line in SHA-1 order.

To move the pin:

1. Run `node tools/catalog/no-intro.mjs latest`. It prints the newest
   upstream commit that touched the DAT, the file's SHA-256, its header
   version and entry count, and whether that matches the current pin.
2. Try the candidate with `lookup ... --commit <sha> --sha256 <digest>`.
3. Update `DAT_COMMIT` and `DAT_SHA256` together, and run
   `node --test tools/catalog/*.test.mjs`.
4. Run `node tools/catalog/no-intro.mjs names` and commit the table.

The tests use a handwritten fixture and need no network.

## Romhack releases and patch CDN

[`romhacks.mjs`](romhacks.mjs) normalizes the local Pokémon, Golden Sun and
Fire Emblem collections under `.amp/in/` to one release shape. It validates
the recorded base identity and each patch's SHA-1, then writes the
metadata-only [`romhacks.json`](../../server/src/romhacks.json) seed and an
ignored `normalized.json` beside each collection. No base, patched ROM, save,
or patch bytes enter the repository. Releases without a verified patch remain
in the catalog with `patch: null`; entries without a playable output hash do
not become versions. Because the Library installs only a game's latest
release, a hack with patches for Golden Sun and for its sequel The Lost Age
has two catalog games, each with its own base.

[`hooks.json`](hooks.json) contains the short, game-specific hooks shown in
the Library; the collections' complete blurbs remain in `description` for
Show more. The builder requires a distinct concise hook for every game. A
version's project-page link is populated only from an author-controlled
release thread, page or repository backed by the collection's evidence. For
Fire Emblem the selected patch/version must also have `confirmed` attribution
in the local collection; ambiguous pairings remain unlinked. Mirror and
aggregator URLs are not presented as official attribution. The Pokémon
collection has no verified author-post links apart from the Elite Redux
project repository, so the other project links remain unset.

```sh
node tools/catalog/romhacks.mjs build   # regenerate the metadata and local test inventories
node tools/catalog/romhacks.mjs check   # fail if the seed no longer matches the collections
```

Patch object keys are the patch SHA-1 plus its format extension. The separate
read-only [`zigba-patches`](../../server/src/patches.ts) Worker has an EU R2
binding to `evenfall-patches` and a route on `patches.evenfall-emu.org/*`. The
existing R2 custom domain supplies the proxied DNS record; the Worker route
intercepts requests before the bucket's public origin. This keeps patch
handling off the catalog/account Worker and separate from `zigba-saves`. The
Worker adds browser CORS for `play.evenfall-emu.org` and local development,
serves only content-addressed patch paths, and refuses writes. No bucket CORS
policy is required for responses served by this Worker.

After authenticating Wrangler to the account owning the EU bucket, deploy
the patch Worker before uploading; otherwise the readback/CORS check sees
the bucket's direct endpoint rather than the Worker:

```sh
npx wrangler deploy --config server/patches.wrangler.jsonc
node tools/catalog/romhacks.mjs upload
node tools/catalog/romhacks.mjs verify
```

Wrangler must be authenticated to the account that owns the bucket. `upload`
is resumable: it only puts missing content-addressed objects, then downloads
each object and verifies its SHA-1, size and browser CORS header. `verify`
does the same readback without uploading. Do not seed the public catalog or
deploy the player/account Worker with these URLs until the whole upload and
readback succeeds. The catalog seed is explicit; it is not applied on startup
or by a build.
