# Redesign brief (from the owner, 2026-10-13)

The current web app looks amateur: "just buttons and sections". It must look
**integrated, authentic and unique**, like writing on paper, not a stack of panels.
The v1 prototype was decent; read it for feel and behaviour (not code to copy):
`git show 679f669^:static/index.html`, `679f669^:static/app.js`, `679f669^:static/styles.css`.

## 1. Theme
- **Light is the default and is parchment.** Dark mode exists too (v1 had both), toggled in the gear menu. It never follows the OS — dark only when the user picks it (owner QA, 2026-10-13).
- Light palette (use as `:root` tokens in `web/theme.css`):
  - parchment `#F2E8CF` (page) · wheat `#ECD7AF` · ecru `#E1C38C`
  - chamoisee `#A7865D` (lines, muted text) · coffee `#5D4732` (ink) · golden brown `#9F682E` (accent)
- No hard-edged boxes, borders and card stacks. Use space, rules like a notebook, ink colour and paper texture. Few or no visible "buttons".
- The torn-edge strip and the cup stain currently read as a glitch and dirt: make them subtle or drop them.

## 2. Layout
- **Settings = a small gear, top right.** Everything currently in "View settings" goes in its menu, **including Import .ics** (a menu item that opens the file picker; never a raw "Choose File" input on the page).
- PC: task list left, planner right, both visible. Phone: list first, swipe sideways to the planner.
- Planner opens at working hours (about 08:00, or scrolled to now), not 02:00, and shows **day names** on each column.

## 3. Rambo (the pen mascot)
- **Rambo replaces the Add button.** He rests in a small "nest" (bubble) at the end of the capture line, bottom right. **Tapping/clicking the nest submits** the text.
- Rambo is **separate from the nest**: the user can **drag him out and toy with him**; he follows the finger/mouse **exactly**; on release he recovers and skitters back to the nest.
  - BUG today: dragging sends him off-screen to the left. The drag maths is wrong (pointer vs element offset / transform origin). Fix it and test with mouse and touch.
- **Picking Rambo up makes him listen** (speech capture). There is no mic button.
  - BUG today: "Could not start speech recognition". Find out why (permissions, secure context, starting twice, `continuous`/`interimResults`, Edge vs Chrome) and fix it; show a calm message only when the browser truly lacks it.
- A bit **bigger** than now. Recolour him to fit the paper palette (his blue clashes); he should look drawn on paper.
- **Writing animation** (today: a scribble, then the words appear at once; or he just slides to the row and back): the task title must **appear gradually, letter by letter, under the nib** as he moves along the line, then he returns to the nest.
- More life generally: small idle movements, the speech bubble for his lines, gentle transitions. Keep `prefers-reduced-motion` respected.

## 4. Task rows
- Desktop: a delete control **appears on the side on hover** (as in v1).
- Phone: a barely visible `x`, and **swipe right on a task to delete** it.

## Definition of done (every PR)
Full gate green (incl. `cargo check -p rambl-web --target wasm32-unknown-unknown` and `trunk build --release`), works at 1366x800 and 390x844, light and dark, and the PR body says what to click to see it.

## Owner decisions, round 2 (2026-10-13)

1. **Phone gestures, settled:** swipe **right** on a task = delete (Rambo scribbles it out). Swipe **left** = turn the page (list → planner, with a page-flip); on the planner page, swipe **right** turns back. The two never conflict.
2. **Task reordering on the planner must work:** drag a task to another time or day (keep the existing list drag-reorder too). Make it feel like moving a paper slip.
3. **All suggestions approved, build them:**
   - Rambo **dozes at night** and wakes when you type.
   - Rambo **peeks at the text box** while you type.
   - A **small celebration + hand-drawn strike-through** when a task is ticked done.
   - A **sulky look** at overdue tasks.
   - **Ink splat / wobble** when he is dropped hard.
   - A **faint previous-page shadow** on the planner.
   - **Pen-scratch sound** synced to the writing animation.
