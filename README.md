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
secret environment, not in the repository. See [docs/deploy.md](docs/deploy.md)
for the server layout and startup-task option.

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
