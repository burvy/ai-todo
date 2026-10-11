# ai-todo Deployment to burvy.dev

> **Scope**: This document studies the deployment path for ai-todo onto the existing burvy.dev Windows server infrastructure. It mirrors the patterns established by `plgroup-server` and `primer-server` (see `primer/docs/DEPLOY.md` and `burvy-dev/README.md`).

---

## 1. Architecture Overview

ai-todo deploys as **one more axum server** on the burvy.dev Windows PC, behind Cloudflare, using the same TLS termination pattern as `plgroup-server` and `primer-server` (hosted inside burvy.dev).

```text
browser ──https──▶ Cloudflare ──https :PORT──▶ ai-todo server (0.0.0.0:PORT)
                        (proxied,      (Origin Rule +        (SQLite in
                         Access)       Full (strict),        C:\burvy\data\ai-todo\)
                                        origin cert)
```

- **Server**: `server.exe` (axum + SQLite via sqlx)
- **Frontend**: Served by the existing burvy.dev site (static files), calls `/api/` on `https://ai-todo.burvy.dev` (or `api-todo.burvy.dev`)
- **Database**: SQLite at `C:\burvy\data\ai-todo\ai-todo.db`
- **Secrets**: Jev API key at `C:\burvy\secrets\ai-todo\jev-api-key.env` (raw key only)
- **Certificate**: Cloudflare Origin Certificate at `C:\burvy\certs\ai-todo.burvy.dev\` (or `api-todo.burvy.dev\`)

---

## 2. Repository Layout (ai-todo)

```
ai-todo/
├── Cargo.toml                    # Workspace root
├── crates/
│   ├── core/                     # Task model, types (todo-core)
│   ├── capture/                  # STT, PDF→text (future)
│   ├── judge/                    # Jev HTTP client (future)
│   └── scheduler/                # Deterministic slot-filler
├── server/                       # axum server (this crate)
│   ├── Cargo.toml
│   └── src/main.rs
├── docs/
│   ├── deploy.md                 # This file
│   └── jev.md                    # Jev API reference
└── .github/workflows/ci.yml      # fmt + clippy + test
```

**Server crate name**: `server` (package), binary `server.exe`

---

## 3. One-Time Server PC Setup

These steps are done by hand on the Windows server (`Z:\3. Rust Projects\burvy-dev` is the deploy source; the server PC runs from `C:\burvy\`).

### 3.1 Cloudflare DNS & Rules

| Step | Action |
|------|--------|
| 1 | **DNS**: Add proxied CNAME `ai-todo` (or `api-todo`) pointing at same target as `plgroup-api` |
| 2 | **Origin Rule**: Hostname `ai-todo.burvy.dev` → destination port **3150** (pick a free port) |
| 3 | **Configuration Rule**: Hostname `ai-todo.burvy.dev` → SSL **Full (strict)** |
| 4 | **Origin Certificate**: SSL/TLS → Origin Server → Create for `ai-todo.burvy.dev`. Save as `C:\burvy\certs\ai-todo.burvy.dev\origin.pem` + `origin.key` |
| 5 | **Router**: Forward TCP **3150** to the server PC (same as 3120, 3130, 3140) |

### 3.2 Google OAuth (if sign-in needed later)

- Google Cloud Console → OAuth client ID → Web application
- Authorised JavaScript origins: `https://burvy.dev` (the site origin, not the API host)
- Client ID goes into `AI_TODO_GOOGLE_CLIENT_ID` in deploy config (not a secret)

### 3.3 Jev API Key

- Create `C:\burvy\secrets\ai-todo\jev-api-key.env` containing only the raw key (same format as `jev-api-key.env` in repo root)
- Server reads `JEV_API_KEY_FILE` env var at start; never copied into app folder

### 3.4 Data Directory

- `C:\burvy\data\ai-todo\` created by install script; SQLite file `ai-todo.db` created on first run

---

## 4. Build (on Dev PC)

Run from `Z:\3. Rust Projects\ai-todo`:

```powershell
cargo build --release -p server
```

Output: `target/release/server.exe`

**Deploy artifacts to copy**:
- `target/release/server.exe`
- (Future) `migrations/` folder for sqlx migrations

---

## 5. Install (on Server PC, Admin PowerShell)

### 5.1 Folder Layout on Server

```
C:\burvy\
├── ai-todo\                  # App folder (updated on each deploy)
│   ├── server.exe
│   └── migrations\           # sqlx migrations (future)
├── data\ai-todo\             # Database + backups + logs
│   ├── ai-todo.db
│   ├── backups\
│   └── logs\
├── secrets\ai-todo\
│   └── jev-api-key.env       # Raw Jev key only
└── certs\ai-todo.burvy.dev\
    ├── origin.pem
    └── origin.key
