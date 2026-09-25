import { useEffect, useState } from "react";
import { AboutScene } from "./about-scene";

// Optional instance privacy notice supplied by the deployment build.
const privacyUrl = import.meta.env.VITE_PRIVACY_URL;

export function AboutPage({
  onPlay,
  onLibrary,
  onAccount,
  hasGame,
  resumeTitle,
  resumeDetail,
  busy = false,
}: {
  onPlay: () => void;
  onLibrary: () => void;
  onAccount: () => void;
  hasGame: boolean;
  resumeTitle?: string;
  resumeDetail?: string;
  busy?: boolean;
}) {
  const [paused, setPaused] = useState(
    () => window.matchMedia("(prefers-reduced-motion: reduce)").matches,
  );
  useEffect(() => {
    const media = window.matchMedia("(prefers-reduced-motion: reduce)");
    const changed = () => setPaused(media.matches);
    media.addEventListener("change", changed);
    return () => media.removeEventListener("change", changed);
  }, []);

  return (
    <main className="about-page">
      <section className="about-hero" aria-labelledby="about-title">
        <div className="about-shell">
          <div className="about-screen">
            <AboutScene paused={paused} />
            <div className="about-intro">
              <h1 id="about-title">
                The GBA you remember.
                <span> A new horizon.</span>
              </h1>
              <p className="about-lede">
                Your favorite games, with a little more magic.
                <br /> Play locally. Pick up where you left off.
              </p>
              <button
                className="about-start"
                type="button"
                onClick={onPlay}
                disabled={busy}
              >
                {hasGame ? "Resume adventure" : "Enter player"}
                <span aria-hidden="true">▶</span>
              </button>
              {resumeTitle && (
                <p className="about-resume" title={resumeTitle}>
                  Continue · {resumeTitle}
                </p>
              )}
              {resumeDetail && (
                <p className="about-resume-detail" role="status">
                  {resumeDetail}
                </p>
              )}
              <button
                className="about-library"
                type="button"
                onClick={onLibrary}
                disabled={busy}
              >
                Browse library
              </button>
              <p className="about-tagline">
                Your games. Your saves. Your adventure.
              </p>
            </div>
            <div className="about-scene-footer">
              <button
                className="about-motion"
                type="button"
                onClick={onAccount}
                disabled={busy}
                aria-haspopup="dialog"
              >
                Account
              </button>
              <button
                className="about-motion"
                type="button"
                aria-label={paused ? "Play scene" : "Pause scene"}
                aria-pressed={paused}
                onClick={() => setPaused(!paused)}
              >
                {paused ? "Play scene" : "Pause scene"}
              </button>
            </div>
          </div>
        </div>
      </section>
      <footer className="about-legal">
        <p>
          ZiGBA is an independent GBA emulator, not affiliated with or endorsed
          by Nintendo. Game Boy Advance is a trademark of Nintendo. Pokémon is a
          trademark of Nintendo, Creatures Inc. and GAME FREAK inc.
        </p>
        {privacyUrl && (
          <p>
            <a href={privacyUrl} target="_blank" rel="noopener noreferrer">
              Privacy notice
            </a>
          </p>
        )}
      </footer>
    </main>
  );
}
