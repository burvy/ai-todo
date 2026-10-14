# Front-end plan: where `web/` stands and how to restructure it

Written 2026-10-14 by Emilio (front-end owner) after the human's report on the test site (fa48124): *Rambo still moves with his nest and flies to the top left*. Part 1 is what I found. Part 2 is the plan: principles and a target that stay stable, the first steps as tickets for GLM agents, and later steps as intent only. `docs/DESIGN-DIRECTION.md` stays the authority on look and feel; this file is about structure.

## 1. Assessment

### 1.1 The Rambo/nest bug: five causes, all fixed in FE-0 (#267)

Reproduced on my own server at desktop width in Chrome with a real mouse. I could not get the browser below 905px wide, so phone width and touch were checked from the code and layout, not on a device.

| What the human saw | Cause | Where |
|---|---|---|
| "Moves with his nest" | The paper dish was painted on Rambo's own button (`.rambo` had the sunken paper background, border and round radius). The real nest button was transparent and `nest.svg` was never used. Dragging Rambo carried the dish; the nest looked gone. | `web/theme.css` `.rambo`, `.nest` |
| "Letting go sends him to the corner" (PC and phone, every drop) | The button had a 200ms `transform` transition for every state. It also runs when a state's animation ends: held to dazed, or splat to dazed. Chrome started that transition near the window's top-left corner. Measured on main: on release his drawn position jumped to (101, 21), then slid back to the drop point at (500, 450) over 200ms. The same transition flew him in from (0, 0) on every cold load. | `web/theme.css` (motion block) |
| "Flies to the top left" (on press) | A rule from the days when Rambo sat inside the nest, `.rambo:active { transform: translateY(1rem) rotate(8deg) }`, comes later in the file than every state rule, so while he is pressed it replaces his `translate(--pen-x, --pen-y)`. With the 200ms transform transition he *slides* to the window's top-left corner. Touch browsers apply `:active` for the whole press. | `web/theme.css` (motion block) |
| Floats away from the nest / vanishes off the top | His home spot was measured only on mount, on a window `resize` and on a state change. On desktop the nest is in the masthead and scrolls; Rambo is `position: fixed` and stayed put. Dropped, then the page scrolled, he skittered to where the nest had been when he set off and stayed off screen (measured: stuck at y = -299 after scrolling back). On a cold load he sat 10px off the nest, because the scrollbar appears as tasks load and fires no resize. | `web/src/rambo.rs` |
| Flies off the left edge on a phone (from code) | After a capture the pen flies to the new row to write it. On a phone the two panes sit side by side in a sideways scroll-snap; with the planner page showing, the list pane is one screen to the left, so the row's rectangle is at negative x and near the top. | `web/src/tasks/mod.rs`, `tasks/write.rs` |

### 1.2 Component structure

- `lib.rs` (242 lines) owns the `App`: about a dozen signals (snapshot, settings, error, pane/page, mascot state, talking, typing, `pen_pos`, ink `strokes`, `capture_text`, `pen_queue`, brain speech and pose) and mounts `CaptureBox`, `brain::Controller`, `TaskList`, the week, the tack board, `InkLayer` and `Rambo`. The #255 split made this readable.
- Three files are still large and do several jobs: `tack.rs` (1426: physics, drag, bin, rendering), `week.rs` (1303: grid, placement maths, planner drag), `tasks/mod.rs` (1296: list, filters, row view, the writing queue).
- Pure logic is well separated and tested: `mascot.rs` (state machine), `pen.rs` (writing schedule), `row.rs` (swipe), `ink.rs`, `rambl-brain`. The DOM code that drives them is not tested at all.

### 1.3 State ownership: who moves Rambo

