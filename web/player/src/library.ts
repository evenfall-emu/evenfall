import { Context, Data, Effect, Layer } from "effect";
import { openDB, type IDBPDatabase, type IDBPTransaction } from "idb";
import type { LoadOptions, ScenarioExport } from "@evenfall/gba";
export class StorageError extends Data.TaggedError("StorageError")<{
  cause: unknown;
}> {
  get message() {
    return this.cause instanceof Error
      ? this.cause.message
      : String(this.cause);
  }
}
export type Game = {
  hash: string;
  title: string;
  romSize: number;
  lastPlayed: number;
  resumeSavedAt?: number;
  activePlaythroughId?: string;
};
export type PlaythroughRef = { hash: string; playthroughId: string };
export type Playthrough = {
  hash: string;
  id: string;
  name: string;
  createdAt: number;
  lastPlayed: number;
  resumeSavedAt?: number;
};
const runKey = (ref: PlaythroughRef) => [ref.hash, ref.playthroughId];
export const playthroughLockKey = (ref: PlaythroughRef) =>
  `playthrough:${JSON.stringify(runKey(ref))}`;
export const quickStateKey = (ref: PlaythroughRef, slot: number) =>
  `quick-state:${JSON.stringify([ref.hash, ref.playthroughId, slot])}`;
/** Slots 1-10; Digit0 addresses slot 10. Matches the server's default limit. */
export const QUICK_SLOTS = 10;
export function quickSlotForKey(code: string): number | undefined {
  const digit = /^Digit([0-9])$/.exec(code)?.[1];
  return digit === undefined ? undefined : Number(digit) || QUICK_SLOTS;
}
/**
 * A quick slot's bytes and sync identity. `cloudRevision` is the revision this
 * content carries in `accountId`'s outbox; slots stored before cloud sync were
 * bare state bytes and read back without either field.
 */
export type QuickSlot = {
  data: Uint8Array;
  savedAt: number;
  cloudRevision?: string;
  accountId?: string;
};
export type QuickSlotSummary = {
  slot: number;
  savedAt?: number;
  sync?: "pending" | "synced" | "conflict";
};
function quickRecord(value: unknown): QuickSlot | undefined {
  if (value instanceof Uint8Array) return { data: value, savedAt: 0 };
  return (value as QuickSlot | undefined) ?? undefined;
}
type GameImport = Omit<Game, "romSize"> & { rom: Uint8Array };
export type ResumeGeneration = {
  data: Uint8Array;
  savedAt: number;
  cycle: string;
  options: LoadOptions;
  cloudRevision?: string;
};
export type ResumeCheckpoint = ResumeGeneration & {
  previous?: ResumeGeneration;
};
export type SaveKind = "resume" | "battery";
/** Doubles as the URL suffix: `/api/saves/:hash/:run/quick/:slot`. */
export type QuickKind = `quick/${number}`;
export type CloudKind = SaveKind | QuickKind;
export const quickKind = (slot: number): QuickKind => `quick/${slot}`;
export function quickSlotOf(kind: CloudKind): number | undefined {
  return kind.startsWith("quick/") ? Number(kind.slice(6)) : undefined;
}
function isCloudKind(kind: unknown): kind is CloudKind {
  return (
    kind === "resume" ||
    kind === "battery" ||
    (typeof kind === "string" && /^quick\/[1-9][0-9]?$/.test(kind))
  );
}
/** Server metadata for one cloud quick slot. */
export type QuickRemote = {
  slot: number;
  revision: string;
  digest: string;
  capturedAt: number;
  etag: string;
  uploadedAt: string;
};
export type CloudIdentity = { accountId: string | null; enabled: boolean };
export type CloudJob = {
  accountId: string;
  hash: string;
  playthroughId: string;
  kind: CloudKind;
  revision: string;
  capturedAt: number;
  lastInputAt: number;
  baseEtag?: string;
  uploadedRevision?: string;
  uploadedAt?: number;
  confirmedAt?: number;
  retryAt?: number;
  attempts?: number;
  conflict?: boolean;
  flight?: { revision: string; digest: string };
  /** Quick slots only: the diverged cloud copy awaiting a user decision. */
  remote?: QuickRemote;
  /** Quick slots only: the remote ETag the user was already asked about. */
  prompted?: string;
};
export const CLOUD_IDLE_MS = 5 * 60_000;
export const CLOUD_INTERVAL_MS = 2 * 60_000;
/** Pacing for repeated explicit saves of one slot; bounds budget use. */
export const QUICK_INTERVAL_MS = 30_000;
export function cloudEligible(job: CloudJob, now: number) {
  // Quick slots upload on explicit save, not on activity: no idle cutoff.
  if (quickSlotOf(job.kind) !== undefined)
    return (
      !job.conflict &&
      Boolean(job.revision) &&
      job.revision !== job.uploadedRevision &&
      now >= (job.retryAt ?? 0) &&
      now >= (job.uploadedAt ?? 0) + QUICK_INTERVAL_MS
    );
  return (
    !job.conflict &&
    job.revision !== job.uploadedRevision &&
    job.lastInputAt > 0 &&
    now >= job.lastInputAt &&
    now - job.lastInputAt < CLOUD_IDLE_MS &&
    now >= (job.retryAt ?? 0) &&
    now >= (job.uploadedAt ?? 0) + CLOUD_INTERVAL_MS
  );
}
const cloudKey = (
  job: Pick<CloudJob, "accountId" | "hash" | "playthroughId" | "kind">,
) => [job.accountId, job.hash, job.playthroughId, job.kind];

