import { expect, test, type Page } from "@playwright/test";

// Home's pixel-art fresco. Playwright's viewport emulation reports
// CSS-sized device boxes, so these checks cover framing and behavior at
// scale 1; whole-pixel rendering at fractional browser scales needs a
// browser launched with a real scale factor.

const plates = (page: Page) => page.locator("canvas.landing-plate");

async function open(page: Page) {
  await page.goto("/");
  await expect(
    page.getByRole("heading", {
      level: 1,
      name: "The games you love. Worlds you haven’t met.",
    }),
  ).toBeVisible();
  await expect(plates(page)).toHaveCount(4);
  for (const plate of await plates(page).all())
    await expect(plate).toHaveAttribute("data-scale", /^\d+$/);
  await page.evaluate(() => document.fonts.ready);
}

/** The sun's box in CSS pixels, from the surface canvas's camera. */
function sun(page: Page) {
  return plates(page)
    .first()
    .evaluate((canvas: HTMLCanvasElement) => {
      const { scale, x, y, ratio } = canvas.dataset;
      const k = Number(scale) / Number(ratio);
      const left = Number(x) / Number(ratio);
      const top = Number(y) / Number(ratio);
      // SUN in fresco.ts: art pixels 297–346 across, 159–197 down.
      return {
        left: left + 297 * k,
        right: left + 347 * k,
        top: top + 159 * k,
        bottom: top + 198 * k,
        width: canvas.getBoundingClientRect().width,
        height: canvas.getBoundingClientRect().height,
      };
    });
}

test("the sun stays whole and clear of the hero text from ultrawide to phone", async ({
  page,
}) => {
  for (const viewport of [
    { width: 3440, height: 1440, across: 0.72 },
    { width: 1512, height: 982, across: 0.72 },
    { width: 1280, height: 720, across: undefined },
    { width: 390, height: 844, across: 0.52 },
  ]) {
    await page.setViewportSize(viewport);
    await open(page);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    const box = await sun(page);
    expect(box.left).toBeGreaterThanOrEqual(0);
    expect(box.right).toBeLessThanOrEqual(box.width);
    expect(box.top).toBeGreaterThanOrEqual(0);
    expect(box.bottom).toBeLessThanOrEqual(box.height);
    if (viewport.across !== undefined)
      expect((box.left + box.right) / 2 / box.width).toBeCloseTo(
        viewport.across,
        1,
      );
    const text = await page
      .getByRole("heading", { level: 1 })
      .locator("..")
      .boundingBox();
    // The sun sits beside the text on wide views and below it on phones.
    const clear =
      box.left >= text!.x + text!.width || box.top >= text!.y + text!.height;
    expect(clear, `${viewport.width}×${viewport.height}`).toBe(true);
  }
});

test("the lower plates share the surface's grid, so their seams line up", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await open(page);
  const cameras = await plates(page).evaluateAll((canvases) =>
    canvases.map((canvas) => ({
      scale: (canvas as HTMLCanvasElement).dataset.scale,
      x: (canvas as HTMLCanvasElement).dataset.x,
    })),
  );
  expect(new Set(cameras.map((camera) => camera.scale)).size).toBe(1);
  expect(new Set(cameras.map((camera) => camera.x)).size).toBe(1);
});

/** A canvas's pixels, for comparing frames. */
const pixels = (canvas: ReturnType<typeof plates>) =>
  canvas.evaluate((element: HTMLCanvasElement) =>
    Array.from(
      element
        .getContext("2d")!
        .getImageData(0, 0, element.width, element.height).data,
    ).join(),
  );

test("the sea rolls and the layers on screen animate, with no pause control", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await open(page);
  await page.bringToFront();
  await expect(page.getByRole("button", { name: /scene/i })).toHaveCount(0);
  const surface = plates(page).first();
  const descent = plates(page).nth(1);
  // At the top only the surface is on screen: its waves move, the water
  // below the fold stays still.
  await expect(surface).toHaveAttribute("data-motion", "playing");
  await expect(descent).toHaveAttribute("data-motion", "still");
  const before = await pixels(surface);
  await expect.poll(() => pixels(surface)).not.toBe(before);

  await descent.scrollIntoViewIfNeeded();
  await expect(descent).toHaveAttribute("data-motion", "playing");
  const bubbles = await pixels(descent);
  await expect.poll(() => pixels(descent)).not.toBe(bubbles);
  await plates(page).nth(2).scrollIntoViewIfNeeded();
  await expect(plates(page).nth(2)).toHaveAttribute("data-motion", "playing");
});

