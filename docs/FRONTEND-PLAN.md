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
| Flies off the left edge on a phone (from code) | After a capture the pen flies to the new row to write it. On a phone (before the stacked pages) the two panes sat side by side in a sideways scroll-snap; with the planner page showing, the list pane was one screen to the left, so the row's rectangle was at negative x and near the top. The pages now lie stacked, and the list is not laid out while the planner shows, so the pen has no row rectangle to fly to (`display: none`). | `web/src/tasks/mod.rs`, `tasks/write.rs` |

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

**Done:** every z-index now reads the `--layer-*` scale in `web/theme.css` `:root` (guarded by `mascot::tests::every_z_index_reads_the_layer_scale`). Rambo and his bubbles sit under the edit sheet and the gear menu; the tack bin and drag ghost still ride above Rambo. A tack strip's own pile order (`strip.z`) is a local stacking index inside the board, not a rung.

### 1.6 CSS files

`theme.css` is 1769 lines and `tack.css` 308. The end of `theme.css` has about 14 one-line rules appended by different PRs (swipe, delete mark, lifted row) outside their sections. There are two phone breakpoints, `max-width: 700px` / `min-width: 701px` and `max-width: 720px` (the delete mark), so between 701 and 720px the desktop layout shows the phone's delete mark.

### 1.7 Other things I saw while reproducing

- At 905px wide the To do column is about 100px: the tack board takes 160px and the planner the rest, so titles wrap word by word ("Send / — / 30 min · / Wed 8:30 / am"). The desktop grid needs a minimum for the list.
- When writing ends, the pen does not fly home: `FlyHome` keeps `pen_pos` at the end of the row, then he jumps when the state turns `Home`.

### 1.8 The cloud audit (issue #259)

A read-only audit of `web/src` and the CSS on 3fc7aba, made by the cloud session in headless Chromium with mouse and touch. Its trace of the release bug agrees with 1.1. It adds one detail: on touch, Chromium never matched `:active`, so the corner jump on a re-grab was a PC bug. Its other findings, and where each one lands in this plan:

| # | Finding | Lands in |
|---|---|---|
| 1 | Done strips are never pinned: `just_done` is built from the same snapshot as `listed`, so it is always empty (`tack.rs`) | BUG-1 (2.5) |
| 2 | Saved strip positions are wiped on load: the reconcile runs on the empty first snapshot and saves it | BUG-2 |
| 3 | Crumple and restore on a repeat-day strip send a `series:date` id the server refuses | BUG-3 |
| 4 | Rambo's Edit only cycles priority, and Move only sends him home; spec 8.1 promises real ones | later step: the ask bubble (2.6) |
| 5 | Planner day and hour labels never refresh (after midnight, or a clock-format change) | BUG-4 |
| 6 | Rambo reacts to "task added" on every page load (`brain.rs`) | BUG-5 |
| 7 | The edit dialog is not modal (`open`, not `showModal()`): no backdrop, the page stays clickable | z-index step: switch to `showModal()`, which also puts it above Rambo, as answered |
| 8 | Every invalid time says "does not exist on this day (clocks change)" (`tasks/fields.rs`) | BUG-7 |
| 9 | The snapshot's date window depends on who fetched last; changing the days setting never re-fetches | BUG-6 |
| 10 | The pen queue is never drained with reduced motion or Lobotomized, and a mid-run reset drops rows | interruptions step (the pausable queue) |
| 11 | `pen_pos` has four writers, counting the nest measure | resolver step (as 1.3) |
| 12 | A row's hold timer is not tied to its press: tap, release and press again within 280ms, and the old timer lifts the new press | gesture step |
| 13–15 | Two occurrence-id parsers; a planner move that bypasses `patch_and_show`; the overdue rule written twice; two copies of `prefers_reduced_motion` | BUG-8 (13, 14); ticker step (15) |
| 16 | Reduced motion is mostly ignored: the `*` reset has zero specificity, so every class rule beats it | cascade layers step: in the last layer it wins whatever the specificity |
| 17–18 | The phone tack board (z 4) covers the capture bar (z 3); everything competes in one stacking context, so day names paint over the capture bar and the ink | z-index step |
| 19 | Rambo's bubbles are children of his transformed button: they tilt 34° and shrink while he writes, run off a 360px screen, and `.ask` nests buttons in a button (invalid HTML) | two-element step: bubbles become siblings of the button in the Rambo layer |
| 20 | Torn strip edges never show (opaque background over a `z-index: -1` edge) | notebook look |
| 21, 24 | Undefined `--coffee`; hard-coded `.error` colour; no dark `--fold-shadow`; body padding 68px under a 72px bar; the 720px breakpoint | FE-5 (21, 24 except the breakpoint); FE-3 (breakpoint) |
| 22–23 | Dead Rust (unused API functions, native-only stubs, HTML drag handlers Rambo never fires) and CSS no element uses (`.mic`, `.task-editor`, `.delete`, `.import`, `.swipe-*`, four `pose-*`, the `prefers-color-scheme` block in `tack.css`) | FE-5 |

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

