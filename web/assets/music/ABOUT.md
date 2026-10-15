# Music: `music/`

Calm solo piano that plays softly under the **Everything + music** Sound preset, and
only then. It starts after the first press once that preset is chosen (a browser will
not play sound before a gesture), plays the files in order and starts over after the
last (`web/src/music.rs:TRACKS`). Volume: `web/src/tuning.rs:MUSIC_VOLUME`.

| File | Piece | Performer | Source | Licence |
|---|---|---|---|---|
| `nocturne-op15-no1.mp3` | Chopin, Nocturne in F major, Op. 15 no. 1 (1833) | Luke Faulkner (Musopen) | [Wikimedia Commons](https://commons.wikimedia.org/wiki/File:Chopin_-_Nocturne_Op._15_no._1_in_F_major.ogg), from [Musopen](https://musopen.org/music/2255/frederic-chopin/nocturnes-op-15/) | CC0 (recording); composition public domain |
| `gymnopedie-no1.mp3` | Satie, Gymnopedie no. 1, "Lent et douloureux" (1888) | Robin Alciatore (Musopen) | [Wikimedia Commons](https://commons.wikimedia.org/wiki/File:Erik_Satie_-_gymnopedies_-_la_1_ere._lent_et_douloureux.ogg), from musopen.com | public domain (PD-author); composition public domain |

Both are the Commons originals (Ogg Vorbis) re-encoded to mono 22.05 kHz MP3 at 40 kbps
so that the whole set stays under 3 MB (2.4 MB). Nothing else was changed.

## Swapping a track

Replace the file (same name) or change `TRACKS`. Keep it a calm instrumental recording
whose composition **and** performance are public domain or CC0, from Wikimedia Commons
or Musopen. Keep the set under 3 MB (a test checks it) and add the source and licence
here and in `../CREDITS.md`.
