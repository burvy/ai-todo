# Front-end plan: what is still open

Checked 2026-10-15 against main. The target shape is `docs/ARCHITECTURE.md` section 8A (the spec wins over this file). The look and feel is `docs/DESIGN-DIRECTION.md`. This file lists only the front-end work that is still open. The ticket ids (FE-, BUG-) are kept so code comments and older notes still resolve.

## Done

- **FE-0** Rambo and the nest (#267).
- **FE-2** Rambo's art follows the page theme. No SVG under `web/assets` has a `prefers-color-scheme` block, and each drawing has its `#art` group.
- **FE-3** one phone breakpoint: `theme.css` uses `max-width: 700px` only (#282).
- **FE-4** every Rambo state change goes through `mascot::transition`: no direct `State` set is left in `web/src` (#286).
- **FE-5** most of it: `tack.css` is gone, the `--coffee` reference is gone, `.error` uses a token, and `--fold-shadow` has a dark value. Still open: see below.
- **BUG-4** planner hour labels and day headers are reactive; the header re-reads each minute, so it moves at midnight (`week.rs`).
- **BUG-5** "task added" fires only for a new plain task id, never on the first load (`brain.rs`, `task_added`).
- **BUG-6** every snapshot fetch uses the server's default window, one constant in `crates/core/src/tuning.rs`.
- **BUG-7** an empty, bad or skipped time gets its own message (`tasks/fields.rs`, with tests).

## Gone with the tack board

The strip board (`tack.rs`, `tack.css`) is removed, so these no longer apply: BUG-1, BUG-2, BUG-3, and audit items 1 to 3, 19 (strip part), and 20. Audit items 22 and 23 are in FE-5 above.

## Still open

1. **Rambo's position has eight writers in four files.** `pen_pos` is set in `rambo.rs` (six), `capture.rs` (two) and `tasks/write.rs` (one). It should have one owner: the anchor resolver in ARCHITECTURE 8A.5. Moved there as a planned step.
2. **BUG-8, the occurrence parser.** `occurrence_parts` (`tasks/mod.rs`, about line 95) still uses `split_once(':')`. A date never has a colon, so it should split from the right (`rsplit_once`), with a test for a series id that contains a colon. The move path already goes through the store; the overdue rule's second copy (`is_overdue_at`) was not checked.
3. **FE-5 leftovers.** `api::revise_task` (`api.rs`, about line 345) has no caller: delete it. The CSS rules for `.swipe-*` and `.pose-*` were not re-checked; grep `web/src` for each class before deleting.
4. **BUG-9, the planner scroll to the last day on a phone.** Not checked in a browser here. Open until someone at 390x844 with touch can scroll to every day shown.
5. **2.6, the ask bubble.** Edit opens the sheet. Move, and the choices for a planner slip or an empty hour, are open. The choices are DESIGN-DIRECTION §9's, not this file's.

## Pointers

Code comments name this file for FE-2, section 1.5 (the z-index scale, done) and 1.8 item 7 (the edit sheet as a modal). DESIGN-DIRECTION §9 names it for the owner's answers. Keep the file name, so those references still resolve.
