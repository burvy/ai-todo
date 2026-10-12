# Rambl features

Every feature, numbered so it can be pointed at in conversation ("Feature IV.2"). The
specification is [ARCHITECTURE.md](ARCHITECTURE.md); where the spec and the code disagree,
the spec wins.

**Status.** *Built* means it is in the merged code. *(planned)* means it is specified but
not built yet. Features are numbered within their section, so a reference is a Roman
numeral and a number, for example `II.3`.

The user-facing sections come first, then the backend that makes them work.

---

## I. CAPTURE

**I.1 Ramble to tasks**
a. One sentence of anything you ramble about becomes one task, with its title, date,
length and label worked out for you.
b. Type `call mom tomorrow at 3` in the box at the top of the page and press Add.
*(planned)* The text box renders and the server endpoint works, but the box is not yet
wired to it; `web/src/lib.rs:37` only suppresses the form's default submit.

**I.2 Speaking your ramble**
a. You talk instead of typing, and the browser turns your speech into the same text a
ramble would have been.
b. Hold Rambl and speak, or hold the Space bar with your cursor outside the text box.
*(planned)* Both triggers are built and show "I'm listening", but nothing transcribes
yet; voice recognition is the next piece of work on this page.

**I.3 Ramble works in your own time zone**
a. "Tomorrow" means tomorrow where you are, not where the server is.
b. Not visible; covered by tests in `crates/dates/src/lib.rs:194` — on Sunday night at
21:00 in New York, "tomorrow" resolves to Monday, not Tuesday.

**I.4 Chit-chat is thrown away**
a. Observations, wishes, feelings, past events and filler are not turned into tasks, so
what is left in your list is only what you actually have to do.
b. Not visible; covered by tests in `crates/judge/src/fake.rs:247` and
`crates/capture/src/segment.rs`.

**I.5 Upload a printed PDF**
a. A printed page — a syllabus, a mailout, a calendar print — is read and turned into
tasks.
b. Upload a `.pdf` to the capture endpoint. Not visible; covered by tests in
`server/src/v2/http.rs:758`. The parser lives in `crates/capture/src/pdf.rs`.

**I.6 Upload an email**
a. An exported email becomes a task, with its headers understood, its quoted reply chain
and signature removed, and its sent date used as the anchor for relative deadlines.
b. Upload a `.eml` to the capture endpoint. Not visible; covered by tests in
`crates/capture/src/email.rs:215`.

**I.7 Off with the date words**
a. When the date has been captured separately it does not repeat inside the task title.
b. Not visible; covered by tests in `crates/capture/src/pipeline.rs:321` — "Lab report
due Wednesday" becomes "Lab report".

**I.8 Capture works with no AI key**
a. With no Jev key configured the app still captures, using fixed keyword rules, so you
can use and test it entirely offline.
b. Leave `TYPESAFE_API_KEY` unset. Not visible; covered by tests in
`crates/capture/tests/golden.rs` and `crates/judge/src/fake.rs`.

---

## II. TASK LIST

**II.1 The list**
a. Every task you have, with its title, how long it should take and the time it is
scheduled for.
b. The "Tasks" panel on the left of the page.

**II.2 One screen, one truth**
a. The list and the week are drawn from the same single server response, so a task can
never show one time in the list and a different time on the calendar.
b. Change nothing and compare: the time beside a task in the list is the time in the
week. Not visible; covered by tests in `web/src/lib.rs:226` and
`server/src/v2/http.rs:388`.

**II.3 Adding a task by hand**
a. You can type in a task instead of rambling it, for the things you already know about.
b. Not visible yet; the endpoint (`POST /api/v1/tasks`) is built and covered by tests in
`server/src/v2/http.rs:449`, but the form is *(planned)*.

**II.4 Checking a task off**
a. Marking a task done takes it off the calendar and keeps it in the list.
b. Tick a task off. *(planned)* The endpoint (`PATCH` with `done`) is built and covered by
tests in `server/src/v2/http.rs:506`; the tick box is *(planned)*.

