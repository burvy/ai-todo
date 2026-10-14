# Front-end plan: where `web/` stands and how to restructure it

Written 2026-10-14 by Emilio (front-end owner) after the human's report on the test site (fa48124): *Rambo still moves with his nest and flies to the top left*. Part 1 is what I found. Part 2 is the plan as tickets for GLM agents, each one PR that leaves the app working, reviewed by Emilio. `docs/DESIGN-DIRECTION.md` stays the authority on look and feel; this file is about structure.

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

- **Position.** `pen_pos` is one `RwSignal<(f64, f64)>` written from three modules: `rambo.rs` (drag, home, skitter), `capture.rs` (caret) and `tasks/write.rs` (writing). Nobody owns "where is he", so each writer has to know the others' rules. The bugs above are what happens when one writer forgets the page can move. Fixed by FE-13 and FE-14.
- **Mood state.** `mascot::transition` is the documented way to change state, but nine call sites set a `State` directly (`set_state.set(mascot::State::Home)` and similar) in `rambo.rs`, `tasks/mod.rs` and `capture.rs`, so the machine's tests do not cover every path the page takes.
- **Timing.** Animations run on a mix of `setInterval(16ms)`, chained `Timeout`s with ticket counters, CSS transitions and CSS keyframes. Background tabs throttle all of them differently. The writing loop and the skitter each keep their own clock.

### 1.4 The Rambo/nest layering and positioning model

Rambo is one `position: fixed` button at `top: 0; left: 0`, moved only by `transform: translate(calc(var(--pen-x) - 50%), calc(var(--pen-y) - 50%))`. The model is right (it is the standing rule: Rambo independent of the fixed nest), but it is fragile in CSS. 15 `.rambo…` rules each restate the whole translate before adding a rotate or scale, and any rule that sets `transform` without it teleports him to the corner. A test now guards this and the `:active` rule of 1.1 (`mascot::tests::every_state_of_the_button_keeps_him_where_he_is`), and it stays. The lasting fix is two elements: an outer one that is only placed, and an inner one that is only posed (FE-12).

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

## 2. Plan: tickets for GLM agents

