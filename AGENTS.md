# AGENTS.md

Guidance for AI agents working in this repository (Rambl, repo name `ai-todo`).

## Read first

- **`docs/ARCHITECTURE.md` is the single design document and it is authoritative.**
  Where code disagrees with it, the doc wins; when changing behavior, update the doc
  first, then the code. It defines the data model, API, invariants, and the v2 phase plan.
- `docs/jev.md` documents the Jev (TypeSafe System One) HTTP API: question primitives
  (`noul`/`choice`/`score`), limits, and the exact request shapes the judge sends.

## Two systems coexist (v1 → v2 migration in progress)

- **v1 (hackathon build, live in production)**: `server/src/main.rs` + `static/`.
  This is what `make run` actually serves today. Fix production bugs here.
- **v2 (rewrite, in progress)**: `crates/{core,dates,judge,capture,scheduler}` and
  `web/` (Leptos CSR). Recent work lands here per the phase plan in
  `docs/ARCHITECTURE.md` §10. `static/` and the old routes get deleted in phase 4.
- Root `src/main.rs` is a stub ("Hello, world") — vestigial root package, ignore it.

## Commands

```sh
make run      # cargo run -p server (the v1 server; reads .env, port 8080)
make check    # fmt + lint + test — run before pushing
make fmt      # cargo fmt --all
make lint     # cargo clippy --workspace --all-targets -- -D warnings
make test     # cargo test --workspace
```

- **CI (`.github/workflows/ci.yml`) is stricter than the Makefile**: it uses
  `--all-features` on both clippy and test. Run
  `cargo clippy --workspace --all-targets --all-features -- -D warnings` and
  `cargo test --workspace --all-features` before pushing to avoid CI-only failures.
- Local run scripts (with useful flags):
  `./run-local.sh --fresh` (in-memory DB, clean demo), `--debug` (fast debug build
  instead of ~100s `--release`), `--port N`, `--log-capture` (`CAPTURE_DEBUG=1`).
  `run-local.ps1` is the Windows counterpart.
- Deploy (Windows): `deploy/build-release.ps1` stages only the server;
  `deploy/update-server.ps1 -From <staged>` updates the server install keeping its `data/`.
- The `web/` crate builds with Trunk (`trunk build --release` → `web/dist`); it needs
  the `wasm32-unknown-unknown` target. Its tests are `wasm_bindgen_test` and run in a
  browser, not under plain `cargo test`.

## Environment / secrets

- `TYPESAFE_API_KEY` — Jev API key. Server loads `.env` itself (dotenvy).
  `run-local.sh` also falls back to `jev-api-key.env` in the repo root (untracked,
  either `TYPESAFE_API_KEY=...` or the bare key). **Never commit a key or print its value.**
- Without a key, capture falls back to one-task-per-line via `FakeJudge` — everything
  still works and tests run without network.
- v1 env vars: `PORT`, `TZ_NAME` (default `America/New_York`), `DATA_PATH`
  (default `data/ai-todo.db`, `:memory:` for throwaway), `CAPTURE_DEBUG`.
  The v2 design renames these to `RAMBL_DATA` / `RAMBL_DEFAULT_TZ` — check
  `server/src/main.rs` for what the current code reads.

## Hard rules (from ARCHITECTURE.md; each prevents a real v1 bug)

- **All time goes through `LocalContext { now, tz }`.** Nothing outside
  `core::clock` may call `Utc::now()` or `.date_naive()` on a UTC value. The
  user's timezone arrives as the `X-Rambl-TZ` header (IANA name from the browser).
- **Jev only selects among candidates that code found** — it never generates dates
  or titles itself. Candidate extraction lives in `crates/dates`; turning a chosen
  candidate into an instant is pure code.
- **Every mutating API call returns the full `Snapshot`** (tasks + plan). The client
  never computes placements; the server is the only planner.
- **Locks are never held across a call that locks again** — one `App` method per
  request, load/mutate/re-plan/return inside one critical section.
- **Imported calendar events are ordinary tasks** (`source: Calendar`,
  `when: At(start)`, `external_uid` = iCalendar UID). There is no "busy" or grey
  type; re-import updates by UID instead of duplicating.
- Every kept bug fix carries a regression test; tests are the definition of done.

## Testing patterns

- `FakeJudge` (deterministic, picks first candidate) makes capture tests run end to
  end without network. Live Jev tests are `#[ignore]` and run by hand.
- Server tests use `MemoryStore` + `FixedClock` + `FakeJudge` via
  `tower::ServiceExt::oneshot`. A shared check runs on every returned snapshot
  (placement shown for open tasks, none for done/deleted).
- `crates/dates` has a phrase table at fixed instants including DST transitions and
  Sunday 21:00 New York (already Monday in UTC).
- `RAMBL_SAMPLE_ICS` points at a personal calendar file outside the repo for a local
  `#[ignore]` test — never commit that sample.

## Conventions

- Rust edition 2024, workspace resolver 2. Clippy warnings are errors.
- Crate naming: the core crate's package name is `todo-core` (dir `crates/core`);
  the web crate is `rambl-web`.
- `crates/core` must stay wasm-safe (serde + chrono only — no tokio, no IO).
- Commits are conventional-style with a scope, e.g. `feat(core): ...`,
  `fix(capture): ...`, squashed per PR with the PR number appended.
- Dev machine is Windows; the shell may lack coreutils (`sed`, `head`), so prefer
  `rg`, and read files with tooling instead of shell pipelines. Scripts and paths
  must work on both Windows (PowerShell) and Linux (CI, deploy target).
