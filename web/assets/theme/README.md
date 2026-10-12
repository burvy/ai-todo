# Theme Assets (Placeholder)

All files in this directory are AI-generated placeholder graphics. Replace them with real art before shipping.

| File | Purpose | Size | Format | Used in |
|---|---|---|---|---|
| paper.svg | Tileable light parchment background | 512x512 | SVG (tileable) | Theme backgrounds |
| paper-dark.svg | Tileable dark-mode parchment | 512x512 | SVG (tileable) | Dark theme backgrounds |
| edge.svg | Deckled paper edge strip | 512x24 | SVG (repeat-x) | Page borders |
| stain.svg | Faint ink stain overlay | 256x256 | SVG (transparent) | Background texture |
| rambo-idle.svg | Mascot neutral pose | 256x256 | SVG (transparent) | Home screen mascot |
| rambo-happy.svg | Mascot happy pose | 256x256 | SVG (transparent) | Success / positive feedback |
| rambo-sleepy.svg | Mascot sleepy pose | 256x256 | SVG (transparent) | Idle / rest state |
| rambo-teasing.svg | Mascot playful pose | 256x256 | SVG (transparent) | Teasing / playful feedback |
| icon-mic.svg | Microphone icon | 24x24 | SVG (currentColor) | Speech / voice actions |
| icon-settings.svg | Settings gear icon | 24x24 | SVG (currentColor) | Settings / preferences |
| icon-list.svg | List icon | 24x24 | SVG (currentColor) | List / task list views |
| icon-calendar.svg | Calendar icon | 24x24 | SVG (currentColor) | Calendar / scheduling |

## Replacement steps

1. Draw or source the replacement graphic. Keep the same dimensions and transparent background where specified.
2. Keep the exact filename (Dwight's theme.css references these names).
3. Keep the same viewBox if it is an SVG.
4. Replace the file directly; no other changes needed.
5. Rebuild: `.\build.ps1 -Deploy -Modules @() -Servers rambl` in burvy-dev.
6. Confirm the file appears in `web/dist/assets/theme/` after `trunk build --release`.

## Used by

- `paper.svg` / `paper-dark.svg` / `edge.svg` / `stain.svg`: referenced from `web/theme.css` (`--texture-paper`, `--texture-edge`, `--texture-stain`) — page background, masthead edge, masthead stain.
- `rambo-*.svg` and `icon-*.svg`: not referenced yet. The mascot `<img>` still points at `web/assets/pen.svg` and the buttons still show text labels; wiring poses per state class and icon buttons is a `web/src` change.
- The page reads correctly with any of these files deleted: the CSS falls back to the flat paper colours in `docs/THEME.md`.
