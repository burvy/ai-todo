# ai-todo: team handoff

**Pitch:** "Every scheduling tool asks you to do data entry. This one just listens."
Ramble or drop a file, walk away, and your tasks are written and scheduled for you.

Repo: `github.com/burvy/ai-todo`. Run `git pull origin main` before you start.

**Live:** https://ai-todo-api.burvy.dev (anyone can open it; Chrome or Edge for the mic).
**Run locally:** `.env` with `TYPESAFE_API_KEY=...` in the repo root, then `.\run-local.ps1` → http://localhost:8080 (`-Fresh` = throwaway data).

---

## 1. Status (synced 2026-10-11 ~16:31)

**Works on `main` (all live-tested by the orchestrator against real Jev):**

| Area | What works |
|---|---|
| Web app (`static/`) | Parchment UI (light by default), pen mascot, task list (check off, edit, delete, add), always-on text box, push-to-talk (mic button / space bar), spoken replies, 7-day planner starting today with busy blocks + placed tasks, "Schedule it", "pencilled in" low-confidence fields, sample sentence when the list is empty, phone layout, Night desk theme |
| AI capture | Messy spoken rambles split into clauses; Jev keeps real tasks and drops chit-chat; clean titles ("Study for calc quiz"); relative dates (today, tomorrow, weekdays, "by the weekend", "next week") with 17:00 local deadlines; durations, priority, labels |
| Conversation | `POST /api/message`: add tasks, "I finished X" (marks done, unschedules), "my 3pm got cancelled" (removes busy block, re-plans), "what does tomorrow look like" (jumps planner) |
| Scheduler | Deadline first then priority; ≤2h chunks; 9:00–18:00 local; skips done tasks |
| Calendar import | `POST /api/calendar/import` takes an Outlook/Google `.ics` export and **replaces** the busy blocks, converting `TZID` times to UTC. `demo/outlook/week.ics` loads all 9 events. Persisted, survives restart |
| Persistence | SQLite (`data/ai-todo.db`): tasks and busy blocks survive restarts |
| Ops | CI (fmt, clippy, tests), `run-local.ps1`, `.env` loading, **deployed** at `ai-todo-api.burvy.dev` |

**Known gaps:** PDF/email parsing exists (`crates/capture`) but no upload route yet; no week/month view yet; labels partly keyword-based; no login (everyone shares one list on the live site).

---

## 2. Who owns what

| Owner | Doing now | Task list |
|---|---|---|
| **Frontend** | Week / month planner views, then import buttons | `docs/TASKS-frontend.md` |
| **Backend 2** | Done: email/PDF parsers (#32), calendar import (#37). Nothing open | `docs/TASKS-backend.md` |
| **New teammate** | Email / PDF upload → tasks (uses `crates/capture`) | `docs/TASKS-pdf.md` (start here) |
| **burvy** | Demo rehearsal, redeploys | |
| **AI agents** | Review, merge, live-test, docs | |

**Schedule (deadline ~19:55):**
- **17:30** PRs up for week/month view, `.ics` import, email/PDF.
- **18:30 feature freeze.** After this, only bug fixes. Orchestrator merges, builds the final release, burvy redeploys.
- **18:30–19:30** rehearse the demo twice on the live site.

**Avoid merge conflicts:** both backend people add a route in `server/src/main.rs`. Keep that edit to a few lines and put the logic in your own file (`server/src/ics.rs`, `server/src/email.rs`). Pull `main` before you push.

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
