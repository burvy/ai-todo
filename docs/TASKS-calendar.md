# Task list: new teammate (calendar import)

Welcome. Your job: make the planner show the user's **real** calendar instead of 3 fake busy blocks, so tasks get scheduled around actual classes and meetings. It is the biggest missing piece for the demo.

## Get set up (15 min)
1. `git clone https://github.com/burvy/ai-todo` and install Rust (`rustup`) if you don't have it.
2. Ask burvy for the Jev key and put it in `.env` in the repo root: `TYPESAFE_API_KEY=...` (git-ignored; never commit it).
3. `.\run-local.ps1 -Fresh` → opens http://localhost:8080. Say or type: *"I need to finish the lab report by Friday and email my TA tomorrow."* Click **Schedule it**.
4. Skim `docs/TEAM.md` (status, API, schedule) and `README.md` (layout).
5. The live site is https://ai-todo-api.burvy.dev.

**Where things are:** `server/src/main.rs` (axum routes, app state), `server/src/db.rs` (SQLite, has `save_busy`), `crates/core` (types), `demo/outlook/week.ics` (test calendar).

## P1: `.ics` import (PR by 17:30)
- [ ] `server/src/ics.rs`: `parse_ics(text) -> Vec<BusyRow>` (`BusyRow` is in `db.rs`) (`icalendar` or `ical` crate, or hand-parse `BEGIN:VEVENT` / `DTSTART` / `DTEND` / `SUMMARY`). Convert `TZID=America/New_York` and floating times to UTC; keep `...Z` times as is. Test: `demo/outlook/week.ics` gives 9 events, Oct 12–16. *(45m)*
- [ ] `POST /api/calendar/import`, body `{ics_text}` (multipart `file` too if quick): replace the busy blocks in memory **and** with `db.save_busy(...)`, then return the same JSON as `GET /api/calendar`. *(30m)*
- [ ] A unit test for `parse_ics` on `week.ics` (count and one start time in UTC).
- [ ] Tell the frontend the route works (they add the "Import calendar" button).

## P2: only if P1 is merged before 18:00
- [ ] `{ics_url}`: fetch a published Outlook or Google calendar link (`reqwest` is already a dependency of `crates/judge`).
- [ ] Recurring events (`RRULE` weekly) expanded for the next 2 weeks.

## Rules
- Branch `yourname/ics-import`, small PR, never push to `main`, never commit `.env` or `data/`.
- Keep the `main.rs` edit to the route only (backend 2 is adding a route too); pull `main` before you push.
- Before pushing: `cargo fmt --all` and `cargo clippy --workspace --all-targets -- -D warnings`. CI runs them plus `cargo test --workspace`.
- **Feature freeze 18:30.** After that, only bug fixes. The orchestrator reviews and live-tests every PR before merging.
