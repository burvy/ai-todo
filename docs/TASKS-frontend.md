# Task list: frontend engineer

Read `docs/TEAM.md` first (status, API, visual direction). Tick `- [x]` in the same PR when you finish an item.

Run: `.env` with `TYPESAFE_API_KEY=...`, then `.\run-local.ps1` from the repo root → `http://localhost:8080` (serves `static/`). Live: https://ai-todo-api.burvy.dev

_Synced 2026-10-11 ~16:30 by the orchestrator (PRs #20, #27, #29, #33). New top section: pen in the text box._

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

## P0-now: the pen lives in the text box (burvy's direction, do this first)
Build on the original UI (`index.html` / `app.js` / `styles.css`), not the phone page.
- [ ] **Home position:** move `#mascotTrack` out of the masthead (`index.html` ~line 24) into `.composer-row`, at the right end of the text box, replacing `#mic`. Remove the `#mic` button.
- [ ] **The pen is the button:** click the pen or press Enter → send (same `submit()`); hold the pen or hold space → talk, release → send. Real `<button>` with `aria-label="Send, or hold to talk"`.
- [ ] **Listening pose** while held: pen leans toward the user, with the ink trail or a pulse.
- [ ] **Pen goes up and writes:** when `/api/message` returns new tasks, the pen flies from the text box to the first new row, plays the writing animation on each new title in turn (reuse `--pen-x` / `--pen-y` in `app.js` ~lines 174–189), then flies home. If it's quick, done tasks get the same trip with a tick.
- [ ] While the pen is away: show "the pen is writing…" and don't accept input. On an error: pen goes straight home, error shows.
- [ ] Three poses only: idle (gentle bob in the text box), listening, writing.
- [ ] **One UI:** remove the "Phone view" link and `static/mobile.html`, `mobile.js`, `mobile.css` in the same PR. Check the pen-button is easy to tap at 390px wide.

## P2: polish
- [ ] **Week / month views**: optional now, only if the pen is done before 17:30
- [x] Pen mascot (idle / listening / writing)
- [x] "Writing" animation for new tasks
- [x] Low-confidence fields "pencilled in"
- [x] Jump to `focus_date`
- [x] Busy state + error toast
- [x] **Opens in the light parchment theme** — no `prefers-color-scheme` rule anywhere in `static/`, so it always opens light; Night desk only if you ask for it, and that choice is remembered in `localStorage`
- [x] **Planner starts at today**, not Monday: a rolling seven-day window from today, so a Sunday demo shows the Monday the tasks actually land on. Prev/next step seven days, the date label jumps back to today
- [x] Empty-state hint with a sample sentence to say (taken from `demo/rambles.md`, set in handwriting so it reads like a note on the page)
- [x] Phone width check (#29). Re-check after the pen moves into the text box: it must stay visible on phones now.

## P1b: import buttons (after week/month, as the routes land)
- [ ] **Import calendar** button: file picker for `.ics` → `POST /api/calendar/import` with `{ics_text}` → redraw the planner from the response (same shape as `GET /api/calendar`). **The route is live on `main` since #37** — nothing is blocking this.
- [ ] **Drop an email / PDF**: file drop or picker → multipart `file` on `POST /api/capture` → new tasks come back as `{tasks}`; run the same "writing" animation. Owner of the route: new teammate (`docs/TASKS-pdf.md`).

## P3: only if time
- [ ] Weather background (Open-Meteo, no key, plus browser location)

**Pen in the text box: PR by 17:30. Then the import buttons as the routes land. Feature freeze 18:30.**

## Deploy handoff
Nothing to do: the server serves `static/` and the app uses relative `/api/...` URLs, so it works on burvy.dev unchanged.
