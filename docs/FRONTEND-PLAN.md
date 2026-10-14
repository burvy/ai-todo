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

- **Position.** `pen_pos` is one `RwSignal<(f64, f64)>` written from three modules: `rambo.rs` (drag, home, skitter), `capture.rs` (caret) and `tasks/write.rs` (writing). Nobody owns "where is he", so each writer has to know the others' rules. The bugs above are what happens when one writer forgets the page can move. Fixed by FE-8.
- **Mood state.** `mascot::transition` is the documented way to change state, but nine call sites set a `State` directly (`set_state.set(mascot::State::Home)` and similar) in `rambo.rs`, `tasks/mod.rs` and `capture.rs`, so the machine's tests do not cover every path the page takes.
- **Timing.** Animations run on a mix of `setInterval(16ms)`, chained `Timeout`s with ticket counters, CSS transitions and CSS keyframes. Background tabs throttle all of them differently. The writing loop and the skitter each keep their own clock.

### 1.4 The Rambo/nest layering and positioning model

Rambo is one `position: fixed` button at `top: 0; left: 0`, moved only by `transform: translate(calc(var(--pen-x) - 50%), calc(var(--pen-y) - 50%))`. The model is right (it is the standing rule: Rambo independent of the fixed nest), but it is fragile in CSS. 24 `.rambo…` rules each restate the whole translate before adding a rotate or scale, and any rule that sets `transform` without it teleports him to the corner. A test now guards this (`mascot::tests::every_state_of_the_button_keeps_him_where_he_is`). The lasting fix is two elements: an outer one that is only placed, and an inner one that is only posed (FE-7).

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

## 2. Plan: tickets for GLM agents

The human wants the restructure built by GLM agents, from a spec written by us and reviewed carefully by a Claude agent. So each step below is a self-contained ticket that one GLM agent can finish in one PR without reading anything else. Emilio (front-end owner) is the architect and reviewer, not the builder: he reviews every PR against its ticket before god merges. Tickets wait for the human's answers to section 4 where marked.

### Rules for every ticket

- Read `AGENTS.md`, then only the files the ticket lists. Touch only those files. If the work needs another file, stop and ask god.
- Own worktree and branch: `git worktree add ../ai-todo-wt/<you>-<ticket> origin/main -b <you>/<ticket>`.
- One PR per ticket. The title starts with the ticket id, for example `FE-4: z-index scale`.
- Run the full gate once at the end and paste the results (fmt, clippy `-D warnings`, `cargo test --workspace`, the wasm32 check, `trunk build --release`).
- **Visual tickets** (any CSS change): attach screenshots before and after at 1366x800 and 390x844, in light and dark. State "no visual change" or list each change on purpose.
- **Behaviour tickets**: add a test that fails without the change, and say in the PR that you checked it fails.
- Never edit `docs/DESIGN-DIRECTION.md`. Never add a crate. Never change server code.
- Respect the standing rules: Rambo stays a separate fixed layer, independent of the nest. No blue focus or tap highlight. No native browser controls or scrollbars.
- The reviewer (Emilio) checks each acceptance item one by one. A PR that misses one goes back.

### Tickets

**FE-0: Rambo/nest fix.** Done by Emilio in PR #267; it is the base for everything below.

---

**FE-1: z-index scale** · CSS · small · waits for question 5
- Files: `web/theme.css`, `web/tack.css`, `docs/THEME.md`.
- Do: add z-index tokens to `:root` in `theme.css`: `--z-sheet: 2; --z-capture: 3; --z-ink: 4; --z-rambo: 20; --z-bubble: 21; --z-drag: 30; --z-menu: 40; --z-dialog: 50; --z-note: 60`. Replace every raw `z-index` number in both files with a token, using the table in section 1.5 to map each layer. `z-index: -1` inside a single element may stay. Add the scale as a table to `docs/THEME.md`.
- Accept:
  - `grep -n "z-index: [0-9]" web/*.css` finds nothing.
  - With the gear menu open, the menu covers Rambo.
  - With a task dialog open, the dialog covers Rambo.
  - The tack pin and bin draw under Rambo when he is dragged over them.
  - Screenshots show no other change.
- Do not touch: any `.rs` file, and any property other than `z-index`.

**FE-2: one phone breakpoint** · CSS · small
- Files: `web/theme.css`.
- Do: change `@media (max-width: 720px)` (the delete mark) to `@media (max-width: 700px)`, so every phone rule switches at the same width as the layout.
- Accept:
  - `grep -n "@media (max-width" web/*.css` shows only `700px`.
  - At 710px the desktop layout shows the desktop delete control, not the phone one.
- Do not touch: anything else.

**FE-3: fold the appended rules** · CSS · small
- Files: `web/theme.css`.
- Do: move the one-line rules appended at the end of the file (swipe, delete mark, lifted row, about 14 lines after the reduced-motion block) into the sections they belong to, written in the file's normal multi-line style. Delete any rule that an earlier rule already fully repeats.
- Accept:
  - Screenshots show no visual change.
  - Every rule's selector still appears exactly once per media block.
  - The file ends with its last section, not with loose rules.
