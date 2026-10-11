# Task list: teammate 2 (backend)

Read `docs/TEAM.md` first (API, status, visual direction). Work top to bottom; do not start a P2 item until your P1 works end to end.

Run the backend locally: `cargo run -p server` → `http://localhost:8080` (put `TYPESAFE_API_KEY=...` in `.env`).

**Goal:** tasks survive restarts (database), then get real Outlook data in: calendar → busy time, emails/PDFs → tasks.

## P0: SQLite database (~1–1.5h), do this first
Today tasks and busy blocks live in memory (`Arc<Mutex<...>>` in `server/`) and are lost on restart. You own persistence; no agent will touch it.
- [ ] Add `rusqlite` with the `bundled` feature (no system install needed). *(5m)*
- [ ] New module `server/src/db.rs`: open the DB at env `DATA_PATH` (default `data/ai-todo.db`, create the folder), run `CREATE TABLE IF NOT EXISTS` for `tasks` (all Task fields: id, title, notes, duration_min, deadline, priority, status, confidence, source, label) and `busy_blocks` (id, start, end, title). *(30m)*
- [ ] Replace the in-memory store: load on startup; insert/update/delete on every `POST /api/tasks`, `/api/capture`, `/api/message`, `PATCH`, `DELETE`, and when busy blocks change. Seed the 3 demo busy blocks only if the table is empty. *(40m)*
- [ ] Add `data/` to `.gitignore`. *(1m)*
- [ ] Test: create a task, restart the server, it is still there. Unit test a round trip with an in-memory DB (`:memory:`). *(15m)*

## P1: calendar import (~1.5h)
- [ ] Create `crates/capture`, add it to workspace `members`. *(10m)*
- [ ] `parse_ics(text) -> Vec<BusyBlock>` with the `icalendar` or `ical` crate. Handle `TZID=America/New_York` → UTC. Test with `demo/outlook/week.ics` (9 events). *(45m)*
- [ ] `POST /api/calendar/import` (multipart file or `{ics_text}`): replace the server's busy blocks, reschedule. *(30m)*
- [ ] Optional: `{ics_url}` to fetch a published Outlook or Google ICS link. *(15m)*

## P1: email / PDF → tasks (~1.5h)
- [ ] `pdf_to_text(bytes)` with the `pdf-extract` crate. *(20m)*
- [ ] `parse_email(text) -> Email {subject, sent, from, body}`: read `From:`, `Sent:`, `Subject:`; cut the quoted reply chain (`-----Original Message-----` or a second `From:`/`Sent:` block), signatures and disclaimers. *(40m)*
- [ ] Wire the PDF upload in `POST /api/capture` (multipart `file`) → `capture_text(body, Source::Pdf, sent)`, using the **email's sent date as the anchor** so "by Friday" means the Friday after it was sent. *(30m)*
- [ ] Unit tests on one real Outlook PDF export with personal data removed.

## P2: only after P1
- [ ] Microsoft Graph test, max 20 minutes: can a `psu.edu` account consent to `Calendars.Read` and `Mail.Read` for our app? Yes → plan device code sign-in. "Need admin approval" → drop it, or demo with a personal outlook.com account.

**Rules:** you own the store and persistence in `server/`. An agent is adding only the `POST /api/tasks` route (manual add) in the next ~15 minutes: pull `main` before you replace the store, and keep other edits to `server/src/main.rs` small. Merge often. Run `cargo fmt` and `cargo clippy --workspace --all-targets -- -D warnings` before every PR.
