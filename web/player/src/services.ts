import { Context, Data, Effect, Layer, ManagedRuntime } from "effect";
import {
  Emulator,
  type Scenario,
  type FailureBundle,
  type LabReport,
} from "@evenfall/gba";
import { BridgeHost } from "@evenfall/gba/bridge";
import { GameLibraryLive } from "./library";
export class SessionError extends Data.TaggedError("SessionError")<{
  cause: unknown;
}> {
  get message() {
    return this.cause instanceof Error
      ? this.cause.message
      : String(this.cause);
  }
}
export class LabError extends Data.TaggedError("LabError")<{ cause: unknown }> {
  get message() {
    return this.cause instanceof Error
      ? this.cause.message
      : String(this.cause);
  }
}
export const makeEmulator = () =>
  // The bundler emits the SDK's worker and WASM with the player.
  Emulator.create({ diagnosticsIntervalMs: 1000 });
export class Session {
  releaseLock?: () => void;
  readonly bridges: BridgeHost;
  private cleanups: (() => void)[] = [];
  constructor(readonly emulator: Emulator) {
    this.bridges = new BridgeHost(emulator);
  }
  ownSaveLock(release?: () => void) {
    this.releaseLock?.();
    this.releaseLock = release;
  }
  onClose(cleanup: () => void) {
    this.cleanups.push(cleanup);
  }
  close() {
    this.cleanups.forEach((cleanup) => cleanup());
    this.cleanups = [];
    this.ownSaveLock();
    this.bridges.dispose();
    this.emulator.terminate();
  }
}
export class EmulatorSession extends Context.Tag("EmulatorSession")<
  EmulatorSession,
  Session
>() {}
export const EmulatorSessionLive = Layer.scoped(
  EmulatorSession,
  Effect.acquireRelease(
    Effect.tryPromise({
      try: async () => new Session(await makeEmulator()),
      catch: (cause) => new SessionError({ cause }),
    }),
    (session) => Effect.sync(() => session.close()),
  ),
);
export class Lab {
  constructor(
    private readonly create: () => Promise<Emulator> = makeEmulator,
  ) {}
  worker?: Emulator;
  private activeManifest?: Scenario;
  starting = false;
  generation = 0;
  cancelled = false;
  listeners = new Set<() => void>();
  state: {
    status: string;
    checkpoints: Record<string, unknown>[];
    artifacts: { name: string; bytes: Uint8Array }[];
    report?: LabReport;
  } = { status: "Ready", checkpoints: [], artifacts: [] };
  subscribe = (f: () => void) => {
    this.listeners.add(f);
    return () => {
      this.listeners.delete(f);
    };
  };
  snapshot = () => this.state;
  update(p: Partial<Lab["state"]>) {
    this.state = { ...this.state, ...p };
    this.listeners.forEach((f) => f());
  }
  async run(
    manifest: Scenario,
    assets: Record<string, Uint8Array>,
    bundle?: FailureBundle,
  ): Promise<LabReport> {
    if (this.worker || this.starting)
      throw new LabError({ cause: "A test is already running" });
    this.starting = true;
    this.activeManifest = structuredClone(manifest);
    this.cancelled = false;
    const generation = ++this.generation;
    this.update({
      status: "Running",
      checkpoints: [],
      artifacts: [],
      report: undefined,
    });
    let worker: Emulator;
    try {
      worker = await this.create();
    } catch (cause) {
      this.starting = false;
      this.update({ status: "Error" });
      throw new LabError({ cause });
    }
    this.starting = false;
    if (generation !== this.generation) {
      worker.terminate("LabTerminated");
      return this.state.report ?? { status: "terminated", manifest };
    }
    if (this.cancelled) {
      worker.terminate();
      const report = { status: "cancelled" };
      this.update({ status: "cancelled", report });
      return report;
    }
    this.worker = worker;
    worker.addEventListener("progress", (e) =>
      this.update({
        checkpoints: [
          ...this.state.checkpoints,
          (e as CustomEvent).detail.checkpoint,
        ],
      }),
    );
    worker.addEventListener("artifact", (e) => {
      const { name, bytes } = (e as CustomEvent).detail;
      this.update({ artifacts: [...this.state.artifacts, { name, bytes }] });
    });
    try {
      if (bundle)
        await worker.load(assets.rom, {
          ...manifest.config,
          rtcEpoch: BigInt(manifest.config.rtcEpoch),
          bios: assets.bios,
          save: assets.save,
        });
      if (this.cancelled) {
        const report = { status: "cancelled" };
        this.update({ status: "cancelled", report });
        return report;
      }
      const report = await (bundle
        ? worker.replay(bundle)
        : worker.runScenario(manifest, assets));
      this.update({ status: report.status, report });
      return report;
    } catch (cause) {
      if (this.state.status === "Terminated")
        return this.state.report ?? { status: "terminated", manifest };
      this.update({ status: "Error" });
      throw new LabError({ cause });
    } finally {
      worker.terminate();
      if (this.worker === worker) this.worker = undefined;
    }
  }
  cancel() {
    this.cancelled = true;
    return this.worker?.cancel();
  }
  terminate() {
    this.generation++;
    this.update({
      status: "Terminated",
      report: {
        status: "terminated",
        manifest: this.activeManifest,
        checkpoints: this.state.checkpoints,
        failures: [],
        reason: "Worker terminated by user",
      },
    });
    this.worker?.terminate("LabTerminated");
    this.worker = undefined;
  }
}
export class TestLab extends Context.Tag("TestLab")<TestLab, Lab>() {}
export const TestLabLive = Layer.scoped(
  TestLab,
  Effect.acquireRelease(
    Effect.sync(() => new Lab()),
    (lab) => Effect.sync(() => lab.terminate()),
  ),
);
export const runtime = ManagedRuntime.make(
  Layer.mergeAll(GameLibraryLive, EmulatorSessionLive, TestLabLive),
);
