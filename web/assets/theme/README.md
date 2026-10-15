# Theme assets

Rambo's poses live in `web/assets/rambo/` (see `art/GUIDE.md`), not here.

| File | Purpose | Size | Format | Used in |
|---|---|---|---|---|
| `paper-grain.webp` | Real paper grain, tileable, laid over the page colour in light and dark | 320x320, 69 KB | WebP with alpha | `--texture-paper` in `web/theme.css`, every paper surface |

## Source and licence

Cut from *Paper006* (beige paper) by ambientCG, **CC0 1.0** (public domain dedication):
<https://ambientcg.com/a/Paper006>, `Paper006_1K-JPG.zip`, colour map. Processing: greyscale, the
low-frequency shading removed with a wrap-around blur (so the tile stays seamless), the darker
fibres kept as coffee-brown (`#5d4732`) speckle with alpha up to 18%, resized to 320x320,
WebP quality 70. Details are also in `web/assets/CREDITS.md`.

## Replacing it

Keep the filename and a seamless tile; a transparent-background grain, so it works on both
parchment colours. Rebuild with `trunk build --release` and check the file lands in
`web/dist/assets/theme/`.
