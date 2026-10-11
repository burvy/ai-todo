# Task list: teammate 2 (backend)

Read `docs/TEAM.md` first (status, API). Tick `- [x]` in the same PR when you finish an item.

Run: `.env` with `TYPESAFE_API_KEY=...`, then `cargo run -p server` from the repo root → `http://localhost:8080`.

_Synced 2026-10-11 ~14:45 by the orchestrator from what is on `main` (SQLite merged in PR #24)._

## P0: SQLite database ✅
- [x] `rusqlite` (bundled), `server/src/db.rs`, `tasks` + `busy_blocks` tables at `DATA_PATH`
- [x] Every write path persists (add, capture, message, patch, delete, complete, cancel event). The orchestrator added the missing save for `POST /api/tasks` during merge.
- [x] `data/` ignored; restart test passes

## P1: calendar import (~1.5h), do next
- [ ] `parse_ics(text) -> Vec<BusyRow>` (`icalendar` or `ical` crate; put it in a new `crates/capture` or `server/src/ics.rs`). Handle `TZID=America/New_York` → UTC. Test with `demo/outlook/week.ics` (9 events, Oct 12–16). *(45m)*
- [ ] `POST /api/calendar/import` (multipart file or `{ics_text}`): replace busy blocks with `db.save_busy(...)` and the in-memory `busy` list, then return `/api/calendar`-shaped JSON. *(30m)*
- [ ] Optional: `{ics_url}` to fetch a published Outlook or Google ICS link. *(15m)*
- [ ] Tell the frontend the route is live (they add the import button).

## P1: email / PDF → tasks (~1.5h)
- [ ] `pdf_to_text(bytes)` with `pdf-extract`. *(20m)*
- [ ] `parse_email(text) -> {subject, sent, from, body}`; cut quoted reply chains (`-----Original Message-----`, a second `From:`/`Sent:` block), signatures, disclaimers. *(40m)*
- [ ] Multipart `file` on `POST /api/capture` → `capture_text(body, Source::Pdf, sent)`, using the **email's sent date as the anchor**. *(30m)*
- [ ] Test with one real Outlook export (personal data removed).

## P2: only after P1
- [ ] Microsoft Graph test, max 20 min: can a `psu.edu` account consent to `Calendars.Read` + `Mail.Read`? Yes → device code sign-in. "Need admin approval" → drop or use a personal outlook.com account.

**Rules:** keep `server/src/main.rs` edits small; agents may also touch it, so pull `main` first and merge often. The orchestrator reviews and live-tests every PR before merging.
