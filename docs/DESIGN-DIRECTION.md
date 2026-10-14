# Design direction (Fable, 2026-10-14)

What the front end must look and feel like, from the owner's test of main `02ff51e`. Builders follow this file together with `docs/DESIGN-DECISIONS.md`; on settled behaviour (gestures, the tack board, the order of work) DESIGN-DECISIONS wins, on look and motion this file wins. Token names here replace the ones in `docs/THEME.md` where they differ.

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

He has a voice (text to speech) and a bubble, and they are one thing. A line he says appears in **one paper speech bubble rising from the dish** (wheat paper, a `--rambo-tint` rim, a torn tail pointing at him), written in the neat hand, one line, at the moment the voice starts, and it stays until the voice ends plus 800ms. Muted, the bubble alone shows for 1.5s. He never says anything that is printed elsewhere on the page, and his lines are never loose text in a corner. While speaking he tilts 3° toward the capture line and his eyes narrow; listening (held) is the existing pose.

## 4. The writing animation

Verdict on #213: the model is right, the choreography around it is not. "Lay the whole title out at once and clip only the ink, with the nib at the ink edge, each glyph paid for by its width" is exactly letters appearing under a pen, and it ends the row reflow that today stacks "— 30 min" vertically. Keep `pen::schedule`, `inked`, `PER_GLYPH_MS = 90`, the 500–2400ms clamp, the word-gap pause and the scratch sound tied to nib movement. Change the rest:

- **Beat 0, submit (0ms).** The typed text leaves the capture line in 150ms. The new row appears at the top of the list at full height with an empty ruled line: nothing below it moves again.
- **Beat 1, lift-off (0–450ms).** Rambo hops out of the dish (80ms squash) and flies to the start of the empty line in 450ms along a slight upward arc, ease-in-out, rotating from 12° to his writing angle of 35°. He lands nib-first with a 60ms dip. No 2000ms linger at the caret: he goes when you submit.
- **Beat 2, writing.** Nib on the baseline, body up and to the right, so the pen never covers the words. He is not shrunk (drop `WRITE_SCALE`); he is offset. The ink edge is soft, 2px, not a hard clip. A word gap is a 2px lift and drop of the pen. A title that wraps is traced on its first line, then the pen hops to the second line's start and continues; the wipe-only fallback is not acceptable.
- **Beat 3, finish (120ms + 250ms).** A short flick as the last letter closes, the scratch stops, a 250ms hold.
- **Beat 4, home (450ms).** Fly back to the dish, ease-out, one small overshoot, settle in 150ms. Then he speaks ("Okay, that one's in.") per section 3.
- **Speed rule.** Rambl is a fast capture tool. A second submit during writing completes the current ink instantly and sends Rambo straight to the new row: no queue, no waiting.
- **Reduced motion.** The title appears at once in ink; Rambo stays in the dish; the bubble still appears.

For 20 glyphs the whole thing is about 3.2 seconds, and the row is readable from beat 2.

## 5. Five things that make the app look dysfunctional, and the target look

1. **Rambo's speech is loose italic text in the page corner** ("Okay, that one's in.", "Adding it now.", printed over the last row on the phone). Target: section 3. Physically: a voice comes from a mouth, so the words come from him.
2. **The planner is a 24-hour ladder with no day names** (columns start at 00:00, the names scroll out of view, tasks pile up at 9 pm in 16px slips cut with "…"). Target: each column headed in the neat hand, "Tue 14", today underlined, pinned while the hours scroll; hours written as "8 am", "12 pm", "5 pm" (12-hour clock is the default, 24-hour is a gear-menu choice); the working day 8 am to 8 pm unless the gear menu widens it; a slip is a strip of paper never shorter than two lines of its title, with a 1px torn edge and the sheet shadow, and it lifts 2px while you slide it. Nothing on it is cut.
3. **Rows break their meta into fragments** ("— 30 min · Tue" on one line, "21:00" alone on the next; a ticked task looks the same as an open one; nothing appears on hover). Target: the title on its own line in the expressive hand; under it one small neat-hand line that never splits: "30 min, Tue 9 pm"; done is a hand-drawn strike and the ink fading to `--ink-faint`; on desktop a small drawn x appears at the row's right edge on hover. **Tags** go on the same meta line as **ink stamps**: the tag word in the neat hand, lower case, in `--accent`, inside a hand-drawn oval outline 1px thick that does not quite close, rotated between -3° and 3° by a hash of the tag name, with no fill. Several tags sit in a row with a 0.6em gap. They never become pills, never get a solid background, and tapping one filters the list with the stamp pressing 1px into the paper.
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
