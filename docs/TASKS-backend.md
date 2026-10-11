# Task list: backend 2 (calendar import)

Read `docs/TEAM.md` first (status, API, schedule). Tick `- [x]` in the same PR when you finish an item.

Run: `.env` with `TYPESAFE_API_KEY=...`, then `.\run-local.ps1` from the repo root → `http://localhost:8080`.

_Synced 2026-10-11 ~16:10 by the orchestrator. Your email/PDF parsers merged in #32; the upload route for them is now the new teammate's (`docs/TASKS-pdf.md`)._

## P0: SQLite database ✅
- [x] `rusqlite` (bundled), `server/src/db.rs`, `tasks` + `busy_blocks` tables at `DATA_PATH`
- [x] Every write path persists
- [x] `data/` ignored; restart test passes

## P1a: email / PDF parsers ✅ (#32)
- [x] `crates/capture`: `pdf_to_text(bytes)`, `parse_email(text)`, 15 tests

## P1b: calendar import, now (PR by 17:30)
- [x] `parse_ics(text) -> Vec<BusyRow>` in `crates/capture` (or `server/src/ics.rs`). `TZID=America/New_York` and floating times → UTC; `...Z` kept. Test: `demo/outlook/week.ics` gives 9 events, Oct 12–16. *(45m)*
- [x] `POST /api/calendar/import`, body `{ics_text}` (multipart `file` too if quick): replace busy blocks in memory **and** with `db.save_busy(...)`, return the same JSON as `GET /api/calendar`. *(30m)*
- [x] Tell the frontend the route works (they add the "Import calendar" button).
- [ ] Optional, only if merged before 18:00: `{ics_url}` for a published Outlook or Google calendar link.

**Merge order:** you and the new teammate both add a route in `server/src/main.rs`. Whoever merges second pulls `main` first. Keep the `main.rs` edit to the route only.

## P1c: now (burvy, ~17:00), in this order
- [ ] **`pinned_start` for drag-to-reorder** (frontend drags planner blocks). Add `pinned_start: Option<DateTime<Utc>>` to `Task` (`crates/core`, `#[serde(default)]`), a nullable column in `db.rs` (add it with `ALTER TABLE ... ADD COLUMN` if missing so old DBs still open), accept it in `PATCH /api/tasks/:id` (`null` clears it). Scheduler: place pinned tasks first at exactly that start (full duration, even if it overlaps a busy block), treat them as busy for everyone else. Tests: pinned task lands at its time; others flow around it. *(45m)*
- [ ] **`POST /api/reset`**: clear all tasks, placements and busy blocks (memory + SQLite), so the shared live list starts empty before judging. *(15m)*
- [ ] **Reschedule after `/api/calendar/import`** so the response's `placements` are fresh. *(10m)*

## Dropped for today
- Microsoft Graph sign-in.

**Feature freeze 18:30.** After that, only bug fixes. The orchestrator reviews and live-tests every PR before merging.
