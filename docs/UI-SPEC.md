# UI spec (burvy, 18:05): one page, real paper

The single source of truth for the look and the interactions. One website (`static/`) for PC and phone. Where code disagrees with this file, this file wins.

## Layout
- **No masthead.** No "ai-todo" title or tagline at the top of the page.
- **Two sheets of paper side by side on PC:** left = **To do** (with the pen text box under the list), right = **Planner**. On a phone they are two pages: swipe left/right, with a small "To do · Planner" tab strip.
- **Settings:** one small gear (or ink-blot) button **to the right of the "To do" heading**. It opens a little paper popover with: Voice on/off, Night desk on/off, Add to my calendar. Nothing else sits in the To do header, and nothing overlaps the heading or the red margin line.
- **Planner header:** title, ‹ week ›, **Schedule it**. Import lives on the planner sheet as a quiet footer row: **Import calendar (.ics)** and **Import email / PDF** (paperclip). These are infrequent; keep them small.

## Paper, authentic
- Warm off-white sheet with subtle paper grain (an inline SVG `feTurbulence` noise at low opacity is enough), a faint shadow, slightly uneven edge (optional). Ruled blue lines and one red margin line on the To do sheet. Ink near-black; one blue ink accent; red pencil for overdue.
- Task titles in the handwriting font, sitting **on** the ruled lines. Chips (duration, deadline, label) small, in pencil grey, under or after the title; no boxed "pill" look, more like pencil notes. No "text" source chip.
- Planner = a diary page: hour lines like ruling, busy blocks as grey hatched sticky notes, tasks as ink-coloured notes by label.
- Night desk = same layout on dark paper. Light is the default.

## The pen
- **Home: inside the text box, right end.** Not in a corner of the page. Tap / Enter sends; hold (or hold space) talks; **listening pose** while held.
- **Writes:** when new tasks arrive, the pen lifts out of the text box, travels to each new line in turn, and the title is **written stroke by stroke** (clip-path or `stroke-dashoffset` reveal left to right, ~500 ms per title, the nib follows the reveal edge). Then it returns home.
- **Ticks:** completing a task (checkbox or "I finished X") → the pen goes to that line, draws the tick and a hand-drawn strike-through, returns.
- **Bug to fix (reported by backend 2):** when tasks are entered, existing lines get "re-checked" / re-animated. Only the **new** lines animate; existing rows must not re-render their tick or strike animations. Key rows by task id and diff; never rebuild the whole list for an add.
- `prefers-reduced-motion`: no flight, titles appear instantly.

## Editing and order
- **Edit a task:** click (tap) the title to edit inline; deadline / duration / label editable from their pencil notes. Enter saves (`PATCH`), Esc cancels.
- **Reorder the list:** drag a row by a small handle (long-press on phones). The new order is saved (`PATCH /api/tasks/:id {"position": n}`) and is the tiebreaker the scheduler uses after deadline.
- **Reorder the planner:** drag a task note to another slot (snap 15 min) → `PATCH {"pinned_start": "<UTC>"}` → reschedule; pinned notes show a small pin; tap the pin to unpin (`null`).

## Imports
- **Calendar:** Import calendar (.ics) → upload; if the server answers `409` (clashes with existing blocks), show a small paper prompt listing the clashes, "Import anyway / Cancel"; Import anyway resends the same file with the multipart field `confirm=true`. Then reschedule.
- **Email / PDF:** paperclip → multipart `file` to `POST /api/capture` → new tasks come back and the pen writes them like a ramble.

## Done means
Looks like paper on a 1400-wide laptop and a 390-wide phone, no overlaps, no console errors, every action above works against the live API.
