# Fonts

The web view's seven families, as `woff2` files that the daemon serves under `/fonts/`. The view
never loads a font from another host (DESIGN.md, Themes).

Each file is the Latin subset Google Fonts serves, copied byte for byte on 2026-11-20. The variable
files cover a weight range, and Mona Sans also its width axis, 75 % to 125 %.

| Family | Files | Themes | License | Copyright |
| ------ | ----- | ------ | ------- | --------- |
| Mona Sans | `mona-sans-200-900.woff2` | Indigo, Ink | [OFL 1.1](OFL-monasans.txt), Reserved Font Name "Mona" | The Mona Sans Project Authors |
| JetBrains Mono | `jetbrains-mono-400-500.woff2` | Indigo, Ink | [OFL 1.1](OFL-jetbrainsmono.txt) | The JetBrains Mono Project Authors |
| EB Garamond | `eb-garamond-400-800.woff2` | Parchment | [OFL 1.1](OFL-ebgaramond.txt) | The EB Garamond Project Authors |
| IBM Plex Sans | `ibm-plex-sans-400-700.woff2` | Parchment | [OFL 1.1](OFL-ibmplexsans.txt), Reserved Font Name "Plex" | IBM Corp. |
| IBM Plex Mono | `ibm-plex-mono-400.woff2`, `ibm-plex-mono-500.woff2` | Parchment, Abbey | [OFL 1.1](OFL-ibmplexmono.txt), Reserved Font Name "Plex" | IBM Corp. |
| Alegreya | `alegreya-400-900.woff2` | Abbey | [OFL 1.1](OFL-alegreya.txt) | The Alegreya Project Authors |
| Alegreya Sans | `alegreya-sans-400.woff2` to `alegreya-sans-800.woff2` | Abbey | [OFL 1.1](OFL-alegreyasans.txt) | The Alegreya Sans Project Authors |

The SIL Open Font License 1.1 allows bundling and redistributing the fonts with software, here
inside the `priory` binary, as long as each font keeps its license and copyright notice. These
files do that. The license forbids selling a font by itself and giving a modified font a Reserved
Font Name. Priory sells nothing and does not modify the files.

A browser downloads a family only when text in the active theme, or in an Appearance preview,
uses it.
