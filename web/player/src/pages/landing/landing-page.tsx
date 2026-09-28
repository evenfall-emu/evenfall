import { useEffect, useRef, useState, type ReactNode } from "react";
import logo from "../../assets/brand/evenfall-logo.svg";
import { Icon, PixelIcon } from "../../ui/icons";
import { button, cx, focusRing } from "../../ui/styles";
import { createFresco, type Fresco, type LayerName } from "./fresco";
import { RewindDemo } from "./rewind-demo";
import { AccountDevices, GamePanels, RomhackFan, SpeedMeter } from "./showcase";

// Optional instance privacy notice supplied by the deployment build.
const privacyUrl = import.meta.env.VITE_PRIVACY_URL;
const SOURCE_URL = "https://github.com/evenfall-emu/zigba";

function Section({
  title,
  accent,
  children,
  illustration,
  flip = false,
  id,
}: {
  title: string;
  accent?: string;
  children: ReactNode;
  illustration: ReactNode;
  flip?: boolean;
  id?: string;
}) {
  // One column until lg, where each half is 456 px: the width every
  // illustration is laid out to fit. Columns never grow past the page, so
  // an illustration too wide for its column shows as overflow, not a
  // wider page.
  return (
    <section
      id={id}
      className="mx-auto grid max-w-5xl grid-cols-[minmax(0,1fr)] items-center gap-10 px-6 lg:grid-cols-[minmax(0,1fr)_minmax(0,1fr)] lg:gap-16"
    >
      <div className={cx(flip && "lg:order-2")}>
        <h2 className="text-3x text-heading md:text-4x">
          {title}
          {accent && <span className="block text-peach">{accent}</span>}
        </h2>
        <div className="mt-6 max-w-md space-y-4 text-2x text-content">
          {children}
        </div>
      </div>
      <div className={cx("flex justify-center", flip && "lg:order-1")}>
        {illustration}
      </div>
    </section>
  );
}

/**
 * Home: a pixel-art fresco that descends from a coastal sunset, through the
 * water, to a sunken ruin, carrying the product story on the way. Its main
 * action resumes the last playthrough when there is one.
 */
