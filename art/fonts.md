# Fonts

**Status: placeholder (stock OFL fonts, self-hosted).** Per
`docs/DESIGN-DECISIONS.md` every word is handwritten: about three self-hosted
OFL handwriting fonts live in `web/assets/fonts/` — no CDN, no interface font.
`@font-face` rules to paste into the theme: `web/assets/fonts/README.md`.

| Font | File(s) | Tier / use | Licence |
|------|---------|------------|---------|
| Patrick Hand | `patrick-hand.woff2` (400) | neat, very legible hand — UI text: times, dates, settings, day names | SIL Open Font License 1.1 |
| Caveat (variable) | `caveat-var.woff2` (400–700) | expressive hand — task titles | SIL Open Font License 1.1 |
| Kalam | `kalam-light/regular/bold.woff2` (300/400/700) | Rambo's tool writing: light = ballpoint, regular = fountain pen / quill, bold = marker | SIL Open Font License 1.1 |

Sources: Google Fonts repository (`github.com/google/fonts`, `ofl/…`); full
licence texts sit next to the files (`OFL-*.txt`). Recorded in
`web/assets/CREDITS.md`.

The v1 prototype (`static/index.html`) loaded Caveat + Inter from the Google
Fonts CDN — v2 does not; v2 self-hosts. The SIL Open Font License allows free
use, embedding and self-hosting. You may not sell the font files on their own;
a modified version may not reuse the reserved font name.

To replace a stock font with burvy's own handwriting, see "Fonts" in
`GUIDE.md` — same idea as the art: scan, clean, export, keep the file name.
