import {
  cloudEligible,
  QUICK_SLOTS,
  quickKind,
  quickSlotOf,
  type CloudIdentity,
  type CloudJob,
  type Library,
  type PlaythroughRef,
  type QuickRemote,
  type QuickSlot,
  type SaveKind,
} from "./library";

export type CloudResume = {
  accountId: string;
  hash: string;
  playthroughId: string;
  playthroughName: string;
  title: string;
  capturedAt: number;
  uploadedAt: number;
  /** The version inspected when the user made a load decision. */
  etag?: string;
};
/** A slot changed on both sides since their last common revision. */
export type QuickConflict = {
  accountId: string;
  hash: string;
  playthroughId: string;
  slot: number;
  localSavedAt: number;
  cloudSavedAt: number;
  etag: string;
  prompted: boolean;
};
export type CloudStatus = CloudIdentity & {
  message: string;
  uploadedAt?: number;
  latest?: CloudResume;
  resumes?: CloudResume[];
  usage?: { playthroughs: number; limit: number };
  discovering?: boolean;
  discoveryError?: string;
  quickConflicts?: QuickConflict[];
};
type CloudSession = { user: { id: string } } | null;
type RemoteSave = { accountId: string; data: Uint8Array; etag: string };
const STATE_MAGIC = new TextEncoder().encode("ZIGBAST1");
const MAX_STATE = 2 * 1024 * 1024;

/** A downloaded, verified shared state and what it was shared as. */
export type SharedState = {
  hash: string;
  title: string;
  playthroughName: string;
  data: Uint8Array;
};

/**
 * Downloads a shared state by its link ID. Needs no account: the link is the
 * capability. The body is checked against its digest and the state magic.
 */
export async function fetchSharedState(id: string): Promise<SharedState> {
  let response: Response;
  try {
    response = await fetch(`/api/shares/${encodeURIComponent(id)}`, {
      credentials: "omit",
      cache: "no-store",
      signal: AbortSignal.timeout(30_000),
    });
  } catch {
    throw new Error("Could not reach the server. Check your connection.");
  }
  if (response.status === 404)
    throw new Error("This share link has expired or does not exist.");
  if (!response.ok)
    throw new Error("The shared state is unavailable. Try again later.");
  const bytes = await readBounded(response.body, MAX_STATE);
  if ((await sha256(bytes)) !== response.headers.get("X-Zigba-Digest"))
    throw new Error("The shared state failed its integrity check.");
  const data = await readBounded(
    new Blob([bytes]).stream().pipeThrough(new DecompressionStream("gzip")),
    MAX_STATE,
  );
  const hash = response.headers.get("X-Zigba-Rom") ?? "";
  if (
    !STATE_MAGIC.every((byte, index) => data[index] === byte) ||
    !/^[0-9a-f]{40}$/.test(hash)
  )
    throw new Error("This link does not hold a ZiGBA state.");
  const label = (header: string, fallback: string) => {
    try {
      return (
        privateLabel(decodeURIComponent(response.headers.get(header) ?? "")) ||
        fallback
      );
    } catch {
      return fallback;
    }
  };
  return {
    hash,
    title: label("X-Zigba-Title", `Game ${hash.slice(0, 8)}`),
    playthroughName: label("X-Zigba-Playthrough-Name", "Shared playthrough"),
    data,
  };
}

export type QuickPlan =
  | { type: "none" }
  | { type: "download" }
  | { type: "adopt" }
  | { type: "claim"; baseEtag?: string }
  | { type: "conflict" };

/**
 * Decides one quick slot against the account's last common revision: the
 * outbox job's acknowledged `uploadedRevision` and cloud `baseEtag`. One-sided
 * changes resolve silently; only a change on both sides is a conflict. A slot
 * last written for another account is never uploaded without a user choice.
 */
