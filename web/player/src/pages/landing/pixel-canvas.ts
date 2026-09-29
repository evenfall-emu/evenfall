/** A canvas's size in device pixels, and device pixels per CSS pixel. */
export interface DeviceSize {
  width: number;
  height: number;
  ratio: number;
}

/**
 * Keeps a canvas's backing store equal to its box in device pixels, so the
 * browser never resamples what is drawn into it: whatever is drawn at whole
 * device pixels stays exactly there on screen.
 *
 * The size comes from ResizeObserver's device-pixel-content-box, measured
 * per canvas: devicePixelRatio can disagree with it under browser-level
 * scaling. Safari reports no device-pixel box, so there the size is the CSS
 * size times devicePixelRatio, rounded, which can be one device pixel off at
 * an edge.
 */
export function observeDeviceSize(
  canvas: HTMLCanvasElement,
  onResize: (size: DeviceSize) => void,
): () => void {
  const observer = new ResizeObserver(([entry]) => {
    const css = entry.contentBoxSize[0];
    const device = entry.devicePixelContentBoxSize?.[0];
    if (!css || css.inlineSize === 0 || css.blockSize === 0) return;
    const width = device
      ? device.inlineSize
      : Math.round(css.inlineSize * devicePixelRatio);
    const height = device
      ? device.blockSize
      : Math.round(css.blockSize * devicePixelRatio);
    // Assigning a size clears the canvas even when it is unchanged.
    if (canvas.width !== width) canvas.width = width;
    if (canvas.height !== height) canvas.height = height;
    onResize({ width, height, ratio: width / css.inlineSize });
  });
  try {
    observer.observe(canvas, { box: "device-pixel-content-box" });
  } catch {
    observer.observe(canvas);
  }
  return () => observer.disconnect();
}

/** An image, decoded and ready to draw. */
export function loadImage(src: string): Promise<HTMLImageElement> {
  const image = new Image();
  image.src = src;
  return image.decode().then(() => image);
}
