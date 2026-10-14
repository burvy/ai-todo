# Rambl features

This is the implementation map for the current `origin/main`. Each feature has a
stable reference such as **I.1**. *Planned* means the behaviour is not in the
current code.

## I. CAPTURE

### 1. Ramble to tasks
a. Type a sentence and Rambl creates a task with a title, duration, date, label and priority.
b. Type text in the box labelled "Ramble" and tap the nest (Rambo's dish) at its right end, or press Enter.
c. While the box is empty, Rambo whispers a short prompt in it ("Ramble on…", "What's on
today?", …); the line rotates every ~6 s with a soft fade, and with reduced motion it stays
on one line with no fade.
d. Code: `web/src/capture.rs:CaptureBox, WHISPER_PROMPTS, schedule_whisper`; `web/src/api.rs:capture_text`;
`crates/capture/src/pipeline.rs:capture_text, clean_title`.

### 2. Capture from speech
a. Picking Rambo up starts speech recognition when the browser supports it; there is no mic button.
b. Grab Rambo ("Grab Rambo to listen") and speak; the transcription appears in the capture box
and can be submitted as a task.
c. Code: `web/src/capture.rs:CaptureBox`; `web/src/speech.rs:Session, supported`;
`web/src/api.rs:capture_text`.

### 3. Capture in the user's time zone
a. Words such as "tomorrow", weekdays and clock times are resolved in the user's
time zone rather than the server's.
b. Type a phrase with a date or time, such as `call Sam tomorrow at 3`; the result
appears in the task and week view.
c. Code: `web/src/api.rs:timezone`; `crates/core/src/lib.rs:LocalContext`;
`crates/dates/src/lib.rs:candidates, resolve, pinned_start, due_by, when`.

### 4. Non-tasks are filtered
a. Chit-chat, observations and wishes do not become tasks.
b. Not visible; covered by `crates/judge/src/fake.rs` and
`crates/capture/tests/golden.rs`.
c. Code: `crates/capture/src/segment.rs:split_clauses`;
`crates/capture/src/pipeline.rs:capture_text`; `crates/judge/src/fake.rs`.

### 5. PDF and email capture
a. A PDF, email, or text upload is converted to task candidates; email headers,
quoted replies and signatures are cleaned before judging. An email's send time anchors
words like "tomorrow": a `Date:` with an offset is that instant, while Outlook's bare
`Sent:` time is read in the user's zone. The daily Jev caps still count on the server's day.
b. Not visible; upload through `POST /api/v1/capture`. Covered by tests in
`crates/capture/src/pdf.rs` and `crates/capture/src/email.rs`.
c. Code: `crates/capture/src/pdf.rs:pdf_to_text`;
`crates/capture/src/email.rs:parse_email, Sent`; `server/src/v2/http.rs:file_text`.

### 6. Offline capture fallback
a. Capture still works without a Jev key by using deterministic rules.
b. Not visible; unset `TYPESAFE_API_KEY` and capture text.
c. Code: `crates/judge/src/fake.rs:FakeJudge`;
`crates/capture/tests/golden.rs:rambles_match_the_golden_output`.

## II. TASK LIST

### 1. Shared task list
a. The page lists every task with its status, label, priority, duration and planned time.
b. Open the **Tasks** panel.
c. Code: `web/src/tasks/mod.rs:TaskList`; `web/src/lib.rs:App`; `web/src/api.rs:fetch_snapshot`.

### 2. Add, complete and delete
a. Tasks can be added from the capture box, marked done, and deleted; each change
refreshes the complete snapshot. An edit made from a stale copy of a task (it changed on another
device since this page loaded it) is refused with `409 stale_update`; the page keeps what the server has.
b. Type a task and press **Add**; use the task row's controls to finish or delete it.
c. Code: `web/src/capture.rs:CaptureBox`; `web/src/tasks/mod.rs:TaskList`;
`web/src/api.rs:capture_text, patch_task, delete_task`;
`server/src/v2/http.rs:capture, update, remove`.

### 3. Reorder tasks
a. Dragging task rows changes their saved order.
b. Drag a task row above or below another row: which half of the row it was
dropped on decides which.
c. The whole row is the grab area. Pressing and holding it for ~280 ms picks it
up; there is no separate grip to find.
d. A row answers three gestures from one finger, and which one it is depends on
travel and time, not on where you touched:
- a **tap** — released early, without moving — is the old behaviour: the box
ticks, the title opens the dialog, the x deletes;
- a **press-and-hold** picks the row up to move it, on mouse and on touch
alike;
- a **swipe right** far enough (~64 px, mostly horizontal) crosses the row out
and deletes it. Backed out before that, it snaps home.
e. A finger that slides before the hold is a **scroll**, and the row keeps its
hands off it. That is also what keeps the gestures apart from the page flip: a
swipe left is never this row's to delete, so it reaches the panes and turns the
page (DESIGN-DECISIONS 3).
f. Rows for series occurrences are not reorderable: a drag that starts or lands
on one does nothing (occurrences share the series' row, not a slot of their own —
ARCHITECTURE.md 7.1 lists reordering them as out of scope). A slip dropped where
no row is goes to the end.
g. Code: `web/src/row.rs` (the gesture rules, unit-tested) and its `dom` module
(pointer events); `web/src/tasks/mod.rs:reorder_ids`, the row's handlers;
`web/src/api.rs:reorder_tasks`; `server/src/v2/http.rs:reorder`;
`server/src/v2/app.rs:reorder`.

### 4. Edit through Rambo
a. Dropping Rambo on a task offers actions; the built actions can change priority,
toggle completion, or delete the task.
b. Drag Rambo onto a task, then choose **Edit**, **Done**, or **Delete**.
c. Code: `web/src/rambo.rs:Rambo`; `web/src/api.rs:patch_task, delete_task`.

### 5. Labels and priorities
a. Tasks carry one of six labels and one of four priorities.
b. Inspect a task in the list or use the capture result. Use the label buttons above the
list to filter tasks; the selected filter is remembered in the browser.
c. Code: `web/src/tasks/mod.rs:TaskList, label_name, load_label_filter, save_label_filter`;
`crates/core/src/lib.rs:Label, Priority, Task`;
`crates/judge/src/fake.rs:label_for`; `server/src/v2/store.rs:label_to_stored, priority_to_stored`.

### 6. Persistence
a. Tasks and their order survive a server restart when the SQLite store is enabled.
b. Restart the server and reload the page.
c. Code: `server/src/v2/store.rs:SqliteStore, Store`;
`server/src/v2/mod.rs:app_from_env`.

### 7. Due dates keep their zone
a. A `DueBy` deadline remembers the zone it was set in (`due_tz`), so a deadline set as Friday 17:00 in New York reads Friday 17:00 New York even after the user travels or changes zone; the instant itself never moves. Day-only captures become the end of that day (the user's deadline setting, 23:59 by default) in the capture zone (the same end of day either side of a DST change). Editing a task's instant re-anchors the deadline to the zone the edit is made in — an edit that leaves the instant alone (a title-only save, or a revision that repeats the day the task already has) keeps the stored zone — and a task that stops being a deadline has no zone.
b. Open a task whose due date was set in another zone: the edit box still shows the original zone's time, names the zone next to the date field, and saving keeps the same instant. A deadline typed into the dialog is read as that zone's wall clock; a repeated fall-back hour resolves to its first occurrence. A clock time inside a spring-forward gap names no instant, so saving shows a short inline message and leaves the dialog open instead of doing nothing.
c. Code: `crates/core/src/lib.rs:Task::due_tz, When::DueBy`;
`server/src/v2/app.rs:due_zone, set_when, patch, apply_revision`;
`web/src/tasks/fields.rs:due_display, parse_edited_due, parse_due_input, typed_when`.

## III. WEEK PLANNER

### 1. Scrollable week view
a. The planner shows a configurable number of days and a scrollable 24-hour range.
Each placement block shows the task's title in the handwriting font and its start time,
truncated with an ellipsis when the block is too narrow.
b. Open the **Week** panel and scroll through the grid.
c. Code: `web/src/week.rs:WeekGrid`; `web/src/settings.rs:GearMenu, Settings`.

### 2. One server snapshot
a. The task list and planner use the same server snapshot, so a task cannot show
different data in the two views.
b. Add, complete, delete, import, or reorder a task and compare both panels.
c. Code: `web/src/lib.rs:App`; `web/src/api.rs:fetch_snapshot`;
`crates/core/src/lib.rs:Snapshot`.

### 3. Fixed and flexible placement
a. Tasks with an explicit start stay there; other tasks flow into available
working hours around them. Tasks with a deadline are planned first, earliest
deadline first, so a task higher in the list cannot take the only room a deadline has.
b. Capture a phrase such as `dinner Friday at 7`, then inspect the Friday block.
c. Code: `server/src/v2/app.rs:plan, working_hours`;
`crates/scheduler/src/lib.rs:plan, WorkingHours`.

### 3a. Day-only tasks
a. A task with a day but no time ("call mom Friday") is due at the end of that day
(23:59 by default) and is placed on that day, in the free slot closest to the average
time of day of the times the user has given explicitly; with no timed tasks yet, around
12:00. If the day is full it falls back to the earliest free slot before the deadline.
b. Both defaults, the deadline time and the no-history placement time, are per user and
read and changed with `GET`/`PATCH /api/v1/settings` (`deadline`, `placement`, `HH:MM`).
The settings controls in the gear menu are *(planned)*.
c. Code: `crates/scheduler/src/lib.rs:plan, usual_time, slot_near`;
`crates/core/src/lib.rs:DaySettings`; `crates/dates/src/lib.rs:when_from`;
`server/src/v2/app.rs:patch_day_settings`; `server/src/v2/http.rs:get_settings`.

### 4. Unplaced tasks are reported
a. A task that cannot fit is reported as unplaced rather than silently dropped.
b. Not visible; covered by scheduler and server tests.
c. Code: `crates/core/src/lib.rs:Placement, Plan`;
`server/src/v2/app.rs:plan`.

### 5. Light and dark paper
a. The paper is Light or Dark by the reader's own choice in the gear menu, not by
the operating system's setting. `data-theme="light"` is baked into `web/index.html`
so the first paint is light and never flashes; there is no "System" row.
b. Open the gear menu and choose Dark, then Light, and reload the page.
c. Code: `web/theme.css` (`:root` and `:root[data-theme="dark"]`);
`web/src/settings.rs:GearMenu, apply_theme`.

### 6. Parchment theme
a. The page is aged paper: warm palette, serif headings, a tiled paper texture that switches to a dark variant in dark mode.
b. Look at the page background and the masthead in light and dark mode; with `web/assets/theme/*.svg` deleted it still reads as parchment.
c. Code: `web/theme.css`; spec: `docs/THEME.md`; graphics: `web/assets/theme/paper.svg`, `paper-dark.svg` (placeholders).

## IV. RAMBL, THE MASCOT

### 1. Talk button
a. The pen beside the capture box is Rambo's send and talk control.
b. Click the pen to send, or press and hold it to talk. Only a Rambo resting in
his nest listens: picked up while dazed (below), he writes instead.
c. Code: `web/src/capture.rs:CaptureBox`; `web/src/rambo.rs:Rambo`; `web/src/mascot.rs:State, Event, transition`.

### 2. Drag, daze, write and return
a. The first drag listens. Dropped on the page (not on a task or his nest), Rambo
is dazed: he stays where he landed, tipped over, and does not listen. Dragging a
dazed Rambo writes on the page in his tool's ink. He skitters home on his own
after 8 seconds untouched, or comes home at once when dropped on his nest, and
forgives you a little for putting him back (a small lift in the brain's cheer).
Home again, his writing is tidied away and he listens again.
b. Drag the pen to empty space and release it; drag it again to write; drop it on
the nest, or leave it 8 seconds.
c. Code: `web/src/rambo.rs:Rambo`; `web/src/mascot.rs:transition, listens_on_grab,
put_back, over_nest, DAZED_MS`; `web/src/inklayer.rs:InkLayer`; `web/src/ink.rs`;
`crates/rambl-brain/src/lib.rs:Event::PutBack`.

### 3. Drop actions
a. Dropping Rambo on a task opens a prompt with Edit, Move, Done, Delete and
Never mind actions. Edit opens the task dialog (`<dialog class="task-dialog">` in `web/src/tasks/mod.rs`).
b. Drag the pen onto a task row and choose an action.
c. Code: `web/src/rambo.rs:Rambo`; `web/src/api.rs:patch_task, delete_task`.

### 4. Reduced motion
a. With reduced motion enabled, Rambo returns immediately instead of animating.
b. Enable reduced motion in the operating system and drag Rambo away from home.
c. Code: `web/src/mascot.rs:prefers_reduced_motion`; `web/src/rambo.rs:Rambo`.

### 5. Mascot brain
a. The mascot crate provides deterministic, seeded behaviour without web code; it is
wired into the web mascot by `web/src/brain.rs:Controller`, which turns a brain event
into a pose and a line of Rambo's own dialogue.
b. Not visible; covered by tests in `crates/rambl-brain/src/lib.rs`.
c. Code: `crates/rambl-brain/src/lib.rs:Brain, step, step_with`;
`crates/rambl-brain/src/rng.rs:Seeded`.

### 6. Rambo's voice
a. A line Rambo shows in his speech bubble is also spoken, in one friendly English
voice, a little brisker and a little higher than the browser default. The bubble and
the voice are one thing: the bubble appears when the voice starts and goes 800ms after
it ends. With no voice — muted, the toggle off, or a browser without speech — the
bubble shows alone for 1.5s. He never speaks while the microphone is listening, because
his own voice would come back as a transcript.
b. Let Rambo react to the list (add a task, complete one, let one go overdue) and
listen. Toggle **Rambo talks** in the gear menu: the bubble still shows, the voice does
not.
c. Code: `web/src/voice.rs:say, cancel, audible, pick_voice`, called from
`web/src/brain.rs` where the brain says a line; the toggle is
`settings::Settings::rambo_talks`. Spec: docs/DESIGN-DIRECTION.md 3.

## V. IMPORT AND EXPORT

### 1. Import an iCalendar file
a. An `.ics` calendar becomes ordinary editable tasks with their original times.
b. Choose a file in **Import .ics**.
c. Code: `web/src/settings.rs:GearMenu`; `web/src/api.rs:import_ics`;
`server/src/v2/http.rs:import`; `server/src/v2/ics_import.rs`.

### 2. Re-import without duplicates
a. Importing the same event again updates it by its calendar identifier instead
of creating a duplicate.
b. Import the same `.ics` file twice.
c. Code: `server/src/v2/app.rs:import, ImportedEvent`;
`server/src/v2/store.rs:find_by_uid`; `crates/core/src/lib.rs:Task`.

### 3. Skip cancelled and transparent events
a. Cancelled events and transparent free-time events do not become tasks.
b. Not visible; covered by tests in `server/src/v2/http.rs` and `server/src/v2/ics_import.rs`.
c. Code: `server/src/v2/ics_import.rs`.

### 4. Edit imported-event tasks
a. Imported calendar tasks can be edited locally without changing the source calendar.
b. Choose **Edit** on an imported task; the editor warns that changes will not sync back.
c. Code: `web/src/tasks/mod.rs:TaskList` and the `.calendar-notice` element in the edit
dialog; `crates/core/src/lib.rs:Source`.

### 5. Export the plan
a. The current plan can be downloaded as a standard iCalendar file.
b. Click **Download calendar (.ics)** in **View settings**, or call `GET /api/v1/calendar.ics`.
c. Code: `web/src/api.rs:export_calendar`;
`server/src/v2/http.rs:calendar_ics`; `server/src/v2/ics_export.rs:build_ics`.

### 6. Accepted import formats
a. Calendars use `.ics`; capture accepts PDF, email and text; Outlook `.pst` is
not read.
b. Not visible; covered by the capture and HTTP tests.
c. Code: `server/src/v2/http.rs:file_text`;
`crates/capture/src/pdf.rs:pdf_to_text`; `crates/capture/src/email.rs:parse_email`.

## VI. SETTINGS

### 1. Planner settings
a. You can choose the first hour, last hour and number of days shown in the planner.
b. Open the **gear menu** and change the fields.
c. Code: `web/src/settings.rs:GearMenu, Settings, load, save`.

### 2. Version and calendar export
a. The gear menu displays the Rambl version and build date and links to an iCalendar export.
b. Open the **gear menu**.
c. Code: `web/src/settings.rs:GearMenu`; `web/src/api.rs:export_calendar`.

### 3. Browser time zone
a. Requests carry the browser's IANA time zone so dates and working hours are local.
b. Not visible; capture or load a snapshot from the page.
c. Code: `web/src/api.rs:timezone`; `server/src/v2/http.rs:zone, zone_from_env`.

### 4. Environment configuration
a. The API key, port, database path and default time zone are configured by
environment variables rather than a settings screen.
b. Not visible; see the environment table in `docs/ARCHITECTURE.md`.
c. Code: `server/src/v2/mod.rs:app_from_env`;
`server/src/v2/http.rs:jev_daily_cap_from_env, zone_from_env`.

### 5. Task order and overdue state
a. New tasks appear newest first (docs/ARCHITECTURE.md 4.1). Unfinished tasks past their
`DueBy` deadline remain in the list as overdue and are scheduled into the next free slot
from now, before any normal flexible tasks. Rambo teases about overdue tasks.
b. Add tasks in sequence, then inspect the list and a task whose due time has passed.
c. Code: `server/src/v2/app.rs:create, import, add_drafts` (new task order, newest first),
`server/src/v2/app.rs:sort_tasks`;
`crates/scheduler/src/lib.rs:plan` (overdue detection and priority placement);
`web/src/tasks/mod.rs:is_overdue, TaskList` (overdue badge);
`web/src/brain.rs:142` (emits `TaskOverdue`);
`crates/rambl-brain/src/lib.rs:Event::TaskOverdue`;
`crates/rambl-brain/content/lines.toml` (overdue teasing lines).

## VII. ACCOUNTS

### 1. Google sign-in
a. A deployment with `GOOGLE_CLIENT_ID` accepts a verified Google ID token and
returns a 90-day session.
b. Call `POST /api/v1/auth/google` with `{ "credential": "..." }`.
c. Code: `server/src/v2/http.rs:sign_in_google`;
`server/src/v2/auth.rs:GoogleVerifier, open_session`.

### 2. Account and session controls
a. Signed-in users can identify themselves, sign out, or delete their account and
its tasks.
b. Use `GET /api/v1/me`, `POST /api/v1/auth/signout`, or `DELETE /api/v1/me`.
c. Code: `server/src/v2/http.rs:me, sign_out, delete_me`;
`server/src/v2/auth.rs:session_user, sign_out, delete_user`.

### 3. Local mode
a. Without `GOOGLE_CLIENT_ID`, the server runs as one local user without sign-in.
b. Run the local server and open the page; no login screen is shown.
c. Code: `server/src/v2/auth.rs:LOCAL_USER`;
`server/src/v2/http.rs:router, build_router`.

### 4. Anonymous use
a. Use without an account. A request with no bearer token gets an `X-Rambl-Anon` id the server issued; sending that id back keeps the same tasks. Signing in adopts the anonymous tasks (a calendar event or weekly series the account already has keeps the signed-in copy) and ends the anonymous id. Forged, unknown or malformed ids get a fresh id, and each client address may be issued `RAMBL_ANON_IDS_PER_IP` (default 20) new ids a day.
b. Not visible: the anonymous id is kept in the page and sent on every request. The sign-in reminder is *(planned)*. Anonymous accounts unused for 30 days are deleted with their tasks by the daily backup task, after the backup; signed-in accounts never are.
c. Code: `server/src/v2/http.rs:assign_anonymous_id, CurrentUser`; `server/src/v2/store.rs:adopt_anonymous, touch_anonymous, expire_anonymous`; `server/src/v2/backup.rs:run_cycle_sync`.

### 5. Administration
a. A local console reads commands from the server's terminal: `usage` (today's top users and addresses), `blocked`, `block|unblock user <email|id>`, `block|unblock ip <addr>`, and `plan <email|id> <anon|free|paid>`. A blocked user or address still uses the app, but capture is judged by `FakeJudge`.
b. Type a command into the terminal the server runs in.
c. Code: `server/src/v2/admin.rs`; blocks and usage are in `server/src/v2/store.rs`.

### 6. Web administration
a. With `RAMBL_ADMIN_EMAILS` set (a comma-separated list of emails), a signed-in user whose email is in the list can open `/admin` and block or unblock a user or address, move users between plans, and see today's usage (captures by user and address, and totals per plan; one capture is one Jev call). The page is a single static document served by the server, with no build step; it signs in through the same Google session the main page uses. Unset or empty `RAMBL_ADMIN_EMAILS` turns the whole surface off (404).
b. Open `/admin` in a browser after signing in on the main page.
c. Code: `server/src/v2/admin_web.rs` (data lives in `server/src/v2/admin.rs` and `server/src/v2/store.rs`); routes in `server/src/v2/http.rs:admin_page, admin_of`.
d. Block and plan actions reuse the console's resolution: a user may be named by id or email, and the same `block`, `unblock`, `blocked` and `plan` storage calls serve both.

### 7. Data export
a. Everything the caller owns can be downloaded as one JSON file: the task list with every field. Planner settings are not included; they are stored in the browser, not on the server. The response is `Cache-Control: no-store`, so a cache does not keep a person's task list.
b. Call `GET /api/v1/export`, signed in or with the anonymous id; the download is named `rambl-export-YYYY-MM-DD.json` (the date is the caller's own). A request with no credentials at all is a brand-new anonymous account: it downloads an empty list, not a 401, exactly as the rest of the API treats a signed-out visitor.
c. Code: `server/src/v2/http.rs:export`, reading through `App::tasks` rather than `App::snapshot`, so a download does not run the planner.

## VIII. DATES

### 1. Local context
a. Every date decision uses one instant plus the user's time zone.
b. Not visible; covered by tests in `crates/core/src/lib.rs`.
c. Code: `crates/core/src/lib.rs:LocalContext`; `crates/core/src/clock.rs:SystemClock, FixedClock`.

### 2. Date, time and duration phrases
a. The date parser recognises relative days, weekdays, month dates, clock times
and durations.
b. Not visible; covered by tests in `crates/dates/src/lib.rs` and
`crates/dates/src/phrases.rs`.
c. Code: `crates/dates/src/lib.rs:candidates, resolve`;
`crates/dates/src/phrases.rs:day, time, duration`.

### 3. Daylight-saving changes
a. Local clock times remain on the clock face the user named across daylight-saving
time changes.
b. Not visible; covered by tests in `crates/core/src/lib.rs`.
c. Code: `crates/core/src/lib.rs:LocalContext::at`;
`crates/dates/src/lib.rs:pinned_start`.

## IX. JEV JUDGE

### 1. Candidate selection
a. Code finds candidate dates and titles; Jev selects among them instead of doing
date arithmetic.
b. Not visible; covered by capture golden tests.
c. Code: `crates/capture/src/pipeline.rs:question, title_spans, capture_text`;
`crates/judge/src/jev.rs:questions`.

### 2. Task classification and metadata
a. Jev decides whether each clause is a task and selects its title, date, time,
duration, label and priority.
b. Not visible; covered by `crates/judge/src/fake.rs`.
c. Code: `crates/judge/src/lib.rs:ClauseQuestion, ClauseAnswer`;
`crates/judge/src/jev.rs:JevJudge`.

### 3. Daily fallback cap
a. After the per-user daily Jev cap, capture continues with the deterministic judge.
b. Not visible; configure `RAMBL_JEV_DAILY_CAP` and capture past the limit.
c. Code: `server/src/v2/http.rs:jev_daily_cap_from_env, over_jev_cap`;
`server/src/v2/app.rs:captures_today`.

## X. SCHEDULER

### 1. Deterministic placement
a. The scheduler puts flexible tasks into local working-hour gaps around fixed tasks.
b. Not visible; covered by scheduler tests.
c. Code: `crates/scheduler/src/lib.rs:plan, WorkingHours`;
`server/src/v2/app.rs:plan`.

### 2. Whole-task invariant
a. A task is placed once as a whole; if no complete slot exists it is unplaced.
b. Not visible; covered by tests in `crates/scheduler/src/lib.rs`.
c. Code: `crates/core/src/lib.rs:Placement, Plan`.

### 3. Scheduler v2
a. The `plan()`-based scheduler places fixed and flexible tasks and is covered by
invariant tests.
b. Not visible; track the implementation in `server/src/v2/app.rs:plan`.
c. Code: `server/src/v2/app.rs:plan`; `crates/scheduler/src/lib.rs:plan`;
`crates/scheduler/src/lib.rs:generated_plans_preserve_scheduler_invariants`.

## XI. SERVER API

### 1. Snapshot endpoints
a. Reads and mutations return a complete `Snapshot` containing tasks and placements.
b. Not visible; call `GET /api/v1/snapshot` or any mutating task endpoint.
c. Code: `crates/core/src/lib.rs:Snapshot`;
`server/src/v2/http.rs:snapshot, create, update, remove`.

### 2. Capture, task and calendar routes
a. The API supports capture, task creation and changes, reorder, revise, calendar
import and iCalendar export.
b. Not visible; see the route table in `docs/ARCHITECTURE.md`.
c. Code: `server/src/v2/http.rs:build_router, capture, import, revise_task, calendar_ics`.

### 3. User scoping and errors
a. Requests are scoped to the authenticated user and return structured API errors.
b. Not visible; covered by HTTP tests in `server/src/v2/http.rs`.
c. Code: `server/src/v2/http.rs:CurrentUser, HttpError`;
`server/src/v2/auth.rs:session_user`; `crates/core/src/lib.rs:ApiError`.

## XII. STORAGE

### 1. SQLite persistence
a. SQLite stores tasks, order, imported event identities, users and sessions.
b. Not visible; restart the server and request a snapshot.
c. Code: `server/src/v2/store.rs:SqliteStore, MIGRATIONS, SCHEMA_VERSION`.

### 2. Imported-event identity
a. An imported event keeps its external UID so re-import can update it.
b. Not visible; import the same `.ics` twice.
c. Code: `crates/core/src/lib.rs:Task.external_uid`;
`server/src/v2/store.rs:find_by_uid`; `server/src/v2/app.rs:import`.

### 3. In-memory test store
a. Tests use an in-memory store so they do not write production data.
b. Not visible; covered by server HTTP tests.
c. Code: `server/src/v2/store.rs:MemoryStore`.

### 4. Daily backup
a. Automatic daily SQLite backups are created at startup and every 24 hours, with backups
older than 14 days removed.
b. Not visible; inspect `data/backups/rambl-YYYY-MM-DD.db` after the server starts.
After a successful backup the same task deletes anonymous accounts idle for 30 days (see VII.4);
if the backup fails, nothing is deleted.
c. Code: `server/src/v2/backup.rs:BackupService, run_cycle_sync, run_backup_sync, delete_old_backups`;
`server/src/v2/mod.rs:start_backups`.

## XIII. DEPLOY

### 1. Local run
a. One script starts the server with local settings.
b. Run `.\run-local.ps1` on Windows or `./run-local.sh` on Linux and macOS.
c. Code: `run-local.ps1`; `run-local.sh`; `server/src/v2/mod.rs:app_from_env`.

### 2. Single-file release
a. The release build produces one self-contained `rambl-server.exe` that serves the
web application and API; the former v1 server and legacy routes are removed.
b. Run `.\deploy\build-release.ps1`, then copy the staged release.
c. Code: `deploy/build-release.ps1`; `server/src/main.rs:WEB_DIST`;
`DEPLOY.md`.

### 3. Update without losing data
a. The update procedure replaces the executable while preserving the data directory.
b. Follow the replacement and rollback steps in `DEPLOY.md` on the server.
c. Code: `DEPLOY.md`; `deploy/build-release.ps1`.

### 4. Continuous checks
a. Formatting, linting, workspace tests and the web build run in continuous integration.
b. Not visible; inspect `.github/workflows/ci.yml`.
c. Code: `.github/workflows/ci.yml`.
