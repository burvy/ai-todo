# Rambl

Rambl turns a voice or text ramble into tasks, deadlines, priorities, and
durations. Jev (TypeSafe's System One model) understands the ramble, and a
deterministic scheduler places tasks into available working hours. The web
interface also accepts updates such as “I finished the lab report” and
re-plans the schedule.

## Run locally

Create a `rambl.env` file in the repository root:

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

Open `http://localhost:3160`. Without `TYPESAFE_API_KEY`, capture falls back
to one task per line. Sample rambles are in `crates/capture/tests/fixtures/rambles.md`.

### Environment variables

Settings are read from `rambl.env` in the working folder (the repository root
locally, the exe's folder on a server). Real environment variables win over it.

| Variable | Default | Purpose |
|---|---|---|
| `TYPESAFE_API_KEY` | none | Jev API key |
| `RAMBL_PORT` | `3160` | HTTP port (falls back to `PORT`) |
| `RAMBL_DEFAULT_TZ` | `America/New_York` | Zone for requests with no `X-Rambl-TZ` |
| `RAMBL_DATA` | `data/rambl.db` | SQLite database path |
| `GOOGLE_CLIENT_ID` | none | Enables Google sign-in; unset means local mode |

## Deploy

See [DEPLOY.md](DEPLOY.md).
