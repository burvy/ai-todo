# Design direction

The one design document for the front end: what Rambl looks like, how it moves, and the settled behaviour. It merges the owner's redesign brief (2026-10-13), the owner's design decisions (2026-10-13) and Fable's direction (2026-10-14). Where parts differ, later wins; on look and motion sections 0 to 6 win, on settled behaviour (gestures, tack board, order of work) sections 7 and 8 win. Section 9, the owner's answers of 2026-10-14, wins over everything before it. Token names here replace the ones in `docs/THEME.md` where they differ.

## 0. The principle: a near-physical list you can handle

Rambl must not look like a generic to-do list. It is **a real sheet of paper on a desk, with a real pen living on it**, and everything on the page must pass one test: *could I touch this, and would it respond the way the object would?* A row is a line of handwriting you can strike out or tear off. A slip on the planner is a piece of paper you slide. The nest is a dish that sits there. Rambo is the one thing on the desk that moves on its own. Nothing is a widget: no card stacks, pills, native pickers or hover-glow buttons. Every item below answers to this test, and so does any future feature. If a builder cannot name the physical object a new element stands for, it is not ready to build.

Two consequences run through the whole file. **Paper is warm and ink is coffee; the only cool colour on the desk is Rambo**, so the eye finds him first. **Motion is what the object would do**: paper slides and settles, ink appears under a nib, a pen flies and lands; nothing fades in from nowhere or scales up from a point.

## 1. Rambo's colour: petrol ink, brass cap

Today he is coffee brown (`#3e2f1f`, OKLCH hue 68°, chroma 0.03) on an ecru disc: the same hue as the paper, the ink, the rules and the accent, with less chroma than the accent. He vanishes by construction, and a pen you cannot see is not a pen you reach for.

Every paper and ink colour sits between OKLCH hue 64° and 89° (warm yellow-brown). Rambo gets **hue 215°, a petrol blue-green**: 150° away, so it reads as a different family rather than vibrating like a dead complement would, and it is the colour of fountain-pen ink, so it still belongs on a desk. Chroma 0.075 matches the accent's 0.10 in intensity, so he looks painted on the paper, not stuck on. The cap and clip stay **brass** (the existing `--accent`): petrol body plus brass trim is the classic pen, and it ties him to the only other emphasis colour on the page.

| Token | Light paper | Dark paper | Where on Rambo |
|---|---|---|---|
| `--rambo` | `#146272` | `#64bbc4` | body |
| `--rambo-deep` | `#004552` | `#358e9b` | outline, the shaded side, pupils |
| `--rambo-tint` | `#4a939f` | `#a3dde0` | the highlight stripe, the ink tail of a stroke, his bubble's rim |
| `--rambo-brass` | `--accent` `#9f682e` | `--accent` `#d9a45c` | cap, clip, nib |
| `--rambo-eye` | `#fcfaf4` | `#fcfaf4` | eye whites |

Contrast of the body as a shape: 5.7:1 on parchment and 4.1:1 on the ecru dish in light; 7.6:1 on dark paper. All above the 3:1 non-text floor, and the deep outline keeps his silhouette at 40px. At nest size: outline at least 1.5px in `--rambo-deep`, eyes at least 8px across, cap at least a quarter of his height. All five tools share these tokens; the tool changes the shape (pencil has a wood cone and a pink eraser, marker a flat chisel, quill a feather in `--rambo-tint`), never the family colour, so he stays one character.

## 2. The nest is furniture, Rambo is a layer

The nest is a fixed object on the page: a shallow paper dish (`nest.svg` as drawn, in `--ecru` with a `--chamoisee` rim), not a button, with no hover state. Phone: the right end of the capture bar at the bottom, 56px, its rim 6px above the bar's rule. Desktop: the right end of the capture line in the masthead, 64px. Tapping the dish submits the text, as today; the dish dips 1px under the finger and comes back, like pressing a saucer.

Rambo is a separate fixed layer above the page, never a child of the nest or of any row. At rest he overlaps the dish, nib down, body leaning 12° to the right, his lower third hidden behind the rim. He is 1.4x today's size (phone 64px tall, desktop 80px). Picking him up lifts him 4px with a soft shadow under him; dragging moves only him, exactly under the finger; the dish never moves. His idle is a slow 4s breathe (1px rise, 1° tilt), so he is alive before you touch him.

## 3. Rambo speaks

