# Wiki colour theme

Internal design note for the public wiki's palettes (`wiki/docs/assets/extra.css`). The light palette is unchanged;
the dark one was redesigned ("night paper").

## Why

The old dark scheme was Material's default `slate` with a neutral `#242424` page and the GNOME blue link colour. It looked
like every other docs site and clashed with nothing and matched nothing. Bookx is a reading app, so the dark scheme is
meant to feel like reading at night: warm, calm, high contrast, with one accent.

Directions mocked up as CSS and compared on the built pages (home, a long page with admonitions, tables and code, the
editing page, a phone width):

- **(a) night paper**: ink-brown page, parchment text, amber accent. **Chosen.** It is the colour of the library's
  "Continue reading" strip (a warm brown) in the screenshots, the Adwaita `#242424` windows sit on it as slightly
  cooler, lighter panels (so the framed screenshots read as windows, not as holes), and blue (the logo, the light theme)
  and amber are complementary.
- (b) midnight ink-blue with cream text: pretty, but close to every "dark docs" theme and the screenshots' blue switches
  and highlights blend into the page.
- (c) near-black neutral with a sepia accent: the accent is too weak to be memorable.

## Tokens (dark, `[data-md-color-scheme="slate"]`)

| Token | Value | Use |
|---|---|---|
| `--bx-page` | `#17130f` | page |
| `--bx-bar` | `#1e1913` | header, tabs, footer |
| `--bx-surface` | `#231e17` | code blocks, admonitions, tables, cards |
| `--bx-raised` | `#2e271e` | inline code, table head, search, format chips |
| `--bx-line` | `#3d3427` | borders and rules |
| `--bx-text` | `#ece5d6` | body text (parchment) |
| `--bx-muted` | `#bdb3a0` | secondary text |
| `--bx-faint` | `#8f8571` | decoration |
| `--bx-accent` | `#f0b64f` | links, active nav, primary button (the one accent) |
| `--bx-accent-hover` | `#ffd07a` | hover |
| `--bx-on-accent` | `#1b1409` | text on the accent |

Code highlighting (`--md-code-hl-*`) uses a muted set of the same warmth (terracotta keywords, sage strings, dusty blue
functions, lilac constants). Admonition families: note/info blue `#8fc0e0`, tip/success sage `#a7c98a`, question
(the `???` blocks) amber, warning orange `#ee9a52`, danger/bug red `#ec7b74`, example lilac `#b79be0`, quote faint.

Other choices: headings h1/h2 and the home page titles in a system serif stack (no web fonts: the privacy page promises
no third-party requests), underlined amber links, cards with an amber hover border, a soft amber glow behind the hero
and a gradient "Try Bookx" panel (CSS only, no images), `::selection` in amber, a 2px amber `:focus-visible` ring, brown
scrollbars, transitions only under `prefers-reduced-motion: no-preference`. The serif is dark-only so that the light
palette stays exactly as it was.

## Things that must not move

- The scheme keeps Material's name `slate` (toggle, stored preference, `#only-dark` pictures,
  `build-aux/wiki-theme-check.py`).
- `--md-primary-bg-color` stays `#ffffff` in the dark block: `build-aux/screenshot-frame.py` outlines the dark
  screenshots with it (a faint white ring, alpha 22) and `build-aux/check-wiki-frames.py` reads it from `extra.css`.
  The header text colour is set explicitly instead. The framed pictures have transparent corners and a black shadow, so
  they work on any dark page: **no picture needs re-framing.**
- The social cards use `[project.plugins.social]` in `wiki/zensical.toml`: background `#17130f`, text `#f6efe0`
  (was navy).

## Contrast

`build-aux/check-wiki-contrast.py` (called from `build-aux/wiki.sh`) parses the tokens from `extra.css` and fails below
WCAG AA (4.5:1 text, 3:1 UI parts). Results at the time of writing:

| Pair | Ratio |
|---|---|
| text on page | 14.7 |
| text on surface / raised | 13.2 / 11.8 |
| muted on page / bar / surface | 8.9 / 8.4 / 8.0 |
| faint on page | 5.1 |
| accent on page / surface / raised | 10.1 / 9.1 / 8.1 |
| accent hover on page | 12.8 |
| button text on accent | 10.0 |
| code tokens on surface | 5.2 (comments) to 9.1 |
| admonition accents on surface | 6.0 to 9.0 |
| light: text on page / link on page | 12.1 / 4.6 |
