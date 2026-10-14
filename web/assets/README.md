# `web/assets/` — what each file is and how to replace it

Everything here is **placeholder** (AI-made or stock, see `CREDITS.md`),
meant to be swapped for hand-made work later. The replacement rule is always:
**same file name, same viewBox / duration, drop it in**. Nothing else in the
app needs to change.

## Rambo, the pen mascot — `rambo/`

**Tool skins:** `rambo/<skin>/<pose>.svg` with `<skin>` = `fountain`
(default, the original art), `pencil`, `ballpoint`, `quill`, `marker`.
Every skin has the same 11 poses with the same class names
(`pen-barrel`, `pen-cap`, `pen-nib`, `eye-white`, `eye-black`, `brow`,
`mouth`, …) and the same viewBox `0 0 64 112`, so the app swaps skins by
path/skin id and the animation rig is untouched. Each SVG carries its palette
as `--rambo-*` CSS variables with a `prefers-color-scheme: dark` variant.

| Pose | Used for |
|---|---|
| `idle.svg` | default state in the nest |
| `listening.svg` | picked up / speech capture |
| `writing.svg` | letter-by-letter writing animation |
| `flying.svg` | moving to a row, recovering, skittering home |
| `happy.svg` | task done, list empty |
| `sleepy.svg` | drowsy: idle a while |
| `dozing.svg` | dedicated: asleep, head tilted (night; wakes when you type) |
| `teasing.svg` | being dragged / poked |
| `worried.svg` | overdue task, dropped outside the nest |
| `sulking.svg` | dedicated: sulking at overdue |
| `lobotomized.svg` | no-brain mode |

Shared, skin-independent: `rambo/nest.svg` (viewBox `0 0 96 56`, the bubble
he rests in; tapping it submits) and the mood fx in `rambo/fx/` below.
`rambo/sheet.html` previews every skin × pose on light and dark paper —
open it in a browser to check art.

**Replace:** redraw at the same viewBox, keep the class names. Scan-and-
vectorize steps: `art/GUIDE.md`.

## Mood fx — `rambo/fx/`

Small overlays the rig composes over poses. Convention (agreed with Chuan):
viewBox `0 0 48 48`, artwork centred on (24, 24), stroke-based,
`currentColor`, **no animation inside the SVG** (opacity/transform are
animated from CSS so `prefers-reduced-motion` can switch them off).

| File | Used for |
|---|---|
| `ink-splat.svg` | dropped hard: splat under the nib |
| `sleepy-z.svg` | floating Z's over `sleepy`/`dozing` |
| `sparks.svg` | celebration stars (task done, list empty) |
| `scribble-cloud.svg` | annoyed cloud; inner zigzag doubles as the scribble-out over a deleted row |

## Tack board — `tackboard/`

Per Huang's layout (`web/tack.css`, `web/src/tack.rs`); transparent
backgrounds, paper palette, they render inside fixed boxes and scale.

| File | What | Box |
|---|---|---|
| `strip-1.svg` `strip-2.svg` `strip-3.svg` | torn task strips (plain tear / corner fold / ragged), stretched to 140×32, picked per task by hash | viewBox 200×46 |
| `pin.svg` | push pin, ~16 px head, needle down | 24×24 |
| `pin-pressed.svg` | pin pressed flat (tidy-up beat) | 24×16 |
| `tape.svg` | translucent tape piece, torn ends | 64×24 |
| `crumple.svg` | crumpled paper ball (end state ~20 px) | 40×40 |
| `bin.svg` | small waste basket, drop target | 48×64 |

## The pen used by the running app — root of `assets/`

| File | What | Where used |
|---|---|---|
| `pen-underline.svg` | wobbly golden-brown underline, viewBox `0 0 200 8`, stretched (`preserveAspectRatio="none"`) | under a freshly written task title |

## Fonts — `fonts/`

Self-hosted OFL handwriting fonts (no CDN), per `docs/DESIGN-DIRECTION.md`:
Patrick Hand (neat UI hand), Caveat variable (expressive titles), Kalam in
three weights (ballpoint / fountain / marker tool writing). Paste-ready
`@font-face` rules and replacement steps: `fonts/README.md`.

## Theme — `theme/`

| File | What | Notes |
|---|---|---|
| `paper.svg` | light parchment grain tile, 512×512, seamless (`stitchTiles`) | use as repeating `background-image`; faint on purpose |
| `paper-dark.svg` | dark parchment grain tile | same, for night mode |
| `stain.svg`, `edge.svg` | cup stain / torn edge | currently unused (the brief calls them dirt — keep them out) |

## Sounds — `sounds/`

Mono 44.1 kHz MP3, 0.12–0.98 s, 2–13 KB each, peak-normalized quiet (−10 to
−14 dBFS). Sources and licences: `CREDITS.md`. Per
`docs/DESIGN-DIRECTION.md` sounds are **on by default**, gentle; the mute
toggle lives in the gear menu (wired in the app, not in these files).

| File | Plays when |
|---|---|
| `write.mp3` | the pen writes a task title (letter-by-letter) |
| `tick.mp3` | a task is ticked done |
| `page.mp3` | switching To do / Planner (page flip) |
| `listen.mp3` | Rambo is picked up (cap click) |
| `drop.mp3` | Rambo is put down |
| `send.mp3` | a ramble is sent |
| `scribble.mp3` | a task is deleted (scribbled out) |
| `delete.mp3` | spare: paper-swipe delete, if scribble reads wrong |
| `crumple.mp3` | a done task strip is crumpled and thrown away |
| `pin.mp3` | a strip is tacked to the tack board |

**Replace:** record real foley (what to record: `art/sounds/SPEC.md`),
export mono MP3 under 100 KB and under 1 s, same file name.

## Replacing art in general

Draw on paper, scan at 600 dpi, vectorize, size to the original viewBox,
save as plain SVG — the full walkthrough is `art/GUIDE.md`.
