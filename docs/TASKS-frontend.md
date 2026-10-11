# Task list: frontend engineer

Read `docs/TEAM.md` first (status, API, visual direction). Tick `- [x]` in the same PR when you finish an item.

Run: `.env` with `TYPESAFE_API_KEY=...`, then `.\run-local.ps1` from the repo root → `http://localhost:8080` (serves `static/`). Live: https://ai-todo-api.burvy.dev

_Synced 2026-10-11 ~15:45 by the orchestrator from what is on `main` (PRs #20, #27, #29)._

## P0: plain TODO list ✅
- [x] Page shell: parchment, ruled lines, Caveat + Inter fonts
- [x] Task list from `GET /api/tasks` with deadline, duration, label chips
- [x] Checkbox → `PATCH {"status":"done"}`, strike-through
- [x] Delete
- [x] Inline edit (title, deadline, duration)
- [x] "Add one task" → `POST /api/tasks` (#27)

## P1: AI flow ✅
- [x] Text box → `POST /api/message`
- [x] Push-to-talk (mic button / space bar, Web Speech API)
- [x] Speak `reply` aloud (Voice toggle)
- [x] Week planner: busy blocks + placements, UTC → local
- [x] "Schedule it" → `POST /api/schedule`

## P2: polish
- [ ] **Week / month views** (in progress)
- [x] Pen mascot (idle / listening / writing)
- [x] "Writing" animation for new tasks
- [x] Low-confidence fields "pencilled in"
- [x] Jump to `focus_date`
- [x] Busy state + error toast
- [x] **Opens in the light parchment theme** — no `prefers-color-scheme` rule anywhere in `static/`, so it always opens light; Night desk only if you ask for it, and that choice is remembered in `localStorage`
- [x] **Planner starts at today**, not Monday: a rolling seven-day window from today, so a Sunday demo shows the Monday the tasks actually land on. Prev/next step seven days, the date label jumps back to today
- [x] Empty-state hint with a sample sentence to say (taken from `demo/rambles.md`, set in handwriting so it reads like a note on the page)
- [x] Phone width check — verified at 390x844 and 1400x900 in a real browser: no horizontal overflow, titles wrap, planner scrolls inside its own column, mascot hidden so it stops covering the page

## P1b: import buttons (after week/month, as the routes land)
- [ ] **Import calendar** button: file picker for `.ics` → `POST /api/calendar/import` with `{ics_text}` → redraw the planner from the response (same shape as `GET /api/calendar`). Owner of the route: new teammate (`docs/TASKS-calendar.md`).
- [ ] **Drop an email / PDF**: file drop or picker → multipart `file` on `POST /api/capture` → new tasks come back as `{tasks}`; run the same "writing" animation. Owner of the route: backend 2.

## P3: only if time
- [ ] Weather background (Open-Meteo, no key, plus browser location)

**Week/month view and both buttons: PR by 17:30. Feature freeze 18:30.**

## Deploy handoff
Nothing to do: the server serves `static/` and the app uses relative `/api/...` URLs, so it works on burvy.dev unchanged.