He has a voice (text to speech) and a bubble, and they are one thing. A line he says appears in **one paper speech bubble rising from the dish** (wheat paper, a `--rambo-tint` rim, a torn tail pointing at him), written in the neat hand, one line, at the moment the voice starts, and it stays until the voice ends plus 800ms. Muted, the bubble alone shows for 1.5s. He never says anything that is printed elsewhere on the page, and his lines are never loose text in a corner. While speaking he tilts 3° toward the capture line and his eyes narrow. Listening (held): see section 9.

## 4. The writing animation

Verdict on #213: the model is right, the choreography around it is not. "Lay the whole title out at once and clip only the ink, with the nib at the ink edge, each glyph paid for by its width" is exactly letters appearing under a pen, and it ends the row reflow that today stacks "— 30 min" vertically. Keep `pen::schedule`, `inked`, `PER_GLYPH_MS = 90`, the 500–2400ms clamp, the word-gap pause and the scratch sound tied to nib movement. Change the rest:

- **Beat 0, submit (0ms).** The typed text leaves the capture line in 150ms. The new row appears at the top of the list at full height with an empty ruled line: nothing below it moves again.
- **Beat 1, lift-off (0–450ms).** Rambo hops out of the dish (80ms squash) and flies to the start of the empty line in 450ms along a slight upward arc, ease-in-out, rotating from 12° to his writing angle of 35°. He lands nib-first with a 60ms dip. No 2000ms linger at the caret: he goes when you submit.
- **Beat 2, writing.** Nib on the baseline, body up and to the right, so the pen never covers the words. He is not shrunk (drop `WRITE_SCALE`); he is offset. The ink edge is soft, 2px, not a hard clip. A word gap is a 2px lift and drop of the pen. A title that wraps is traced on its first line, then the pen hops to the second line's start and continues; the wipe-only fallback is not acceptable.
- **Beat 3, finish (120ms + 250ms).** A short flick as the last letter closes, the scratch stops, a 250ms hold.
- **Beat 4, home (450ms).** Fly back to the dish, ease-out, one small overshoot, settle in 150ms. Then he speaks ("Okay, that one's in.") per section 3.
- **Interruptions.** Replaced by section 9: anything the user does interrupts him, writing pauses and resumes when he is let go, and the stored title pops to full on a reload.
- **Reduced motion.** The title appears at once in ink; Rambo stays in the dish; the bubble still appears.

For 20 glyphs the whole thing is about 3.2 seconds, and the row is readable from beat 2. Section 9 asks for slower, more considered writing in v1's feel; the times above are a starting point, not a limit.

## 5. Five things that make the app look dysfunctional, and the target look

1. **Rambo's speech is loose italic text in the page corner** ("Okay, that one's in.", "Adding it now.", printed over the last row on the phone). Target: section 3. Physically: a voice comes from a mouth, so the words come from him.
2. **The planner is a 24-hour ladder with no day names** (columns start at 00:00, the names scroll out of view, tasks pile up at 9 pm in 16px slips cut with "…"). Target: each column headed in the neat hand, "Tue 14", today underlined, pinned while the hours scroll; hours written as "8 am", "12 pm", "5 pm" (12-hour clock is the default, 24-hour is a gear-menu choice); the working day 8 am to 8 pm unless the gear menu widens it; a slip is a strip of paper never shorter than two lines of its title, with a 1px torn edge and the sheet shadow, and it lifts 2px while you slide it. Nothing on it is cut.
3. **Rows break their meta into fragments** ("— 30 min · Tue" on one line, "21:00" alone on the next; a ticked task looks the same as an open one; nothing appears on hover). Target: the title on its own line in the expressive hand; under it one small neat-hand line that never splits: "30 min, Tue 9 pm"; done is a hand-drawn strike and the ink fading to `--ink-faint`; on desktop a small drawn x appears at the row's right edge on hover. **Tags** are tiny and symbolic (an emoji or a letter, section 9); the stamp below is the drawing they sit in. They go on the same meta line as **ink stamps**: the tag word in the neat hand, lower case, in `--accent`, inside a hand-drawn oval outline 1px thick that does not quite close, rotated between -3° and 3° by a hash of the tag name, with no fill. Several tags sit in a row with a 0.6em gap. They never become pills, never get a solid background, and tapping one filters the list with the stamp pressing 1px into the paper.
4. **Rambo reads as an icon in a button** (a dark pen in a brown disc). Target: sections 1 and 2. He is big, petrol, overlapping a dish that is plainly furniture.
5. **The edit dialog and the gear menu are boxed forms** (native mm/dd/yyyy and --:-- pickers, radio buttons, a stray "15" input, a card with a drop shadow). Target: both are a sheet of wheat laid on the page at a 0.5° tilt with the soft sheet shadow only, and they slide in from the page edge as a sheet would, 250ms; labels in the neat hand on the left; choices are words you tap (Today, Tomorrow, Mon…; 15, 30, 60, 120); the date is a small drawn month grid; the time is a written "5:00 pm" on a rule. No native control is ever visible.

