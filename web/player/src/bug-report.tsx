import { useState } from "react";
import { Dialog, DialogActions } from "./ui/dialog";
import { CheckboxRow, FormError, TextField } from "./ui/fields";
import { PixelIcon } from "./ui/icons";
import { button, cx, focusRing } from "./ui/styles";

// Optional instance privacy notice supplied by the deployment build.
const privacyUrl = import.meta.env.VITE_PRIVACY_URL;
// The official build names its event processor, which notifies maintainers.
const notifiesPostHog = Boolean(import.meta.env.VITE_POSTHOG_KEY);
const MAX_MESSAGE = 2000;

/**
 * The bug report dialog. `send` uploads the report and resolves to its ID.
 * The dialog keeps the message when sending fails, so the reporter can retry,
 * and cannot be dismissed while a report is on its way.
 */
export function BugReportDialog({
  hasGame,
  signedIn,
  container,
  send,
  onClose,
}: {
  /** A game is open, so its state and battery save can be attached. */
  hasGame: boolean;
  signedIn: boolean;
  container?: HTMLElement;
  send: (report: {
    message: string;
    attachGame: boolean;
    includeAccount: boolean;
  }) => Promise<string>;
  onClose: () => void;
}) {
  const [message, setMessage] = useState("");
  const [attachGame, setAttachGame] = useState(true);
  const [includeAccount, setIncludeAccount] = useState(false);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState("");
  const [sentId, setSentId] = useState("");
  const length = Array.from(message.trim()).length;
  const ready = length > 0 && length <= MAX_MESSAGE;

  function submit() {
    if (sending || !ready) return;
    setSending(true);
    setError("");
    send({
      message: message.trim(),
      attachGame: hasGame && attachGame,
      includeAccount: signedIn && includeAccount,
    })
      .then(setSentId, (cause: unknown) =>
        setError(cause instanceof Error ? cause.message : String(cause)),
      )
      .finally(() => setSending(false));
  }

  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      busy={sending}
      container={container}
      title="Report a bug"
      subtitle={sentId ? undefined : "Tell the maintainers what went wrong."}
      leading={<PixelIcon name="bug" scale={3} className="mt-1 text-mint" />}
    >
      {sentId ? (
        <>
          <p role="status" className="mt-5">
            Thank you. Your report was sent.
          </p>
          <p className="mt-2 text-1x5 text-muted">
            Report ID: <code className="break-all">{sentId}</code>
          </p>
          <button
            className={cx(button("primary"), "mt-6 w-full")}
            onClick={onClose}
          >
            Close
          </button>
        </>
      ) : (
        <form
          noValidate
          onSubmit={(event) => {
            event.preventDefault();
            submit();
          }}
        >
          <div className="mt-5 space-y-5">
            <TextField
              label="What happened?"
              value={message}
              onChange={setMessage}
              rows={5}
              maxLength={MAX_MESSAGE}
              disabled={sending}
              placeholder="What you did, what you expected, and what happened instead."
            />
            {hasGame && (
              <CheckboxRow
                checked={attachGame}
                onChange={setAttachGame}
                disabled={sending}
              >
                <span>
                  Attach this game’s state and battery save
                  <span className="mt-1 block text-1x5 text-muted">
                    They hold the game’s memory, including names you typed in
                    the game. The ROM is never sent, only its SHA-1 and title.
                  </span>
                </span>
              </CheckboxRow>
            )}
            {signedIn && (
              <CheckboxRow
                checked={includeAccount}
                onChange={setIncludeAccount}
                disabled={sending}
              >
                Include my account, so the maintainers can follow up
              </CheckboxRow>
            )}
          </div>
          <p className="mt-5 text-1x5 text-muted">
            Your browser’s user agent and the player’s version go with the
            report.
            {notifiesPostHog &&
              " The message and these details also go to PostHog, which notifies the maintainers."}
            {privacyUrl && (
              <>
                {" "}
                <a
                  href={privacyUrl}
                  target="_blank"
                  rel="noopener noreferrer"
                  className={cx(
                    "text-mint underline underline-offset-4 hover:text-mint-hover",
                    focusRing,
                  )}
                >
                  Privacy notice
                </a>
              </>
            )}
          </p>
          <FormError message={error} />
          <DialogActions>
            <button
              type="button"
              className={button("outline")}
              disabled={sending}
              onClick={onClose}
            >
              Cancel
            </button>
            <button
              type="submit"
              className={button("primary")}
              disabled={sending || !ready}
            >
              {sending ? "Sending…" : "Send report"}
            </button>
          </DialogActions>
        </form>
      )}
    </Dialog>
  );
}
