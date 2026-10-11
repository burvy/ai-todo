# Task list: teammate 2 (backend)

Read `docs/TEAM.md` first (API, status, visual direction). Work top to bottom; do not start a P2 item until your P1 works end to end.

Run the backend locally: `cargo run -p server` → `http://localhost:8080` (put `TYPESAFE_API_KEY=...` in `.env`).

**Goal:** get real Outlook data in: calendar → busy time, emails/PDFs → tasks.

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

**Rules:** keep edits to `server/src/main.rs` small (add routes and calls only); agents also work there, so merge often. Run `cargo fmt` and `cargo clippy --workspace --all-targets -- -D warnings` before every PR.
