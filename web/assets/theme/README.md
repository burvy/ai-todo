# Theme Assets (Placeholder)

Rambo's poses live in `web/assets/rambo/` (see `art/GUIDE.md`), not here.

All files in this directory are AI-generated placeholder graphics. Replace them with real art before shipping.

| File | Purpose | Size | Format | Used in |
|---|---|---|---|---|
| paper.svg | Tileable light parchment grain | 512x512 | SVG (tileable) | Page background |
| paper-dark.svg | Tileable dark-mode parchment grain | 512x512 | SVG (tileable) | Dark page background |
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
