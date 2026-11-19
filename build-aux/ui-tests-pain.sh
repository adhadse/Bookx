# Scenarios of the pain-point work (docs/pain-points-calibre.md, docs/pain-points-ebook-managers.md): citations,
# file integrity, small covers, offline mode, the reading ruler, "not a duplicate". Sourced by ui-tests.sh (uses its
# `scenario`, `check`, `has`, $ROOT, $CLI). Dev commands: `library:citations=…`, `library:health=verify|scan-covers`,
# `prefs:privacy-offline=on|off`, `prefs:ruler-on|off`, `reader:ruler=toggle|up|down|state`.

scenario pain_citations 'library:citations=write=bibtex|UI Test|@D@/refs.bib,library:citations=write=ris||@D@/refs.ris,library:citations=write=csl-json||@D@/refs.json,library:citations=write=apa|nothing-matches-this|@D@/refs.txt,library:citations=write=mla||@D@/none.txt,library:citations=open,wait:800,quit' '
    has "^citations: wrote 1 BibTeX"; check "the search narrows the bibliography to the books it finds" $?
    grep -q "^@book{author.*," "$d/refs.bib" && grep -q "title = {{UI Test Book}}" "$d/refs.bib"; check "BibTeX has a key and the title" $?
    grep -q "^TY  - BOOK" "$d/refs.ris" && grep -q "^TI  - UI Test Book" "$d/refs.ris"; check "RIS has one tag per line" $?
    python3 -c "import json,sys; v=json.load(open(sys.argv[1])); sys.exit(0 if v and v[0][\"type\"]==\"book\" else 1)" "$d/refs.json"; check "CSL JSON is valid JSON with book items" $?
    has "^citations: wrote 0 APA"; check "a search that finds nothing writes an empty bibliography" $?
    has "^citations: unknown style mla"; check "an unknown style is refused" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "the window opens without errors or GTK warnings" $?'

scenario pain_health 'library:health=open,wait:800,library:health=verify,wait:1500,library:health=scan-covers,wait:1500,quit' '
    has "^health_fixity: .*1 DAMAGED"; check "a file whose content changed while its date and size did not is reported as damaged" $?
    has "^health_fixity: [0-9]+ file\(s\) checked: [0-9]+ intact"; check "the summary counts the intact files" $?
    has "^health_covers: poor=1 total=[0-9]+"; check "the small cover is found among the covers" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario pain_offline 'prefs:,wait:600,prefs:pick=Privacy,prefs:privacy-offline=on,wait:300,prefs:search=connect,wait:300,quit' '
    grep -q "\"offline\": *true" "$d/cfg/bookx/config.json"; check "offline mode is saved in the settings" $?
    ! has "panicked|CRITICAL|Gtk-WARNING.*(rivacy|ffline)"; check "no errors from the Privacy page" $?'

scenario pain_ruler 'read:1,wait:2500,reader:ruler=state,reader:ruler=toggle,wait:1500,reader:ruler=state,reader:ruler=down,reader:ruler=down,reader:ruler=up,reader:ruler=toggle,quit' '
    R=$(grep "^ruler: " <<<"$OUT")
    sed -n 1p <<<"$R" | grep -q "enabled=false visible=false"; check "the ruler is off by default" $?
    sed -n 2p <<<"$R" | grep -q "enabled=true visible=true"; check "the menu entry turns it on" $?
    t() { sed -n "$1p" <<<"$R" | sed -E "s/.*band=([0-9]+)\\.\\..*/\\1/"; }
    [ "$(sed -n 3p <<<"$R" | sed -E "s/.*view_h=([0-9]+).*/\\1/")" -gt 100 ]; check "the overlay covers the page" $?
    [ "$(t 5)" -gt "$(t 3)" ]; check "Alt+Down moves the band down" $?
    [ "$(t 6)" -lt "$(t 5)" ]; check "Alt+Up moves it back up" $?
    sed -n 7p <<<"$R" | grep -q "enabled=false visible=false"; check "it can be turned off again" $?
    grep -q "\"enabled\": *false" "$d/cfg/bookx/config.json"; check "the choice is saved" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario pain_apart 'library:dups,library:dupsvisible,library:dupsapart,library:dupsvisible,library:dups,quit' '
    has "^dups: 1 groups"; check "three copies form a group" $?
    has "^dupsvisible: 1"; check "the group is shown" $?
    has "^dupsapart: 1 groups saved=true"; check "Not a Duplicate is remembered" $?
    has "^dupsvisible: 0"; check "the remembered group is no longer suggested" $?
    [ -s "$d/lib/.bookx/not-duplicates.json" ]; check "the memory is a file in the library" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'