export function planQuickSlot(
  accountId: string,
  local: QuickSlot | undefined,
  job: CloudJob | undefined,
  remote: QuickRemote | undefined,
): QuickPlan {
  // Same content on both sides, e.g. an upload whose acknowledgement was lost.
  if (local?.cloudRevision && local.cloudRevision === remote?.revision)
    return job?.baseEtag === remote.etag &&
      job.uploadedRevision === remote.revision &&
      !job.conflict
      ? { type: "none" }
      : { type: "adopt" };
  const tracked =
    local?.accountId === accountId &&
    Boolean(local.cloudRevision) &&
    job?.revision === local.cloudRevision;
  const foreign = Boolean(local?.accountId) && local?.accountId !== accountId;
  const localChanged =
    Boolean(local) && (!tracked || job!.revision !== job!.uploadedRevision);
  const remoteChanged = remote
    ? remote.etag !== job?.baseEtag
    : Boolean(job?.baseEtag);
  if (!remote) {
    if (!local || foreign || (tracked && !remoteChanged))
      return { type: "none" };
    return { type: "claim" };
  }
  if (!local || !localChanged) return { type: "download" };
  // The cloud still holds the common base. A stale conflict flag (for
  // example from an interrupted pass) is re-queued rather than left stuck.
  if (!remoteChanged)
    return foreign || (tracked && !job!.conflict)
      ? { type: "none" }
      : { type: "claim", baseEtag: remote.etag };
  return { type: "conflict" };
}

// The page drains durable metadata, never an in-memory snapshot queue. A
// service worker caches the player, but cannot decide whether a user is active.
export class CloudSync {
  private running = false;
  private timer?: ReturnType<typeof setInterval>;
  private identity: CloudIdentity = { accountId: null, enabled: false };
  private identityEpoch = 0;
  private uploadedAt?: number;
  private resumes: CloudResume[] = [];
  private usage?: CloudStatus["usage"];
  private discoveryEpoch?: number;
  private discoveryError?: string;
  private quickConflicts: QuickConflict[] = [];
  private message = "Cloud backups are off.";
  /** The player's loaded run; its quick slots reconcile on reconnect. */
  activeRun?: () => PlaythroughRef | undefined;
  quickChanged?: (ref: PlaythroughRef) => void;
  private wake = () => {
    void this.flush();
  };
  private reconnect = () => {
    void this.discover();
    this.wake();
    this.syncActiveQuick();
  };
  constructor(
    private library: Library,
    private changed: (state: CloudStatus) => void,
  ) {}

