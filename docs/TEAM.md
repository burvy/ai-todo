# ai-todo: team handoff

**Pitch:** "Every scheduling tool asks you to do data entry. This one just listens."
Ramble or drop a file, walk away, and your tasks are written and scheduled for you.

Repo: `github.com/burvy/ai-todo`. Run `git pull origin main` before you start.

**Live:** https://ai-todo-api.burvy.dev (anyone can open it; Chrome or Edge for the mic).
**Run locally:** `.env` with `TYPESAFE_API_KEY=...` in the repo root, then `.\run-local.ps1` → http://localhost:8080 (`-Fresh` = throwaway data).

---

## 1. Status (synced 2026-10-11 17:05, main at #44)

**Works on `main` (all live-tested by the orchestrator against real Jev):**

| Area | What works |
|---|---|
| Web app (`static/`) | Parchment UI, task list (check off, edit, delete, add), **pen in the text box** (tap/Enter sends, hold to talk, flies up to write new tasks), spoken replies, 7-day planner from today with busy blocks + placed tasks, "Schedule it", pencilled-in low-confidence fields, phone layout, Night desk theme |
| AI capture | Rambles become clean tasks: chit-chat and self-talk dropped, "upload it" merged into its task, "X and Y" kept together; relative dates with 17:00 local deadlines; durations; School / Errands / Personal / Work labels. All 3 `demo/rambles.md` come out clean |
| Conversation | "I finished X" → "Done: X." · "my 3pm got cancelled" → "Cancelled 1:1 at 3:00" · "what does tomorrow/Friday look like" → day summary · "what should I do now" → next task. Honest "I couldn't…" replies when nothing matches |
| Calendar import | `POST /api/calendar/import {ics_text}` replaces busy blocks with a real Outlook/Google `.ics` (`demo/outlook/week.ics` = 9 events) |
| Alarms | "Add to my calendar" downloads `GET /api/calendar.ics`: every planned task with a 10-minute alarm, for Google / Outlook / iPhone. Open page: notification + spoken "Up next" 5 min before a task |
| Scheduler / storage | Deadline first then priority, ≤2h chunks, 9:00–18:00 local; SQLite, survives restarts |
| Ops | CI, `run-local.ps1`, deployed at `ai-todo-api.burvy.dev` |

**Known gaps:** no PDF upload route yet; no phone swipe between list and planner or drag-to-reorder yet; task titles truncated in the list; overlapping planner blocks; no `/api/reset`; no login (everyone shares one list on the live site).

---

## 2. Who owns what

