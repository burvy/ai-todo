# Self-hosted handwriting fonts (OFL)

Per `docs/DESIGN-DIRECTION.md`: every word handwritten, ~3 self-hosted OFL
fonts, no CDN. Two tiers:

| Tier | Font | File | Notes |
|---|---|---|---|
| Neat, very legible — UI text (times, dates, settings, day names) | **Patrick Hand** | `patrick-hand.woff2` | single weight 400; check legibility at 390 px |
| Expressive hand — task titles | **Caveat** (variable) | `caveat-var.woff2` | one file covers weight 400–700 (`font-weight: 400 700`) |
| Tool writing — Rambo's 5 tools | **Kalam** | `kalam-light.woff2` / `kalam-regular.woff2` / `kalam-bold.woff2` | light = ballpoint (thin), regular = fountain pen / quill, bold = marker |

Suggested tool styling (art side of the decision): pencil = Kalam regular in
grey with grain, ballpoint = Kalam light blue-black, quill = Kalam regular or
Caveat in brown with the odd blot, marker = Kalam bold and thick.

Licences: each family's full text sits next to the fonts (`OFL-*.txt`).

## Paste-ready `@font-face`

```css
@font-face {
  font-family: "Patrick Hand";
  src: url("assets/fonts/patrick-hand.woff2") format("woff2");
  font-weight: 400;
  font-display: swap;
}
@font-face {
  font-family: "Caveat";
  src: url("assets/fonts/caveat-var.woff2") format("woff2");
  font-weight: 400 700; /* variable */
  font-display: swap;
}
@font-face {
  font-family: "Kalam";
  src: url("assets/fonts/kalam-light.woff2") format("woff2");
  font-weight: 300;
  font-display: swap;
}
@font-face {
  font-family: "Kalam";
  src: url("assets/fonts/kalam-regular.woff2") format("woff2");
  font-weight: 400;
  font-display: swap;
}
@font-face {
  font-family: "Kalam";
  src: url("assets/fonts/kalam-bold.woff2") format("woff2");
  font-weight: 700;
  font-display: swap;
}
```

(Adjust the `url()` prefix to how `theme.css` references `assets/`.)

## Replacing a font

Keep the file name, re-export the new font as WOFF2 (e.g.
`pip install fonttools brotli`, then a two-line Python script with
`fontTools.ttLib.TTFont(src); f.flavor = "woff2"; f.save(dst)`), and update
the row in `web/assets/CREDITS.md`.
