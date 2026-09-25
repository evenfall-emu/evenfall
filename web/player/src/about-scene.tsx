import { useEffect, useRef } from "react";
import coast from "./assets/sunset/coast.png";
import travelers from "./assets/sunset/travelers.png";
import { createSunsetRenderer, type SunsetRenderer } from "./sunset-renderer";

export function AboutScene({ paused = false }: { paused?: boolean }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const rendererRef = useRef<SunsetRenderer | null>(null);
  const pausedRef = useRef(paused);
  pausedRef.current = paused;

  useEffect(() => {
    const canvas = canvasRef.current!;
    let disposed = false;
    let visible = true;
    const background = new Image();
    const sprites = new Image();
    background.src = coast;
    sprites.src = travelers;

    const sync = () =>
      rendererRef.current?.setRunning(
        !pausedRef.current && visible && !document.hidden,
      );
    const observer = new IntersectionObserver(([entry]) => {
      visible = entry.isIntersecting;
      sync();
    });
    observer.observe(canvas);
    document.addEventListener("visibilitychange", sync);

    void Promise.all([background.decode(), sprites.decode()])
      .then(() => {
        if (disposed) return;
        rendererRef.current = createSunsetRenderer(canvas, background, sprites);
        canvas.dataset.ready = "true";
        sync();
      })
      .catch(() => {
        // The ordinary image underneath remains visible if decoding fails.
        if (!disposed) canvas.dataset.ready = "error";
      });

    return () => {
      disposed = true;
      observer.disconnect();
      document.removeEventListener("visibilitychange", sync);
      rendererRef.current?.dispose();
      rendererRef.current = null;
    };
  }, []);

  useEffect(() => {
    const rect = canvasRef.current?.getBoundingClientRect();
    const visible = rect && rect.bottom > 0 && rect.top < window.innerHeight;
    rendererRef.current?.setRunning(!paused && !!visible && !document.hidden);
  }, [paused]);

  return (
    <div className="about-scene" aria-hidden="true">
      <img className="scene-fallback" src={coast} alt="" decoding="async" />
      <canvas
        ref={canvasRef}
        className="scene-canvas"
        width={836}
        height={471}
      />
    </div>
  );
}