## 6. Palette tokens

Light stays the owner's six. Dark is the same desk at night. New: the five Rambo tokens, and the two per-tool ink tokens kept from #213.

| Token | Light | Dark | Role |
|---|---|---|---|
| `--parchment` | `#f2e8cf` | `#241c11` | the page |
| `--wheat` | `#ecd7af` | `#2d2315` | a sheet laid on the page (dialog, gear menu, capture bar on phone, the bubble) |
| `--ecru` | `#e1c38c` | `#1a140b` | pressed-in paper (the dish) |
| `--chamoisee` | `#a7865d` | `#a7865d` at 46% | rules only, never text |
| `--ink` | `#5d4732` | `#efe3c8` | everything written |
| `--ink-soft` | `#75604a` | `#c9b693` | meta lines, day names, hour labels |
| `--ink-faint` | `#6e5a43` | `#b3a17e` | done tasks, version line |
| `--accent` | `#9f682e` | `#d9a45c` | the one emphasis: active tab, today's underline, tag stamps, Rambo's brass |
| `--danger` | `#a5342a` | `#e0705f` | overdue, delete |
| `--now` | `#b03a2a` | `#d96a4f` | the now line |
| `--rambo` | `#146272` | `#64bbc4` | Rambo only |
| `--rambo-deep` | `#004552` | `#358e9b` | Rambo only |
| `--rambo-tint` | `#4a939f` | `#a3dde0` | Rambo and his bubble's rim only |
| `--ink-ballpoint` | `#2f4f7a` | `#9fc0e8` | a row written with the ballpoint |
| `--ink-quill` | `#6b4a2a` | `#d9a45c` | a row written with the quill |

The fountain pen writes in `--ink`, the pencil in `--ink` at 70% opacity with the grain texture, the marker in `--ink` bold. The three `--rambo*` tokens appear nowhere but on Rambo and his bubble; that exclusivity is what makes him pop.

## 7. Owner brief (2026-10-13), the original redesign

The current web app looks amateur: "just buttons and sections". It must look
**integrated, authentic and unique**, like writing on paper, not a stack of panels.
The v1 prototype was decent; read it for feel and behaviour (not code to copy):
`git show 679f669^:static/index.html`, `679f669^:static/app.js`, `679f669^:static/styles.css`.

### 1. Theme
- **Light is the default and is parchment.** Dark mode exists too (v1 had both), toggled in the gear menu. It never follows the OS — dark only when the user picks it (owner QA, 2026-10-13).
- Light palette (use as `:root` tokens in `web/theme.css`):
  - parchment `#F2E8CF` (page) · wheat `#ECD7AF` · ecru `#E1C38C`
  - chamoisee `#A7865D` (lines, muted text) · coffee `#5D4732` (ink) · golden brown `#9F682E` (accent)
- No hard-edged boxes, borders and card stacks. Use space, rules like a notebook, ink colour and paper texture. Few or no visible "buttons".
- The torn-edge strip and the cup stain currently read as a glitch and dirt: make them subtle or drop them.

### 2. Layout
- **Settings = a small gear, top right.** Everything currently in "View settings" goes in its menu, **including Import .ics** (a menu item that opens the file picker; never a raw "Choose File" input on the page).
- PC: task list left, planner right, both visible. Phone: list first, swipe sideways to the planner.
- Planner opens at working hours (about 08:00, or scrolled to now), not 02:00, and shows **day names** on each column.

### 3. Rambo (the pen mascot)
- **Rambo replaces the Add button.** He rests in a small "nest" (bubble) at the end of the capture line, bottom right. **Tapping/clicking the nest submits** the text.
- Rambo is **separate from the nest**: the user can **drag him out and toy with him**; he follows the finger/mouse **exactly**; on release he recovers and skitters back to the nest.
  - BUG today: dragging sends him off-screen to the left. The drag maths is wrong (pointer vs element offset / transform origin). Fix it and test with mouse and touch.
