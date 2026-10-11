# Task list: frontend engineer

Read `docs/TEAM.md` first (API, status, visual direction). Work top to bottom; do not start a P2 item until your P1 works end to end.

Run the backend locally: `cargo run -p server` → `http://localhost:8080` (put `TYPESAFE_API_KEY=...` in `.env`).

**Goal:** a working paper-style TODO list first, then the AI flow on top.

## P0: plain TODO list (~1.5h)
- [ ] Page shell: parchment background, ruled lines, fonts (Caveat or Patrick Hand for task titles, clean sans for UI). *(30m)*
- [ ] Task list from `GET /api/tasks`: title, deadline ("Fri 5pm"), duration ("3h"), label tag. *(30m)*
- [ ] Checkbox → `PATCH /api/tasks/:id {"status":"done"}`, hand-drawn strike-through. *(15m)*
- [ ] Delete → `DELETE /api/tasks/:id`. *(10m)*
- [ ] Inline edit of title, deadline, duration → `PATCH`. *(20m)*
- [ ] "Add task" box → `POST /api/tasks {title}` (agents are finishing this endpoint; use `/api/capture` until it lands).

## P1: AI flow, the demo core (~2h)
- [ ] Always-visible text box → `POST /api/message {text}`, redraw the list from `tasks`. *(20m)*
- [ ] Push-to-talk: hold button or spacebar → `webkitSpeechRecognition` → send transcript to `/api/message` on release. *(40m)*
- [ ] Speak `reply` aloud with `speechSynthesis`. *(10m)*
- [ ] Week timeline from `GET /api/calendar` (or `placements` from `/api/message`): grey busy blocks + tasks coloured by label. A task can have several placements. Convert UTC to local time. *(60m)*
- [ ] "Schedule it" button → `POST /api/schedule` → redraw the timeline. *(10m)*

## P2: polish (~1.5h, only after P1 works)
- [ ] Pen mascot SVG with 3 states: idle, listening, writing. *(30m)*
- [ ] "Writing" animation when new tasks appear (`stroke-dashoffset`). *(30m)*
- [ ] Fields with `confidence < 0.6` look "pencilled in" (grey, lighter). *(10m)*
- [ ] Jump the timeline to `focus_date` when set. *(15m)*
- [ ] Loading state while Jev thinks; error toast. *(15m)*

## P3: only if time
- [ ] Weather background (Open-Meteo, no key, plus browser geolocation).

**Handoff for deploy:** a static build (`index.html` + assets) placed in `static/`; the server serves it. Use relative URLs (`/api/...`) so it works on burvy.dev unchanged.
