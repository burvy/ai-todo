# Rambl theme: parchment

The spec for how Rambl looks. Code lives in `web/theme.css` (linked from `web/index.html` with `<link data-trunk rel="css" href="theme.css">`, so trunk copies it into `web/dist`, which the server embeds). Components must use the tokens below rather than raw colours.

## Palette

**Light** (aged parchment and iron-gall ink):

| Token | Hex | Use |
|---|---|---|
| `--paper` | `#f2e6cd` | page background |
| `--paper-raised` | `#fdf7ea` | cards, panels, inputs (the sheet, clearly lighter than the page) |
| `--paper-sunken` | `#eadcc0` | the mascot's circle (deeper paper) |
| `--ink` | `#2b2318` | body text |
| `--ink-soft` | `#5f5138` | secondary text |
| `--ink-faint` | `#6f6248` | muted text, hour labels, version (5.3:1 on `--paper-raised`, so 12px passes WCAG AA) |
| `--rule` | `#c9b58c` | borders and hairlines |
| `--accent` | `#8a4b1f` | walnut ink: focus, active chips, placement bar |
| `--danger` | `#a5342a` | red ink: errors, Overdue, Delete |
| `--now` | `#b03a2a` | the current-time line |
| `--wash` | `rgba(138, 75, 31, 0.12)` | tint behind active chips and placements |
| `--backdrop` | `rgba(43, 35, 24, 0.35)` | dialog backdrop |
| `--glow` | white highlight top, walnut bloom bottom | the lamp over the page (`body` background, top layer) |

**Dark** (the same paper, aged and lamp-lit), under `@media (prefers-color-scheme: dark)`:

| Token | Hex |
|---|---|
| `--paper` | `#191410` |
| `--paper-raised` | `#2b241b` |
| `--paper-sunken` | `#110e0a` |
| `--ink` | `#f2e6cb` |
| `--ink-soft` | `#cdbe9f` |
| `--ink-faint` | `#9c8c6c` |
| `--rule` | `#574733` |
| `--accent` | `#e2ab61` |
| `--danger` | `#e88b7c` |
| `--now` | `#e4705a` |
| `--wash` | `rgba(226, 171, 97, 0.16)` |
| `--glow` | amber highlight top, black bloom bottom |
| `--backdrop` | `rgba(10, 8, 5, 0.6)` |

The dark steps are deliberately wide: page, sheet and sunken circle are told apart at a
glance, so nothing reads as brown-on-brown.

`color-scheme` is `light` or `dark` to match, so checkboxes, scrollbars and the dialog follow the paper. Contrast of `--ink` on `--paper` is about 13:1 in light and 15:1 in dark; both clear the 7:1 bar for body text. Every text token clears 4.5:1 for small text: `--ink-faint` is the weakest at 5.3:1 on `--paper-raised` in light and 4.7:1 in dark (it was 3.7:1 before review).

## Type

System stacks only, no downloads.

- Headings and speech (`--font-heading`): `Georgia, "Iowan Old Style", "Times New Roman", serif`. This is the hand-written feel — titles, dialog titles and Rambo's lines read like ink on paper.
- Body (`--font-body`): `system-ui, -apple-system, "Segoe UI", Roboto, Helvetica, Arial, sans-serif`. Inputs, buttons and list text stay plainly readable.

Scale (rem, base 16px): `--text-xs` 0.75, `--text-sm` 0.85, `--text-base` 1, `--text-lg` 1.15, `--text-xl` 1.4, `--text-2xl` 1.75. Line height 1.5 for body, 1.2 for headings. Overdue tags are `--text-xs`, uppercase, 0.06em tracking.

- **Handwriting** (`--font-hand`): task titles only — the title in the list (`.task-open > strong`) and the title at the top of the edit dialog (`.dialog-task-title`). A system handwriting stack (`Segoe Print`, `Bradley Hand`, `Comic Sans MS`, `Chalkboard SE`, `Marker Felt`, `cursive`), never a download; a reader without those fonts gets `cursive`, then the browser's default. Everything else stays in the body or heading stack.

## Spacing, borders, radii

- Gutter: `--gutter` 1rem (16px) on every side; page max width 1100px. The page must not scroll sideways at 360px.
- Steps: `--space-1` 0.25rem, `--space-2` 0.5rem, `--space-3` 0.75rem, `--space-4` 1rem. Grid gaps are 1rem.
- Week rows: `--hour-h` 64px, the same number `web/src/week.rs` uses to place the now-line — change both together.
- Borders: 1px `--rule` everywhere; 2px `--now` for the current-time line; the active chip uses a 2px `--accent` outline; the accent bar on a scheduled placement is 3px `--accent`.
- Radii: `--radius-sm` 0.25rem (inputs, chips), `--radius-md` 0.5rem (cards, dialog, ask bubble), `--radius-round` for Rambo's circles.
- Task rows are separated by a dashed `--rule` line, like entries in a ledger.

## Paper texture