- **Picking Rambo up makes him listen** (speech capture). There is no mic button.
  - BUG today: "Could not start speech recognition". Find out why (permissions, secure context, starting twice, `continuous`/`interimResults`, Edge vs Chrome) and fix it; show a calm message only when the browser truly lacks it.
- A bit **bigger** than now. Recolour him to fit the paper palette (his blue clashes); he should look drawn on paper.
- **Writing animation** (today: a scribble, then the words appear at once; or he just slides to the row and back): the task title must **appear gradually, letter by letter, under the nib** as he moves along the line, then he returns to the nest.
- More life generally: small idle movements, the speech bubble for his lines, gentle transitions. Keep `prefers-reduced-motion` respected.

### 4. Task rows
- Desktop: a delete control **appears on the side on hover** (as in v1).
- Phone: a barely visible `x`, and **swipe right on a task to delete** it.

### Definition of done (every PR)
Full gate green (incl. `cargo check -p rambl-web --target wasm32-unknown-unknown` and `trunk build --release`), works at 1366x800 and 390x844, light and dark, and the PR body says what to click to see it.

### Owner decisions, round 2 (2026-10-13)

1. **Phone gestures, settled:** swipe **right** on a task = delete (Rambo scribbles it out). Swipe **left** = turn the page (list → planner, with a page-flip); on the planner page, swipe **right** turns back. The two never conflict.
2. **Task reordering on the planner must work:** drag a task to another time or day (keep the existing list drag-reorder too). Make it feel like moving a paper slip.
3. **All suggestions approved, build them:**
   - Rambo **dozes at night** and wakes when you type.
   - Rambo **peeks at the text box** while you type.
   - A **small celebration + hand-drawn strike-through** when a task is ticked done.
   - A **sulky look** at overdue tasks.
   - **Ink splat / wobble** when he is dropped hard.
   - A **faint previous-page shadow** on the planner.
   - **Pen-scratch sound** synced to the writing animation.

## 8. Owner decisions (2026-10-13), settled

These are settled.

### Handwriting everywhere
- Every word is handwritten. Self-host about 3 OFL handwriting fonts in `web/assets/fonts/` (no CDN).
- Two tiers: a **neat, very legible hand** for small UI text (times, dates, settings, day names) and an **expressive hand** for task titles. Check legibility at 390px.

### Rambo styles (writing tools)
- Gear menu > "Rambo": fountain pen (default), pencil, ballpoint, quill, marker. Remembered per device.
- Each tool has its own art (Ping) and its own writing: font from the 3, plus ink colour, weight and texture (pencil grey and grainy, marker thick, quill brown with the odd blot, ballpoint thin blue-black within the palette).
- All styles share the same states/animations (Stanley): idle, peeking while you type, dozing at night (wakes when you type), held/listening, writing, scribbling out, celebrating, sulking at overdue, dropped hard (ink splat/wobble), annoyed (tidying the tack board), returning to the nest.

### Sound
- **On by default**, gentle. Mute toggle in the gear menu. Pen scratch synced to writing; pick-up/drop; tick on done; scribble on delete; page turn; paper crumple; tape/pin.

### Done tasks: the tack board
- Ticking a task done: hand-drawn strike-through, a moment's pause, then **Rambo cuts the line out as a paper strip and tacks it** onto the tack board.
- Tack board: **PC = the right edge of the list page; phone = a strip along the bottom you pull up.**
- Strips hang with simple **simulated physics** (hang from the pin, swing, settle, flutter when dropped). Hand-written small integrator in Rust/wasm, no physics library. Cap about 30 visible strips; older ones stack under the pin.
- The user can drag a strip and:
  - **restore** it (drop it back on the list: task becomes not done),
  - **set it aside** (drop it anywhere on the board/desk: it lies there loose),
  - **crumple and throw it away** (fling it off the page, or drop on the bin: deletes the task, with a crumple animation and sound).
