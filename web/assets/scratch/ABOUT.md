# The scratch paper: `scratch/`

The piece of ripped scratch paper the capture line is written on, on a desktop (a wide
screen: `min-width: 701px`). The line is pinned to the top of the screen while the page
scrolls, and Rambo writes on this scrap: his nib stands at the end of the words you type.
On a phone the capture bar is the screen's bottom edge and this art is not used.

| File | Used for |
|---|---|
| `paper.svg` | the light theme |
| `paper-dark.svg` | the dark theme (`data-theme="dark"`) |

The art is the `.capture input`'s background, stretched to the line
(`background-size: 100% 100%`, `--scratch-art` in `web/theme.css`), so:

- SVG, **viewBox `0 0 800 84`**, `preserveAspectRatio="none"`. The line is about 950 px wide
  and `--scrap-h` (64 px) tall, so the art is stretched about 1.2 across and 0.8 down: keep
  its detail soft enough to take that.
- Torn on all four sides, with a little room around the paper for its own shadow (the page
  also adds a soft `drop-shadow`). The words sit inside `--scrap-pad-x` (28 px) of the left
  and right ends, and the writing line is drawn about 26 units above the bottom edge.
- Transparent outside the paper.
- Colours as `--scratch-*` CSS variables inside the file, one light and one dark file, like the
  nest. Paper a shade lighter than the page so it reads as a separate sheet laid on it.
- Replace: same file names, same viewBox, drop them in.

Placeholder: generated in this repository (a seeded script of ragged edges), no external
source.
