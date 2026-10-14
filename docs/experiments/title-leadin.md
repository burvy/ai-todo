# Title lead-ins: can Jev pick the word where the real title starts?

**Question.** "Add a task to buy milk" should become the task "Buy milk". Which is better at
finding where the title starts: a plain code rule, or a Jev `Choice` over start words the
code proposes?

**Answer.** Jev choosing among code-made candidates, with a sharper question. It got 67 of
67 on the set the rule was built against and 44 of 46 on a set written afterwards, against
33 of 46 for the rule. It is also what Jev already does for titles, so it costs no extra
call. The rule stays, as the candidate Jev sees first and the answer the offline judge gives.

## Method

- **Eval sets**: `crates/capture/tests/fixtures/leadins.txt` (67 clauses, written while
  building the rule) and `leadins.holdout.txt` (46, written after the rule was fixed, in
  phrasings it was not built for). Each line is `clause => expected title`. Each set mixes
  clauses with a lead-in, clauses without one, and tricky ones that only look like a lead-in
  ("Add milk to the list", "Schedule a meeting with Sam", "Write a note to the landlord").
  Expected titles were written by hand.
- **Input to the step**: the clause with its date, time and duration phrases and its filler
  already removed (`pipeline::base_title`), so dates are not part of the test.
- **Today**: that title as it comes out now (`clean_title` only).
- **Code rule** (`leadin::strip`): a strict grammar. Optional opener ("please", "can you",
  "I want you to", "let's", ...), a request verb (add, create, make, schedule, set up, put, log,
  jot down, write down), optional article and "new", a task noun (task, todo, to-do, reminder,
  note, item), optional "on my list", then a link word (to, that, called, named, titled, about,
  for, saying); or a bare label ("New task:", "To do:"); or "remind me to". It cuts only when
  all of that matches and something is left. "to" followed by "the/a/my/me/..." is not a link
  ("a note to the landlord").
- **Jev**: the code lists candidate titles (the whole clause, one for each word after a link
  word or colon among the first eight, when a request cue came before it, and the rule's own
  answer). Jev answers the existing `title` Choice (`judge::jev`) over them, in the same
  request as the other questions. Real calls to `jev-latest`; answers recorded in
  `leadins.jev.json`.

## Results

| | design set (67) | holdout (46) |
|---|---|---|
| Today (`clean_title` only) | 32 (48%) | 17 (37%) |
| Code rule | 67 (100%) | 33 (72%) |
| Jev, original question ("best names the task"), link-word candidates | 62 (93%) | 41 (89%) |
| Jev, sharper question, link-word candidates | 64 (96%) | 41 (89%) |
| Jev, sharper question, **every word** as a candidate | 61 (91%) | 39 (85%) |
| **Jev, sharper question, link-word candidates + the rule's answer (shipped)** | **67 (100%)** | **44 (96%)** |

Notes on reading this:

- The rule's 100% is on the set it was built against. The holdout number (72%) is the honest
  one. After the first holdout run the rule got two small fixes (a "to my list" form and
  "that says"), which moved it from 27 to 31 and then 33; so the holdout is no longer fully
  untouched for the rule. Jev's holdout figures changed only through candidate changes, not
  through any tuning to individual clauses.
- Every Jev miss with the final setup is a clause where the right title was **not on offer**
  (44 of 46 were reachable): "Add the task of mowing the lawn" (the code does not offer a
  start after "of") and a clause where Jev took the nearer link word ("... for me to renew").
  When the right title is on offer Jev picked it every time, including the tricky ones.
- Offering **every word** as a start made Jev over-cut ("Add milk to the list" became "Milk to
  the list"). Starts after link words only is better.
- Sharpening the question text ("words that only ask the app to create the task are not part
  of the title") fixed Jev's habit of keeping "Add a reminder to ..." whole.
- The rule on its own cuts wrongly in a few holdout cases before the "to + the/me" guard
  ("Write a note to the landlord" became "The landlord"). That is why the rule is not used alone.
- Today's pipeline already handles "remind me to" and "I need to". The remaining 52% / 63% of
  failures are the "add/create/make/schedule a task to" family and openers.

## What shipped

- `crates/capture/src/leadin.rs`: `strip` (the rule) and `options` (the candidates, the
  rule's answer first).
- `pipeline::title_spans` now offers `leadin::options`, so Jev's `title` answer picks among
  them. `FakeJudge` and any judge with no opinion answer `title: 0`, which is the rule's cut.
- `judge::jev`: the `title` question reads "Which span is the task itself, as a short title?
  Words that only ask the app to create the task ... are not part of the title."
- Tests: `tests/leadins.rs` replays the recorded Jev answers offline and checks them on both
  sets; unit tests for the rule and the candidates; the golden file moved by one line ("Please
  make a task to review chapters six and seven" is now "Review chapters six and seven").
- To re-run with real Jev: `TYPESAFE_API_KEY=... cargo run -p capture --example leadin_eval`
  (add `-- --record` to rewrite the recorded answers; re-record whenever the candidates or the
  question change, because the recorded numbers are indexes into the candidate list).

## Not done / open

- A start after "of" ("the task of mowing the lawn") and fully free-form requests ("I was
  thinking I should maybe write down that I have to ...") are not offered as candidates.
- The eval sets are small and written by one person. Real captures should be added to them.
- Cost: no extra Jev call. The only change to the request is more title candidates in one
  `Choice`, and a longer instruction string.
