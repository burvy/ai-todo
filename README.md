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

See [DEPLOY.md](DEPLOY.md).

Design: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). Jev notes: [docs/jev.md](docs/jev.md).

## Project layout

- `crates/judge/`: Jev client and task extraction
- `crates/scheduler/`: deterministic calendar placement
- `server/`: HTTP API and SQLite persistence
- `web/`: Leptos web interface
- `art/`: icons, illustrations, brand card, sound and font specs (placeholders; redo guide in [art/GUIDE.md](art/GUIDE.md))
- `deploy/`: Windows release build script

Run checks with:

```powershell
cargo test --workspace
cargo clippy --workspace -- -D warnings
```
