# Sounds: spec

**Status: placeholder, made by code.** There are no recorded sounds yet. Every sound the app plays today is a short synthesized beep (WebAudio, the browser's built-in sound generator). Nobody has recorded any of them.

## How the v1 app plays a sound

Source: `static/app.js` (`SYNTH`, `synth()`, `play()`, around line 1860–1910).

1. It looks for `assets/sounds/<name>.mp3` and plays it at volume **0.3**, one sound at a time. A new sound stops the previous one.
2. If the file is missing or will not play, it falls back to the synth below.
3. Sounds play only after the user has clicked or typed once, and never when Voice is off in the settings gear.

## Synth fallback parameters (what plays today)

Each sound is one oscillator (a pure tone generator). Its gain (volume) starts at `0.3 × 0.4 = 0.12` and fades exponentially to 0.001 over the length shown.

| Name   | When it plays                                    | Frequency | Length | Waveform |
|--------|--------------------------------------------------|-----------|--------|----------|
| write  | the pen writes a new task title                  | 520 Hz    | 0.05 s | triangle |
| tick   | a task is ticked done                            | 880 Hz    | 0.07 s | sine     |
| page   | switching To do / Planner pages on a phone       | 300 Hz    | 0.06 s | triangle |
| listen | the pen starts listening (hold to talk)          | 660 Hz    | 0.09 s | sine     |
| send   | a ramble or a revise request is sent             | 440 Hz    | 0.08 s | sine     |

## What a real recording should be

| File         | Real foley to record (foley = everyday sounds recorded for a soundtrack) |
|--------------|---------------------------------------------------|
| `write.mp3`  | a pen scratching a short word on paper            |
| `tick.mp3`   | a pencil making one quick tick mark               |
| `page.mp3`   | a single page turn                                |
| `listen.mp3` | a pen cap clicking off                            |
| `send.mp3`   | a soft paper slide or swish                       |

Format: mono mp3, 44.1 kHz, shorter than 1.5 s (the app is tuned for under 0.5 s), smaller than 100 KB, normalized. See `../GUIDE.md` for how to record them.

`README.txt` in this folder is a copy of `static/assets/sounds/README.txt` from v1.