**II.5 Editing a task**
a. Change a task's title, length, label or time by hand and the plan is recomputed at
once.
b. Edit a task. *(planned)* The endpoint is built and covered by tests in
`server/src/v2/http.rs:116`; the editing UI is *(planned)*.

**II.6 Deleting a task**
a. A task you no longer want is gone from both the list and the week.
b. Delete a task. *(planned)* The endpoint is built and covered by tests in
`server/src/v2/http.rs:534`; the delete control is *(planned)*.

**II.7 Reordering by hand**
a. Drag tasks into the order you want them in and that order is kept.
b. Drag a task row up or down. *(planned)* Rows are already marked draggable in
`web/src/lib.rs:149` and the endpoint is covered by tests in
`server/src/v2/http.rs:561`, but the drag is *(planned)*.

**II.8 Six labels and four priorities**
a. Every task is filed under School, Work, Errands, Health, Personal or Other, and given
Low, Medium, High or Critical.
b. Every task carries its label and priority. Not visible; covered by tests in
`crates/judge/src/fake.rs:189`.

**II.9 Tasks survive a restart**
a. Your tasks are still there after you close the app or restart the machine.
b. Restart the server and reload. *(planned)* SQLite storage is built
(`server/src/v2/store.rs`), but the server falls back to memory unless `RAMBL_DATA` is
set — see `server/src/v2/mod.rs:18`. This is being fixed.

---

## III. WEEK PLANNER

**III.1 The seven-day week**
a. A seven-day grid shows where everything landed this week.
b. The "Week" panel on the right of the page.

**III.2 The server does the planning**
a. The server, not your browser, decides where each task goes; the page only draws the
answer it was given.
b. Not visible; every placement in the week came from `POST` or `GET /api/v1/snapshot`,
and the server is the only thing that schedules (`server/src/v2/app.rs:369`).

**III.3 Pinned tasks sit exactly where you said**
a. A task with an explicit day and time is always shown at that instant, and other tasks
plan around it.
b. Say "dinner with Sam Friday at 7" and it appears at 19:00 that Friday. Not visible;
covered by tests in `server/src/v2/http.rs:467`.

**III.4 A floating task is never late**
a. A task with a day but no time is placed before the end of that day, or 17:00 local,
whichever the rule resolves to.
b. Say "lab report due Wednesday" and it lands before Wednesday evening. Not visible;
covered by tests in `crates/dates/src/lib.rs:377` and `crates/capture/src/pipeline.rs:363`.

**III.5 Only what fits gets placed**
a. A task that cannot fit around your fixed events is honestly reported as unplaced
rather than dropped or squeezed in.
b. Fill a day, then look at the week's unplaced list. Not visible; covered by tests in
`server/src/v2/app.rs:404`.

**III.6 Light and dark follow your system**
a. The page uses your operating system's light or dark setting, with no switch to find.
b. Change your system theme. Not visible; set in `web/index.html` via `color-scheme`.

---

## IV. RAMBL, THE MASCOT

**IV.1 Rambl is the talk button**
a. The pen character in the small circle next to the text box is how you send: tap to
send, hold to talk.
b. The circle beside the text box. Not visible; built in `web/src/lib.rs:53`.

**IV.2 Picking him up**
a. Rambl can be picked up and dropped anywhere on the page.
b. Drag the Rambl circle. Not visible; built in `web/src/lib.rs:86`.

**IV.3 He falls and comes back**
a. Dropped on empty space, he falls to the bottom of the window and skitters back to his
circle.
b. Drag Rambl onto blank space and let go. Not visible; built in `web/src/lib.rs:60` with
the timings in `web/index.html`.

**IV.4 He always lets you talk**
a. Wherever he is, holding him talks, and so does holding Space when your cursor is
outside a text field; dragging him away never breaks voice.
b. Hold Rambl anywhere, or hold Space. Not visible; built in `web/src/lib.rs:15` and
`web/src/lib.rs:92`, and the rule "Talking always works" holds in every state of the
machine (`web/src/mascot.rs:42`).

