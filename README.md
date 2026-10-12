# Rambl

Rambl turns a voice or text ramble into tasks, deadlines, priorities, and
durations. Jev (TypeSafe's System One model) understands the ramble, and a
deterministic scheduler places tasks into available working hours. The web
interface also accepts updates such as “I finished the lab report” and
re-plans the schedule.

## Run locally

Create a `.env` file in the repository root:

```text
TYPESAFE_API_KEY=your-key-here
```

Start the server from the repository root:

```powershell
.\run-local.ps1
```

Or on Linux/macOS:

```sh
./run-local.sh
```

Open `http://localhost:8080`. Without `TYPESAFE_API_KEY`, capture falls back
to one task per line. Sample rambles are in `demo/rambles.md`.

### Environment variables

| Variable | Default | Purpose |
|---|---|---|
| `TYPESAFE_API_KEY` | none | Jev API key |
| `PORT` | `8080` | HTTP port |
| `TZ_NAME` | `America/New_York` | Working-hours and deadline timezone |

`DATA_PATH` can override the SQLite database path; its default is
`data/ai-todo.db`.

## Deploy

On a development machine, pull the desired revision and stage a release:

```powershell
.\deploy\build-release.ps1
```

This builds `server.exe` and copies it with `static/` into
`deploy\dist\ai-todo\`. Copy that staged directory to the server without
overwriting its `data/` directory, then update the running installation:

```powershell
.\deploy\update-server.ps1 -From C:\path\to\staged\ai-todo
```

Stop the existing server before running the update script. It installs the
staged server and static files, keeps the server's data, and starts it again.
Keep `TYPESAFE_API_KEY` in the server's
secret environment, not in the repository.

Hosting (burvy.dev server): the server listens on `PORT` (3160). The router
forwards TCP 3160 to the server PC, the Windows firewall allows it, and
Cloudflare has a proxied CNAME `ai-todo-api` with an Origin Rule to port 3160.
`deploy\install-task.ps1` registers a startup task. Check with
`curl http://localhost:3160/api/tasks` on the server.

Design: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). Jev notes: [docs/jev.md](docs/jev.md).

## Project layout

- `crates/judge/`: Jev client and task extraction
- `crates/scheduler/`: deterministic calendar placement
- `server/`: HTTP API and SQLite persistence
- `static/`: web interface
- `deploy/`: Windows build and update scripts

Run checks with:

```powershell
cargo test --workspace
cargo clippy --workspace -- -D warnings
```
