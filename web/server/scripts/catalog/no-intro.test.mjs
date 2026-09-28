import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import {
  findByCrc32,
  findByName,
  findBySha1,
  indexDat,
  loadDat,
  namesJson,
  parseDat,
  sha256,
  toBaseRow,
} from "./no-intro.mjs";

const CLI = fileURLToPath(new URL("./no-intro.mjs", import.meta.url));
const COMMIT = "a".repeat(40);

// Handwritten in the libretro No-Intro layout. The Emerald hashes are the
// No-Intro dump's; the two "Shared CRC" entries are invented to collide.
const FIXTURE = `clrmamepro (
\tname "Nintendo - Game Boy Advance"
\tdescription "Nintendo - Game Boy Advance"
\tversion "2026.01.02"
)

game (
\tname "Pokemon - Emerald Version (USA, Europe)"
\tregion "USA"
\tserial "BPEE"
\trom ( name "Pokemon - Emerald Version (USA, Europe).gba" size 16777216 crc 1F1C08FB md5 605B89B67018ABCEA91E693A4DD25BE3 sha1 F3AE088181BF583E55DAF962A92BB46F4F1D07B7 serial "BPEE" )
)
game (
\tname "Tricky (Name), With (Parens, Commas) (Europe) (En,Fr,De)"
\treleaseyear "2004"
\trom ( name "Tricky (Name), With (Parens, Commas) (Europe) (En,Fr,De).gba" size 4194304 crc 0000ABCD md5 0123456789ABCDEF0123456789ABCDEF sha1 1111111111111111111111111111111111111111 )
)
game (
\tname "Shared CRC (Japan)"
\tregion "Japan"
\tserial "ZZZJ"
\trom ( name "Shared CRC (Japan).gba" size 8388608 crc DEADBEEF md5 00000000000000000000000000000001 sha1 2222222222222222222222222222222222222222 serial "ZZZJ" )
)
game (
\tname "Shared CRC (USA)"
\tregion "USA"
\trom ( name "Shared CRC (USA).gba" size 8388608 crc DEADBEEF md5 00000000000000000000000000000002 sha1 3333333333333333333333333333333333333333 serial "ZZZE" )
)
`;

const EMERALD = {
  name: "Pokemon - Emerald Version (USA, Europe)",
  region: "USA",
  serial: "BPEE",
  size: 16777216,
  crc32: "1f1c08fb",
  md5: "605b89b67018abcea91e693a4dd25be3",
  sha1: "f3ae088181bf583e55daf962a92bb46f4f1d07b7",
};

test("parses entries, normalizes hashes and exposes the header version", () => {
  const { version, entries } = parseDat(FIXTURE);
  assert.equal(version, "2026.01.02");
  assert.equal(entries.length, 4);
  assert.deepEqual(entries[0], EMERALD);
  assert.deepEqual(entries[1], {
    name: "Tricky (Name), With (Parens, Commas) (Europe) (En,Fr,De)",
    region: null,
    serial: null,
    size: 4194304,
    crc32: "0000abcd",
    md5: "0123456789abcdef0123456789abcdef",
    sha1: "1".repeat(40),
  });
  assert.equal(entries[3].serial, "ZZZE", "falls back to the rom serial");
});