| Owner | Doing now | Task list |
|---|---|---|
| **Frontend** | Swipe left/right between list and planner on phones (#1), full titles, planner overlaps, Import calendar button, drag to reorder | `docs/TASKS-frontend.md` |
| **Backend 2 (Rayansh)** | `/api/reset`, reschedule after import, `pinned_start` for drag | `docs/TASKS-backend.md` |
| **New teammate** | PDF / email upload on `POST /api/capture` (cut-off 18:00) | `docs/TASKS-pdf.md` |
| **burvy** | Redeploys, demo rehearsal | |
| **AI agents** | Review, live-test, merge; Jim fixes rehearsal bugs | |

**Schedule (deadline ~19:55):**
- **18:00** PDF upload works or is cut.
- **18:15** anything shaky (drag) gets hidden.
- **18:30 feature freeze.** Only bug fixes after. Orchestrator builds the final release, burvy redeploys and runs `POST /api/reset`.
- **18:30–19:30** rehearse the demo twice on the live site.

**Avoid merge conflicts:** backend 2 and the new teammate both touch `server/src/main.rs`. Keep edits to a few lines, put logic in your own file, pull `main` before you push.

**Tick your boxes:** when you finish an item, change `- [ ]` to `- [x]` in your task list in the same PR. The orchestrator updates this file.

**Git rules:** branch `yourname/feature`, small PRs, pull `main` before pushing, never push to `main`, never commit `.env`, keys or `data/`. Run `cargo fmt` and `cargo clippy --workspace --all-targets -- -D warnings` before pushing.

---

## 3. API (frontend builds against this)

Task:
```json
{ "id": "t1", "title": "Finish chem lab report", "notes": "", "duration_min": 180,
  "deadline": "2026-10-16T21:00:00Z", "priority": 2, "status": "todo",
  "confidence": 0.72, "source": "voice", "label": "School" }
```

| Call | Body | Returns |
|---|---|---|
| `GET /api/tasks` | | `{tasks:[Task]}` |
| `POST /api/tasks` | `{title}` | `201` Task |
| `PATCH /api/tasks/:id` | e.g. `{"status":"done"}` | `Task` |
| `DELETE /api/tasks/:id` | | `204` |
| `POST /api/capture` | `{text, source:"voice"\|"text"}` | `{tasks:[Task]}` (new ones) |
| `POST /api/message` | `{text}` | `{reply, intent, tasks, placements, unscheduled, focus_date}` |
| `POST /api/schedule` | | `{placements:[{task_id,start,end}], unscheduled:[{task_id,reason}]}` |
| `GET /api/calendar?from&to` | | `{busy:[{start,end,title}], placements:[...]}` |
| `POST /api/calendar/import` | `{ics_text}` (the whole `.ics` file as a string) | `{busy:[...], placements:[...]}` — **replaces** the busy blocks. `400 {error}` if it is not an `.ics` |

All times are UTC ISO strings; convert to local time in the UI. A long task can have **several placements** (chunks).

**Simplest wiring:** send every user message to `POST /api/message`, speak `reply` aloud (browser `speechSynthesis`), redraw the list from `tasks` and the timeline from `placements`, and jump to `focus_date` if it is set.

---

## 4. Task lists

Frontend: `docs/TASKS-frontend.md` · Backend 2: `docs/TASKS-backend.md` · New teammate: `docs/TASKS-pdf.md`. Tick boxes as you finish.

---

## 5. Visual direction: recommendation **parchment**

The pen mascot and "it writes your tasks for you" only make sense on paper. Liquid glass is the look of every other productivity app, the exact category we are pitching against, and it fights the pencil metaphor. Pick parchment.

| | Parchment (recommended) | Liquid glass |
|---|---|---|
| Fits pen mascot and story | Yes: the pen literally writes on the page | No: a pen on glass is odd |
| Stands out to judges | Yes, distinctive | Looks like Motion/Reclaim |
| Build cost | Low: colours, fonts, one SVG animation | Higher: blur layers, performance tuning on `backdrop-filter` |
| Risk | Can look "old" if overdone; keep layout modern | Low contrast and readability on blurred backgrounds |

**How to make parchment feel modern, not dusty:**
- Warm off-white page (e.g. `#F6F1E7`), ink near-black (`#2B2A28`), one accent ink (e.g. blue `#2F5D9E`) and a red pencil for overdue.
- Clean sans or serif for UI chrome; a **handwriting font only for task titles** (Google Fonts: *Caveat* or *Patrick Hand*).
- Faint ruled lines behind the list; tasks sit on the lines.
- **Signature moment:** when tasks arrive, the pen mascot "writes" each title (SVG text with `stroke-dasharray` / `stroke-dashoffset` animation, ~600 ms each), then ticks the checkbox when done. Done tasks get a hand-drawn strike-through.
- Low-confidence fields look **pencilled in** (grey, lighter weight); confirmed ones are in ink. This shows the AI's confidence without any UI chrome.
- Timeline as a planner page: day columns like a paper diary, busy blocks as grey sticky notes, tasks in the label's ink colour.
- Dark mode (if time): "chalkboard" or night-desk variant, same layout.

**Mascot:** a pen with simple eyes; three states are enough: idle (bobbing), listening (leaning toward the mic while push-to-talk is held), writing (moving across the new task). Keep it a small SVG/CSS animation, no 3D.

---

## 6. Demo script (90 seconds)

1. "Every scheduling tool makes you type the task, the duration, the deadline. Ours just listens."
2. Hold push-to-talk, ramble one of `demo/rambles.md`. The pen writes 3 tasks; chit-chat is ignored.
3. "Schedule it" → tasks slide into the week around the classes from `demo/outlook/week.ics`.
4. Say "my 3pm got cancelled" → everything re-flows; the pen replies out loud.
5. Say "I finished the lab report" → strike-through, schedule updates.
6. Close: "No forms. No fields. Jev turns a ramble into a plan."
