# Task list: frontend engineer

Read `docs/TEAM.md` first (status, API). Tick `- [x]` in the same PR when you finish an item.

Run: `.env` with `TYPESAFE_API_KEY=...`, then `.\run-local.ps1` from the repo root → `http://localhost:8080`. Live: https://ai-todo-api.burvy.dev

_Synced 2026-10-11 17:05 by the orchestrator (main at #44). **Now: bug fixes and polish. 18:15 anything shaky gets hidden. 18:30 feature freeze.**_

**One website (burvy):** build every change on the existing `static/` site (`index.html`, `app.js`, `styles.css`), which serves both PC and phone. Phone behaviour is a media query (`@media (max-width: 720px)`) on that same page, never a separate page or a rebuild. The `design/mobile-reference` branch is a visual reference only; take ideas from it, don't build on it.

## Done ✅
- P0/P1: parchment UI, task list (check, edit, delete, add), text box → `/api/message`, push-to-talk, spoken replies, week planner, "Schedule it" (#20, #27)
- P2: pencilled-in fields, writing animation, focus date, error toast, light theme default, planner starts today, empty-state sample, phone width (#29)
- Pen in the text box: tap/Enter sends, hold talks, listening pose, flies up to write each new task and tick done ones, sticky composer on phones, phone page removed (#38)
- From Jim (#44): "Add to my calendar" link + upcoming-task reminders in `static/reminders.js` (please don't edit that file; restyle via CSS)

## NOW, in this order
1. - [x] **Phone: swipe left / right between the task list and the planner (burvy's #1).** At phone width the two are full-screen pages side by side: **To do** first, **Planner** to the right. Swipe left → planner, swipe right → list. Two dots or a "To do · Planner" tab strip at the top show where you are, and tapping them switches too. The pen text box stays fixed at the bottom on both pages. Simplest build: a horizontal `scroll-snap-type: x mandatory` container with two `scroll-snap-align: start` pages (native, smooth, no JS gesture code); JS only to sync the dots/tabs. Vertical scrolling inside each page must still work. When new tasks arrive while on the planner, slide back to the list so the pen can write them.
2. - [x] **Full task titles.** Titles are cut off ("Call my …"). Let them wrap to 2 lines, or move the chips under the title in small text; hide the "text" source chip.
3. - [x] **Planner overlaps.** Overlapping blocks side by side (split the column width); full title on hover/tap.
4. - [x] **Import calendar button.** File picker for `.ics` → read as text → `POST /api/calendar/import {ics_text}` → `POST /api/schedule` → redraw. Test with `demo/outlook/week.ics` (9 classes/shifts, Oct 12–16).
5. - [x] **Style the "Add to my calendar" header link** like the Voice / Night desk pills (CSS only; the link has the text "Add to my calendar" and sits in `.masthead-tools`).
6. - [ ] **Drag planner tasks to reorder** (only if 1–4 are merged). Placed tasks only, never busy blocks; snap to 15 min → `PATCH /api/tasks/:id {"pinned_start":"<UTC ISO>"}` → `POST /api/schedule` → redraw; pin icon on pinned blocks, "unpin" sends `{"pinned_start":null}`. Long-press 300 ms to start a drag on phones. Needs backend 2's `pinned_start`. **Hide it if it's not solid by 18:15.**
7. - [ ] **Drop an email / PDF** once the new teammate's route lands: multipart `file` on `POST /api/capture` → `{tasks}` → same writing animation.

## Dropped
- Week / month views, weather background.

## Blocked (backend hasn't landed — nothing to hide, nothing shipped)
6. Drag-to-reorder needs `PATCH /api/tasks/:id {"pinned_start": ...}`. `pinned_start` does not appear anywhere in `server/src/main.rs`, so there is no route to call. Not started.
7. Email / PDF needs multipart on `POST /api/capture`; `server/src/main.rs:579` still takes `Json(body): Json<CaptureIn>` and has never had a multipart branch. Not started.

## Notes for whoever picks these up
- **Item 5 had to use `!important`.** `static/reminders.js` builds the link with an inline `style.cssText`, which outranks any plain class. The overrides are scoped to `.masthead-tools > a[href="/api/calendar.ics"]` and commented as such — please don't "clean them up" without checking that file.
- **Item 3 lays blocks out from painted geometry, not timestamps.** A block shorter than its 16px minimum height sticks out of its slot, and that is an overlap you can see even when the data has none. Lanes are measured after the day is attached to the document (`offsetTop` is 0 on a detached element).
- **Item 4 re-runs `/api/schedule` before redrawing**, otherwise freshly imported events would be hidden behind placements computed against the old busy list. A file that isn't a calendar gets a plain toast, not a console error.

## Deploy handoff
Nothing to do: the server serves `static/`, relative `/api/...` URLs.
