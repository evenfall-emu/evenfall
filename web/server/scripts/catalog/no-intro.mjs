#!/usr/bin/env node
// No-Intro Game Boy Advance DAT: pinned download, clrmamepro parser and
// CRC32/SHA-1 lookup. Romhack patches name their source ROM by CRC32, and the
// catalog identifies bases by SHA-1; this DAT maps both to the No-Intro dump.
//
// The source is libretro-database's copy of the DAT, pinned to a commit and to
// the SHA-256 of the file bytes. Downloads are verified and cached under
// artifacts/no-intro/<commit>.dat. See README.md for the update procedure.

import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export const DAT_REPO = 'libretro/libretro-database';
export const DAT_PATH = 'metadat/no-intro/Nintendo - Game Boy Advance.dat';
export const DAT_COMMIT = 'dd2d75d15d5c50cca0c3ef601c32d4dbf6ba431c';
export const DAT_SHA256 = '90188f6e4e481cb2be98eda6271844b7f56813575497bebaaeddf7b76a6353db';
export const CACHE_DIR = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..', 'artifacts', 'no-intro');

export const sha256 = (bytes) => createHash('sha256').update(bytes).digest('hex');
export const datUrl = (commit) =>
  `https://raw.githubusercontent.com/${DAT_REPO}/${commit}/${DAT_PATH.split('/').map(encodeURIComponent).join('/')}`;

const HEX = { crc32: 8, md5: 32, sha1: 40, sha256: 64, commit: 40 };

// Returns the lowercase form of a hex digest, or throws if it is malformed.
export function normalizeHex(value, kind) {
  const digits = HEX[kind];
  if (typeof value !== 'string' || !new RegExp(`^[0-9a-fA-F]{${digits}}$`).test(value)) {
    throw new Error(`${kind} must be ${digits} hex digits, got ${JSON.stringify(value)}`);
  }
  return value.toLowerCase();
}

export function verifyDat(bytes, expected) {
  const actual = sha256(bytes);
  if (actual !== expected) throw new Error(`DAT SHA-256 mismatch: expected ${expected}, got ${actual}`);
  return bytes;
}

export async function downloadDat(commit, fetch = globalThis.fetch) {
  const response = await fetch(datUrl(commit));
  if (!response.ok) throw new Error(`GET ${datUrl(commit)}: HTTP ${response.status}`);
  return Buffer.from(await response.arrayBuffer());
}

// Returns the verified DAT bytes for `commit`, from the cache when its digest
// matches and otherwise from upstream. Unverified bytes are never cached.
export async function loadDat({ commit = DAT_COMMIT, digest = DAT_SHA256, cacheDir = CACHE_DIR, fetch = globalThis.fetch } = {}) {
  commit = normalizeHex(commit, 'commit');
  digest = normalizeHex(digest, 'sha256');
  const path = join(cacheDir, `${commit}.dat`);
  let cached = null;
  try { cached = readFileSync(path); } catch (error) { if (error.code !== 'ENOENT') throw error; }
  if (cached && sha256(cached) === digest) return { bytes: cached, path, cached: true };
  const bytes = verifyDat(await downloadDat(commit, fetch), digest);
  mkdirSync(cacheDir, { recursive: true });
  writeFileSync(`${path}.tmp`, bytes);
  renameSync(`${path}.tmp`, path);
  return { bytes, path, cached: false };
}

// Returns the newest upstream commit that touched the DAT.
export async function latestCommit({ fetch = globalThis.fetch } = {}) {
  const url = `https://api.github.com/repos/${DAT_REPO}/commits?per_page=1&path=${encodeURIComponent(DAT_PATH)}`;
  const headers = { accept: 'application/vnd.github+json' };
  if (process.env.GITHUB_TOKEN) headers.authorization = `Bearer ${process.env.GITHUB_TOKEN}`;
  const response = await fetch(url, { headers });
  if (!response.ok) throw new Error(`GET ${url}: HTTP ${response.status}`);
  const [latest] = await response.json();
  if (!latest) throw new Error(`no upstream commit touches ${DAT_PATH}`);
  return { commit: normalizeHex(latest.sha, 'commit'), date: latest.commit.committer.date };
}

