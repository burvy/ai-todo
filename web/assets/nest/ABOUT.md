# Rambo's nest: `nest/`

The paper nest Rambo rests in, at the end of the capture bar: the bottom-right
corner on a phone, the right end of the capture line on a desktop. Tapping it
submits what you typed. It is drawn in two layers so he sits *in* it rather than
on top of it:

| File | Layer | Drawn |
|---|---|---|
| `back.svg` | The back wall, the inside of the bowl, the strips poking up behind, and the shadow on the ground | Behind Rambo, on the nest button (`.nest::before`) |
| `rim.svg` | The front lip of the bowl | In front of Rambo (`.nest-rim`), over his tip when he is home |

The order is data, in `web/theme.css` `:root`: the back layer belongs to the
capture bar (`--layer-capture`), Rambo is `--layer-rambo`, and the rim is
`--layer-nest-rim`, one above him. Nothing else decides it.

## Size and format

- SVG, both files **viewBox `0 0 170 100`**, the same frame, so the two layers
  always line up. Transparent background.
- Rambo's rest point is the point **(85, 40)** of that frame (`tuning::NEST_HOME`: 50% across,
  40% down). Draw the bowl around it.
- Drawn to fill **the nest's box**: the `.nest` button, `--nest-w` by `--nest-h` in `web/theme.css`
  (104 px wide on a phone, as tall as the capture bar, so it sits wholly on screen in the
  bottom-right corner; 112 x 96 px on a desktop, at the right end of the capture line). The
  art is stretched to the box (`100% 100%`), so it need not be 170 x 100.
- Rambo (64 x 64 px on a phone, 80 x 80 on a desktop) rests at `tuning::NEST_HOME` of the box
  (the middle across, 40% down), sunk `--nest-sink` lower. His home, the back layer and the
  front rim all come from that one box (`mascot::nest_home`); there is no other measurement.
  The rim's top edge must cross his lower quarter so his tip is inside the bowl.
- Each layer turns about the middle of its base (`transform-origin: 50% 100%`), so
  the nest can sag and rock later without redrawing.
- Colours as `--nest-*` CSS variables in the file, with a
  `prefers-color-scheme: dark` variant, like Rambo's poses.

## Mood

A small, cozy paper nest: torn and woven strips of the same paper as the page,
sewn together, a little lopsided. Warm, soft, never a hard dish.

## Source and licence

Placeholder, AI-drawn (see `web/assets/CREDITS.md`).

## How to replace

Draw both layers in the same 170 × 100 frame with the anchor where it is, save
them over `back.svg` and `rim.svg`, and rebuild (`trunk build --release`). No code
changes. To move the anchor or change the size, change the `--nest-*` tokens in
`web/theme.css`.
