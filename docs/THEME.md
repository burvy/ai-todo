# Rambl theme: one sheet of paper

The spec for how Rambl looks. Code lives in `web/theme.css` (linked from `web/index.html` with `<link data-trunk rel="css" href="theme.css">`, so trunk copies it into `web/dist`, which the server embeds). Components must use the tokens below rather than raw colours.

The look is **one page of parchment, written on in ink** (owner brief, `docs/DESIGN-DIRECTION.md`): space and ruled lines divide the page — no boxes, borders or stacked cards. Controls are words on the page until hover or focus underlines them; inputs are written on a rule. Only two things may look like objects: the edit dialog (a loose sheet laid over the page) and Rambo's speech (a paper bubble — his ask menu and his spoken lines share it, anchored beside his nest).

## Palette

**Light** (the owner's parchment palette, `docs/DESIGN-DIRECTION.md`):

| Token | Hex | Use |
|---|---|---|
| `--parchment` (`--paper`) | `#f2e8cf` | the page |
| `--wheat` (`--paper-raised`) | `#ecd7af` | a sheet laid on the page: dialog, capture bar |
| `--ecru` (`--paper-sunken`) | `#e1c38c` | pressed-in paper: Rambo's nest circle, a pressed choice (with the `--pressed` inset shadow); carries `--ink` only |
| `--chamoisee` | `#a7865d` | **rules and lines only, never text** (2.8:1 on parchment) |
| `--ink` | `#5d4732` | coffee: everything written |
| `--ink-soft` | `#6f5b46` | secondary text (5.3:1 on parchment, 4.6:1 on wheat) |
| `--ink-faint` | `#6e5a43` | muted text, hour labels, version (4.7:1 on wheat, so 12px passes WCAG AA) |
| `--brass` | `#9f682e` | Rambo's cap, clip and nib: a shape, not text |
| `--accent` | `#815525` | golden brown: focus, the active tab/filter underline, the placement bar, a picked choice (5.3:1 on parchment, 4.6:1 on wheat) |
| `--danger` | `#a5342a` | red pencil: errors, Overdue, Delete |
| `--now` | `#a83728` | the current-time line (held to 4.5:1 like text) |
| `--rule` | chamoisee 42% | hairlines between rows and hours |
| `--rule-strong` | chamoisee 62% | page fold, input rules |
| `--margin-line` | `rgba(165,52,42,.24)` | the notebook's red margin, on the task list |
| `--backdrop` | `rgba(46,35,21,.35)` | dialog backdrop |

`--paper`, `--paper-raised`, `--paper-sunken` are kept as aliases so existing rules read naturally.

Every text ink (`--ink`, `--ink-soft`, `--ink-faint`, `--accent`, `--danger`) is at least 4.5:1 (WCAG AA) on `--paper` and `--paper-raised` in both themes, `--ink` (the only ink written on it) is on `--paper-sunken`, and `--now` is on the page and raised paper: `web/src/contrast.rs` reads `web/theme.css` and fails on any pair below it (ATD-144).

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
| `--now` | `#da6e53` |
| `--backdrop` | `rgba(10,8,5,.55)` |

`color-scheme` is `light` or `dark` to match, so checkboxes, scrollbars and the dialog follow the paper.

## Type

Every word is handwritten (`docs/DESIGN-DIRECTION.md`), in the three self-hosted OFL faces in `web/assets/fonts/` (Ping's woff2 set; sources, licences and the tool mapping live in that folder's README):

- **The neat, very legible hand** (`--font-hand-neat`): Patrick Hand, 400. Writes all UI text — buttons, tabs, labels, hour labels, settings, day names. `--font-body` and `--font-heading` resolve to it.
- **The expressive hand** (`--font-hand`): Caveat, variable 400-700. Task titles (`.task-open > strong`, `.dialog-task-title`, `.placement strong`).
- **The tool hand** (`--font-marker`): Kalam, 300/400/700. Rambo's writing tools (ballpoint = light, fountain pen/quill = regular, marker = bold).

System handwriting stacks remain in the tokens as fallbacks only. Hands run small and narrow against a body stack, so the type scale is lifted a step (`--text-base` 1.05rem, `--text-xs` 0.85rem) and body line-height is 1.55; re-check legibility at 390px when touching these numbers.

Scale (rem): `--text-xs` 0.85, `--text-sm` 0.95, `--text-base` 1.05, `--text-lg` 1.25, `--text-xl` 1.5, `--text-2xl` 1.9. Overdue tags are `--text-xs`, uppercase, 0.06em tracking. Headings are weight 700 with 0.05em tracking (no small-caps: synthesized caps on a script face look wrong). Both UI faces are preloaded from `web/index.html`.

## Space, rules and shape

- Gutter: `--gutter` 1rem (16px) on every side; page max width 1100px. The page must not scroll sideways at 360px.
- Steps: `--space-1` 0.25rem, `--space-2` 0.5rem, `--space-3` 0.75rem, `--space-4` 1rem. Grid gaps are 1rem.
- Week rows: `--hour-h` 64px, the same number `web/src/week.rs` uses to place the now-line — change both together.
- **Sections are not cards:** `section` has no padding, border, radius, background or shadow. A heading with a small-caps face and the whitespace divide the page.
- **The page fold:** on wide screens the planner pane gets a 1px `--rule-strong` left border and 2rem padding, like the next page of an open spread. The task list keeps the red `--margin-line` on its left at every size — it is the written page.
- **Controls:** buttons are plain ink text (44px targets kept); hover underlines at 4px offset. The active tab or filter is 700-weight accent text with a 2px accent underline at 6px offset — no outline, no fill.
- **Inputs:** transparent background, no box, one `--rule-strong` underline — writing on a rule. Focus turns the underline `--accent`.
- **Task rows** are separated by a solid `--rule` hairline, like ruled paper.
- **Placements** are slips of `--paper-raised` paper with the page's grain (`--texture-paper`), laid on the hour lines: cut straight at a 3px `--brass` left edge, torn down the right (`--slip-cut`, `--slip-tear`, masks that tile so a tall slip tears as finely as a short one), with `--shadow-drop` and a crook of under half a degree. Back-to-back slips keep a hairline of page between them. The drag ghost is the same paper in hand, lifted by `--lift-drop`, a drop shadow that follows the tear.
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

- **Panes:** the base rules are the phone: `main` is one grid cell holding the To do pane over the Planner pane, and `main[data-pane]` says which shows (the other is out of the layout until a turn). The pages are turned by a swipe, or by the note at the page's foot (`.page-foot`, which says where the other page is). Those declarations (`grid-area: 1 / 1`, `position: relative`) are behaviour and are not restyled; there is no header and no tab switcher: the settings gear and the account link sit at the top right of the planner page only (`.planner-corner`), and the tack board belongs to the first page. From `min-width: 701px` the panes become a two-column grid, the fold rule appears between them, and the page-foot note hides.
- **Tap targets:** every control is at least `--tap` (2.75rem / 44px) tall or wide — tabs, chips, Mic, Add, Delete, the settings summary and the download link. Checkboxes and radios stay 1.25rem so they do not become grey boxes; their 44px hit area comes from the row (`.task-list li`) or the `<label>` they sit in.
- **Type:** inputs and body text sit at `--text-base` (1.05rem ≈ 17px), above 16px so iOS does not zoom the inputs. Only meta text (`--text-xs` 0.85rem: hour labels, version, the Overdue tag) is smaller — re-check its legibility at 390px when changing.
- **Capture box:** on a phone it is a fixed bar at the bottom edge with the safe-area inset, so Mic, Rambo and Add sit under the thumb; the body reserves matching bottom padding. It is page-coloured with a top rule — it belongs to the page, it is not a floating card. On desktop it stays in the masthead, fully transparent.
- **No sideways page scroll:** the page itself never scrolls horizontally — only the pane row and the week grid scroll, each inside its own box (`--days` columns, 8rem minimum per day on a phone, 9rem on desktop).
- **Dialog:** fixed and centred (behaviour from #135), `box-sizing: border-box`, `max-width: calc(100vw - 2rem)`, `width: min(32rem, 100vw - 2 * gutter)`, `max-height: calc(100dvh - 2 * gutter)`, scrolls inside itself. Its fieldsets are divided by a top rule, not boxed.
- **No scroll chaining:** the week grid and the pane row both use `overscroll-behavior: contain` (`overscroll-behavior-x` on `.week-scroll`), so a flick there never scrolls the page.

## Light and dark

**Parchment light is the default and the page never follows the OS** (owner QA, 2026-10-13): `web/index.html` ships `<html data-theme="light">` so even the paint before the app loads is light, the OS media query is not used at all, and dark exists only when the user picks Dark in the gear menu (`settings::Theme`, stored with the other view settings). Nothing may rely on a texture alone for contrast.

## Motion

- Durations: `--dur-fast` 120ms (colour changes on buttons and inputs), `--dur` 200ms (press of the talk button, general transitions), `--dur-slow` 450ms (slow colour changes). Easing `--ease` (`ease`), except Rambo's keyframes which keep their in/out easing.
- Animate transform, opacity and colour only — never layout-affecting properties, and nothing that moves text while it is being read.
- Nothing loops except the mic's listening pulse (an opacity dip, not a glow).
- `prefers-reduced-motion: reduce` turns every animation and transition off (`animation: none; transition: none`), including Rambo's fall, which then snaps home — matching section 8.1 of `docs/ARCHITECTURE.md`.

## Rules for changes

1. Colours come from a token; no hex literals outside `:root` and its dark blocks.
2. Type comes from the three self-hosted faces in `web/assets/fonts/` (`--font-hand-neat`, `--font-hand`, `--font-marker`); no CDNs, no new families without updating the table there.
3. Keep every class name in `web/src` working; add a token rather than a special case.
4. If you add a graphic, put it in the contract table above and make the page look right with the file deleted.
5. Chamoisee never carries text; if a line colour needs to be a text colour, mix it darker first and check the ratio in both themes.
