# Fonts

**Status: placeholder.** Both fonts are stock Google Fonts, not hand-made. The app loads them from Google Fonts. No font files are stored in the repo.

| Font    | Weights loaded | CSS token | Where it is used | Licence |
|---------|----------------|-----------|------------------|---------|
| Caveat  | 500, 600, 700 | `--hand` (fallbacks: Bradley Hand, Segoe Print, cursive) | the handwritten parts: task titles the pen writes, the "To do" heading, planner notes, margin notes | SIL Open Font License 1.1 |
| Inter   | 300, 400, 500, 600; italic 300, 400 | `--chrome` (fallbacks: system-ui, Segoe UI, Helvetica, Arial) | the interface: buttons, settings, meta lines, toasts, forms | SIL Open Font License 1.1 |

Loaded in `static/index.html`:

```
https://fonts.googleapis.com/css2?family=Caveat:wght@500;600;700&family=Inter:ital,wght@0,300;0,400;0,500;0,600;1,300;1,400&display=swap
```

The tokens are defined in `static/styles.css` (`:root`, lines 21–22).

The SIL Open Font License (OFL) allows free use, embedding and self-hosting. You may not sell the font files on their own. A modified version may not reuse the reserved font name.

To replace Caveat with burvy's own handwriting, see "Fonts" in `GUIDE.md`.