**IV.5 Asking about what you dropped him on**
a. Drop Rambl on a task or a week block and he stays there and asks, a bit annoyed, what
you want to do about it.
b. Drag Rambl onto a task row or a block in the week. Not visible; built in
`web/src/lib.rs:143` and `web/src/lib.rs:180`.

**IV.6 His little menu**
a. The bubble he asks in offers Edit, Move, Done, Delete and Never mind, and choosing any
of them sends him home.
b. Drop him on a task and pick from the bubble. *(planned)* The five buttons render
(`web/src/lib.rs:105`) but they only send him home; wiring them to the endpoints is the
open TODO in that block.

**IV.7 He respects reduced motion**
a. If your system asks for reduced motion, he does not fall or skitter; he simply
reappears in his circle.
b. Turn on "reduce motion" in your system settings. Not visible; built in
`web/src/mascot.rs:21` and `web/index.html`.

**IV.8 He is a tested state machine**
a. Rambl's behaviour is five states and the rules between them, tested without a browser,
so it cannot quietly break.
b. Not visible; covered by tests in `web/src/mascot.rs` (Home, Dragging, Falling,
Returning, Asking).

---

## V. IMPORT AND EXPORT

**V.1 Import a calendar**
a. Upload an `.ics` file and every event in it appears as a task you can edit, move,
reorder, finish or delete like any other.
b. Upload a `.ics` to `POST /api/v1/import`. Not visible; covered by tests in
`server/src/v2/http.rs:639`.

**V.2 No grey blocks**
a. There is no separate kind of un-editable calendar entry; an imported event is an
ordinary task from the moment it arrives.
b. Open an imported event after importing. Covered by tests in `server/src/v2/app.rs:141`.

**V.3 Re-importing does not duplicate**
a. Importing the same calendar twice updates the tasks it already made instead of making
a second copy.
b. Import the same file twice. Not visible; covered by tests in
`server/src/v2/http.rs:665` and `server/src/v2/store.rs:868`.

**V.4 Cancelled and transparent events are skipped**
a. Events you cancelled, and events marked "free time", do not become tasks.
b. Import a calendar containing either. Not visible; the skipping is in
`server/src/ics.rs:63` and covered by tests there. *(planned)* only the cancelled case is
covered through the v2 route.

**V.5 Export the plan as a calendar**
a. Download your week as a standard iCalendar file that any calendar app can open.
b. `GET /api/v1/calendar.ics`. Not visible; covered by tests in
`server/src/v2/ics_export.rs:86` and `server/src/v2/http.rs:865`.

**V.6 Right file for the job**
a. Calendars come in as `.ics`; emails as PDF or text; `.pst` is never read.
b. Not visible; the accepted set is in `server/src/v2/http.rs:249`.

---

## VI. SETTINGS

**VI.1 Your time zone comes from your browser**
a. Rambl uses the time zone your computer reports, not a server default.
b. Not visible; every request carries `X-Rambl-TZ`, built in `web/src/api.rs:215` and
`server/src/v2/http.rs:64`.

**VI.2 A sensible default if the browser will not say**
a. If no time zone is sent, the server falls back to a configured default rather than
guessing wrong.
b. Not visible; `RAMBL_DEFAULT_TZ` in `server/src/v2/http.rs:76`. *(planned)* the deploy
scripts do not set this variable yet.

**VI.3 Configuration is by environment variable**
a. The API key, port, database path and default time zone are set with environment
variables, not in a settings screen.
b. Not visible; listed in `docs/ARCHITECTURE.md` section 7 and implemented in
`server/src/v2/mod.rs:17`.

**VI.4 The Jev API key**
a. Without a key, capture falls back to fixed rules; with one, Jev reads the ramble.
b. Not visible; `TYPESAFE_API_KEY`, built in `crates/judge/src/jev.rs:30`.

