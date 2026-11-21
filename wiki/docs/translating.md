# Translating Bookx

Bookx speaks English, Spanish, German, French and Hindi. Translations are plain gettext `.po` files, the format
[Weblate](https://weblate.org), Poedit, Lokalize and every other translation tool read, so you do not need to write code.

## Status

| Language | Coverage | State |
|---|---|---|
| English | – | the source text |
| Deutsch (`de`) | everything | machine-translated: written by an AI assistant, not yet read by a native speaker |
| Español (`es`) | everything | machine-translated: written by an AI assistant, not yet read by a native speaker |
| Français (`fr`) | the most visible strings (about 7 %) | machine-translated, starting file |
| हिन्दी (`hi`) | the most visible strings (about 5 %) | machine-translated, starting file |

**Machine-translated** means no native speaker has reviewed the language yet; a language becomes **reviewed** once one
has. Anything not translated shows in English, so a partial file is fine. Corrections from native speakers are the most
useful contribution there is. The table above is kept in `po/STATUS.md`; the shared word list is `po/GLOSSARY.md`.

## How languages reach the app

Every language's `.po` file is compiled into the program, so a Flatpak or a single binary needs no extra files, and
the language list in **Preferences → General → Language** shows the languages that are built in. A compiled `.mo` file
in `$BOOKX_LOCALE_DIR`, `/app/share/locale` or `/usr/share/locale` (`<lang>/LC_MESSAGES/bookx.mo`) wins over the built-in
text, and a language that only exists there is listed too.

## Improve a translation

Two ways, both end up in the same `.po` file:

- **On Weblate** (a web editor, no tools to install): once the project is public, the link is on this page and in the
  README. Suggest a change or translate a string; reviewers approve it and Weblate sends the change back to the project
  as a pull request.
- **With a pull request**: edit `po/<code>.po` with Poedit, Lokalize or any text editor, check it with
  `cargo test -p bookx i18n::`, and open a pull request. Say in the description that you are a native speaker, so the
  language can be marked as reviewed.

## Add a language

1. Get the source (`git clone https://github.com/adhadse/Bookx`) and run `build-aux/i18n.sh`: it rewrites the template
   `po/bookx.pot` from the strings in the program.
2. Copy `po/bookx.pot` to `po/<code>.po` (`po/it.po`), fill in the header (`Language`, `Plural-Forms` for your
   language, see the [plural rules](https://docs.translatehouse.org/projects/localization-guide/en/latest/l10n/pluralforms.html))
   and translate. Read `po/GLOSSARY.md` for the words other languages already settled on and the form of address.
3. Add the code to `po/LINGUAS`, to `LANGUAGES` in `src/i18n.rs` (code, its name in its own language, and
   `include_str!` of the file) and, for number and date style, to `fmt_int_for` / `fmt_date_for` if it differs from
   English. Add a row to `po/STATUS.md`.
4. Run `cargo test -p bookx i18n::`: it checks that the template is current, every placeholder survives, and every
   plural has all its forms.
5. Open a pull request. Text written with an AI assistant must say so in the file header and in the status row
   (`machine-translated`) until a native speaker has read it.

### Things to keep right

- `{}` is a placeholder (a name, a number, a path). Keep every one, in any order that reads well in your language
  (`"Sent {} to {}"`): they are filled in left to right, so if your word order swaps them, rephrase instead.
- **Plurals** have one form per `msgstr[n]` of your language (`"{} book"` / `"{} books"`); the `{}` is the count, written
  with the language's digit grouping.
- A `msgctxt` tells apart two words that are the same in English but not in your language; translate each.
- No label ends in `…` or `...` (menu items, buttons, titles, placeholders, progress texts such as "Loading"); underscores and markup (`<b>`) stay as they are.
- Do not translate file names, `bookx://`, or keyboard names inside backticks.

## Test your translation

```bash
BOOKX_LANG=fr cargo run --release                 # your file is embedded; this picks it
BOOKX_LOCALE_DIR=/tmp/loc cargo run --release     # or a compiled file:
msgfmt -o /tmp/loc/fr/LC_MESSAGES/bookx.mo po/fr.po
```

A `.mo` file in `$BOOKX_LOCALE_DIR`, `/app/share/locale` or `/usr/share/locale` wins over the embedded text, so you can
try a translation without rebuilding. Or choose the language under **Language** on the **General** page of Preferences (main menu → Preferences) (menus and the main
window change after a restart). `make install` also installs the compiled files.

## Weblate

The `.po` files and `po/bookx.pot` follow Weblate's defaults (one gettext component, template `po/bookx.pot`, files
`po/*.po`), and Bookx is set up to use [hosted Weblate](https://hosted.weblate.org). The project page will be linked here
when it is open; until then, use a pull request.