The human wants the restructure built by GLM agents, from a spec written by us and reviewed carefully by a Claude agent. So each step below is a self-contained ticket that one GLM agent can finish in one PR without reading anything else. Emilio (front-end owner) is the architect and reviewer, not the builder: he reviews every PR against its ticket before god merges. Tickets wait for the human's answers to section 4 where marked. Fable's design review of this plan (#268) is folded in.

### Rules for every ticket

- Read `AGENTS.md`, then only the files the ticket lists. Touch only those files. If the work needs another file, stop and ask god.
- Own worktree and branch: `git worktree add ../ai-todo-wt/<you>-<ticket> origin/main -b <you>/<ticket>`.
- One PR per ticket. The title starts with the ticket id, for example `FE-6: z-index tokens`.
- Run the full gate once at the end and paste the results (fmt, clippy `-D warnings`, `cargo test --workspace`, the wasm32 check, `trunk build --release`).
- **Visual tickets** (any CSS or art change): a pixel diff, not eyeballing. Take four screenshots on `main` and four on the branch, on the same machine with the same fonts: 1366x800 and 390x844, each in light and dark. Report for each pair either "identical" or the share of pixels that differ. Over 0.1% must be a change the ticket asks for, and the PR names it.
- **Behaviour tickets**: add a test that fails without the change, and say in the PR that you checked it fails.
- **Guard tests stay green.** `mascot.rs` holds tests that read the source and CSS as text (`rambo_rules` and the ones these tickets add). Never weaken one; a ticket that needs to may not go ahead without Emilio.
- Never edit `docs/DESIGN-DIRECTION.md`. Never add a crate. Never change server code.
- Respect the standing rules: Rambo stays a separate fixed layer, independent of the nest. No blue focus or tap highlight. No native browser controls or scrollbars. The theme never follows the OS (DESIGN-DIRECTION §7).
- The reviewer (Emilio) checks each acceptance item one by one. A PR that misses one goes back.

### Tickets

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
  - Show it with `<svg><use href="assets/rambo/<tool>/<pose>.svg#art"/></svg>` instead of `<img>` or a background. Content shown through `<use>` inherits CSS custom properties from the page, so `--rambo-*` from `:root[data-theme]` reaches it.
  - The nest is a CSS background since #267; draw it the same way, as an `<svg><use>` inside `.nest`.
  - Add any missing `--rambo-*` dark values to the dark-theme block of `theme.css`, taken from the SVGs' old media blocks.
- Accept:
  - `grep -rln "prefers-color-scheme" web/assets` finds nothing.
  - A test in `mascot.rs` checks that `Pose::art` returns a `…svg#art` reference for every pose and tool.
  - Pixel diff with the OS in light mode: page Light and page Dark both identical to main.
  - Screenshots with the OS in dark mode and the page on Light: Rambo and the dish are the light drawing.
- Do not touch: the drawings' shapes, and any colour that has no `--rambo-*` token (ask instead).
- Note: `nest.svg` is a placeholder ("PLACEHOLDER: replace me") in the old brown palette, not the dish in DESIGN-DIRECTION §2. The real dish is art for Ping. God names the owner; until it lands, no ticket styles the placeholder.

**FE-3: one phone breakpoint** · CSS · small
- Files: `web/theme.css`.
- Do: change `@media (max-width: 720px)` (the delete mark) to `@media (max-width: 700px)`, so every phone rule switches at the same width as the layout.
- Accept:
  - `grep -n "@media (max-width" web/*.css` shows only `700px`.
  - At 710px the desktop layout shows the desktop delete control, not the phone one.
- Do not touch: anything else.

**FE-4: fold the appended rules** · CSS · small
- Files: `web/theme.css`.
- Do: move the one-line rules appended at the end of the file (swipe, delete mark, lifted row, about 14 lines after the reduced-motion block) into the sections they belong to, written in the file's normal multi-line style. Delete any rule that an earlier rule already fully repeats.
- Accept:
  - Pixel diff: all four identical.
  - Every rule's selector still appears exactly once per media block.
  - The file ends with its last section, not with loose rules.
- Do not touch: values. This is a move, not a redesign.

**FE-5: every Rambo state change through `transition`** · Rust · small
- Files: `web/src/mascot.rs`, `web/src/rambo.rs`, `web/src/tasks/mod.rs`.
- Do: `grep -rn "\.set(mascot::State::\|\.set(State::" web/src` finds nine places that set a `State` directly (seven in `rambo.rs`, two in `tasks/mod.rs`). Replace each with `set_state.set(mascot::transition(&before, mascot::Event::X))`. Where no event fits, add one to `mascot::Event` with a doc comment and its arms in `transition`. Behaviour must not change.
- Accept:
  - A guard test in `mascot.rs` reads `rambo.rs`, `tasks/mod.rs`, `tasks/write.rs` and `capture.rs` with `include_str!` and fails on any direct `.set(…State::…)`. Checked to fail before the change.
  - Each new event or arm has a test in `mascot.rs` (input state and event, then the expected state).
  - FE-1's check list passes for drag, drop, put-back, skitter home and capture-then-write.
- Do not touch: CSS, `pen_pos`, timers.

**FE-6: z-index tokens** · CSS + test · small · mechanical
- Files: `web/theme.css`, `web/tack.css`, `web/src/mascot.rs` (test only), `docs/THEME.md`.
- Do: add z-index tokens to `:root` in `theme.css` and replace every raw `z-index` number in both files with one. The tokens keep today's stacking (section 1.5). Ink stays at page + 1, under the capture line and every dialog, as the comment in `theme.css` and DESIGN-DIRECTION say; never above them. Leave gaps of 10 between groups so a later layer fits without renumbering.
  - `--z-ink: 1; --z-sheet: 2; --z-capture: 3; --z-days: 4; --z-tack: 5; --z-menu: 10; --z-dialog: 20; --z-note: 40; --z-rambo: 50; --z-bubble: 51; --z-drag: 60; --z-bin: 70`.
  - `z-index: -1` inside a single element may stay.
  - Add the scale as a table to `docs/THEME.md`, with one sentence: the task dialog opens with the `open` attribute, so these tokens apply to it; a `showModal()` dialog sits in the browser's top layer above every token.
- Accept:
  - A guard test in `mascot.rs` reads `theme.css` and `tack.css` and fails on any `z-index:` that is not `var(--z-*)` or `-1`. Checked to fail before the change.
  - Pixel diff: all four identical.
- Do not touch: the order of Rambo, menus, dialogs, pin and bin. That is FE-10, after the human answers question 4.

**FE-7: one tuning file for the web: Rust side** · Rust · medium
- Files: new `web/src/tuning.rs`, `web/src/lib.rs` (module line), and the modules whose constants move: `mascot.rs`, `pen.rs`, `row.rs`, `tack.rs`, `voice.rs`, `rambo.rs`, `tasks/write.rs`, `week.rs`.
- Do: move every motion and gesture constant (durations, distances, slop, scales, speeds) into `tuning.rs`, one `pub const` each, with a one-line comment on why it has that value. Group them under headings that follow DESIGN-DIRECTION §4's beats (rest, pick up, flight, writing, drop, skitter, page). Follow the convention of `docs/TUNING.md` (Claudio, in progress for the server and crates); if it has landed, add a "web" section to it pointing at `tuning.rs`. Values do not change.
- Accept:
  - Each moved constant is removed from its old module, which now reads `tuning::NAME`.
  - `grep -rn "const .*: \(u32\|u64\|f64\|i32\|f32\) =" web/src --include=*.rs` outside `tuning.rs` finds only non-tuning constants (sizes of arrays, ids); list any left in the PR with a reason.
  - All tests pass unchanged.
- Do not touch: CSS (FE-8), behaviour.

**FE-8: one tuning file for the web: CSS side** · Rust + CSS · medium · after FE-7
- Files: `web/src/tuning.rs`, `web/src/lib.rs`, `web/theme.css`, `web/tack.css`, `web/src/mascot.rs` (test only).
- Do: on mount, `App` writes each tuning value the CSS needs onto `:root` as a custom property: `--t-fly: 450ms`, `--t-splat: …`. `tuning.rs` holds one list, `CSS_VARS: &[(&str, …)]`, that is both the source of the writes and what the test reads. Every duration and delay in `transition` and `animation` declarations then uses `var(--t-*)`, including the ones today written as `--dur-slow`, `400ms`, `.18s`, `.25s`. Where CSS and Rust disagree today (for example `FLY_MS = 450` against the CSS flight), the Rust value wins; list each one in the PR.
- Accept:
  - A guard test in `mascot.rs` fails on any time value (`ms` or `s`) in a `transition`, `animation` or `*-duration`/`*-delay` declaration that is not `var(--t-*)`, and on any `--t-*` used in CSS that `CSS_VARS` does not define.
  - Pixel diff: identical at rest. Each motion that changed speed because of a fixed mismatch is named in the PR.
- Do not touch: anything other than times and the tuning list.

**FE-9: CSS cascade layers** · CSS · medium · after FE-4, FE-6 and FE-8
- Why: the `:active` bug in 1.1 and the appended one-liners in 1.6 are both "a later rule wins" problems. Cascade layers make the order explicit: a rule in an earlier layer cannot beat a state rule in a later one, wherever a later PR appends it.
- Files: `web/theme.css`, `web/tack.css`, `web/src/mascot.rs` (test only), `docs/THEME.md`.
- Do: declare `@layer tokens, layout, rows, planner, tack, dialogs, rambo, overrides;` once, as the first line of `theme.css`. Wrap each existing section, unchanged, in `@layer <name> { … }`, and all of `tack.css` in `@layer tack { … }`. The two `prefers-reduced-motion` blocks are last in the file today, so they go in `overrides`. The layer order must keep every pair of rules that compete for the same element in the order they have today; the pixel diff checks it. Rules outside any layer would beat every layer, so none may stay outside. Splitting into several files is not part of this ticket.
- Accept:
  - A guard test in `mascot.rs` fails on any rule outside an `@layer` block in either file.
  - Pixel diff: all four identical.
  - `docs/THEME.md` lists the layers and what goes in each.
- Do not touch: any rule's content or selector.

**FE-10: layer order** · CSS · small · waits for question 4
- Files: `web/theme.css` (the `:root` tokens only), `docs/THEME.md`.
- Do: reorder the `--z-*` values to the human's answer. Proposed: Rambo and his bubbles under the gear menu, dialogs and the page note; the tack pin and bin under Rambo.
- Accept:
  - With the gear menu open, the menu covers Rambo. With a task dialog open, the dialog covers Rambo.
  - The tack pin and bin draw under Rambo when he is dragged over them.
  - Pixel diff with nothing open: identical.
- Do not touch: anything but the token values and the table.

**FE-11: desktop desk widths** · CSS · small · waits for question 5
- Files: `web/theme.css` (layout layer), `web/tack.css`.
- Do: in the `min-width: 701px` layout, give the list column a minimum of `18rem`, and let the tack board shrink first (to `8rem`), then the planner. Adjust to the human's answer.
- Accept:
  - At 905, 1024, 1100 and 1366px a title like "Send the invoice to Bob" stays on one line, and its "30 min · Wed 8:30" line stays below it.
  - Nothing scrolls sideways at any width from 701 to 1920px.
  - This PR's screenshots add 905x800 and 1024x768 to the standard four.
- Do not touch: the phone layout (`max-width: 700px`).

**FE-12: two-element Rambo** · Rust + CSS · medium · Emilio pairs on review · after FE-5 and FE-9
- Files: `web/src/rambo.rs`, `web/theme.css` (rambo layer), `web/src/mascot.rs` (tests only).
- Do: wrap Rambo's inside in one new element, `<span class="rambo-pose">`. The outer `.rambo` button keeps only `transform: translate(calc(var(--pen-x) - 50%), calc(var(--pen-y) - 50%))` and nothing else that moves. Every state rule and keyframe that adds a rotate or scale moves to `.rambo.is-X .rambo-pose`, and its `translate(...)` part is deleted. The splat burst `::after` and the art (FE-2's `<svg><use>`) move into the pose element.
  - At rest his lower third hides behind the dish's rim (DESIGN-DIRECTION §2): a `clip-path` on `.rambo-pose` in `Home` only. No third layer.
  - His box is the size DESIGN-DIRECTION §2 gives, once the human confirms it (question 11).
