# Rambo art: inventory and redo guide

> **Every asset in this folder is a placeholder made by AI or by code.** None of it was drawn, photographed or recorded by hand. Each one is meant to be redone by a person and dropped in, following the steps below.

This folder is the source of art for the v2 front end (Leptos, see `docs/ARCHITECTURE.md`). The v1 app in `static/` still carries its own inline copies; do not edit `static/` to change art.

## Inventory

| File | What it is | Where used today (v2 / Leptos) | Format and size | Current origin |
|------|------------|-----------------------|-------------------------------|----------------|
| `brand/rambl.png` | brand card: ruled page with "Rambl" and three sample tasks | burvy.dev project card (burvy-dev repo, `assets/images/rambl.png`) | PNG 2000×1000 | placeholder, code (`brand/rambl_card.py`, Pillow) |
| `brand/rambl_card.py` | script that drew `rambl.png` | run by hand: `python rambl_card.py out.png` (needs Windows fonts Segoe Script and Ink Free) | — | AI-written code |
| `web/assets/sounds/` (`write`, `tick`, `page`, `listen`, `drop`, `send`, `delete`) | UI sounds | v2 wires them from `web/assets/sounds/`; v1 `play()` in `static/app.js` | mp3, mono, 44.1 kHz, 0.16–0.98 s, 3–13 KB | placeholder: real foley cut from CC0/public-domain recordings (`web/assets/CREDITS.md`), `listen.mp3` synthesized |
| `web/assets/rambo/<skin>/<pose>.svg` | Rambo tool skins: `fountain` (default), `pencil`, `ballpoint`, `quill`, `marker` — same 11 poses, class names and viewBox `0 0 64 112` per skin, so the app swaps by skin id | mascot rig (`web/src/mascot.rs`), tool picker | 5 skins × 11 poses | placeholder, AI (scripted edits of the fountain art) |
| `web/assets/rambo/nest.svg` | Rambo's nest (the bubble he rests in) | end of the capture line; tapping it submits | SVG, viewBox 0 0 96 56 | placeholder, AI |
| `web/assets/rambo/fx/` (`ink-splat`, `sleepy-z`, `sparks`, `scribble-cloud`) | mood fx overlays, `currentColor`, no inner animation | moods, scribble-out on delete | SVG, viewBox 0 0 48 48 | placeholder, AI |
| `web/assets/tackboard/` (`strip-1..3`, `pin`, `pin-pressed`, `tape`, `crumple`, `bin`) | tack board art for Huang (`web/src/tack.rs`) | done-task strips, pins, tape, crumple throw, bin drop target | SVG, boxes listed in `web/assets/README.md` | placeholder, AI |
| `fonts.md` (Caveat, Inter) | the handwriting and interface fonts | everywhere, via `--hand` and `--chrome` | Google Fonts web fonts | stock fonts (OFL licence), not hand-made |

Notes:
- **Drag handle**: the "⋮⋮" grip is plain text in the task template, not artwork. To make it art, draw a 10×18 SVG and add it as `icons/grip.svg`.
- **Microphone**: there is no separate mic icon. The pen mascot is the talk button.
- **Torn edge**: no torn-edge artwork exists in v1. If v2 wants one, draw it as `illustrations/torn-edge.svg` (a wide, short strip, e.g. viewBox 0 0 400 12).

## Redo: icons and illustrations (drawing on paper)

1. **Draw.** Use a pen or fine liner on plain white paper. Draw each icon large, about 5 cm square, and the pen mascot about 6×10 cm. Keep the proportions of the current viewBox: icons are square, the pen is 64:112.
2. **Scan** at **600 dpi**, greyscale, PNG. A phone scanner app is fine if the light is flat and the page lies flat.
3. **Clean up** in any image editor. Crop to the drawing, raise the contrast until the background is pure white, and erase specks.
4. **Vectorize** in Inkscape (free): File › Import the PNG, select it, then Path › Trace Bitmap. Use "Brightness cutoff" for line art; set "Multiple scans" to colours for the pen mascot. Click Apply and delete the bitmap.
5. **Simplify**: Path › Simplify (Ctrl+L), once or twice, until the curves are smooth but the hand-made wobble survives.
6. **Size it**: Document Properties › set the viewBox to the one in the inventory (e.g. `0 0 24 24`) and scale the drawing to fit it, leaving about 1 unit of margin.
7. **Colour for dark mode**: set icon fills and strokes to `currentColor` so they follow the theme. For the pen mascot, put each part into the class named in the inventory, and leave the colours to CSS.
8. **Export**: File › Save As › **Plain SVG** (not Inkscape SVG). Check that the file has a `viewBox`, no `width`/`height` in mm, and no embedded bitmap.

