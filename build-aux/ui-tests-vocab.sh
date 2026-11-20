# Scenarios of your own vocabulary (docs/enhancements.md "Own vocabulary"): saving a term that is in no dictionary with
# your own definition, marking it in the text, the popover of a marked term, "Your definition" in the look-up popover, the
# list with its filters, a pasted glossary, CSV import and exports, the setting that turns the marks off. Sourced by
# ui-tests.sh (uses its `scenario`, `check`, `has`, $ROOT, $d). The test book (book 1) has the sentence
# "Paragraph 1 of chapter 1 talks about functionality, readers and the world around them in plain words." in every
# paragraph; commas cannot be used in a dev script, so the definitions have none. Dev commands: `reader:select-text=T`,
# `reader:ui-vocab-add` (the button of the selection popover), `reader:ui-vocab-dialog=term=|def=|note=|scope=book|all|save`,
# `reader:ui-vocab-click` (a click on the first marked term of the page), `reader:ui-vocab-mark=edit|remove|open`,
# `reader:vocab-dump`, `reader:ui-vocab-def-btn`, `reader:ui-vocab-terms`, `vocab:own=term|def[|book]`, `vocab:own-dump`,
# `vocab:show-words`, `vocab:list=filter=mine|search=Q|add=T|D|paste-open|paste-text=..|paste-add|import=F|export=KIND:F|edit=T|delete=T`,
# `vocab:dialog=...`, `prefs:vocab-marks=on|off`.

mkdir -p "$ROOT/vocab"
printf 'term;definition;note\nzorbling;a small rolling thing;from ch 2\nreaders;the people who read\n"quoted, term";"a ""tricky"" line"\n' >"$ROOT/vocab/glossary.csv"