// Metadata and the referenced save commit together. No snapshot bytes enter
// the outbox; a newer persistence replaces its pending revision in place.
async function queueCloud(
  tx: IDBPTransaction<unknown, any, "readwrite">,
  ref: PlaythroughRef,
  kind: CloudKind,
  capturedAt: number,
  lastInputAt: number,
  expectedAccountId?: string,
): Promise<{ cloudRevision?: string }> {
  const store = tx.objectStore("cloud");
  const identity: CloudIdentity | undefined = await store.get("identity");
  if (expectedAccountId && identity?.accountId !== expectedAccountId)
    throw new Error("Account changed during cloud restore.");
  if (!identity?.accountId || !identity.enabled) return {};
  const key = [identity.accountId, ref.hash, ref.playthroughId, kind];
  const old: CloudJob | undefined = await store.get(key);
  const revision = crypto.randomUUID();
  await store.put(
    {
      ...old,
      accountId: identity.accountId,
      ...ref,
      kind,
      revision,
      capturedAt,
      lastInputAt,
      // An explicit quick save is a fresh request; keep the attempt count so a
      // failing server still backs off, but do not wait out the old deadline.
      ...(quickSlotOf(kind) !== undefined ? { retryAt: 0 } : {}),
    } satisfies CloudJob,
    key,
  );
  if (kind === "resume") {
    const batteryKey = [
      identity.accountId,
      ref.hash,
      ref.playthroughId,
      "battery",
    ];
    const battery: CloudJob | undefined = await store.get(batteryKey);
    if (battery) await store.put({ ...battery, lastInputAt }, batteryKey);
  }
  return { cloudRevision: revision };
}
function metadata(game: GameImport): Game {
  return {
    hash: game.hash,
    title: game.title,
    romSize: game.rom.byteLength,
    lastPlayed: game.lastPlayed,
    ...(game.resumeSavedAt === undefined
      ? {}
      : { resumeSavedAt: game.resumeSavedAt }),
  };
}
async function requireRun(
  tx: IDBPTransaction<unknown, any, "readwrite">,
  ref: PlaythroughRef,
): Promise<Playthrough> {
  const run = await tx.objectStore("playthroughs").get(runKey(ref));
  if (!run || run.hash !== ref.hash || run.id !== ref.playthroughId)
    throw new Error("Playthrough is no longer stored.");
  return run;
}
async function finishWrite<T>(
  tx: IDBPTransaction<unknown, any, "readwrite">,
  work: () => Promise<T>,
): Promise<T> {
  try {
    const result = await work();
    await tx.done;
    return result;
  } catch (error) {
    try {
      tx.abort();
    } catch {
      /* Already aborted. */
    }
    await tx.done.catch(() => {});
    throw error;
  }
}
export class Library {
  constructor(
    readonly db: IDBPDatabase | null,
    readonly error?: unknown,
  ) {}
  async list(): Promise<Game[]> {
    return this.db
      ? (await this.db.getAll("games")).sort(
          (a: Game, b: Game) => b.lastPlayed - a.lastPlayed,
        )
      : [];
  }
  async rom(hash: string): Promise<Uint8Array | undefined> {
    return this.db?.get("roms", hash);
  }
  async playthroughs(hash?: string): Promise<Playthrough[]> {
    if (!this.db) return [];
    const runs: Playthrough[] = hash
      ? await this.db.getAllFromIndex("playthroughs", "hash", hash)
      : await this.db.getAll("playthroughs");
    return runs.sort((a, b) => b.lastPlayed - a.lastPlayed);
  }
  async getPlaythrough(ref: PlaythroughRef): Promise<Playthrough | undefined> {
    const run = await this.db?.get("playthroughs", runKey(ref));
    return run?.hash === ref.hash && run?.id === ref.playthroughId
      ? run
      : undefined;
  }
  async ensurePlaythrough(hash: string): Promise<Playthrough> {
    if (!this.db) throw new StorageError({ cause: this.error });
    const tx = this.db.transaction(["games", "playthroughs"], "readwrite");
    return finishWrite(tx, async () => {
      const games = tx.objectStore("games");
      const game: Game | undefined = await games.get(hash);
      const runs = tx.objectStore("playthroughs");
      const id = game?.activePlaythroughId ?? "default";
      let run: Playthrough | undefined = await runs.get([hash, id]);
      if (!run) {
        run = {
          hash,
          id,
          name: "Playthrough 1",
          createdAt: Date.now(),
          lastPlayed: game?.lastPlayed ?? Date.now(),
        };
        await runs.put(run, [hash, id]);
      }
      if (game && game.activePlaythroughId !== id)
        await games.put(
          {
            ...game,
            activePlaythroughId: id,
            resumeSavedAt: run.resumeSavedAt,
          },
          hash,
        );
      return run;
    });
  }
  async createPlaythrough(
    hash: string,
    name?: string,
    id: string = crypto.randomUUID(),
  ): Promise<Playthrough> {
    if (!this.db) throw new StorageError({ cause: this.error });
    if (!hash || !id || (!name?.trim() && name !== undefined))
      throw new Error("Playthrough identity and name required.");
    const tx = this.db.transaction(["games", "playthroughs"], "readwrite");
    return finishWrite(tx, async () => {
      const runs = tx.objectStore("playthroughs");
      if (await runs.get([hash, id]))
        throw new Error("Playthrough already exists.");
      const game: Game | undefined = await tx.objectStore("games").get(hash);
      const run: Playthrough = {
        hash,
        id,
        name:
          name?.trim() ??
          `Playthrough ${(await runs.index("hash").getAll(hash)).length + 1}`,
        createdAt: Date.now(),
        lastPlayed: Date.now(),
      };
      await runs.put(run, [hash, id]);
      if (game) {
        const { resumeSavedAt: _, ...rest } = game;
        await tx
          .objectStore("games")
          .put(
            { ...rest, activePlaythroughId: id, lastPlayed: run.lastPlayed },
            hash,
          );
      }
      return run;
    });
  }
  async selectPlaythrough(ref: PlaythroughRef): Promise<Playthrough> {
    if (!this.db) throw new StorageError({ cause: this.error });
    const tx = this.db.transaction(["games", "playthroughs"], "readwrite");
    return finishWrite(tx, async () => {
      const run = await requireRun(tx, ref);
      const now = Date.now();
      const selected = { ...run, lastPlayed: now };
      await tx.objectStore("playthroughs").put(selected, runKey(ref));
      const game: Game | undefined = await tx
        .objectStore("games")
        .get(ref.hash);
      if (game) {
        const { resumeSavedAt: _, ...rest } = game;
        await tx.objectStore("games").put(
          {
            ...rest,
            activePlaythroughId: ref.playthroughId,
            lastPlayed: now,
            ...(run.resumeSavedAt === undefined
              ? {}
              : { resumeSavedAt: run.resumeSavedAt }),
          },
          ref.hash,
        );
      }
      return selected;
    });
  }
  async renamePlaythrough(ref: PlaythroughRef, name: string) {
    if (!name.trim()) throw new Error("Name required");
    if (!this.db) throw new StorageError({ cause: this.error });
    const tx = this.db.transaction("playthroughs", "readwrite");
    await finishWrite(tx, async () => {
      const run = await requireRun(tx, ref);
      await tx.store.put({ ...run, name: name.trim() }, runKey(ref));
    });
  }
  async put(game: GameImport): Promise<Game> {
    if (!this.db) throw new StorageError({ cause: this.error });
    const tx = this.db.transaction(
      ["games", "roms", "playthroughs"],
      "readwrite",
    );
    return finishWrite(tx, async () => {
      const existing = await tx.objectStore("games").get(game.hash);
      let run: Playthrough | undefined = await tx
        .objectStore("playthroughs")
        .get([game.hash, existing?.activePlaythroughId ?? "default"]);
      if (!run) {
        run = {
          hash: game.hash,
          id: "default",
          name: "Playthrough 1",
          createdAt: Date.now(),
          lastPlayed: game.lastPlayed,
        };
        await tx.objectStore("playthroughs").put(run, [game.hash, "default"]);
      }
      const info = {
        ...metadata(game),
        title: existing?.title ?? game.title,
        activePlaythroughId: run.id,
        ...(run.resumeSavedAt === undefined
          ? {}
          : { resumeSavedAt: run.resumeSavedAt }),
      };
      // The key is the ROM's content hash. Reopening a stored game only updates
      // its metadata; it must not clone and rewrite the ROM on every selection.
      if (!existing) await tx.objectStore("roms").put(game.rom, game.hash);
      await tx.objectStore("games").put(info, game.hash);
      return info;
    });
  }
  async rename(hash: string, title: string) {
    if (!title.trim()) throw new Error("Title required");
    if (!this.db) throw new StorageError({ cause: this.error });
    const tx = this.db.transaction("games", "readwrite");
    const game = await tx.store.get(hash);
    if (game) await tx.store.put({ ...game, title: title.trim() }, hash);
    await tx.done;
  }
  async remove(hash: string) {
    if (!this.db) throw new StorageError({ cause: this.error });
    const tx = this.db.transaction(["games", "roms"], "readwrite");
    await tx.objectStore("games").delete(hash);
    await tx.objectStore("roms").delete(hash);
    await tx.done;
  }
  async save(
    ref: PlaythroughRef,
    data: Uint8Array,
    lastInputAt = 0,
    expectedAccountId?: string,
  ) {
    if (!this.db) throw new StorageError({ cause: this.error });
    const tx = this.db.transaction(
      ["saves", "cloud", "playthroughs"],
      "readwrite",
    );
    await finishWrite(tx, async () => {
      await requireRun(tx, ref);
      const store = tx.objectStore("saves");
      const old = await store.get(runKey(ref));
      const updatedAt = Date.now();
      const cloud = await queueCloud(
        tx,
        ref,
        "battery",
        updatedAt,
        lastInputAt,
        expectedAccountId,
      );
      await store.put(
        { data, previous: old?.data, updatedAt, ...cloud },
        runKey(ref),
      );
    });
  }
  async getSave(ref: PlaythroughRef): Promise<Uint8Array | undefined> {
    return (await this.db?.get("saves", runKey(ref)))?.data;
  }
  async getResume(ref: PlaythroughRef): Promise<ResumeCheckpoint | undefined> {
    return this.db?.get("resumes", runKey(ref));
  }
  async saveResume(
    ref: PlaythroughRef,
    data: Uint8Array,
    options: LoadOptions,
    cycle: bigint,
    lastInputAt = 0,
    expectedAccountId?: string,
  ) {
    if (!this.db) throw new StorageError({ cause: this.error });
    const tx = this.db.transaction(
      ["games", "resumes", "cloud", "playthroughs"],
      "readwrite",
    );
    return finishWrite(tx, async () => {
      const run = await requireRun(tx, ref);
      const games = tx.objectStore("games");
      const game: Game | undefined = await games.get(ref.hash);
      const resumes = tx.objectStore("resumes");
      const old = (await resumes.get(runKey(ref))) as
        | ResumeCheckpoint
        | undefined;
      const savedAt = Date.now();
      const current: ResumeGeneration = {
        data,
        options,
        cycle: cycle.toString(),
        savedAt,
        ...(await queueCloud(
          tx,
          ref,
          "resume",
          savedAt,
          lastInputAt,
          expectedAccountId,
        )),
      };
      await resumes.put(
        {
          ...current,
          ...(old
            ? {
                previous: {
                  data: old.data,
                  options: old.options,
                  cycle: old.cycle,
                  savedAt: old.savedAt,
                },
              }
            : {}),
        },
        runKey(ref),
      );
      await tx
        .objectStore("playthroughs")
        .put(
          { ...run, resumeSavedAt: savedAt, lastPlayed: savedAt },
          runKey(ref),
        );
      if (game?.activePlaythroughId === ref.playthroughId)
        await games.put({ ...game, resumeSavedAt: savedAt }, ref.hash);
      return savedAt;
    });
  }
  async deleteSave(ref: PlaythroughRef) {
    if (!this.db) throw new StorageError({ cause: this.error });
    const tx = this.db.transaction(
      ["games", "saves", "resumes", "cloud", "playthroughs"],
      "readwrite",
    );
    await finishWrite(tx, async () => {
      const run = await requireRun(tx, ref);
      await tx.objectStore("saves").delete(runKey(ref));
      await tx.objectStore("resumes").delete(runKey(ref));
      // Quick slots survive a progress reset, so their sync bases do too.
      for (const job of await tx
        .objectStore("cloud")
        .index("hash")
        .getAll(ref.hash))
        if (
          job.playthroughId === ref.playthroughId &&
          (job.kind === "resume" || job.kind === "battery")
        )
          await tx.objectStore("cloud").delete(cloudKey(job));
      const { resumeSavedAt: _, ...withoutResume } = run;
      await tx.objectStore("playthroughs").put(withoutResume, runKey(ref));
      const games = tx.objectStore("games");
      const game: Game | undefined = await games.get(ref.hash);
      if (game?.activePlaythroughId === ref.playthroughId) {
        const { resumeSavedAt: _resume, ...withoutProjection } = game;
        await games.put(withoutProjection, ref.hash);
      }
    });
  }
  async deletePlaythrough(ref: PlaythroughRef) {
    if (!this.db) throw new StorageError({ cause: this.error });
    const tx = this.db.transaction(
      ["games", "playthroughs", "saves", "resumes", "cloud", "assets"],
      "readwrite",
    );
    return finishWrite(tx, async () => {
      await requireRun(tx, ref);
      const runs = tx.objectStore("playthroughs");
      const remaining = (await runs.index("hash").getAll(ref.hash))
        .filter((run) => run.id !== ref.playthroughId)
        .sort((a, b) => b.lastPlayed - a.lastPlayed);
      const created = remaining.length === 0;
      const next: Playthrough = remaining[0] ?? {
        hash: ref.hash,
        id: crypto.randomUUID(),
        name: "Playthrough 1",
        createdAt: Date.now(),
        lastPlayed: Date.now(),
      };
      if (created)
        await runs.put(
          next,
          runKey({ hash: ref.hash, playthroughId: next.id }),
        );
      await runs.delete(runKey(ref));
      await tx.objectStore("saves").delete(runKey(ref));
      await tx.objectStore("resumes").delete(runKey(ref));
      for (const job of await tx
        .objectStore("cloud")
        .index("hash")
        .getAll(ref.hash))
        if (job.playthroughId === ref.playthroughId)
          await tx.objectStore("cloud").delete(cloudKey(job));
      for (let slot = 1; slot <= QUICK_SLOTS; slot++)
        await tx.objectStore("assets").delete(quickStateKey(ref, slot));
      const games = tx.objectStore("games");
      const game: Game | undefined = await games.get(ref.hash);
      if (game?.activePlaythroughId === ref.playthroughId) {
        const { resumeSavedAt: _, ...withoutResume } = game;
        await games.put(
          {
            ...withoutResume,
            activePlaythroughId: next.id,
            ...(next.resumeSavedAt === undefined
              ? {}
              : { resumeSavedAt: next.resumeSavedAt }),
          },
          ref.hash,
        );
      }
      return { next, created };
    });
  }
  async cloudIdentity(): Promise<CloudIdentity> {
    return (
      (await this.db?.get("cloud", "identity")) ?? {
        accountId: null,
        enabled: false,
      }
    );
  }
  async setCloudIdentity(accountId: string | null, enabled?: boolean) {
    if (!this.db) throw new StorageError({ cause: this.error });
    const tx = this.db.transaction("cloud", "readwrite");
    const identity = {
      accountId,
      enabled: accountId
        ? (enabled ?? (await tx.store.get(`enabled:${accountId}`)) ?? true)
        : false,
    };
    if (accountId && enabled !== undefined)
      await tx.store.put(enabled, `enabled:${accountId}`);
    await tx.store.put(identity, "identity");
    await tx.done;
    return identity;
  }
  async forgetCloudAccount(accountId: string) {
    if (!this.db) throw new StorageError({ cause: this.error });
    const tx = this.db.transaction("cloud", "readwrite");
    await finishWrite(tx, async () => {
      for (const job of await tx.store.index("accountId").getAll(accountId))
        if (isCloudKind(job.kind)) await tx.store.delete(cloudKey(job));
      await tx.store.delete(`enabled:${accountId}`);
      const identity: CloudIdentity | undefined =
        await tx.store.get("identity");
      if (identity?.accountId === accountId)
        await tx.store.put({ accountId: null, enabled: false }, "identity");
    });
  }
  async cloudJobs(accountId: string): Promise<CloudJob[]> {
    return (
      (await this.db?.getAllFromIndex("cloud", "accountId", accountId)) ?? []
    ).filter((value): value is CloudJob => isCloudKind(value.kind));
  }
  async cloudSource(job: CloudJob): Promise<Uint8Array | undefined> {
    const slot = quickSlotOf(job.kind);
    if (slot !== undefined) {
      const quick = await this.getQuickSlot(job, slot);
      return quick?.cloudRevision === job.revision &&
        quick.accountId === job.accountId
        ? quick.data
        : undefined;
    }
    const source = await this.db?.get(
      job.kind === "resume" ? "resumes" : "saves",
      runKey(job),
    );
    return source?.cloudRevision === job.revision ? source.data : undefined;
  }
  async beginCloud(job: CloudJob, digest: string, manual = false) {
    if (!this.db) return false;
    const tx = this.db.transaction("cloud", "readwrite");
    const identity: CloudIdentity = await tx.store.get("identity");
    const current: CloudJob | undefined = await tx.store.get(cloudKey(job));
    const allowed =
      identity?.accountId === job.accountId &&
      identity.enabled &&
      current?.revision === job.revision &&
      !current.conflict &&
      (manual || cloudEligible(current, Date.now()));
    if (allowed)
      await tx.store.put(
        { ...current, flight: { revision: job.revision, digest } },
        cloudKey(job),
      );
    await tx.done;
    return allowed;
  }
  async acknowledgeCloud(
    job: CloudJob,
    revision: string,
    etag: string,
    uploadedAt: number,
    confirmedAt = uploadedAt,
  ) {
    if (!this.db) return;
    const tx = this.db.transaction("cloud", "readwrite");
    const current: CloudJob | undefined = await tx.store.get(cloudKey(job));
    // An old acknowledgement advances the remote base, never the pending
    // revision. Deletion or replacement of the flight cannot resurrect a job.
    if (current?.flight?.revision === revision) {
      await tx.store.put(
        {
          ...current,
          baseEtag: etag,
          uploadedRevision: revision,
          uploadedAt,
          confirmedAt,
          flight: undefined,
          attempts: 0,
          retryAt: 0,
          conflict: false,
          remote: undefined,
          prompted: undefined,
        },
        cloudKey(job),
      );
    }
    await tx.done;
  }
  async cloudFailure(job: CloudJob, conflict = false) {
    if (!this.db) return;
    const tx = this.db.transaction("cloud", "readwrite");
    const current: CloudJob | undefined = await tx.store.get(cloudKey(job));
    if (current) {
      const attempts = Math.min((current.attempts ?? 0) + 1, 8);
      await tx.store.put(
        {
          ...current,
          conflict,
          attempts,
          retryAt: Date.now() + Math.min(60 * 60_000, 15_000 * 2 ** attempts),
        },
        cloudKey(job),
      );
    }
    await tx.done;
  }
  async adoptCloudBase(
    accountId: string,
    ref: PlaythroughRef,
    kind: CloudKind,
    etag?: string,
  ) {
    if (!this.db) throw new StorageError({ cause: this.error });
    const tx = this.db.transaction(["cloud", "playthroughs"], "readwrite");
    await finishWrite(tx, async () => {
      await requireRun(tx, ref);
      const key = [accountId, ref.hash, ref.playthroughId, kind];
      const current: CloudJob | undefined = await tx
        .objectStore("cloud")
        .get(key);
      await tx.objectStore("cloud").put(
        {
          ...current,
          accountId,
          ...ref,
          kind,
          revision: current?.revision ?? "",
          capturedAt: current?.capturedAt ?? 0,
          lastInputAt: 0,
          baseEtag: etag,
          uploadedRevision: undefined,
          flight: undefined,
          conflict: false,
          retryAt: 0,
        },
        key,
      );
    });
  }
  async getQuickSlot(
    ref: PlaythroughRef,
    slot: number,
  ): Promise<QuickSlot | undefined> {
    return quickRecord(await this.db?.get("assets", quickStateKey(ref, slot)));
  }
  async quickSlots(
    ref: PlaythroughRef,
    accountId?: string | null,
  ): Promise<QuickSlotSummary[]> {
    const jobs = accountId ? await this.cloudJobs(accountId) : [];
    const summaries: QuickSlotSummary[] = [];
    for (let slot = 1; slot <= QUICK_SLOTS; slot++) {
      const local = await this.getQuickSlot(ref, slot);
      const job = jobs.find(
        (item) =>
          item.hash === ref.hash &&
          item.playthroughId === ref.playthroughId &&
          item.kind === quickKind(slot),
      );
      summaries.push({
        slot,
        ...(local ? { savedAt: local.savedAt } : {}),
        ...(job?.conflict
          ? { sync: "conflict" as const }
          : local &&
              job &&
              local.accountId === accountId &&
              local.cloudRevision === job.revision
            ? {
                sync:
                  job.uploadedRevision === job.revision
                    ? ("synced" as const)
                    : ("pending" as const),
              }
            : {}),
      });
    }
    return summaries;
  }
  /** Local-first: the slot and its replaceable outbox revision commit together. */
  async saveQuickSlot(ref: PlaythroughRef, slot: number, data: Uint8Array) {
    if (!this.db) throw new StorageError({ cause: this.error });
    if (!Number.isInteger(slot) || slot < 1 || slot > QUICK_SLOTS)
      throw new Error("Unknown quick slot.");
    const tx = this.db.transaction(
      ["assets", "cloud", "playthroughs"],
      "readwrite",
    );
    return finishWrite(tx, async () => {
      await requireRun(tx, ref);
      const savedAt = Date.now();
      const { cloudRevision } = await queueCloud(
        tx,
        ref,
        quickKind(slot),
        savedAt,
        savedAt,
      );
      const identity: CloudIdentity | undefined = await tx
        .objectStore("cloud")
        .get("identity");
      await tx.objectStore("assets").put(
        {
          data,
          savedAt,
          ...(cloudRevision
            ? { cloudRevision, accountId: identity!.accountId! }
            : {}),
        } satisfies QuickSlot,
        quickStateKey(ref, slot),
      );
      return { queued: Boolean(cloudRevision), savedAt };
    });
  }
  /**
   * Applies one reconciliation decision if neither the local slot nor the
   * account changed since it was planned. Returns false when it was stale.
   */
  async settleQuickSlot(
    accountId: string,
    ref: PlaythroughRef,
    slot: number,
    seen: { cloudRevision?: string; savedAt?: number },
    settle:
      | { type: "download"; data: Uint8Array; remote: QuickRemote }
      | { type: "adopt"; remote: QuickRemote }
      | { type: "claim"; baseEtag?: string }
      | { type: "conflict"; remote: QuickRemote },
  ): Promise<boolean> {
    if (!this.db) throw new StorageError({ cause: this.error });
    const tx = this.db.transaction(
      ["assets", "cloud", "playthroughs"],
      "readwrite",
    );
    return finishWrite(tx, async () => {
      await requireRun(tx, ref);
      const cloud = tx.objectStore("cloud");
      const identity: CloudIdentity | undefined = await cloud.get("identity");
      if (identity?.accountId !== accountId || !identity.enabled) return false;
      const assets = tx.objectStore("assets");
      const assetKey = quickStateKey(ref, slot);
      const local = quickRecord(await assets.get(assetKey));
      if (
        local?.cloudRevision !== seen.cloudRevision ||
        local?.savedAt !== seen.savedAt
      )
        return false;
      const kind = quickKind(slot);
      const key = cloudKey({ accountId, ...ref, kind });
      const current: CloudJob | undefined = await cloud.get(key);
      const base = {
        ...current,
        accountId,
        ...ref,
        kind,
        lastInputAt: 0,
        flight: undefined,
        retryAt: 0,
      };
      if (settle.type === "download" || settle.type === "adopt") {
        const { remote } = settle;
        if (settle.type === "download")
          await assets.put(
            {
              data: settle.data,
              savedAt: remote.capturedAt,
              cloudRevision: remote.revision,
              accountId,
            } satisfies QuickSlot,
            assetKey,
          );
        await cloud.put(
          {
            ...base,
            revision: remote.revision,
            uploadedRevision: remote.revision,
            capturedAt: remote.capturedAt,
            baseEtag: remote.etag,
            conflict: false,
            remote: undefined,
            prompted: undefined,
            attempts: 0,
          } satisfies CloudJob,
          key,
        );
        return true;
      }
      if (!local) return false;
      if (settle.type === "conflict") {
        await cloud.put(
          {
            ...base,
            revision: current?.revision ?? "",
            capturedAt: current?.capturedAt ?? local.savedAt,
            conflict: true,
            remote: settle.remote,
          } satisfies CloudJob,
          key,
        );
        return true;
      }
      // Claim: this device's copy becomes the account's pending revision on
      // top of the given cloud ETag (none creates with If-None-Match).
      const revision = crypto.randomUUID();
      await assets.put(
        { ...local, cloudRevision: revision, accountId } satisfies QuickSlot,
        assetKey,
      );
      await cloud.put(
        {
          ...base,
          revision,
          capturedAt: local.savedAt,
          baseEtag: settle.baseEtag,
          uploadedRevision: undefined,
          uploadedAt: 0,
          conflict: false,
          remote: undefined,
          prompted: undefined,
        } satisfies CloudJob,
        key,
      );
      return true;
    });
  }
  /** Records that the user saw these divergences, so they are asked once. */
  async markQuickPrompted(
    accountId: string,
    ref: PlaythroughRef,
    etags: Map<number, string>,
  ) {
    if (!this.db) return;
    const tx = this.db.transaction("cloud", "readwrite");
    await finishWrite(tx, async () => {
      for (const [slot, etag] of etags) {
        const key = cloudKey({ accountId, ...ref, kind: quickKind(slot) });
        const current: CloudJob | undefined = await tx.store.get(key);
        if (current?.remote?.etag === etag)
          await tx.store.put({ ...current, prompted: etag }, key);
      }
    });
  }
  async retain(kind: "scenarios" | "reports", value: unknown) {
    if (!this.db) throw new StorageError({ cause: this.error });
    await this.db.put(kind, value, `${Date.now()}-${crypto.randomUUID()}`);
  }
  async scenarios() {
    return this.db?.getAll("scenarios") ?? [];
  }
  async reports() {
    return this.db?.getAll("reports") ?? [];
  }
  async remember(item: ScenarioExport) {
    if (!this.db) throw new StorageError({ cause: this.error });
    const tx = this.db.transaction(["assets", "scenarios"], "readwrite");
    for (const [name, bytes] of Object.entries(item.assets))
      await tx
        .objectStore("assets")
        .put(bytes, item.manifest.assets[name].sha256);
    await tx.objectStore("scenarios").put(item.manifest, crypto.randomUUID());
    await tx.done;
  }
  async asset(hash: string): Promise<Uint8Array | undefined> {
    return (await this.db?.get("assets", hash)) ?? (await this.rom(hash));
  }
  async addAsset(hash: string, bytes: Uint8Array) {
    if (!this.db) throw new StorageError({ cause: this.error });
    await this.db.put("assets", bytes, hash);
  }
  close() {
    this.db?.close();
  }
}
export async function openLibrary() {
  let blocked = false;
  let rejectBlocked: (error: Error) => void = () => {};
  const failure = new Promise<never>((_, reject) => {
    rejectBlocked = reject;
  });
  const opened = openDB("zigba-local", 6, {
    upgrade(db, oldVersion, _newVersion, tx) {
      for (const name of [
        "saves",
        "games",
        "roms",
        "resumes",
        "assets",
        "scenarios",
        "reports",
      ])
        if (!db.objectStoreNames.contains(name)) db.createObjectStore(name);
      if (!db.objectStoreNames.contains("cloud")) {
        const cloud = db.createObjectStore("cloud");
        cloud.createIndex("accountId", "accountId");
        cloud.createIndex("hash", "hash");
      }
      const runs = db.createObjectStore("playthroughs");
      runs.createIndex("hash", "hash");
      // Keep all v1-v5 conversions in one version transaction. Cursor iteration
      // avoids loading ROMs, saves and snapshots into one large JS allocation.
      const migrate = async () => {
        const games = tx.objectStore("games");
        let cursor = await games.openCursor();
        while (cursor) {
          const old = cursor.value as Game & { rom?: Uint8Array };
          const game =
            oldVersion < 3 && old.rom ? metadata(old as GameImport) : old;
          if (oldVersion < 3 && old.rom)
            await tx.objectStore("roms").put(old.rom, cursor.primaryKey);
          await runs.put(
            {
              hash: game.hash,
              id: "default",
              name: "Playthrough 1",
              createdAt: game.lastPlayed,
              lastPlayed: game.lastPlayed,
              ...(game.resumeSavedAt === undefined
                ? {}
                : { resumeSavedAt: game.resumeSavedAt }),
            },
            [game.hash, "default"],
          );
          await cursor.update({ ...game, activePlaythroughId: "default" });
          cursor = await cursor.continue();
        }
        for (const storeName of ["saves", "resumes"] as const) {
          const store = tx.objectStore(storeName);
          let entry = await store.openCursor();
          while (entry) {
            if (typeof entry.primaryKey !== "string") {
              entry = await entry.continue();
              continue;
            }
            const hash = entry.primaryKey as string;
            if (!(await runs.get([hash, "default"]))) {
              const savedAt =
                storeName === "resumes"
                  ? entry.value.savedAt
                  : entry.value.updatedAt;
              await runs.put(
                {
                  hash,
                  id: "default",
                  name: "Playthrough 1",
                  createdAt: savedAt ?? 0,
                  lastPlayed: savedAt ?? 0,
                  ...(storeName === "resumes"
                    ? { resumeSavedAt: savedAt }
                    : {}),
                },
                [hash, "default"],
              );
            } else if (storeName === "resumes") {
              const run: Playthrough = await runs.get([hash, "default"]);
              if (run.resumeSavedAt === undefined)
                await runs.put({ ...run, resumeSavedAt: entry.value.savedAt }, [
                  hash,
                  "default",
                ]);
            }
            await store.put(entry.value, [hash, "default"]);
            await entry.delete();
            entry = await entry.continue();
          }
        }
        const cloud = tx.objectStore("cloud");
        let jobCursor = await cloud.openCursor();
        while (jobCursor) {
          const key = jobCursor.primaryKey;
          if (Array.isArray(key) && key.length === 3) {
            const job = jobCursor.value as CloudJob;
            await cloud.put({ ...job, playthroughId: "default" }, [
              job.accountId,
              job.hash,
              "default",
              job.kind,
            ]);
            await jobCursor.delete();
          }
          jobCursor = await jobCursor.continue();
        }
        const assets = tx.objectStore("assets");
        let assetCursor = await assets.openCursor();
        while (assetCursor) {
          const key = assetCursor.primaryKey;
          if (typeof key === "string" && key.startsWith("quick-state:")) {
            const match = /^quick-state:(.+):(\d+)$/.exec(key);
            if (match) {
              await assets.put(
                assetCursor.value,
                quickStateKey(
                  { hash: match[1], playthroughId: "default" },
                  Number(match[2]),
                ),
              );
              await assetCursor.delete();
            }
          }
          assetCursor = await assetCursor.continue();
        }
      };
      void tx.done.catch(() => db.close());
      void migrate().catch(() => {
        try {
          tx.abort();
        } catch {
          // A failed IndexedDB request may already have aborted the upgrade.
        }
      });
    },
    blocked() {
      blocked = true;
      rejectBlocked(
        new Error(
          "Close older ZiGBA tabs to upgrade storage. Export saves until then.",
        ),
      );
    },
    blocking() {
      void opened.then((db) => db.close());
    },
  });
  void opened
    .then((db) => {
      if (blocked) db.close();
    })
    .catch(() => {});
  return new Library(await Promise.race([opened, failure]));
}
export class GameLibrary extends Context.Tag("GameLibrary")<
  GameLibrary,
  Library
>() {}
export const GameLibraryLive = Layer.scoped(
  GameLibrary,
  Effect.acquireRelease(
    Effect.tryPromise({
      try: openLibrary,
      catch: (cause) => new StorageError({ cause }),
    }).pipe(
      Effect.catchAll((error) => Effect.succeed(new Library(null, error))),
    ),
    (library) => Effect.sync(() => library.close()),
  ),
);
