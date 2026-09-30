/**
 * Moves the player to newly deployed versions. The service worker
 * (offline.ts) installs a new version next to the running one; it activates
 * when a page sends `skip-waiting`, and every tab then runs the old version
 * until it reloads. A tab reloads by itself only when that loses nothing the
 * user can see; otherwise it offers a reload, which saves the game first.
 */
import { UPDATE_RELOAD_KEY } from "./storage-names";

export type UpdateStatus = "current" | "ready" | "reloading";

/** How often an open player checks for a new version. */
const CHECK_INTERVAL_MS = 60 * 60 * 1000;
/**
 * Automatic reloads in one tab are at least this far apart, so a server that
 * alternates between two versions cannot reload the page in a loop.
 */
const AUTOMATIC_RELOAD_GAP_MS = 60 * 1000;
const AUTOMATIC_RELOAD_KEY = UPDATE_RELOAD_KEY;

export class AppUpdate {
  status: UpdateStatus = "current";
  private listeners = new Set<() => void>();
  private registration?: ServiceWorkerRegistration;
  /** This tab sent `skip-waiting` and reloads when a new version controls it. */
  private requested = false;
  /** Before any input, the page shows only what its URL describes. */
  private interacted = false;

  constructor(
    private readonly options: {
      /** No game is loaded, nothing is changing and no dialog is open. */
      idle(): boolean;
      /** Commits progress that a reload would lose. */
      beforeReload(): Promise<void>;
    },
  ) {}

  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  snapshot = () => this.status;
  private set(status: UpdateStatus) {
    this.status = status;
    this.listeners.forEach((listener) => listener());
  }

  async start(script: URL) {
    const workers = navigator.serviceWorker;
    for (const type of ["pointerdown", "keydown"])
      addEventListener(type, () => (this.interacted = true), {
        capture: true,
        once: true,
      });
    // A page loaded without a controller (a first visit) already runs the
    // version whose installation then claims it.
    let controlled = Boolean(workers.controller);
    workers.controller?.postMessage({ type: "prune" });
    workers.addEventListener("controllerchange", () => {
      const replaced = controlled;
      controlled = true;
      if (!replaced) return;
      if (this.requested) location.reload();
      // Another tab activated a new version; this one still runs the old.
      else if (this.status !== "reloading") this.offer();
    });
    const registration = await workers.register(script);
    this.registration = registration;
    const waiting = () => {
      if (!registration.waiting || !workers.controller) return;
      // A newer version replaced the one this tab asked for.
      if (this.requested)
        registration.waiting.postMessage({ type: "skip-waiting" });
      else if (this.status === "current") this.offer();
    };
    registration.addEventListener("updatefound", () => {
      const worker = registration.installing;
      worker?.addEventListener("statechange", () => {
        if (worker.state === "installed") waiting();
      });
    });
    waiting();
    // Browsers look for a new version when a page loads. A player left open,
    // or an installed app brought back to the foreground, looks here.
    const check = () => void registration.update().catch(() => {});
    setInterval(check, CHECK_INTERVAL_MS);
    addEventListener("online", check);
    document.addEventListener("visibilitychange", () => {
      if (!document.hidden) check();
      else if (this.status === "ready") this.offer();
    });
  }

  /** Reloads at once when that loses nothing, and otherwise asks. */
  private offer() {
    if (
      this.options.idle() &&
      (document.hidden || !this.interacted) &&
      claimAutomaticReload()
    )
      void this.apply().catch(() => {});
    else this.set("ready");
  }

  /** Saves progress, then reloads this tab into the newest version. */
  async apply() {
    if (this.status === "reloading") return;
    this.set("reloading");
    try {
      await this.options.beforeReload();
    } catch (error) {
      this.set("ready");
      throw error;
    }
    const waiting = this.registration?.waiting;
    if (!waiting) return location.reload();
    this.requested = true;
    waiting.postMessage({ type: "skip-waiting" });
  }
}

function claimAutomaticReload() {
  try {
    const last = Number(sessionStorage.getItem(AUTOMATIC_RELOAD_KEY));
    if (Date.now() - last < AUTOMATIC_RELOAD_GAP_MS) return false;
    sessionStorage.setItem(AUTOMATIC_RELOAD_KEY, String(Date.now()));
    return true;
  } catch {
    return false;
  }
}
