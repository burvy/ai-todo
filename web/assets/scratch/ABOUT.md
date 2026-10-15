# The scratch paper: `scratch/`

The piece of ripped scratch paper the capture line is written on, on a desktop (a wide
screen: `min-width: 701px`). The line is pinned to the top of the screen while the page
scrolls, and Rambo writes on this scrap: his nib stands at the end of the words you type.
On a phone the capture bar is the screen's bottom edge and this art is not used.

| File | Used for |
|---|---|
| `paper.svg` | the light theme |
| `paper-dark.svg` | the dark theme (`data-theme="dark"`): the same scrap tinted for its older, darker parchment page |

The art is the `.capture input`'s `border-image` (`--scratch-art` in `web/theme.css`): the
outer `--scrap-cap` units (40 of 800) at each end are the torn ends, drawn at the box's side
borders (`--scrap-pad-x`, 28 px); the middle stretches between them. So:

- SVG with `width="800" height="84"` (the border image's natural size, which its slice is
  measured in), **viewBox `0 0 800 84`**, `preserveAspectRatio="none"`. The line is about 950
  px wide and `--scrap-h` (64 px) tall: keep the middle's detail soft enough to stretch, and
  the first and last 40 units for the torn ends.
- Torn on all four sides, with a little room around the paper for its own shadow (the page
  also adds a soft `drop-shadow`). The words start 28 px inside the ends, and the writing
  line is drawn at **y 51**, where the words' baseline falls in a 64 px box (51 x 64 / 84 = 39
  px): Rambo's nib rests on it.
- Transparent outside the paper.
- Colours as `--scratch-*` CSS variables inside the file, one light and one dark file, like the
  nest. Paper a shade lighter than the page so it reads as a separate sheet laid on it.
- Replace: same file names, same viewBox, drop them in.

Placeholder: generated in this repository (a seeded script of ragged edges), no external
source.
