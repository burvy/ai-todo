# Bugs found

Written 2026-10-11 by backend 2 (Rayansh / `secre`), during extra testing of the
calendar-import and email/PDF work.

**How these were found:** the committed tests all pass, but they run against
`demo/outlook/week.ics`, which is a sanitised fixture — LF line endings, and no
`VTIMEZONE`, folded lines, `RRULE` or `STATUS`. Real Outlook and Gmail exports
contain every one of those. Writing tests against the shape real exports
actually arrive in surfaced five failures.

**The tests are on this branch** (`server/src/ics.rs`, `crates/capture`), so
anyone can reproduce them:

```sh
cargo test -p capture -p server
```

They assert the *correct* behaviour, so **this branch is red on purpose and must
not be merged until the five items below are fixed.** Fixing a bug turns its
test green; that is the signal it is done.

---

## Open

- [x] **14. PATCH accepts unknown task statuses** — `PATCH /api/tasks/:id`

      Task status is restricted to `todo` and `done`; other values now return
      HTTP 400 with an error object.

- [x] **1. Cancelled events still block time**
      `server/src/ics.rs` — `parse_ics`

      Outlook keeps cancelled meetings in the export with `STATUS:CANCELLED`.
      We import them like any other event, so the planner books time over a
      meeting that no longer exists — the entire point of cancelling it was to
      free that slot.

      ```
      a cancelled event must not block time, got ["Advising appointment"]
      ```

      *Impact:* medium-high. A student's real export is full of these, and they
      accumulate. Demo-relevant: it quietly makes the schedule worse the more
      real the calendar is.

      *Fix:* skip events whose `STATUS` is `CANCELLED` in the `filter_map`.

- [x] **2. `TRANSP:TRANSPARENT` events still block time**
      `server/src/ics.rs` — `parse_ics`

      `TRANSP:TRANSPARENT` is the iCalendar way of saying "display this, but do
      not treat it as busy". All-day birthdays, holidays and reminders use it.
      We treat them as busy, and an all-day one blocks from midnight to
      midnight.

      ```
      a TRANSPARENT event should not block the day, got ["Someone's birthday"]
      ```

      *Impact:* medium. One all-day transparent event can make a whole day look
      full, and the scheduler will then report `no_available_time`.

      *Fix:* skip events whose `TRANSP` is `TRANSPARENT`. Same place as (1).

- [x] **3. Gmail / Apple Mail quoted replies are not cut**
      `crates/capture` — `parse_email`

      We cut at Outlook's `-----Original Message-----`, the underscore rule,
      `>` quotes and duplicate `From:`/`Sent:` blocks. Gmail and Apple Mail use
      neither: they write an *attribution line*, and everything under it is the
      previous message.

      ```
      left:  "Please finish the lab report by Friday.\n\nOn Mon, Oct 12, 2026
              at 9:15 AM Jane Doe <jane@example.edu> wrote:"
      right: "Please finish the lab report by Friday."
      ```

      *Impact:* **high, and it threatens the demo flow.** The whole pitch is
      "drop an email, get tasks". Handing Jev an entire reply chain produces
      phantom tasks from the *sender's* words, not the user's.

      *Fix:* cut at a line matching `On ... wrote:` (and the `wrote :` French
      variant), which is what both clients emit. Also worth handling
      `----- Message snippe` and the `__________` variant Gmail uses.

- [x] **4. Folded headers truncate the subject (and can break the date anchor)**
      `crates/capture` — `parse_email` / `split_headers`

      RFC 5322 folds any header longer than 78 octets onto a continuation line
      beginning with a single space. We treat that continuation line as the
      start of the body, so the header is cut short.

      ```
      left:  "A very long subject line about the chemistry lab that"
      right: "A very long subject line about the chemistry lab that would not
              fit on one line"
      ```

      *Impact:* medium. The subject often names the task, so a long one loses
      its meaning. Worse, a folded `Date:` or `Sent:` would silently drop the
      date and fall back to anchoring deadlines on *now*, which is exactly the
      bug P1 was meant to fix.

      *Fix:* in `split_headers`, a line starting with a space or tab continues
      the previous header rather than starting the body.

- [x] **5. A reply that adds no words of its own yields phantom text**
      `crates/capture` — `parse_email`

      Direct consequence of (3). A reply containing only a quoted message
      should produce an empty body, so the caller can say so. Instead the
      attribution line survives and becomes the "message".

      ```
      expected an empty body, got "On Mon, Oct 12, 2026 at 9:15 AM
      Jane <jane@example.edu> wrote:"
      ```

      *Impact:* low-medium on its own, but it is the same root cause as (3) and
      should fall out of that fix.

      *Fix:* none needed separately once (3) is fixed — this test then passes.

---

## Checked and clean

Worth recording, because these were the plausible-looking risks and they are
**not** bugs. No action needed.

- [x] **`pdf_to_text` does not panic on malformed PDFs.** `docs/TASKS-pdf.md`
      asks the new teammate to guard against this, and it is worth knowing the
      guard already holds at our level: seven hostile inputs (header-only,
      empty xref table, dangling object reference, binary junk, truncated
      mid-object, negative object count) all return `Err` cleanly. Panics only
      need catching at the route if something more exotic turns up.
- [x] **CRLF line endings parse.** Every real `.ics` is CRLF; the fixture is LF,
      so this was untested until now.
- [x] **`VTIMEZONE` blocks are not mistaken for events**, despite containing
      their own `DTSTART` and `RRULE` lines.
- [x] **Folded `.ics` lines are unfolded** back into one event.
- [x] **RFC 5545 escapes are unescaped** — `\,` `\;` in titles render as `,` `;`.
- [x] **Non-ASCII titles survive** (accents, em dashes).
- [x] **`RRULE` does not break parsing.** Recurring events are not expanded, so a
      weekly lecture appears once. Deliberate, and the first occurrence is the
      correct one — but noted as a known limitation, not a bug.
- [x] **One malformed event does not lose the other eight.**
- [x] **Events spanning midnight keep their length.**
- [x] **An empty calendar is valid** and yields no blocks.
- [x] **`Date:` (RFC 5322) is read as the send time** when `Sent:` is absent —
      Gmail exports need this.

---

## Not bugs, but worth knowing

- **Recurrences are not expanded.** A `FREQ=WEEKLY` lecture blocks one slot, not
  every week of the term. Fine for a one-week demo; wrong for a real product.
- **All-day `DTEND` is exclusive** per RFC 5545, which happens to work out: a
  `VALUE=DATE` range of Oct 13–14 correctly covers Oct 13 only.
- **`demo/outlook/week.ics` is not representative.** It should probably be
  regenerated from a real Outlook export so the committed tests cover CRLF,
  `VTIMEZONE` and folding by default rather than by accident.
