# ai-todo

> Every scheduling tool asks you to do data entry. This one just listens.

Ramble (voice or text) and ai-todo writes your task list: Jev (TypeSafe's System One model) judges which sentences are tasks, their deadline, priority and duration; a deterministic scheduler places them into your free time. Tell it "my 3pm got cancelled" or "I finished the lab report" and the plan re-flows.

## Run it

1. Put the Jev key in `.env` in the repo root (git-ignored):
   ```text
   TYPESAFE_API_KEY=your-key-here
   ```
2. Start the server from the repo root (it serves the web app from `static/`):
   ```powershell
   cargo run -p server
   ```
3. Open `http://localhost:8080`. Hold the mic button or the space bar and ramble, or type in the box. Sample rambles: `demo/rambles.md`.

| Env var | Default | Meaning |
|---|---|---|
| `TYPESAFE_API_KEY` | none | Jev key. Without it, capture falls back to one task per line (no AI). |
| `PORT` | `8080` | HTTP port |
| `DATA_PATH` | `data/ai-todo.db` | SQLite file. `:memory:` = throwaway. Delete the file to reset demo data. |
| `TZ_NAME` | `America/New_York` | Working hours (9:00–18:00) and default deadline time (17:00) |
| `CAPTURE_DEBUG` | off | `1` logs each clause with Jev's task probability, deadline and duration |

## API

All times are UTC ISO strings. A long task can have several placements.

| Call | Body | Returns |
|---|---|---|
| `GET /api/tasks` | | `{tasks:[Task]}` |
| `POST /api/tasks` | `{title, notes?, duration_min?, deadline?, priority?, label?}` | `201` Task |
| `PATCH /api/tasks/:id` | any Task fields, e.g. `{"status":"done"}` | Task |
| `DELETE /api/tasks/:id` | | `204` / `404` |
| `POST /api/capture` | `{text, source?: "voice"\|"text"}` | `{tasks:[Task]}` (new ones) |
| `POST /api/message` | `{text}` | `{reply, intent, tasks, placements, unscheduled, focus_date}` |
| `POST /api/schedule` | | `{placements:[{task_id,start,end}], unscheduled:[{task_id,reason}]}` |
| `GET /api/calendar?from&to` | | `{busy:[{start,end,title}], placements}` |

Task: `id, title, notes, duration_min, deadline|null, priority 0-3, status, confidence 0-1, source, label` (School, Work, Health, Errands, Personal, Other).

`/api/message` intents: `add_tasks`, `complete_task`, `cancel_event`, `query_day`, `other`.

## Layout

| Path | What |
|---|---|
| `crates/core` (`todo_core`) | Task types |
| `crates/judge` | Jev client, sentence/clause splitting, date candidates, title cleanup |
| `crates/scheduler` | Deterministic slot placement |
| `server/` | axum API, SQLite (`db.rs`), intent routing |
| `static/` | Web app (parchment UI) |
| `deploy/` | Windows build, run and install scripts |
| `docs/` | `TEAM.md` (status, owners), task lists, `jev.md`, `deploy.md` |

## Checks (CI runs these on every PR)

```powershell
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Deploy (burvy.dev)

Plan: `ai-todo-api.burvy.dev` → Cloudflare proxied CNAME + Origin Rule → port `3160` on the server PC. Details: `docs/deploy.md`.

1. `deploy\build-release.ps1` builds and stages `deploy\dist\ai-todo\` (server.exe + static).
2. Put the key in `C:\burvy\secrets\ai-todo\jev-key.env`.
3. The startup task runs as the low-privilege `LocalService` account. Grant it access once (admin PowerShell):
   `icacls C:\burvy\secrets\ai-todo /grant "NT AUTHORITY\LOCAL SERVICE:(OI)(CI)R"` and
   `icacls <repo>\deploy /grant "NT AUTHORITY\LOCAL SERVICE:(OI)(CI)M"` (M: it writes the SQLite file).
4. `deploy\install-task.ps1` as Administrator: registers the "ai-todo-server" startup task and opens TCP 3160.
