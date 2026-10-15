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
| Pin | `pin.mp3` | a scrap or a done strip arrives at the pin on the list's page | no | yes | yes | yes |
| Tap | `tick.mp3` | any button is pressed (dropped under reduced motion) | no | no | yes | yes |
| Send | `send.mp3` | a ramble becomes tasks | no | no | yes | yes |
| Delete | `delete.mp3` | a task is deleted and its row tears into a scrap | no | no | yes | yes |
| Toss | `toss.mp3` | Rambo is thrown, by a flick or swept off by the page | no | no | yes | yes |
| Drop | `drop.mp3` | Rambo is put down: in the nest, on the page, a task or an hour | no | no | yes | yes |
| Listen | `listen.mp3` | Rambo starts listening | no | no | yes | yes |

Rambo's two (`Toss`, `Drop`) come from `sound::for_change`, the one table of which change of
his state makes which sound.
The music for the last preset is in `../music/ABOUT.md`.

## Swapping a sound

Replace the file, same name: mono MP3, under 1 s, under 100 KB, peak around
-10 to -14 dBFS (the code also caps each cue's volume at 0.5). Add the source and
licence to `../CREDITS.md`. A test fails if a cue's file goes missing.
