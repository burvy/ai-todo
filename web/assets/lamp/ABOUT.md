# The desk lamp: `lamp/`

The lamp on the desk is the dark-mode switch (`web/src/lamp.rs:DeskLamp`): tap it or pull
its cord and the light is turned down over the page, the paper is swapped under the dim,
and the light comes back up. Day is the lamp off, night is the lamp on.

| File | Drawn |
|---|---|
| `lamp-off.svg` | the lamp by day (light paper) |
| `lamp-on.svg` | the lamp by night (dark paper), with a warm glow under the shade |

Both are SVG, **viewBox `0 0 60 84`**, the same frame so the two line up, transparent
background. They are placeholders drawn in-repo; an artist can replace them with any art
in the same frame and same names. The pull cord is CSS (`.desk-lamp-cord`), not part of
these files. Colours are fixed in the files, because an SVG used as an image cannot read
the page's tokens.
