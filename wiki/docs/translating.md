# Translating Bookx

Bookx speaks English, Spanish, German, French and Hindi. Translations are plain gettext `.po` files, the format
[Weblate](https://weblate.org), Poedit, Lokalize and every other translation tool read, so you do not need to write code.

## Status

| Language | File | State |
|---|---|---|
| English | – | the source text |
| Español | `po/es.po` | everything, machine-assisted: needs a native review |
| Deutsch | `po/de.po` | everything, machine-assisted: needs a native review |
| Français | `po/fr.po` | starting file: the most visible 150 strings, machine-assisted |
| हिन्दी | `po/hi.po` | starting file: the most visible 150 strings, machine-assisted |

A header comment in each file says whether it still needs review. Anything not translated shows in English, so a partial
file is fine.

## Improve a translation or add a language

1. Get the source (`git clone https://github.com/adhadse/Bookx`) and run `build-aux/i18n.sh`: it rewrites the template
   `po/bookx.pot` from the strings in the program.
2. **New language:** copy `po/bookx.pot` to `po/<code>.po` (`po/it.po`), fill in the header (`Language`, `Plural-Forms`
   for your language, see the [plural rules](https://docs.translatehouse.org/projects/localization-guide/en/latest/l10n/pluralforms.html))
   and translate. **Existing language:** edit its file; Poedit or `msgmerge -U po/de.po po/bookx.pot` brings in new strings.
3. Add the language to `LANGUAGES` in `src/i18n.rs` (code, its name in its own language, and `include_str!` of the file)
   and, for number and date style, to `fmt_int_for` / `fmt_date_for` if it differs from English.
4. Run `cargo test -p bookx i18n::`: it checks that the template is current, every placeholder survives, and every
   plural has all its forms.
5. Open a pull request. Machine-assisted text must say so in the header (`# Machine-assisted, needs review`) until a
   native speaker has read it.

### Things to keep right

- `{}` is a placeholder (a name, a number, a path). Keep every one, in any order that reads well in your language
  (`"Sent {} to {}"`): they are filled in left to right, so if your word order swaps them, rephrase instead.
- **Plurals** have one form per `msgstr[n]` of your language (`"{} book"` / `"{} books"`); the `{}` is the count, written
  with the language's digit grouping.
- A `msgctxt` tells apart two words that are the same in English but not in your language; translate each.
- `…` ends a menu item that opens a dialog; underscores and markup (`<b>`) stay as they are.
- Do not translate file names, `bookx://`, or keyboard names inside backticks.

## Test your translation

```bash
BOOKX_LANG=fr cargo run --release                 # your file is embedded; this picks it
BOOKX_LOCALE_DIR=/tmp/loc cargo run --release     # or a compiled file:
msgfmt -o /tmp/loc/fr/LC_MESSAGES/bookx.mo po/fr.po
```

A `.mo` file in `$BOOKX_LOCALE_DIR`, `/app/share/locale` or `/usr/share/locale` wins over the embedded text, so you can
try a translation without rebuilding. Or choose the language in *Preferences → General → Language* (menus and the main
window change after a restart). `make install` also installs the compiled files.

## Weblate

The `.po` files and `po/bookx.pot` follow Weblate's defaults: one monolingual-free gettext component, template
`po/bookx.pot`, files `po/*.po`. Bookx has no hosted Weblate project yet; when one exists, it is announced here.
