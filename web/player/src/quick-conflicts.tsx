import { useState } from "react";
import type { QuickConflict } from "./cloud";

type Keep = "local" | "cloud";

const when = (time: number) =>
  time > 0
    ? new Date(time).toLocaleString(undefined, {
        dateStyle: "medium",
        timeStyle: "short",
      })
    : "time unknown";

/**
 * One decision for slots changed on both sides since their last sync. Each
 * slot preselects its newest save; nothing is replaced until Keep is pressed.
 */
export function QuickConflictChoices({
  conflicts,
  busy,
  apply,
  later,
}: {
  conflicts: QuickConflict[];
  busy: boolean;
  apply: (choices: { slot: number; keep: Keep }[]) => void;
  later: () => void;
}) {
  const [choices, setChoices] = useState<Record<number, Keep>>(() =>
    Object.fromEntries(
      conflicts.map((conflict) => [
        conflict.slot,
        conflict.localSavedAt >= conflict.cloudSavedAt ? "local" : "cloud",
      ]),
    ),
  );
  const choose = (slot: number, keep: Keep) =>
    setChoices((current) => ({ ...current, [slot]: keep }));
  return (
    <>
      <p>
        {conflicts.length === 1
          ? "This quick slot was"
          : "These quick slots were"}{" "}
        saved on this device and on another device since they last synced.
        Choose the copy to keep; the newer save is selected. The other copy is
        replaced, so export a state first if you need both.
      </p>
      <div className="quick-conflicts">
        {conflicts.map((conflict) => (
          <fieldset key={conflict.slot} disabled={busy}>
            <legend>Slot {conflict.slot}</legend>
            {(["local", "cloud"] as const).map((keep) => (
              <label key={keep}>
                <input
                  type="radio"
                  name={`quick-conflict-${conflict.slot}`}
                  checked={choices[conflict.slot] === keep}
                  onChange={() => choose(conflict.slot, keep)}
                />
                {keep === "local" ? "This device" : "Cloud"} ·{" "}
                {when(
                  keep === "local"
                    ? conflict.localSavedAt
                    : conflict.cloudSavedAt,
                )}
              </label>
            ))}
          </fieldset>
        ))}
      </div>
      <div className="resume-recovery-actions">
        <button
          disabled={busy}
          onClick={() =>
            apply(
              conflicts.map((conflict) => ({
                slot: conflict.slot,
                keep: choices[conflict.slot],
              })),
            )
          }
        >
          Keep selected copies
        </button>
        <button disabled={busy} onClick={later}>
          Decide later
        </button>
      </div>
    </>
  );
}