## Redo: paper

To replace the generated grain with real paper, photograph or scan a blank sheet of good paper at 600 dpi. Crop a square of about 1200×1200 px and make it tileable: in GIMP, Filters › Map › Tile Seamless. Save it as `paper.jpg` under 300 KB. The v1 app uses `static/assets/paper.jpg` automatically when that file exists. For the night desk, `paper-night.jpg` is optional (a darker sheet).

## Redo: brand card (`brand/rambl.png`)

1. Take a real ruled notebook page, with a red margin line if possible.
2. Hand-letter **"Rambl"** large in dark blue ink. Below it, write three short sample tasks with an empty tick circle before each, like the current card.
3. Photograph it in daylight, straight on, or scan it at 600 dpi.
4. Crop to a **2:1** ratio and resize to exactly **2000×1000 px**.
5. Export a PNG, aiming under 1 MB, and replace `brand/rambl.png`. Then copy it to the burvy-dev repo at `assets/images/rambl.png`.
6. Once the card is hand-made, `rambl_card.py` is no longer needed. Keep it only as a record.

## Redo: sounds

1. **Gear**: a phone or a USB microphone, in a quiet room (soft furnishings, no fan, no fridge hum).
2. **Record real foley** (everyday sounds made for the recording). Each file has a matching action in `sounds/SPEC.md`: a pen scratching on paper, a pencil tick, a page turn, a pen cap click, and a soft paper swish. Record several takes of each.
3. **Edit** in Audacity (free). Pick the best take and trim the silence on both ends. Keep it **under 1.5 s**; under 0.5 s feels best in the app. Add a 10 ms fade-in and fade-out to avoid clicks.
4. **Normalize** to about -3 dB peak (Effect › Normalize). The app plays sounds at 30% volume, so avoid anything harsh.
5. **Export** as mono mp3, 44.1 kHz, 96–128 kbps, **under 100 KB**, named exactly `write.mp3`, `tick.mp3`, `page.mp3`, `listen.mp3` and `send.mp3`.
6. Put them in `art/sounds/`. For v1, also put them in `static/assets/sounds/`; no code change is needed.

## Redo: fonts (optional)

