# The desk lamp: `lamp/`

The lamp on the desk is the dark-mode switch (`web/src/lamp.rs:DeskLamp`): tap it or pull
its cord and a sheet of the other paper is laid down over the page, the paper is swapped
under it, and the sheet goes. Day is the lamp off, night is the lamp on, and by night a
warm pool of light (`.desk-lamp::before`) falls on the page's corner.

| File | Drawn |
|---|---|
| `lamp-off.svg` | the lamp by day (light paper) |
| `lamp-on.svg` | the lamp by night (dark paper), with a warm glow under the shade |

Both are SVG, **viewBox `0 0 60 84`**, the same frame so the two line up, transparent
background. They are placeholders drawn in-repo; an artist can replace them with any art
in the same frame and same names. The pull cord is CSS (`.desk-lamp-cord`), not part of
these files. The lamp is drawn at 72 x 100 px on a desk and 44 x 62 on a phone, with its
base on the bottom of the frame. Colours are fixed in the files, because an SVG used as an image cannot read
the page's tokens.
