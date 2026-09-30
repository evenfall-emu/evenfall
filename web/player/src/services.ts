import { Context, Data, Effect, Layer, ManagedRuntime } from "effect";
import { Emulator } from "@evenfall/gba";
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
export const runtime = ManagedRuntime.make(
  Layer.mergeAll(GameLibraryLive, EmulatorSessionLive),
);
