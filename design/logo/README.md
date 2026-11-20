# Priory logo

The mark is a closed book with a P cut out of its cover and its ribbon hanging out: the ribbon
marks the page where a person is needed. It is provisional, chosen on 2026-11-20, and may change.

## Files

| File                                            | Use                                                        |
| ----------------------------------------------- | ---------------------------------------------------------- |
| `mark.svg`                                      | The flat mark in `currentColor`, for one-color uses        |
| `mark-16.svg`                                   | The flat mark without the P, for 16 px                     |
| `mark-two-tone.svg`                             | The book in `currentColor`, the ribbon in Indigo's action color |
| `lockup.svg`, `lockup-dark.svg`                 | The mark and the outlined wordmark, for light and dark grounds |
| `favicon.svg`                                   | The flat mark without the P, ink in light mode and white in dark mode |
| `favicon.ico`                                   | 16, 32 and 48 px, the flat mark in the action color, the P from 32 px |
| `icon-192.png`, `icon-512.png`                  | PWA icons, purpose `any`: a rounded ink tile               |
| `icon-maskable-192.png`, `icon-maskable-512.png` | PWA icons, purpose `maskable`: a full-bleed ink tile, the mark inside the 80 % safe zone |
| `apple-touch-icon.png`                          | 180 px, a full-bleed ink tile that iOS rounds              |

The mark is drawn on a 32 px grid with no strokes, its edges on even units so 16 px lands on
whole pixels. The P is drawn on the same grid, not set in type. The wordmark is "Priory" in Mona
Sans 600 at 112.5 % width, converted to a path, so the files need no font. Its cap height matches
the book, from the top of the cover to the bottom of the pages.

## Clear space

Keep a quarter of the mark's height clear on every side: 8 px around a 32 px mark. In the lockup,
measure it from the mark's height.

## Minimum size

- Flat mark: 16 px. At 16 px use `mark-16.svg`: the P's counter closes at that size. Use it for
  the favicon, tabs and sidebars.
- The P: 24 px.
- Two-tone mark: 24 px. Below that, the ribbon's color stops reading; use the flat mark.
- Lockup: 24 px tall.

## Colors

| Use                      | Book      | Ribbon    |
| ------------------------ | --------- | --------- |
| Flat                     | the text color of the ground | the same as the book |
| Two-tone on a light ground | `#0B0B14` | `#5856D6`, Indigo's light action color |
| Two-tone on a dark ground | `#FFFFFF` | `#5E5CE6`, Indigo's dark action color |
| App icon tiles           | `#FFFFFF` on `#0B0B14` | `#5E5CE6` |

Inside the web view, draw the mark in the active theme: the book in `--text` and the ribbon in
`--action`. In Ink, where `--action` equals `--text`, use the flat mark. Never fill the mark with a
gradient, add a shadow or outline it.
