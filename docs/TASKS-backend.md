# Task list: backend 2 (Rayansh)

Read `docs/TEAM.md` first (status, API). Tick `- [x]` in the same PR when you finish an item.

Run: `.env` with `TYPESAFE_API_KEY=...`, then `.\run-local.ps1` from the repo root → `http://localhost:8080`.

_Synced 2026-10-11 17:05 by the orchestrator (main at #44). **Now: bug fixes and polish. 18:15 anything shaky gets hidden. 18:30 feature freeze.**_

## Done ✅
- SQLite persistence (#24)
- `crates/capture`: `pdf_to_text`, `parse_email` (#32)
- Calendar import: `parse_ics` + `POST /api/calendar/import` (#37); live-tested with `demo/outlook/week.ics`

## NOW, in this order
1. - [ ] **`POST /api/reset`** (do first: it is what we run right before judging). Clear tasks, placements and busy blocks in memory **and** SQLite; return `204`. *(15m)*
2. - [ ] **Reschedule after `/api/calendar/import`** so the response's `placements` are fresh. *(10m)*
3. - [ ] **`pinned_start` for drag-to-reorder.** `pinned_start: Option<DateTime<Utc>>` on `Task` (`crates/core`, `#[serde(default)]`); nullable column in `db.rs` (`ALTER TABLE ... ADD COLUMN` if missing, so the live server's existing DB still opens); accepted by `PATCH /api/tasks/:id` (`null` clears it). Scheduler: place pinned tasks first at exactly that start, full duration; they count as busy for everything else. Tests: pinned task lands at its time; others flow around it. Tell the frontend when it is on `main`. *(45m)*
4. - [ ] Help the new teammate land the PDF upload if they are stuck (you wrote `crates/capture`).

## Rules
- Pull `main` before you push; small PRs; branch `secre/...`. Jim (agent) added `server/src/export.rs` and one route in `main.rs` (#44); don't move it.

## Dropped
- `{ics_url}` import, Microsoft Graph sign-in.