  private report(message = this.message) {
    this.message = message;
    const resumes = this.resumes.filter(
      (run) => run.accountId === this.identity.accountId,
    );
    this.changed({
      ...this.identity,
      message,
      uploadedAt: this.uploadedAt,
      latest: resumes[0],
      resumes,
      usage: this.usage,
      discovering: this.discoveryEpoch === this.identityEpoch,
      discoveryError: this.discoveryError,
      quickConflicts: this.quickConflicts.filter(
        (conflict) => conflict.accountId === this.identity.accountId,
      ),
    });
  }
  async start() {
    if (!this.library.db) return;
    await this.setIdentity((await this.library.cloudIdentity()).accountId);
    window.addEventListener("online", this.reconnect);
    this.timer = setInterval(this.wake, 15_000);
    // Discover a login after a redirect even if Account is never opened.
    try {
      await this.refreshIdentity();
    } catch {
      /* Offline play keeps the last known account. */
    }
    this.wake();
  }
  close() {
    clearInterval(this.timer);
    window.removeEventListener("online", this.reconnect);
  }
  async setIdentity(accountId: string | null) {
    const epoch = ++this.identityEpoch;
    if (accountId !== this.identity.accountId) {
      this.resumes = [];
      this.usage = undefined;
      this.quickConflicts = [];
    }
    this.discoveryError = undefined;
    const identity = await this.library.setCloudIdentity(accountId);
    if (epoch !== this.identityEpoch) return;
    this.identity = identity;
    const jobs = accountId ? await this.library.cloudJobs(accountId) : [];
    if (epoch !== this.identityEpoch) return;
    this.uploadedAt =
      Math.max(
        0,
        ...jobs.map((job) => job.confirmedAt ?? job.uploadedAt ?? 0),
      ) || undefined;
    this.report(
      this.identity.enabled
        ? "Cloud backups enabled."
        : "Cloud backups are off.",
    );
    if (accountId) void this.discover();
    await this.loadQuickConflicts();
    this.syncActiveQuick();
  }
  async discover() {
    const accountId = this.identity.accountId;
    const epoch = this.identityEpoch;
    if (!accountId || this.discoveryEpoch === epoch) return;
    this.discoveryEpoch = epoch;
    this.discoveryError = undefined;
    this.report();
    try {
      let cursor: string | null = null;
      const resumes: CloudResume[] = [];
      let usage: CloudStatus["usage"];
      do {
        const response = await fetch(
          `/api/saves${cursor ? `?cursor=${encodeURIComponent(cursor)}` : ""}`,
          {
            credentials: "same-origin",
            cache: "no-store",
            headers: { "X-Zigba-Account": accountId },
            signal: AbortSignal.timeout(15_000),
          },
        );
        if (!response.ok) {
          if (response.status === 401 || response.status === 403)
            await this.refreshIdentity();
          throw new Error(
            "Cloud progress is unavailable. Local play still works.",
          );
        }
        const page = (await response.json()) as {
          saves: (Omit<CloudResume, "accountId" | "uploadedAt"> & {
            uploadedAt: string;
          })[];
          cursor: string | null;
          usage?: CloudStatus["usage"];
        };
        const identity = await this.library.cloudIdentity();
        if (epoch !== this.identityEpoch || identity.accountId !== accountId)
          return;
        for (const save of page.saves) {
          const uploadedAt = Date.parse(save.uploadedAt);
          resumes.push({ ...save, accountId, uploadedAt });
        }
        usage = page.usage ?? usage;
        cursor = page.cursor;
      } while (cursor);
      // R2 lists by key, not by date. Do not publish a partial-page winner.
      this.resumes = resumes.sort((a, b) => b.uploadedAt - a.uploadedAt);
      this.usage = usage;
    } catch {
      if (epoch === this.identityEpoch)
        this.discoveryError =
          "Cloud progress is unavailable. Local play still works.";
    } finally {
      if (epoch === this.identityEpoch) {
        this.discoveryEpoch = undefined;
        this.report();
      }
    }
  }
  async enable(enabled: boolean) {
    await this.refreshIdentity();
    if (!this.identity.accountId)
      throw new Error("Sign in to enable cloud backups.");
    this.identity = await this.library.setCloudIdentity(
      this.identity.accountId,
      enabled,
    );
    this.report(
      enabled
        ? "Cloud backups enabled. New progress will sync."
        : "Cloud backups are off. Existing cloud copies are retained.",
    );
    if (enabled) this.syncActiveQuick();
  }
  /** Check the selected run's current remote version, not the account's latest run. */
  async inspectResume(ref: PlaythroughRef, title: string, name: string) {
    const accountId = this.identity.accountId;
    if (!accountId || !navigator.onLine) return;
    await this.refreshIdentity();
    if (this.identity.accountId !== accountId) return;
    const response = await this.request(
      { accountId, ...ref, kind: "resume" },
      "HEAD",
    );
    if (response.status === 404) return;
    if (!response.ok) throw new Error("Cloud progress is unavailable.");
    const etag = response.headers.get("ETag");
    const uploadedAt = Date.parse(
      response.headers.get("X-Zigba-Uploaded-At") ?? "",
    );
    if (!etag || !Number.isFinite(uploadedAt))
      throw new Error("Cloud checkpoint metadata is unavailable.");
    return {
      etag,
      backup: {
        accountId,
        ...ref,
        playthroughName: name,
        title,
        capturedAt: Number(response.headers.get("X-Zigba-Captured-At") ?? 0),
        uploadedAt,
        etag,
      } satisfies CloudResume,
    };
  }
  /**
   * Uploads one state and returns its share link. Needs a signed-in account
   * but not cloud backups: sharing is an explicit, one-off upload.
   */
  async share(state: {
    hash: string;
    title: string;
    playthroughName: string;
    data: Uint8Array;
  }): Promise<string> {
    await this.refreshIdentity();
    const accountId = this.identity.accountId;
    if (!accountId) throw new Error("Sign in to share a state.");
    const bytes = await gzip(state.data);
    const title = privateLabel(state.title);
    let response: Response;
    try {
      response = await fetch("/api/shares", {
        method: "POST",
        credentials: "same-origin",
        cache: "no-store",
        headers: {
          "Content-Type": "application/octet-stream",
          "X-Zigba-Account": accountId,
          "X-Zigba-Rom": state.hash,
          "X-Zigba-Digest": await sha256(bytes),
          "X-Zigba-Playthrough-Name": encodeURIComponent(
            privateLabel(state.playthroughName) || "Playthrough",
          ),
          ...(title ? { "X-Zigba-Title": encodeURIComponent(title) } : {}),
        },
        body: bytes,
        signal: AbortSignal.timeout(30_000),
      });
    } catch {
      throw new Error("Could not reach the server. Check your connection.");
    }
    if (!response.ok) {
      if (response.status === 401 || response.status === 403)
        await this.refreshIdentity().catch(() => {});
      throw new Error(
        response.status === 429
          ? "Upload limit reached. Try sharing again later."
          : response.status === 401 || response.status === 403
            ? "Sign in again to share this state."
            : "The state could not be shared. Try again.",
      );
    }
    const { url } = (await response.json()) as { url?: string };
    if (!url) throw new Error("The server returned an invalid share link.");
    return url;
  }
  private async refreshIdentity() {
    const epoch = this.identityEpoch;
    const response = await fetch("/api/auth/get-session", {
      credentials: "same-origin",
      cache: "no-store",
      signal: AbortSignal.timeout(15_000),
    });
    if (!response.ok)
      throw new Error("Sign-in is unavailable. Progress stays local.");
    const session = (await response.json()) as CloudSession;
    if (epoch !== this.identityEpoch) return;
    const accountId = session?.user.id ?? null;
    if (accountId !== this.identity.accountId)
      await this.setIdentity(accountId);
    else this.identity = await this.library.cloudIdentity();
  }
  async exclusive<T>(operation: () => Promise<T>): Promise<T> {
    if (!navigator.locks)
      throw new Error(
        "Cloud sync needs browser storage locks. Local play is still available.",
      );
    return navigator.locks.request("zigba-cloud-sync", operation);
  }
  async deleteAccount(expectedAccountId: string) {
    await this.exclusive(async () => {
      await this.refreshIdentity();
      if (this.identity.accountId !== expectedAccountId)
        throw new Error("Account changed. Reopen Account before deleting it.");
      // Do not enqueue more progress while the server purges the account.
      // On failure, leave uploads off so the user can safely retry deletion.
      this.identity = await this.library.setCloudIdentity(
        expectedAccountId,
        false,
      );
      this.report("Deleting account and cloud progress…");
      try {
        const response = await fetch("/api/account", {
          method: "DELETE",
          credentials: "same-origin",
          cache: "no-store",
          headers: { "X-Zigba-Account": expectedAccountId },
          signal: AbortSignal.timeout(60_000),
        });
        const result = (await response.json()) as {
          accountDeleted?: boolean;
          cloudDeletion?: string;
        };
        if (
          response.status !== 202 ||
          result.accountDeleted !== true ||
          result.cloudDeletion !== "pending"
        )
          throw new Error("Deletion not confirmed");
      } catch {
        this.report(
          "Cloud backups are off. Account deletion needs another attempt.",
        );
        throw new Error(
          "Account deletion was not confirmed. Please retry. Local games and saves are unchanged.",
        );
      }
      await this.library.forgetCloudAccount(expectedAccountId);
      await this.setIdentity((await this.library.cloudIdentity()).accountId);
      this.report(
        "Account deleted. Cloud-save cleanup continues automatically. Local games and saves were kept.",
      );
    });
  }
  async flush(manual?: PlaythroughRef, replace = false) {
    if (this.running) {
      if (manual)
        throw new Error(
          "A cloud backup is already running. Try again after it finishes.",
        );
      return;
    }
    this.running = true;
    try {
      await this.exclusive(async () => {
        this.identity = await this.library.cloudIdentity();
        if (!this.identity.enabled || !this.identity.accountId) {
          if (manual) throw new Error("Enable cloud backups in Account first.");
          return;
        }
        let jobs = await this.library.cloudJobs(this.identity.accountId);
        if (!manual && !jobs.some((job) => cloudEligible(job, Date.now())))
          return;
        if (!navigator.onLine)
          throw new Error(
            "Offline. The latest local progress will sync when eligible and connected.",
          );
        const requestedAccount = this.identity.accountId;
        await this.refreshIdentity();
        if (!this.identity.accountId || !this.identity.enabled)
          throw new Error("Sign in to the account that owns these backups.");
        if (this.identity.accountId !== requestedAccount)
          throw new Error(
            "Account changed. No backup was transferred to the new account.",
          );
        jobs = await this.library.cloudJobs(this.identity.accountId);
        let uploaded = false;
        let resumeConfirmed = false;
        let quotaMessage: string | undefined;
        let quickDiverged = false;
        for (let job of jobs) {
          const quickSlot = quickSlotOf(job.kind);
          if (
            manual
              ? job.hash !== manual.hash ||
                job.playthroughId !== manual.playthroughId
              : !cloudEligible(job, Date.now())
          )
            continue;
          // Diverged quick slots wait for the user's per-slot decision; the
          // explicit replace action covers resume and in-game saves only.
          if (quickSlot !== undefined && (job.conflict || replace)) continue;
          if (job.conflict && !replace)
            throw new Error(
              "Cloud progress differs. Restore it, or explicitly replace the cloud copy.",
            );
          try {
            // A lost PUT response can be recovered without keeping its old
            // payload. Local persistence may already have replaced it.
            if (job.flight || replace) {
              const head = await this.request(job, "HEAD");
              if (head.status !== 404 && !head.ok)
                throw new Error("Could not check the cloud backup.");
              if (
                job.flight &&
                head.ok &&
                head.headers.get("X-Zigba-Revision") === job.flight.revision &&
                head.headers.get("X-Zigba-Digest") === job.flight.digest
              ) {
                this.uploadedAt = Date.parse(
                  head.headers.get("X-Zigba-Uploaded-At")!,
                );
                if (!Number.isFinite(this.uploadedAt))
                  throw new Error("Invalid cloud acknowledgement time.");
                await this.library.acknowledgeCloud(
                  job,
                  job.flight.revision,
                  head.headers.get("ETag")!,
                  Date.now(),
                  this.uploadedAt,
                );
                uploaded = true;
              }
              if (replace)
                await this.library.adoptCloudBase(
                  job.accountId,
                  job,
                  job.kind,
                  head.headers.get("ETag") ?? undefined,
                );
              job = (await this.library.cloudJobs(job.accountId)).find(
                (item) =>
                  item.hash === job.hash &&
                  item.playthroughId === job.playthroughId &&
                  item.kind === job.kind,
              )!;
              if (!job || (!manual && !cloudEligible(job, Date.now())))
                continue;
            }
            const data = await this.library.cloudSource(job);
            if (!data) continue; // Deleted, replaced, or now belongs to another account.
            if (job.revision === job.uploadedRevision) {
              uploaded = true;
              resumeConfirmed ||= job.kind === "resume";
              continue;
            }
            const bytes = await gzip(data);
            const digest = await sha256(bytes);
            const game = (await this.library.list()).find(
              (game) => game.hash === job.hash,
            );
            const title = privateLabel(game?.title ?? "");
            const run = await this.library.getPlaythrough(job);
            const playthroughName = privateLabel(run?.name ?? "Playthrough");
            if (!(await this.library.beginCloud(job, digest, Boolean(manual))))
              continue;
            this.report("Uploading cloud backup…");
            const response = await this.request(job, "PUT", {
              headers: {
                "Content-Type": "application/octet-stream",
                "X-Zigba-Revision": job.revision,
                "X-Zigba-Captured-At": String(job.capturedAt),
                "X-Zigba-Digest": digest,
                "X-Zigba-Playthrough-Name": encodeURIComponent(playthroughName),
                ...(title
                  ? { "X-Zigba-Title": encodeURIComponent(title) }
                  : {}),
                ...(job.baseEtag
                  ? { "If-Match": job.baseEtag }
                  : { "If-None-Match": "*" }),
              },
              body: bytes,
            });
            if (!response.ok) {
              const problem = (await response.json().catch(() => null)) as {
                error?: { code?: string };
              } | null;
              if (problem?.error?.code === "playthrough_limit") {
                quotaMessage =
                  "Your account has reached 100 cloud playthroughs. This run stays local; existing cloud playthroughs can still sync.";
                if (manual) throw new Error(quotaMessage);
                await this.library.cloudFailure(job);
                continue;
              }
              if (problem?.error?.code === "quick_slot_limit") {
                quotaMessage = `Quick slot ${quickSlot} is beyond this account's cloud limit. It stays on this device.`;
                await this.library.cloudFailure(job);
                continue;
              }
              if (
                quickSlot !== undefined &&
                (response.status === 409 || response.status === 412)
              ) {
                // Another device replaced this slot. Record both sides and
                // let the user choose; neither copy is overwritten here.
                await this.library.cloudFailure(job, true);
                await this.reconcileQuick(job.accountId, job).catch(() => {});
                quickDiverged = true;
                continue;
              }
              if (response.status === 409 || response.status === 412) {
                await this.library.cloudFailure(job, true);
                throw new Error(
                  "Cloud progress differs. Neither copy was overwritten; restore or explicitly replace the cloud copy.",
                );
              }
              throw new Error(
                response.status === 429
                  ? "Cloud upload limit reached. Backups will retry later."
                  : "Cloud upload failed. Local progress is safe; a retry is queued.",
              );
            }
            const result = (await response.json()) as {
              etag: string;
              revision: string;
              uploadedAt: string;
            };
            if (result.revision !== job.revision || !result.etag)
              throw new Error("Invalid cloud acknowledgement.");
            this.uploadedAt = Date.parse(result.uploadedAt);
            if (!Number.isFinite(this.uploadedAt))
              throw new Error("Invalid cloud acknowledgement time.");
            await this.library.acknowledgeCloud(
              job,
              result.revision,
              result.etag,
              Date.now(),
              this.uploadedAt,
            );
            uploaded = true;
            resumeConfirmed ||= job.kind === "resume";
            if (quickSlot !== undefined) this.quickChanged?.(job);
          } catch (error) {
            const current = (await this.library.cloudJobs(job.accountId)).find(
              (item) =>
                item.hash === job.hash &&
                item.playthroughId === job.playthroughId &&
                item.kind === job.kind,
            );
            if (!current?.conflict) await this.library.cloudFailure(job);
            throw error;
          }
        }
        if (quickDiverged) await this.loadQuickConflicts();
        this.report(
          quotaMessage ??
            (quickDiverged
              ? "A quick slot changed on another device. Choose which copy to keep."
              : uploaded
                ? "Cloud backup confirmed."
                : "No eligible new progress to upload."),
        );
        if (manual && !resumeConfirmed)
          throw new Error(
            "The latest local resume was not confirmed. Keep this device until backup succeeds.",
          );
      });
    } catch (error) {
      // Auth outages also back off durably; reconnect/reload must not reset
      // retries to a request every timer tick. Offline itself consumes no retry.
      if (navigator.onLine && this.identity.accountId) {
        try {
          for (const job of await this.library.cloudJobs(
            this.identity.accountId,
          )) {
            if (
              cloudEligible(job, Date.now()) &&
              (!manual ||
                (job.hash === manual.hash &&
                  job.playthroughId === manual.playthroughId))
            )
              await this.library.cloudFailure(job);
          }
        } catch {
          /* Preserve the original error if local storage also fails. */
        }
      }
      this.report(
        error instanceof Error
          ? error.message
          : "Cloud sync unavailable. Local progress is safe.",
      );
      if (manual) throw error;
    } finally {
      this.running = false;
    }
  }
  private request(
    job: Pick<CloudJob, "accountId" | "hash" | "playthroughId" | "kind">,
    method: string,
    init: RequestInit = {},
  ) {
    return fetch(`/api/saves/${job.hash}/${job.playthroughId}/${job.kind}`, {
      ...init,
      method,
      credentials: "same-origin",
      cache: "no-store",
      headers: { ...init.headers, "X-Zigba-Account": job.accountId },
      signal: AbortSignal.timeout(30_000),
    });
  }
  private syncActiveQuick() {
    const ref = this.activeRun?.();
    if (ref) void this.syncQuick(ref);
  }
  /**
   * Silently reconciles one run's quick slots, then uploads local-only
   * changes. Runs on open, sign-in, enabling backups and reconnecting; local
   * slots stay usable when the network or account is unavailable.
   */
  async syncQuick(ref: PlaythroughRef) {
    if (!this.library.db) return;
    const identity = await this.library.cloudIdentity();
    if (!identity.enabled || !identity.accountId || !navigator.onLine) return;
    try {
      await this.exclusive(async () => {
        await this.refreshIdentity();
        if (
          !this.identity.enabled ||
          this.identity.accountId !== identity.accountId
        )
          return;
        await this.reconcileQuick(identity.accountId!, ref);
      });
    } catch {
      this.report("Quick slots could not sync. They remain on this device.");
    }
    await this.loadQuickConflicts();
    this.quickChanged?.(ref);
    await this.flush();
  }
  // Caller holds exclusive().
  private async reconcileQuick(accountId: string, ref: PlaythroughRef) {
    const response = await fetch(
      `/api/saves/${ref.hash}/${ref.playthroughId}/quick`,
      {
        credentials: "same-origin",
        cache: "no-store",
        headers: { "X-Zigba-Account": accountId },
        signal: AbortSignal.timeout(15_000),
      },
    );
    if (!response.ok) throw new Error("Quick-slot listing unavailable.");
    const { slots } = (await response.json()) as { slots: QuickRemote[] };
    const jobs = await this.library.cloudJobs(accountId);
    let failure: unknown;
    for (let slot = 1; slot <= QUICK_SLOTS; slot++) {
      try {
        await this.reconcileQuickSlot(accountId, ref, slot, slots, jobs);
      } catch (error) {
        // One unreadable slot must not hold back the others.
        failure ??= error;
      }
    }
    if (failure) throw failure;
  }
  private async reconcileQuickSlot(
    accountId: string,
    ref: PlaythroughRef,
    slot: number,
    slots: QuickRemote[],
    jobs: CloudJob[],
  ) {
    const remote = slots.find((item) => item.slot === slot);
    const local = await this.library.getQuickSlot(ref, slot);
    const job = jobs.find(
      (item) =>
        item.hash === ref.hash &&
        item.playthroughId === ref.playthroughId &&
        item.kind === quickKind(slot),
    );
    const plan = planQuickSlot(accountId, local, job, remote);
    const seen = {
      cloudRevision: local?.cloudRevision,
      savedAt: local?.savedAt,
    };
    if (plan.type === "none") return;
    if (plan.type === "claim")
      return this.library.settleQuickSlot(accountId, ref, slot, seen, plan);
    if (plan.type !== "download")
      return this.library.settleQuickSlot(accountId, ref, slot, seen, {
        type: plan.type,
        remote: remote!,
      });
    const data = await this.fetchQuick(accountId, ref, remote!);
    // Undefined means a newer upload landed after listing; the next pass sees it.
    if (data)
      await this.library.settleQuickSlot(accountId, ref, slot, seen, {
        type: "download",
        data,
        remote: remote!,
      });
  }
  private async fetchQuick(
    accountId: string,
    ref: PlaythroughRef,
    remote: QuickRemote,
  ): Promise<Uint8Array | undefined> {
    const response = await this.request(
      { accountId, ...ref, kind: quickKind(remote.slot) },
      "GET",
    );
    if (response.status === 404) return undefined;
    if (!response.ok) throw new Error("Quick-slot download unavailable.");
    if (response.headers.get("ETag") !== remote.etag) {
      await response.body?.cancel();
      return undefined;
    }
    const bytes = await readBounded(response.body, 2 * 1024 * 1024);
    if ((await sha256(bytes)) !== response.headers.get("X-Zigba-Digest"))
      throw new Error("Cloud quick slot integrity check failed.");
    const data = await readBounded(
      new Blob([bytes]).stream().pipeThrough(new DecompressionStream("gzip")),
      2 * 1024 * 1024,
    );
    if (!STATE_MAGIC.every((byte, index) => data[index] === byte))
      throw new Error("Cloud quick slot is not a ZiGBA state.");
    return data;
  }
  private async loadQuickConflicts() {
    const accountId = this.identity.accountId;
    const conflicts: QuickConflict[] = [];
    for (const job of accountId
      ? await this.library.cloudJobs(accountId)
      : []) {
      const slot = quickSlotOf(job.kind);
      if (slot === undefined || !job.conflict || !job.remote) continue;
      const local = await this.library.getQuickSlot(job, slot);
      if (!local) continue;
      conflicts.push({
        accountId: job.accountId,
        hash: job.hash,
        playthroughId: job.playthroughId,
        slot,
        localSavedAt: local.savedAt,
        cloudSavedAt: job.remote.capturedAt,
        etag: job.remote.etag,
        prompted: job.prompted === job.remote.etag,
      });
    }
    if (accountId !== this.identity.accountId) return;
    this.quickConflicts = conflicts.sort((a, b) => a.slot - b.slot);
    this.report();
  }
  /** Divergences are asked about once; later ones only show in the menu. */
  async markQuickPrompted(conflicts: QuickConflict[]) {
    for (const conflict of conflicts)
      await this.library.markQuickPrompted(
        conflict.accountId,
        conflict,
        new Map([[conflict.slot, conflict.etag]]),
      );
    await this.loadQuickConflicts();
  }
  /** Applies the user's explicit per-slot choice for diverged copies. */
  async resolveQuick(
    ref: PlaythroughRef,
    choices: { slot: number; keep: "local" | "cloud" }[],
  ) {
    await this.exclusive(async () => {
      await this.refreshIdentity();
      const accountId = this.identity.accountId;
      if (!accountId || !this.identity.enabled)
        throw new Error("Enable cloud backups in Account first.");
      const jobs = await this.library.cloudJobs(accountId);
      let changedAgain = false;
      for (const { slot, keep } of choices) {
        const job = jobs.find(
          (item) =>
            item.hash === ref.hash &&
            item.playthroughId === ref.playthroughId &&
            item.kind === quickKind(slot),
        );
        if (!job?.conflict || !job.remote) continue;
        const local = await this.library.getQuickSlot(ref, slot);
        const seen = {
          cloudRevision: local?.cloudRevision,
          savedAt: local?.savedAt,
        };
        if (keep === "local") {
          // Uploads with If-Match on the cloud copy the user chose to replace.
          await this.library.settleQuickSlot(accountId, ref, slot, seen, {
            type: "claim",
            baseEtag: job.remote.etag,
          });
          continue;
        }
        const data = await this.fetchQuick(accountId, ref, job.remote);
        if (!data) {
          changedAgain = true;
          continue;
        }
        await this.library.settleQuickSlot(accountId, ref, slot, seen, {
          type: "download",
          data,
          remote: job.remote,
        });
      }
      // The cloud moved again after the user looked; show the new copy.
      if (changedAgain) await this.reconcileQuick(accountId, ref);
    });
    await this.loadQuickConflicts();
    this.quickChanged?.(ref);
    await this.flush();
  }
  // Caller holds exclusive() through successful emulator restore and local
  // persistence; merely downloading must not advance the outbox's remote base.
  async download(
    ref: PlaythroughRef,
    kind: SaveKind,
    expectedAccountId?: string,
    expectedEtag?: string,
  ): Promise<RemoteSave> {
    await this.refreshIdentity();
    const accountId = this.identity.accountId;
    if (!accountId) throw new Error("Sign in to restore cloud progress.");
    if (expectedAccountId && accountId !== expectedAccountId)
      throw new Error(
        "Account changed. Choose a backup from the current account.",
      );
    const response = await this.request({ accountId, ...ref, kind }, "GET");
    if (!response.ok)
      throw new Error(
        response.status === 404
          ? "No cloud backup exists for this playthrough."
          : "Cloud restore unavailable. Local progress is unchanged.",
      );
    if (expectedEtag && response.headers.get("ETag") !== expectedEtag) {
      await response.body?.cancel();
      throw new Error(
        "Cloud checkpoint changed since you chose it. Reopen this playthrough to compare the latest copies.",
      );
    }
    const bytes = await readBounded(response.body, 2 * 1024 * 1024);
    if ((await sha256(bytes)) !== response.headers.get("X-Zigba-Digest"))
      throw new Error("Cloud backup integrity check failed.");
    const etag = response.headers.get("ETag");
    if (!etag) throw new Error("Cloud backup metadata is missing.");
    const data = await readBounded(
      new Blob([bytes]).stream().pipeThrough(new DecompressionStream("gzip")),
      kind === "resume" ? 2 * 1024 * 1024 : 128 * 1024,
    );
    if ((await this.library.cloudIdentity()).accountId !== accountId)
      throw new Error("Account changed during cloud restore.");
    return { accountId, data, etag };
  }
}

