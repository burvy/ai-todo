# ai-todo: team handoff

**Pitch:** "Every scheduling tool asks you to do data entry. This one just listens."
Ramble or drop a file, walk away, and your tasks are written and scheduled for you.

Repo: `github.com/burvy/ai-todo`. Run `git pull origin main` before you start.

---

## 1. What is done (on `main`)

| Area | What works | Where |
|---|---|---|
| Server | All API routes, port `8080` (`PORT` env). **Tasks are in memory: lost on restart.** | `server/` |
| AI capture | Text → sentences → date candidates → Jev judges (is it a task, deadline, priority, duration) → clean tasks. Falls back to one task per line if Jev fails. Live-tested. | `crates/judge`, `server/` |
| Conversation | `POST /api/message` routes any message with Jev: add tasks, mark done, cancel an event, ask about a day. Re-plans after every change. Live-tested. | `server/` |
| Scheduler | Nearest deadline first, then priority. Blocks of 2h or less, 9:00–18:00 Eastern (`TZ_NAME`), avoids busy time. | `crates/scheduler` |
| CI | fmt + clippy + tests on every push and PR | `.github/workflows/ci.yml` |
| Deploy | Guide plus build, run and install scripts for `ai-todo-api.burvy.dev:3160` | `docs/deploy.md`, `deploy/` |
| Demo inputs | 4 sample rambles, a syllabus PDF, an Outlook calendar week (`.ics`) | `demo/` |

**Known bugs:** "do X and Y" in one sentence becomes 1 task; tasks marked done still get scheduled; reply says "1 new tasks"; labels come from keyword rules, not Jev.

---

## 2. Who owns what

| Owner | Work | Priority |
|---|---|---|
| **Frontend teammate** | Whole UI (see §4) | P0 now |
| **Teammate 2** | Outlook export → text (PDF/email) and `.ics` → busy blocks. Optional: 20-min Microsoft Graph test (admin consent may block `psu.edu`). | P2 |
| **burvy** | Deploy: Cloudflare CNAME + Origin Rule, router port forward 3160, run `deploy/` scripts. Then demo script and rehearsal. | P1 |
| **AI agents** | Manual add (`POST /api/tasks`) + saving tasks to disk (next), then the bug list above. | P0 |

**Priority order:** P0 a plain to-do list works → P1 AI capture and the schedule show on screen → P2 conversation re-plan, TTS replies, `.ics` → P3 Graph, weather.

**Git rules:** branch `yourname/feature`, small PRs, never push to `main`, never commit `.env` or keys. Run `cargo fmt` and `cargo clippy --workspace --all-targets -- -D warnings` before pushing.

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
| `POST /api/tasks` *(coming)* | `{title}` | `Task` |
| `PATCH /api/tasks/:id` | e.g. `{"status":"done"}` | `Task` |
| `DELETE /api/tasks/:id` | | `204` |
| `POST /api/capture` | `{text, source:"voice"\|"text"}` | `{tasks:[Task]}` (new ones) |
| `POST /api/message` | `{text}` | `{reply, intent, tasks, placements, unscheduled, focus_date}` |
| `POST /api/schedule` | | `{placements:[{task_id,start,end}], unscheduled:[{task_id,reason}]}` |
| `GET /api/calendar?from&to` | | `{busy:[{start,end,title}], placements:[...]}` |

All times are UTC ISO strings; convert to local time in the UI. A long task can have **several placements** (chunks).

**Simplest wiring:** send every user message to `POST /api/message`, speak `reply` aloud (browser `speechSynthesis`), redraw the list from `tasks` and the timeline from `placements`, and jump to `focus_date` if it is set.

---

## 4. Frontend checklist (in order)

1. **List:** tasks with checkbox (PATCH done), delete, low-confidence fields (< 0.6) shown faded or underlined "pencilled in".
2. **Input:** always-visible text box + push-to-talk (hold button or spacebar → `webkitSpeechRecognition` → `/api/message`).
3. **Timeline:** day/week view of busy blocks (grey) and placements (by label colour).
4. **TTS:** speak `reply` (max 2 sentences, it never reads lists).
5. *Stretch:* weather background (Open-Meteo, no key), timeline scrolls to `focus_date`.

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
