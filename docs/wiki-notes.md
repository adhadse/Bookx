# Wiki notes (internal)

How the public wiki (`wiki/`, https://bookx.adhadse.com) is built and deployed, what the owner must set up by hand,
and which pages are weak. User-facing content only goes into `wiki/`; this file stays internal (see AGENTS.md).

## How it works

- **Generator:** [Zensical](https://zensical.org/) (`zensical==0.1.1`, pinned in `build-aux/wiki.sh`). Config:
  `wiki/zensical.toml` (`[project]`, `[project.theme]`, navigation, search plugin, edit links to
  `github.com/adhadse/Bookx/edit/main/wiki/docs/`). Pages: `wiki/docs/*.md`, pictures: `wiki/docs/assets/`.
- **Look:** classic variant with light and dark palettes and `wiki/docs/assets/extra.css` (libadwaita colours,
  rounded corners, system fonts: Cantarell/Adwaita Sans where installed, no Google Fonts request).
- **CNAME:** `wiki/docs/CNAME` is copied to the site root by Zensical (the build script checks it).
- **Build/check:** `build-aux/wiki.sh` (venv in `~/.cache/bookx-wiki-venv` with Zensical and Pillow, output `wiki/site/`,
  gitignored): build, `wiki-optimize.py` (WebP, lazy loading), `check-site-links.py` (stdlib only: internal links, anchors,
  images, CSS/JS references, light/dark picture pairs), `wiki-theme-check.py` (the palette toggle in headless Firefox;
  skipped without Firefox, `WIKI_BROWSER=0` skips it). Zensical itself also reports broken links in the Markdown.
- **Deploy:** `.github/workflows/wiki.yml`. Pull requests that touch `wiki/**` build and check only. Pushes to `main`
  that touch `wiki/**`, and manual runs, also upload the Pages artifact and run `actions/deploy-pages`
  (permissions `pages: write`, `id-token: write`). The existing `adhadse/wiki` repository deploys by pushing to a
  `gh-pages` branch with `peaceiris/actions-gh-pages`; this repository uses the newer artifact-based Pages deployment
  instead, so no extra branch is needed.

## What the owner must do by hand

None of this could be done or verified from the repository.

1. **Push** the repository to GitHub as `adhadse/Bookx` on branch `main` (the edit links, repo link and the workflow
   assume that name and branch; change `repo_url`/`edit_uri` in `wiki/zensical.toml` if it differs).
2. **GitHub → Settings → Pages → Build and deployment → Source: "GitHub Actions"**.
3. **DNS**: at the DNS provider of `adhadse.com`, add a `CNAME` record: name `bookx`, value `adhadse.github.io`
   (the user account's Pages host). Do not add an `A` record for a subdomain.
4. **GitHub → Settings → Pages → Custom domain**: `bookx.adhadse.com`, save. With the Actions source GitHub takes the
   domain from this setting; the `CNAME` file in the artifact is kept so the two never disagree. Wait for the DNS check
   to pass.
5. Tick **Enforce HTTPS** once GitHub has issued the certificate (can take up to about an hour after the DNS check).
6. Run the workflow once (*Actions → Wiki → Run workflow*) or push a change under `wiki/`.
7. If the `github-pages` environment has branch protection rules, allow `main`.

## Not verified

- The workflow has never run on GitHub (no git repository here). The build step is the same script that was run
  locally; the upload and deploy steps follow the standard `actions/upload-pages-artifact@v3` /
  `actions/deploy-pages@v4` recipe.
- The custom domain, DNS and HTTPS.
- The site was only looked at in headless Firefox (desktop and a 420 px wide window), not in Chrome, Safari or on a phone. The
  theme check needs Firefox on the CI runner (`ubuntu-latest` has it); if that ever breaks the deploy, set `WIKI_BROWSER=0` in the workflow.
- Zensical is young (0.x): config keys can change between versions. Upgrade by changing `ZENSICAL_VERSION` in
  `build-aux/wiki.sh` and running it.

## Screenshots (enhancement 27)

One command re-takes every screenshot of the wiki: `build-aux/wiki-screenshots.sh` (`SKIP_BUILD=1` reuses the built
binaries, `CARGO_TARGET_DIR=...` picks the build, `SCHEMES=dark`, `SCENARIOS='_wiki_hero _wiki_map'` and `DRY=1` narrow or
stop it; `build-aux/wiki-screenshots-reader.sh` is the Reading page only). Re-run it after any UI change that shows on a page.

**Light and dark.** The scripts run every scenario twice, with `WIKI_SCHEME=light` and `dark`: the sandbox's GNOME
`color-scheme` is set (`org.gnome.desktop.interface`, in the keyfile settings of `build-aux/session/sandbox.py`), so
libadwaita draws the real Light or Dark style. The result is `wiki/docs/assets/<name>-light.png` and `<name>-dark.png`.
Pages show both: `![alt](assets/x-light.png#only-light)` and `![alt](assets/x-dark.png#only-dark)`. Zensical's CSS hides
the one that does not match `data-md-color-scheme` (the palette toggle sets it), `extra.css` has the same rule for the
landing page's `<img class="only-light">`. `build-aux/check-site-links.py` fails when a picture has no partner, when a page
shows only one variant, when the fragment is missing, or when a picture is not used by any page;
`build-aux/wiki-theme-check.py` (headless Firefox over Marionette, standard library only) clicks the real palette toggle on
five pages and checks that exactly the matching variants are visible. `WIKI_SHOTS=/some/dir build-aux/wiki.sh` also saves
full-page screenshots of the built site (light and dark, desktop and phone width) there.

**No focus rings, no cursor (bug 10).** Four layers: (1) every scenario writes a user style sheet
(`$XDG_CONFIG_HOME/gtk-4.0/gtk.css`, `no_rings()` in `scenarios_wiki.py`) that turns every outline off, so no keyboard-focus ring,
no focused-entry outline and no focus-visible ring of the reading canvas is drawn whatever keys the scenario pressed (GTK loads it
above the application's CSS); (2) `wshot()` takes the picture with `cursor=false` and first parks the pointer on the shell's top bar, so
nothing is drawn hovered (`park=False` only for the book map's hover card); (3) `build-aux/check-screenshots.py` scans the raw
captures (from `wiki-screenshots.sh`, before framing) and the finished pictures (from `wiki.sh`) for the shape of a ring (two long thin
accent-blue horizontal lines of the same extent joined by thin blue vertical ones; progress bars, sliders and fills do not match;
`--allow FILE` lists pictures that may show one; thresholds at the top of the script) and fails on a scenario that asks for the
cursor; (4) it found the rings that were in the first capture (reading canvas, editor preview, Notes search).
It does not look for a cursor sprite in the pixels: the cursor is absent by construction.

**Where the scenarios are.** `build-aux/session/scenarios_wiki.py` (hero banner, the reader page with highlights and its
popover, strip, details, notes, table, book map, read aloud, editor preview and split menu, vertical text, e-ink) plus
`_wiki_shots` (`scenarios_a11y.py`: device banner, Device window, Preferences, palette, editor) and `_wiki_reader_ux`
(`scenarios_reader_ux.py`: sidebar, handles, Wikipedia, footer). The leading `_` keeps them out of the normal
`session-tests.sh` run. They use the public-domain test books of `~/.cache/bookx-testbooks`, a shelf of twelve generated
books (`build-aux/make-wiki-books.py`: public-domain titles whose covers are the public-domain cover art of Standard
Ebooks, downloaded once into `~/.cache/bookx-wiki-covers` and cropped to 2:3; `NET=0` or any download error falls back
to a drawn Pillow cover), a fake Boox folder and the mock read-aloud engine, and take the picture without the mouse
pointer. `wiki-screenshots.sh` fails when a picture of the `PICTURES` list was not taken (a scenario that skips a step
still passes by itself); add new pictures to that list.

**The window shadow.** Mutter in a headless session draws no shadow, so `build-aux/screenshot-frame.py` paints it (Pillow):
the GNOME top bar is cut off, the corners are rounded (14 px, as GNOME draws them), a 1 px outline is added and a soft shadow
(blur 24 px, offset 8 px down, alpha 0.35 on light, 0.5 on dark) goes around the window on a transparent background.
Every window is maximised on its screen (a floating window on a bigger screen would show the wallpaper; the scenarios that
need a bigger screen press Super+Up). The shadow is tuned per variant, because a black shadow is nearly invisible on a dark
page: the dark variant has a stronger shadow and a light outline. `screenshot-frame.py --preview OUT.png A.png B.png` composites
framed pictures on a light and a dark page to judge it.

**Size.** The framed PNGs (about 1392 x 880, RGBA) are the sources in `wiki/docs/assets/`. `build-aux/wiki-optimize.py` runs
on the built site: lossy WebP with a lossless alpha channel (4 to 5 times smaller; PNG quantisation was tried and rejected, it
bands the cover gradients and destroys the shadow), `loading="lazy"` with width and height on every picture (a hidden variant is
never fetched), PNGs dropped from the built site. It needs Pillow, which `wiki.sh` installs into the wiki venv.

**Landing page.** `wiki/docs/index.md` is Markdown with HTML blocks (`md_in_html`), the classes are in
`assets/extra.css`: hero (tagline, description, Install / Documentation / GitHub, the library with the Continue Reading card
as a large banner), six feature rows with alternating picture and text (they stack on a phone), a card grid
(`grid cards`, Material icons through `pymdownx.emoji`), the format strip, a privacy/open-source paragraph, an install call to
action and community links. The structure follows what a product page such as readest.com does (hero, alternating feature
blocks, grid, formats, install, community); no text or asset is copied. The page hides the sidebar, table of contents and footer
navigation (`hide:` in the front matter). The hero's large banner is the app's own *Continue reading banner: Large*
setting (config `banner_mode`), because the automatic banner is slim on a 1280 x 800 window. All claims on the page come from
the feature pages; device support is described as new and tried on one Kobo only.

**Known gaps.** The e-ink picture is the normal screen with the `eink` setting, not a real e-ink panel. The sync-summarydialog (it was a mockup) is no longer shown. Pictures that depend on the app's look change with every UI pass: the
last capture is from the date in the log below.

Log: 2026-11-14 (wiki agent): all 30 pictures x 2 styles re-taken in one `wiki-screenshots.sh` run, 0 focus rings found, every picture looked at; the
shelf covers are real public-domain Standard Ebooks covers (`make-wiki-books.py`, cached in `~/.cache/bookx-wiki-covers`, drawn covers offline).
If the machine runs out of inotify instances the script waits 90 s and tries again (3 times).
Log: 2026-11-14: the shelf now uses real Standard Ebooks cover art (`make-wiki-books.py`), and the "Pages made for
reading" picture is `_wiki_reading` (five highlights in four styles, the selection popover with a note and the Wikipedia
look-up); the hero, reader, library, details, notes, strip and table pictures were retaken in both styles.

## Pages that are thin or need more screenshots

Real screenshots cover the library (hero banner and strip, details, notes, table, device banner), Device window, Preferences, palette, reader
(page, contents, sidebar, notes, handles, Wikipedia, footer, vertical, e-ink), book map, read aloud (three), and the editor (preview, find,
palette, split menu). Missing:

| Page | What is missing |
|---|---|
| Getting started | screenshots of the welcome page, *Add from the Web* and the Calibre import report |
| Library | Library Health window, Preferences → Library files, Find Duplicates, the Trash |
| Reading | style tweaks dialog, colour modes (Theme/Original/Custom), vocabulary and statistics windows, highlight colours |
| Editing | Tools menu, Check results panel, contents editor, checkpoints |
| Converting | Convert To menu |
| Devices and sharing | Share Library dialog, e-mail preferences, the sync summary. No real hardware (except one Kobo) is behind any statement (see `docs/hardware-test-plan.md`) |
| News and Web | News Subscriptions window and the edit dialog |
| Plugins and patches | Preferences → Plugins page, a patch dry run |
| Command line | none needed; check the examples against `bookx-cli --help` when the CLI changes |
| Shortcuts | only lists what is in the code; the editor and reader keys that are not in the in-app shortcuts window were taken from the key handlers |
| Troubleshooting | thin; grows with real bug reports |

Statements to re-check when the app changes: shortcut tables (`src/app.rs`, `src/ui/editor/mod.rs`,
`src/ui/reader.rs`), the Tools menu labels (`src/editor_tools.rs`), the format table (`bookx-cli formats`), the Share
Library dialog (`src/ui/share.rs`), the news dialog (`src/ui/news.rs`).