test("hiding the page stops the scene", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await open(page);
  await page.bringToFront();
  const surface = plates(page).first();
  await expect(surface).toHaveAttribute("data-motion", "playing");
  await page.evaluate(() => {
    Object.defineProperty(document, "hidden", {
      value: true,
      configurable: true,
    });
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await expect(surface).toHaveAttribute("data-motion", "still");
  const frozen = await pixels(surface);
  await page.waitForTimeout(400);
  expect(await pixels(surface)).toBe(frozen);
});

test("reduced motion shows a still scene", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.setViewportSize({ width: 1440, height: 900 });
  await open(page);
  const surface = plates(page).first();
  await expect(surface).toHaveAttribute("data-motion", "still");
  const still = await pixels(surface);
  await page.waitForTimeout(400);
  expect(await pixels(surface)).toBe(still);
});

test("the footer sits on the seabed's sand, and the sand ends the page", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await open(page);
  const seabed = plates(page).nth(3);
  const footer = page.getByRole("contentinfo");
  const [canvas, text] = await Promise.all([
    seabed.boundingBox(),
    footer.getByText(/not affiliated with or endorsed by/).boundingBox(),
  ]);
  // The seabed plate rests on the canvas's bottom, which is the page's end.
  // The footer's text sits on its sand: below the plate's row 88, where the
  // ruins end (see its README).
  const { scale, plateTop } = await seabed.evaluate(
    (element: HTMLCanvasElement) => {
      const ratio = Number(element.dataset.ratio);
      return {
        scale: Number(element.dataset.scale) / ratio,
        plateTop: Number(element.dataset.y) / ratio,
      };
    },
  );
  expect(text!.y).toBeGreaterThan(canvas!.y + plateTop + 88 * scale);
  expect(canvas!.y + canvas!.height).toBeCloseTo(
    await page.evaluate(() => document.documentElement.scrollHeight),
    0,
  );
  // The canvas's last row is one flat color: the sand's settled floor.
  // A Set would not survive the trip out of the page, so return an array.
  const floor = await seabed.evaluate((element: HTMLCanvasElement) => {
    const data = element
      .getContext("2d")!
      .getImageData(0, element.height - 1, element.width, 1).data;
    return [
      ...new Set(
        Array.from({ length: element.width }, (_, x) =>
          [data[x * 4], data[x * 4 + 1], data[x * 4 + 2]].join(),
        ),
      ),
    ];
  });
  expect(floor).toEqual(["9,52,77"]);
});

test("the descent and the seabed meet the abyss on deep water at every width", async ({
  page,
}) => {
  // Wider than 1920 pixels the scale rises to cover the width: the seabed's
  // section grows with it, so its whole plate fits, while the descent's
  // canvas crops its plate on ultrawide views, as the seabed's does on
  // tablets, and a runtime dither replaces the cropped fade.
  for (const viewport of [
    { width: 3440, height: 1440 },
    { width: 2516, height: 1300 },
    { width: 768, height: 1024 },
    { width: 390, height: 844 },
  ]) {
    await page.setViewportSize(viewport);
    await open(page);
    const descent = plates(page).nth(1);
    const seabed = plates(page).nth(3);
    if (viewport.width > 1920)
      expect(
        Number(await seabed.getAttribute("data-y")),
        `the whole seabed plate at ${viewport.width}`,
      ).toBeGreaterThanOrEqual(0);
    // The canvas row at each edge that meets the abyss is all deep water:
    // the art never stops at a line.
    for (const [layer, edge] of [
      [descent, "bottom"],
      [seabed, "top"],
    ] as const) {
      const art = await layer.evaluate((element: HTMLCanvasElement, edge) => {
        const row = edge === "top" ? 0 : element.height - 1;
        const data = element
          .getContext("2d")!
          .getImageData(0, row, element.width, 1).data;
        // A Set would not survive the trip out of the page.
        return [
          ...new Set(
            Array.from({ length: element.width }, (_, x) =>
              [data[x * 4], data[x * 4 + 1], data[x * 4 + 2]].join(),
            ),
          ),
        ];
      }, edge);
      expect(art, `${edge} edge at ${viewport.width}`).toEqual(["1,45,83"]);
    }
  }
});

