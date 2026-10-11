# ai-todo: submission

> **Every scheduling tool asks you to do data entry. This one just listens.**

**Live:** https://ai-todo-api.burvy.dev · **Code:** https://github.com/burvy/ai-todo

## The problem
Planners like Motion, Reclaim and Google Tasks schedule well, but only after you type every task, duration and deadline into a form. Students and busy people don't plan badly; they skip the data entry. The list never gets written, so nothing gets scheduled.

## What it does
You talk the way you'd talk to a friend: *"Okay, I need to finish the history paper and upload it by Friday, email Professor Chen about the missing reading, oh, and buy printer ink. The library closes early tonight, I think."*

ai-todo:
1. **Writes the list.** Three tasks appear, written by a pen mascot: *Finish the history response paper* (due Friday 5pm, School), *Email Professor Chen about the missing reading* (School), *Buy printer ink* (Errands). The chit-chat about the library is ignored.
2. **Plans the week.** It places every task into the free time around your real classes and shifts, deadline first, in chunks of at most two hours.
3. **Keeps listening.** *"My 3pm got cancelled"* frees the slot and re-plans. *"I finished my slides"* ticks the task off. *"What does tomorrow look like?"* summarises the day aloud. *"What should I do now?"* names the next task.
4. **Rings when it's time.** *Add to my calendar* exports the plan with 10-minute alarms to Google Calendar, Outlook or an iPhone, and the open page says "Up next: …" five minutes before each task.

It also imports an existing calendar from an Outlook or Google `.ics` export (cancelled meetings and "show as free" events are skipped), and works on a phone.

## How we use Jev
Jev (TypeSafe's System One model) answers small, typed questions; our code owns the workflow. Jev never writes free text in our app: it **selects among candidates our code built**, and our code acts on its answer.

| Step | Jev question | Answer type |
|---|---|---|
| Route a message | What does this message intend: add tasks, complete a task, cancel an event, ask about a day, other? | Choice |
| Filter a ramble | Does this clause contain a real obligation or deadline (not chit-chat or self-talk)? | Noul (yes probability) |
| Read the deadline | Our code parses every date phrase ("Friday", "by the weekend", "next week"); which candidate is the deadline? | Choice |
| Prioritise | How urgent is this task, low to critical? | Score |
| Estimate | How long will it take? | Score |
| Match *(in progress)* | Which open task did the user just finish / which event was cancelled / which label fits? | Choice |

The probabilities drive the UI: fields Jev is unsure about are drawn **"pencilled in"** (grey, lighter) rather than in ink, so the user sees the AI's confidence without any extra UI.

Everything a rule can do stays in code: clause splitting, date parsing, the scheduler, the calendar import and export. That keeps it fast, testable and cheap: one Jev request per ramble, with all questions asked in parallel.

## How it's built
- **Rust** workspace: `axum` server, `crates/judge` (Jev client, clause splitting, date candidates, title cleanup), `crates/scheduler` (deterministic slot placement), `crates/capture` (email and PDF text), SQLite storage.
- **Web app:** vanilla HTML/CSS/JS on parchment paper with a pen mascot; push-to-talk and spoken replies use the browser's Web Speech API. One page for PC and phone.
- **Deployed** on a home Windows server behind Cloudflare.
- CI runs `fmt`, `clippy -D warnings` and about 85 tests on every pull request; the demo rambles are covered by tests with the exact transcript text.

## Challenges
- **Real speech is messy.** "…finish the paper, it is maybe two pages, and upload it by Friday" has to become one task with a Friday deadline, not three. We test against recorded-style rambles, not clean sentences.
- **Dates are relative to when you said them**, and an evening demo crosses midnight in UTC. All date logic runs in the user's time zone.
- **Real calendar exports aren't like the examples:** CRLF line endings, time-zone blocks, folded lines, cancelled meetings. A teammate wrote tests against real export shapes and found five bugs before the judges could.

## What's next
- Drop an email or PDF (a syllabus, a professor's email) and get its tasks, with deadlines read relative to when it was sent. The parsing is built; the upload is in progress.
- Drag a task in the planner to pin it; the rest re-flows around it.
- Recurring events, sign-in and per-user lists.

## Team
- **burvy:** product, deployment, demo
- **Frontend:** parchment UI, pen mascot, phone layout
- **Rayansh (backend):** storage, calendar import, email/PDF parsing, real-export bug hunt
- **New teammate:** email/PDF upload
- AI coding agents (orchestrated): capture pipeline, scheduler, conversation replies, alarms, reviews and live testing

---

## 90-second demo video script

| Time | Screen | Say |
|---|---|---|
| 0:00 | Empty parchment page | "Every scheduling tool makes you type the task, the duration, the deadline. Ours just listens." |
| 0:08 | Click **Import calendar** (once the button lands), pick `demo/outlook/week.ics` | "Here's my real week: classes and a campus job." |
| 0:15 | Hold the pen, read ramble 1 from `demo/rambles.md` | (the ramble, naturally, about 20 seconds) |
| 0:38 | The pen writes 3 tasks | "Three tasks. It skipped the bit about the library. Friday's deadline went to the paper, not the ink." |
| 0:48 | Click **Schedule it** | "Every task goes into free time around my classes, deadline first." |
| 0:55 | Hold the pen: "my 3pm got cancelled" | "Plans change, so I just say so. It frees the slot and re-plans." |
| 1:05 | Hold the pen: "I finished the history paper" | "Done. Struck through, schedule updated." |
| 1:12 | Hold the pen: "what should I do now?" | (it answers aloud) |
| 1:18 | Click **Add to my calendar**, show the phone calendar | "And it goes onto my phone with alarms." |
| 1:25 | Pen idle | "No forms. No fields. Jev turns a ramble into a plan." |

**Before recording:** clear the list (`POST /api/reset` once it lands, or delete the server's `data\` folder and restart), use Chrome or Edge, allow the microphone, and turn the Voice toggle on.
