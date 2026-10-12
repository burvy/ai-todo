# Rambl art: inventory and redo guide

> **Every asset in this folder is a placeholder made by AI or by code.** None of it was drawn, photographed or recorded by hand. Each one is meant to be redone by a person and dropped in, following the steps below.

This folder is the source of art for the v2 front end (Leptos, see `docs/ARCHITECTURE.md`). The v1 app in `static/` still carries its own inline copies; do not edit `static/` to change art.

## Inventory

| File | What it is | Where used today (v1) | Format and size the app needs | Current origin |
|------|------------|-----------------------|-------------------------------|----------------|
| `icons/favicon.svg` | browser tab icon: the ✎ pen glyph as text | `<link rel="icon">` in `static/index.html` | SVG, viewBox 0 0 64 64, legible at 16×16 | placeholder, code (a Unicode glyph) |
| `icons/gear.svg` | settings gear | gear button by "To do", `index.html` | SVG, viewBox 0 0 24 24, shown at 20×20, `currentColor` fill | placeholder, AI |
| `icons/tick.svg` | done tick (check mark) | task checkbox, `index.html` template | SVG, viewBox 0 0 24 24, one stroke path (drawn in by animation, so it must stay a single open path) | placeholder, AI |
| `icons/delete-x.svg` | red-pencil cross | task delete button, `index.html` template | SVG, viewBox 0 0 24 24, strokes only | placeholder, AI |
| `icons/paperclip.svg` | paperclip | "Import email / PDF" button | SVG, viewBox 0 0 24 24, shown at 16×16, `currentColor` stroke | placeholder, AI |
| `icons/calendar-mask.svg` | calendar outline, used as a mask (a stencil filled with the text colour) | "Add to my calendar" link, `styles.css` ~146 | SVG, viewBox 0 0 24 24, shown at 14×14; only its shape counts, colour is ignored | placeholder, AI |
| `illustrations/pen.svg` | the pen mascot (barrel, cap, nib, eyes, brows) | send/talk button, `index.html` ~79 | SVG, viewBox 0 0 64 112. **Keep the class names** (`pen-barrel`, `pen-cap`, `pen-nib`, `pen-lead`, `pen-gloss`, `eye-white`, `eye-black`, `brow`): the app animates blinking, looking and brows by class | placeholder, AI |
| `illustrations/pen-underline.svg` | wobbly blue pen underline under a new task title | `styles.css` ~404 | SVG, viewBox 0 0 200 8, stretched to the title width (`preserveAspectRatio="none"`) | placeholder, AI |
| `illustrations/overdue-zigzag.svg` | red zigzag mark beside an overdue task | `styles.css` ~493 | SVG, viewBox 0 0 10 18 | placeholder, AI |
| `illustrations/paper-grain.svg` | paper grain texture (generated noise) | background of the list and planner sheets, `styles.css` ~912 | SVG 180×180, tiled. A real scan replaces it as `paper.jpg` (see Paper below) | placeholder, code (SVG noise filter) |
| `brand/rambl.png` | brand card: ruled page with "Rambl" and three sample tasks | burvy.dev project card (burvy-dev repo, `assets/images/rambl.png`) | PNG 2000×1000 | placeholder, code (`brand/rambl_card.py`, Pillow) |
| `brand/rambl_card.py` | script that drew `rambl.png` | run by hand: `python rambl_card.py out.png` (needs Windows fonts Segoe Script and Ink Free) | — | AI-written code |
| `sounds/` (`write`, `tick`, `page`, `listen`, `send`) | UI sounds | `play()` in `static/app.js` | mp3, mono, under 1.5 s, under 100 KB. **No files exist yet**: synth beeps play instead (`sounds/SPEC.md`) | placeholder, code (WebAudio synth) |
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

- [ ] **Same file name** and path as the placeholder it replaces.
- [ ] **Same viewBox** for SVG (the inventory lists them), or the **same pixel size** for PNG/JPG.
- [ ] SVG saved as plain SVG. It opens in a browser, contains no embedded bitmaps, and uses `currentColor` where the inventory says so.
- [ ] The pen mascot keeps its class names (`eye-white`, `eye-black`, `brow` and the rest) so blinking and looking still work.
- [ ] Sounds are under 1.5 s and under 100 KB.
- [ ] Viewed in the app in **light and dark** mode (settings gear › desk), on a phone width as well as a desktop.
- [ ] Update this inventory: change "placeholder" to "hand-made by <name>, <date>" for that row.
