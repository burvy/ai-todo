# Rambo: `rambo/`

Rambo the pen, one drawing per pose for each pen he can be (the gear menu's **Pen**), plus
the little pictures that float beside him. **All of it is placeholder art**, drawn in-repo
as SVG so the page has something to show; an artist can replace any file with art in the
same frame and the same name (see `art/GUIDE.md` for the style).

| Folder | Pen | Files |
|---|---|---|
| `pencil/` | Pencil | one per pose, listed below |
| `ballpoint/` | Ballpoint | the same |
| `fountain/` | Fountain pen | the same |
| `marker/` | Marker | the same |
| `quill/` | Quill | the same |
| `fx/` | none | `ink-splat`, `scribble-cloud`, `sleepy-z`, `sparks` |

The poses the page draws, ten per pen (`web/src/mascot.rs:Pose::id`): `idle`, `teasing`,
`listening`, `writing`, `flying`, `dozing`, `happy`, `sulking`, `worried`, `lobotomized`.
Each folder also has a `sleepy.svg` that no pose draws today.
`sheet.html` shows them all side by side, to compare a redraw against.

## Size and format

- SVG, **viewBox `0 0 64 112`**, transparent background, the pen standing upright with
  its nib at the bottom.
- The drawing's root group has **`id="art"`**: the page shows it through `<use href="...#art">`
  (`Pose::art`), not as an `<img>`, so it inherits the page's tokens.
- **No colours of their own.** Every fill and stroke is one of the `--rambo-*` names in
  `web/theme.css` `:root` (`--rambo-barrel`, `--rambo-cap`, `--rambo-nib`, `--rambo-lead`,
  `--rambo-ink`, `--rambo-pupil`, `--rambo-brow`, `--rambo-fx`, `--rambo-eye`,
  `--rambo-blush`), each with a fallback, so he follows the light and dark paper.
- He is drawn 64 x 64 px on a phone and 80 x 80 on a desktop.

## Source and licence

Placeholder, AI-drawn in this repository; no external source, no licence needed (see
`web/assets/CREDITS.md`).
