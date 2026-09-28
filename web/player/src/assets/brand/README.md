# Evenfall brand mark

`evenfall-icon.svg` is the mark: a pixel-art sunset in the peach and mint brand
colors, drawn on a 32-unit grid. It is the site icon, declared by the player's
`index.html`, and it is the source of truth for the bitmaps beside it:

| File | Surface |
|---|---|
| `evenfall-icon-32.png` | Tabs of browsers that do not render SVG favicons; transparent, so it reads on light and dark tab bars |
| `evenfall-icon-180.png` | iOS home screen; the mark sits on the `#073659` theme color, inside the centre 80% that survives iOS's corner mask |

Regenerate both after editing the SVG, with
[`scripts/render-brand-icons.mjs`](../../../scripts/render-brand-icons.mjs):
the script draws the art at 1:1 and then scales it by whole pixels, so it
keeps its pixel grid and stays crisp.

```sh
npm run icons -w frontend
```

`evenfall-logo.svg` is the mark with the wordmark beside it, used by the
player's header; `evenfall-wordmark.svg` is the wordmark alone.