// clrmamepro text: `key value` pairs where a value is a bare word, a quoted
// string (no escapes) or a parenthesized block of further pairs.
function tokenize(text) {
  const tokens = [];
  let line = 1;
  for (let i = 0; i < text.length;) {
    const c = text[i];
    if (c === '\n') { line++; i++; } else if (/\s/.test(c)) i++;
    else if (c === '(' || c === ')') { tokens.push({ type: c, line }); i++; }
    else if (c === '"') {
      const end = text.indexOf('"', i + 1);
      const newline = text.indexOf('\n', i + 1);
      if (end < 0 || (newline >= 0 && newline < end)) throw new Error(`line ${line}: unterminated string`);
      tokens.push({ type: 'string', value: text.slice(i + 1, end), line });
      i = end + 1;
    } else {
      const start = i;
      while (i < text.length && !/[\s()"]/.test(text[i])) i++;
      tokens.push({ type: 'word', value: text.slice(start, i), line });
    }
  }
  return tokens;
}

function parseBlocks(tokens) {
  let at = 0;
  const block = (depth) => {
    const pairs = [];
    while (at < tokens.length && tokens[at].type !== ')') {
      const key = tokens[at++];
      if (key.type !== 'word') throw new Error(`line ${key.line}: expected a key, found ${key.value ?? key.type}`);
      const value = tokens[at++];
      if (!value || value.type === ')') throw new Error(`line ${key.line}: ${key.value} has no value`);
      if (value.type === '(') {
        pairs.push({ key: key.value, line: key.line, block: block(depth + 1) });
        if (tokens[at++]?.type !== ')') throw new Error(`line ${key.line}: ${key.value} block is not closed`);
      } else pairs.push({ key: key.value, line: key.line, value: value.value });
    }
    if (depth === 0 && at < tokens.length) throw new Error(`line ${tokens[at].line}: unmatched )`);
    return pairs;
  };
  return block(0);
}

// Returns a key->value object for a block, rejecting repeated keys.
function fields(pairs, what) {
  const out = {};
  for (const pair of pairs) {
    if (Object.hasOwn(out, pair.key)) throw new Error(`line ${pair.line}: ${what} repeats ${pair.key}`);
    out[pair.key] = pair;
  }
  return out;
}

function scalar(map, key, what, required = true) {
  const pair = map[key];
  if (!pair) {
    if (required) throw new Error(`${what} has no ${key}`);
    return null;
  }
  if (pair.block) throw new Error(`line ${pair.line}: ${what} ${key} must be a value, not a block`);
  return pair.value;
}

function parseGame(pair) {
  const game = fields(pair.block.filter((p) => p.key !== 'rom'), 'game');
  const name = scalar(game, 'name', `game at line ${pair.line}`);
  const what = `game ${JSON.stringify(name)}`;
  const roms = pair.block.filter((p) => p.key === 'rom');
  if (roms.length !== 1 || !roms[0].block) throw new Error(`${what} must have exactly one rom ( ... ) block`);
  const rom = fields(roms[0].block, `${what} rom`);
  const sizeText = scalar(rom, 'size', `${what} rom`);
  const size = Number(sizeText);
  if (!/^[0-9]+$/.test(sizeText) || !Number.isSafeInteger(size) || size <= 0) throw new Error(`${what} rom size must be a positive integer`);
  const hex = (kind, key = kind) => {
    try { return normalizeHex(scalar(rom, key, `${what} rom`), kind); } catch (error) { throw new Error(`${what}: ${error.message}`); }
  };
  const serial = scalar(game, 'serial', what, false);
  const romSerial = scalar(rom, 'serial', `${what} rom`, false);
  if (serial !== null && romSerial !== null && serial !== romSerial) throw new Error(`${what} has conflicting serials`);
  return {
    name,
    region: scalar(game, 'region', what, false),
    serial: serial ?? romSerial,
    size,
    crc32: hex('crc32', 'crc'),
    md5: hex('md5'),
    sha1: hex('sha1'),
  };
}

// Parses clrmamepro DAT text into `{ version, entries }`. Throws on any
// malformed entry instead of skipping it.
export function parseDat(text) {
  let version = null;
  const entries = [];
  for (const [index, pair] of parseBlocks(tokenize(text)).entries()) {
    if (!pair.block) throw new Error(`line ${pair.line}: top-level ${pair.key} must be a block`);
    if (pair.key === 'clrmamepro') {
      if (index !== 0) throw new Error(`line ${pair.line}: clrmamepro header must come first`);
      version = scalar(fields(pair.block, 'clrmamepro header'), 'version', 'clrmamepro header', false);
    } else if (pair.key === 'game') entries.push(parseGame(pair));
    else throw new Error(`line ${pair.line}: unexpected top-level ${pair.key} block`);
  }
  if (entries.length === 0) throw new Error('DAT has no game entries');
  return { version, entries };
}

// Indexes entries by SHA-1 (unique) and CRC32 (possibly shared).
export function indexDat(entries) {
  const bySha1 = new Map();
  const byCrc32 = new Map();
  for (const entry of entries) {
    if (bySha1.has(entry.sha1)) throw new Error(`SHA-1 ${entry.sha1} is listed twice`);
    bySha1.set(entry.sha1, entry);
    byCrc32.set(entry.crc32, [...(byCrc32.get(entry.crc32) ?? []), entry]);
  }
  return { entries, bySha1, byCrc32 };
}

export const findBySha1 = (index, sha1) => index.bySha1.get(normalizeHex(sha1, 'sha1')) ?? null;
export const findByCrc32 = (index, crc32) => index.byCrc32.get(normalizeHex(crc32, 'crc32')) ?? [];
export const findByName = (index, text) => {
  const needle = text.toLowerCase();
  return index.entries.filter((entry) => entry.name.toLowerCase().includes(needle));
};

// The catalog `bases` row fields a DAT entry determines.
export const toBaseRow = (entry) => ({ sha1: entry.sha1, crc32: entry.crc32, displayName: entry.name, serial: entry.serial });

export async function loadIndex(options) {
  const { bytes } = await loadDat(options);
  const { version, entries } = parseDat(bytes.toString('utf8'));
  return { version, ...indexDat(entries) };
}

const USAGE = `Usage: node tools/catalog/no-intro.mjs lookup (--crc32 HEX | --sha1 HEX | --name TEXT) [options]
       node tools/catalog/no-intro.mjs latest
Options: --commit SHA --sha256 HEX  use another libretro-database commit and its DAT digest
         --cache DIR                 cache directory (default artifacts/no-intro)
Exit status: 0 found, 1 not found, 2 usage, download or verification error.`;

class UsageError extends Error {}

function parseArgs(argv) {
  const [command, ...rest] = argv;
  const options = {};
  for (let i = 0; i < rest.length; i += 2) {
    const flag = rest[i];
    if (!['--crc32', '--sha1', '--name', '--commit', '--sha256', '--cache'].includes(flag)) throw new UsageError(`unknown argument ${flag}`);
    if (rest[i + 1] === undefined) throw new UsageError(`${flag} needs a value`);
    options[flag.slice(2)] = rest[i + 1];
  }
  return { command, options };
}

async function main(argv) {
  const { command, options } = parseArgs(argv);
  if (command === 'latest') {
    if (Object.keys(options).length) throw new UsageError('latest takes no options');
    // Report what a new pin would be; parsing checks the parser still accepts it.
    const latest = await latestCommit();
    const bytes = await downloadDat(latest.commit);
    const { version, entries } = parseDat(bytes.toString('utf8'));
    const pinned = latest.commit === DAT_COMMIT && sha256(bytes) === DAT_SHA256;
    console.log(JSON.stringify({ ...latest, sha256: sha256(bytes), version, entries: entries.length, pinned }, null, 2));
    return 0;
  }
  if (command !== 'lookup') throw new UsageError(command ? `unknown command ${command}` : 'missing command');
  const keys = ['crc32', 'sha1', 'name'].filter((key) => key in options);
  if (keys.length !== 1) throw new UsageError('lookup needs exactly one of --crc32, --sha1 or --name');
  if (('commit' in options) !== ('sha256' in options)) throw new UsageError('--commit and --sha256 must be given together');
  const [key] = keys;
  try { if (key !== 'name') normalizeHex(options[key], key); } catch (error) { throw new UsageError(error.message); }
  const index = await loadIndex({
    commit: options.commit ?? DAT_COMMIT,
    digest: options.sha256 ?? DAT_SHA256,
    cacheDir: options.cache ? resolve(options.cache) : CACHE_DIR,
  });
  const found = key === 'sha1' ? [findBySha1(index, options.sha1)].filter(Boolean)
    : key === 'crc32' ? findByCrc32(index, options.crc32) : findByName(index, options.name);
  console.log(JSON.stringify({ commit: (options.commit ?? DAT_COMMIT).toLowerCase(), version: index.version, matches: found }, null, 2));
  return found.length ? 0 : 1;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2)).then((code) => { process.exitCode = code; }, (error) => {
    console.error(`no-intro: ${error.message}`);
    if (error instanceof UsageError) console.error(USAGE);
    process.exitCode = 2;
  });
}
