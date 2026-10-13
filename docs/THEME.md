# Rambl theme: one sheet of paper

The spec for how Rambl looks. Code lives in `web/theme.css` (linked from `web/index.html` with `<link data-trunk rel="css" href="theme.css">`, so trunk copies it into `web/dist`, which the server embeds). Components must use the tokens below rather than raw colours.

The look is **one page of parchment, written on in ink** (owner brief, `docs/REDESIGN.md`): space and ruled lines divide the page — no boxes, borders or stacked cards. Controls are words on the page until hover or focus underlines them; inputs are written on a rule. Only two things may look like objects: the edit dialog (a loose sheet laid over the page) and Rambo's ask (a speech bubble).

## Palette

**Light** (the owner's parchment palette, `docs/REDESIGN.md`):

| Token | Hex | Use |
|---|---|---|
| `--parchment` (`--paper`) | `#f2e8cf` | the page |
| `--wheat` (`--paper-raised`) | `#ecd7af` | a sheet laid on the page: dialog, capture bar |
| `--ecru` (`--paper-sunken`) | `#e1c38c` | pressed-in paper: Rambo's nest circle |
| `--chamoisee` | `#a7865d` | **rules and lines only, never text** (2.8:1 on parchment) |
| `--ink` | `#5d4732` | coffee: everything written |
| `--ink-soft` | `#75604a` | secondary text (4.9:1 on parchment) |
| `--ink-faint` | `#6e5a43` | muted text, hour labels, version (4.7:1 on wheat, so 12px passes WCAG AA) |
| `--accent` | `#9f682e` | golden brown: focus, the active tab/filter underline, the placement bar — bold text only (3.9:1) |
| `--danger` | `#a5342a` | red pencil: errors, Overdue, Delete |
| `--now` | `#b03a2a` | the current-time line |
| `--rule` | chamoisee 42% | hairlines between rows and hours |
| `--rule-strong` | chamoisee 62% | masthead rule, page fold, input rules |
| `--margin-line` | `rgba(165,52,42,.24)` | the notebook's red margin, on the task list |
| `--wash` | `rgba(159,104,46,.1)` | tint behind a placement |
| `--backdrop` | `rgba(46,35,21,.35)` | dialog backdrop |

`--paper`, `--paper-raised`, `--paper-sunken` are kept as aliases so existing rules read naturally.

**Dark** (the same desk at night: deep paper, parchment-coloured ink), under `@media (prefers-color-scheme: dark) { :root:not([data-theme="light"]) }` and `:root[data-theme="dark"]`:

| Token | Hex |
|---|---|
| `--parchment` | `#241c11` |
| `--wheat` | `#2d2315` |
| `--ecru` | `#1a140b` |
| `--ink` | `#efe3c8` |
| `--ink-soft` | `#c9b693` |
| `--ink-faint` | `#b3a17e` |
| `--accent` | `#d9a45c` |
| `--danger` | `#e0705f` |
| `--now` | `#d96a4f` |
| `--wash` | `rgba(217,164,92,.14)` |
| `--backdrop` | `rgba(10,8,5,.55)` |

`color-scheme` is `light` or `dark` to match, so checkboxes, scrollbars and the dialog follow the paper.

## Type

System stacks only, no downloads.

- The masthead (`h1`, "Rambl") is in `--font-hand`: the owner's signature on the page.
- Headings and speech (`--font-heading`): `Georgia, "Iowan Old Style", "Times New Roman", serif`, small-caps with 0.04em tracking on `h2` — a chapter heading in a notebook.
- Body (`--font-body`): `system-ui, -apple-system, "Segoe UI", Roboto, Helvetica, Arial, sans-serif`.

Scale (rem, base 16px): `--text-xs` 0.75, `--text-sm` 0.85, `--text-base` 1, `--text-lg` 1.15, `--text-xl` 1.4, `--text-2xl` 1.75. Line height 1.5 for body, 1.2 for headings. Overdue tags are `--text-xs`, uppercase, 0.06em tracking.

- **Handwriting** (`--font-hand`): the masthead and task titles only — the title in the list (`.task-open > strong`), the title at the top of the edit dialog (`.dialog-task-title`) and titles inside placements (`.placement strong`). A system handwriting stack (`Segoe Print`, `Bradley Hand`, `Comic Sans MS`, `Chalkboard SE`, `Marker Felt`, `cursive`), never a download. Everything else stays in the body or heading stack.

## Space, rules and shape

- Gutter: `--gutter` 1rem (16px) on every side; page max width 1100px. The page must not scroll sideways at 360px.
- Steps: `--space-1` 0.25rem, `--space-2` 0.5rem, `--space-3` 0.75rem, `--space-4` 1rem. Grid gaps are 1rem.
- Week rows: `--hour-h` 64px, the same number `web/src/week.rs` uses to place the now-line — change both together.
- **Sections are not cards:** `section` has no padding, border, radius, background or shadow. A heading with a small-caps face and the whitespace divide the page.
- **The page fold:** on wide screens the planner pane gets a 1px `--rule-strong` left border and 2rem padding, like the next page of an open spread. The task list keeps the red `--margin-line` on its left at every size — it is the written page.
- **Controls:** buttons are plain ink text (44px targets kept); hover underlines at 4px offset. The active tab or filter is 700-weight accent text with a 2px accent underline at 6px offset — no outline, no fill.
- **Inputs:** transparent background, no box, one `--rule-strong` underline — writing on a rule. Focus turns the underline `--accent`.
- **Task rows** are separated by a solid `--rule` hairline, like ruled paper.
- **Placements** are a `--wash` tint with a 3px `--accent` left bar and square corners — ink laid across the hour lines, not a chip.
- Radii are 0 everywhere (`--radius-sm`, `--radius-md` 2px for the loose-sheet dialog) except round things: Rambo's nest (`--radius-round`) and the ask bubble (12px).
- Errors keep a red left bar (3px `--danger`) on a faint red wash, square corners.

## Paper texture

Two graphics live in `web/assets/theme/` and are copied into `web/dist` by `index.html`'s `copy-dir` link:

| File | Size | Where it is used |
|---|---|---|
| `paper.svg` | 512x512, tileable | page background, light mode (`--texture-paper`): full-bleed `#f2e8cf` with stitched grain at 5% alpha |
| `paper-dark.svg` | 512x512, tileable | page background, dark mode: `#241c11` with the same grain |

The torn edge strip and the corner stain from earlier drafts were dropped: they read as a glitch and dirt, not paper (owner brief). If a texture ever comes back it must be subtler than the grain and must not sit in the masthead.

Every texture sits on top of a `--paper*` colour, so a missing file degrades to flat parchment with no layout change. The files are drawn separately; names are fixed by the asset contract in `web/assets/theme/README.md`.

## Mobile first

Check 360px and 390px before a wide window. The layout behaviour comes from `web/src/lib.rs` and `web/src/week.rs` (PR #135) and is left as it is; the theme owns colours, fonts, spacing, paper and ruled lines.

- **Panes:** the base rules are the phone: `main` is a horizontal scroll-snap row — the To do pane and the Planner pane sit side by side, swiped or tapped via `.pane-switch`. Those declarations (scroll-snap, `grid-auto-columns: 100%`, `position: relative`, hidden scrollbar) are behaviour and are not restyled; the theme colours `.pane-switch button.active` (golden-brown underline, 700 weight) and gives the tab buttons a 44x44 target. The buttons carry `aria-pressed`; tapping one scrolls the pane row to that pane's own `offset_left` (`show_pane` in `web/src/lib.rs`). From `min-width: 701px` the panes become a two-column grid, the fold rule appears between them, and the switcher hides.
- **Tap targets:** every control is at least `--tap` (2.75rem / 44px) tall or wide — tabs, chips, Mic, Add, Delete, the settings summary and the download link. Checkboxes and radios stay 1.25rem so they do not become grey boxes; their 44px hit area comes from the row (`.task-list li`) or the `<label>` they sit in.
- **Type:** body text never below 1rem (16px), so iOS does not zoom the inputs. Only meta text (`--text-xs`: hour labels, version, the Overdue tag) is smaller.
- **Capture box:** on a phone it is a fixed bar at the bottom edge with the safe-area inset, so Mic, Rambo and Add sit under the thumb; the body reserves matching bottom padding. It is page-coloured with a top rule — it belongs to the page, it is not a floating card. On desktop it stays in the masthead, fully transparent.
- **No sideways page scroll:** the page itself never scrolls horizontally — only the pane row and the week grid scroll, each inside its own box (`--days` columns, 8rem minimum per day on a phone, 9rem on desktop).
- **Dialog:** fixed and centred (behaviour from #135), `box-sizing: border-box`, `max-width: calc(100vw - 2rem)`, `width: min(32rem, 100vw - 2 * gutter)`, `max-height: calc(100dvh - 2 * gutter)`, scrolls inside itself. Its fieldsets are divided by a top rule, not boxed.
- **No scroll chaining:** the week grid and the pane row both use `overscroll-behavior: contain` (`overscroll-behavior-x` on `.week-scroll`), so a flick there never scrolls the page.

## Light and dark

Dark follows the OS (`prefers-color-scheme`) until the user picks a theme. The CSS already honours `data-theme="dark"` and `data-theme="light"` on `:root` over the OS preference; the in-app toggle that sets them lives in settings (planned, `docs/REDESIGN.md` section 1). Nothing may rely on a texture alone for contrast.

## Motion

- Durations: `--dur-fast` 120ms (colour changes on buttons and inputs), `--dur` 200ms (press of the talk button, general transitions), `--dur-slow` 450ms (Rambo's fall and skitter home). Easing `--ease` (`ease`), except Rambo's keyframes which keep their in/out easing.
- Animate transform, opacity and colour only — never layout-affecting properties, and nothing that moves text while it is being read.
- Nothing loops except the mic's listening pulse (an opacity dip, not a glow).
- `prefers-reduced-motion: reduce` turns every animation and transition off (`animation: none; transition: none`), including Rambo's fall, which then snaps home — matching section 8.1 of `docs/ARCHITECTURE.md`.

## Rules for changes

1. Colours come from a token; no hex literals outside `:root` and its dark blocks.
2. System fonts only.
3. Keep every class name in `web/src` working; add a token rather than a special case.
4. If you add a graphic, put it in the contract table above and make the page look right with the file deleted.
5. Chamoisee never carries text; if a line colour needs to be a text colour, mix it darker first and check the ratio in both themes.
