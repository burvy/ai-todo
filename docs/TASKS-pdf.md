# Task list: new teammate (email / PDF → tasks)

Welcome. Your job: let a user **drop an Outlook email (saved as PDF) and get tasks out of it**, with deadlines read relative to when the email was sent. The parsing is already written (`crates/capture`, merged in #32); you wire it into the server.

## Get set up (15 min)
1. Accept the GitHub invite, `git clone https://github.com/burvy/ai-todo`, install Rust (`rustup`) if needed.
2. Get the Jev key from burvy **privately** and put it in `.env` in the repo root: `TYPESAFE_API_KEY=...` (git-ignored; never commit it). Without it, capture just makes one task per line.
3. `.\run-local.ps1 -Fresh` → opens http://localhost:8080. Type: *"I need to finish the lab report by Friday and email my TA tomorrow."* Click **Schedule it**.
4. Skim `docs/TEAM.md` (status, API, schedule) and `README.md` (layout).
5. Live site: https://ai-todo-api.burvy.dev

**Where things are:** `server/src/main.rs` (routes, `capture_text`), `crates/capture` (`pdf_to_text`, `parse_email`), `demo/syllabus.pdf` (test PDF).

## P1: upload route (PR by 18:00; if it is not working by then, push a draft PR and we cut it from the demo)
- [ ] Accept a multipart `file` on `POST /api/capture` (axum `Multipart`; keep `{text, source}` JSON working). Keep the `main.rs` edit small; put helpers in `server/src/upload.rs`.
- [ ] PDF → `capture::pdf_to_text` → `capture::parse_email` → `capture_text(body, Source::Pdf, sent)` so "tomorrow" means the day after the email was **sent**. A plain PDF that is not an email: use the whole text, anchor = now.
- [ ] `pdf-extract` can **panic** on odd PDFs: wrap the call in `std::panic::catch_unwind` (or `spawn_blocking` and handle the join error) and return `400 {error}` instead of crashing the server.
- [ ] Return `{tasks:[Task]}` (new ones), same as text capture.
- [ ] Test: `demo/syllabus.pdf`, plus one real Outlook email exported to PDF (remove personal data before committing anything to `demo/`).
- [ ] Tell the frontend the route works (they add a file drop).

## Rules
- Branch `yourname/pdf-upload`, small PR, never push to `main`, never commit `.env` or `data/`.
- Backend 2 is adding a calendar route in `main.rs` too: whoever merges second pulls `main` first.
- Before pushing: `cargo fmt --all` and `cargo clippy --workspace --all-targets -- -D warnings`. CI runs them plus `cargo test --workspace`.
- **Feature freeze 18:30.** After that, only bug fixes. The orchestrator reviews and live-tests every PR before merging.