```

### 5.2 Install Script (to be created: `tools/deploy/install.ps1`)

```powershell
# Creates folders, copies server.exe, writes Caddyfile equivalent (or uses axum TLS directly),
# adds firewall rule "ai-todo (Cloudflare only)" (port 3150, Cloudflare IPs only),
# registers scheduled task "ai-todo server" (at startup, restart on failure),
# starts the server.
```

**Environment set by install script / run script**:

| Variable | Value |
|----------|-------|
| `SERVER_ADDR` | `0.0.0.0:3150` |
| `SERVER_TLS_CERT` | `C:\burvy\certs\ai-todo.burvy.dev\origin.pem` |
| `SERVER_TLS_KEY` | `C:\burvy\certs\ai-todo.burvy.dev\origin.key` |
| `DATABASE_URL` | `sqlite://C:/burvy/data/ai-todo/ai-todo.db?mode=rwc` |
| `JEV_API_KEY_FILE` | `C:\burvy\secrets\ai-todo\jev-api-key.env` |
| `AI_TODO_GOOGLE_CLIENT_ID` | (from config, optional) |
| `RUST_LOG` | `info,sqlx=warn` |

**Note**: Unlike Primer standalone, ai-todo runs **inside burvy.dev** like `plgroup-server` — the server serves HTTPS directly to Cloudflare (no Caddy). `SERVER_TLS_CERT` + `SERVER_TLS_KEY` both set, or neither (HTTP for local dev only).

---

## 6. Run Script (to be created: `tools/deploy/run.ps1`)

```powershell
# Sets env vars above, runs server.exe.
# Registered as scheduled task at startup, with restart-on-failure.
```

---

## 7. Update Flow

1. Dev PC: `cargo build --release -p server`
2. Copy `target/release/server.exe` to `C:\burvy\incoming\ai-todo\` on server
3. Server (admin PowerShell):
   ```powershell
   cd C:\burvy\incoming\ai-todo
   .\tools\deploy\update.ps1   # stops task, backs up DB, replaces exe, restarts task
   ```

---

## 8. Backups

- Daily scheduled task: `C:\burvy\ai-todo\tools\deploy\backup.ps1` → `C:\burvy\data\ai-todo\backups\ai-todo-<date>.db` (keep 14)
- Manual backup: run `backup.ps1` anytime
- Restore: stop task, copy backup over `ai-todo.db`, start task

---

## 9. Sqlx Migrations

- Migrations live in `server/migrations/` (created with `sqlx migrate add`)
- `DATABASE_URL` must be set at build time for `sqlx prepare` (CI does this)
- On server, migrations apply automatically on server start (sqlx runtime)
- **Important**: The live DB exists; for new columns, also run `ALTER TABLE ...` manually or via migration

---

## 10. Health Checks

```powershell
# Local (on server PC)
Invoke-WebRequest http://127.0.0.1:3150/health -UseBasicParsing  # 200 OK
Get-Content C:\burvy\data\ai-todo\logs\server.log -Tail 10

# External (from another network)
Invoke-WebRequest https://ai-todo.burvy.dev/health -UseBasicParsing
```

---

## 10. Safety Checklist (pre-public)

- [ ] `server.exe` listens on `0.0.0.0:3150` only (checked via `netstat -an`)
- [ ] Port 3150 open to Cloudflare IPs only (firewall rule)
- [ ] Cloudflare: Full (strict) + Origin Rule port 3150 + Origin Certificate
- [ ] Jev key outside app folder, never copied/printed (`JEV_API_KEY_FILE`)
- [ ] Google client id (if used) allows `https://burvy.dev` origin
- [ ] Database backups running daily, copied off-machine
- [ ] `RUST_LOG` appropriate for production

---

## 11. Teardown

```powershell
Unregister-ScheduledTask 'ai-todo server' -Confirm:$false
Remove-NetFirewallRule -DisplayName 'ai-todo (Cloudflare only)'
# Data in C:\burvy\data\ai-todo\ stays until manually deleted
```

---

## 12. Port Allocation Reference (burvy.dev server PC)

| Service | Hostname | Cloudflare Port | Local Port | Notes |
|---------|----------|-----------------|------------|-------|
| plgroup-api | `plgroup-api.burvy.dev` | 3120 | 3120 | axum + TLS |
| Primer (standalone) | `primer.burvy.dev` | 3130 | 3131 (Caddy→server) | Caddy in front |
| Primer (inside burvy.dev) | `primer-api.burvy.dev` | 3140 | 3140 | axum + TLS |
| **ai-todo (this project)** | **`ai-todo.burvy.dev`** | **3150** | **3150** | **axum + TLS** |

**Pick the next free port** (3150 shown here; verify nothing else uses it).

---

## 13. Future: Frontend Integration

The ai-todo **frontend lives in the burvy-dev site** (Leptos), not in this repo. The site calls:

```rust
// In burvy-dev/crates/ai-todo-wasm or similar
const API_BASE: &str = "https://ai-todo.burvy.dev/api";
```

CORS: Server must allow `https://burvy.dev` origin (configure via `ALLOWED_ORIGINS` env var).

---

## 14. Related Documents

- `primer/docs/DEPLOY.md` — Full Primer deployment (standalone + hosted modes)
- `burvy-dev/README.md` — Burvy-dev build/deploy scripts (`build.ps1 -Deploy`)
- `docs/jev.md` — Jev HTTP API integration spec
- `hive/board.md` — Overall architecture & repo layout decisions