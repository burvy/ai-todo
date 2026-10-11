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

- [ ] **6. "What does tomorrow look like" skips the first task of the day**
      `server/src/main.rs` — `query_day` reply

      After the demo rambles + `week.ics`, Monday's first placement is
      *Clean up the chemistry lab data* at 9:00 (one chunk of a split task).

      ```
      POST /api/message {"text":"what does tomorrow look like"}
      -> "Tomorrow: 8 tasks, starting with Share a draft with the group at 11:15."
      POST /api/message {"text":"what should I do now"}
      -> "Next: Clean up the chemistry lab data at 9:00 (60 minutes)."
      ```

      The two replies contradict each other. *Impact:* medium, both are demo lines.

- [ ] **7. "What should I do now" does not say it is tomorrow**
      Asked Sunday 18:10 local: `"Next: Clean up the chemistry lab data at 9:00 (60 minutes)."`
      Reads as if it is 9:00 today. *Impact:* low-medium.

- [ ] **8. Past days are answered as if they were today**
      ```
      "what does yesterday look like" -> "Today is free."  (focus_date = today)
      ```
      *Impact:* low.

- [ ] **9. Reply to a ramble hides tasks that could not be placed**
      After ramble 4 (`Today I need to submit the financial aid form`) the task
      gets deadline today 17:00 (already past at 18:00 local) and is returned in
      `unscheduled` with `deadline_passed`, but the reply is only
      `"Got it, 6 new tasks."`. Same for a task that did not fit
      (`no_available_time`, seen for *Clean up the chemistry lab data* before a
      cancellation). *Impact:* medium: the spoken reply claims success.

- [ ] **10. Finishing an already-finished task gives a misleading reply**
      ```
      "I finished the history paper"   -> "Done: Finish the history response paper."
      "I finished the history paper"   -> "I couldn't tell which task you finished."
      ```
      Should say it is already done. *Impact:* low.

- [ ] **11. Same text submitted twice creates duplicate tasks**
      Sending ramble 1 then 2, 3, 4 is fine, but re-sending the same text (or a
      sentence repeated 400 times: `"I need to call mom. " x400`) creates 400
      identical tasks and 400 placements (4.2 s, server stays up). No dedupe.
      *Impact:* medium: a double tap of the mic button on stage doubles the list.

- [ ] **12. Garbled and raw titles**
      ```
      ramble 4 -> task "Study for make flashcards before the exam on"
      ramble 4 -> task "I also promised to call my mom tonight"   (no clean-up, no deadline,
                  placed Tue 17:30; "tonight" is Sunday)
      ```
      *Impact:* medium: titles are visible in the demo (`judge` / `clean_title`).

- [ ] **13. Valid-looking inputs silently dropped**
      ```
      "I need to do a 20 hour project by tomorrow" -> intent other, "I can add tasks, cancel..."
      "my bookstore shift got cancelled" (a Thursday shift exists) -> intent other, same reply
      ```
      Honest, but unhelpful. *Impact:* low.

- [ ] **14. `PATCH /api/tasks/:id` accepts any status** (Jim's area, not edited)
      ```
      PATCH {"status":"bogus"} -> 200, status stored as "bogus"
      ```
      *Impact:* low. Should be 400.

- [ ] **15. "Cancelled ... at 2:00" has no am/pm** in the cancel reply.
      `"my 2pm on Monday got cancelled"` -> `"Cancelled Campus bookstore shift at 2:00; your plan is updated."`
      *Impact:* low.

Checked clean in this pass (demo script by API, `week.ics` import 409 then `confirm=true`,
all four rambles, `/api/schedule`, `/api/calendar.ics`, empty/garbage/emoji input, invalid
JSON -> 400, non-ics upload -> 400): no placement overlapped a busy block or another
placement, none was in the past or outside 9:00-18:00 local (UTC-4), chunks always summed
to the task duration, no request took over 5 s.

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
