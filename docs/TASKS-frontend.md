# Task list: frontend engineer

Read `docs/TEAM.md` first (status, API, visual direction). Tick `- [x]` in the same PR when you finish an item.

Run: `.env` with `TYPESAFE_API_KEY=...`, then `cargo run -p server` from the repo root → `http://localhost:8080` (serves `static/`).

_Synced 2026-10-11 ~14:45 by the orchestrator from what is on `main` (PRs #20, #27)._

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
- [ ] **Open in the light parchment theme by default** (today it follows the OS, so dark-mode laptops show Night desk; the pitch is paper)
- [ ] **Planner starts at today**, not Monday: on a Sunday demo most tasks land next week and the visible week looks empty
- [ ] Empty-state hint with a sample sentence to say
- [ ] Phone width check (judges may open the URL on a phone)

## P3: only if time
- [ ] Weather background (Open-Meteo, no key, plus browser location)
- [ ] Calendar import button once the backend `/api/calendar/import` lands (file picker for `.ics`)

## Deploy handoff
Nothing to do: the server serves `static/` and the app uses relative `/api/...` URLs, so it works on burvy.dev unchanged.
