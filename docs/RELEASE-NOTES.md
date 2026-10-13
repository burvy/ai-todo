# Rambl release notes

Plain-language notes for the release built from `main` up to `#173`. Every item says
which pull request (PR) it came from, so it can be found later. How to deploy is in
[DEPLOY.md](../DEPLOY.md).

## What you will notice

**The page loads without crashing.** The page used to fail on load and on sign-in
because the browser build asked for the computer's clock, which a web page does not
have, and because the sign-in state belonged to one part of the page and was read by
another. Both are fixed; today's date now comes from the browser (#135).

**Tasks and the week have their own panes.** On a desktop they sit side by side with
the week wide. On a phone you swipe between them, or use the Tasks / Week switch (#135,
#140). A long title no longer wraps a task row onto three lines on a small screen, and
the page no longer scrolls sideways (#157).

**The planner shows the task, not a code.** A scheduled block used to show a raw id
like `t77d445f`. It shows the task's title and its start time, in the time zone the
server planned with — a 10:00 New York task reads 10:00, not 14:00 (#142, #158). A
block sits at its real time inside the day instead of pushing the hours down (#151).

**Deadlines remember the zone they were set in.** A deadline written as Friday 17:00 in
New York still reads Friday 17:00 New York after you travel or change your computer's
zone; the instant never moves (#163). Editing a task no longer quietly moves a deadline
into the zone you happen to be editing from (#171).

**New tasks land at the top.** The newest task is first in the list, in the response
that created it, after a calendar import and after a batch of captures (#155).

**Overdue tasks get placed first.** Overdue work now goes into the next free slot
before anything flexible, in one pass, and a task due at exactly this minute counts as
overdue (#154). Rambo also waits for the tick before teasing you about an overdue task,
instead of teasing on every change and missing tasks that had been sitting there (#156).

**Capture understands more.** `2h` is read as two hours, emoji survive the title
cleanup, and non-English clauses keep their words instead of being split or stripped
(#164, #166).

**Rambo writes with a pen.** While you type, his pen follows the caret and flies home
when you stop. Pick him up and speech recognition starts; release without dragging and
the transcript is captured. New tasks make him visit each row and write the title out.
He has nine poses, including a listening one (#152, #153), and the art ships inside the
built page instead of 404ing (#137). That change briefly stopped the capture box from
passing on what you typed (#153); the text is set again in #175, which merged after
these notes were written.

**The parchment look.** Colours, type, spacing and paper texture are tokens in
`theme.css` with a dark variant, and `docs/THEME.md` records the palette and the
mobile rules (#140).

## What changed for whoever runs it

**There is an admin page now.** `/admin` is served by the server, needs no build step,
and shows today's captures by user and by address plus the totals per plan. From the
same page you can block or unblock a user or an address and move a user between plans
— the same things the local console command line does, using the same code (#161).

Set `RAMBL_ADMIN_EMAILS` to a comma-separated list of the emails allowed to open it.
Unset or empty turns the whole page off and answers 404, to anybody (#161).

**Cloudflare's address is only believed from the tunnel.** The client address used for
per-address limits and blocks was taken from the `CF-Connecting-IP` header whenever the
header was there, so anyone who reached the server directly (the LAN, an open port)
could claim any address and step past those limits. The header is now read only when the
connection came from a trusted proxy: loopback always is, which covers the tunnel
running on the same machine, and `RAMBL_TRUSTED_PROXIES` adds a proxy anywhere else as
a comma-separated list of addresses or networks (#168). Leave it unset on the current
server.

**Anonymous accounts clean themselves up.** An anonymous account unused for 30 days is
deleted with its tasks. Signed-in accounts never are. The deletion only happens after
that day's backup has succeeded (#146).

**A local install no longer locks itself out.** In local mode (no Google sign-in
configured) the server used to issue an anonymous id per fresh browser profile and
count it against the per-address allowance, so a machine with a lot of browser profiles
was refused for a day. Local mode now issues none (#159).

**A refused edit says why.** When an edit is turned away because the task changed
elsewhere, the page says so and reloads, and the reload keeps the day count you chose
(#150). Edits themselves are last-write-wins: the server stamps every change, and an
edit against an older version is refused with `stale_update` and changes nothing (#146).

**The server listens on this machine only.** It used to listen on every network
interface, which put it on the LAN where anything that could reach the box could talk to
it, and on Windows every new build path popped a firewall prompt. It now listens on
`127.0.0.1`, which is all the tunnel on this machine needs. `RAMBL_BIND` brings the old
behaviour back on purpose (`0.0.0.0`, or one address), and a value that is not an IP
address stops the server with an error rather than falling back. Widening the bind does
not by itself let anyone name their own address — a peer's `CF-Connecting-IP` is still
only read from a trusted proxy — but a tunnel arriving from outside then has to be named
in `RAMBL_TRUSTED_PROXIES`. The window prints the address it actually bound (#185, merged
after these notes were written).

**Task ids cannot collide between accounts.** Two accounts cannot claim the same task
id (#148).

## Not in this release

**Recurring tasks are not built.** The design is written and reviewed (#170) and the
phrase scanner that reads `every Monday and Thursday`, `weekly on Tue and Fri` and
`until December` out of a ramble is built (#173), but nothing is stored, nothing
repeats, and the page has no control for it. Monthly and "every other" repeats are out
of scope of that design. Read it as groundwork, not as a feature you can use.

**One mascot consolidation is still open.** Unifying Rambo into a single mascot
element (#162) has not merged; the behaviour in #153 shipped against the current
structure.

## Testing added

Relative-date edge cases such as "tomorrow at 00:01" and the year and leap-year
roll-overs (#143); eleven capture tests that found three real bugs (#164, #166);
calendar-import tests (#165); and scheduler edge cases including the zero-duration
drop, since fixed (#167).

## Notes for a rollback

The database file only moves forward: opening an older file applies the migrations it
has not seen and marks the file as the current version (7). Since #146 the code applies
only what a file is missing, so an executable from this release or later opens a
migrated file, changes nothing, and does not read the columns it does not know about.
An executable older than that re-ran every migration on open and refuses such a file, so
copy `data\rambl.db` before upgrading if the rollback may go that far. The startup
backup in `data\backups` is written after the migration, so it is not that copy.