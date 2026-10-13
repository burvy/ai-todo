# `web/assets/` — what each file is and how to replace it

Everything here is **placeholder** (AI-made or stock, see `CREDITS.md`),
meant to be swapped for hand-made work later. The replacement rule is always:
**same file name, same viewBox / duration, drop it in**. Nothing else in the
app needs to change.

## Rambo, the pen mascot — `rambo/`

SVG, viewBox `0 0 64 112` (except the nest), palette via CSS variables inside
each file (`--rambo-*`), with a `prefers-color-scheme: dark` variant. Colours
follow the paper theme: body and tip coffee `#5D4732`, cap golden brown
`#9F682E`, eyes paper-white, blush terracotta, motion lines chamoisee.

| File | Pose | Used for |
|---|---|---|
| `idle.svg` | resting, watching | default state in the nest |
| `listening.svg` | alert, held | picked up / speech capture |
| `writing.svg` | leaning to the nib | letter-by-letter writing animation |
| `flying.svg` | dashing | moving to a row, recovering, skittering home |
| `happy.svg` | cheer | task done, list empty |
| `sleepy.svg` | dozing | idle for a long time |
| `teasing.svg` | playful | being dragged / poked |
| `worried.svg` | concerned | overdue task, dropped outside the nest |
| `lobotomized.svg` | blank | no-brain mode |
| `nest.svg` | his nest (viewBox `0 0 96 56`) | the bubble he rests in, end of the capture line; tapping it submits |
| `sheet.html` | preview of all poses on light and dark paper | open in a browser to check art |

**Replace:** redraw at the same viewBox, keep the class names
(`pen-barrel`, `pen-cap`, `pen-nib`, `eye-white`, `eye-black`, …) — the rig
animates by class. Scan-and-vectorize steps: `art/GUIDE.md`.

## The pen used by the running app — root of `assets/`

| File | What | Where used |
|---|---|---|
| `pen.svg` | single-pose Rambo (same classes as above) | send/talk button, mascot rig (`web/src/lib.rs`, `web/src/mascot.rs`) |
| `pen-underline.svg` | wobbly golden-brown underline, viewBox `0 0 200 8`, stretched (`preserveAspectRatio="none"`) | under a freshly written task title |
| `tick.svg` | check mark, single open path, coffee ink | task checkbox (drawn in by stroke animation — keep it one open path) |
| `delete-x.svg`, `gear.svg`, `paperclip.svg`, `calendar-mask.svg`, `favicon.svg`, `overdue-zigzag.svg`, `paper-grain.svg` | icons | see `art/GUIDE.md` inventory |

## Fonts — `fonts/`

Self-hosted OFL handwriting fonts (no CDN), per `docs/DESIGN-DECISIONS.md`:
Patrick Hand (neat UI hand), Caveat variable (expressive titles), Kalam in
three weights (ballpoint / fountain / marker tool writing). Paste-ready
`@font-face` rules and replacement steps: `fonts/README.md`.

## Theme — `theme/`

| File | What | Notes |
|---|---|---|
| `paper.svg` | light parchment grain tile, 512×512, seamless (`stitchTiles`) | use as repeating `background-image`; faint on purpose |
| `paper-dark.svg` | dark parchment grain tile | same, for night mode |
| `icon-settings.svg` | line gear, viewBox `0 0 24 24`, `currentColor` | top-right settings button; crisp at 20–24 px |
| `icon-list.svg`, `icon-calendar.svg`, `icon-mic.svg` | line icons, `currentColor` | view switcher |
| `stain.svg`, `edge.svg` | cup stain / torn edge | currently unused (the brief calls them dirt — keep them out) |

## Sounds — `sounds/`

Mono 44.1 kHz MP3, 0.12–0.98 s, 2–13 KB each, peak-normalized quiet (−10 to
−14 dBFS). Sources and licences: `CREDITS.md`. Per
`docs/DESIGN-DECISIONS.md` sounds are **on by default**, gentle; the mute
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
