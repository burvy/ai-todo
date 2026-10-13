# Design decisions (owner, 2026-10-13), read with docs/REDESIGN.md

These are settled. Where this file and REDESIGN.md differ, this file wins.

## Handwriting everywhere
- Every word is handwritten. Self-host about 3 OFL handwriting fonts in `web/assets/fonts/` (no CDN).
- Two tiers: a **neat, very legible hand** for small UI text (times, dates, settings, day names) and an **expressive hand** for task titles. Check legibility at 390px.

## Rambo styles (writing tools)
- Gear menu > "Rambo": fountain pen (default), pencil, ballpoint, quill, marker. Remembered per device.
- Each tool has its own art (Ping) and its own writing: font from the 3, plus ink colour, weight and texture (pencil grey and grainy, marker thick, quill brown with the odd blot, ballpoint thin blue-black within the palette).
- All styles share the same states/animations (Stanley): idle, peeking while you type, dozing at night (wakes when you type), held/listening, writing, scribbling out, celebrating, sulking at overdue, dropped hard (ink splat/wobble), annoyed (tidying the tack board), returning to the nest.

## Sound
- **On by default**, gentle. Mute toggle in the gear menu. Pen scratch synced to writing; pick-up/drop; tick on done; scribble on delete; page turn; paper crumple; tape/pin.

## Done tasks: the tack board
- Ticking a task done: hand-drawn strike-through, a moment's pause, then **Rambo cuts the line out as a paper strip and tacks it** onto the tack board.
- Tack board: **PC = the right edge of the list page; phone = a strip along the bottom you pull up.**
- Strips hang with simple **simulated physics** (hang from the pin, swing, settle, flutter when dropped). Hand-written small integrator in Rust/wasm, no physics library. Cap about 30 visible strips; older ones stack under the pin.
- The user can drag a strip and:
  - **restore** it (drop it back on the list: task becomes not done),
  - **set it aside** (drop it anywhere on the board/desk: it lies there loose),
  - **crumple and throw it away** (fling it off the page, or drop on the bin: deletes the task, with a crumple animation and sound).
- "Tidy up" (ask Rambo, e.g. tap the board's pin): Rambo grumpily puts all the set-aside strips back on the tack.
- Data: done stays done on the server; tacked/set-aside/positions are presentation only, saved per device in localStorage. Restore = undo done. Crumple = delete.

## Gestures (phone)
- Swipe **right** on a task = delete (Rambo scribbles it out). Swipe **left** = flip the page to the planner (parchment page flip); swipe right on the planner flips back.

## Planner
- Opens at working hours or now; handwritten day names, today underlined; drag a task to another time/day like moving a paper slip; long-press an empty hour pre-fills the capture line with that slot.

## Order of work (ship in this order; each step must leave the app working)
1. Theme + layout + gear menu + fonts + planner hours/day names.
2. Rambo fixed: drag follows pointer, nest = send, listening pose, letter-by-letter writing, bigger, paper colours.
3. Swipe-right delete with scribble-out; page flip; planner drag reorder; sounds on.
4. Tack board with physics.
5. Rambo styles + all moods; Zhipu's extras (rotating placeholder, long-press hour, fading pencil, nib gear).
Deploy candidate = whatever is merged and passes god's end-to-end check of add, write, done, delete, planner.
