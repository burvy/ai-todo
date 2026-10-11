# Deploying ai-todo to burvy.dev

Status: study only. No code or config in `burvy-dev` was changed. Sources read (read only): `burvy-dev/README.md`, `build.ps1`, `menu.ps1`, `Cargo.toml`, `Trunk.toml`, `server/src/main.rs`, `deploy/`.

## 1. How burvy-dev ships servers

- Build: `.\build.ps1 -Deploy` builds the site and each server exe into `burvy-dev\deploy\`. The server list is in `build.ps1` (`$ServerList`). ai-todo is not on it yet.
- Server PC folders (from the README):
  - Data: `C:\burvy\data\<project>\`. Back it up.
  - Certs: `C:\burvy\certs\<host>\cert.pem` and `key.pem` (Cloudflare Origin Certificates).
  - Secrets: `C:\burvy\secrets\<project>\` (for example `jev-key.env`).
- Fronting: Cloudflare sits in front. Each API host is a **proxied CNAME** with an **Origin Rule** that sends traffic to the server's port. The router forwards that TCP port to the server PC. The Windows firewall must also allow it.
- SSL mode: the zone default is **Flexible** (browser to Cloudflare is HTTPS, Cloudflare to origin is HTTP). Hosts that need strict TLS get a **Configuration Rule** "Full (strict)" plus an origin cert.
- Restarts: a scheduled restart already runs on the server PC (the Primer notes say it refreshes certs). I could not see the task's name or settings in the repo.

## 2. Frontend stack

- Rust, Leptos 0.8 (client-side rendering), built with Trunk into `dist/`.
- The site lives on burvy.dev. **I could not find in the repo what serves `dist/` on the server PC** (no Caddy, nginx, IIS or ServeDir config). Ask the burvy-dev owner before step 4.

## 3. Fronting options for ai-todo

| Option | What it needs | Verdict |
|---|---|---|
| A. Own API host, `ai-todo-api.burvy.dev` (same pattern as `primer-api` / `plgroup-api`) | Proxied CNAME, Origin Rule to our port, router forward. Zone default Flexible means no cert is needed. | **Recommended for the demo.** No code change. |
| B. Subpath `burvy.dev/ai-todo/api` | A reverse proxy on burvy.dev. None exists in the repo. | Not for the demo. |

Option A details:
- Our server has no TLS. That is fine under Flexible. Cloudflare to origin is plain HTTP on the LAN path.
- Our server sends CORS headers (`CorsLayer::permissive()`), so a frontend on `burvy.dev` can call `ai-todo-api.burvy.dev` directly.
- Simplest for the demo: serve the frontend from the same server (`./static`), so the page and API share one origin and CORS is not needed at all.

## 4. Exact steps (option A, single origin)

1. **Build the server** (in the ai-todo repo):
   `cargo build --release -p server`
   Output: `target\release\server.exe`.
2. **Build the frontend** and copy its `dist\` output to a `static\` folder next to the exe.
3. **Create the deploy folder** on the server PC: `C:\burvy\ai-todo\`, containing `server.exe` and `static\`.
4. **Secrets**: put the key in `C:\burvy\secrets\ai-todo\jev-api-key.env`. Set it as the env var `TYPESAFE_API_KEY` in the service (step 7). Never copy it into the repo. The server does not read it yet; the judge crate will.
5. **Pick a port** (for example `3160`, not used by any project in the README). Set `PORT=3160` in the service environment.
6. **Network**:
   - Router: forward TCP `3160` to the server PC.
   - Windows firewall: allow inbound TCP `3160`.
   - Cloudflare: proxied CNAME `ai-todo-api` (same target as `plgroup-api`), and an Origin Rule for `ai-todo-api.burvy.dev` with destination port `3160`. Leave the zone SSL at Flexible.
7. **Run as a service**: a Task Scheduler task "At startup", running `C:\burvy\ai-todo\server.exe` with `PORT` and `TYPESAFE_API_KEY` set, and restart on failure. Match the existing burvy-dev task pattern if one exists (I could not confirm its name). No NSSM or other service wrapper is in the repo.
8. **Check**:
   - On the server PC: `curl http://localhost:3160/api/tasks` returns `{"tasks":[]}`.
   - From outside: `https://ai-todo-api.burvy.dev/api/tasks`.
   - Open `https://ai-todo-api.burvy.dev/` for the frontend.

## 5. Code changes

- **burvy-dev: none needed** for option A.
- **ai-todo: none needed** for option A. Our server already serves `./static` and listens on `0.0.0.0:$PORT`.
- Option B would need a reverse-proxy route on burvy.dev. Not drafted, since it is out of scope for the demo.

## 6. Open items

- Confirm what serves burvy.dev's `dist\` today, so the ai-todo frontend can be added to it instead, if you prefer one origin.
- Confirm the port (`3160`) is free on the router and the server PC.
- Confirm the existing scheduled task's name and settings, so the ai-todo task matches it.
- ATD-4 is also assigned to kevin-mv4249xz in `tasks.json`. Check for overlap before merging.
