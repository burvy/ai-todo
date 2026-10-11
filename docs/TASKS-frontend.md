# Task list: frontend engineer

Read `docs/TEAM.md` first (status, API). Tick `- [x]` in the same PR when you finish an item.

Run: `.env` with `TYPESAFE_API_KEY=...`, then `.\run-local.ps1` from the repo root → `http://localhost:8080`. Live: https://ai-todo-api.burvy.dev

_Synced 2026-10-11 17:05 by the orchestrator (main at #44). **Now: bug fixes and polish. 18:15 anything shaky gets hidden. 18:30 feature freeze.**_

## Done ✅
- P0/P1: parchment UI, task list (check, edit, delete, add), text box → `/api/message`, push-to-talk, spoken replies, week planner, "Schedule it" (#20, #27)
- P2: pencilled-in fields, writing animation, focus date, error toast, light theme default, planner starts today, empty-state sample, phone width (#29)
- Pen in the text box: tap/Enter sends, hold talks, listening pose, flies up to write each new task and tick done ones, sticky composer on phones, phone page removed (#38)
- From Jim (#44): "Add to my calendar" link + upcoming-task reminders in `static/reminders.js` (please don't edit that file; restyle via CSS)

## NOW, in this order
1. - [ ] **Phone: swipe left / right between the task list and the planner (burvy's #1).** At phone width the two are full-screen pages side by side: **To do** first, **Planner** to the right. Swipe left → planner, swipe right → list. Two dots or a "To do · Planner" tab strip at the top show where you are, and tapping them switches too. The pen text box stays fixed at the bottom on both pages. Simplest build: a horizontal `scroll-snap-type: x mandatory` container with two `scroll-snap-align: start` pages (native, smooth, no JS gesture code); JS only to sync the dots/tabs. Vertical scrolling inside each page must still work. When new tasks arrive while on the planner, slide back to the list so the pen can write them.
2. - [ ] **Full task titles.** Titles are cut off ("Call my …"). Let them wrap to 2 lines, or move the chips under the title in small text; hide the "text" source chip.
3. - [ ] **Planner overlaps.** Overlapping blocks side by side (split the column width); full title on hover/tap.
4. - [ ] **Import calendar button.** File picker for `.ics` → read as text → `POST /api/calendar/import {ics_text}` → `POST /api/schedule` → redraw. Test with `demo/outlook/week.ics` (9 classes/shifts, Oct 12–16).
5. - [ ] **Style the "Add to my calendar" header link** like the Voice / Night desk pills (CSS only; the link has the text "Add to my calendar" and sits in `.masthead-tools`).
6. - [ ] **Drag planner tasks to reorder** (only if 1–4 are merged). Placed tasks only, never busy blocks; snap to 15 min → `PATCH /api/tasks/:id {"pinned_start":"<UTC ISO>"}` → `POST /api/schedule` → redraw; pin icon on pinned blocks, "unpin" sends `{"pinned_start":null}`. Long-press 300 ms to start a drag on phones. Needs backend 2's `pinned_start`. **Hide it if it's not solid by 18:15.**
7. - [ ] **Drop an email / PDF** once the new teammate's route lands: multipart `file` on `POST /api/capture` → `{tasks}` → same writing animation.

## Dropped
- Week / month views, weather background.

## Deploy handoff
Nothing to do: the server serves `static/`, relative `/api/...` URLs.
