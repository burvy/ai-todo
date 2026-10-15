# Sounds: `sounds/`

Short mono MP3 cues. The gear menu's **Sound** preset decides which play
(`web/src/sound.rs:plays` is the one event-to-sound table):

| Cue | File | Plays when | Off | A quiet desk | Every touch | Everything + music |
|---|---|---|---|---|---|---|
| Write | `write.mp3` | the pen writes a title (looped while the nib moves) | no | yes | yes | yes |
| Knock | `tick.mp3` | his dry nib knocks on the nest's rim, once a tap (shares the button tick until it has a file of its own) | no | yes | yes | yes |
| Scribble | `scribble.mp3` | a task is crossed out | no | yes | yes | yes |
| Page | `page.mp3` | a page turns (dropped under reduced motion) | no | yes | yes | yes |
| Crumple | `crumple.mp3` | a strip is crumpled into the bin | no | yes | yes | yes |
| Tap | `tick.mp3` | any button is pressed (dropped under reduced motion) | no | no | yes | yes |
| Send | `send.mp3` | a ramble becomes tasks | no | no | yes | yes |

The other files here (`delete`, `drop`, `listen`, `pin`, `toss`) are not wired yet.
The music for the last preset is in `../music/ABOUT.md`.

## Swapping a sound

Replace the file, same name: mono MP3, under 1 s, under 100 KB, peak around
-10 to -14 dBFS (the code also caps each cue's volume at 0.5). Add the source and
licence to `../CREDITS.md`. A test fails if a cue's file goes missing.
