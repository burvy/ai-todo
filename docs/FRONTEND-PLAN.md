# Front-end plan: where `web/` stands and how to restructure it

Written 2026-10-14 by Emilio (front-end owner) after the human's report on the test site (fa48124): *Rambo still moves with his nest and flies to the top left*. Part 1 is what I found. Part 2 is the plan in small steps, each one PR that leaves the app working. `docs/DESIGN-DIRECTION.md` stays the authority on look and feel; this file is about structure.

## 1. Assessment

### 1.1 The Rambo/nest bug: four causes, all fixed in step 0

Reproduced on my own server at desktop width in Chrome with a real mouse. I could not get the browser below 905px wide, so phone width and touch were checked from the code and layout, not on a device.

| What the human saw | Cause | Where |
|---|---|---|
| "Moves with his nest" | The paper dish was painted on Rambo's own button (`.rambo` had the sunken paper background, border and round radius). The real nest button was transparent and `nest.svg` was never used. Dragging Rambo carried the dish; the nest looked gone. | `web/theme.css` `.rambo`, `.nest` |
| "Flies to the top left" (on press) | A rule from the days when Rambo sat inside the nest, `.rambo:active { transform: translateY(1rem) rotate(8deg) }`, comes later in the file than every state rule, so while he is pressed it replaces his `translate(--pen-x, --pen-y)`. With the 200ms transform transition he *slides* to the window's top-left corner. Touch browsers apply `:active` for the whole press. | `web/theme.css` (motion block) |
| Floats away from the nest / vanishes off the top | His home spot was measured only on mount, on a window `resize` and on a state change. On desktop the nest is in the masthead and scrolls; Rambo is `position: fixed` and stayed put. Dropped, then the page scrolled, he skittered to where the nest had been when he set off and stayed off screen (measured: stuck at y = -299 after scrolling back). On a cold load he sat 10px off the nest, because the scrollbar appears as tasks load and fires no resize. | `web/src/rambo.rs` |
| Flies off the left edge on a phone (from code) | After a capture the pen flies to the new row to write it. On a phone the two panes sit side by side in a sideways scroll-snap; with the planner page showing, the list pane is one screen to the left, so the row's rectangle is at negative x and near the top. | `web/src/tasks/mod.rs`, `tasks/write.rs` |

### 1.2 Component structure

- `lib.rs` (242 lines) owns the `App`: about a dozen signals (snapshot, settings, error, pane/page, mascot state, talking, typing, `pen_pos`, ink `strokes`, `capture_text`, `pen_queue`, brain speech and pose) and mounts `CaptureBox`, `brain::Controller`, `TaskList`, the week, the tack board, `InkLayer` and `Rambo`. The #255 split made this readable.
- Three files are still large and do several jobs: `tack.rs` (1426: physics, drag, bin, rendering), `week.rs` (1303: grid, placement maths, planner drag), `tasks/mod.rs` (1296: list, filters, row view, the writing queue).
- Pure logic is well separated and tested: `mascot.rs` (state machine), `pen.rs` (writing schedule), `row.rs` (swipe), `ink.rs`, `rambl-brain`. The DOM code that drives them is not tested at all.

### 1.3 State ownership: who moves Rambo

- **Position.** `pen_pos` is one `RwSignal<(f64, f64)>` written from three modules: `rambo.rs` (drag, home, skitter), `capture.rs` (caret) and `tasks/write.rs` (writing). Nobody owns "where is he", so each writer has to know the others' rules. The bugs above are what happens when one writer forgets the page can move.
- **Mood state.** `mascot::transition` is the documented way to change state, but nine call sites set a `State` directly (`set_state.set(mascot::State::Home)` and similar) in `rambo.rs`, `tasks/mod.rs` and `capture.rs`, so the machine's tests do not cover every path the page takes.
- **Timing.** Animations run on a mix of `setInterval(16ms)`, chained `Timeout`s with ticket counters, CSS transitions and CSS keyframes. Background tabs throttle all of them differently. The writing loop and the skitter each keep their own clock.

### 1.4 The Rambo/nest layering and positioning model

Rambo is one `position: fixed` button at `top: 0; left: 0`, moved only by `transform: translate(calc(var(--pen-x) - 50%), calc(var(--pen-y) - 50%))`. The model is right (it is the standing rule: Rambo independent of the fixed nest), but it is fragile in CSS. 24 `.rambo…` rules each restate the whole translate before adding a rotate or scale, and any rule that sets `transform` without it teleports him to the corner. A test now guards this (`mascot::tests::every_state_of_the_button_keeps_him_where_he_is`). The lasting fix is two elements: an outer one that is only placed, and an inner one that is only posed (step 3).

### 1.5 CSS layering and z-index

Raw numbers, no scale. What they say today:

