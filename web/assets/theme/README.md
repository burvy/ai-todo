# Theme graphics

Shared contract between the theme (`web/theme.css`, Dwight) and the art (Oscar). Do not rename or resize a file without telling god — the CSS references these exact paths.

| File | Size | Notes |
|---|---|---|
| `paper.svg` | 512x512 | Tileable parchment texture, light. Tiles on the page background. |
| `paper-dark.svg` | 512x512 | Tileable aged/dark paper for dark mode. Same tile grid as `paper.svg`, so a mode switch does not jump. |
| `edge.svg` | 512x24 | Deckled/torn paper edge strip. Repeats along the X axis at the bottom of the masthead. |
| `stain.svg` | 256x256 | Faint ink/coffee stain, transparent background, shown at 45% opacity in the masthead corner. |
| `rambo-idle.svg` | 256x256 | Rambo poses, transparent background. Not referenced by CSS yet: the mascot `<img>` still points at `web/assets/pen.svg`, and wiring a pose per state class is a `web/src` change. |
| `rambo-happy.svg` | 256x256 | " |
| `rambo-sleepy.svg` | 256x256 | " |
| `rambo-teasing.svg` | 256x256 | " |
| `icon-mic.svg` | 24x24 | Uses `currentColor` for strokes so it follows the ink colour. Not referenced by CSS yet (the buttons still show text labels). |
| `icon-settings.svg` | 24x24 | " |
| `icon-list.svg` | 24x24 | " |
| `icon-calendar.svg` | 24x24 | " |

## Replacing a graphic

1. Export plain SVG: no embedded bitmaps, no Inkscape-only metadata, a `viewBox`, no `width`/`height` in mm (see `art/GUIDE.md`).
2. Keep the exact file name and the size above.
3. Overwrite the file here. Trunk copies this directory into `web/dist/assets/` on every `trunk build`, and the server serves only `web/dist`.
4. Check both colour schemes: the page must read correctly with the file deleted (the CSS falls back to flat paper colours), and the textures must not carry text contrast on their own.

The page is complete without these files — they are an accent, never a dependency.
