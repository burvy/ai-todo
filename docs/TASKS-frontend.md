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
- [x] **Home position:** moved `#mascotTrack` out of the masthead into `.composer-row`, at the right end of the text box, replacing `#mic`. The `#mic` button is gone.
- [x] **The pen is the button:** tap the pen or press Enter → send (same `submit()`); hold the pen or hold space → talk, release → send. Real `<button aria-label="Send, or hold to talk">`. A hold that produced speech suppresses the trailing click so nothing sends twice.
- [x] **Listening pose** while held: pen leans, brows lift, and an ink trail blooms above the nib — seven blobs merged into one by a `blur` + `contrast` filter.
- [x] **Pen goes up and writes:** on new tasks it flies from the text box to each new row in turn, plays the writing animation, then flies home. Tasks that came back *done* get the same trip with a tick. Home is measured from the button, so repeated trips never compound.
- [x] While the pen is away: "the pen is writing…" is shown and the text box plus pen are locked. On an error the pen goes straight home and the toast explains.
- [x] Three poses only: idle (bobbing in the text box), listening, writing.
- [x] **One UI:** removed the "Phone view" link and `static/mobile.html`, `mobile.js`, `mobile.css`.
- [x] Pen is easy to tap at 390px — 58px target, and the composer is now sticky on phones so the pen stays in reach while the list scrolls (it used to sit ~450px below the fold once the list had more than a few tasks).

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
- [x] Phone width check (#29). Re-checked after the pen moved into the text box: no horizontal overflow at 390px, pen on screen and pinned while scrolling.

## P1b: import buttons (after week/month, as the routes land)
- [ ] **Import calendar** button: file picker for `.ics` → `POST /api/calendar/import` with `{ics_text}` → redraw the planner from the response (same shape as `GET /api/calendar`). Owner of the route: backend 2.
- [ ] **Drop an email / PDF**: file drop or picker → multipart `file` on `POST /api/capture` → new tasks come back as `{tasks}`; run the same "writing" animation. Owner of the route: new teammate (`docs/TASKS-pdf.md`).

## P3: only if time
- [ ] Weather background (Open-Meteo, no key, plus browser location)

**Pen in the text box: PR by 17:30. Then the import buttons as the routes land. Feature freeze 18:30.**

## Deploy handoff
Nothing to do: the server serves `static/` and the app uses relative `/api/...` URLs, so it works on burvy.dev unchanged.
