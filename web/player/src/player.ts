import {
  EmulatorSession,
  makeEmulator,
  runtime,
  type Session,
} from "./services";
import {
  GameLibrary,
  type Game,
  type Library,
  type Playthrough,
  type PlaythroughRef,
  type QuickSlotSummary,
  type ResumeCheckpoint,
  type ResumeGeneration,
  type SaveKind,
  playthroughLockKey,
} from "./library";
import {
  CloudSync,
  type CloudResume,
  type CloudStatus,
  type QuickConflict,
  type SharedState,
} from "./cloud";
import { acquireSaveLock } from "./storage.js";
import { type Emulator, type Snapshot, type LoadOptions } from "@evenfall/gba";
import { AudioOutput, VideoOutput } from "./media";
import { previousRewindCheckpoint } from "./rewind-model";
import { WriteStats } from "./write-stats";
import { isLegacyState, LegacyStateError } from "./state-format";

/** Shown when a resume point from the previous core falls back to the battery save. */
/**
 * Persisted rewind checkpoints are at least this many captures apart, about
 * ten seconds of play at any speed, since captures scale with speed. Memory
 * keeps one per second; a reload keeps one per ten.
 */
const REWIND_PERSIST_GAP = 10;
export const LEGACY_RESUME_MESSAGE =
  "This resume point was saved by an older version of Evenfall and can't be loaded. Started from the in-game save instead; the old resume point is kept.";
