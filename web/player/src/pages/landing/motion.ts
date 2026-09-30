/** Everything moves in whole art pixels; fifteen frames a second is enough. */
export const FRAME_SECONDS = 1 / 15;

const REDUCED = "(prefers-reduced-motion: reduce)";

/** Whether the viewer prefers reduced motion right now. */
export const prefersStill = () => window.matchMedia(REDUCED).matches;

/**
 * Reports whether pixel-art scenes may move: always, except while the page
 * is hidden or the window is unfocused, and never under a reduced-motion
 * preference. Calls `onChange` at once and on every change; returns a
 * function that stops watching.
 */
export function watchSceneMotion(
  onChange: (moving: boolean) => void,
): () => void {
  const reduced = window.matchMedia(REDUCED);
  const sync = () =>
    onChange(!reduced.matches && !document.hidden && document.hasFocus());
  document.addEventListener("visibilitychange", sync);
  window.addEventListener("focus", sync);
  window.addEventListener("blur", sync);
  reduced.addEventListener("change", sync);
  sync();
  return () => {
    document.removeEventListener("visibilitychange", sync);
    window.removeEventListener("focus", sync);
    window.removeEventListener("blur", sync);
    reduced.removeEventListener("change", sync);
  };
}