- **Position.** `pen_pos` is one `RwSignal<(f64, f64)>` written from three modules: `rambo.rs` (drag, home, skitter), `capture.rs` (caret) and `tasks/write.rs` (writing). Nobody owns "where is he", so each writer has to know the others' rules. The bugs above are what happens when one writer forgets the page can move. The target in 2.2 gives it one owner.
- **Mood state.** `mascot::transition` is the documented way to change state, but nine call sites set a `State` directly (`set_state.set(mascot::State::Home)` and similar) in `rambo.rs`, `tasks/mod.rs` and `capture.rs`, so the machine's tests do not cover every path the page takes.
- **Timing.** Animations run on a mix of `setInterval(16ms)`, chained `Timeout`s with ticket counters, CSS transitions and CSS keyframes. Background tabs throttle all of them differently. The writing loop and the skitter each keep their own clock.

### 1.4 The Rambo/nest layering and positioning model

Rambo is one `position: fixed` button at `top: 0; left: 0`, moved only by `transform: translate(calc(var(--pen-x) - 50%), calc(var(--pen-y) - 50%))`. The model is right (it is the standing rule: Rambo independent of the fixed nest), but it is fragile in CSS. 15 `.rambo…` rules each restate the whole translate before adding a rotate or scale, and any rule that sets `transform` without it teleports him to the corner. A test now guards this and the `:active` rule of 1.1 (`mascot::tests::every_state_of_the_button_keeps_him_where_he_is`), and it stays. The lasting fix is two elements: an outer one that is only placed, and an inner one that is only posed (2.2).

### 1.5 CSS layering and z-index

Raw numbers, no scale. What they say today:

| Layer | z-index | Problem |
|---|---|---|
| ink layer, page curl, placements, lifted row, desktop tack board | 2 | fine |
| capture bar, landing | 3 | fine |
| day names, phone tack board | 4 / 5 | fine |
| gear backdrop / gear menu | 8 / 9 | **under Rambo (50)**: he covers the open menu |
| task dialog | 10 | **under Rambo** and under the page note. It opens with the `open` attribute, not `showModal()`, so z-index applies to it |
| page note | 40 | fine |
| Rambo, his status bubble / the ask bubble | 50 / 51 | above dialogs |
| drag ghost, tack pin | 60 | the pin is above Rambo; he slides under it |
| tack bin | 70 | above everything, including dialogs |

### 1.6 CSS files

`theme.css` is 1769 lines and `tack.css` 308. The end of `theme.css` has about 14 one-line rules appended by different PRs (swipe, delete mark, lifted row) outside their sections. There are two phone breakpoints, `max-width: 700px` / `min-width: 701px` and `max-width: 720px` (the delete mark), so between 701 and 720px the desktop layout shows the phone's delete mark.

### 1.7 Other things I saw while reproducing

- At 905px wide the To do column is about 100px: the tack board takes 160px and the planner the rest, so titles wrap word by word ("Send / — / 30 min · / Wed 8:30 / am"). The desktop grid needs a minimum for the list.
- When writing ends, the pen does not fly home: `FlyHome` keeps `pen_pos` at the end of the row, then he jumps when the state turns `Home`.

## 2. Plan

**This plan is revised after every step lands.** Only the next few steps are written as tickets; the later ones are intent, re-planned with what each step teaches us. Any agent may propose a change to god at any time. Sections 2.1 and 2.2 are what stays stable; everything after them can move.

The builders are GLM agents, from a spec written by us and reviewed by a Claude agent. Emilio (front-end owner) is the architect and reviewer, not the builder: he reviews each PR against its ticket before god merges, and writes the next tickets when a step lands.

### 2.1 Principles