| Layer | z-index | Problem |
|---|---|---|
| ink layer, page curl, placements, lifted row, desktop tack board | 2 | fine |
| capture bar, landing | 3 | fine |
| day names, phone tack board | 4 / 5 | fine |
| gear backdrop / gear menu | 8 / 9 | **under Rambo (50)**: he covers the open menu |
| task dialog | 10 | **under Rambo** and under the page note |
| page note | 40 | fine |
| Rambo, his status bubble / the ask bubble | 50 / 51 | above dialogs |
| drag ghost, tack pin | 60 | the pin is above Rambo; he slides under it |
| tack bin | 70 | above everything, including dialogs |

### 1.6 CSS files

`theme.css` is 1769 lines and `tack.css` 308. The end of `theme.css` has about 14 one-line rules appended by different PRs (swipe, delete mark, lifted row) outside their sections. There are two phone breakpoints, `max-width: 700px` / `min-width: 701px` and `max-width: 720px` (the delete mark), so between 701 and 720px the desktop layout shows the phone's delete mark.

### 1.7 Other things I saw while reproducing

- At 905px wide the To do column is about 100px: the tack board takes 160px and the planner the rest, so titles wrap word by word ("Send / — / 30 min · / Wed 8:30 / am"). The desktop grid needs a minimum for the list.
- When writing ends, the pen does not fly home: `FlyHome` keeps `pen_pos` at the end of the row, then he jumps when the state turns `Home`.
- At home there is still a 200ms transform transition, so on a fast scroll he trails the nest by a moment before settling.

## 2. Plan

Each step is one PR, gated in full, small enough to review in one sitting, and leaves the app working. "Core" means Rust or behaviour and the front-end owner does it. "CSS" means an isolated stylesheet change a GLM agent can take, with before/after screenshots at 1366x800 and 390x844, light and dark, in the PR.

| # | Step | Owner | Size |
|---|---|---|---|
| 0 | **Rambo/nest fix** (the four causes in 1.1): dish on the nest, the `:active` rule gone, follow the nest on any scroll/resize/page-size change/font load, aim the skitter at the nest's current place, no flight to off-screen rows. Test for the transform rule and for `pen::on_screen`. | Core (Emilio) | done, separate PR |
| 1 | **One owner for his place.** A `Place` enum in `mascot.rs` (`Nest`, `At(x, y)`, `Nib(x, y)`); `rambo.rs` turns it into `--pen-x/--pen-y`, measuring the nest live for `Nest`. Writers say where he should be, never compute pixels for him. Fixes the jump at the end of writing (1.7). | Core | M |
| 2 | **Every state change through `transition`.** Remove the nine direct `set_state(State::…)` calls; add the missing events; one test per path. | Core | S |
| 3 | **Two-element Rambo.** An outer element that is only placed (translate) and an inner one that is only posed (rotate, scale, keyframes). The 24 restated translates go; a pose can no longer move him. Drop the home transition so he sits on the nest exactly. | Core view + CSS | M |
| 4 | **z-index scale.** Tokens in `:root`: page 0, sheet 2, capture 3, ink 4, Rambo 20, bubble 21, drag 30, menu 40, dialog 50, note 60. Rambo goes under menus and dialogs; the tack pin and bin go under Rambo. One table in `docs/THEME.md`. | CSS (GLM) | S |
| 5 | **Split `theme.css`.** `tokens.css`, `layout.css`, `rambo.css`, `rows.css`, `planner.css`, `dialogs.css`, each linked by trunk; fold the appended one-liners into their sections; one breakpoint (700/701) everywhere. No visual change: screenshots must match. One file per PR. | CSS (GLM) | S each |
| 6 | **Desktop desk widths.** Give the list a minimum (`minmax(18rem, 1fr)`) and let the tack board shrink first, so titles never wrap word by word at 905–1366px. | CSS (GLM), checked by Core | S |
| 7 | **One animation clock.** A small `ticker` module on `requestAnimationFrame` that the skitter, the writing loop and the tack physics share, replacing `setInterval(16)` and ticket counters. Pauses cleanly in a background tab. | Core | M |
| 8 | **One pointer-gesture module.** Rambo drag, row swipe, planner drag and tack drag each repeat slop, pointer capture, cancel and Esc. Share it as `gesture.rs` with pure tests, as `row::advance` already is. | Core | M |
| 9 | **Split the three big files** the way `lib.rs` was split: `tack.rs` (physics / drag / view), `week.rs` (layout maths / drag / view), `tasks/mod.rs` (list view / writing queue). No behaviour change. | Core | M each |
| 10 | **A browser check list in `QA-BASELINE.md`** for every front-end PR: cold load, first drag, dazed drag, put back, skitter home, page turn, resize, scroll, at desktop and phone width with touch. QA (Antonio) runs it on the test site. | QA | S |

Order: 0 now; then 2 and 4 (small, independent); then 1 and 3 together, since they touch the same code; 5 and 6 can run in parallel with GLM agents any time after 4; 7, 8 and 9 last. The design backlog (UI-BUGS #8–#11) stays with the human and fits after step 5, when each look lives in its own file.

## 3. Not checked

- Phone width and touch were not reproduced in a browser (the window would not shrink below 905px here); the off-pane flight is read from code and layout. QA should confirm step 0 on a phone.
- Dark mode was not looked at for the nest drawing; `nest.svg` restates its own dark colours.
