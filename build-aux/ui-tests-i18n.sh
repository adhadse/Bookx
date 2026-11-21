# Scenarios of translations, plurals, diagnostics and bookx:// links (docs/roadmap-status.md: X6, X17, X19, X20).
# Sourced by ui-tests.sh (uses its `scenario`, `check`, `has`, $ROOT). Dev commands: `library:language=CODE`,
# `library:menus-tr`, `ntr:N`, `diag:`, `open-uri:URI`, `resolve-uri:URI` (src/app.rs).

scenario i18n_french_hindi 'library:language=fr,wait:400,library:menus-tr,ntr:1,ntr:3,library:language=hi,wait:400,library:menus-tr,ntr:1,ntr:3,library:language=xx,wait:400,ntr:2,quit' '
    N=$(grep "^ntr:" <<<"$OUT")
    has "^mainmenu_tr: .*Ouvrir le dossier de la bibliothèque"; check "the French main menu is translated" $?
    has "^mainmenu_tr: .*Aide"; check "the Help entry is in the menu and translated" $?
    sed -n 1p <<<"$N" | grep -q "^ntr: 1 livre | 1[^0-9]234[^0-9]567"; check "French: one book, thousands separated by narrow spaces" $?
    sed -n 2p <<<"$N" | grep -q "^ntr: 3 livres "; check "French: three books" $?
    has "^mainmenu_tr: .*लाइब्रेरी फ़ोल्डर खोलें"; check "the Hindi main menu is translated" $?
    sed -n 3p <<<"$N" | grep -q "^ntr: 1 पुस्तक | 12,34,567"; check "Hindi: one book, lakh grouping" $?
    sed -n 4p <<<"$N" | grep -q "^ntr: 3 पुस्तकें "; check "Hindi: three books" $?
    sed -n 5p <<<"$N" | grep -q "^ntr: 2 books | 1,234,567"; check "an unknown language is English with English plurals" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario i18n_plurals_de_es 'library:language=de,wait:400,ntr:0,ntr:1,ntr:2,library:language=es,wait:400,ntr:1,ntr:5,quit' '
    N=$(grep "^ntr:" <<<"$OUT")
    sed -n 1p <<<"$N" | grep -q "^ntr: 0 Bücher | 1.234.567"; check "German: zero is plural, thousands use dots" $?
    sed -n 2p <<<"$N" | grep -q "^ntr: 1 Buch "; check "German: one book" $?
    sed -n 3p <<<"$N" | grep -q "^ntr: 2 Bücher "; check "German: two books" $?
    sed -n 4p <<<"$N" | grep -q "^ntr: 1 libro "; check "Spanish: one book" $?
    sed -n 5p <<<"$N" | grep -q "^ntr: 5 libros "; check "Spanish: five books" $?'

scenario library_subtitle_translated 'library:language=de,wait:500,library:dump,library:search=zzzz,wait:400,library:dump,library:language=es,wait:500,library:search=,wait:300,library:dump,quit' '
    T=$(grep "^title:" <<<"$OUT")
    sed -n 1p <<<"$T" | grep -q "^title: Start | 2 Bücher"; check "switching the language re-translates the library title and subtitle at once" $?
    sed -n 2p <<<"$T" | grep -q "| 0 von 2 Büchern"; check "a search subtitle (n of total) is translated" $?
    sed -n 3p <<<"$T" | grep -q "^title: Inicio | 2 libros"; check "and again for Spanish" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario diagnostics 'diag:,action:copy-diagnostics,wait:300,quit' '
    D=$(grep "^diag:" <<<"$OUT")
    grep -q "^diag: Bookx [0-9]" <<<"$D"; check "the report starts with the version" $?
    grep -q "GTK [0-9]*\.[0-9]*\.[0-9]*, libadwaita [0-9]" <<<"$D"; check "it names the GTK and libadwaita versions" $?
    grep -q "Features: " <<<"$D"; check "it lists the features" $?
    ! grep -qF "$HOME" <<<"$D"; check "the home folder is not in it" $?
    ! sed "s/com\.adhadse\.Bookx//g" <<<"$D" | grep -qF "$(id -un)"; check "the user name is not in it (the application id is not personal)" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario uri_links 'resolve-uri:bookx://open?id=1,resolve-uri:bookx://open?id=999,resolve-uri:bookx://book/isbn:9999999999999,resolve-uri:bookx://nonsense,resolve-uri:https://example.org/,open-uri:bookx://open?id=1,wait:3500,reader:dump,quit' '
    R=$(grep "^resolve_uri:" <<<"$OUT")
    sed -n 1p <<<"$R" | grep -q "book 1"; check "an id link names the book" $?
    sed -n 2p <<<"$R" | grep -q "no book"; check "an id that is not in the library names no book" $?
    sed -n 3p <<<"$R" | grep -q "no book"; check "an unknown identifier names no book" $?
    sed -n 4p <<<"$R" | grep -q "invalid"; check "a bookx link of an unknown kind is invalid" $?
    sed -n 5p <<<"$R" | grep -q "invalid"; check "another scheme is not ours" $?
    has "^labels:"; check "opening the link opens the book in the reader" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'
