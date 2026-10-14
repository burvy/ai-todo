# Sounds: `sounds/`

Short mono MP3 cues. The gear menu's **Sound** preset decides which play
(`web/src/sound.rs:plays` is the one event-to-sound table):

| Cue | File | Plays when | A quiet desk | Every touch | Everything + music |
|---|---|---|---|---|---|
| Write | `write.mp3` | the pen writes a title (looped while the nib moves) | yes | yes | yes |
| Scribble | `scribble.mp3` | a task is crossed out | yes | yes | yes |
| Page | `page.mp3` | a page turns (dropped under reduced motion) | yes | yes | yes |
| Crumple | `crumple.mp3` | a strip is crumpled into the bin | yes | yes | yes |
| Tap | `tick.mp3` | any button is pressed (dropped under reduced motion) | no | yes | yes |
| Send | `send.mp3` | a ramble becomes tasks | no | yes | yes |

The other files here (`delete`, `drop`, `listen`, `pin`, `toss`) are not wired yet.
The music for the last preset is (planned).

## Swapping a sound

Replace the file, same name: mono MP3, under 1 s, under 100 KB, peak around
-10 to -14 dBFS (the code also caps each cue's volume at 0.5). Add the source and
licence to `../CREDITS.md`. A test fails if a cue's file goes missing.