- "Tidy up" (ask Rambo, e.g. tap the board's pin): Rambo grumpily puts all the set-aside strips back on the tack.
- Data: done stays done on the server; tacked/set-aside/positions are presentation only, saved per device in localStorage. Restore = undo done. Crumple = delete.

### Gestures (phone)
- Swipe **right** on a task = delete (Rambo scribbles it out). Swipe **left** = flip the page to the planner (parchment page flip); swipe right on the planner flips back.

### Planner
- Opens at working hours or now; handwritten day names, today underlined; drag a task to another time/day like moving a paper slip; long-press an empty hour pre-fills the capture line with that slot.

### Order of work (ship in this order; each step must leave the app working)
1. Theme + layout + gear menu + fonts + planner hours/day names.
2. Rambo fixed: drag follows pointer, nest = send, listening pose, letter-by-letter writing, bigger, paper colours.
3. Swipe-right delete with scribble-out; page flip; planner drag reorder; sounds on.
4. Tack board with physics.
5. Rambo styles + all moods; Zhipu's extras (rotating placeholder, long-press hour, fading pencil, nib gear).
Deploy candidate = whatever is merged and passes god's end-to-end check of add, write, done, delete, planner.

## 9. Owner answers (2026-10-14), settled

The owner's answers to the questions in `docs/FRONTEND-PLAN.md` (interview 1, recorded in the hive's `SPEC-ANSWERS-2026-10-14.md`). They win over every section above.

### The page
- **One notebook page per page.** The whole app is a notebook, not a generic site; it must feel interactable and unique. The art is redrawn as a high-quality parchment journal page (Ping, reviewed by a Claude agent).
- **The gear menu sits in the corner of the paper**, not in a toolbar.
- **Task text wraps like handwriting in a notebook**, running on to the next ruled line.
- **Tags are tiny and symbolic**: an emoji or a letter.
- **768px upright (a tablet) gets the desktop layout.**
- **Layers:** Rambo goes under dialogs.

### Phone
- The **task list opens first**.
- The **capture bar shows on the planner page too**.
- A **+ button** adds a task by hand.
- With the on-screen keyboard up, **Rambo rides up with the dish** above it.

### Rambo
- **Size:** 1.4x today (64px phone, 80px desktop).
- **Listening (held):** a listening pose, and he rides a little above the finger so the user sees him. The rest of the page dims a little, as if the light were turned down.
- **Dazed** for about 3 seconds after a drop, then home. The time lives in the web tuning file, so it is easy to change.
- **Scrolled or page turned while he is out of the nest:** he is smacked into the side of the screen, or flattened by the paper and pokes a hole through it. Only when he is out of the nest; at home he rides along.
- **Writing takes a bit longer and gets more thought.** v1's writing was "miles better" (UI-BUGS #11, v1 commit `297dd44`): the pen glided along the line in one smooth motion with the ink revealed under it, and wrote along with the caret while you typed.
- **Interruptions:** anything the user does interrupts him (a second capture, a tap, a drag, a page turn). **Writing pauses and resumes** when he is let go, or when the page is turned back. The task is already saved, so a reload shows the title in full at once. Users can wait, and can edit the title meanwhile.

### The physical principle (interview 2)
- **You should know why something happens because it would make sense in real life.** Things come and go like real objects: they slide, lift, fold, are laid down or taken away. Nothing fades or cuts in and out.
- **The one exception is ink.** Ink fades in as it is written.
- **The dark-mode switch is physical too.** For example, the page is swapped for a night page, never a colour cross-fade.
- **A page turn is a real page turn:** the paper bends and curls as it turns. Not a sideways swipe, and no gradient that pops in and out.

### From the owner's phone test (interview 2, on b005174)
- **Dazed for 3 seconds**, then he recovers and skitters home shortly after.
- **Paper:** a brown-ish texture that is clearly writing or journal paper, not craft paper.
- **Tap Rambo while there is text in the capture box: it submits**, as Enter does.
- **Clearing the capture box:** with text in the box, a small paper cut-out × slides out beside it. Tapping it makes Rambo scribble the text out, and a fresh piece of paper is laid down.
- **Writing must be convincing:** today it is a big scribble that blocks the screen. Port v1's writing (UI-BUGS #11, v1 commit `297dd44`).
- **Planner:** it must scroll forward to every day it shows. Scrolling back in time is not needed yet. A full calendar view comes later.

### Sound (interview 2)
Three presets in the settings; the mute stays in the gear menu:
- **A. Quiet desk.** Sound only where paper or ink really makes a noise: the nib's scratch, the page turn, the crumple, the pin, the smack and the flatten.
- **B. Every touch answers.** Every motion has its own soft sound.
- **C. The most immersive.** Every sound, plus calm classical background music that fits the parchment, and Rambo dances to it. Recordings must be CC0 or public domain.

### The desktop capture box (interview 2, Q1)
- **Pinned on desktop too**: it stays on screen while the page scrolls, as on a phone.
- **It is a piece of ripped scratch paper** laid on the desk, and Rambo really writes on it: as you type, his nib follows the words on that scrap. On submit the scrap's words go to the list, and a fresh scrap is laid down (see the paper cut-out x above).