---

## VII. DATES AND TIME

**VII.1 One context for every time decision**
a. Everything that depends on "now" goes through one object holding the instant and the
user's zone, so nothing can accidentally reason in the server's time.
b. Not visible; `LocalContext` in `crates/core/src/lib.rs:15`, enforced by tests at
`crates/core/src/lib.rs:177`.

**VII.2 Day phrases**
a. "Today", "tomorrow", "Wednesday", "next Friday", "this weekend", "by Monday",
"Oct 30" and "2026-10-30" are all understood.
b. Not visible; covered by tests in `crates/dates/src/lib.rs:193` through
`crates/dates/src/lib.rs:298`.

**VII.3 A weekday said on that day means today**
a. Saying "Tuesday" on a Tuesday before 17:00 means today; after 17:00 it means next
Tuesday.
b. Not visible; covered by tests in `crates/dates/src/lib.rs:231` and
`crates/dates/src/lib.rs:238`.

**VII.4 Bare hours are read as a person would say them**
a. After "at", 1 to 6 means the afternoon and 7 to 12 means the morning, while "3pm" and
"15:45" are taken literally.
b. Not visible; covered by tests in `crates/dates/src/lib.rs:301`.

**VII.5 Lengths**
a. "an hour", "half an hour", "90 minutes", "2 hours", "45 min" all become a task length.
b. Not visible; covered by tests in `crates/dates/src/lib.rs:316`.

**VII.6 Clocks change, and Rambl copes**
a. On the day the clocks go forward or back, a time you name still lands on the clock
face you said.
b. Not visible; covered by tests in `crates/core/src/lib.rs:187` and
`crates/core/src/lib.rs:199`.

**VII.7 Nothing else reads the clock**
a. Only one file in the whole program is allowed to ask what time it is, which is what
keeps the tests repeatable.
b. Not visible; `crates/core/src/clock.rs:13`.

---

## VIII. THE JEV JUDGE

**VIII.1 Jev picks, code decides**
a. Jev chooses from the date, time, length and title options the code found in your
words; only the code turns that choice into a real date.
b. Not visible; the rule is stated in `crates/judge/src/lib.rs:1` and enforced in
`crates/capture/src/pipeline.rs:201`.

**VIII.2 Six questions per clause**
a. For each thing you said, Jev answers whether it is a task, which title, which day,
which time, which length, which label and how urgent, all in one go.
b. Not visible; the questions built in `crates/judge/src/jev.rs:71`.

**VIII.3 Your local time as context only**
a. Jev is told what time it is for you so it can read "tonight" correctly, but it is
never asked to do arithmetic on it.
b. Not visible; `crates/capture/src/pipeline.rs:62`.

**VIII.4 A fixed judge for tests**
a. A deterministic stand-in for Jev lets the whole pipeline be tested end to end without
the network.
b. Not visible; `crates/judge/src/fake.rs:30`, used by `crates/capture/tests/golden.rs`.

**VIII.5 Live Jev checks run by hand**
a. The tests that talk to the real Jev service are marked so they never run by accident.
b. Not visible; run with `cargo test -p judge -- --ignored` with `TYPESAFE_API_KEY` set,
see `crates/judge/src/jev.rs:289`.

---

## IX. THE SCHEDULER

**IX.1 Fixed events hold their time**
a. A pinned task always sits at the instant you gave it and blocks the time around it.
b. Not visible; `server/src/v2/app.rs:376`.

**IX.2 Everything else flows around them**
a. Tasks with no fixed time are placed into the gaps left by your fixed events.
b. Not visible; covered by tests in `server/src/v2/http.rs:467`.

**IX.3 Working hours are local**
a. Placing happens between 09:00 and 17:00 in your own time zone, and the scheduler works
out each day's offset itself so clock changes do not shift your week.
b. Not visible; `server/src/v2/app.rs:24`. *(planned)* the scheduler is being rewritten
around a new `plan()` entry point.

