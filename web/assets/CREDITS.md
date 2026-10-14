# Credits — sources and licences for stock assets in `web/assets/`

Everything not listed here was made inside this repository (placeholder art
drawn as SVG by AI agents, sounds synthesized by code) and carries no external
claim. All of it is placeholder, to be replaced by hand-made work (see
`README.md` and `art/GUIDE.md`).

## Sounds (`web/assets/sounds/`)

Every file below is a short cut (times given as `start+length` from the source
recording), faded at both ends, peak-normalized, mono 44.1 kHz MP3.

| File | Source recording | Author | Licence | Source URL | Cut |
|---|---|---|---|---|---|
| `write.mp3` | *Writing with feltpen.ogg* | stephan | Public domain | <https://commons.wikimedia.org/wiki/File:Writing_with_feltpen.ogg> | 2.16 s + 0.70 s |
| `tick.mp3` | *292934-152 Writing-pen-paper-various marks and lines.wav* | soundsnap | CC0 | <https://commons.wikimedia.org/wiki/File:292934-152_Writing-pen-paper-various_marks_and_lines.wav> | 17.02 s + 0.15 s |
| `page.mp3` | *Turning a page.ogg* | planish | Public domain | <https://commons.wikimedia.org/wiki/File:Turning_a_page.ogg> | 1.35 s + 0.97 s |
| `send.mp3` | *Book paper pages assorted.ogg* | stephan | Public domain | <https://commons.wikimedia.org/wiki/File:Book_paper_pages_assorted.ogg> | 75.60 s + 0.62 s |
| `delete.mp3` | *Old book.ogg* | cori | Public domain | <https://commons.wikimedia.org/wiki/File:Old_book.ogg> | 12.80 s + 0.68 s |
| `drop.mp3` | *Old book.ogg* | cori | Public domain | <https://commons.wikimedia.org/wiki/File:Old_book.ogg> | 5.76 s + 0.32 s |
| `listen.mp3` | — made by code (two damped sine bursts, a pen-cap click) | — | none needed (this repo) | — | — |
| `crumple.mp3` | *Food wrapper crumpling.ogg* | stilgar | Public domain | <https://commons.wikimedia.org/wiki/File:Food_wrapper_crumpling.ogg> | 2.58 s + 0.72 s |
| `pin.mp3` | *Tack tack tack.ogg* | stephan | Public domain | <https://commons.wikimedia.org/wiki/File:Tack_tack_tack.ogg> | 0.00 s + 0.12 s |
| `scribble.mp3` | *Writing with feltpen.ogg* | stephan | Public domain | <https://commons.wikimedia.org/wiki/File:Writing_with_feltpen.ogg> | 7.22 s + 0.70 s |
| `toss.mp3` | *cloth1.ogg* (RPG Audio pack) | Kenney | CC0 (Kenney) | <https://kenney.nl/assets/rpg-audio> | whole clip, 0.66 s |

Processing: `ffmpeg` (`-ss`/`-t` cut, `volume` to target peak, `afade` 30–50 ms
in/out, `libmp3lame -b:a 96k`). Target peaks: `write` −14 dB, `tick` −11 dB,
`page` −10 dB, `send` −13 dB, `delete` −11 dB, `drop` −12 dB, `listen` −11 dB,
`crumple` −12 dB, `pin` −10 dB, `scribble` −12 dB, `toss` −12 dB.

## Fonts (`web/assets/fonts/`)

Downloaded from the Google Fonts GitHub repository (upstream `ofl/…`), each
family's full SIL Open Font License sits beside the files (`OFL-*.txt`).
Converted TTF → WOFF2 with `fontTools`.

| File | Family and use | Licence | Source |
|---|---|---|---|
| `patrick-hand.woff2` | Patrick Hand 400 — neat UI hand | OFL | <https://github.com/google/fonts/tree/main/ofl/patrickhand> |
| `caveat-var.woff2` | Caveat variable 400–700 — expressive titles | OFL | <https://github.com/google/fonts/tree/main/ofl/caveat> |
| `kalam-light.woff2` | Kalam Light 300 — ballpoint | OFL | <https://github.com/google/fonts/tree/main/ofl/kalam> |
| `kalam-regular.woff2` | Kalam Regular 400 — fountain pen / quill | OFL | same |
| `kalam-bold.woff2` | Kalam Bold 700 — marker | OFL | same |

## Art

All SVGs in `web/assets/` (Rambo skins and poses, `nest.svg`, mood fx,
tack-board art, `pen.svg`, icons, paper textures) are placeholders drawn in
this repository by AI agents; no external source, no licence needed. The
fountain-pen skin was recoloured to the paper palette (coffee `#5D4732` body,
golden brown `#9F682E` cap) on 2026-10-13; the pencil/ballpoint/quill/marker
skins, `dozing`/`sulking` poses, `rambo/fx/` and `tackboard/` art were
generated in-repo on 2026-10-14 by scripted edits of the fountain poses.