- Do not touch: values. This is a move, not a redesign.

**FE-4: split `theme.css`** · CSS · medium (one PR per new file) · after FE-1 to FE-3
- Files: `web/theme.css`, `web/index.html`, and the new files.
- Do: move whole sections, unchanged, into `web/css/tokens.css` (`:root`, dark-mode tokens, fonts), `layout.css` (body, header, main, panes, capture bar), `rambo.css` (nest, Rambo, bubbles), `rows.css` (task list and rows), `planner.css` (week grid, slips), `dialogs.css` (task dialog, gear menu, notes). Link each with `<link data-trunk rel="css" href="css/<name>.css">` in this order, before `tack.css`. `theme.css` disappears in the last PR.
  - The CSS tests in `web/src/mascot.rs` read `../theme.css`. In the PR that moves Rambo's rules, change their `include_str!` path to `../css/rambo.css`.
  - Update `docs/ARCHITECTURE.md` §8, which names `theme.css`.
- Accept:
  - Screenshots show no visual change.
  - `trunk build --release` passes, and `web/dist` contains each file.
  - `cargo test -p rambl-web` passes, with the CSS tests reading the new file.
- Do not touch: any rule's content.

**FE-5: desktop desk widths** · CSS · small · waits for question 6
- Files: `web/theme.css` (or `layout.css` after FE-4), `web/tack.css`.
- Do: in the `min-width: 701px` layout, give the list column a minimum of `18rem`, and let the tack board shrink first (to `8rem`), then the planner.
- Accept:
  - At 905, 1100 and 1366px a title like "Send the invoice to Bob" stays on one line, and its "30 min · Wed 8:30" line stays below it.
  - Nothing scrolls sideways at any width from 701 to 1920px.
  - Screenshots at those three widths are attached.
- Do not touch: the phone layout (`max-width: 700px`).

**FE-6: every Rambo state change through `transition`** · Rust · small
- Files: `web/src/mascot.rs`, `web/src/rambo.rs`, `web/src/tasks/mod.rs`.
- Do: `grep -rn "\.set(mascot::State::" web/src` finds the nine places that set a `State` directly. Replace each with `set_state.set(mascot::transition(&before, mascot::Event::X))`. Where no event fits, add one to `mascot::Event` with a doc comment and its arms in `transition`. Behaviour must not change.
- Accept:
  - The grep finds nothing.
  - Each new event or arm has a test in `mascot.rs` (input state and event, then the expected state).
  - The drag, drop, put-back, skitter-home and capture-then-write flows behave as before on the test site.
- Do not touch: CSS, `pen_pos`, timers.

**FE-7: two-element Rambo** · Rust + CSS · medium · Emilio pairs on review · after FE-6
- Files: `web/src/rambo.rs`, `web/theme.css` (or `rambo.css`), `web/src/mascot.rs` (tests only).
- Do: wrap Rambo's inside in one new element, `<span class="rambo-pose">`. The outer `.rambo` button keeps only `transform: translate(calc(var(--pen-x) - 50%), calc(var(--pen-y) - 50%))` and nothing else that moves. Every state rule and keyframe that adds a rotate or scale moves to `.rambo.is-X .rambo-pose`, and its `translate(...)` part is deleted. The splat burst `::after` moves to the pose element too.
- Accept:
  - `grep -c "pen-x" web/theme.css` (or `rambo.css`) is 1.
  - Extend the test in `mascot.rs` to read `.rambo-pose` rules too: no `.rambo-pose` rule may mention `--pen-x`, and the `.rambo` button rules may not use `rotate` or `scale`.
  - Every pose still looks the same: held, dazed, splat, writing, asking, skittering. Screenshots of each are attached.
  - Drag follows the pointer exactly.
- Do not touch: `pen_pos` maths and the state machine.

**FE-8: one owner for Rambo's place** · Rust · medium · Emilio pairs on review · after FE-7
- Files: `web/src/mascot.rs`, `web/src/rambo.rs`, `web/src/capture.rs`, `web/src/tasks/write.rs`, `web/src/tasks/mod.rs`, `web/src/lib.rs`.
- Do: replace the shared `pen_pos: RwSignal<(f64, f64)>` with `place: RwSignal<mascot::Place>`, where `enum Place { Nest, At(f64, f64), Nib(f64, f64) }`. `At` is his centre; `Nib` is where his nib should touch, and `pen::writing_pen_center` does the conversion. Only `rambo.rs` turns a `Place` into `--pen-x/--pen-y`; for `Nest` it measures the nest live, as `follow_nest` does today. The others set a `Place`. When writing ends, set `Place::Nest`, so he really flies home instead of jumping.
- Accept:
  - `grep -rn "pen_pos" web/src` finds nothing.
  - Unit tests for the `Place` to centre conversion, given a nest rectangle.
  - After writing a row, he flies to the nest. Record a GIF.
- Do not touch: CSS.