- **Rambo's place.** `mascot::State` carries what each state needs (`WritingRow { task, progress }`, `Dazed { at }`, `Smacked { side }`, `Flattened { at }`), and a pure `anchor(&State)` gives `Nest`, `Caret`, `Row { task, progress }`, `Held { x, y }` or `Dropped { x, y }`. One resolver in `rambo.rs` measures the anchor live and is the only code that sets `--pen-x/--pen-y`. It re-resolves on page moves (#267's `on_page_moved`) and on `visualViewport` changes (the phone keyboard), and decides once what to do when the anchor is off screen. `pen_pos` goes away. Every state change goes through `mascot::transition`.
- **Interruptions are events.** A second capture, a tap, a drag, a scroll or a page turn is a `mascot::Event`. Writing is a queue of rows with progress, so it pauses and resumes from where it stopped; a reload shows stored titles in full. Scroll or page turn while he is out of the nest is the event that smacks or flattens him (DESIGN-DIRECTION §9).
- **Rambo's element.** Two elements: an outer button that is only placed, an inner one that is only posed (rotate, scale, splat, art). His rim at rest is a clip on the inner one.
- **His art follows the page theme**, not the OS: the drawings use the page's `--rambo-*` tokens. The real dish replaces the `nest.svg` placeholder; Ping redraws all the art (DESIGN-DIRECTION §9).
- **Tuning.** `web/src/tuning.rs` holds every motion and gesture constant with its reason, grouped by DESIGN-DIRECTION §4's beats. The CSS gets the times as `--t-*` custom properties written onto `:root` from that list, and uses no raw times.
- **CSS.** Cascade layers declared once (`@layer tokens, layout, rows, planner, tack, dialogs, rambo, overrides;`), every rule inside a layer. z-index as named tokens with gaps, ink always under the capture line and dialogs. One phone breakpoint. Splitting into files is optional once layers hold the order.
- **One animation clock and one gesture module.** A rAF ticker that every motion registers with (with a fixed-step mode for the tack physics, and `reduced_motion()`); non-animation timers (the 60s now-line, the 5s brain poll) stay. A gesture module with pure tests for slop, claim and release by pan direction, `pointercancel` (real touch ends with it once the browser takes a pan, as #210 proved), Esc and velocity; it sets `touch-action` per surface.
- **Files under about 600 lines.** `tack.rs`, `week.rs` and `tasks/mod.rs` split by job, move-only.

### Rules for every ticket

- **Design PRs carry pictures.** Any PR that changes what the page looks like gets before and after screenshots from the test site attached to it. GLM builders cannot take them, so Emilio or Antonio adds them during review. After the merge, god lists the PR in the hive's `REVIEW-QUEUE.md` for the owner.
- **Claude agents use the design skills.** A Claude agent on an FE ticket invokes `/emil-design-eng` for motion, timing, easing and gestures, `/frontend-design` for anything visible, and `/color-expert` for paper and ink colours in light and dark. The reviewer uses the same skills.
- Read `AGENTS.md`, then only the files the ticket lists. Touch only those files. If the work needs another file, stop and ask god.
- Own worktree and branch: `git worktree add ../ai-todo-wt/<you>-<ticket> origin/main -b <you>/<ticket>`.
- One PR per ticket. The title starts with the ticket id, for example `FE-3: one breakpoint, folded rules`.
- Run the full gate once at the end and paste the results (fmt, clippy `-D warnings`, `cargo test --workspace`, the wasm32 check, `trunk build --release`).
- **Visual tickets** (any CSS or art change): a pixel diff, not eyeballing. Take four screenshots on `main` and four on the branch, on the same machine with the same fonts: 1366x800 and 390x844, each in light and dark. Report for each pair either "identical" or the share of pixels that differ. Over 0.1% must be a change the ticket asks for, and the PR names it.
- **Behaviour tickets**: add a test that fails without the change, and say in the PR that you checked it fails.
- **Guard tests stay green.** `mascot.rs` holds tests that read the source and CSS as text (`rambo_rules` and the ones these tickets add). Never weaken one; a ticket that needs to may not go ahead without Emilio.
- **Art lives in asset slots** (DESIGN-DIRECTION §9). Code and CSS name a slot's stable file, `web/assets/<slot>/<name>`, never a source's own file name. No art is drawn in CSS except as the fallback under a slot. A ticket that adds or changes art also writes or updates the slot's `ABOUT.md`, the `web/assets/README.md` index and `web/assets/CREDITS.md`.
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
  - a long press listens; a plain drag does not
  - release: no jump
  - dazed drag writes
  - put back in the nest
  - dazed for about 3s, then home
  - scroll or turn the page while he is out: smacked or flattened, then he recovers (once that step lands)
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
- Ping is redrawing all the art (DESIGN-DIRECTION §9). This ticket sets the contract the new drawings follow: colours only through `--rambo-*` tokens, the root group `id="art"`, no media queries. Converting today's files now fixes the bug until the new art lands.
- Note: `nest.svg` is a placeholder ("PLACEHOLDER: replace me") in the old brown palette, not the dish in DESIGN-DIRECTION §2. Ping draws the real dish; until it lands, no ticket styles the placeholder.

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

**FE-4: every Rambo state change through `transition`** · Rust · small
- Why: the owner's answers add interruptions, pause and resume, and the smack and flatten. All of them are new events, so every state change must already go through the state machine.
- Files: `web/src/mascot.rs`, `web/src/rambo.rs`, `web/src/tasks/mod.rs`.
- Do: `grep -rn "\.set(mascot::State::\|\.set(State::" web/src` finds nine places that set a `State` directly (seven in `rambo.rs`, two in `tasks/mod.rs`). Replace each with `set_state.set(mascot::transition(&before, mascot::Event::X))`. Where no event fits, add one to `mascot::Event` with a doc comment and its arms in `transition`. Behaviour must not change.
- Accept:
  - A guard test in `mascot.rs` reads `rambo.rs`, `tasks/mod.rs`, `tasks/write.rs` and `capture.rs` with `include_str!` and fails on any direct `.set(…State::…)`. Checked to fail before the change.
  - Each new event or arm has a test in `mascot.rs` (input state and event, then the expected state).
  - FE-1's check list passes for drag, drop, put-back, going home and capture-then-write.
- Do not touch: CSS, `pen_pos`, timers, the daze time.
- Done in #286. The owner's notice beat is its own later step (2.4, "The reaction beat").

**FE-5: dead code and undefined colours** · CSS + Rust · small · after FE-3
- Why: the cloud audit (1.8, items 21–24) found code nothing calls and colours nothing defines. Less code means less for every later ticket to read.
- Files: `web/theme.css`, `web/tack.css`, `web/src/api.rs`, `web/src/mascot.rs`, `web/src/week.rs`, `web/src/tasks/mod.rs`, `web/src/edits.rs`, `web/src/lib.rs`.
- Do:
  - Delete the CSS rules for selectors no element uses: `.mic`, `.task-editor`, `.delete`, `.import`, `.swipe-deleted`, `.swipe-snapped`, `.rambo.pose-teasing`, `.pose-dozing`, `.pose-happy`, `.pose-sulking`, and the `prefers-color-scheme` block in `tack.css`. Before deleting each one, grep `web/src` for the class name, including in `format!` strings, and keep any that is used.
  - Delete the unused Rust: `api::use_snapshot`, `api::revise_task`, `api::capture_file`, the `PatchTaskBody::label` and `source` builders, and the `on:drop`/`on:dragover` handlers in `week.rs`. Leave `mascot::is_away` and the native-only stubs alone unless clippy or the wasm32 check pass without them.
  - Fix the undefined colours: the placement grip's `var(--coffee)` becomes `var(--ink)` (the coffee colour). In `tack.css`, `--tack-ink` and `--tack-accent` use `var(--ink)` and `var(--accent)` directly, which is what their fallbacks give today. Add a dark value for `--fold-shadow`, and make the `.error` background a token.
  - Make the body's bottom padding match the phone capture bar's height.
- Accept:
  - For each deleted selector and function, the PR lists the grep that shows nothing uses it.
  - Pixel diff: identical in light, apart from the placement grip, which now shows. Dark differs only where the new dark values apply, and the PR names them.
  - Gate green, with the wasm32 check.
- Do not touch: the planner drop behaviour (dropping Rambo on a planner block is a later fix), anything the audit did not name.

### Queue

Who has which ticket right now is in the hive's `STATUS.md`; this list is the order, as of 2026-10-14. While GLM agents build, only one builds at a time (owner, interview 2). They are out until 2026-10-20, so Claude and other agents take tickets in parallel meanwhile, each on its own files.

- Done: FE-0 (#267), FE-3 (#282), FE-4 (#286).
- In review or building: FE-5 (#289), FE-2, BUG-1 to BUG-3 (one agent, in turn, as they share `tack.rs`), BUG-4, BUG-5, BUG-6, BUG-7 (#288), and on the cloud session (issue #259): BUG-9, the port of v1's writing, then the reaction beat.
- Next: BUG-8, then the later steps in 2.4.
- Waiting for the picked assets: the paper and nest art. Dario sources candidates (hive `ASSETS-PLAN.md`). Ping's concept is saved on `ping/paper-concept` (342c4e2, `art/previews/paper-concept.html`); the art step builds on the chosen assets, in asset slots.

**BUG-9: the planner cannot scroll forward to every day** · Rust or CSS · small
- The bug (owner's phone test, on b005174): on the phone the planner does not scroll right far enough to show Friday.
- Files: `web/src/week.rs` and the planner rules in `web/theme.css`. Find the cause first and say it in the PR; if it lies elsewhere, stop and ask god.
- Do: every day the planner shows can be scrolled to, at 390x844 with touch and at 1366x800.
- Accept:
  - A test where the cause can be tested in Rust (for example a width calculation); otherwise the PR shows how the cause was found.
  - Before and after screenshots at 390x844 with the last day in view. Emilio checks them with the pixel-diff script.
  - Scrolling back before today stays as it is.
- Do not touch: the day count setting, the planner's look.

### 2.4 Later steps: intent only

Each becomes a ticket when the steps before it have landed, written with what they taught us. The order is a guess, and so is the split into steps. Fable's review of #268 and the owner's answers (DESIGN-DIRECTION §9) are the guidance.

- **Tuning.** Constants into `web/src/tuning.rs`, following `docs/TUNING.md` (#270, one `tuning.rs` per crate), then the CSS times as `--t-*`, each with a guard test. The daze becomes about 3s here. Where CSS and Rust disagree today (`FLY_MS = 450` against the CSS flight), Rust wins. The reaction beat's length lives here too, as `NOTICE_BEAT_MS`.
- **z-index tokens.** Name the stacking, a guard test that every `z-index` is a token, and put Rambo under dialogs (answered). Note in THEME.md that a `showModal()` dialog sits above every token.
- **Cascade layers.** Wrap every rule in its layer, keeping today's order where rules compete; pixel diff identical.
- **Two-element Rambo** at 1.4x, with the rim clip. Paired review with Emilio.
- **His state says where he is, then one resolver**, including the keyboard ride-up (`visualViewport`). Probably two PRs. Paired review.
- **Interruptions:** writing as a pausable queue, every user action an event, the smack and the flatten (with the hole in the paper). Needs the resolver.
- **Writing feel:** slower and more considered, in v1's feel (UI-BUGS #11, v1 `297dd44`). Times come from tuning, set with the human watching.
- **Listening:** the pose, riding above the finger, the page dimmed.
- **Phone:** list page first, capture bar on the planner page, a + button.
- **The notebook look** (with Ping's art): one notebook page per page, the gear in the paper's corner, handwriting-style wrapping, tiny symbolic tags. These replace the "desktop widths" step: wrapping fixes the squeezed column.
- **Capture box: tap Rambo to submit, and the paper cut-out × to clear it.** Tapping him with text in the box submits it. The × slides out beside the box; tapping it makes him scribble the text out and lays down fresh paper.
- **The capture box as pinned scratch paper** (DESIGN-DIRECTION §9): pinned on desktop as on the phone, drawn as a ripped scrap (Ping's art), with Rambo writing on it as you type and a fresh scrap laid down on submit. Fits with the tap-to-submit and x step.
- **A real page turn:** the paper bends and curls (a CSS 3D fold, or a mesh of strips driven by the ticker), with no gradient popping in and out.
- **A physical dark-mode switch:** the day page is swapped for a night page.
- **Sound engine and presets A, B and C** (DESIGN-DIRECTION §9): one module that plays named sounds, a preset choice in the settings, the mute in the gear menu.
- **Preset C: music and dancing.** Calm classical background music from CC0 or public-domain recordings, and Rambo dancing to its beat. After the sound engine.
- **Full calendar view** (owner: later): scrolling back in time and further ahead.
- **Rambo's ask bubble does what it says** (2.6): Edit, Move, the planner-slip choices and the empty-hour task, one ticket each.
- **Scribble-out delete** (DESIGN-DIRECTION §9, Gestures, final): swipe right on a row draws Rambo's scribble over the words, growing with the finger's travel; past `SWIPE_PX` (`row.rs`) the row is deleted, short of it the ink lifts away. It replaces today's slide-away (`.swipe-deleted`) and keeps `row.rs`'s gesture rules. Accept: a pure test of how much scribble shows for a given travel, and that the delete threshold is unchanged.
- **Planner swipes** (with or after BUG-9): on the planner page, swipe left scrolls to later days; swipe right inside the grid scrolls back, and outside the grid turns the page back to the list. Accept: a pure test that maps the swipe's direction and where it started (inside or outside the grid) to scroll or turn.
- (cut 2026-10-15, ARCHITECTURE §8A.3) **The page rips:** a left-to-right swipe on the list page that does not start on a task tears the page edge a little. Rambo walks over and tapes it, and his face stays grumpy for a while; the tape and tear fade after a while. Tear and tape art in asset slots; the times in the tuning file. Accept: a pure test that a swipe on a task deletes and one off a task rips; the tear never covers a row's words.
- **The reaction beat** (done: `mascot::notice`; every move goes through the same mover): every Rambo move (skitter home, fly to a row, go to write) starts with a beat: he notices, his expression changes, then he moves. One place in `mascot.rs` adds the beat in front of every move, with its length in `web/src/tuning.rs` as `NOTICE_BEAT_MS`, and a guard test that no move starts without it. After FE-4, since every change of state goes through `transition` there. The interruption step (writing pauses and resumes) must keep the beat short enough that capture never feels slow.
- **Scrap undo** (with the scribble-out): the deleted row's scrap lies in the page corner for a few seconds; a tap tapes it back and the task returns as it was. Accept: a test that undo restores the task unchanged, and that the scrap goes after its time.
- **Ink dries:** a new row is glossy for a few seconds after Rambo writes it; a tap in that time smudges it. Accept: the gloss time is a tuning constant; the smudge is only a look and never changes the task.
- **The nib runs dry:** after a long run of rows, Rambo taps his nib on the dish's rim before the next. Accept: a pure test of when the tap comes (a tuning count of rows); it never delays a capture by more than the tap.
- **Bookmark ribbon:** a cloth ribbon down the planner's edge marks today; pulling it scrolls the planner back to today. Art in an asset slot. Accept: the pull lands on today at both screen sizes.
- **Desk-lamp theme switch:** a small lamp in the page corner replaces the theme control; pulling its chain lays the night page down (the physical dark-mode switch). Art in an asset slot. The theme still never follows the OS.
- **One animation clock, then one gesture module.**
- **Split the big files**, move-only, last.

Not front end, already with others: stripping lead-ins such as "Add a task to…" from titles (Dario, capture and Jev).

### 2.5 Bug tickets from the audit

Small functional bugs found by the cloud audit (1.8). Each ticket is one PR, follows the rules for every ticket above, and starts with a test that fails before the fix. None waits for the restructure. Line numbers are against main `ae22919`; find the code by the names given if they have moved.

BUG-1 and BUG-2 both change the snapshot effect in `tack.rs`: give them to one agent, or run BUG-2 after BUG-1 lands.

**BUG-1: done strips are never pinned** · Rust · small
- The bug: the snapshot effect in `tack.rs` (about line 652) builds `listed` (the to-do ids) from the *same* snapshot as `done`, so a task that was just ticked is never in both, `reconcile` returns an empty `just_done`, and the "cut out and tack" moment (the swing-in, the pin sound, `Cue::CutAndTack`) never happens. `reconcile`'s own doc (about line 282) says `listed` is what the *previous* snapshot showed as to-do.
- Files: `web/src/tack.rs`.
- Do: keep the previous pass's to-do ids and pass those to `reconcile`. Put the rule in a small pure type next to `reconcile`, for example `struct Watch { listed: HashSet<String> }` with `fn pass(&mut self, strips, snapshot) -> Pass`, which uses the stored `listed` and then stores the new one. The effect only calls it.
- Accept:
  - A test: a `Watch` sees snapshot 1 with task "a" open, then snapshot 2 with "a" done, and `just_done` is `["a"]`. A task already done in the first snapshot is not in `just_done`. Checked to fail when `listed` comes from the current snapshot.
  - On the site: tick a task; after the pause its strip swings onto the tack board.
- Do not touch: `reconcile` itself, the physics, CSS.

**BUG-2: saved strip positions are wiped on load** · Rust · small · after BUG-1
- The bug: the page starts with an empty snapshot (`empty_snapshot()` in `lib.rs`). The tack board's snapshot effect runs on it before the real load, finds no done tasks, keeps no strips, and the save effect writes that empty board to localStorage. Every reload loses where the user left their strips.
- Files: `web/src/tack.rs`, `web/src/lib.rs`.
- Do: the board does not reconcile, and does not save, until the first real snapshot has arrived. `lib.rs` already knows that moment (the `Ok(loaded)` arm of the first fetch); give `TackBoard` a `loaded: Signal<bool>` and have both effects wait for it.
- Accept:
  - A pure test: the stored strips survive a pass over the empty first snapshot (for example a `Watch::pass` that is given `loaded = false` returns the strips unchanged and says not to save).
  - On the site: move a strip off the tack, reload, and it is where you left it.
- Do not touch: the storage format.

**BUG-3: repeat-day strips cannot be crumpled or restored** · Rust · small
- The bug: a done occurrence of a repeating task has the id `"{series}:{date}"`. Crumpling its strip calls `api::delete_task` with that id (`tack.rs` about line 995) and restoring it calls `toggle_done_body` and `patch_and_show` (about line 923). The server answers 400 to both. The list already does this right: `delete_with_scribble` skips the occurrence, and `tick_action` / `apply_tick` clear its done mark (`tasks/mod.rs`).
- Files: `web/src/tack.rs`.
- Do: crumple goes through the same path as `delete_with_scribble` (an occurrence is skipped, a plain task deleted), and restore through `tick_action` + `apply_tick`, instead of building its own requests.
- Accept:
  - A test on the pure part: for an occurrence id the board chooses the occurrence action (skip on crumple, clear done on restore); for a plain id, delete and the task PATCH.
  - On the site, with a task that repeats daily, ticked done: crumple its strip and the day is skipped; restore another and it is open again. No error note.
- Do not touch: `api.rs`, the list's own paths.

**BUG-4: planner labels never refresh** · Rust · small
- The bug: in `week.rs` the day headers (`day_label(...)`, about line 180) and the hour labels (`clock::format_time(..., settings.get().clock)`, about line 241) are plain values inside `<For>`, so they are computed once. After midnight the first column still says yesterday, and switching the 12/24-hour setting leaves the old labels.
- Files: `web/src/week.rs`.
- Do: make both labels reactive closures (`move || ...`). The day header must also depend on the date, so read the planner's existing one-minute "now" signal (`now_minutes`) in it, so that the header changes when the day does.
- Accept:
  - A pure test for the label function: the same day offset gives the next day's name once "now" crosses midnight.
  - On the site: switch the clock setting in the gear menu and the hour labels change at once.
- Do not touch: the planner layout, CSS.

**BUG-5: Rambo reacts to "task added" on every page load** · Rust · small
- The bug: `brain.rs` keeps `previous` starting from the empty first snapshot (about line 109) and calls it `TaskAdded` whenever there are more tasks than before (about line 152). So the first load, a wider date window, or new repeat-day rows all make him react.
- Files: `web/src/brain.rs`.
- Do: compare ids, not counts, and only after a real snapshot. `TaskAdded` fires when the current snapshot has a plain task id (not a repeat-day occurrence) that the previous *loaded* snapshot did not. The first loaded snapshot never fires it. Put the rule in a pure function beside `overdue_changes`, with tests.
- Accept:
  - Tests: first load fires nothing; one new plain task fires `TaskAdded`; new occurrence rows only fire nothing; a deleted plus an added task (same count) fires `TaskAdded`.
  - On the site: reload the page and Rambo says nothing about a new task.
- Do not touch: the other events, the brain crate.

**BUG-6: one date window for every snapshot** · Rust · small
- The bug: the first load and the reload after a refused edit ask the server for `settings.days` days (`lib.rs` about line 69, `edits.rs` about line 126). Every edit response uses the server's own default window. Changing the days setting (`settings.rs` about line 333) never re-fetches. With days = 3, repeat-day rows and their strips come and go depending on which call ran last.
- Files: `web/src/lib.rs`, `web/src/edits.rs`, `web/src/api.rs`, `crates/core/src/tuning.rs` (from #270; `crates/core/src/lib.rs` if #270 has not landed), and one line in `server/src/v2/http.rs` (an allowed exception to "no server code": the import only).
- Do: every fetch asks for the same window as the server's default, `DEFAULT_WINDOW_DAYS = 7` (`server/src/v2/http.rs` about line 980). Move that constant, unchanged, to core's tuning file with a one-line reason, so the server and the page share it. Every `fetch_snapshot` call in the page uses it. The days setting then only chooses how many planner columns show, so changing it needs no fetch.
- Accept:
  - `grep -n "fetch_snapshot(" web/src` shows only calls that use the one constant.
  - The server's tests pass unchanged, and `grep -rn "DEFAULT_WINDOW_DAYS" server crates web` shows one definition, in core.
  - On the site with days = 3: tick a repeat-day task; the list keeps the same rows before and after.
- Do not touch: any other server code, the planner's column count.

**BUG-7: the right message for a bad time** · Rust · small
- The bug: `typed_when` in `tasks/fields.rs` (about line 206) answers every failure with "That time does not exist on this day (clocks change)", including an empty field and text that is not a time.
- Files: `web/src/tasks/fields.rs`.
- Do: tell three cases apart: empty ("Pick a time"), not a time ("That is not a time"), and a time skipped by a clock change (the message today). Parse first, then resolve the local time, and map each failure to its message.
- Accept: a test for each of the three, plus a valid time that still works.
- Do not touch: the dialog's look.

**BUG-8: one occurrence parser, one move path, one overdue rule** · Rust · small · three commits
- The bug: the same rules are written twice and have drifted.
  - Occurrence ids are parsed by `occurrence_parts` (`tasks/mod.rs` about line 93, `split_once(':')`) and `occurrence_of` (`week.rs` about line 476, `rsplit_once(':')`).
  - A planner move (`week.rs` about line 787) calls `patch_task` itself instead of `patch_and_show`, so a stale move shows raw error text and does not reload.
  - The overdue rule is in `brain.rs` (`overdue_changes`) and in `tasks/mod.rs` (`is_overdue_at`).
- Files: `web/src/tasks/mod.rs`, `web/src/week.rs`, `web/src/brain.rs`, `web/src/edits.rs`.
- Do:
  - One parser, `rsplit_once(':')`, since a date never contains a colon, as `pub(crate) fn occurrence_parts` in `edits.rs`. Both callers use it; delete the other.
  - The planner's plain-task move calls `patch_and_show`.
  - `brain.rs` uses `tasks::is_overdue_at`.
- Accept:
  - The parser's tests cover a plain id, an occurrence id, and a series id that contains a colon.
  - `grep -rn "split_once(':')" web/src` finds only time parsing.
  - On the site: moving a slip on the planner works as before. Two tabs, move the same task in both: the second shows the reload note, not raw error text.
- Do not touch: the Overdue badge's look.

### 2.6 Rambo's ask bubble: answered (DESIGN-DIRECTION §9, interview 3)

Rambo's ask bubble (dropping him on a task) offers Edit, Move, Done and Never mind. Today Edit only changes the priority and Move only sends him home (`rambo.rs` about line 450). Dropping him on a planner block does nothing, because those handlers are for HTML drag and drop, which he does not use. The owner's answers become later steps (2.4), each a small ticket once the two-element and gesture steps have landed:

- **Edit:** opens the edit sheet for the task and starts listening; a spoken change goes through Jev like a capture and is applied to that task.
- **Move:** turns the page to the planner and waits for a tap on an hour; the bubble also offers the next free time as one button.
- **Dropped on a planner slip:** a bubble with time choices: Move, Longer, Shorter, Off the planner. Needs a hit test for slips, as `row_under` is for rows.
- **Dropped on an empty hour:** a new task at that time, and he listens; a tap or click elsewhere cancels.

Also decided: the bubble keeps Edit, Move, Done and Never mind, with no Delete; deleting is the scribble-out (2.4).

The gestures are final too (DESIGN-DIRECTION §9, Gestures, final): swipe right on a task is the scribble-out delete, swipe left turns to the planner, and on the planner the swipes scroll inside the grid and turn back outside it. Nothing in 2.6 is open.

## 3. Not checked

- Phone width and touch were not reproduced in a browser (the window would not shrink below 905px here); the off-pane flight is read from code and layout. QA should confirm FE-0 on a phone.
- That content shown through `<use>` picks up the page's `--rambo-*` tokens (FE-2) is how browsers specify it; the ticket's screenshots are the check.

## 4. Spec questions: answered

The owner answered the twelve questions on 2026-10-14. The answers are in DESIGN-DIRECTION §9 and win over this plan. Two are open.

1. **Desktop capture box: decided.** Pinned, as a ripped scrap of scratch paper that Rambo writes on (DESIGN-DIRECTION §9).
2. **Motion and sound: decided.** The owner chose all three, as presets in the settings (DESIGN-DIRECTION §9); C became the most immersive, with music and dancing. The options as first put: All keep every motion; reduced motion still makes them instant. Sound stays on by default with a mute in the gear menu (§8).
   - **A. Quiet desk.** Sound only where paper or ink really makes a noise: the scratch while the nib moves, the page turn, the crumple, the pin, the smack and the flatten. Flights, hops, idle breathing and the daze are silent. Calm, and never tiring after a hundred tasks.
   - **B. Every touch answers.** Each motion has its own soft sound: pick-up, the flight's whoosh, the landing tap, the scratch, the flick at the end, the splat, the smack, the page turn, the pin, the crumple. The most alive, and the most to tune and to tire of.
   - **C. Writing only, plus the hits.** The scratch while writing, the page turn, and the smack and flatten, because those are the moments that need a sound to read. Everything else is silent. The smallest set to build and keep in sync.
   - Recommended: **A**, which matches "near-physical": what would make a sound on a real desk does, nothing else.