**IX.4 One task, one slot**
a. A task is placed once, whole; it is never chopped into pieces across the week.
b. Not visible; a task that needs two slots is reported unplaced instead
(`server/src/v2/app.rs:415`).

---

## X. THE SERVER API

**X.1 One snapshot from every call**
a. Every call that changes anything returns the complete picture — all tasks and the
whole plan — so the page can never show something half-updated.
b. Not visible; `Snapshot` in `crates/core/src/lib.rs:137`, returned by every mutating
endpoint.

**X.2 The v1 API surface**
a. Nine endpoints cover capture, the task list, editing, reordering, revising in plain
language, importing and exporting.
b. `docs/ARCHITECTURE.md` section 6 for the full table; routes built in
`server/src/v2/http.rs:83`.

**X.3 One client, one place**
a. The web app makes every request through a single typed module and keeps the result in
one signal.
b. Not visible; `web/src/api.rs:204`.

**X.4 Errors that say what went wrong**
a. A failed call returns a short machine-readable code and a human sentence, with a
status that fits.
b. Not visible; covered by tests in `server/src/v2/http.rs:336` and
`server/src/v2/http.rs:359`.

**X.5 No half-written requests**
a. The service loads, changes, re-plans and answers inside one lock, so two simultaneous
requests cannot interleave.
b. Not visible; `server/src/v2/app.rs:1` and `server/src/v2/http.rs:104`.

---

## XI. STORAGE

**XI.1 Tasks are kept in a database**
a. Tasks, their order and their calendar identity are stored in SQLite so they outlive
the process.
b. Not visible; `SqliteStore` in `server/src/v2/store.rs:223`.

**XI.2 Imported events keep their identity**
a. Each imported event remembers the calendar's own identifier for it, which is how a
re-import knows to update rather than duplicate.
b. Not visible; `external_uid` in `crates/core/src/lib.rs:108`, looked up in
`server/src/v2/app.rs:151`.

**XI.3 Reordering happens in one go**
a. A reorder is a single transaction, so a failure part-way cannot leave the list
half-renumbered.
b. Not visible; `server/src/v2/app.rs:280`.

**XI.4 An in-memory store for tests**
a. Tests run against a store that never touches a disk, so they are fast and cannot
corrupt anything.
b. Not visible; `MemoryStore` in `server/src/v2/store.rs:125`, used throughout
`server/src/v2/http.rs`.

---

## XII. RUN AND DEPLOY

**XII.1 Run it locally**
a. One script starts the server with the right settings on your machine.
b. `.\run-local.ps1` on Windows, `./run-local.sh` on Linux or macOS. See `README.md`.

**XII.2 Build a release**
a. A script builds the server and stages a folder you can copy to the machine it runs on.
b. `.\deploy\build-release.ps1`. See `README.md`.

**XII.3 Update a running install without losing data**
a. The update script installs the new build and keeps the existing database.
b. `.\deploy\update-server.ps1`. See `README.md`.

**XII.4 Everything is checked on every push**
a. Formatting, lints, the whole test suite and the web build all run in CI, so a broken
build cannot land quietly.
b. Not visible; `.github/workflows/ci.yml`.

**XII.5 Serving the web app**
a. The server serves the built web app itself, so there is only one thing to deploy.
b. *(planned)* the built app is `web/dist`; the server still serves the old `static`
folder today (`server/src/main.rs:1708`).

---

## Notes for keeping this file honest

- Built against `main` at `6bbb9de`.
- Anything marked *(planned)* is in [ARCHITECTURE.md](ARCHITECTURE.md) but not in the
  code yet. The largest gaps are the wiring between the page and the server (nothing on
  the page calls the API yet, apart from loading the snapshot), voice transcription, and
  Rambl's five menu buttons.
- The full API table is in [ARCHITECTURE.md](ARCHITECTURE.md) section 6; the API notes
  for Jev are in [jev.md](jev.md).