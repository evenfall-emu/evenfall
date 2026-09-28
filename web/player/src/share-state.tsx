import { useRef, useState } from "react";
import type { SharedState } from "./cloud";

export type ShareProgress =
  | { status: "signed-out" }
  | { status: "creating" }
  | { status: "ready"; url: string }
  | { status: "failed"; message: string };

const comparable = (name: string) => name.trim().toLocaleLowerCase();
/** Names match ignoring surrounding space and case, as a reader would. */
export function nameTaken(names: string[], name: string) {
  return names.some((taken) => comparable(taken) === comparable(name));
}

/** The sharer's side: creating a link to the current state, then copying it. */
export function ShareLink({
  progress,
  title,
  playthroughName,
  retry,
  signIn,
}: {
  progress: ShareProgress;
  title: string;
  playthroughName: string;
  retry: () => void;
  signIn: () => void;
}) {
  const [copied, setCopied] = useState<"" | "copied" | "failed">("");
  const link = useRef<HTMLInputElement>(null);
  if (progress.status === "signed-out")
    return (
      <>
        <p>
          Sign in to create share links. The people you send a link to don’t
          need an account.
        </p>
        <div className="resume-recovery-actions">
          <button onClick={signIn}>Sign in</button>
        </div>
      </>
    );
  return (
    <>
      <p>
        <strong>
          {title} · {playthroughName}
        </strong>
      </p>
      {progress.status === "creating" && (
        <p role="status">Creating a link to this moment…</p>
      )}
      {progress.status === "failed" && (
        <>
          <p role="alert">{progress.message}</p>
          <div className="resume-recovery-actions">
            <button onClick={retry}>Try again</button>
          </div>
        </>
      )}
      {progress.status === "ready" && (
        <>
          <p>
            Anyone with this link can start this exact moment as a new
            playthrough with their own copy of the game. It works for at least
            24 hours.
          </p>
          <div className="share-link">
            <input
              ref={link}
              readOnly
              value={progress.url}
              aria-label="Share link"
              onFocus={(event) => event.currentTarget.select()}
            />
            <button
              onClick={() => {
                navigator.clipboard.writeText(progress.url).then(
                  () => setCopied("copied"),
                  () => {
                    setCopied("failed");
                    link.current?.select();
                  },
                );
              }}
            >
              {copied === "copied" ? "Copied" : "Copy link"}
            </button>
          </div>
          {copied === "failed" && (
            <p role="alert">Copying failed. Select the link and copy it.</p>
          )}
        </>
      )}
    </>
  );
}

/**
 * The recipient's side, shown only when starting needs input: a new name
 * when the shared one is already used for this game, or the game's ROM.
 */
export function SharedStateChoices({
  shared,
  needsRom,
  takenNames,
  busy,
  error,
  start,
}: {
  shared: SharedState;
  needsRom: boolean;
  takenNames: string[];
  busy: boolean;
  error: string;
  start: (name: string, file?: File) => void;
}) {
  const [renaming] = useState(() =>
    nameTaken(takenNames, shared.playthroughName),
  );
  const [name, setName] = useState(shared.playthroughName);
  const [nameError, setNameError] = useState("");
  const romInput = useRef<HTMLInputElement>(null);
  const nameInput = useRef<HTMLInputElement>(null);
  const length = [...name.trim()].length;
  const problem = !length
    ? "Give your playthrough a name."
    : length > 160
      ? "Use 160 characters or fewer."
      : nameTaken(takenNames, name)
        ? "A playthrough with this name already exists for this game."
        : "";
  const validName = () => {
    setNameError(problem);
    if (problem) nameInput.current?.focus();
    return !problem;
  };
  return (
    <form
      noValidate
      onSubmit={(event) => {
        event.preventDefault();
        if (busy || !validName()) return;
        if (needsRom) romInput.current?.click();
        else start(name.trim());
      }}
    >
      <p>
        <strong>
          {shared.title} · {shared.playthroughName}
        </strong>
      </p>
      {renaming && (
        <>
          <p>
            You already have a playthrough called “{shared.playthroughName}” for
            this game. Name this one differently; your playthroughs stay
            untouched.
          </p>
          <label className="share-name">
            Playthrough name
            <input
              ref={nameInput}
              value={name}
              disabled={busy}
              autoComplete="off"
              aria-invalid={Boolean(nameError)}
              onChange={(event) => {
                setName(event.target.value);
                setNameError("");
              }}
              onFocus={(event) => event.currentTarget.select()}
            />
          </label>
          {nameError && <p role="alert">{nameError}</p>}
        </>
      )}
      {needsRom && (
        <p>
          This state needs the exact game version it was shared from. Choose
          that ROM; it stays on this device and is never uploaded.
        </p>
      )}
      <details className="cloud-rom-identity">
        <summary>ROM details</summary>
        SHA-1: <code>{shared.hash}</code>
      </details>
      {error && <p role="alert">{error}</p>}
      {busy && <p role="status">Validating and starting…</p>}
      <div className="resume-recovery-actions">
        <button type="submit" disabled={busy}>
          {needsRom ? "Choose ROM and start" : "Start playthrough"}
        </button>
        <input
          ref={romInput}
          hidden
          type="file"
          accept=".gba,.zip"
          aria-label="Choose ROM for shared state"
          disabled={busy}
          onChange={(event) => {
            const file = event.target.files?.[0];
            event.target.value = "";
            if (file && validName()) start(name.trim(), file);
          }}
        />
      </div>
    </form>
  );
}