Four graphics, shared with the mascot art, live in `web/assets/theme/` and are copied into `web/dist` by `index.html`'s `copy-dir` link:

| File | Size | Where it is used |
|---|---|---|
| `paper.svg` | 512x512, tileable | page background, light mode (`--texture-paper`) |
| `paper-dark.svg` | 512x512, tileable | page background, dark mode (`--texture-paper` under the dark media query) |
| `edge.svg` | 512x24, repeat-x | torn deckle along the bottom of the masthead (`header::before`, `--edge-h` = 24px). The file is an **alpha shape**: only the page colour below the ragged line is filled, and the strip paints it through a CSS `mask`, with a rule-tinted head as the sheet's shadow. The same file therefore reads right on a light or a dark sheet — no colour override is needed. The masthead's top and bottom padding are at least the strip's height, so the tear never crosses the title. |
| `stain.svg` | 256x256, transparent | alpha cup ring in the masthead corner, painted through a `mask` in `color-mix(--accent 55%)` at 50% opacity and clipped by the header, so it is ink rather than a smudge |
| `paper.svg` / `paper-dark.svg` | 512x512 | full-bleed colour + stitched grain, no frame, so the tile repeats without seams (the framed placeholder showed a grid of panels) |

Graphics are drawn by Oscar under the names above (`web/assets/theme/README.md` is the replacement guide).

Every texture sits on top of a `--paper*` colour, so a missing file degrades to flat parchment with no layout change. The files are drawn separately; names are fixed by the asset contract in `web/assets/theme/README.md`.

## Mobile first

Check 360px and 390px before a wide window. The layout behaviour comes from `web/src/lib.rs` and `web/src/week.rs` (PR #135) and is left as it is; the theme owns colours, fonts, spacing, paper and ruled lines.

- **Panes:** the base rules are the phone: `main` is a horizontal scroll-snap row — the To do pane and the Planner pane sit side by side, swiped or tapped via `.pane-switch`. Those declarations (scroll-snap, `grid-auto-columns: 100%`, `position: relative`, hidden scrollbar) are behaviour and are not restyled; the theme only colours `.pane-switch button.active` (walnut accent, `--wash` fill, 2px accent outline) and gives the tab buttons a 44x44 target. The buttons carry `aria-pressed` (toggle state; an earlier `aria-current` never rendered and was dropped); tapping one scrolls the pane row to that pane's own `offset_left` (`show_pane` in `web/src/lib.rs`), not a multiple of the pane width, because the grid gap sits between panes. From `min-width: 701px` the panes become a two-column grid and the switcher hides.
- **Tap targets:** every control is at least `--tap` (2.75rem / 44px) tall or wide — tabs, chips, Mic, Add, Delete, the settings summary and the download link. Checkboxes and radios stay 1.25rem so they do not become grey boxes; their 44px hit area comes from the row (`.task-list li`) or the `<label>` they sit in.
- **Type:** body text never below 1rem (16px), so iOS does not zoom the inputs. Only meta text (`--text-xs`: hour labels, version, the Overdue tag) is smaller.
- **Capture box:** on a phone it is a fixed bar at the bottom edge with the safe-area inset, so Mic, Rambo and Add sit under the thumb; the body reserves matching bottom padding. On desktop it stays in the masthead.
- **No sideways page scroll:** the page itself never scrolls horizontally — only the pane row and the week grid scroll, each inside its own box (`--days` columns, 8rem minimum per day on a phone, 9rem on desktop).
- **Dialog:** fixed and centred (behaviour from #135), `box-sizing: border-box`, `max-width: calc(100vw - 2rem)`, `width: min(32rem, 100vw - 2 * gutter)`, `max-height: calc(100dvh - 2 * gutter)`, scrolls inside itself.
- **No scroll chaining:** the week grid and the pane row both use `overscroll-behavior: contain` (`overscroll-behavior-x` on `.week-scroll`), so a flick there never scrolls the page.

## Light and dark

Light and dark follow the OS only (`prefers-color-scheme`); there is no in-app toggle. Both variants are always shipped, and both must read as paper: warm and pale in light, deep and lamp-lit in dark. Nothing may rely on a texture alone for contrast.

## Motion

- Durations: `--dur-fast` 120ms (colour changes on buttons and inputs), `--dur` 200ms (press of the talk button, general transitions), `--dur-slow` 450ms (Rambo's fall and skitter home). Easing `--ease` (`ease`), except Rambo's keyframes which keep their in/out easing.
- Animate transform, opacity and colour only — never layout-affecting properties, and nothing that moves text while it is being read.
- Nothing loops except the mic's listening pulse.
- `prefers-reduced-motion: reduce` turns every animation and transition off (`animation: none; transition: none`), including Rambo's fall, which then snaps home — matching section 8.1 of `docs/ARCHITECTURE.md`.

## Rules for changes

1. Colours come from a token; no hex literals outside `:root` and its dark block.
2. System fonts only.
3. Keep every class name in `web/src` working; add a token rather than a special case.
4. If you add a graphic, put it in the contract table above and make the page look right with the file deleted.