**FE-9: one animation clock** · Rust · medium · after FE-8
- Files: new `web/src/ticker.rs`, plus `web/src/rambo.rs` (skitter), `web/src/tasks/write.rs` (writing loop), `web/src/tack.rs` (its own rAF loop), `web/src/lib.rs` (module line).
- Do: one `requestAnimationFrame` loop. Callers register a closure that gets the elapsed milliseconds and returns whether to keep going. It replaces the skitter's `set_interval_with_handle(16ms)`, the writing loop's `TimeoutFuture(INK_TICK_MS)` steps, and tack's own `request_animation_frame` loop. Timing comes from the rAF timestamp, never the wall clock.
- Accept:
  - `grep -rn "set_interval_with_handle\|request_animation_frame" web/src` finds only `ticker.rs`.
  - A pure test for the registry (run, finish, two at once).
  - Skitter, writing and tack swing look the same.
- Do not touch: `week.rs`'s one-minute clock, which stays an interval.

**FE-10: one pointer-gesture module** · Rust · medium · after FE-9
- Files: new `web/src/gesture.rs`, plus `web/src/row.rs`, `web/src/rambo.rs`, `web/src/week.rs`, `web/src/tack.rs`.
- Do: lift the shared parts into `gesture.rs`: the slop distance, pointer capture, `pointercancel`, Esc to cancel and drop velocity. Keep `row::advance` as the model: a pure state machine with tests, plus thin DOM glue. Move one caller per commit.
- Accept:
  - Each drag behaves as before with mouse and touch: Rambo, row swipe, planner slip, tack strip.
  - `gesture.rs` has pure tests for slop, cancel and velocity.
- Do not touch: CSS.

**FE-11: split the big files** · Rust · medium each (three PRs) · last
- Files: `web/src/tack.rs` into `tack/{physics,drag,view}.rs`; `web/src/week.rs` into `week/{layout,drag,view}.rs`; `web/src/tasks/mod.rs` with the writing queue moved to `tasks/queue.rs`.
- Do: move code only, the way #255 split `lib.rs`.
- Accept:
  - No file over 600 lines.
  - Every test still passes, unchanged.
  - `git diff --stat` shows moves, not rewrites.
- Do not touch: behaviour.

**FE-12: front-end QA check list** · QA (Antonio) · small
- Files: hive `QA-BASELINE.md`.
- Do: a check list run on the test site for every front-end PR, at 1366x800 with a mouse and at 390x844 with touch:
  - cold load: Rambo on the nest
  - first drag listens
  - release: no jump
  - dazed drag writes
  - put back in the nest
  - skitter home after 8s
  - page turn
  - resize
  - scroll
  - capture, then he writes the row
  - capture made on the planner page
- Accept: Antonio runs it once on main after #267 and records the result.

### Order

FE-0 (now) → FE-12, FE-2, FE-3 and FE-6 (independent) → FE-1 and FE-5 (after the human's answers) → FE-4 → FE-7 → FE-8 → FE-9 → FE-10 → FE-11. GLM agents can run FE-1 to FE-5 in parallel, one file each. FE-7 and FE-8 touch Rambo's core, so they go one at a time with Emilio pairing on the review. The design backlog (UI-BUGS #8–#11) stays with the human and fits after FE-4, when each look lives in its own file.

## 3. Not checked

- Phone width and touch were not reproduced in a browser (the window would not shrink below 905px here); the off-pane flight is read from code and layout. QA should confirm FE-0 on a phone.
- Dark mode was not looked at for the nest drawing; `nest.svg` restates its own dark colours.

## 4. Open spec questions for the human

God will settle these with the human before the restructure (FE-1 to FE-11) is built. FE-0 does not depend on them.

1. **Rambo at rest.** DESIGN-DIRECTION §2 says his lower third hides behind the dish's rim. That needs a rim drawn above him, a third layer. Still wanted, or is "standing in the dish" enough?
2. **Desktop capture line.** On desktop the capture line and nest sit in the masthead and scroll away with the page, and Rambo goes with them. Should they stay pinned on screen as they do on a phone, so he is always within reach?
3. **Dazed.** Dropped on the page he stays dazed for 8 seconds, then runs home. Is 8 seconds right? Should scrolling or turning the page send him home early?
4. **Writing a row that is off screen.** After a capture made on the planner page (phone), or while scrolled down, the new row is off screen. Today he skips the flight and the row just appears. Should the page turn or scroll to the row so he can write it?
5. **Layers.** Should Rambo sit under the gear menu and dialogs (proposed in FE-1), or stay above everything? Should the tack pin and bin go under him?
6. **Desktop widths.** Between 905 and 1366px, which gives way first: the tack board, the planner or the list? What is the narrowest the list may get?
7. **Breakpoint.** The phone layout ends at 700px, so a tablet held upright (768px) gets the desktop layout. Is that right?
8. **Phone pages.** Which page opens first? Should the capture bar show on the planner page too?
9. **Motion budget.** Which motions are must-haves for speed and feel: flight to the row, letter-by-letter writing, splat, skitter home, idle breathing, page curl? Which can go if they cost time?
10. **Listening feedback.** While he is held and listening, what should show: a pose, a bubble, live transcript text? What should a browser without speech recognition show?
11. **Already pending from #262:** theme follows the OS or a manual switch.
