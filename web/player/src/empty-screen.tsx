import { useEffect, useRef } from "react";
import { watchSceneMotion } from "./pages/landing/motion";
import { createSunset, type Sunset } from "./pages/landing/sunset";
import { PixelIcon } from "./ui/icons";
import { button, cx } from "./ui/styles";

/**
 * The game screen before a game is open: Home's sunset over the sea, framed
 * to the screen with the sun below the text, and the file picker. Its sizes
 * follow the screen through container queries on `.game-viewport`.
 */
export function EmptyScreen({
  ready,
  onOpen,
}: {
  /** Whether the player can open a file yet. */
  ready: boolean;
  onOpen: (file: File) => void;
}) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const text = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const scene = canvas.current!;
    let disposed = false;
    let sunset: Sunset | null = null;
    // It moves like Home's scene, and only while on screen.
    let moving = false;
    let visible = false;
    const sync = () => sunset?.setRunning(moving && visible);
    const textBottom = () =>
      (text.current?.getBoundingClientRect().bottom ?? 0) -
      scene.getBoundingClientRect().top;
    const watch = new IntersectionObserver(([entry]) => {
      visible = entry.isIntersecting;
      sync();
    });
    // The text reflows when its font loads or a container query changes
    // its size, which may not resize the canvas.
    const reflow = new ResizeObserver(() => sunset?.layout());
    void createSunset(scene, textBottom)
      .then((created) => {
        if (disposed) return created.dispose();
        sunset = created;
        watch.observe(scene);
        reflow.observe(text.current!);
        sync();
      })
      .catch(() => {
        scene.dataset.ready = "error";
      });
    const stopMotion = watchSceneMotion((next) => {
      moving = next;
      sync();
    });
    return () => {
      disposed = true;
      watch.disconnect();
      reflow.disconnect();
      stopMotion();
      sunset?.dispose();
    };
  }, []);

  return (
    <div
      data-ui
      className="screen-empty absolute inset-0 overflow-hidden bg-[#072957] font-sans"
    >
      <canvas
        ref={canvas}
        aria-hidden="true"
        className="absolute inset-0 size-full"
      />
      <div
        ref={text}
        className="relative flex flex-col items-center px-4 pt-6 text-center [text-shadow:2px_2px_0_#101c2a] @min-[480px]:pt-10 @min-[700px]:pt-16"
      >
        <h1 className="text-2x text-heading @min-[480px]:text-3x @min-[700px]:text-4x">
          Your next adventure.
        </h1>
        <p className="mt-2 text-1x5 text-content @min-[480px]:mt-3 @min-[480px]:text-2x">
          Open a .gba or .zip file.
        </p>
        {/* The file input covers the button, so it takes clicks and focus
            itself; the button shows its focus and disabled state. */}
        <label
          className={cx(
            button("primaryDark", "lg"),
            "relative mt-4 [text-shadow:none] @min-[480px]:mt-6",
            "has-[:focus-visible]:outline-2 has-[:focus-visible]:outline-offset-2 has-[:focus-visible]:outline-mint",
            "has-[:disabled]:cursor-not-allowed has-[:disabled]:border-line has-[:disabled]:bg-raised has-[:disabled]:text-muted",
          )}
        >
          Open game
          <PixelIcon name="playSmall" />
          <input
            type="file"
            aria-label="Open game"
            accept=".gba,.zip"
            disabled={!ready}
            className="absolute inset-0 size-full cursor-pointer opacity-0 disabled:cursor-not-allowed"
            onChange={(event) => {
              const file = event.target.files?.[0];
              event.target.value = "";
              if (file) onOpen(file);
            }}
          />
        </label>
        {/* Only where the screen has room for it above the sun. */}
        <p className="mt-4 hidden text-1x5 text-content [@container(min-width:480px)_and_(min-height:400px)]:block">
          Games and saves stay on this device.
        </p>
      </div>
    </div>
  );
}
