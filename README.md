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
to one task per line. Sample rambles are in `crates/capture/tests/fixtures/rambles.md`.

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

This builds only the server and stages a fresh `deploy\dist\rambl-release\`
(`server.exe`, `static\`, `.env.example`), then opens it in Explorer. On the
server, put a `.env` with `TYPESAFE_API_KEY` and `PORT=3160` next to
`server.exe` once; after that, double-clicking `server.exe` runs Rambl (it works
from its own folder, so `static\`, `data\` and `.env` beside it are found).
To update an existing install while keeping its `data\`:

```powershell
.\deploy\update-server.ps1 -From C:\path\to\staged\ai-todo
```

Stop the existing server before running the update script. It installs the
staged server and static files, keeps the server's data, and starts it again.
Keep `TYPESAFE_API_KEY` in the server's
secret environment, not in the repository.

Hosting (burvy.dev server): the server listens on `PORT` (3160). The router
forwards TCP 3160 to the server PC, the Windows firewall allows it, and
Cloudflare has proxied CNAMEs `rambl` (https://rambl.burvy.dev, the main address) and the old `ai-todo-api`, both sent to port 3160 by one Origin Rule.
`deploy\install-task.ps1` registers a startup task. Check with
`curl http://localhost:3160/api/tasks` on the server.

Design: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). Jev notes: [docs/jev.md](docs/jev.md).

## Project layout

- `crates/judge/`: Jev client and task extraction
- `crates/scheduler/`: deterministic calendar placement
- `server/`: HTTP API and SQLite persistence
- `static/`: web interface
- `art/`: icons, illustrations, brand card, sound and font specs (placeholders; redo guide in [art/GUIDE.md](art/GUIDE.md))
- `deploy/`: Windows build and update scripts

Run checks with:

```powershell
cargo test --workspace
cargo clippy --workspace -- -D warnings
```
