import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { player, playerReady } from "./player";

// Optional instance privacy notice supplied by the deployment build.
const privacyUrl = import.meta.env.VITE_PRIVACY_URL;

type Session = { user: { id: string; name: string; email: string } } | null;

async function authRequest<T>(
  path: string,
  body?: unknown,
  signal?: AbortSignal,
): Promise<T> {
  const response = await fetch(`/api/auth/${path}`, {
    method: body === undefined ? "GET" : "POST",
    credentials: "same-origin",
    cache: "no-store",
    headers:
      body === undefined ? undefined : { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal,
  });
  if (!response.ok)
    throw new Error(
      response.status === 429
        ? "Too many attempts. Wait a minute before trying again."
        : "Sign-in is unavailable right now. You can still play locally.",
    );
  return response.json();
}

export function Account() {
  const playerState = useSyncExternalStore(player.subscribe, player.snapshot);
  const requests = useRef<AbortController | null>(null);
  const [session, setSession] = useState<Session>();
  const [refresh, setRefresh] = useState(0);
  const [email, setEmail] = useState("");
  const [busy, setBusy] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [notice, setNotice] = useState("");
  const [error, setError] = useState(() => {
    const code = new URLSearchParams(location.search).get("error");
    if (!code) return "";
    return ["discord_email_not_verified", "email_not_found"].includes(code)
      ? "Discord needs a verified email address. Verify it in Discord or use an email sign-in link."
      : "Sign-in did not complete. The link may have expired or already been used. Please try again.";
  });

  useEffect(() => {
    const url = new URL(location.href);
    for (const key of ["account", "error", "error_description"])
      url.searchParams.delete(key);
    history.replaceState(history.state, "", url);
    const controller = new AbortController();
    requests.current = controller;
    void authRequest<Session>("get-session", undefined, controller.signal)
      .then(async (value) => {
        if (controller.signal.aborted) return;
        setSession(value);
        await playerReady;
        if (!controller.signal.aborted && player.library.db)
          await player.cloud.setIdentity(value?.user.id ?? null);
      })
      .catch(() => {
        if (!controller.signal.aborted)
          setError(
            "Sign-in is unavailable right now. You can still play locally.",
          );
      });
    return () => controller.abort();
  }, [refresh]);

  async function submit(action: (signal: AbortSignal) => Promise<void>) {
    const signal = requests.current!.signal;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await action(signal);
    } catch (cause) {
      if (signal.aborted) return;
      setError(
        cause instanceof Error
          ? cause.message
          : "Sign-in is unavailable right now. You can still play locally.",
      );
    } finally {
      if (!signal.aborted) setBusy(false);
    }
  }

  return (
    <section className="account-panel" aria-label="Sign-in options">
      <p>
        Sign-in is optional. ROMs stay on this device. Cloud backups are on by
        default when signed in; you can turn them off here.
      </p>
      {error && (
        <p className="account-error" role="alert">
          {error}
        </p>
      )}
      {notice && (
        <p className="account-notice" role="status">
          {notice}
        </p>
      )}
      {session === undefined ? (
        error ? (
          <button
            onClick={() => {
              setError("");
              setRefresh(refresh + 1);
            }}
          >
            Retry connection
          </button>
        ) : (
          <p role="status">Checking your session…</p>
        )
      ) : session ? (
        <>
          {!confirmDelete && (
            <>
              <h3>Signed in</h3>
              <p className="account-email">{session.user.email}</p>
              <button
                disabled={busy || !playerState.ready || !player.library.db}
                aria-pressed={playerState.cloud.enabled}
                onClick={() =>
                  void submit(async () => {
                    await player.cloud.enable(!playerState.cloud.enabled);
                  })
                }
              >
                {playerState.cloud.enabled
                  ? "Disable cloud backups"
                  : "Enable cloud backups on this device"}
              </button>
              <p>
                Back up active play every two minutes. Automatic uploads stop
                after five minutes without game input. Offline play keeps saving
                locally.
              </p>
              <p>
                {playerState.cloud.usage
                  ? `${playerState.cloud.usage.playthroughs} / ${playerState.cloud.usage.limit} cloud playthroughs used across all games.`
                  : "Up to 100 cloud playthroughs across all games."}
              </p>
              <p role="status">{playerState.cloud.message}</p>
              {playerState.cloud.uploadedAt && (
                <p>
                  Last confirmed backup:{" "}
                  {new Date(playerState.cloud.uploadedAt).toLocaleString()}
                </p>
              )}
              <button
                disabled={busy}
                onClick={() =>
                  void submit(async (signal) => {
                    await authRequest("sign-out", {}, signal);
                    if (player.library.db) await player.cloud.setIdentity(null);
                    setSession(null);
                    setEmail("");
                  })
                }
              >
                Sign out
              </button>
            </>
          )}
          {confirmDelete ? (
            <section
              className="account-delete"
              aria-label="Confirm account deletion"
            >
              <p>
                Permanently delete your account and all cloud progress? You will
                be signed out and cloud-save cleanup will run automatically.
                Local games and saves stay on this device.
              </p>
              <button
                disabled={busy}
                onClick={() =>
                  void submit(async (signal) => {
                    await player.cloud.deleteAccount(session.user.id);
                    if (signal.aborted) return;
                    setSession(null);
                    setEmail("");
                    setConfirmDelete(false);
                    setNotice(
                      "Account deleted. Cloud-save cleanup continues automatically. Local games and saves were kept.",
                    );
                  })
                }
              >
                {busy ? "Deleting account…" : "Delete account permanently"}
              </button>
              <button disabled={busy} onClick={() => setConfirmDelete(false)}>
                Cancel
              </button>
            </section>
          ) : (
            <button
              className="account-delete-button"
              disabled={busy || !playerState.ready || !player.library.db}
              onClick={() => setConfirmDelete(true)}
            >
              Delete my account
            </button>
          )}
        </>
      ) : (
        <>
          <button
            className="account-discord"
            disabled={busy}
            onClick={() =>
              void submit(async (signal) => {
                const result = await authRequest<{ url: string }>(
                  "sign-in/social",
                  {
                    provider: "discord",
                    callbackURL: "/?account=1",
                    errorCallbackURL: "/?account=1",
                  },
                  signal,
                );
                const url = new URL(result.url);
                if (url.origin !== "https://discord.com")
                  throw new Error("Invalid provider URL");
                location.assign(url.href);
              })
            }
          >
            Continue with Discord
          </button>
          <div className="account-divider">or use your email</div>
          <form
            onSubmit={(event) => {
              event.preventDefault();
              void submit(async (signal) => {
                await authRequest(
                  "sign-in/magic-link",
                  {
                    email: email.trim().toLowerCase(),
                    callbackURL: "/?account=1",
                    errorCallbackURL: "/?account=1",
                  },
                  signal,
                );
                setNotice(
                  "Check your inbox. If delivery is available, a sign-in link will arrive shortly. It expires in 5 minutes.",
                );
              });
            }}
          >
            <label htmlFor="account-email">Email address</label>
            <input
              id="account-email"
              type="email"
              autoComplete="email"
              required
              maxLength={254}
              value={email}
              disabled={busy}
              onChange={(event) => setEmail(event.target.value)}
            />
            <button type="submit" disabled={busy}>
              {busy ? "Please wait…" : "Email me a sign-in link"}
            </button>
          </form>
        </>
      )}
      {privacyUrl && (
        <p className="account-privacy">
          <a href={privacyUrl} target="_blank" rel="noopener noreferrer">
            Privacy notice
          </a>
        </p>
      )}
    </section>
  );
}
