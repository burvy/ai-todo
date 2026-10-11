Replaceable sounds for ai-todo. Drop files here with exactly these names;
no code change is needed. Missing files fall back to a tiny WebAudio synth.

  write.mp3   the pen writing a new task title
  tick.mp3    a task ticked done
  page.mp3    switching To do / Planner pages on a phone
  listen.mp3  the pen starts listening (hold to talk)
  send.mp3    a ramble or a revise request is sent

Keep them short (under ~0.5 s) and quiet; the app plays them at volume 0.3,
one at a time, never on page load, and not at all when Voice is off in the
settings gear.

Paper: static/assets/paper.jpg (tileable light paper) replaces the SVG grain
when present; paper-night.jpg is optional for the Night desk.