test("rejects malformed input instead of skipping it", () => {
  const emerald = FIXTURE.split("game (")[1];
  const game = (body) => `game (${body}`;
  const cases = [
    [game(emerald.replace(" crc 1F1C08FB", "")), /has no crc/],
    [
      game(emerald.replace("1F1C08FB", "1F1C08F")),
      /crc32 must be 8 hex digits/,
    ],
    [
      game(emerald.replace("sha1 F3AE", "sha1 G3AE")),
      /sha1 must be 40 hex digits/,
    ],
    [
      game(emerald.replace("size 16777216", "size 0x1000000")),
      /size must be a positive integer/,
    ],
    [
      game(
        emerald.replace('\tregion "USA"', '\tregion "USA"\n\tregion "Europe"'),
      ),
      /repeats region/,
    ],
    [
      game(emerald.replace('\tserial "BPEE"\n', '\tserial "BPEP"\n')),
      /conflicting serials/,
    ],
    [game(emerald.replace(/\trom \(.*\n/, "")), /exactly one rom/],
    [game(emerald.replace(/(\trom \(.*\n)/, "$1$1")), /exactly one rom/],
    [
      game(emerald.replace('\tname "Pokemon', '\ttitle "Pokemon')),
      /has no name/,
    ],
    [
      game(
        emerald.replace(
          '"Pokemon - Emerald Version (USA, Europe)"',
          '"Pokemon',
        ),
      ),
      /unterminated string/,
    ],
    [game(emerald.replace(/\)\n$/, "")), /block is not closed/],
    [`${game(emerald)})\n`, /unmatched \)/],
    [`resource (\n\tname "x"\n)\n`, /unexpected top-level resource/],
    [
      `${game(emerald)}clrmamepro (\n\tversion "1"\n)\n`,
      /header must come first/,
    ],
    ['clrmamepro (\n\tversion "1"\n)\n', /no game entries/],
  ];
  for (const [text, error] of cases)
    assert.throws(() => parseDat(text), error, text);
});

test("looks entries up by SHA-1, shared CRC32 and name substring", () => {
  const index = indexDat(parseDat(FIXTURE).entries);
  assert.deepEqual(
    findBySha1(index, "F3AE088181BF583E55DAF962A92BB46F4F1D07B7"),
    EMERALD,
  );
  assert.equal(findBySha1(index, "4".repeat(40)), null);
  assert.deepEqual(findByCrc32(index, "1f1c08fb"), [EMERALD]);
  assert.deepEqual(
    findByCrc32(index, "DEADBEEF").map((e) => e.name),
    ["Shared CRC (Japan)", "Shared CRC (USA)"],
  );
  assert.deepEqual(findByCrc32(index, "12345678"), []);
  assert.throws(
    () => findByCrc32(index, "1f1c08f"),
    /crc32 must be 8 hex digits/,
  );
  assert.deepEqual(
    findByName(index, "emerald").map((e) => e.sha1),
    [EMERALD.sha1],
  );
  assert.equal(findByName(index, "shared crc").length, 2);
  assert.deepEqual(toBaseRow(EMERALD), {
    sha1: EMERALD.sha1,
    crc32: EMERALD.crc32,
    displayName: EMERALD.name,
    serial: "BPEE",
  });
  const { entries } = parseDat(FIXTURE);
  assert.throws(
    () => indexDat([...entries, { ...entries[2], name: "Copy" }]),
    /listed twice/,
  );
});

test("writes the SHA-1 -> name table sorted, with its source and license", () => {
  const { version, entries } = parseDat(FIXTURE);
  const text = namesJson(entries, { commit: COMMIT, version });
  const table = JSON.parse(text);
  assert.equal(table.version, "2026.01.02");
  assert.match(
    table.source,
    new RegExp(
      `/blob/${COMMIT}/metadat/no-intro/Nintendo%20-%20Game%20Boy%20Advance\\.dat$`,
    ),
  );
  assert.match(table.license, /^CC BY-SA 4\.0/);
  assert.deepEqual(Object.entries(table.names), [
    [
      "1111111111111111111111111111111111111111",
      "Tricky (Name), With (Parens, Commas) (Europe) (En,Fr,De)",
    ],
    ["2222222222222222222222222222222222222222", "Shared CRC (Japan)"],
    ["3333333333333333333333333333333333333333", "Shared CRC (USA)"],
    [EMERALD.sha1, EMERALD.name],
  ]);
  // One dump per line, so a new DAT diffs by entry.
  assert.equal(
    text.split("\n").filter((line) => /^ {4}"[0-9a-f]{40}": /.test(line))
      .length,
    4,
  );
});