export class Player {
  emulator!: Emulator;
  session!: Session;
  library!: Library;
  cloud!: CloudSync;
  private lastInputAt = 0;
  audio = new AudioOutput();
  canvas?: HTMLCanvasElement;
  listeners = new Set<() => void>();
  get releaseLock() {
    return this.session?.releaseLock;
  }
  saveTail = Promise.resolve();
  private lastSaved?: Uint8Array;
  private lastResumeCycle?: bigint;
  private persistenceError?: unknown;
  private persistenceEpoch = 0;
  /** Rewind payload keys stored for `ref`, valid while `epoch` holds. */
  private storedRewind?: {
    ref: PlaythroughRef;
    epoch: number;
    keys: ReadonlySet<string>;
  };
  private resumeBlockedRef?: PlaythroughRef;
  /** Autosave write statistics, for the console: `evenfallWrites.report()`. */
  readonly writeStats = new WriteStats();
  private activeOptions: LoadOptions = {};
  options: LoadOptions = { saveType: "auto" };
  private video = new VideoOutput((pixels) =>
    this.emulator.recyclePresentation(pixels),
  );
  private rewindEpoch = 0;
  private rewindResume = false;
  private rewindBusy = false;
  private lastRewindAt = -Infinity;
  private activeRewindTimeline?: string;
  keyboard = 0;
  touch = 0;
  gamepad = 0;
  sent = 0;
  state: {
    ready: boolean;
    playing: boolean;
    speed: number;
    boosting: boolean;
    rewinding: boolean;
    rewindTimeline: boolean;
    changing: boolean;
    resumeFailed?: boolean;
    message: string;
    saveMessage: string;
    cloud: CloudStatus;
    games: Game[];
    runs: Playthrough[];
    game?: Game;
    playthrough?: Playthrough;
    playthroughs: Playthrough[];
    quickSlots: QuickSlotSummary[];
  } = {
    ready: false,
    playing: false,
    speed: 1,
    boosting: false,
    rewinding: false,
    rewindTimeline: false,
    changing: false,
    message: "Starting…",
    saveMessage: "",
    cloud: {
      accountId: null,
      enabled: false,
      message: "Cloud backups are off.",
    },
    games: [],
    runs: [],
    playthroughs: [],
    quickSlots: [],
  };
  subscribe = (f: () => void) => {
    this.listeners.add(f);
    return () => {
      this.listeners.delete(f);
    };
  };
  snapshot = () => this.state;
  update(p: Partial<Player["state"]>) {
    this.state = { ...this.state, ...p };
    this.listeners.forEach((f) => f());
  }
  error = (e: unknown) =>
    this.update({ message: e instanceof Error ? e.message : String(e) });
  async init() {
    this.session = await runtime.runPromise(EmulatorSession);
    this.emulator = this.session.emulator;
    this.audio.onRecycle = (samples) =>
      this.emulator.recyclePresentation(undefined, samples);
    this.session.onClose(() => this.audio.close());
    this.session.onClose(() => this.video.clear());
    this.session.onClose(() => this.invalidateRewindTimeline());
    (window as unknown as { emulator: Emulator }).emulator = this.emulator;
    (window as unknown as { player: Player }).player = this;
    (window as unknown as { evenfallWrites: WriteStats }).evenfallWrites =
      this.writeStats;
    this.library = await runtime.runPromise(GameLibrary);
    this.cloud = new CloudSync(this.library, (cloud) => this.update({ cloud }));
    this.cloud.activeRun = () => this.activeRef;
    this.cloud.quickChanged = (ref) => {
      if (this.matches(ref)) void this.refreshQuickSlots();
    };
    this.session.onClose(() => this.cloud.close());
    void this.cloud.start().catch(() =>
      this.update({
        cloud: {
          ...this.state.cloud,
          message: "Cloud sync unavailable. Local play is unaffected.",
        },
      }),
    );
    this.emulator.addEventListener("frame", (e) => {
      const d = (e as CustomEvent).detail;
      if (this.canvas) this.video.send(this.canvas, d.pixels);
      else this.emulator.recyclePresentation(d.pixels);
      this.audio.send(d.audio);
      this.latest = d.state;
    });
    this.emulator.addEventListener("clear", (event) => {
      // Preserve the last accepted image on pause before retiring its buffers.
      this.video.flush();
      this.audio.clear();
      const timeline = (event as CustomEvent).detail.timeline as string | null;
      if (
        this.state.rewindTimeline &&
        this.activeRewindTimeline &&
        timeline !== this.activeRewindTimeline
      ) {
        this.invalidateRewindTimeline();
        this.update({
          message: "Rewind history changed. Open the timeline again.",
        });
      }
    });
    this.emulator.addEventListener("stopped", (e) => {
      this.releaseKeys();
      this.audio.clear();
      this.update({
        playing: false,
        message:
          (e as CustomEvent).detail.state.fault ??
          "CPU stopped. Press Play to resume.",
      });
      void this.persist();
    });
    this.emulator.addEventListener("error", (e) =>
      this.update({
        playing: false,
        message: (e as CustomEvent).detail.message,
      }),
    );
    this.update({
      ready: true,
      message: "Open a game to begin.",
      games: await this.library.list(),
      runs: await this.library.playthroughs(),
      saveMessage: this.library.db
        ? "Games and saves stay in this browser."
        : "Storage unavailable. Export your saves.",
    });
    const saveTimer = setInterval(() => void this.persist(), 5000);
    this.session.onClose(() => {
      clearInterval(saveTimer);
    });
  }
  latest?: Snapshot;
  async refresh() {
    this.update({
      games: await this.library.list(),
      runs: await this.library.playthroughs(),
      playthroughs: this.state.game
        ? await this.library.playthroughs(this.state.game.hash)
        : [],
    });
  }
  get activeRef(): PlaythroughRef | undefined {
    const game = this.state.game;
    const playthrough = this.state.playthrough;
    return game && playthrough
      ? { hash: game.hash, playthroughId: playthrough.id }
      : undefined;
  }
  private matches(ref: PlaythroughRef) {
    return (
      this.activeRef?.hash === ref.hash &&
      this.activeRef?.playthroughId === ref.playthroughId
    );
  }
  private blocked(ref: PlaythroughRef) {
    return (
      this.resumeBlockedRef?.hash === ref.hash &&
      this.resumeBlockedRef?.playthroughId === ref.playthroughId
    );
  }
  async pause(flushDuringChange = false) {
    this.invalidateRewindTimeline();
    if (!this.state.game) return;
    this.releaseKeys();
    this.update({ playing: false });
    this.latest = (await this.emulator.pause()) ?? this.latest;
    this.audio.clear();
    await this.persist(flushDuringChange);
  }
  persist(force = false, expectedAccountId?: string) {
    if ((this.state.changing || this.state.rewinding) && !force) {
      this.writeStats.skip("busy");
      return this.saveTail;
    }
    const game = this.state.game;
    const ref = this.activeRef;
    const lock = this.releaseLock;
    const epoch = this.persistenceEpoch;
    if (
      !game ||
      !ref ||
      !lock ||
      this.state.resumeFailed ||
      this.blocked(ref)
    ) {
      this.writeStats.skip(
        !game || !ref ? "no game" : !lock ? "no save lock" : "resume blocked",
      );
      return this.saveTail;
    }
    this.saveTail = this.saveTail
      .then(async () => {
        if (
          !this.matches(ref) ||
          this.releaseLock !== lock ||
          this.persistenceEpoch !== epoch ||
          this.state.resumeFailed ||
          this.blocked(ref)
        )
          return;
        this.persistenceError = undefined;
        const started = performance.now();
        const playing = this.state.playing;
        const cpu = await this.emulator.inspect();
        if (
          cpu.romHash !== game.hash ||
          (!force && cpu.cycle === this.lastResumeCycle)
        ) {
          this.writeStats.skip(
            cpu.romHash !== game.hash ? "game changed" : "unchanged",
          );
          return;
        }
        const data = await this.emulator.exportSave();
        const known =
          this.storedRewind?.epoch === epoch &&
          this.matches(this.storedRewind.ref)
            ? this.storedRewind.keys
            : await this.library.rewindKeys(ref);
        // One call returns the state and the history that matches it, copying
        // out only payloads that aren't stored yet. Stored checkpoints stay,
        // and new ones are kept REWIND_PERSIST_GAP captures apart.
        const rewind = await this.emulator.saveRewindParts(
          known,
          REWIND_PERSIST_GAP,
        );
        if (
          !this.matches(ref) ||
          this.releaseLock !== lock ||
          this.persistenceEpoch !== epoch ||
          this.state.resumeFailed ||
          this.blocked(ref)
        )
          return;
        const batteryChanged =
          data.length > 0 &&
          !(
            this.lastSaved?.length === data.length &&
            data.every((byte, i) => byte === this.lastSaved![i])
          );
        if (this.sent && this.state.playing) this.lastInputAt = Date.now();
        if (batteryChanged)
          await this.library.save(
            ref,
            data,
            this.lastInputAt,
            expectedAccountId,
          );
        const { savedAt, rewindKeys, written } = await this.library.saveResume(
          ref,
          rewind.state,
          this.activeOptions,
          cpu.cycle,
          this.lastInputAt,
          expectedAccountId,
          rewind,
        );
        this.storedRewind = { ref, epoch, keys: rewindKeys };
        this.writeStats.record({
          ...written,
          playing,
          ms: performance.now() - started,
          stateRaw: rewind.state.byteLength,
          battery: batteryChanged ? data.byteLength : 0,
          largestPayload: Math.max(
            0,
            ...[...rewind.payloads.values()].map((bytes) => bytes.byteLength),
          ),
          historyItems: rewind.keys.length,
        });
        this.lastSaved = data;
        this.lastResumeCycle = cpu.cycle;
        const stored = { ...game, resumeSavedAt: savedAt };
        const playthrough = {
          ...this.state.playthrough!,
          resumeSavedAt: savedAt,
        };
        this.update({
          game: stored,
          playthrough,
          playthroughs: this.state.playthroughs.map((run) =>
            run.id === ref.playthroughId ? playthrough : run,
          ),
          runs: this.state.runs.map((run) =>
            run.hash === ref.hash && run.id === ref.playthroughId
              ? playthrough
              : run,
          ),
          games: this.state.games.map((item) =>
            item.hash === game.hash ? stored : item,
          ),
          saveMessage: "Progress stored locally.",
        });
      })
      .catch((e) => {
        this.storedRewind = undefined;
        this.persistenceError = e;
        this.writeStats.fail(e);
        this.update({
          saveMessage: `Progress save failed: ${String(e)}. Export save to keep progress.`,
        });
      });
    return this.saveTail;
  }
  checkpointState() {
    this.lastResumeCycle = undefined;
    return this.persist(true);
  }
  async refreshQuickSlots() {
    const ref = this.activeRef;
    const quickSlots = ref
      ? await this.library.quickSlots(
          ref,
          (await this.library.cloudIdentity()).accountId,
        )
      : [];
    if (ref ? this.matches(ref) : !this.activeRef) this.update({ quickSlots });
  }
  /** Local slots are shown at once; cloud reconciliation follows silently. */
  async syncQuickSlots() {
    const ref = this.activeRef;
    if (!ref) return this.update({ quickSlots: [] });
    await this.refreshQuickSlots();
    await this.cloud.syncQuick(ref);
  }
  async resolveQuickConflicts(
    choices: { slot: number; keep: "local" | "cloud" }[],
  ) {
    const ref = this.activeRef;
    if (!ref) return;
    await this.cloud.resolveQuick(ref, choices);
    this.update({
      message: "Quick slots reconciled. The copies you chose are kept.",
    });
  }
  markQuickPrompted(conflicts: QuickConflict[]) {
    return this.cloud.markQuickPrompted(conflicts);
  }
  async backupCloud(replace = false) {
    if (!this.state.game || this.state.changing) return;
    if (!this.releaseLock) throw new Error("Another tab owns this save.");
    if (this.state.resumeFailed || this.resumeBlockedRef)
      throw new Error("Resolve the local resume failure before backing up.");
    this.update({ changing: true });
    try {
      await this.pause(true);
      if (this.persistenceError)
        throw new Error(
          "Local progress could not be saved. Export it before switching devices.",
        );
      // Manual backup explicitly includes an unchanged in-game save too.
      const data = await this.emulator.exportSave();
      const ref = this.activeRef!;
      if (data.length) await this.library.save(ref, data, this.lastInputAt);
      await this.cloud.flush(ref, replace);
      this.update({
        message: "Cloud backup confirmed. You can switch devices.",
      });
    } finally {
      this.update({ changing: false });
    }
  }
  async restoreCloud(kind: SaveKind, target?: CloudResume, file?: File) {
    const hash = target?.hash ?? this.state.game?.hash;
    const ref = target
      ? { hash: target.hash, playthroughId: target.playthroughId }
      : this.activeRef;
    if (!this.state.ready) throw new Error("Player is still starting.");
    if (!hash || !ref || this.state.changing) return;
    if (!target && !this.releaseLock)
      throw new Error("Another tab owns this save.");
    this.update({ changing: true });
    let replacing = false;
    try {
      await this.pause(true);
      if (this.persistenceError && this.releaseLock && !this.state.resumeFailed)
        throw new Error(
          "Local progress could not be saved. Export it before switching.",
        );
      await this.cloud.exclusive(async () => {
        let remote = await this.cloud.download(
          ref,
          kind,
          target?.accountId,
          target?.etag,
        );
        // A resume point from the previous core can't be loaded. Restore the
        // playthrough's in-game save instead; the old state stays in the cloud.
        let restored = kind;
        if (kind === "resume" && isLegacyState(remote.data)) {
          remote = await this.cloud
            .download(ref, "battery", remote.accountId)
            .catch(() => {
              throw new LegacyStateError("This cloud resume point");
            });
          restored = "battery";
        }
        if (target) {
          const source =
            file ??
            (await this.library.list()).find((game) => game.hash === hash);
          if (!source)
            throw new Error("Choose the exact ROM used by this backup.");
          if (file && file.size > 40 * 1024 * 1024)
            throw new Error("ROM exceeds 40 MiB");
          const rom = file
            ? new Uint8Array(await file.arrayBuffer())
            : await this.library.rom(hash);
          if (!rom)
            throw new Error("Game is no longer stored. Choose its ROM again.");
          const options = {
            ...((await this.library.getResume(ref))?.options ?? this.options),
            sampleRate: await this.audio.ready(),
            zipped: Boolean(file && /\.zip$/i.test(file.name)),
          };
          const validatedRom = await this.validatedState(
            rom,
            options,
            hash,
            restored === "resume" ? remote.data : undefined,
            "That ROM does not match this backup. Choose the exact same game version.",
          );
          if (
            (await this.library.cloudIdentity()).accountId !== remote.accountId
          )
            throw new Error("Account changed during cloud restore.");
          if (file)
            await this.library.put({
              hash,
              title: file.name.replace(/\.(gba|zip)$/i, ""),
              lastPlayed: Date.now(),
              rom: validatedRom,
            });
          if (!(await this.library.getPlaythrough(ref)))
            await this.library.createPlaythrough(
              hash,
              target.playthroughName,
              ref.playthroughId,
            );
          replacing = true;
          this.resumeBlockedRef = ref;
          await this.openInternal(source, undefined, options, ref, false, true);
          if (!this.releaseLock)
            throw new Error(
              "Another tab owns this save. Close it before restoring.",
            );
        }
        this.latest =
          restored === "resume"
            ? await this.emulator.loadState(remote.data)
            : await this.emulator.resetWithSave(remote.data);
        replacing = true;
        this.lastInputAt = 0;
        this.lastResumeCycle = undefined;
        this.resumeBlockedRef = undefined;
        this.update({ resumeFailed: false });
        await this.persist(true, remote.accountId);
        if (this.persistenceError)
          throw new Error(
            "Cloud progress is loaded, but local persistence failed. Export it before closing.",
          );
        await this.library.adoptCloudBase(
          remote.accountId,
          ref,
          restored,
          remote.etag,
        );
        this.update({
          message:
            restored === kind
              ? "Cloud progress restored and stored locally. Press Play to continue."
              : "The cloud resume point was saved by an older version of Evenfall and can't be loaded. Restored the cloud in-game save instead; the old resume point stays in the cloud.",
        });
        if (target) {
          await this.emulator.setRewindConfig();
          await this.play(true);
        }
      });
    } catch (error) {
      if (replacing) {
        this.resumeBlockedRef = ref;
        this.update({ resumeFailed: true, playing: false });
      }
      throw error;
    } finally {
      this.update({ changing: false });
    }
  }
  /**
   * Loads `rom` and then `state`, if any, in an isolated Worker, so wrong
   * media or an incompatible state never replaces this player's machine or
   * stored progress. Returns the raw ROM, extracted when `options.zipped`.
   */
  private async validatedState(
    rom: Uint8Array,
    options: LoadOptions,
    hash: string,
    state: Uint8Array | undefined,
    mismatch: string,
  ): Promise<Uint8Array> {
    const candidate = await makeEmulator();
    try {
      const cpu = await candidate.load(rom, options);
      if (cpu.romHash !== hash) throw new Error(mismatch);
      if (state) await candidate.loadState(state);
      return options.zipped ? await candidate.exportRom() : rom;
    } finally {
      candidate.terminate();
    }
  }
  /** Captures the current machine as a share link; see CloudSync.share. */
  async shareState() {
    const game = this.state.game;
    const playthrough = this.state.playthrough;
    if (!game || !playthrough || this.state.changing)
      throw new Error("Open a game to share its state.");
    if (this.state.resumeFailed)
      throw new Error("Resolve the local resume failure before sharing.");
    await this.pause();
    const link = await this.cloud.share({
      hash: game.hash,
      title: game.title,
      playthroughName: playthrough.name,
      data: await this.emulator.saveState(),
    });
    return { ...link, hash: game.hash };
  }
  /** Includes the stored ROM `hash` with share `id`; see CloudSync.includeRom. */
  async includeShareRom(id: string, hash: string) {
    const rom = await this.library.rom(hash);
    if (!rom) throw new Error("This game's ROM is no longer on this device.");
    await this.cloud.includeRom(id, rom);
  }
  /**
   * Starts a shared state as a new local playthrough called `name`, from the
   * stored ROM with the state's SHA-1. Nothing existing is replaced: the run
   * gets a fresh ID, and the state is validated against the ROM in an
   * isolated emulator before the player changes.
   */
  async openSharedState(shared: SharedState, name: string) {
    if (!this.state.ready) throw new Error("Player is still starting.");
    if (this.state.changing) return;
    this.update({ changing: true });
    const ref = { hash: shared.hash, playthroughId: crypto.randomUUID() };
    let replacing = false;
    try {
      await this.pause(true);
      if (this.persistenceError && this.releaseLock && !this.state.resumeFailed)
        throw new Error(
          "Local progress could not be saved. Export it before switching.",
        );
      const game = (await this.library.list()).find(
        (item) => item.hash === shared.hash,
      );
      const rom = game && (await this.library.rom(shared.hash));
      if (!game || !rom) throw new Error(`Choose the ROM for ${shared.title}.`);
      const options = {
        ...this.options,
        sampleRate: await this.audio.ready(),
        zipped: false,
      };
      await this.validatedState(
        rom,
        options,
        shared.hash,
        shared.data,
        "The stored ROM does not match this shared state.",
      );
      await this.library.createPlaythrough(
        shared.hash,
        name,
        ref.playthroughId,
      );
      replacing = true;
      this.resumeBlockedRef = ref;
      await this.openInternal(game, undefined, options, ref, false, true);
      if (!this.releaseLock)
        throw new Error(
          "This browser could not lock the new playthrough, so it cannot be stored.",
        );
      this.latest = await this.emulator.loadState(shared.data);
      this.lastInputAt = 0;
      this.lastResumeCycle = undefined;
      this.resumeBlockedRef = undefined;
      this.update({ resumeFailed: false });
      await this.persist(true);
      if (this.persistenceError)
        throw new Error(
          "The shared state is loaded, but local persistence failed. Export it before closing.",
        );
      this.update({ message: "Shared state started as a new playthrough." });
      await this.emulator.setRewindConfig();
      await this.play(true);
    } catch (error) {
      if (replacing) {
        this.resumeBlockedRef = ref;
        this.update({ resumeFailed: true, playing: false });
      }
      throw error;
    } finally {
      this.update({ changing: false });
    }
  }
  // Debug callers opt out of automatic playback and rewind retention.
  async open(
    file: File | Game,
    saveOverride?: Uint8Array,
    autoplay = true,
    ref?: PlaythroughRef,
    fresh = false,
  ) {
    if (!this.state.ready) throw new Error("Player is still starting.");
    if (this.state.changing) return;
    this.update({ changing: true });
    try {
      await this.openInternal(file, saveOverride, undefined, ref, fresh);
      this.resumeBlockedRef = undefined;
      this.update({ resumeFailed: false });
      if (autoplay) {
        await this.emulator.setRewindConfig();
        await this.play(true);
      }
    } finally {
      this.update({ changing: false });
    }
  }
  private async openInternal(
    file: File | Game,
    saveOverride?: Uint8Array,
    resumeOptions?: LoadOptions,
    requested?: PlaythroughRef,
    fresh = false,
    prepared = false,
  ) {
    if (!prepared) {
      await this.pause(true);
      await this.saveTail;
      if (this.persistenceError && this.releaseLock && !this.state.resumeFailed)
        throw new Error(
          "Local progress could not be saved. Export it before switching.",
        );
    }
    this.persistenceEpoch++;
    this.invalidateRewindTimeline();
    const sampleRate = await this.audio.ready();
    if (file instanceof File && file.size > 40 * 1024 * 1024)
      throw new Error("ROM exceeds 40 MiB");
    const rom =
      file instanceof File
        ? new Uint8Array(await file.arrayBuffer())
        : this.state.game?.hash === file.hash
          ? await this.emulator.exportRom()
          : await this.library.rom(file.hash);
    if (!rom) throw new Error("Game is no longer stored. Open its ROM again.");
    if (rom.length > 40 * 1024 * 1024) throw new Error("ROM exceeds 40 MiB");
    // Resolve wall time once at boot, then pin it for saves and recordings.
    const bootOptions = {
      ...(resumeOptions ?? this.options),
      rtcEpoch:
        (resumeOptions ?? this.options).rtcEpoch ??
        BigInt(Math.floor(Date.now() / 1000)),
    };
    const zipped = file instanceof File && /\.zip$/i.test(file.name);
    const cpu = await this.emulator.load(rom, {
      ...bootOptions,
      sampleRate: resumeOptions?.sampleRate ?? sampleRate,
      zipped,
    });
    const raw = zipped ? await this.emulator.exportRom() : rom;
    if (requested && requested.hash !== cpu.romHash)
      throw new Error("That ROM does not match this playthrough.");
    const game = {
      hash: cpu.romHash,
      title:
        file instanceof File
          ? file.name.replace(/\.(gba|zip)$/i, "")
          : file.title,
      romSize: raw.byteLength,
      lastPlayed: Date.now(),
    };
    this.lastSaved = undefined;
    this.lastResumeCycle = undefined;
    this.activeOptions = {
      ...bootOptions,
      sampleRate: resumeOptions?.sampleRate ?? sampleRate,
      zipped: false,
    };
    const canPlayWithoutStorage =
      !requested &&
      (file instanceof File ||
        (!this.activeRef && this.state.game?.hash === cpu.romHash));
    const storageFailure = async (error: unknown, snapshot = cpu) => {
      // The loaded ROM no longer matches the previous writer's machine.
      this.session.ownSaveLock();
      if (canPlayWithoutStorage && saveOverride?.length)
        snapshot = await this.emulator.resetWithSave(saveOverride);
      this.lastInputAt = 0;
      this.latest = snapshot;
      this.update({
        game: canPlayWithoutStorage ? game : undefined,
        playthrough: undefined,
        playthroughs: [],
        quickSlots: [],
        playing: false,
        saveMessage: `Storage unavailable: ${String(error)}. Export your save to keep progress.`,
        message: canPlayWithoutStorage
          ? "Storage unavailable. This game is playable without autosave."
          : "Could not open this stored playthrough.",
      });
      if (canPlayWithoutStorage) {
        this.canvas?.focus();
        return;
      }
      throw error;
    };
    let stored: Game;
    let ref: PlaythroughRef;
    let playthrough: Playthrough;
    try {
      // A new ROM must exist before its first playthrough can be selected.
      stored = await this.library.put({ ...game, rom: raw });
      ref = requested ?? {
        hash: cpu.romHash,
        playthroughId: (await this.library.ensurePlaythrough(cpu.romHash)).id,
      };
      await this.library.selectPlaythrough(ref);
      const selected = await this.library.getPlaythrough(ref);
      if (!selected) throw new Error("Playthrough is no longer available.");
      playthrough = selected;
      const identity = await this.library.cloudIdentity();
      const pending = identity.accountId
        ? await this.library.cloudJobs(identity.accountId)
        : [];
      this.lastInputAt =
        pending.find(
          (job) =>
            job.hash === game.hash &&
            job.playthroughId === ref.playthroughId &&
            job.kind === "resume",
        )?.lastInputAt ?? 0;
    } catch (error) {
      await storageFailure(error);
      return;
    }
    const keepsLock = this.matches(ref) && !!this.releaseLock;
    if (!keepsLock) this.session.ownSaveLock();
    this.update({
      game,
      playthrough,
      playing: false,
      message: "Ready. Press Play.",
    });
    this.latest = cpu;
    try {
      if (!keepsLock)
        this.session.ownSaveLock(
          (await acquireSaveLock(playthroughLockKey(ref))) ?? undefined,
        );
    } catch {
      /* read-only session */
    }
    let save = saveOverride;
    try {
      if (!fresh) save ??= await this.library.getSave(ref);
    } catch (error) {
      await storageFailure(error);
      return;
    }
    if (save?.length) {
      this.latest = await this.emulator.resetWithSave(save);
      this.lastSaved = saveOverride ? undefined : save;
    }
    this.update({
      saveMessage: this.releaseLock
        ? "Autosave active."
        : "Another tab owns this save, or locking is unavailable. Export save manually.",
    });
    try {
      this.update({
        game: stored,
        playthrough: await this.library.getPlaythrough(ref),
      });
      await this.refresh();
      this.update({
        game: this.state.games.find((item) => item.hash === ref.hash) ?? stored,
      });
    } catch (error) {
      await storageFailure(error, this.latest);
      return;
    }
    void this.syncQuickSlots().catch(() => {});
    this.canvas?.focus();
  }
  /**
   * The newest generation of `checkpoint` that loads in an isolated Worker, or
   * undefined when every generation is a state from the previous core. Such a
   * playthrough starts from its battery save; its old states stay stored.
   */
  private async validatedResume(
    game: Game,
    checkpoint: ResumeCheckpoint,
  ): Promise<ResumeGeneration | undefined> {
    const generations = [
      checkpoint,
      ...(checkpoint.previous ? [checkpoint.previous] : []),
    ].filter((generation) => !isLegacyState(generation.data));
    if (!generations.length) return undefined;
    const rom =
      this.state.game?.hash === game.hash
        ? await this.emulator.exportRom()
        : await this.library.rom(game.hash);
    if (!rom) throw new Error("Game is no longer stored. Open its ROM again.");
    const candidate = await makeEmulator();
    let failure: unknown;
    try {
      for (const generation of generations) {
        try {
          await candidate.load(rom, { ...generation.options, zipped: false });
          const cpu = await candidate.loadState(generation.data);
          if (cpu.romHash !== game.hash)
            throw new Error("Resume point belongs to another ROM.");
          return generation;
        } catch (error) {
          failure = error;
        }
      }
      throw new Error("Could not restore local progress.", { cause: failure });
    } finally {
      candidate.terminate();
    }
  }
  /**
   * Loads a resume generation into the paused emulator, with the rewind
   * history stored beside it when that history loads. Returns whether it did.
   */
  private async loadGeneration(
    ref: PlaythroughRef,
    generation: ResumeGeneration,
  ) {
    let history = false;
    if (generation.rewind)
      try {
        this.latest = await this.emulator.loadRewindParts(
          generation.data,
          generation.rewind,
          await this.library.rewindPayloads(ref),
        );
        history = true;
      } catch {
        // History that doesn't load (a payload lost, a newer state format)
        // only costs the history: the state alone was validated already.
      }
    if (!history) this.latest = await this.emulator.loadState(generation.data);
    this.lastResumeCycle = this.latest!.cycle;
    return history;
  }
  async resume(game: Game, autoplay = true) {
    if (!this.state.ready) throw new Error("Player is still starting.");
    if (this.state.changing) return;
    this.update({ changing: true });
    const ref = {
      hash: game.hash,
      playthroughId: game.activePlaythroughId ?? "default",
    };
    try {
      await this.pause(true);
      if (this.persistenceError && this.releaseLock && !this.state.resumeFailed)
        throw new Error(
          "Local progress could not be saved. Export it before switching.",
        );
      this.resumeBlockedRef = ref;
      const checkpoint = await this.library.getResume(ref);
      if (!checkpoint)
        throw new Error("No local resume point is available for this game.");
      const generation = await this.validatedResume(game, checkpoint);
      await this.openInternal(
        game,
        undefined,
        (generation ?? checkpoint).options,
        ref,
        false,
        true,
      );
      const history =
        generation && (await this.loadGeneration(ref, generation));
      this.update({
        resumeFailed: false,
        message: !generation
          ? LEGACY_RESUME_MESSAGE
          : generation === checkpoint
            ? "Resumed local progress."
            : "Recovered the previous local resume point.",
        saveMessage: this.releaseLock
          ? "Autosave active."
          : "Another tab owns this save, or locking is unavailable. Export save manually.",
      });
      if (!history) await this.emulator.setRewindConfig();
      if (autoplay) await this.play(true);
      this.resumeBlockedRef = undefined;
    } catch (error) {
      this.update({
        playing: false,
        resumeFailed: true,
        message:
          error instanceof Error && /No local resume point/.test(error.message)
            ? error.message
            : "Local resume failed. Retry, or explicitly start from the battery save.",
        saveMessage: "The stored resume point was preserved.",
      });
      throw error;
    } finally {
      this.update({ changing: false });
    }
  }
  async switchPlaythrough(playthroughId?: string, name?: string) {
    const game = this.state.game;
    if (!game || this.state.changing) return;
    const outgoing = this.activeRef;
    const blocked = this.resumeBlockedRef;
    const wasPlaying = this.state.playing;
    this.update({ changing: true });
    try {
      await this.pause(true);
      await this.saveTail;
      if (this.persistenceError && this.releaseLock && !this.state.resumeFailed)
        throw new Error(
          "Local progress could not be saved. Export it before switching.",
        );
      const selected = playthroughId
        ? await this.library.getPlaythrough({ hash: game.hash, playthroughId })
        : await this.library.createPlaythrough(game.hash, name);
      if (!selected) throw new Error("Playthrough is no longer available.");
      const ref = { hash: game.hash, playthroughId: selected.id };
      if (this.matches(ref)) return;
      const checkpoint = await this.library.getResume(ref);
      let legacy = false;
      let history = false;
      if (checkpoint) {
        const generation = await this.validatedResume(game, checkpoint);
        legacy = !generation;
        this.resumeBlockedRef = ref;
        await this.openInternal(
          game,
          undefined,
          (generation ?? checkpoint).options,
          ref,
          false,
          true,
        );
        if (generation) history = await this.loadGeneration(ref, generation);
        this.update({ resumeFailed: false });
      } else {
        await this.openInternal(
          game,
          undefined,
          undefined,
          ref,
          !playthroughId,
          true,
        );
      }
      this.resumeBlockedRef = undefined;
      this.update({
        resumeFailed: false,
        message: legacy
          ? LEGACY_RESUME_MESSAGE
          : playthroughId
            ? "Playthrough selected."
            : "New playthrough started.",
      });
      if (!history) await this.emulator.setRewindConfig();
      if (wasPlaying) await this.play(true);
    } catch (error) {
      // Creation selects immediately. If the machine never adopted the new
      // run, keep the library's active projection aligned with the old one.
      if (outgoing && this.matches(outgoing)) {
        this.resumeBlockedRef = blocked;
        await this.library.selectPlaythrough(outgoing);
      }
      throw error;
    } finally {
      this.update({ changing: false });
    }
  }
  /**
   * Reads a chosen .gba or single-game ZIP in an isolated Worker, returning
   * the raw ROM and its SHA-1 without storing or running it.
   */
  async inspectRom(file: File) {
    if (!this.state.ready) throw new Error("Player is still starting.");
    if (file.size > 40 * 1024 * 1024) throw new Error("ROM exceeds 40 MiB");
    const zipped = /\.zip$/i.test(file.name);
    const rom = new Uint8Array(await file.arrayBuffer());
    const candidate = await makeEmulator();
    try {
      const cpu = await candidate.load(rom, {
        ...this.options,
        sampleRate: await this.audio.ready(),
        zipped,
      });
      return {
        hash: cpu.romHash,
        title: file.name.replace(/\.(gba|zip)$/i, ""),
        rom: zipped ? await candidate.exportRom() : rom,
      };
    } finally {
      candidate.terminate();
    }
  }
  async renamePlaythrough(name: string) {
    const ref = this.activeRef;
    if (!ref || this.state.changing) return;
    await this.library.renamePlaythrough(ref, name);
    this.update({ playthrough: await this.library.getPlaythrough(ref) });
    await this.refresh();
  }
  async deletePlaythrough() {
    const ref = this.activeRef;
    const game = this.state.game;
    if (!ref || !game || this.state.changing) return;
    if (!this.releaseLock)
      throw new Error(
        "Another tab owns this save. Close it before deleting the playthrough.",
      );
    const wasPlaying = this.state.playing;
    let deleted = false;
    this.update({ changing: true });
    try {
      await this.pause(true);
      await this.saveTail;
      if (this.persistenceError && !this.state.resumeFailed)
        throw new Error(
          "Local progress could not be saved. Export it before deleting.",
        );
      const survivor = (await this.library.playthroughs(ref.hash)).find(
        (run) => run.id !== ref.playthroughId,
      );
      const checkpoint =
        survivor &&
        (await this.library.getResume({
          hash: ref.hash,
          playthroughId: survivor.id,
        }));
      const generation =
        checkpoint && (await this.validatedResume(game, checkpoint));
      const { next, created } = await this.cloud.deletePlaythrough(ref);
      deleted = true;
      this.persistenceEpoch++;
      this.session.ownSaveLock();
      const target = { hash: ref.hash, playthroughId: next.id };
      let history = false;
      if (generation && survivor?.id === next.id) {
        this.resumeBlockedRef = target;
        await this.openInternal(
          game,
          undefined,
          generation.options,
          target,
          false,
          true,
        );
        history = await this.loadGeneration(target, generation);
      } else {
        await this.openInternal(
          game,
          undefined,
          undefined,
          target,
          created,
          true,
        );
      }
      this.resumeBlockedRef = undefined;
      this.update({
        resumeFailed: false,
        message:
          checkpoint && !generation && survivor?.id === next.id
            ? `Playthrough deleted. ${LEGACY_RESUME_MESSAGE}`
            : "Playthrough deleted.",
      });
      if (!history) await this.emulator.setRewindConfig();
      if (wasPlaying) await this.play(true);
    } catch (error) {
      if (deleted) {
        this.session.ownSaveLock();
        this.update({
          game: undefined,
          playthrough: undefined,
          playthroughs: [],
        });
        await this.refresh();
      }
      throw error;
    } finally {
      this.update({ changing: false });
    }
  }
  async reset() {
    const game = this.state.game;
    if (!game || this.state.changing || this.state.resumeFailed) return;
    const wasPlaying = this.state.playing;
    this.update({ changing: true });
    try {
      await this.pause(true);
      await this.saveTail;
      if (this.persistenceError && this.releaseLock)
        throw new Error(
          "Local progress could not be saved. Export it before resetting.",
        );
      const battery = await this.emulator.exportSave();
      this.persistenceEpoch++;
      this.invalidateRewindTimeline();
      this.latest = await this.emulator.resetWithSave(battery);
      this.lastSaved = battery;
      this.lastResumeCycle = undefined;
      this.update({ message: "Restarted with the current in-game save." });
      await this.emulator.setRewindConfig();
      if (wasPlaying) await this.play(true);
    } finally {
      this.update({ changing: false });
    }
  }
  get effectiveSpeed() {
    return this.state.boosting ? 16 : this.state.speed;
  }
  setSpeed(speed: number) {
    if (!Number.isFinite(speed) || speed < 0.5 || speed > 16) return;
    this.update({ speed });
    this.applySpeed();
  }
  boost(boosting: boolean) {
    if (this.state.boosting === boosting) return;
    this.update({ boosting });
    this.applySpeed();
  }
  private applySpeed() {
    this.audio.setSpeed(this.effectiveSpeed);
    if (this.state.ready)
      void this.emulator.setSpeed(this.effectiveSpeed).catch(this.error);
  }
  async play(ownedOperation = false) {
    if (this.state.changing && !ownedOperation) return;
    if (this.state.resumeFailed || (!ownedOperation && this.resumeBlockedRef)) {
      this.update({
        message:
          "Local resume is blocked. Retry it, or explicitly start from the battery save.",
      });
      return;
    }
    this.invalidateRewindTimeline();
    this.releaseKeys();
    if (this.state.playing) return this.pause();
    const audio = this.audio.play();
    void audio.catch((error) =>
      this.error(`Sound unavailable: ${String(error)}. Playback continues.`),
    );
    await this.emulator.play();
    this.update({ playing: true, message: "Playing" });
    this.canvas?.focus();
  }
  private invalidateRewindTimeline() {
    this.rewindEpoch++;
    this.activeRewindTimeline = undefined;
    if (this.state.rewindTimeline) this.update({ rewindTimeline: false });
  }
  async rewindOnce(now = performance.now()) {
    if (
      !this.state.game ||
      this.state.changing ||
      this.state.rewindTimeline ||
      this.rewindBusy ||
      now - this.lastRewindAt < 300
    )
      return;
    this.lastRewindAt = now;
    this.rewindBusy = true;
    const resume = this.state.playing;
    let message: string | undefined;
    this.update({ rewinding: true });
    const paused = this.pause();
    const epoch = this.rewindEpoch;
    try {
      await paused;
      if (epoch !== this.rewindEpoch) return;
      const timeline = await this.emulator.rewindCheckpoints();
      if (epoch !== this.rewindEpoch) return;
      const checkpoint = previousRewindCheckpoint(timeline);
      if (!checkpoint) {
        message =
          "No rewind checkpoint at least one second earlier is available yet.";
        return;
      }
      const cpu = await this.emulator.rewind(checkpoint.steps);
      if (epoch !== this.rewindEpoch) return;
      this.latest = cpu;
      this.audio.clear();
      this.releaseKeys();
      message = "Rewound to the previous checkpoint.";
      await this.persist(true);
    } catch (error) {
      this.error(error);
    } finally {
      this.rewindBusy = false;
      this.lastRewindAt = performance.now();
      this.update({ rewinding: false });
      if (epoch !== this.rewindEpoch) return;
      if (resume && !this.state.changing && !this.state.rewindTimeline)
        await this.play();
      if (message) this.update({ message });
    }
  }
  async openRewindTimeline() {
    if (
      !this.state.game ||
      this.state.changing ||
      this.state.rewindTimeline ||
      this.rewindBusy
    )
      return;
    const resume = this.state.playing;
    const paused = this.pause();
    const epoch = this.rewindEpoch;
    await paused;
    if (epoch !== this.rewindEpoch) return;
    this.rewindResume = resume;
    this.update({ rewindTimeline: true });
    try {
      const timeline = await this.emulator.rewindCheckpoints();
      if (epoch !== this.rewindEpoch) return;
      if (!timeline.checkpoints.length) {
        await this.closeRewindTimeline();
        this.update({ message: "No rewind checkpoints are available yet." });
        return;
      }
      this.activeRewindTimeline = timeline.timeline;
      return timeline;
    } catch (error) {
      if (epoch === this.rewindEpoch) {
        this.error(error);
        await this.closeRewindTimeline();
      }
    }
  }
  async rewindPreview(timeline: string, index: number) {
    const epoch = this.rewindEpoch;
    if (!this.state.rewindTimeline) return;
    const current = await this.emulator.rewindCheckpoints();
    if (
      epoch !== this.rewindEpoch ||
      current.timeline !== timeline ||
      !current.checkpoints.some((checkpoint) => checkpoint.index === index)
    )
      return;
    let pixels: Uint16Array;
    try {
      pixels = await this.emulator.rewindFrame(index);
    } catch (error) {
      // Loads and other timeline replacements may overtake a thumbnail request.
      // Their clear event invalidates the open UI; that expected stale request
      // must not replace the useful timeline-changed message with an ABI error.
      if (epoch !== this.rewindEpoch || !this.state.rewindTimeline) return;
      throw error;
    }
    if (epoch !== this.rewindEpoch || !this.state.rewindTimeline) return;
    const latest = await this.emulator.rewindCheckpoints();
    if (latest.timeline !== timeline) return;
    return pixels;
  }
  async closeRewindTimeline(selection?: {
    timeline: string;
    index: number;
    steps: number;
  }) {
    if (!this.state.rewindTimeline) return;
    const resume = this.rewindResume;
    const epoch = this.rewindEpoch;
    this.update({ changing: !!selection });
    try {
      if (selection) {
        const current = await this.emulator.rewindCheckpoints();
        const checkpoint = current.checkpoints.find(
          (entry) => entry.index === selection.index,
        );
        if (
          epoch !== this.rewindEpoch ||
          current.timeline !== selection.timeline ||
          checkpoint?.steps !== selection.steps
        )
          throw new Error("Rewind history changed. Open the timeline again.");
        this.activeRewindTimeline = undefined;
        this.latest = await this.emulator.rewind(selection.steps);
        this.audio.clear();
        this.releaseKeys();
        this.update({ message: "Rewind checkpoint restored." });
        await this.persist(true);
      }
    } finally {
      this.update({ changing: false });
      if (epoch === this.rewindEpoch) {
        this.rewindEpoch++;
        this.update({ rewindTimeline: false });
        if (resume) await this.play();
        else this.canvas?.focus();
      }
    }
  }
  keys() {
    if (
      !this.state.game ||
      this.state.changing ||
      this.state.rewinding ||
      this.state.rewindTimeline
    )
      return;
    const keys = this.keyboard | this.touch | this.gamepad;
    if (keys && this.state.playing) this.lastInputAt = Date.now();
    if (keys !== this.sent) {
      this.sent = keys;
      this.writeStats.inputs++;
      void this.emulator.setKeys(keys).catch(this.error);
    }
  }
  releaseKeys() {
    this.boost(false);
    this.keyboard = this.touch = this.gamepad = this.sent = 0;
    if (this.state.game) void this.emulator.setKeys(0).catch(this.error);
  }
}
export const player = new Player();
export const playerReady = player.init();
void playerReady.catch(player.error);
function saveOnHide() {
  // Commit locally before attempting the ordinary, activity/cadence-gated
  // upload. Shutdown may interrupt either step; periodic saves remain primary.
  void player
    .pause()
    .then(() => player.cloud?.flush())
    .catch(player.error);
}
window.addEventListener("pagehide", saveOnHide);
document.addEventListener("visibilitychange", () => {
  if (document.hidden) saveOnHide();
  // Another device may have replaced a quick slot while this tab was away.
  else if (player.cloud) void player.syncQuickSlots().catch(() => {});
});
window.addEventListener("blur", () => {
  player.releaseKeys();
  if (player.state.rewinding) void player.pause().catch(player.error);
});
export function gameInputBlocked(target: EventTarget | null) {
  return (
    !player.state.game ||
    player.state.changing ||
    player.state.rewindTimeline ||
    !player.canvas ||
    !!document.querySelector('dialog[open],[role="dialog"]') ||
    (target instanceof Element &&
      !!target.closest(
        'input,select,textarea,[contenteditable]:not([contenteditable="false"]),[role="textbox"],[role="combobox"],[role="listbox"],[role="option"]',
      ))
  );
}
document.addEventListener("focusin", (event) => {
  if (gameInputBlocked(event.target)) {
    player.releaseKeys();
  }
});
const keyMap: Record<string, number> = {
  ArrowUp: 64,
  ArrowDown: 128,
  ArrowLeft: 32,
  ArrowRight: 16,
  KeyX: 1,
  KeyZ: 2,
  Enter: 8,
  Backspace: 4,
  KeyA: 512,
  KeyS: 256,
};
for (const name of ["keydown", "keyup"])
  window.addEventListener(name, (event) => {
    const e = event as KeyboardEvent;
    if (e.code === "Tab") {
      if (e.type === "keyup") player.boost(false);
      else if (
        !e.shiftKey &&
        !e.ctrlKey &&
        !e.altKey &&
        !e.metaKey &&
        !e.defaultPrevented &&
        !gameInputBlocked(e.target) &&
        player.state.playing &&
        !player.state.rewinding &&
        !player.state.rewindTimeline
      ) {
        e.preventDefault();
        player.boost(true);
      }
      return;
    }
    const key = keyMap[e.code];
    if (!key) return;
    if (e.type === "keyup") {
      player.keyboard &= ~key;
      player.keys();
      return;
    }
    if (
      gameInputBlocked(e.target) ||
      player.state.rewinding ||
      player.state.rewindTimeline ||
      e.defaultPrevented ||
      e.ctrlKey ||
      e.altKey ||
      e.metaKey ||
      (e.code === "Enter" &&
        (e.target as HTMLElement)?.closest?.("button,a,summary"))
    )
      return;
    e.preventDefault();
    player.keyboard |= key;
    player.keys();
  });
function poll() {
  const pad =
    document.hasFocus() &&
    !gameInputBlocked(document.activeElement) &&
    !player.state.rewinding &&
    !player.state.rewindTimeline
      ? [...(navigator.getGamepads?.() ?? [])].find(
          (p) => p?.connected && p.mapping === "standard",
        )
      : null;
  let mask = 0;
  if (pad) {
    [1, 2, 0, 0, 512, 256, 0, 0, 4, 8, 0, 0, 64, 128, 32, 16].forEach(
      (key, i) => {
        if (pad.buttons[i]?.pressed) mask |= key;
      },
    );
    if (pad.axes[0] < -0.5) mask |= 32;
    if (pad.axes[0] > 0.5) mask |= 16;
    if (pad.axes[1] < -0.5) mask |= 64;
    if (pad.axes[1] > 0.5) mask |= 128;
  }
  player.gamepad = mask;
  player.keys();
  requestAnimationFrame(poll);
}
requestAnimationFrame(poll);