function privateLabel(value: string) {
  return [...value.replace(/[\u0000-\u001f\u007f]/g, " ").trim()]
    .slice(0, 160)
    .join("");
}

async function gzip(bytes: Uint8Array) {
  return new Uint8Array(
    await new Response(
      new Blob([new Uint8Array(bytes)])
        .stream()
        .pipeThrough(new CompressionStream("gzip")),
    ).arrayBuffer(),
  );
}
async function sha256(bytes: Uint8Array<ArrayBuffer>) {
  return Array.from(
    new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)),
    (byte) => byte.toString(16).padStart(2, "0"),
  ).join("");
}
async function readBounded(
  stream: ReadableStream<Uint8Array> | null,
  limit: number,
) {
  if (!stream) throw new Error("Missing cloud backup body.");
  const reader = stream.getReader();
  const chunks: Uint8Array[] = [];
  let length = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      length += value.length;
      if (length > limit)
        throw new Error("Cloud backup exceeds the supported size.");
      chunks.push(value);
    }
  } finally {
    await reader.cancel().catch(() => {});
    reader.releaseLock();
  }
  const result = new Uint8Array(length);
  let offset = 0;
  for (const chunk of chunks) {
    result.set(chunk, offset);
    offset += chunk.length;
  }
  return result;
}