test("verifies cached and downloaded bytes against the pinned digest", async (t) => {
  const cacheDir = mkdtempSync(join(tmpdir(), "no-intro-"));
  t.after(() => rmSync(cacheDir, { recursive: true, force: true }));
  const good = Buffer.from(FIXTURE);
  const digest = sha256(good);
  const path = join(cacheDir, `${COMMIT}.dat`);
  const requests = [];
  const serve = (bytes) => async (url) => {
    requests.push(url);
    return new Response(bytes);
  };
  const offline = async () => assert.fail("fetched despite a valid cache");

  // Nothing cached: download, verify, cache.
  let result = await loadDat({
    commit: COMMIT,
    digest,
    cacheDir,
    fetch: serve(good),
  });
  assert.equal(result.cached, false);
  assert.match(
    requests[0],
    new RegExp(
      `/libretro-database/${COMMIT}/metadat/no-intro/Nintendo%20-%20Game%20Boy%20Advance\\.dat$`,
    ),
  );
  assert.deepEqual(readFileSync(path), good);

  // A cache with the right digest is reused.
  result = await loadDat({ commit: COMMIT, digest, cacheDir, fetch: offline });
  assert.equal(result.cached, true);
  assert.deepEqual(result.bytes, good);

  // A corrupted cache is replaced by a verified download.
  writeFileSync(path, "corrupt");
  result = await loadDat({
    commit: COMMIT,
    digest,
    cacheDir,
    fetch: serve(good),
  });
  assert.equal(result.cached, false);
  assert.deepEqual(readFileSync(path), good);

  // A download with the wrong digest fails and is never cached.
  writeFileSync(path, "corrupt");
  await assert.rejects(
    loadDat({ commit: COMMIT, digest, cacheDir, fetch: serve("tampered") }),
    /SHA-256 mismatch/,
  );
  assert.equal(readFileSync(path, "utf8"), "corrupt");
  await assert.rejects(
    loadDat({
      commit: COMMIT,
      digest,
      cacheDir,
      fetch: async () => new Response("", { status: 404 }),
    }),
    /HTTP 404/,
  );
});

test("CLI reports matches through its exit status", (t) => {
  const cacheDir = mkdtempSync(join(tmpdir(), "no-intro-"));
  t.after(() => rmSync(cacheDir, { recursive: true, force: true }));
  writeFileSync(join(cacheDir, `${COMMIT}.dat`), FIXTURE);
  const pin = [
    "--commit",
    COMMIT,
    "--sha256",
    sha256(Buffer.from(FIXTURE)),
    "--cache",
    cacheDir,
  ];
  const run = (...args) =>
    spawnSync(process.execPath, [CLI, ...args], { encoding: "utf8" });

  const found = run("lookup", "--crc32", "1F1C08FB", ...pin);
  assert.equal(found.status, 0, found.stderr);
  assert.deepEqual(JSON.parse(found.stdout), {
    commit: COMMIT,
    version: "2026.01.02",
    matches: [EMERALD],
  });
  const shared = run("lookup", "--crc32", "deadbeef", ...pin);
  assert.equal(JSON.parse(shared.stdout).matches.length, 2);
  assert.equal(run("lookup", "--sha1", EMERALD.sha1, ...pin).status, 0);
  assert.equal(run("lookup", "--name", "Nonexistent", ...pin).status, 1);
  assert.equal(run("lookup", "--sha1", "1234", ...pin).status, 2);
  assert.equal(
    run("lookup", "--crc32", "1f1c08fb", "--name", "x", ...pin).status,
    2,
  );
  assert.equal(
    run(
      "lookup",
      "--crc32",
      "1f1c08fb",
      "--commit",
      COMMIT,
      "--cache",
      cacheDir,
    ).status,
    2,
  );
  assert.equal(run("frobnicate").status, 2);
});
