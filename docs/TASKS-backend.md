# Task list: backend 2 (email / PDF → tasks)

Read `docs/TEAM.md` first (status, API, schedule). Tick `- [x]` in the same PR when you finish an item.

Run: `.env` with `TYPESAFE_API_KEY=...`, then `.\run-local.ps1` from the repo root → `http://localhost:8080`.

_Synced 2026-10-11 ~15:45 by the orchestrator. Calendar import moved to the new teammate (`docs/TASKS-calendar.md`)._

## P0: SQLite database ✅
- [x] `rusqlite` (bundled), `server/src/db.rs`, `tasks` + `busy_blocks` tables at `DATA_PATH`
- [x] Every write path persists
- [x] `data/` ignored; restart test passes

## P1: email / PDF → tasks, now (PR by 17:30)
- [ ] `pdf_to_text(bytes)` with `pdf-extract`. *(20m)*
- [ ] `parse_email(text) -> {subject, sent, from, body}`; cut quoted reply chains (`-----Original Message-----`, a second `From:`/`Sent:` block), signatures, disclaimers. *(40m)*
- [ ] Multipart `file` on `POST /api/capture` → `capture_text(body, Source::Pdf, sent)`, using the **email's sent date as the anchor** for "tomorrow", "Friday" etc. Return `{tasks:[Task]}` like text capture. *(30m)*
- [ ] Test with one real Outlook PDF export (personal data removed); put it in `demo/` if it is clean.
- [ ] Tell the frontend the route works (they add a file drop).

Put the logic in `server/src/email.rs` (or a new crate); keep the `main.rs` edit to the route only. The new teammate is also adding a route there, so pull `main` before you push.

## Dropped for today
- Microsoft Graph sign-in: no time to finish safely before the deadline.

**Feature freeze 18:30.** After that, only bug fixes. The orchestrator reviews and live-tests every PR before merging.