export function LandingPage({
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
  /** The game and playthrough the main action resumes. */
  resumeTitle?: string;
  /** Where the resumed checkpoint lives and when it was saved. */
  resumeDetail?: string;
  busy?: boolean;
}) {
  const surface = useRef<HTMLCanvasElement>(null);
  const descent = useRef<HTMLCanvasElement>(null);
  const abyss = useRef<HTMLCanvasElement>(null);
  const seabed = useRef<HTMLCanvasElement>(null);
  const intro = useRef<HTMLDivElement>(null);
  // The illustrations hold still under a reduced-motion preference.
  const [still, setStill] = useState(
    () => window.matchMedia("(prefers-reduced-motion: reduce)").matches,
  );
  useEffect(() => {
    const media = window.matchMedia("(prefers-reduced-motion: reduce)");
    const changed = () => setStill(media.matches);
    media.addEventListener("change", changed);
    return () => media.removeEventListener("change", changed);
  }, []);
  const features = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let disposed = false;
    let fresco: Fresco | null = null;
    const layers = {
      surface: surface.current!,
      descent: descent.current!,
      abyss: abyss.current!,
      seabed: seabed.current!,
    };
    const names = new Map<Element, LayerName>(
      Object.entries(layers).map(([name, canvas]) => [
        canvas,
        name as LayerName,
      ]),
    );
    // The scene always moves, except while the page is hidden or the
    // window is unfocused, and never under a reduced-motion preference.
    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)");
    const sync = () =>
      fresco?.setRunning(
        !reduced.matches && !document.hidden && document.hasFocus(),
      );
    // Only layers on screen animate. A target whose edge only touches the
    // view counts as intersecting; the 1px inset keeps it off until visible.
    const watch = new IntersectionObserver(
      (entries) => {
        for (const entry of entries)
          fresco?.setVisible(names.get(entry.target)!, entry.isIntersecting);
      },
      { rootMargin: "-1px 0px" },
    );
    // The dive burst starts once, when the waterline reaches the upper part
    // of the view.
    const dive = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting && !reduced.matches) fresco?.dive();
      },
      { rootMargin: "0px 0px -35% 0px" },
    );
    // Where the hero text sits over the surface canvas, for the camera to
    // keep the sun clear of it; and how wide the feature text column is, in
    // CSS pixels, for the abyss to keep its walls and bubbles beside it.
    const text = () => {
      const box = intro.current?.getBoundingClientRect();
      const canvas = surface.current?.getBoundingClientRect();
      if (!box || !canvas) return { bottom: 0, right: 0 };
      return {
        bottom: box.bottom - canvas.top,
        right: box.right - canvas.left,
      };
    };
    const column = () =>
      features.current?.querySelector("section")?.getBoundingClientRect()
        .width ?? 0;
    void createFresco(layers, text, column)
      .then((created) => {
        if (disposed) return created.dispose();
        fresco = created;
        for (const canvas of Object.values(layers)) watch.observe(canvas);
        dive.observe(layers.descent);
        sync();
      })
      .catch(() => {
        layers.surface.dataset.ready = "error";
      });
    document.addEventListener("visibilitychange", sync);
    window.addEventListener("focus", sync);
    window.addEventListener("blur", sync);
    reduced.addEventListener("change", sync);
    return () => {
      disposed = true;
      watch.disconnect();
      dive.disconnect();
      document.removeEventListener("visibilitychange", sync);
      window.removeEventListener("focus", sync);
      window.removeEventListener("blur", sync);
      reduced.removeEventListener("change", sync);
      fresco?.dispose();
    };
  }, []);

  const link = cx(
    "cursor-pointer text-content underline underline-offset-4 hover:text-mint-hover",
    focusRing,
  );

  return (
    <div
      data-ui
      className="landing min-h-dvh bg-[#012d53] font-sans text-2x text-content"
    >
      <header className="absolute inset-x-0 top-0 z-20">
        <div className="mx-auto flex h-20 max-w-[1600px] items-center gap-6 px-6 lg:px-10">
          <img src={logo} alt="evenfall" className="h-10 w-auto md:h-12" />
          <nav
            aria-label="Main navigation"
            className="ml-auto flex items-center gap-1 lg:gap-4"
          >
            <a
              href="#features"
              className={cx(button("ghost"), "font-normal max-md:hidden")}
              onClick={(event) => {
                // Dive: glide down past the waterline instead of jumping.
                // The hash stays out of the URL, whose path is the page.
                event.preventDefault();
                const target = features.current;
                if (!target) return;
                target.scrollIntoView({
                  behavior: still ? "auto" : "smooth",
                  block: "start",
                });
                target.focus({ preventScroll: true });
              }}
            >
              Features
            </a>
            {/* On phones Library and Account show only their icons, which
                fit beside the logo down to 360 px. */}
            <button
              className={cx(button("ghost"), "font-normal max-md:px-2")}
              disabled={busy}
              onClick={onLibrary}
            >
              <PixelIcon name="book" scale={3} className="md:hidden" />
              <span className="max-md:sr-only">Library</span>
            </button>
            <button
              className={cx(button("ghost"), "gap-3 font-normal max-md:px-2")}
              aria-haspopup="dialog"
              disabled={busy}
              onClick={onAccount}
            >
              <PixelIcon name="avatar" inks={{ "#": "text-mint" }} />
              <span className="max-md:sr-only">Account</span>
            </button>
          </nav>
        </div>
      </header>

      <main>
        <section
          aria-labelledby="landing-title"
          className="relative isolate h-[max(640px,100svh)] overflow-hidden bg-[#072957] max-md:h-auto max-md:min-h-[max(640px,100svh)] max-md:pb-[460px]"
        >
          <canvas
            ref={surface}
            aria-hidden="true"
            className="landing-plate absolute inset-0 -z-10 size-full"
          />
          {/* On phones the scene sits below the text: the bottom padding
              leaves room for the sun at a full-size art pixel. */}
          <div className="mx-auto h-full max-w-[1600px] px-6 pt-32 [text-shadow:2px_2px_0_#101c2a] md:px-10 md:pt-44">
            <div ref={intro} className="max-w-fit">
              <h1
                id="landing-title"
                className="text-3x leading-[40px] text-heading sm:text-4x sm:leading-[48px] lg:text-5x lg:leading-[60px]"
              >
                The games you love.
                <span className="block text-peach">
                  Worlds you haven’t met.
                </span>
              </h1>
              <p className="mt-6 max-w-[34ch] text-2x text-content">
                Bring your own base game. Discover community-made adventures,
                and play them your way.
              </p>
              <button
                className={cx(button("primaryDark", "lg"), "mt-8")}
                disabled={busy}
                onClick={onPlay}
              >
                {hasGame ? "Resume adventure" : "Open player"}
                <PixelIcon name="playSmall" />
              </button>
              {resumeTitle && (
                <p className="landing-resume mt-6 max-w-[40ch] text-2x text-heading">
                  Continue · {resumeTitle}
                </p>
              )}
              {resumeDetail && (
                <p
                  className="landing-resume-detail mt-1 text-1x5 text-content"
                  role="status"
                >
                  {resumeDetail}
                </p>
              )}
            </div>
          </div>
        </section>

        <div aria-hidden="true" className="relative">
          <canvas
            ref={descent}
            className="landing-plate block h-[clamp(360px,55vw,560px)] w-full"
          />
        </div>

        <div
          id="features"
          ref={features}
          tabIndex={-1}
          aria-label="Features"
          className="relative isolate outline-none"
        >
          {/* The abyss: deep water behind the product story. */}
          <canvas
            ref={abyss}
            aria-hidden="true"
            className="landing-plate absolute inset-0 -z-10 size-full"
          />
          <div className="space-y-28 py-12 pb-28 md:space-y-36">
            <Section
              title="One ROM."
              accent="Dozens of adventures."
              illustration={<RomhackFan still={still} />}
            >
              <p>
                Bring the game you own. The Library turns it into community
                romhacks: new regions, new campaigns, harder challenges. Pick
                one and Evenfall patches your ROM on your device, then plays.
              </p>
              <p className="text-muted">
                Your ROM never leaves your device, and patched games are checked
                against the exact release before they run.
              </p>
            </Section>

            <Section
              flip
              title="Go back hours."
              accent="Even days."
              illustration={<RewindDemo still={still} />}
            >
              <p>
                Evenfall captures your game as you play and keeps it, so you are
                always safe without saving. Pick any checkpoint from minutes,
                hours or days ago, preview it, and play on from there.
              </p>
              <p className="text-muted">
                Delta-encoded snapshots keep a long history in little space:
                dense for recent play, thinning with age.
              </p>
            </Section>

            <Section
              title="One account."
              accent="Every device."
              illustration={<AccountDevices />}
            >
              <p>
                Sign in once and your playthroughs, checkpoints and saves back
                up as you play. Open any device and continue where you left off.
              </p>
              <p>
                Stuck on a boss, or proud of a run? Share state sends a friend a
                link to your exact moment, and they play on from it.
              </p>
              <p className="text-muted">
                Online features are optional. Playing never needs an account.
              </p>
            </Section>

            <Section
              flip
              title="See inside"
              accent="the game."
              illustration={<GamePanels still={still} />}
            >
              <p>
                Supported games get their own dashboards, read live from the
                game as you play: your team, their stats and moves, the opponent
                across the field, the trainers still ahead.
              </p>
              <p className="text-muted">
                Panels only read the game&rsquo;s memory; they never change it.
              </p>
            </Section>

            <Section
              title="Fast,"
              accent="and exact."
              illustration={<SpeedMeter still={still} />}
            >
              <p>
                A new emulator core, accurate to the bus cycle, built for speed
                without shortcuts. It runs twice as fast as mGBA on the game we
                measure, fast forwards to 16×, and leaves plenty of room on a
                laptop or a phone.
              </p>
            </Section>
          </div>
        </div>
      </main>

      {/* The seabed: its canvas runs behind the last section and the footer,
          whose text rests on the sand where it settles into a flat floor. */}
      <div className="relative isolate bg-[#012d53]">
        <canvas
          ref={seabed}
          aria-hidden="true"
          className="landing-plate absolute inset-0 -z-10 size-full"
        />
        <section
          aria-labelledby="landing-open"
          className="mx-auto flex min-h-[clamp(300px,26vw,380px)] max-w-2xl flex-col items-center justify-center gap-6 px-6 pt-16 text-center"
        >
          <h2 id="landing-open" className="text-3x text-heading md:text-4x">
            Built in the open.
          </h2>
          <p className="bg-[#012d53]/80 px-2 text-2x">
            A new emulator core, a WebAssembly player and a TypeScript SDK, all
            open source under GPL licensing.
          </p>
          <a
            href={SOURCE_URL}
            target="_blank"
            rel="noopener noreferrer"
            className={cx(button("outline"), "bg-[#012d53]/80 font-normal")}
          >
            View source
            <Icon name="externalLink" className="size-5" />
          </a>
        </section>
        <footer className="px-6 pt-[clamp(56px,6vw,96px)] pb-8 text-center text-1x5 text-content [text-shadow:2px_2px_0_#011e3c]">
          <p className="mx-auto mt-4 max-w-[72ch] text-muted">
            ZiGBA is an independent GBA emulator, not affiliated with or
            endorsed by Nintendo. Game Boy Advance is a trademark of Nintendo.
            Pokémon is a trademark of Nintendo, Creatures Inc. and GAME FREAK
            inc.
          </p>
          {privacyUrl && (
            <p className="mt-2">
              <a
                className={link}
                href={privacyUrl}
                target="_blank"
                rel="noopener noreferrer"
              >
                Privacy notice
              </a>
            </p>
          )}
        </footer>
      </div>
    </div>
  );
}
