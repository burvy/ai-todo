# Theme Assets

Rambo's poses live in `web/assets/rambo/` (see `art/GUIDE.md`), not here.

Concept art pending review (PR `ping/paper-concept`), drawn in-repo. The
contract for both files:

- **Tileable** at 512x512: every `feTurbulence` carries `stitchTiles="stitch"`,
  so the tile repeats with no seam. Use as a repeating `background-image`.
- **Subtle on purpose**: four stacked layers (dark pulp clouds, light clouds,
  fine dark grain, fine light grain) at single-digit alpha. No stains, no
  aged props.
- **Colour = the app's token, not the file's**: the base rect fills with
  `var(--parchment, <fallback>)`. Inlined in a page that defines the token it
  follows the theme; loaded as `url()` it shows the fallback. There is
  deliberately **no `prefers-color-scheme`** in these files — the app picks
  `paper.svg` for light and `paper-dark.svg` for dark.

| File | Purpose | Size | Format | Used in |
|---|---|---|---|---|
| paper.svg | Tileable light parchment grain (fallback `#f2e8cf`, today's token) | 512x512 | SVG (tileable) | Page background |
| paper-dark.svg | Tileable dark paper grain. Fallback `#48443a` is the **proposed** bright dark paper (UI-BUGS #10: bright, not brown; old `#241c11`) — pending review, the token change comes after | 512x512 | SVG (tileable) | Dark page background |

Preview both on mock journal pages: `art/previews/paper-concept.html`
(open from disk; not served).

## Replacement steps

1. Draw or source the replacement graphic. Keep the same dimensions and transparent background where specified.
2. Keep the exact filename (Dwight's theme.css references these names).
3. Keep the same viewBox if it is an SVG.
4. Replace the file directly; no other changes needed.
5. Rebuild: `.\build.ps1 -Deploy -Modules @() -Servers rambl` in burvy-dev.
6. Confirm the file appears in `web/dist/assets/theme/` after `trunk build --release`.
