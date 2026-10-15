# Rambl features

This is the implementation map for the current `origin/main`. Each feature has a
stable reference such as **I.1**. *Planned* means the behaviour is not in the
current code.

## I. CAPTURE

### 1. Ramble to tasks
a. Type a sentence and Rambl creates a task with a title, duration, date, label and priority.
b. Type text in the box labelled "Ramble" and tap the nest (Rambo's dish, in the bottom-right
corner of the screen) or press Enter. The box is a form with a submit button, so only a real key
press submits; a script's synthetic Enter event does not.
c. While the box is empty, Rambo whispers a short prompt in it ("Ramble on…", "What's on
today?", …); the line rotates every ~6 s with a soft fade, and with reduced motion it stays
on one line with no fade.
d. Code: `web/src/capture.rs:CaptureBox, WHISPER_PROMPTS, schedule_whisper`; `web/src/api.rs:capture_text`;
`crates/capture/src/pipeline.rs:capture_text, clean_title`.

### 2. Capture from speech
a. Holding Rambo still for a moment (`LISTEN_HOLD_MS`, a long press) starts speech
recognition when the browser supports it; there is no mic button. A drag only carries him.
b. Press and hold Rambo ("Grab Rambo to listen") and speak, or press Space or Enter on him;
the transcription appears in the capture box and is submitted as a task when you let go.
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
a. Chit-chat, observations and wishes do not become tasks. With no Jev key the offline
judge decides by the clause's shape: it is a task when it needs, owes or must something, or
when it is an instruction. An instruction is a clause whose first real word, after fillers
such as "please", "also" or "just" and asks such as "can you", is not a pronoun, article,
preposition, auxiliary or conjunction, a gerund or a past form: so "walk the dog", "fix the
sink" and "feed the cat" are tasks and "I think", "for biology" and "the design presentation is coming up"
are not. The asks help only when an action verb follows: "can you call mom" is a task,
while "can you believe it" with a mind verb like "believe" or "imagine" stays chit-chat.
A mind verb still starts an errand when it hands the action on: "can you remember to call mom"
and "can you tell mom to call" are tasks, while "can you remember when we moved" stays chit-chat.
"Remind me to" and "don't forget to" are skipped like fillers; what follows must still be an
instruction of two words or more ("Groceries" alone is too little).
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
a. Capture still works without a Jev key by using deterministic rules (see 4 for what counts
as a task; 7 for the lead-in cut; VIII.2 for times).
b. Not visible; unset `TYPESAFE_API_KEY` and capture "Walk the dog tomorrow": one task.
c. Code: `crates/judge/src/fake.rs:FakeJudge, is_instruction`;
`crates/capture/tests/golden.rs:rambles_match_the_golden_output`.

### 7. Lead-ins are cut from the title
a. "Add a task to buy milk", "please schedule a reminder to call mom" and "can you add a task:
water the plants" become the tasks "Buy milk", "Call mom" and "Water the plants". The cut
needs a make verb, a task noun and a link word ("to", "called", "about", ...) or a colon, so
"add milk to the list" and "a task force meeting" keep their words. Speech to text often
writes "add a task" as "attittask", "attit task", "add it task" or "at a task": "attittask",
"attitask", "attit task" and "add it task" are unmistakable and cut even without a link word; "at a task" and the rest only count
before a link word or colon. Jev is shown the grammar's cut and each candidate start word and
picks; the offline judge takes the grammar's.
b. Say or type `attittask buy milk`: the task is "Buy milk".
c. Code: `crates/capture/src/leadin.rs:strip, options, heard_right`;
`crates/capture/src/pipeline.rs:title_spans`; fixtures `crates/capture/tests/fixtures/leadins.txt`.

## II. TASK LIST

### 1. Shared task list
a. The page lists every task with its status, label, priority, duration and planned time.
b. Open the app: the task list is the first page, with no header, tabs or heading above it
(see 8).
c. Code: `web/src/tasks/mod.rs:TaskList`; `web/src/lib.rs:App`; `web/src/api.rs:fetch_snapshot`.

### 2. Add, complete and delete
a. Tasks can be added from the capture box, marked done, and deleted; each change
refreshes the complete snapshot. An edit made from a stale copy of a task (it changed on another
device since this page loaded it) is refused with `409 stale_update`; the page keeps what the server has.
b. Type a task and tap the nest or press Enter (there is no Add button); swipe a row right to
finish it, or use the row's x to delete it.
On a phone the **+** at the bar's left (a torn scrap beside the box) adds a task by hand: it opens
the edit sheet, headed "New task", on a blank form (no title, 30 minutes, anytime, no repeat). Save
needs a title and a length and makes the task with `POST /api/v1/tasks` (or a weekly series, if
days are ticked); the task exists only from Save, so Cancel and Esc leave nothing behind. Its Done
and x buttons are hidden, since there is nothing yet to finish or delete. A desk, with room to
ramble, has no +. Code: `web/src/capture.rs:CaptureBox` (the button), `web/src/tasks/mod.rs:blank_form,
new_task_draft, DRAFT_ID`, `web/src/api.rs:create_task`.
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
- a **tap** — released early, without moving — is the old behaviour: the title
opens the dialog from the title, the x deletes;
- a **press-and-hold** picks the row up to move it, on mouse and on touch
alike;
- a **swipe right** (mostly horizontal) **scribbles the row out**: the row stays
put and Rambo's ink loops across its words under your finger, in the page's
ink, each row's loops its own (seeded by the row). Let go past
`SCRIBBLE_COMMIT_SHARE` of the row (or flick past `row::SWIPE_PX`) and the ink
finishes across and the task is **done**: it tears off and flies to the pin; short of that, the ink lifts away. Done and
delete are different things: a done task is kept, a deleted one is gone after the undo.
A **done task** (the swipe, Rambo's Done button) leaves its row, crossed out, and flies
to the **pin** at the top right of the list's page, where it hangs as a vertical strip
down the page's right edge, always in view, as a stack: the newest strip on top with its title,
the older ones under it, each `PIN_EDGE_PX` further down (`PIN_EDGES` of them show), so only
their edges show. Only the top strip can be tapped or dragged. Tap it and it is taped back (not
done): its row flies back from the strip, the rows below slide down to make room, and the next
strip slides up into the top place. Drag the top strip into the bottom-left corner (`BIN_CORNER_FRACTION`
of the screen) and it crumples and drops off the page; the task is deleted for good (`DELETE`, no undo).
Released anywhere else it swings back to the pin. (A repeat's done day is skipped, so it
does not hang.)
There is no checkbox and no board. Every delete (the row's x, Rambo's x, the sheet's x)
leaves a **paper scrap** flying from the row to the same pin, hanging for `SCRAP_UNDO_MS`
of time the page is in view; up to `SCRAP_MAX` (two quick deletes leave two, each taping
back its own task; a third sends the oldest away). Tap one and the task is taped back as
it was (`POST /api/v1/tasks/restore`; a repeat's day has its skip cleared), otherwise it
slides away. Paper slides and never fades. The rows below a row that left slide up to close the
gap (`ROW_CLOSE_MS`), never jump. A deleted scrap lands below the stack, never on it. Times and counts are in `web/src/tuning.rs`;
the pin is `web/src/scrap.rs`.
e. A finger that slides before the hold is a **scroll**, and the row keeps its
hands off it. That is also what keeps the gestures apart from the page flip: a
swipe left is never this row's to delete, so it reaches the panes and turns the
page (DESIGN-DECISIONS 3).
f. On a phone the whole page turns **like a book's, hinged on its left edge** (the default
Page turn; `Fast` is the corner bend below): swipe left
on the list, or swipe right on the planner (outside its grid, or on empty paper while the
grid is at its earliest day), or tap the note at the page's foot ("planner on the next page", "task list on the previous
page"). The list is the leaf: forward, its
right edge lifts toward you and swings over the spine, and the planner shows beneath;
back is the same leaf coming back: it swings in from the spine and lies down again over the
planner, the forward turn played in reverse (the same poses, in the opposite order). The edge keeps pace with the
finger, the leaf darkens as it turns away, and it throws a shadow on the page under it
(black in light and dark). Letting go past a share of the width or in a flick finishes
the turn; short of that the leaf falls back and rebounds a little. The note's turn runs on a fixed time, slow, quick, slow, the same both ways. With reduced motion it
slides. Geometry, the leaf's spring and decisions: `web/src/flip.rs` (tested); browser
side: `web/src/flip_turner.rs`; numbers in `web/src/tuning.rs` (`FLIP_*`), looks in
`web/theme.css` `--flip-*` and `.flip-*`. The leaf moves on a spring with a speed of its own,
so a flick carries on at the speed of the finger. The two pages lie stacked in one cell
(the list over the planner, `main[data-pane]` names the one showing), so there is no scroll
row between them. The gear's **Page turn** setting picks `Realistic` (this) or `Fast`, the
older corner bend (`web/src/fold.rs`, `fold_turner.rs`, `--bend-*`): swipe the corner and
the page folds along the line halfway to the finger. `web/src/turner.rs` picks one for each
gesture; in `Fast` too the list is the only leaf and back is the fold run the other way.
The pages fill the screen edge to edge on a phone (safe areas are page padding). On the
earliest day the grid takes `touch-action: pan-y pan-right` and `overscroll-behavior-x:
none`, so a swipe to the right is the page's (never the grid stretching) and a swipe to
the left still scrolls the days.
g. Rows for series occurrences are not reorderable: a drag that starts or lands
on one does nothing (occurrences share the series' row, not a slot of their own —
ARCHITECTURE.md 7.1 lists reordering them as out of scope). A slip dropped where
no row is goes to the end.
h. Code: `web/src/row.rs` (the gesture rules, unit-tested) and its `dom` module
(pointer events); `web/src/tasks/mod.rs:reorder_ids`, the row's handlers;
`web/src/api.rs:reorder_tasks`; `server/src/v2/http.rs:reorder`;
`server/src/v2/app.rs:reorder`.

### 4. Edit through Rambo
a. Dropping Rambo on a task offers actions: **Edit** opens that task's edit sheet,
**Done** toggles completion, **Move** turns the page to the planner to place it (III.7). There is no delete in the bubble; tapping off it is never mind. On Edit
Rambo also listens: say the change ("make it Friday at 3") and the server's revise
call (Jev) applies it, then the sheet shows the result. A repeating task's sheet
edits its series, which revise cannot, so he does not listen there.
b. Drag Rambo onto a task, then choose **Edit**, **Move** or **Done**, or tap off the bubble.
c. Code: `web/src/rambo.rs:Rambo`, which sets `edit_request` for
`web/src/tasks/mod.rs:TaskList` to open its sheet; `web/src/mascot.rs:Choice, asking_dismissed_by`; `web/src/api.rs:patch_task`.
d. The sheet's days and times are picked on the paper's own pickers (handwritten
numbers, keyboard reachable, the paper's 12- or 24-hour clock), never the browser's:
`web/src/pickers.rs:PaperDate, PaperTime, PaperDateTime`; numbers in `web/src/tuning.rs`.

### 5. Labels and priorities
a. Tasks carry one of six labels and one of four priorities.
b. The list has no label filter: the filter tabs were removed. Labels stay on the task data.
c. Code: `web/src/tasks/mod.rs:TaskList`;
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

### 8. The page: no chrome, always-visible capture box, nothing pops
a. There is no app header, no To do / Planner switcher, no "To do" or "Planner" heading and no
All / School / Personal / Other filter tabs: the content explains itself. The task list's
page holds the list and the pin (top right); the planner's page holds the grid and the gear
(top right). On a phone each page fills the screen edge to edge (safe areas are padding) and
ends in the note that names the other page; on a desk (wider than 700 px) both pages sit side
by side and the notes are hidden. Labels and priorities stay on the task data.
b. On a phone the capture box is a bar fixed at the bottom, and each page is exactly the
screen above it (`100dvh` less the bar), so a long list scrolls inside its page and the box
is always in view; the document itself never scrolls. On a desk the box sits at the foot of
the page in the page's flow.
c. The nest is one box in the bottom-right corner of the capture bar, square-ish and fully
on screen (`--nest-w` by `--nest-h`). Rambo sits in it, his tip behind the nest's front rim,
which is a second layer drawn over him (`.nest::before` is the back, `.nest-rim` the front).
Tapping the nest submits the box. On a phone, when the keyboard opens the capture bar rides
up on top of it, with the nest and Rambo in it, and goes back down when it closes.
d. Nothing pops in or out. The only thing on main that fades is ink (it appears over `INK_FADE_MS`
and fades out over `INK_FADE_OUT_MS`); the one other thing allowed to fade is Rambo's
tape patch and the tear it mends (item h). Everything else moves: a done or
deleted row flies to the pin, a taped-back task flies back from it, the rows below slide up
to close a gap (`ROW_CLOSE_MS`), a crumpled strip drops away, and Rambo's ask bubble grows
out of him and folds back into him over `ASK_FOLD_MS` before it leaves the page, at full
opacity throughout.
e. Try it: open the app on a phone and see no heading; add enough tasks to overflow and see
the box stay put; swipe a row right and watch it travel to the pin.
f. On a desk the capture line is **pinned** (it stays at the top of the screen while the page
scrolls, as the phone's bar stays at the bottom) and is drawn as **a piece of ripped scratch
paper** laid on the desk: torn on every side with a shadow, its torn ends kept at their own
shape however wide the line is, and a faint writing line (`web/assets/scratch/`, one file per
theme, see its `ABOUT.md`). The pinned slot is the page's own paper, so the list's rows pass
under it and never show above the scrap. Rambo writes on it: as you type his nib stands on the
writing line at the end of the words you have typed, on every keystroke (a flight already under
way, or him already writing, re-aims at the new end and the wait before he goes home starts
again), and on a scroll while he writes. The caret is measured in the line's own font, inside
the torn ends, and follows the text as a long line scrolls (`pen::caret_x`,
`tasks::write::text_on_line`). The box has no browser spellcheck. A phone keeps the bar at its
bottom edge. A second writing line so a long ramble wraps, and a slight tilt, are *(planned)*; the
paper cut-out x that scribbles the text out and lays down a fresh scrap is *(planned)*.
g. Code: `web/src/lib.rs:App, PageFoot`; `web/src/week.rs:WeekGrid` (the planner page, the
gear); `web/src/scrap.rs` (the pin); `web/theme.css` (`.capture`, `.nest`, `.nest-rim`, the
phone block with `100dvh`); numbers in `web/src/tuning.rs`.
h. **The page rips.** A swipe to the right on bare paper of the list page (a finger on a phone, a
mouse drag on a desk; not on a task, and mostly level: `RIP_MIN_PX`, `RIP_MAX_SLOPE`) tears the
page along the swipe. The tear is a real hole, a ragged slit up to `RIP_GAP_PX` wide: the
planner page shows through it live (its own slips and lines, not a picture), in shade, so the
hole reads as a hole even where the planner is empty paper. The planner is brought to the
tear while it lasts (`rip::slide_px`, `lift_px`, `TEAR_SLIDE_MS`): the first day column with a
slip (else today's) is put under the tear, and an hour line (a slip, on a phone) across it. On a
desk, where the two pages are a spread side by side, it slides under the list and back, its
column's middle under where the tear starts, an hour line to the tear's level (never more than
half a gap). On a phone the planner lies under the list already; the column's left edge is
brought `TEAR_EDGE_IN_PX` inside the tear's start and its nearest slip across the tear, so a
day edge and a slip's edge and first word show in the slit. While a tear lasts the planner's hour
lines and day edges are drawn firmer (`main.torn` in theme.css).
The tear follows the finger, never goes back, stops short of the pin's column, and never
touches a task's words: a press too near a row tears nothing (`rip::lane`). One tear at a
time. When the finger lets go Rambo notices, hops to the tear and tapes it: a strip of tape
every `TAPE_EVERY_PX` across it is laid from one end over `TAPE_LAY_MS`, `TAPE_AT_MS` after
the finger lets go. After `TEAR_HOLD_MS` the tape and the tear fade away together over
`TEAR_FADE_MS`, the one thing besides ink that fades; Rambo has hopped home by then. Only a
resting Rambo (in his nest) goes; a busy one leaves the repair to the page. He is grumpy
while he mends (`Pose::Sulking`). The tear is cleared by a one-shot on the engine's clock
(`engine::every`), with everything else that moves. Try it: swipe or drag right across the
empty part of the list page.
Code: `web/src/rip.rs` (the rules: `is_rip`, `lane`, `Tear::outline`, `sheet_path`,
`slide_px`; pure and tested), `web/src/rip_ui.rs:Ripper, TearLayer` (the pointer, the hole,
the tape), `web/src/mascot.rs:State::Mending, Event::Mend`, `web/theme.css` (`main.torn`,
`.tear-layer`, `.tape`); numbers in `web/src/tuning.rs` (`RIP_*`, `TAPE_*`, `TEAR_*`,
`MEND_MS`).

### 9. The list paper is lightly lined
a. The list page is ruled like a notebook's: each row sits on a faint line, and the lines
run on down the page below the last row at the same spacing, to the foot of the page. The
planner keeps its own grid. The lines are a token (`--paper-line`, fainter than the rules
that divide things, with its own dark value) and the spacing is `--line-pitch`, which is
also each row's least height, so a one-line row is exactly one line. A title that wraps
makes its row taller; the lines below it continue from where it ends. The ruled paper fills
what is left of the page and never makes it longer.
b. Add a few tasks: the rows stand on lines that carry on to the page's foot, in light and
dark paper, on a phone and on a desk.
c. Code: `web/theme.css` (`--paper-line`, `--line-pitch`, `.task-list li`,
`.task-list::after`).

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
The gear menu has them under **A day with no time**: **Due by** and **Placed near**, each a
time on the paper's own picker (in the paper's clock, no Clear: a day always has both). A pick is
saved at once with `PATCH /api/v1/settings` (only that field), the plan is read again so the
planner moves its day-only tasks, and a refused one (a 00:00 deadline) puts the old time back
and shows the reason. The picker keeps the minute it was on, so choosing 5 from 11:59 pm gives
5:59 pm; pick the minute too.
c. Code: `web/src/settings.rs:DayTimes, DayTime`; `web/src/api.rs:fetch_day_settings, patch_day_settings`;
`crates/scheduler/src/lib.rs:plan, usual_time, slot_near`;
`crates/core/src/lib.rs:DaySettings`; `crates/dates/src/lib.rs:when_from`;
`server/src/v2/app.rs:patch_day_settings`; `server/src/v2/http.rs:get_settings`.

### 4. Unplaced tasks are reported
a. A task that cannot fit is reported as unplaced rather than silently dropped.
b. Not visible; covered by scheduler and server tests.
c. Code: `crates/core/src/lib.rs:Placement, Plan`;
`server/src/v2/app.rs:plan`.

### 4a. Taking a task off the planner
a. A task can be taken off the planner on purpose: it stays in the list with its
deadline, and the scheduler leaves it alone until it is placed again, because it is
reported in `Plan::off_planner` rather than as unplaced. This outranks a pin, so an
`At` task taken off the planner leaves the week too. Set it with
`PATCH /api/v1/tasks/{id}` and `{"off_planner": true}`.
b. Not visible yet: the bubble button that calls it is (planned), and the UI that
shows why a task is in the list but not on the week is (planned). Covered by
scheduler, store and server tests.
c. Code: `crates/core/src/lib.rs:Task::off_planner, Plan::off_planner`;
`crates/scheduler/src/lib.rs:plan`; `server/src/v2/app.rs:patch`;
`server/src/v2/store.rs:MIGRATION_12`; `docs/ARCHITECTURE.md` 4.2.

### 5. Light and dark paper
a. The paper is Light or Dark by the reader's own choice in the gear menu, not by
the operating system's setting. `data-theme="light"` is baked into `web/index.html`
so the first paint is light and never flashes; there is no "System" row.
b. Open the gear menu and choose Dark, then Light, and reload the page.
c. Code: `web/theme.css` (`:root` and `:root[data-theme="dark"]`);
`web/src/settings.rs:GearMenu, apply_theme`.

### 5a. The desk lamp
a. A small desk lamp is the dark-mode switch. Tapping it pulls its cord: a sheet of the
other paper is laid down from the top edge over the page, the page's own paper is swapped
under it, and the sheet is taken away on the next frame. The page is swapped out; nothing
fades. The lamp is dark by day and lit by night (by night its light falls on the corner of
the page), and its own light changes at the tap. The gear menu's Light and Dark choices
use the same switch, so there is one behaviour; only with reduced motion does the paper
change at once. It is the same stored Theme setting as ever, and the page still never
follows the OS. On a desk the lamp stands on the bottom edge of the screen in the bottom
left corner, clear of the capture box and the gear; on a phone it stands at the foot, just
above the capture bar.
b. Tap the lamp (or pick Light or Dark in the gear menu) and watch the new paper slide
down; reload and see the choice kept.
c. Code: `web/src/lamp.rs:DeskLamp, switch_to`; `web/theme.css` (`.desk-lamp`,
`.lamp-sheet`); `web/src/tuning.rs:LAMP_SLIDE_MS`. The art: `web/assets/lamp/ABOUT.md`.

### 6. Parchment theme
a. The page is aged paper: warm palette, serif headings, a tiled paper texture that switches to a dark variant in dark mode.
b. Look at the page background in light and dark mode; with `web/assets/theme/*.svg` deleted it still reads as parchment.
c. Code: `web/theme.css`; spec: `docs/THEME.md`; graphics: `web/assets/theme/paper.svg`, `paper-dark.svg` (placeholders).

### 7. Move from the ask bubble
a. Choosing **Move** in Rambo's ask bubble turns the page to the planner, where the
task waits to be placed. A note at the planner's top asks where it goes: tap an hour
to start it there (snapped to the quarter hour; a mouse outlines the slot first), or
tap Rambo's offer of the next free time, the earliest quarter hour from now inside
working hours where the task's length fits without overlapping another slip. **Leave
it**, Esc or turning back to the list leaves the task where it was; the page stays
on the planner. The note floats over the planner's foot (the grid never moves for it)
and slides up into place and back down out of sight rather than popping; it never fades. One day of a
repeating series moves as that day only, as a dragged slip does. Choices for a slip
Rambo is dropped on in the planner (move, longer or shorter, take it off) are
(planned).
b. Drop Rambo on a task, choose Move, then tap an hour or the "Next free" scrap.
c. Code: `web/src/rambo.rs:Rambo` sets `move_request`; `web/src/lib.rs:App` turns
the page; `web/src/placing.rs:PlacingNote, next_free`;
`web/src/week.rs:WeekGrid, place_at`.

### 8. A new task on an empty hour
a. Drop Rambo on an hour of the planner with nothing placed there and he stays on it,
listening, for a new task that starts then (snapped to the quarter hour): the capture
box takes the focus and asks for it ("A task for today 14:30"). Say it, and it is sent
as you stop speaking; or type it and press Enter or tap the nest. Tapping or clicking
anywhere off the capture box, him included, or Esc, is never mind: nothing is created and
the box empties. Dropped on a placed task, or at a time the clock skips that day, he
does not start one.
b. Turn to the planner, drag Rambo onto an empty hour, then speak or type a title.
c. Code: `web/src/rambo.rs:Rambo, free_hour_under`; `web/src/week.rs:hour_at, free_at,
hour_prefill_text`; `web/src/capture.rs:CaptureBox, noted`;
`web/src/mascot.rs:State::Noting, Event::DropHour, asking_dismissed_by`.

## IV. RAMBL, THE MASCOT

### 1. Talk button
a. Rambo, in his nest in the bottom-right corner, is the send and talk control.
b. Tap the nest to send, or press and hold Rambo still to talk. Only a Rambo resting in
his nest listens: picked up while dazed (below), he writes instead. While he listens the
light is turned down a little over everything but him, his nest's rim and his bubble, and
comes back up when he stops; the dim never takes a touch.
c. Code: `web/src/capture.rs:CaptureBox`; `web/src/rambo.rs:Rambo`; `web/theme.css:.listening-dim`; `web/src/mascot.rs:State, Event, transition`.

### 2. Drag, daze, write and return
a. A drag carries him without listening (a still hold listens), riding just above
your finger or cursor so it never covers him. Every move he makes from one spot to
another is a few brisk hops, never a glide, and never instant: first he notices, his face reacts, then he goes. Dropped gently on the page (not on a task or his nest), he
hops home. Let go of him mid-flick and he is thrown: he slides on with the flick's
speed and bounces off the screen's edges. Only a smack into an edge hard enough
dazes him: his ink splats off the edge and he lies tipped over where he hit, not
listening. Dragging a dazed Rambo writes on the page in his tool's ink, and so does
holding a finger anywhere on the page while he is dazed: he comes to the finger and
writes where it goes. After 3
seconds untouched he skitters home in small hops, each leaving a dot of ink, or
he comes home at once when dropped on his nest, and forgives you a little for
putting him back (a small lift in the brain's cheer). A softer throw just slides
and stops, and he hops home undazed. Lying on the page (dazed, or asking about a
row), he holds on to the paper as it scrolls: a slow scroll carries him along with it,
and one faster than he can hold on flings him with the page, a throw like any other,
so a hard enough one smacks him into the edge of the screen. A throw that comes into the bottom-right corner
is caught by his nest, however hard: no daze and no ink, he hops in. The nest is
soft: it sags under him as he lands (deeper the harder he was thrown), rocks as he
leaves or lands to one side, and settles; with reduced motion it stays still. He leans into where he hops, and eases from
one pose into the next, never snapping. After a long stretch of writing his nib runs dry:
home again, he tips toward the nest's rim and taps it there a few times, the nest
bobbing under each tap, then rests. His ink dries and fades on its own; home
again, he listens again.
b. Flick the pen hard at a side of the screen, or into the nest's corner; drag it again to write, or hold
anywhere and drag; drop it on the nest, or leave it 3 seconds.
c. Code: `web/src/rambo.rs:Rambo`; `web/src/engine/world.rs:World, behaviour`;
`web/src/engine/look.rs:pose, blended, Look`; `web/src/mascot.rs:nib_dry`;
`web/src/engine/nest.rs:Nest::tap`;
`web/src/engine/fling.rs:step, splat_strokes`; `web/src/engine/world.rs:swept`; `web/src/engine/nest.rs:Nest`;
`web/src/mascot.rs:transition, listens_on_grab,
put_back, over_nest, DAZED_MS, hops_there, held_at, wants, drawn_over`; `web/src/hop.rs:Hop`;
`web/src/gesture.rs:Recognizer, release_velocity, flung`;
`web/src/inklayer.rs:InkLayer`; `web/src/ink.rs`;
`crates/rambl-brain/src/lib.rs:Event::PutBack`.

### 3. Writing a new row
a. When a task is added, Rambo flies to its empty line and writes the title under
his nib, letter by letter at his own pace, line after line when it wraps; only the
ink appears, the row is already there. Anything you do interrupts him (picking him
up, typing a new capture, turning the page or scrolling the row away): the ink
waits at the letter he reached, and once he is home and can see the row he flies
back and carries on. A row left waiting `WRITE_HOLD_MAX_MS` simply shows its title,
and a reload always shows titles whole.
The ink he has just written is wet for `INK_WET_MS` (4 s): a little darker, with a
sheen along the top of its strokes, drying to the page's ink. A tap on the row while
it is wet smudges it: the fingertip drags a small smear of the row's ink a little way
to the right of where it touched, and the row does not open. The smear is ink and
stays (until the row is redrawn or the page reloads); drying is the only change ink
makes on its own. Once dry, a tap does what it always did.
b. Type a long task and press Enter; pick him up halfway through, then let go. Add
another and tap it as soon as he flies off it.
c. Code: `web/src/tasks/write.rs:write_row_title`; `web/src/pen.rs:schedule,
glyphs_of, stroke_at, inked_count, hand, touch_down, flick, beat`; pace and hand in
`web/src/tuning.rs` (`WRITE_*`, `INK_FADE_MS`); wet ink in `web/src/smudge.rs:is_wet,
smear, Smudges` and `web/src/tasks/mod.rs:smudge_if_wet` (`INK_WET_MS`, `SMUDGE_*`),
the sheen in `web/theme.css` (`.ink-wet`, `ink-dry`).

### 4. Drop actions
a. Dropping Rambo on a task opens a small bubble with Edit, Move and Done, drawn as
paper scraps. Tapping anywhere off the bubble (him too) or pressing Esc is never mind. Edit opens the task dialog (`<dialog class="task-dialog">` in `web/src/tasks/mod.rs`).
b. Drag the pen onto a task row and choose an action.
c. Code: `web/src/rambo.rs:Rambo`; `web/src/mascot.rs:Choice, asking_dismissed_by`;
`web/src/api.rs:patch_task`. Move sets `move_request`; the planner places the task
(III.7).

### 4a. Sound presets
a. The gear menu's **Sound** picks one of four: **Off** (silence; Rambo does not speak
aloud), **A quiet desk** (only the noises paper and ink really make: the nib, the
scribble, the page turn, the crumple; no voice), **Every touch answers** (the default:
also a tick on every button and a swish when a ramble is sent) and **Everything + music**
(the same cues, plus calm piano under them: two public-domain recordings, looped softly,
started by the first press after the choice). It replaces the old Sounds on/off
flag: a stored "off" stays Off, a stored "on" becomes every touch. With reduced motion
on, the tick and the page sound are dropped.
b. Open the gear menu, pick each preset, then tap a button, turn the page and add a task.
c. Code: `web/src/sound.rs:Cue, plays`; `web/src/settings.rs:SoundPreset, with_migrated_sound`;
`web/src/voice.rs:audible`; `web/src/music.rs:sync`. Music files: `web/assets/music/ABOUT.md`. Sound files and how to swap one: `web/assets/sounds/ABOUT.md`.

### 5. Reduced motion
a. With reduced motion enabled, Rambo returns immediately instead of animating.
b. Enable reduced motion in the operating system and drag Rambo away from home.
c. Code: `web/src/mascot.rs:prefers_reduced_motion`; `web/src/rambo.rs:Rambo`.

### 6. Mascot brain
a. The mascot crate provides deterministic, seeded behaviour without web code; it is
wired into the web mascot by `web/src/brain.rs:Controller`, which turns a brain event
into a pose and a line of Rambo's own dialogue.
b. Not visible; covered by tests in `crates/rambl-brain/src/lib.rs`.
c. Code: `crates/rambl-brain/src/lib.rs:Brain, step, step_with`;
`crates/rambl-brain/src/rng.rs:Seeded`.

### 7. Rambo's voice
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
Relative times count from now, in the user's zone: "in 10 minutes", "in an hour", "in half an
hour", "in 90 minutes", "in 2 hours", "in 45min" make a task at that instant (rounded down to
the minute, so the seconds you spoke at are not kept), whatever day or time Jev also picked,
and Jev is not asked about them. Numbers may be spelled out up to sixty ("in twenty minutes",
"in forty-five minutes", "in forty five minutes"), also as a task's length ("a forty five
minutes call"). "In 2 days", "in a week" and "in 3 weeks" are dates (up to a year), kept on the
local calendar across daylight-saving changes; "in a month" and "in 2 months" (up to 12) are
the same day of the month, or the last day of a shorter month. "In a minute" means
soon, not sixty seconds, so it stays in the title. Zero, past a year and "in a
while" are left to the title.
b. Capture "wake up in 10 minutes": the task is "Wake up" at now plus 10 minutes. Covered by
tests in `crates/dates/src/lib.rs`, `crates/dates/src/phrases.rs` and
`crates/capture/src/pipeline.rs`.
c. Code: `crates/dates/src/lib.rs:candidates, resolve, after, Value::After`;
`crates/dates/src/phrases.rs:day, time, duration, minutes_from_now`.

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
