import { useEffect, useRef, useState, type RefObject } from "react";
import { Dialog } from "@base-ui/react/dialog";
import { Field } from "@base-ui/react/field";
import { player } from "./player";

export function PlaythroughDialog({
  mode,
  gameTitle,
  currentName = "",
  disabled,
  createOpen,
  onCreateOpenChange,
  selectTrigger,
}: {
  mode: "create" | "rename";
  gameTitle: string;
  currentName?: string;
  disabled: boolean;
  createOpen?: boolean;
  onCreateOpenChange?: (open: boolean) => void;
  selectTrigger?: RefObject<HTMLButtonElement | null>;
}) {
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const wasPlaying = useRef(false);
  const pausing = useRef(Promise.resolve());
  const rename = mode === "rename";
  const visible = rename ? open : Boolean(createOpen);
  const title = rename ? "Rename playthrough" : "New playthrough";
  const length = [...name.trim()].length;
  const nameError = !length
    ? "Give your playthrough a name."
    : length > 160
      ? "Use 160 characters or fewer."
      : "";

  useEffect(() => {
    if (!rename && createOpen) {
      setName("");
      setError("");
      wasPlaying.current = player.state.playing;
      pausing.current = player.pause();
      void pausing.current.catch((cause) => setError(String(cause)));
    }
  }, [createOpen, rename]);

  async function close() {
    setBusy(true);
    try {
      await pausing.current;
      if (
        wasPlaying.current &&
        !player.state.playing &&
        !player.state.changing &&
        !document.hidden
      )
        await player.play();
    } catch (cause) {
      player.error(cause);
    } finally {
      if (rename) setOpen(false);
      else onCreateOpenChange?.(false);
      setBusy(false);
    }
  }

  return (
    <Dialog.Root
      open={visible}
      onOpenChange={(next, details) => {
        if (busy) {
          details.cancel();
          return;
        }
        if (!next) {
          void close();
          return;
        }
        if (!rename) return;
        setName(rename ? currentName : "");
        setError("");
        wasPlaying.current = player.state.playing;
        pausing.current = player.pause();
        void pausing.current.catch((cause) => setError(String(cause)));
        setOpen(true);
      }}
    >
      {rename && (
        <Dialog.Trigger
          ref={trigger}
          className="playthrough-icon-button"
          aria-label="Rename playthrough"
          title="Rename playthrough"
          disabled={disabled}
        >
          <svg
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.8"
            aria-hidden="true"
          >
            <path d="M12 20h9M4 16.5V20h3.5L19 8.5 15.5 5 4 16.5ZM14 6.5l3.5 3.5" />
          </svg>
        </Dialog.Trigger>
      )}
      <Dialog.Portal>
        <Dialog.Backdrop
          className="playthrough-dialog-backdrop"
          // Opened from the phone menu sheet, the dialog is nested; its own
          // backdrop keeps tap-outside-to-close working there.
          forceRender
        />
        <Dialog.Popup
          className="playthrough-dialog"
          initialFocus={input}
          finalFocus={rename ? trigger : selectTrigger}
          aria-busy={busy}
        >
          <header>
            <Dialog.Title>{title}</Dialog.Title>
            <Dialog.Close aria-label="Close dialog" disabled={busy}>
              ×
            </Dialog.Close>
          </header>
          <form
            noValidate
            onSubmit={async (event) => {
              event.preventDefault();
              if (busy) return;
              if (nameError) {
                setError(nameError);
                input.current?.focus();
                return;
              }
              setBusy(true);
              setError("");
              try {
                await pausing.current;
                if (rename) await player.renamePlaythrough(name.trim());
                else await player.switchPlaythrough(undefined, name.trim());
                await close();
              } catch (cause) {
                setError(
                  cause instanceof Error ? cause.message : String(cause),
                );
                setBusy(false);
              }
            }}
          >
            <p className="playthrough-dialog-game">{gameTitle}</p>
            <Dialog.Description className="playthrough-dialog-description">
              {rename
                ? "A new name for the same adventure. Your saves stay untouched."
                : "Start a fresh adventure with its own saves. Your other playthroughs stay untouched."}
            </Dialog.Description>
            <Field.Root
              className="playthrough-dialog-field"
              name="playthroughName"
              disabled={busy}
              invalid={Boolean(error && nameError)}
            >
              <Field.Label>Playthrough name</Field.Label>
              <Field.Control
                ref={input}
                value={name}
                onValueChange={(value) => {
                  setName(value);
                  setError("");
                }}
                onFocus={(event) => {
                  if (rename) event.currentTarget.select();
                }}
                placeholder="My next adventure"
                autoComplete="off"
                required
              />
              <Field.Description className="playthrough-dialog-hint">
                Up to 160 characters.
              </Field.Description>
              {error && (
                <Field.Error
                  match
                  role="alert"
                  className="playthrough-dialog-error"
                >
                  {error}
                </Field.Error>
              )}
            </Field.Root>
            <div className="playthrough-dialog-actions">
              <Dialog.Close disabled={busy}>Cancel</Dialog.Close>
              <button
                type="submit"
                className="playthrough-dialog-submit"
                disabled={busy}
              >
                {busy ? "Saving…" : rename ? "Save name" : "Create playthrough"}
              </button>
            </div>
          </form>
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

export function DeletePlaythroughDialog({
  name,
  disabled,
}: {
  name: string;
  disabled: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const trigger = useRef<HTMLButtonElement>(null);
  const cancel = useRef<HTMLButtonElement>(null);
  const wasPlaying = useRef(false);
  const pausing = useRef(Promise.resolve());

  async function close() {
    setBusy(true);
    try {
      await pausing.current;
      if (
        wasPlaying.current &&
        player.state.game &&
        !player.state.playing &&
        !document.hidden
      )
        await player.play();
    } catch (cause) {
      player.error(cause);
    } finally {
      setOpen(false);
      setBusy(false);
    }
  }

  return (
    <Dialog.Root
      open={open}
      onOpenChange={(next, details) => {
        if (busy) {
          details.cancel();
          return;
        }
        if (!next) {
          void close();
          return;
        }
        setError("");
        wasPlaying.current = player.state.playing;
        pausing.current = player.pause();
        void pausing.current.catch((cause) => setError(String(cause)));
        setOpen(true);
      }}
    >
      <Dialog.Trigger
        ref={trigger}
        className="playthrough-icon-button"
        aria-label="Delete playthrough"
        title="Delete playthrough"
        disabled={disabled}
      >
        <svg
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.8"
          aria-hidden="true"
        >
          <path d="M3 6h18m-2 0-.8 14H5.8L5 6m4 0V4h6v2m-5 4v7m4-7v7" />
        </svg>
      </Dialog.Trigger>
      <Dialog.Portal>
        <Dialog.Backdrop
          className="playthrough-dialog-backdrop"
          // Opened from the phone menu sheet, the dialog is nested; its own
          // backdrop keeps tap-outside-to-close working there.
          forceRender
        />
        <Dialog.Popup
          className="playthrough-dialog playthrough-delete-dialog"
          initialFocus={cancel}
          finalFocus={trigger}
          aria-busy={busy}
        >
          <header>
            <Dialog.Title>Delete playthrough?</Dialog.Title>
            <Dialog.Close aria-label="Close dialog" disabled={busy}>
              ×
            </Dialog.Close>
          </header>
          <div className="playthrough-delete-content">
            <Dialog.Description>
              Delete “{name}” and its local saves, checkpoints and quick slots?
              This cannot be undone. When online and signed in, its cloud backup
              is also deleted. Offline or signed out, the cloud backup stays.
            </Dialog.Description>
            {error && (
              <p className="playthrough-dialog-error" role="alert">
                {error}
              </p>
            )}
            <div className="playthrough-dialog-actions">
              <Dialog.Close ref={cancel} disabled={busy}>
                Cancel
              </Dialog.Close>
              <button
                className="playthrough-delete-submit"
                disabled={busy}
                onClick={async () => {
                  setBusy(true);
                  setError("");
                  try {
                    await pausing.current;
                    await player.deletePlaythrough();
                    await close();
                  } catch (cause) {
                    setError(
                      cause instanceof Error ? cause.message : String(cause),
                    );
                    setBusy(false);
                  }
                }}
              >
                {busy ? "Deleting…" : "Delete playthrough"}
              </button>
            </div>
          </div>
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