test("the product story fits every width and its pixel art stays on whole multiples", async ({
  page,
}) => {
  // Phones, the 480 and sm steps, single-column tablets, and the lg halves.
  for (const width of [360, 390, 480, 640, 768, 900, 1024, 1440]) {
    await page.setViewportSize({ width, height: 900 });
    await open(page);
    for (const title of [
      "One ROM. Dozens of adventures.",
      "Go back hours. Even days.",
      "One account. Every device.",
      "See inside the game.",
      "Fast, and exact.",
    ])
      await expect(
        page.getByRole("heading", { level: 2, name: title }),
      ).toBeAttached();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
        `${width}`,
      ),
      `no horizontal overflow at ${width}`,
    ).toBe(true);
    // Each illustration stays inside its own column: spilling into the
    // gutter or over the text does not widen the page, so check the cell.
    const spills = await page
      .locator("#features section")
      .evaluateAll((sections) =>
        sections.flatMap((section) => {
          const cell = section.children[1].getBoundingClientRect();
          return [...section.children[1].querySelectorAll("*")]
            .map((element) => element.getBoundingClientRect())
            .filter(
              (box) =>
                box.width > 0 &&
                (box.left < cell.left - 0.5 || box.right > cell.right + 0.5),
            )
            .map(() => section.querySelector("h2")!.textContent);
        }),
      );
    expect(spills, `illustrations inside their columns at ${width}`).toEqual(
      [],
    );
    // Every illustration image is drawn at a whole multiple of its art size,
    // so no art pixel is resampled.
    const images = await page.locator("#features img").evaluateAll((elements) =>
      elements.map((element) => {
        const image = element as HTMLImageElement;
        // The content box, which excludes any border drawn around the art.
        const style = getComputedStyle(image);
        const inset = (a: string, b: string) =>
          parseFloat(style.getPropertyValue(a)) +
          parseFloat(style.getPropertyValue(b));
        const box = image.getBoundingClientRect();
        const width =
          box.width -
          inset("border-left-width", "border-right-width") -
          inset("padding-left", "padding-right");
        const height =
          box.height -
          inset("border-top-width", "border-bottom-width") -
          inset("padding-top", "padding-bottom");
        return {
          // Small sprites are inlined as data URLs; name them by size.
          src: `${image.naturalWidth}×${image.naturalHeight}`,
          x: width / image.naturalWidth,
          y: height / image.naturalHeight,
          // Cover draws the art at the larger of the two box ratios on both
          // axes and crops the rest; fill stretches each axis to its box.
          cover: style.objectFit === "cover",
        };
      }),
    );
    expect(images.length).toBeGreaterThan(10);
    for (const image of images) {
      if (image.cover) {
        const scale = Math.max(image.x, image.y);
        expect(Number.isInteger(scale), `${image.src} ${scale}`).toBe(true);
        continue;
      }
      expect(Number.isInteger(image.x), `${image.src} ${image.x}`).toBe(true);
      expect(image.y, image.src).toBe(image.x);
    }
  }
});

test("Features dives smoothly down to the product story", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await open(page);
  await page
    .getByRole("navigation", { name: "Main navigation" })
    .getByRole("link", { name: "Features" })
    .click();
  // Smooth scrolling passes through the descent: sample the scroll position
  // while it moves, then check where it settles.
  const samples: number[] = [];
  for (let i = 0; i < 20; i++) {
    samples.push(await page.evaluate(() => scrollY));
    await page.waitForTimeout(25);
  }
  const target = await page
    .locator("#features")
    .evaluate((element) => element.getBoundingClientRect().top + scrollY);
  await expect.poll(() => page.evaluate(() => scrollY)).toBeCloseTo(target, 0);
  // It moved through intermediate positions rather than jumping.
  const between = samples.filter((y) => y > 0 && y < target - 1);
  expect(between.length).toBeGreaterThan(0);
  // The URL keeps its path; the product story takes focus.
  await expect.poll(() => new URL(page.url()).pathname).toBe("/");
  await expect(page.locator("#features")).toBeFocused();
});

test("with reduced motion, Features jumps straight to the product story", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.setViewportSize({ width: 1440, height: 900 });
  await open(page);
  await page
    .getByRole("navigation", { name: "Main navigation" })
    .getByRole("link", { name: "Features" })
    .click();
  const target = await page
    .locator("#features")
    .evaluate((element) => element.getBoundingClientRect().top + scrollY);
  expect(await page.evaluate(() => scrollY)).toBeCloseTo(target, 0);
});

test("Home's actions open the Library and the player", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await open(page);
  await page
    .getByRole("navigation", { name: "Main navigation" })
    .getByRole("button", { name: "Library" })
    .click();
  await expect(page).toHaveURL(/\/library$/);
  await page.goBack();
  await expect.poll(() => new URL(page.url()).pathname).toBe("/");
  await page.getByRole("button", { name: "Open player" }).click();
  await expect(page).toHaveURL(/\/player$/);
});
