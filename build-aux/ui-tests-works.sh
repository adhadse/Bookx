# Scenarios of grouped editions (docs/pain-points-ebook-managers.md PP-G-5). Sourced by ui-tests.sh (uses its
# `scenario`, `check`, `has`, $ROOT, $CLI). The library of a `works_*` scenario (see the seed in ui-tests.sh) has
# books 1 and 2 = "Edition Tale" by Ed Writer from two publishers, 3 = its German translation, 4 = "Other Tale",
# 5 and 6 = "UI Test Book" and "Second Book" (the preamble's import). Dev commands: `library:works=on|off|link:A+B|unlink:ID|prefer:ID|
# card:ID[:ctrl]|show:ID|cards|selected`.

# what the catalogue holds: `works_db <dir> <table>` prints "book work preferred" rows
works_db() { python3 -c 'import sqlite3,sys; c=sqlite3.connect(sys.argv[1]+"/lib/bookx-library.db"); print(" ".join(" ".join(map(str,r)) +";" for r in c.execute(sys.argv[2])))' "$1" "$2"; }

scenario works_off_by_default 'library:works=cards,quit' '
    has "^works: cards=6 books=6 groups=\[\] subtitle=6 books"; check "grouping is off by default: every book has its card" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario works_group 'library:works=on,wait:600,library:works=cards,library:works=off,wait:600,library:works=cards,quit' '
    has "^works: cards=5 books=6 groups=\[1=\[1, 2\]\] subtitle=6 books"; check "two editions of one work make one card, the books are still counted by book" $?
    ! has "groups=\[[^]]*3"; check "the German translation is not an edition of it" $?
    has "^works: cards=6 books=6 groups=\[\]"; check "turning the setting off shows every book again" $?
    grep -q "\"group_editions\": *false" "$d/cfg/bookx/config.json"; check "the setting is saved" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario works_select_bulk 'library:works=on,wait:600,library:works=card:1,wait:300,library:works=selected,library:favourite,wait:300,library:works=card:4:ctrl,wait:300,library:works=selected,quit' '
    has "^works: selected=\[1\] books=\[1, 2\]"; check "selecting a group card selects its editions" $?
    has "^works: selected=\[1, 4\] books=\[1, 2, 4\]"; check "it adds up with other cards" $?
    python3 -c "import json,sys; f=json.load(open(sys.argv[1]))[\"favourites\"]; sys.exit(0 if 1 in f and 2 in f and 4 not in f else 1)" "$d/cfg/bookx/config.json"; check "a bulk action (favourite) reaches every edition of the card" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario works_link_unlink 'library:works=on,wait:600,library:works=link:4+5,wait:600,library:works=cards,library:works=unlink:2,wait:600,library:works=cards,library:works=unlink:4,wait:600,library:works=cards,quit' '
    C=$(grep "^works: cards=" <<<"$OUT")
    sed -n 1p <<<"$C" | grep -q "cards=4 books=6 groups=\[1=\[1, 2\] 4=\[4, 5\]\]"; check "two unrelated books can be linked as editions" $?
    sed -n 2p <<<"$C" | grep -q "cards=5 books=6 groups=\[4=\[4, 5\]\]"; check "unlinking an automatically grouped edition takes it out" $?
    sed -n 3p <<<"$C" | grep -q "cards=6 books=6 groups=\[\]"; check "unlinking one of a linked pair dissolves the group" $?
    [ "$(works_db "$d" "select count(*) from work_links")" = "0;" ]; check "no link is left in the catalogue" $?
    works_db "$d" "select book from work_apart order by book" | grep -q "2; 4;"; check "unlinked books stay out of the automatic grouping" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario works_prefer 'library:works=on,wait:600,library:works=prefer:2,wait:600,library:works=cards,library:works=card:2,wait:400,library:works=selected,quit' '
    has "^works: cards=5 books=6 groups=\[2=\[2, 1\]\]"; check "the preferred edition stands for the card" $?
    has "^works: selected=\[2\] books=\[2, 1\]"; check "its card selects both editions" $?
    [ "$(works_db "$d" "select book,preferred from work_links order by book")" = "1 0; 2 1;" ]; check "the choice is kept in the catalogue (and the automatic group became a link)" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario works_popover 'library:works=on,wait:600,library:works=show:1,wait:600,a11y-audit:,library:icons,quit' '
    has "^works: popover for 1 rows=2"; check "the edition list has a row per edition" $?
    has "^a11y: controls=[0-9]+ problems=0"; check "every control of the list has a name" $?
    has "^icons: checked=[0-9]+ missing=\\[\\]"; check "the icons exist" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario works_table 'library:works=on,library:table,wait:800,library:works=cards,quit' '
    has "^works: cards=5 books=6"; check "the table groups too" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'