To use burvy's own handwriting in place of Caveat:
1. Print a template from a handwriting-font tool such as Calligraphr (calligraphr.com), and fill in every letter, digit and common punctuation mark with the same pen.
2. Scan it at 600 dpi and upload it. Adjust baselines and sizes in the tool, then export **TTF** or **OTF**.
3. Convert it to **WOFF2** for the web (the tool's export or `fonttools`), add it to `art/fonts/`, and change `--hand` to use it first, keeping Caveat as a fallback.
4. Check that task titles are still readable at small sizes.

## Replace a file: checklist

## Rambo design placeholder (v2 / Leptos)

The pen mascot (`illustrations/pen.svg`) is Rambo. It is the writing pen, not just an icon. In v1 (`static/app.js`) it sits at the caret, writes as you type (linger ~2 s, then goes home), flies up to write new task titles (WRITE_MS = 600 ms per title, pen-underline under each new title, reduced-motion respected), and blinks/fidgets between actions. For v2, the brain (`crates/rambl-brain`) decides moods (idle, working, cheerful, sleepy, teasing); the pen (`web/src/mascot.rs`, `brain.rs`) is its body. Until the full animation is ported:
- Replace the pen drawing as a whole by drawing a 64×112 pen on paper, scanning at 600 dpi, vectorizing (Trace Bitmap, Brightness cutoff), simplifying, and exporting Plain SVG with the six class names preserved.
- Keep the class names (`pen-barrel`, `pen-cap`, `pen-nib`, `pen-lead`, `pen-gloss`, `eye-white`, `eye-black`, `brow`) so blinking, looking, and brows continue to work via CSS.
- Keep the viewBox `0 0 64 112`.
- All files in this folder remain placeholders; update the inventory row from "placeholder" to "hand-made by <name>, <date>" when replaced.

- [ ] **Same file name** and path as the placeholder it replaces.
- [ ] **Same viewBox** for SVG (the inventory lists them), or the **same pixel size** for PNG/JPG.
- [ ] SVG saved as plain SVG. It opens in a browser, contains no embedded bitmaps, and uses `currentColor` where the inventory says so.
- [ ] The pen mascot keeps its class names (`eye-white`, `eye-black`, `brow` and the rest) so blinking and looking still work.
- [ ] Sounds are under 1.5 s and under 100 KB.
- [ ] Viewed in the app in **light and dark** mode (settings gear › desk), on a phone width as well as a desktop.
- [ ] Update this inventory: change "placeholder" to "hand-made by <name>, <date>" for that row.
- [ ] **Same viewBox** for SVG (the inventory lists them), or the **same pixel size** for PNG/JPG.
- [ ] SVG saved as plain SVG. It opens in a browser, contains no embedded bitmaps, and uses `currentColor` where the inventory says so.
- [ ] The pen mascot keeps its class names (`eye-white`, `eye-black`, `brow` and the rest) so blinking and looking still work.
- [ ] Sounds are under 1.5 s and under 100 KB.
- [ ] Viewed in the app in **light and dark** mode (settings gear › desk), on a phone width as well as a desktop.
- [ ] Update this inventory: change "placeholder" to "hand-made by <name>, <date>" for that row.

## Rambo character sheet (placeholder poses)

Rambo is a friendly, boy-ish fountain pen who wears his cap like a ballcap. The nine poses are in `web/assets/rambo/`; `web/assets/rambo/sheet.html` shows them on light and dark paper at 96 px and 48 px. Every file starts with `<!-- PLACEHOLDER: replace me, see art/GUIDE.md -->`, uses viewBox `0 0 64 112`, and keeps the class names `pen-barrel`, `pen-cap`, `pen-nib`, `pen-lead`, `pen-gloss`, `eye-white`, `eye-black`, `brow` (extra parts: `mouth`, `mouth-fill`, `tongue`, `lid`, `blush`, `ink-drop`, `fx`, `fx-fill`). Colours are CSS variables with fallbacks (`--rambo-barrel`, `--rambo-cap`, `--rambo-nib`, `--rambo-lead`, `--rambo-ink`, `--rambo-eye`, `--rambo-pupil`, `--rambo-brow`, `--rambo-blush`, `--rambo-fx`), so the theme can restyle them and the built-in dark-mode block works.

| Pose (file) | When shown | Brain pose / state (`crates/rambl-brain`) |
|---|---|---|
| `idle.svg` | Resting at home | `home`, `State::Home` |
| `writing.svg` | You are typing, or he is working out your ramble | `State::Listening`, `State::Thinking` (`think`) |
| `flying.svg` | Moving to a task or the window | `State::Wandering` (`wander`), `Spot` moves |
| `happy.svg` | A task is added or done, the list is empty | `cheer`, `smile` (`task_added`, `task_done`, `list_empty`) |
| `sleepy.svg` | Nobody around for a while, or late at night | `sleep`, `State::Dozing` |
| `teasing.svg` | Playing, poked, dragged | `play`, `dangle` (`State::Playing`, `dragged`) |
| `worried.svg` | A task is overdue | `worried` (`task_overdue`) |
| `lobotomized.svg` | Lobotomized mode: no brain, vacant | `lobotomized`, `Mode::Lobotomized` |
| `listening.svg` | Picked up: he listens for your voice. There is no mic button — picking Rambo up is how you talk to him | new rule; Phyllis wires it |

Replace a pose with your own drawing:

1. Draw him about 6 x 10 cm on plain white paper, one drawing per pose, with the cap, brows and eyes clear of the barrel lines.
2. Scan at 600 dpi greyscale, crop, and clean up in an image editor.
3. In Inkscape: Path > Trace Bitmap (Brightness cutoff for line art, Multiple scans for colour), then Path > Simplify.
4. Set the document to viewBox `0 0 64 112`, put each part into its class, and fill with the `var(--rambo-...)` colours instead of fixed hex.
5. Save as Plain SVG with the same file name, and keep the placeholder comment until the art is final, then delete it.
6. Open `sheet.html` and check every pose in light and dark mode at 48 px; then change the inventory row to "hand-made by <name>, <date>".