scenario vocab_own_term_marked 'read:1,wait:2500,reader:select-text=functionality,wait:500,reader:ui-vocab-add,wait:800,reader:ui-vocab-dialog=,reader:ui-vocab-dialog=def=An invented word for things that just work,reader:ui-vocab-dialog=note=my note,reader:ui-vocab-dialog=save,wait:600,vocab:own-dump,reader:vocab-dump,reader:ui-vocab-click,wait:700,reader:ui-vocab-mark=,reader:ui-vocab-mark=remove,wait:500,reader:vocab-dump,vocab:own-dump,quit' '
    D=$(grep "^vocab_dialog:" <<<"$OUT" | head -1)
    grep -q "open=true title=\"Add to Vocabulary\" term=\"functionality\" def=\"\"" <<<"$D"; check "Add to Vocabulary opens the dialog with the selected word as the term (it is in no dictionary)" $?
    has "^vocab_own_entries: functionality=An invented word for things that just work@Book\(\"1\"\)"; check "the term applies to this book by default" $?
    V=$(grep "^vocab_marks:" <<<"$OUT")
    sed -n 1p <<<"$V" | grep -qE "total=[1-9][0-9]* on_page=[1-9][0-9]* first=Some"; check "the term is marked in the text of the page" $?
    sed -n 1p <<<"$V" | grep -q "texts=\[\"functionality\""; check "the marks are on the word itself" $?
    grep "^vocab_def:" <<<"$OUT" | head -1 | grep -q "visible=false"; check "no lookup yet: the Your definition block is hidden" $?
    has "^vocab_mark: open=true text=\"An invented word for things that just work\""; check "clicking a marked term shows your definition in a popover" $?
    sed -n 2p <<<"$V" | grep -q "total=0 on_page=0 first=None"; check "Remove in the popover takes the definition and the marks away" $?
    grep "^vocab_own_entries:" <<<"$OUT" | tail -1 | grep -q "^vocab_own_entries: $"; check "and nothing of it is left in the vocabulary" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario vocab_own_in_lookup 'read:1,wait:2500,reader:select-text=functionality,wait:500,reader:ui-dict,wait:900,reader:vocab-dump,reader:ui-vocab-def-btn,wait:800,reader:ui-vocab-dialog=def=A word I made up,reader:ui-vocab-dialog=save,wait:600,reader:select-text=functionality,wait:400,reader:ui-dict,wait:900,reader:vocab-dump,reader:ui-vocab-def-btn,wait:800,reader:ui-vocab-dialog=,reader:ui-vocab-dialog=def=An edited meaning,reader:ui-vocab-dialog=save,wait:600,reader:select-text=functionality,wait:400,reader:ui-dict,wait:700,reader:vocab-dump,quit' '
    F=$(grep "^vocab_def:" <<<"$OUT")
    sed -n 1p <<<"$F" | grep -q "visible=true definition=\"\" note=\"\" button=\"Add my own definition\""; check "the look-up popover offers Add my own definition when you have none" $?
    sed -n 2p <<<"$F" | grep -q "definition=\"A word I made up\".*button=\"Edit my definition\""; check "after saving, your definition is shown first, labelled, with Edit my definition" $?
    sed -n 3p <<<"$F" | grep -q "definition=\"An edited meaning\""; check "editing replaces it" $?
    D=$(grep "^vocab_dialog:" <<<"$OUT" | sed -n 2p)
    grep -q "term=\"functionality\" def=\"A word I made up\"" <<<"$D"; check "Edit opens the dialog with the saved definition" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario vocab_own_phrase_and_validation 'read:1,wait:2500,reader:select-text=readers and the world,wait:500,reader:ui-vocab-add,wait:800,reader:ui-vocab-dialog=def=,reader:ui-vocab-dialog=save,reader:ui-vocab-dialog=term=,reader:ui-vocab-dialog=save,reader:ui-vocab-dialog=term=readers and the world,reader:ui-vocab-dialog=def=A phrase kept whole,reader:ui-vocab-dialog=scope=all,reader:ui-vocab-dialog=save,wait:600,vocab:own-dump,reader:vocab-dump,quit' '
    D=$(grep "^vocab_dialog:" <<<"$OUT")
    sed -n 2p <<<"$D" | grep -q "status=\"Write your definition first.\""; check "saving without a definition says so and keeps the dialog" $?
    sed -n 4p <<<"$D" | grep -q "status=\"Type the term first.\""; check "saving without a term says so" $?
    has "^vocab_own_entries: readers and the world=A phrase kept whole@All"; check "a phrase of several words is saved whole, for all books" $?
    grep "^vocab_marks:" <<<"$OUT" | grep -q "texts=\[\"readers and the world\""; check "the phrase is marked as one piece" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario vocab_own_scope 'vocab:own=functionality|only in book two|2,vocab:own=readers|for every book,wait:300,read:1,wait:2500,reader:vocab-dump,quit' '
    V=$(grep "^vocab_marks:" <<<"$OUT" | head -1)
    grep -q "texts=\[\"readers\"" <<<"$V"; check "a term of another book is not marked, a term for all books is" $?
    ! grep -q "functionality" <<<"$V"; check "the other book term is absent here" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario vocab_marks_setting 'vocab:own=functionality|a made up word,read:1,wait:2500,reader:vocab-dump,prefs:,wait:900,prefs:vocab-marks=off,wait:400,reader:vocab-dump,prefs:vocab-marks=on,wait:400,reader:vocab-dump,quit' '
    V=$(grep "^vocab_marks:" <<<"$OUT")
    sed -n 1p <<<"$V" | grep -qE "total=[1-9][0-9]*"; check "marks are on by default" $?
    sed -n 2p <<<"$V" | grep -q "total=0"; check "the setting Mark my vocabulary terms in the text turns them off at once" $?
    sed -n 3p <<<"$V" | grep -qE "total=[1-9][0-9]*"; check "and on again" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario vocab_list_filters 'vocab:own=zorb|a rolling ball,vocab:own=Nautilus|a submarine,vocab:show-words,wait:700,vocab:list=,vocab:list=filter=mine,vocab:list=filter=dictionary,vocab:list=filter=all,vocab:list=search=sub,vocab:list=search=,vocab:list=delete=zorb,vocab:list=add=Ned Land|the harpooner,vocab:own-dump,quit' '
    L=$(grep "^vocab_list:" <<<"$OUT")
    sed -n 1p <<<"$L" | grep -q "rows=2 \[Nautilus\[mine\]=a submarine | zorb\[mine\]=a rolling ball\]"; check "your own words are listed with the Mine badge and their definition" $?
    sed -n 2p <<<"$L" | grep -q "rows=2"; check "the Mine filter shows them" $?
    sed -n 3p <<<"$L" | grep -q "rows=0"; check "From dictionary shows none of them" $?
    sed -n 5p <<<"$L" | grep -q "rows=1 \[Nautilus"; check "the search looks in the definitions" $?
    sed -n 7p <<<"$L" | grep -q "rows=1 \[Nautilus"; check "a word can be deleted from the list" $?
    has "^vocab_own_entries: .*Ned Land=the harpooner@All"; check "Add Term in the list saves a term of your own (for all books)" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario vocab_glossary_io "vocab:own=zorb|old meaning,vocab:show-words,wait:700,vocab:list=paste-open,wait:500,vocab:list=paste-text=alpha; first letter\\nbeta — second letter\\nnodef,vocab:list=paste-add,wait:300,vocab:list=import=$ROOT/vocab/glossary.csv,vocab:list=export=csv:@D@/mine.csv,vocab:list=export=json:@D@/mine.json,vocab:list=export=stardict:@D@/sd,vocab:list=export=anki:@D@/anki.txt,vocab:own-dump,quit" '
    has "^vocab_own_entries: .*alpha=first letter@All"; check "a pasted glossary adds its terms (semicolon form)" $?
    has "beta=second letter@All"; check "(and the dash form)" $?
    ! has "nodef"; check "a line without a definition is skipped" $?
    has "zorbling=a small rolling thing@All"; check "a CSV file term;definition is imported" $?
    has "quoted, term=a \"tricky\" line@All"; check "(with quoted fields)" $?
    has "vocab_list: .*status=\"3 terms added, 0 updated.\""; check "the import reports what it did" $?
    head -1 "$d/mine.csv" | grep -q "^term;definition;note;tags$" && grep -q "^zorb;old meaning" "$d/mine.csv"; check "the CSV export holds the definitions" $?
    python3 -c "import json,sys; e=json.load(open(sys.argv[1]))[\"entries\"]; sys.exit(0 if any(x[\"word\"]==\"zorb\" and x[\"own\"]==\"old meaning\" for x in e) else 1)" "$d/mine.json"; check "the JSON export holds them" $?
    ls "$d"/sd/my-vocabulary.ifo "$d"/sd/my-vocabulary.dict "$d"/sd/my-vocabulary.idx >/dev/null 2>&1; check "the StarDict export writes a dictionary" $?
    grep -q "old meaning" "$d/anki.txt"; check "the Anki export includes the definition" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario vocab_book_terms 'vocab:own=functionality|a made up word|1,vocab:own=other|for another book|2,wait:200,read:1,wait:2500,reader:ui-vocab-terms,wait:700,vocab:list=,quit' '
    has "^vocab_list: rows=1 \[functionality\[mine\]=a made up word\]"; check "Terms in this book lists the terms of the book (and not those of another)" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario vocab_a11y 'vocab:own=functionality|a made up word,read:1,wait:2500,reader:select-text=functionality,wait:500,reader:ui-dict,wait:800,a11y-audit:,reader:ui-vocab-def-btn,wait:800,a11y-audit:,reader:ui-vocab-dialog=save,wait:500,reader:ui-vocab-click,wait:600,a11y-audit:,vocab:show-words,wait:700,a11y-audit:,vocab:list=paste-open,wait:600,a11y-audit:,quit' '
    [ "$(grep -c "^a11y:" <<<"$OUT")" -eq 5 ]; check "the audit ran over the look-up popover, the dialog, the term popover, the list and the paste dialog" $?
    ! has "^a11y-problem:"; check "every control of them has an accessible name" $?'


scenario vocab_palette_and_prefs 'vocab:own=zorb|a rolling ball,read:1,wait:2500,palette:open,wait:600,palette:type=add to vocab,wait:300,palette:dump,palette:type=terms in this,wait:300,palette:dump,palette:close,prefs:,wait:900,prefs:search=My Vocabulary,wait:300,prefs:dump,prefs:search=Mark my vocabulary terms,wait:300,prefs:dump,quit' '
    P=$(grep "^palette: query" <<<"$OUT")
    sed -n 1p <<<"$P" | grep -q "results=Command|Add to Vocabulary"; check "the command palette finds Add to Vocabulary in the reader" $?
    sed -n 2p <<<"$P" | grep -q "results=Command|Terms in this Book"; check "and Terms in this Book" $?
    Q=$(grep "^prefs: query" <<<"$OUT")
    sed -n 1p <<<"$Q" | grep -qE "hits=[1-9]"; check "Preferences has the My Vocabulary row among the dictionaries" $?
    sed -n 2p <<<"$Q" | grep -qE "hits=[1-9]"; check "and the setting to mark the terms in the text" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'
