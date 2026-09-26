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

To move the pin:

1. Run `node tools/catalog/no-intro.mjs latest`. It prints the newest
   upstream commit that touched the DAT, the file's SHA-256, its header
   version and entry count, and whether that matches the current pin.
2. Try the candidate with `lookup ... --commit <sha> --sha256 <digest>`.
3. Update `DAT_COMMIT` and `DAT_SHA256` together, and run
   `node --test tools/catalog/*.test.mjs`.

The tests use a handwritten fixture and need no network.
