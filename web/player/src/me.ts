import { useEffect, useState } from "react";

/** `GET /api/me`: the signed-in account and what it may manage. */
export type Me = { id: string; admin: boolean };

/**
 * The signed-in account's rights, asked again whenever the account changes.
 * Signed-out players make no request unless `force` is set, as on a page
 * that must know before the stored account loads. `me` is null while signed
 * out, while `checking`, and when the server cannot say. It only shapes the
 * interface: the server authorizes every request.
 */
export function useMe(accountId: string | null, force = false) {
  const key = accountId ?? (force ? "" : null);
  const [answer, setAnswer] = useState<{ key: string; me: Me | null }>();
  useEffect(() => {
    if (key === null) return;
    const request = new AbortController();
    fetch("/api/me", {
      credentials: "same-origin",
      cache: "no-store",
      signal: request.signal,
    })
      .then((response) =>
        response.ok ? (response.json() as Promise<Me>) : null,
      )
      .then(
        (me) => setAnswer({ key, me }),
        () => {
          if (!request.signal.aborted) setAnswer({ key, me: null });
        },
      );
    return () => request.abort();
  }, [key]);
  const current = key !== null && answer?.key === key;
  return {
    me: current ? answer.me : null,
    checking: key !== null && !current,
  };
}