- Accept:
  - `grep -c "pen-x" web/theme.css` is 1.
  - Extend the `rambo_rules` test to read `.rambo-pose` rules too: no `.rambo-pose` rule may mention `--pen-x`, and the `.rambo` button rules may not use `rotate` or `scale`.
  - Every pose still looks the same: held, dazed, splat, writing, asking, skittering. Pixel diff of each against main, posed by the same script.
  - Drag follows the pointer exactly.
- Do not touch: `pen_pos` maths and the state machine.

**FE-13: his state says where he is** · Rust, pure · medium · Emilio pairs on review · after FE-5
- Why: a separate place signal can disagree with the state (Home, but placed at a row: bug 1.7). Where he is must follow from what he is doing.
- Files: `web/src/mascot.rs` only.
- Do: add `enum Anchor { Nest, Caret, Row { task: TaskId, progress: f64 }, Held { x: f64, y: f64 }, Dropped { x: f64, y: f64 } }` and `fn anchor(state: &State) -> Anchor`, a pure function. Each state variant that needs data carries it (`WritingRow { task, progress }`, `Dazed { at }`, `Dragging { at }`). `Held` covers `Dragging` and `Inking`. `Held` and `Dropped` are the only pixel cases, because there the finger is the truth.
- Accept:
  - A test per state: the state in, the anchor out.
  - A test that `Home` is always `Anchor::Nest`.
  - The page compiles and behaves as before (nothing reads `anchor` yet).