- **One owner for each fact.** Where Rambo is, what he is doing, how long a motion takes: each is decided in one place, and everything else reads it.
- **Where he is follows from what he is doing.** His place is derived from his state, never a second signal that can disagree with it.
- **Anchor to things, not pixels.** He points at the nest, the caret or a row, measured live, so a scroll, a page turn or the phone keyboard cannot strand him. Pixels only where the finger is the truth (held, dropped).
- **Pure logic, thin DOM glue.** Rules live in pure functions with tests (`mascot.rs`, `pen.rs`, `row.rs` are the model); the browser code only measures and draws.
- **Guard tests over good intentions.** A rule worth keeping gets a test that reads the source or the CSS as text and fails when someone breaks it (`rambo_rules` in #267 is the model). Guard tests are never weakened to make a PR pass.
- **Order in CSS is explicit.** Later-rule-wins bugs (1.1 cause 3, the appended rules of 1.6) are prevented by structure, not by care.
- **No magic numbers.** Every duration, distance and z-index has a name and a one-line reason, in one tuning place, following the office convention in `docs/TUNING.md` (Claudio, in progress).
- **Each step leaves the app working**, is one PR, and is checked by the QA list (FE-1) and, for anything visual, a pixel diff.

### 2.2 Target architecture

Where we are heading. The shape is stable; the exact names are guidance, and each step may adjust them with a reason.

- **Rambo's place.** `mascot::State` carries what each state needs (`WritingRow { task, progress }`, `Dazed { at }`), and a pure `anchor(&State)` gives `Nest`, `Caret`, `Row { task, progress }`, `Held { x, y }` or `Dropped { x, y }`. One resolver in `rambo.rs` measures the anchor live and is the only code that sets `--pen-x/--pen-y`. It re-resolves on page moves (#267's `on_page_moved`) and on `visualViewport` changes (the phone keyboard), and decides once what to do when the anchor is off screen. `pen_pos` goes away. Every state change goes through `mascot::transition`.
- **Rambo's element.** Two elements: an outer button that is only placed, an inner one that is only posed (rotate, scale, splat, art). His rim at rest is a clip on the inner one.
- **His art follows the page theme**, not the OS: the drawings use the page's `--rambo-*` tokens. The real dish replaces the `nest.svg` placeholder; that is art for Ping, owner named by god.
- **Tuning.** `web/src/tuning.rs` holds every motion and gesture constant with its reason, grouped by DESIGN-DIRECTION §4's beats. The CSS gets the times as `--t-*` custom properties written onto `:root` from that list, and uses no raw times.
- **CSS.** Cascade layers declared once (`@layer tokens, layout, rows, planner, tack, dialogs, rambo, overrides;`), every rule inside a layer. z-index as named tokens with gaps, ink always under the capture line and dialogs. One phone breakpoint. Splitting into files is optional once layers hold the order.
- **One animation clock and one gesture module.** A rAF ticker that every motion registers with (with a fixed-step mode for the tack physics, and `reduced_motion()`); non-animation timers (the 60s now-line, the 5s brain poll) stay. A gesture module with pure tests for slop, claim and release by pan direction, `pointercancel` (real touch ends with it once the browser takes a pan, as #210 proved), Esc and velocity; it sets `touch-action` per surface.
- **Files under about 600 lines.** `tack.rs`, `week.rs` and `tasks/mod.rs` split by job, move-only.

### Rules for every ticket

- Read `AGENTS.md`, then only the files the ticket lists. Touch only those files. If the work needs another file, stop and ask god.
- Own worktree and branch: `git worktree add ../ai-todo-wt/<you>-<ticket> origin/main -b <you>/<ticket>`.
- One PR per ticket. The title starts with the ticket id, for example `FE-3: one breakpoint, folded rules`.
- Run the full gate once at the end and paste the results (fmt, clippy `-D warnings`, `cargo test --workspace`, the wasm32 check, `trunk build --release`).
- **Visual tickets** (any CSS or art change): a pixel diff, not eyeballing. Take four screenshots on `main` and four on the branch, on the same machine with the same fonts: 1366x800 and 390x844, each in light and dark. Report for each pair either "identical" or the share of pixels that differ. Over 0.1% must be a change the ticket asks for, and the PR names it.
- **Behaviour tickets**: add a test that fails without the change, and say in the PR that you checked it fails.
- **Guard tests stay green.** `mascot.rs` holds tests that read the source and CSS as text (`rambo_rules` and the ones these tickets add). Never weaken one; a ticket that needs to may not go ahead without Emilio.
- Never edit `docs/DESIGN-DIRECTION.md`. Never add a crate. Never change server code.
- Respect the standing rules: Rambo stays a separate fixed layer, independent of the nest. No blue focus or tap highlight. No native browser controls or scrollbars. The theme never follows the OS (DESIGN-DIRECTION §7).
- The reviewer (Emilio) checks each acceptance item one by one. A PR that misses one goes back.

### 2.3 Next steps: tickets

**FE-0: Rambo/nest fix.** Done by Emilio in PR #267; it is the base for everything below.

---

**FE-1: front-end QA check list** · QA (Antonio) · small
- Files: hive `QA-BASELINE.md`.
- Do: a check list run on the test site for every front-end PR, at 1366x800 with a mouse and at 390x844 with touch:
  - cold load: Rambo on the nest
  - first drag listens
  - release: no jump
  - dazed drag writes
  - put back in the nest
  - skitter home after 8s
  - page turn, resize, scroll: he stays on the nest
  - phone: the on-screen keyboard opens and closes
  - capture, then he writes the row
  - capture made on the planner page
  - a touch drag that the browser takes over as a scroll ends cleanly (the drag stops, nothing sticks to the finger)
  - with the page on Light and the OS on dark (and the other way round), Rambo and the dish match the page
- Accept: Antonio runs it once on main after #267 and records the result.

**FE-2: Rambo's art follows the page theme** · bug fix · art + Rust · medium
- The bug: every `web/assets/rambo/**/*.svg` and `nest.svg` carries its own `@media (prefers-color-scheme: dark)` block and is loaded by URL (`Pose::art` in `mascot.rs`), so it follows the OS, not the page's `data-theme`. On a dark OS with the page on Light, Rambo and the dish turn dark on parchment.
- Files: `web/assets/rambo/**/*.svg`, `web/src/rambo.rs`, `web/src/mascot.rs`, `web/theme.css`.
- Do: draw the art with the page's colour tokens instead of a media query.
  - In each SVG, delete the `prefers-color-scheme` block and use `var(--rambo-…, <light value>)` for each colour the block changed. Give the drawing's root group `id="art"`.
  - Suggested: show it with `<svg><use href="assets/rambo/<tool>/<pose>.svg#art"/></svg>` instead of `<img>` or a background. Content shown through `<use>` inherits CSS custom properties from the page, so `--rambo-*` from `:root[data-theme]` reaches it.
  - The nest is a CSS background since #267; draw it the same way, as an `<svg><use>` inside `.nest`.
  - Another way that meets the checks below is fine; say why in the PR.
  - Add any missing `--rambo-*` dark values to the dark-theme block of `theme.css`, taken from the SVGs' old media blocks.
- Accept:
  - `grep -rln "prefers-color-scheme" web/assets` finds nothing.
  - A test in `mascot.rs` checks that `Pose::art` returns a `…svg#art` reference for every pose and tool.
  - Pixel diff with the OS in light mode: page Light and page Dark both identical to main.
  - Screenshots with the OS in dark mode and the page on Light: Rambo and the dish are the light drawing.
- Do not touch: the drawings' shapes, and any colour that has no `--rambo-*` token (ask instead).
- Note: `nest.svg` is a placeholder ("PLACEHOLDER: replace me") in the old brown palette, not the dish in DESIGN-DIRECTION §2. The real dish is art for Ping. God names the owner; until it lands, no ticket styles the placeholder.

**FE-3: CSS tidy: one breakpoint, folded rules** · CSS · small
- Files: `web/theme.css`.
- Do:
  - Change `@media (max-width: 720px)` (the delete mark) to `@media (max-width: 700px)`, so every phone rule switches at the same width as the layout.
  - Move the one-line rules appended at the end of the file (swipe, delete mark, lifted row, about 14 lines after the reduced-motion block) into the sections they belong to, written in the file's normal multi-line style. Delete any rule that an earlier rule already fully repeats.
- Accept:
  - `grep -n "@media (max-width" web/*.css` shows only `700px`.
  - At 710px the desktop layout shows the desktop delete control, not the phone one.
  - Pixel diff: all four identical.
  - Every rule's selector still appears exactly once per media block, and the file ends with its last section, not with loose rules.
- Do not touch: values. This is a move, not a redesign.

### 2.4 Later steps: intent only

Each becomes a ticket when the steps before it have landed, written with what they taught us. The order is a guess, and so is the split into steps. Fable's review of #268 is the main guidance for each.

- **Every state change through `transition`.** Nine places set a `State` directly; route them through `mascot::transition`, with a guard test that finds none. (Small; could be the next ticket.)
- **z-index tokens.** Name today's stacking, ink under the capture line, a guard test that every `z-index` is a token; note in THEME.md that a `showModal()` dialog would sit above every token. Reordering Rambo against menus, dialogs, pin and bin waits for question 4.
- **Tuning.** Constants into `web/src/tuning.rs`, then the CSS times as `--t-*`, each with a guard test. Where CSS and Rust disagree today (`FLY_MS = 450` against the CSS flight), Rust wins.
- **Cascade layers.** Wrap every rule in its layer, keeping today's order where rules compete; pixel diff identical.
- **Desktop widths.** A minimum width for the list, tack board shrinks first; checked at 905, 1024, 1100 and 1366px. Waits for question 5.
- **Two-element Rambo.** Paired review with Emilio. Size and rim from questions 11 and DESIGN-DIRECTION §2.
- **His state says where he is, then one resolver.** Probably two PRs: the pure `anchor` first, then the resolver that deletes `pen_pos`. Paired review. Off-screen behaviour from question 3, keyboard from question 8.
- **One animation clock, then one gesture module.**
- **Split the big files**, move-only, last.

The design backlog (UI-BUGS #8–#11) stays with the human and fits after the layers step, when each look lives in its own layer.

## 3. Not checked

- Phone width and touch were not reproduced in a browser (the window would not shrink below 905px here); the off-pane flight is read from code and layout. QA should confirm FE-0 on a phone.
- That content shown through `<use>` picks up the page's `--rambo-*` tokens (FE-2) is how browsers specify it; the ticket's screenshots are the check.

## 4. Open spec questions for the human

God settles these with the human. The first steps (2.3) do not depend on them; the later ones do, and are re-planned with the answers.

1. **Desktop capture line.** On desktop the capture line and nest sit in the masthead and scroll away with the page, and Rambo goes with them. Should they stay pinned on screen as they do on a phone, so he is always within reach?
2. **Dazed.** Dropped on the page he stays dazed for 8 seconds, then runs home. Is 8 seconds right? Should scrolling or turning the page send him home early?
3. **Writing a row that is off screen.** After a capture made on the planner page (phone), or while scrolled down, the new row is off screen. Today he skips the flight and the row just appears. Should the page turn or scroll to the row so he can write it?
4. **Layers.** Should Rambo sit under the gear menu and dialogs, or stay above everything? Should the tack pin and bin go under him?
5. **Desktop widths.** Between 905 and 1366px, which gives way first: the tack board, the planner or the list? What is the narrowest the list may get?
6. **Breakpoint.** The phone layout ends at 700px, so a tablet held upright (768px) gets the desktop layout. Is that right?
7. **Phone pages.** Which page opens first? Should the capture bar show on the planner page too?
8. **Phone keyboard.** When the on-screen keyboard is up, the capture bar rises above it. Does Rambo ride up with the dish, or stay where he was?
9. **What interrupts him.** What happens on a second capture while he is still writing (DESIGN-DIRECTION §4 says finish the row at once and go)? A tap on the dish while he is dazed? A page turn while he flies?
10. **Motion and sound.** Which motions are must-haves for speed and feel: flight to the row, letter-by-letter writing, splat, skitter home, idle breathing, page curl? Which can go if they cost time? Which sound goes with which motion (the scratch follows the nib)?
11. **Size.** DESIGN-DIRECTION §2 says 1.4 times today's size: 64px on a phone, 80px on desktop. Is that the size to build to? The two-element step (2.4) fixes his box.
12. **Listening feedback.** While he is held and listening, what should show: a pose, a bubble, live transcript text? What should a browser without speech recognition show?

Settled, so not asked: the theme never follows the OS (DESIGN-DIRECTION §7; Rambo's art breaking that is FE-2, a bug fix). Rambo hiding behind the rim is built with a clip on his inner element (2.4, two-element Rambo), no new layer.

For god, not the human: who draws the real dish to replace the `nest.svg` placeholder (Ping?).