- Do not touch: `rambo.rs`, `pen_pos`, CSS.

**FE-14: one resolver for his place** · Rust · medium · Emilio pairs on review · after FE-12 and FE-13
- Files: `web/src/rambo.rs`, `web/src/capture.rs`, `web/src/tasks/write.rs`, `web/src/tasks/mod.rs`, `web/src/lib.rs`, `web/src/pen.rs`, `web/Cargo.toml` (web-sys feature `VisualViewport`).
- Do: delete `pen_pos`. A `Memo` in `rambo.rs` derives the anchor from the state (FE-13), and one resolver measures it live: the nest's rectangle, the caret, the row's text at `progress` (through `pen::writing_pen_center`), or the pixels for `Held`/`Dropped`. It turns the result into `--pen-x/--pen-y`, and is the only code that does. It re-resolves on `on_page_moved` (#267) and on `visualViewport` `resize` and `scroll`, which is how the phone keyboard moves the fixed capture bar. If the anchor is off screen, the resolver decides once what happens (the human's answer to question 3); the checks in each writer go. Writers change the state only.
  - When writing ends the state goes to `Home`, so he flies to the nest instead of jumping.
- Accept:
  - A guard test fails on any `pen_pos` in `web/src`, and on any write to `--pen-x` outside `rambo.rs`.
  - A pure test for the resolver: given rectangles for the nest and a row, each anchor gives the expected centre.
  - After writing a row, he flies to the nest. Record a GIF.
  - FE-1's check list passes, including the phone keyboard.
- Do not touch: CSS.

**FE-15: one animation clock** · Rust · medium · after FE-14
- Files: new `web/src/ticker.rs`, plus `web/src/rambo.rs` (skitter), `web/src/tasks/write.rs` (writing loop), `web/src/tack.rs` (its own rAF loop), `web/src/week.rs` (edge slide), `web/src/lib.rs` (module line).
- Do: one `requestAnimationFrame` loop. Callers register a closure that gets the elapsed milliseconds and returns whether to keep going. It replaces the skitter's `set_interval_with_handle(16ms)`, the writing loop's `TimeoutFuture(INK_TICK_MS)` steps, tack's own `request_animation_frame` loop, and the planner's edge slide while a slip is held near an edge (`set_interval` in `week.rs`). Timing comes from the rAF timestamp, never the wall clock.
  - Tack's physics steps a fixed `PHYS_DT`, so the ticker offers a fixed-step mode with an accumulator, not only a per-frame time.
  - `ticker.rs` also exposes `reduced_motion()`, so every motion asks one place.
  - Timers that are not animation stay as they are: the 60s now-line tick in `week.rs` and the 5s brain poll in `brain.rs`.
- Accept:
  - `grep -rn "set_interval\|request_animation_frame\|Interval::new" web/src` finds only `ticker.rs` and those two timers.
  - Pure tests for the registry: run, finish, two at once, and the fixed-step accumulator (a 50ms frame gives the right number of steps).
  - Skitter, writing and tack swing look the same.
- Do not touch: behaviour.

**FE-16: one pointer-gesture module** · Rust · medium · after FE-15
- Files: new `web/src/gesture.rs`, plus `web/src/row.rs`, `web/src/rambo.rs`, `web/src/week.rs`, `web/src/tack.rs`, and the matching `touch-action` lines in `web/theme.css` / `web/tack.css`.
- Do: lift the shared parts into `gesture.rs`: the slop distance, pointer capture, `pointercancel`, Esc to cancel and drop velocity. Keep `row::advance` as the model: a pure state machine with tests, plus thin DOM glue. The module sets `touch-action` on each surface it handles, and the CSS lines for it go. Move one caller per commit.
  - What #210 proved stays true: real touch ends with `pointercancel`, not `pointerup`, once the browser takes a pan.
- Accept:
  - Pure tests in `gesture.rs`: a horizontal pan past the slop claims the gesture; a vertical pan releases it; `pointercancel` ends it; Esc cancels; velocity at drop. Together they cover the settled rule that swipe-delete and page turn never conflict.
  - Each drag behaves as before with mouse and touch: Rambo, row swipe, planner slip, tack strip (FE-1's list).
- Do not touch: anything else in CSS.

**FE-17: split the big files** · Rust · medium each (three PRs) · last
- Files: `web/src/tack.rs` into `tack/{physics,drag,view}.rs`; `web/src/week.rs` into `week/{layout,drag,view}.rs`; `web/src/tasks/mod.rs` with the writing queue moved to `tasks/queue.rs`.
- Do: move code only, the way #255 split `lib.rs`. If a guard test reads a file by path, update the path.
- Accept:
  - No file over 600 lines.
  - Every test still passes, unchanged except for those paths.
  - `git diff --stat` shows moves, not rewrites.
- Do not touch: behaviour.

Splitting `theme.css` into several files is no longer a ticket: the layers of FE-9 fix the ordering problem the split was for. If the file is still hard to work in after FE-9, the split is a later move-only ticket, with one entry file that keeps the order in one place (`@import "css/rambo.css" layer(rambo);`).

### Order

1. FE-0 (now).
2. FE-1, FE-2, FE-3, FE-4, FE-5, FE-6 and FE-7: independent, in parallel, one file group each. FE-2 is a bug fix and goes first.
3. FE-8, then FE-9.
4. FE-10 and FE-11, after the human's answers.
5. One at a time, Emilio pairing on review: FE-12, FE-13, FE-14.
6. FE-15, FE-16, FE-17.

The design backlog (UI-BUGS #8–#11) stays with the human and fits after FE-9, when each look lives in its own layer.

## 3. Not checked

- Phone width and touch were not reproduced in a browser (the window would not shrink below 905px here); the off-pane flight is read from code and layout. QA should confirm FE-0 on a phone.
- That content shown through `<use>` picks up the page's `--rambo-*` tokens (FE-2) is how browsers specify it; the ticket's screenshots are the check.

## 4. Open spec questions for the human

God will settle these with the human before FE-10 onward is built. FE-0 to FE-9 do not depend on them, except where a ticket says so.

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
11. **Size.** DESIGN-DIRECTION §2 says 1.4 times today's size: 64px on a phone, 80px on desktop. Is that the size to build to? FE-12 fixes his box.
12. **Listening feedback.** While he is held and listening, what should show: a pose, a bubble, live transcript text? What should a browser without speech recognition show?

Settled, so not asked: the theme never follows the OS (DESIGN-DIRECTION §7; Rambo's art breaking that is FE-2). Rambo hiding behind the rim is built with a clip in FE-12, no new layer.

For god, not the human: who draws the real dish to replace the `nest.svg` placeholder (Ping?).
