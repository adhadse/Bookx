# Scenarios of wave 8 (library): the main menu and the Library menu, the selection header and bar,
# notes selection mode, view options, the notes empty state and the search placeholder. Sourced by
# ui-tests.sh (same `scenario NAME 'dev script' 'checks'` format as ui-tests-sync.sh).
#
# Dev hooks used (all `library:<cmd>`):
#   menus         prints `mainmenu: Label [action] | ...`, `librarymenu: ...`, `welcome: ...` and
#                 `menu_counts: main=N library=N add=N main_keys=<labels with access keys, | separated>`
#   selhdr        prints `selhdr: mode=<title|selection> title="..." cancel=<bool> bar=<bool> buttons=<read,edit,shelf,send,convert,more> more=[<overflow labels, | separated>]`
#   cancel-selection   clicks the header Cancel button
#   compact | regular  forces the compact (<= 600 sp) layout of the selection bar on or off
#   viewopts      prints `viewopts: icon=<view button icon> grid_only_visible=<bool> order="<accessible name>"`
#   notes:select-all | notes:clear | notes:selbar | notes:copy-selected | notes:delete-selected
#         notes:selbar prints `notes_bar: selected=N copy=<bool> delete=<bool> export_tip="..." selecting_cards=N`
#   searchph      prints `searchph: placeholder="..."` (the search entry placeholder)

scenario lib8_menus 'library:wide,wait:300,library:menus,quit' '
    has "^mainmenu: Open Library Folder \\[lib.open-folder\\] \\| Sync \\[app.sync\\] \\| Devices \\[app.devices\\] \\| Library Health \\[lib.health\\] \\| Preferences \\[app.preferences\\] \\| Keyboard Shortcuts \\[app.shortcuts\\] \\| Help \\[app.help\\] \\| About Bookx \\[app.about\\]$"; check "the main menu has the eight entries (Saved Searches live in the sidebar) in the decided order, Help before About" $?
    has "^menu_counts: main=8 library=8 add=8 "; check "main menu 8 entries, Library menu 8, + menu 8 (all within 3-12)" $?
    has "^librarymenu: Reading Statistics \\[app.stats\\] \\| Vocabulary \\[app.vocab\\] \\| Reading Trackers \\[lib.trackers\\] \\| Audiobooks \\[lib.audiobooks\\] \\| Online Catalogues \\[lib.opds\\] \\| Subscribe to News \\[lib.news\\] \\| Export as Calibre Library \\[lib.export-calibre\\] \\| Export Citations \\[lib.citations\\]"; check "the Library menu holds the destinations and the library-wide exports" $?
    ! has "^mainmenu: .*(Copy Diagnostics|Import Calibre|Import Highlights|Export All Annotations)"; check "diagnostics, imports and annotation export left the main menu" $?
    has "^menu_counts: .*main_keys=_Open Library Folder\\|"; check "the menu is built as a gio::Menu with access keys" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario lib8_selection_header 'library:wide,wait:300,library:selhdr,library:select1,wait:500,library:selhdr,library:selectall,wait:800,library:selhdr,library:cancel-selection,wait:500,library:selhdr,library:selectall,wait:500,library:compact,wait:300,library:selhdr,library:regular,wait:300,library:selhdr,library:unselect,wait:400,library:selhdr,quit' '
    H=$(grep "^selhdr:" <<<"$OUT")
    sed -n 1p <<<"$H" | grep -q "mode=title title=\"Home\" cancel=false bar=false"; check "nothing selected: the view title, no Cancel, no bar" $?
    sed -n 2p <<<"$H" | grep -q "mode=selection title=\"1 selected\" cancel=true bar=false"; check "one book: the header says 1 selected with a Cancel button" $?
    sed -n 3p <<<"$H" | grep -q "mode=selection title=\"2 selected\" cancel=true bar=true buttons=edit,shelf,send,convert"; check "two books: 2 selected, the bar has Edit Metadata, Add to Shelf, Send to Device, Convert and no Read" $?
    sed -n 4p <<<"$H" | grep -q "mode=title .* cancel=false bar=false"; check "Cancel clears the selection and the title comes back" $?
    sed -n 5p <<<"$H" | grep -q "buttons=more .*more=\[Edit Metadata|Add to Shelf|Send to Device|Convert To\]"; check "in the compact layout every bulk action is in the More menu" $?
    sed -n 6p <<<"$H" | grep -q "buttons=edit,shelf,send,convert"; check "and back to buttons in the regular layout" $?
    sed -n 7p <<<"$H" | grep -q "mode=title"; check "deselecting restores the title" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario lib8_search_placeholder 'library:wide,wait:300,library:searchph,quit' '
    has "^searchph: placeholder=\"Search books\"$"; check "the search placeholder is the short translated \"Search books\"" $?'

scenario lib8_view_options 'library:wide,wait:300,library:viewopts,library:table,wait:500,library:viewopts,library:grid,wait:500,library:viewopts,quit' '
    V=$(grep "^viewopts:" <<<"$OUT")
    sed -n 1p <<<"$V" | grep -q "icon=view-grid-symbolic grid_only_visible=true order=\"Reverse order (now ascending)\""; check "grid mode: grid icon, cover size and grouping shown, the order button says its state" $?
    sed -n 2p <<<"$V" | grep -q "icon=view-list-symbolic grid_only_visible=false"; check "table mode: list icon, the grid-only options are hidden" $?
    sed -n 3p <<<"$V" | grep -q "icon=view-grid-symbolic grid_only_visible=true"; check "back to the grid restores them" $?'

scenario lib8_notes_selection 'library:annotate,library:annotate-more,library:notes,wait:800,library:notes:selbar,library:selhdr,library:notes:select-first,wait:300,library:notes:selbar,library:selhdr,library:notes:select-all,wait:300,library:notes:selbar,library:selhdr,library:notes:clear,wait:300,library:notes:selbar,library:selhdr,quit' '
    B=$(grep "^notes_bar:" <<<"$OUT")
    sed -n 1p <<<"$B" | grep -q "selected=0 copy=false delete=false export_tip=\"Export Everything Shown\" selecting_cards=0"; check "nothing selected: no check boxes at rest, Copy and Delete hidden, Export says it exports everything shown" $?
    sed -n 2p <<<"$B" | grep -q "selected=1 copy=true delete=true export_tip=\"Export 1 Selected\""; check "one note selected: Copy and Delete appear, Export names the count" $?
    sed -n 3p <<<"$B" | grep -qE "selected=[2-9][0-9]* copy=true"; check "Select All (Ctrl+A) selects every shown note" $?
    sed -n 4p <<<"$B" | grep -q "selected=0 copy=false"; check "clearing returns to rest" $?
    H=$(grep "^selhdr:" <<<"$OUT")
    sed -n 2p <<<"$H" | grep -q "mode=selection title=\"1 selected\" cancel=true"; check "the library header shows 1 selected with Cancel while notes are selected" $?
    sed -n 4p <<<"$H" | grep -q "mode=title"; check "the header title comes back when the notes selection is cleared" $?'

scenario lib8_notes_empty 'library:notes,wait:800,library:notes:search=nothing-like-this,wait:400,library:notes:dump,quit' '
    has "^notes_state: total=0 shown=0"; check "a library without highlights shows the empty notes page (with Open Library and Import Highlights buttons)" $?'

# ---- wave 9 (library): Move to Trash with Undo, the jobs panel, the table, cards, details toggle ----
# Dev hooks added in wave 9 (all `library:<cmd>`, printed as `<cmd>: ...`):
#   undo-last        presses the Undo button of the last undo toast (`toast: <text> [undo]` is printed when it shows)
#   tablecols        `tablecols: width=N visible=a,b hscroll=<bool> custom=<bool>`
#   sortcol=Name[:desc]   a click on that column header; sortstate prints key/dropdown/arrow/first rows
#   hover=N | unhover=N | checks | check=N | longpress=N | cardclick=N | carddesc=N   (cards)
#   details-click | details-state | gridwidth | gridcap=N (lowers the grid cap, test only)
# and `jobs:copy-error=ID`, `jobs:icon`; `jobs:rows` says `rows empty=No Background Jobs` when nothing is listed.

scenario lib9_trash_undo 'library:wide,wait:300,library:dump,library:select1,wait:400,library:delete,wait:900,library:dump,library:undo-last,wait:900,library:dump,quit' '
    T=$(grep "^title:" <<<"$OUT")
    sed -n 1p <<<"$T" | grep -q "2 books"; check "the library starts with two books" $?
    has "^toast: 1 book moved to Trash \\[undo\\]"; check "removing a book shows a toast with Undo and no confirmation" $?
    sed -n 2p <<<"$T" | grep -q "1 book"; check "the book left the library" $?
    sed -n 3p <<<"$T" | grep -q "2 books"; check "Undo brings it back from the Trash" $?
    ! has "Remove Book"; check "no confirmation dialog" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario lib9_jobs 'jobs:clear,jobs:open,wait:500,jobs:rows,jobs:close,jobs:start=send|Send Books|no,jobs:finish=2|disk full,jobs:open,wait:500,jobs:rows,jobs:copy-error=2,jobs:icon,jobs:close,quit' '
    R=$(grep "^jobs: rows" <<<"$OUT")
    sed -n 1p <<<"$R" | grep -q "rows empty=No Background Jobs"; check "no jobs: the panel says No Background Jobs" $?
    sed -n 2p <<<"$R" | grep -q "Send Books / Failed: disk full"; check "a failed job lists its reason" $?
    ! sed -n 2p <<<"$R" | grep -q "After 0"; check "no \"After 0 ms\" for a job that failed at once" $?
    has "^jobs: copy-error 2 Send Books / disk full"; check "Copy Error puts the job and its reason on the clipboard" $?
    has "^jobs: icon (blocks|system-run)-symbolic"; check "the Jobs button has its own icon (not the list icon of Shelves and the table)" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario lib9_table_sort 'library:wide,wait:300,library:table,wait:700,library:sortstate,library:sortcol=Author(s):desc,wait:600,library:sortstate,library:sortcol=Author(s),wait:600,library:sortstate,library:sort=2,wait:600,library:sortstate,library:sort=3,wait:600,library:sortstate,quit' '
    S=$(grep "^sortstate:" <<<"$OUT")
    sed -n 1p <<<"$S" | grep -q "key=Title asc=true dropdown=Title arrow=Title arrow_asc=true"; check "the table header shows the sort of the menu (Title, ascending)" $?
    sed -n 2p <<<"$S" | grep -q "key=Author asc=false dropdown=Author arrow=Author(s) arrow_asc=false"; check "a click on Author(s) descending: the menu says Author, reverse order" $?
    sed -n 3p <<<"$S" | grep -q "key=Author asc=true dropdown=Author arrow=Author(s) arrow_asc=true"; check "and ascending again" $?
    sed -n 4p <<<"$S" | grep -q "key=Date added asc=true dropdown=Date added arrow=Added"; check "choosing Date added in the menu moves the arrow to the Added column" $?
    sed -n 5p <<<"$S" | grep -q "key=Last read asc=true dropdown=Last read arrow=none"; check "a key without a column (Last read) clears the arrow" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario lib9_table_columns_900x600 'library:table,wait:900,library:tablecols,quit' '
    C=$(grep "^tablecols:" <<<"$OUT")
    grep -q "hscroll=false" <<<"$C"; check "at 900x600 the table has no sideways scrolling" $?
    ! grep -q "Published" <<<"$C"; check "at 900x600 the low-value columns (Published) are hidden" $?
    grep -q "visible=.*Title.*Author(s)" <<<"$C"; check "the Title and Author(s) columns stay" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario lib9_cards 'library:wide,wait:500,library:checks,library:hover=0,wait:200,library:checks,library:check=0,wait:500,library:checks,library:cardclick=1,wait:500,library:checks,library:selhdr,library:unhover=0,wait:200,library:checks,library:cancel-selection,wait:500,library:checks,library:longpress=1,wait:500,library:checks,library:carddesc=0,library:carddesc=1,quit' '
    K=$(grep "^checks:" <<<"$OUT")
    sed -n 1p <<<"$K" | grep -q "visible=0 active=0 selecting=false selected=0"; check "at rest no card shows a check box" $?
    sed -n 2p <<<"$K" | grep -q "visible=1 active=0"; check "hovering a card shows its check box (and only that one)" $?
    sed -n 3p <<<"$K" | grep -q "active=1 selecting=true selected=1"; check "ticking the check box selects the book and starts selecting" $?
    sed -n 4p <<<"$K" | grep -q "active=2 selecting=true selected=2"; check "while selecting a plain click on another card toggles it in" $?
    grep "^selhdr:" <<<"$OUT" | grep -q "title=\"2 selected\""; check "the header counts the two" $?
    sed -n 5p <<<"$K" | grep -q "visible=2 active=2"; check "selected cards keep their check box when the pointer leaves" $?
    sed -n 6p <<<"$K" | grep -q "visible=0 active=0 selecting=false selected=0"; check "Cancel clears the selection and the check boxes go" $?
    sed -n 7p <<<"$K" | grep -q "active=1 selecting=true selected=1"; check "a long press picks the card and starts selecting (touch)" $?
    has "^carddesc: pos=0 title=\"Second Book\" desc=\"Other Author\""; check "a card describes its author for assistive technology" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario lib9_details_toggle 'library:wide,wait:500,library:details-state,library:details-click,wait:300,library:details-state,library:select1,wait:500,library:details-state,library:details-click,wait:300,library:details-state,library:unselect,wait:300,library:select1,wait:500,library:details-state,quit' '
    D=$(grep "^details:" <<<"$OUT")
    sed -n 1p <<<"$D" | grep -q "shown=false toggle=false pref=true toggle_visible=true icon=sidebar-show-right-symbolic"; check "the Details toggle is in the header at a wide window, off while nothing is selected" $?
    sed -n 2p <<<"$D" | grep -q "shown=true toggle=true"; check "clicking it opens the panel" $?
    sed -n 3p <<<"$D" | grep -q "shown=true toggle=true"; check "selecting a book keeps it open" $?
    sed -n 4p <<<"$D" | grep -q "shown=false toggle=false pref=false"; check "clicking again closes it and remembers" $?
    sed -n 5p <<<"$D" | grep -q "shown=false toggle=false pref=false"; check "a later selection does not reopen a panel the user closed" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario lib9_group_authors 'library:wide,wait:600,library:group-expand=Authors,wait:300,library:group=Authors,library:group-showall=Authors,wait:300,quit' '
    has "^group: Authors rows=8 show_all=true expanded=true"; check "30 authors: 8 rows and a Show All row" $?
    has "^showall: listed=32"; check "Show All lists all 32 authors (30 and the two of the test library)" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario lib9_group_remember 'library:wide,wait:600,library:group-expand=Authors,wait:300,library:group=Authors,quit' '
    ok=0
    python3 -c "import json,sys; c=json.load(open(sys.argv[1])); sys.exit(0 if any(s.endswith(\"|Authors\") for s in c.get(\"sidebar_open\",[])) else 1)" "$d/cfg/bookx/config.json" 2>/dev/null && ok=1
    [ "$ok" = 1 ]; check "an opened sidebar group is remembered for the library (config sidebar_open)" $?'

scenario lib9_saved_and_tips 'library:wide,wait:500,library:saved-state,library:tips-open,wait:300,library:tips,library:tips-close,library:search=author:Test,wait:500,library:tips-open,wait:300,library:tips,library:tips-save=My Authors,wait:500,library:saved-state,quit' '
    T=$(grep "^tips:" <<<"$OUT")
    sed -n 1p <<<"$T" | grep -q "button=true save_sensitive=Some(false)"; check "Search Tips has a Save This Search button, off while the search is empty" $?
    sed -n 2p <<<"$T" | grep -q "save_sensitive=Some(true) name=Some(\"author:Test\")"; check "with a search typed it is on and the name starts as the search" $?
    S=$(grep "^saved:" <<<"$OUT")
    sed -n 1p <<<"$S" | grep -q "saved: rows=1 titles=No saved searches yet"; check "the sidebar group Saved Searches is always there, with a hint while empty" $?
    sed -n 2p <<<"$S" | grep -q "titles=My Authors"; check "a saved search appears in the sidebar at once" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario lib9_shelves 'library:wide,wait:500,library:shelf-empty,library:shelf-empty-click,wait:400,library:dialog,library:dialog-close,library:shelf=create=Alpha,library:shelf=add=Alpha|1,wait:300,library:shelf=delete-undo=Alpha,wait:300,library:shelf=dump,library:undo-last,wait:500,library:shelf=dump,library:shelf-empty,quit' '
    has "^shelf_empty: row=true activatable=true title=Some\\(\"New Shelf\"\\)"; check "with no shelf the group offers a clickable New Shelf row" $?
    has "^dialog: name=\"[A-Za-z]*\" heading=\"New Shelf\""; check "clicking it opens the New Shelf dialog" $?
    has "^shelves: delete-undo shelves 1 -> 0"; check "deleting a shelf needs no confirmation" $?
    has "^toast: Shelf “Alpha” deleted \\[undo\\]"; check "a toast with Undo says so" $?
    D=$(grep "^shelves: list" <<<"$OUT")
    sed -n 1p <<<"$D" | grep -q "list $"; check "the shelf is gone after Delete" $?
    sed -n 2p <<<"$D" | grep -q "Alpha=1:1"; check "Undo brings the shelf back with its book" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario lib9_paste_drop 'library:wide,wait:500,library:paste=hello world,wait:300,library:webdialog,library:paste=https://example.com/article,wait:400,library:webdialog,library:dialog-close,wait:200,library:drophint,library:drag-enter,library:drophint,library:drag-leave,library:drophint,quit' '
    W=$(grep "^webdialog:" <<<"$OUT")
    sed -n 1p <<<"$W" | grep -q "open=false"; check "pasting a sentence does nothing" $?
    sed -n 2p <<<"$W" | grep -q "open=true url=\"https://example.com/article\""; check "pasting an address opens Add from the Web with it filled in" $?
    P=$(grep "^drophint:" <<<"$OUT")
    sed -n 1p <<<"$P" | grep -q "visible=false targetable=false"; check "no drop hint at rest, and it never takes the pointer" $?
    sed -n 2p <<<"$P" | grep -q "visible=true targetable=false text=\"Drop to Add\""; check "dragging files over the books shows Drop to Add" $?
    sed -n 3p <<<"$P" | grep -q "visible=false"; check "leaving hides it" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario lib9_book_menu 'library:ctxmenu,quit' '
    has "^ctxmenu_shape: single_top=([0-9]|1[0-2]) multi_top=([0-9]|1[0-2]) depth=1 "; check "the book menu has at most 12 entries at the first level, single or several books, and no submenu inside a submenu" $?
    has "^ctxmenu_shape: .*Send To=[1-6][, ]"; check "the Send To submenu is short" $?
    has "^ctxmenu: .*Copy Citation"; check "the book menu has Copy Citation" $?
    ! has "^ctxmenu: .*Readwise"; check "no Readwise body among the annotation formats of the book menu" $?'

scenario lib9_citation 'library:wide,wait:400,library:select1,wait:400,library:copy-citation,wait:300,quit' '
    has "^citation: .*Second Book|^citation: .*UI Test Book"; check "Copy Citation renders the selected book as APA text" $?
    has "^toast|^citation:"; check "and says so" $?'

scenario works_row_activate 'library:works=on,wait:600,library:works=show:1,wait:600,library:works=activate:first,wait:800,quit' '
    has "^works: row edition-[0-9]+ activatable=true"; check "an edition row is activatable (Enter or a click opens the edition)" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario lib9_grid_clamp 'wait:900,library:gridwidth,quit' '
    G=$(grep "^gridwidth:" <<<"$OUT")
    grep -q "max=1800" <<<"$G"; check "the grid is capped at 1800 px" $?
    W=$(sed "s/.*clamp=\([0-9]*\).*/\1/" <<<"$G")
    [ "$W" -le 1800 ]; check "the grid is at most 1800 px wide ($W)" $?'

scenario lib9_notes_menus_900x600 'library:annotate,library:annotate-more,library:notes,wait:800,library:notes:export-menu,library:notes:anki-id=apkg:cloze:vocab,library:notes:anki-id=csv:basic,library:notes:anki-id=tsv:basic:vocab,library:notes:anki-id=pdf,library:notes:anki-dialog=apkg,wait:400,library:dialog,library:dialog-close,wait:700,library:notes:toolbar,quit' '
    M=$(grep "^notes_menu:" <<<"$OUT")
    grep -q "anki=3" <<<"$M"; check "Anki is three entries (Package, Text File, CSV)" $?
    T=$(sed "s/.*top=\([0-9]*\).*/\1/" <<<"$M")
    [ "$T" -le 12 ]; check "the Export menu has at most 12 entries ($T)" $?
    ! grep -q "Readwise (JSON" <<<"$M"; check "the Readwise body is not a file entry" $?
    has "^notes_anki: id=Some\\(\"vocab-apkg-cloze\"\\)"; check "options map to the export id (vocabulary, cloze, package)" $?
    has "^notes_anki: id=Some\\(\"anki-csv-basic\"\\)"; check "highlights, front/back, CSV" $?
    has "^notes_anki: id=Some\\(\"vocab-tsv-basic\"\\)"; check "vocabulary, text file" $?
    has "^notes_anki: id=None"; check "an unknown file type maps to nothing" $?
    has "^dialog: name=\"anki-dialog\" heading=\"Export Anki Flashcards\""; check "an Anki entry opens the options dialog" $?
    B=$(grep "^notes_toolbar:" <<<"$OUT")
    MIN=$(sed "s/.*min=\([0-9]*\).*/\1/" <<<"$B"); WID=$(sed "s/.* width=\([0-9]*\).*/\1/" <<<"$B")
    [ "$MIN" -le "$WID" ]; check "the notes toolbar fits one row at 900x600 (needs $MIN of $WID px)" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario lib9_notes_edit_dialog 'library:annotate,library:annotate-more,library:notes,wait:800,library:notes:search=bluequote,wait:400,library:notes:edit-dialog,wait:500,library:dialog,library:notes:edit-keys,library:notes:edit-type=zzz,library:notes:edit-cancel,wait:300,library:notes:dump,library:notes:edit-dialog,wait:500,library:notes:edit-type=typed in the dialog,library:notes:edit-save,wait:500,library:dialog,library:notes:dump,quit' '
    has "^dialog: .*heading=\"Edit Note\""; check "Edit Note is a dialog with a header" $?
    has "^notes_editdialog_keys: .*<Control>Return"; check "Ctrl+Enter is the save shortcut" $?
    D=$(grep "^notes_state:" <<<"$OUT")
    ! sed -n 1p <<<"$D" | grep -q "zzz"; check "Cancel leaves the note alone" $?
    has "^notes_edit: ok=true note=\"typed in the dialog\""; check "Save stores the text" $?
    sed -n 2p <<<"$D" | grep -q "typed in the dialog"; check "and the list shows it" $?'

scenario notes_readwise_job 'library:annotate,library:annotate-more,library:notes,wait:800,library:notes:token=rw-secret-token,library:notes:readwise,wait:300,jobs:dump,wait:1500,jobs:dump,library:dialog,quit' '
    J=$(grep "^jobs: running" <<<"$OUT")
    grep -q "Send to Readwise\|Send Highlights to Readwise" <<<"$J"; check "sending to Readwise is a job in the Jobs panel" $?
    has "^readwise: Sent 4 highlights to Readwise"; check "it sends the highlights" $?
    sed -n 2p <<<"$J" | grep -q "Done"; check "the job ends as Done" $?
    has "^dialog: none"; check "no progress dialog" $?'

scenario lib9_import_dialog 'library:wide,wait:400,library:import-dialog,wait:300,library:dialog-close,quit' '
    has "^import_dialog: body=\"Choose files, or a folder or e-reader, with notes"; check "a one-sentence intro that says a folder may be an e-reader" $?
    has "files=\"Choose Files\" folder=\"Choose Folder\" default=Some\\(\"files\"\\)"; check "the responses are Choose Files and Choose Folder" $?'

scenario lib9_focus 'library:wide,wait:500,library:focus-order,library:focus-grid,wait:200,library:add-web-open,wait:500,library:dialog-close,wait:600,library:focus-where,quit' '
    O=$(grep "^focus_order:" <<<"$OUT")
    echo "$O" | grep -q "ListBox\|Sidebar"; check "Tab starts in the sidebar" $?
    echo "$O" | grep -qi "search"; check "and reaches the search button" $?
    has "^focus_where: in_grid=true"; check "after a dialog closes the focus is back in the book grid" $?'

# grid background and the click on empty space (the capped grid must not shrink the click target nor paint a darker strip)
#   library:gridbg                 `gridbg: scroll=[classes] view=[classes] page=[classes]`
#   library:click-empty=margin|below|inside   `click-empty: <where> reaches_list=<bool> clears=<bool>`
scenario lib9_grid_empty_click 'library:wide,wait:1500,library:gridcap=500,wait:1800,library:gridwidth,library:gridbg,library:select1,wait:400,library:click-empty=margin,wait:400,library:works=selected,library:select1,wait:400,library:click-empty=below,wait:400,library:works=selected,library:select1,wait:1200,library:click-empty=inside,wait:400,library:works=selected,quit' '
    G=$(grep "^gridwidth:" <<<"$OUT")
    grep -q "max=500" <<<"$G"; check "the test lowered the cap to 500 px" $?
    W=$(sed "s/.*clamp=\([0-9]*\).*/\1/" <<<"$G")
    [ "$W" -le 500 ]; check "the grid view is at most the cap wide ($W)" $?
    S=$(sed "s/.*scroll=\([0-9]*\).*/\1/" <<<"$G")
    [ "$S" -gt 500 ]; check "the scrolled window keeps the whole width ($S)" $?
    grep "^gridbg:" <<<"$OUT" | grep -q "scroll=\[[^]]*library-grid.*view=\[[^]]*library-grid"; check "the scroll and the grid carry library-grid (transparent: the window background, no darker strip)" $?
    has "^click-empty: margin reaches_list=true clears=true"; check "a click on the margin beside the capped grid reaches the list handler and clears" $?
    has "^click-empty: below reaches_list=true clears=true"; check "a click below the last row clears" $?
    has "^click-empty: inside reaches_list=true clears=false"; check "a click on a card does not clear" $?
    M=$(grep "^works: selected=" <<<"$OUT")
    sed -n 1p <<<"$M" | grep -q "selected=\[\]"; check "the selection is empty after the margin click" $?
    sed -n 2p <<<"$M" | grep -q "selected=\[\]"; check "the selection is empty after the click below" $?
    sed -n 3p <<<"$M" | grep -vq "selected=\[\]"; check "the selection is kept after a click on a card" $?'

# the Jobs button of the header: nothing animates while no job runs (the theme has no emblem-synchronizing-symbolic and
# GTK's process-working-symbolic is a spinner that never stops), the ring shows only while a job runs
#   jobs:button   `jobs: jobs_button: spinning=<bool> icon=<name>`
scenario lib9_jobs_button_idle 'wait:600,jobs:button,jobs:start=convert|Convert Book|cancel,wait:300,jobs:button,jobs:finish=2|ok,wait:300,jobs:button,quit' '
    B=$(grep "jobs_button:" <<<"$OUT")
    sed -n 1p <<<"$B" | grep -q "spinning=false"; check "idle: the Jobs button does not spin" $?
    sed -n 1p <<<"$B" | grep -qv "process-working"; check "idle: the icon is a static one" $?
    sed -n 2p <<<"$B" | grep -q "spinning=true"; check "a running job shows the spinner" $?
    sed -n 3p <<<"$B" | grep -q "spinning=false"; check "after the job finished the button stops spinning" $?
    sed -n 3p <<<"$B" | grep -qv "process-working"; check "and shows a static icon again" $?'

# wave 10: Online Catalogues is a destination of the sidebar (a page in the main view, not a dialog) and the
# sidebar is configurable (Customize Sidebar: show, hide, reorder; kept per library).
#   library:opds=open|dump|catalog=NAME|back ...   the catalogues page (`opds_view: ...`)
#   library:sidebar-layout                           `sidebar_layout: shown=<keys> hidden=<keys> health=<bool>`
#   library:sidebar-config=open|dump|hide=KEY|show=KEY|up=KEY|down=KEY|reset|close
#                                                    presses the real controls of the Customize Sidebar dialog
#   library:browse=KEY                               the palette's Browse <group> dialog
scenario lib10_catalogues_page 'library:wide,wait:400,library:sidebar,library:nav=catalogues,wait:600,library:sidebar,library:opds=dump,library:opds=add-catalog=Mock|https://mock.example/opds/root.xml|u|p,library:opds=catalog=Mock,wait:1200,library:opds=dump,library:opds=back,wait:400,library:opds=dump,library:nav=home,wait:300,library:sidebar,quit' '
    S=$(grep "^sidebar:" <<<"$OUT")
    sed -n 1p <<<"$S" | grep -q "view=home rows=nav-home,nav-recent,nav-favourites,nav-notes,nav-catalogues"; check "Online Catalogues is a row of the sidebar, after Notes & Highlights" $?
    sed -n 2p <<<"$S" | grep -q "view=catalogues"; check "picking it shows the catalogues in the main view" $?
    O=$(grep "^opds_view:" <<<"$OUT")
    sed -n 1p <<<"$O" | grep -q "title=Online Catalogues | page=list .*back=false"; check "the page opens on the list of catalogues without Back" $?
    sed -n 2p <<<"$O" | grep -q "back=true"; check "inside a catalogue the page has its own Back" $?
    sed -n 3p <<<"$O" | grep -q "title=Online Catalogues | page=list .*back=false"; check "Back returns to the list" $?
    sed -n 3p <<<"$S" | grep -q "view=home"; check "Home is shown again afterwards" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario lib10_sidebar_customize 'library:wide,wait:400,library:sidebar-layout,library:sidebar-config=open,wait:1500,library:sidebar-config=dump,library:sidebar-config=up=catalogues,wait:300,library:sidebar,library:sidebar-layout,library:sidebar-config=hide=notes,wait:300,library:sidebar,library:sidebar-config=hide=health,wait:300,library:sidebar-layout,library:sidebar-config=dump,library:browse=tags,wait:400,quit' '
    L=$(grep "^sidebar_layout:" <<<"$OUT")
    sed -n 1p <<<"$L" | grep -q "shown=home,recent,favourites,notes,catalogues,shelves,audiobooks,saved-searches,authors,series,tags,publishers,languages,ratings,status hidden= health=true"; check "the default sidebar: every entry, today order, Online Catalogues after Notes" $?
    grep -q "^sidebar_config: rows=home,recent,favourites,notes,catalogues,.*,status,health hidden=$" <<<"$OUT"; check "Preferences, Library, Sidebar lists every entry with Library Health last" $?
    S=$(grep "^sidebar:" <<<"$OUT")
    sed -n 1p <<<"$S" | grep -q "rows=nav-home,nav-recent,nav-favourites,nav-catalogues,nav-notes "; check "moving Online Catalogues up puts it before Notes & Highlights" $?
    sed -n 2p <<<"$S" | grep -q "rows=nav-home,nav-recent,nav-favourites,nav-catalogues counts=2,2,0,2 "; check "hiding Notes & Highlights removes its row at once" $?
    sed -n 2p <<<"$L" | grep -q "shown=home,recent,favourites,catalogues,notes,shelves,.* hidden= health=true"; check "the layout keeps the new order" $?
    sed -n 3p <<<"$L" | grep -q "shown=home,recent,favourites,catalogues,shelves,.* hidden=health,notes health=false"; check "the layout keeps what was hidden; Library Health can be hidden too (it is pinned, not moved)" $?
    ok=0
    python3 -c "
import json,sys
c=json.load(open(sys.argv[1]))[\"sidebar_layouts\"]
l=list(c.values())[0]
sys.exit(0 if \"notes\" in l[\"hidden\"] and \"health\" in l[\"hidden\"] and l[\"order\"][2:5]==[\"favourites\",\"catalogues\",\"notes\"] else 1)" "$d/cfg/bookx/config.json" 2>/dev/null && ok=1
    [ "$ok" = 1 ]; check "the layout is kept per library in the settings" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings (the Browse Tags dialog of the palette opens)" $?'

scenario lib10_sidebar_hidden_view_and_reset 'library:wide,wait:400,library:nav=notes,wait:600,library:sidebar-config=open,wait:1500,library:sidebar-config=hide=notes,wait:400,library:sidebar,library:sidebar-config=hide=shelves,library:sidebar-config=hide=authors,wait:300,library:sidebar-layout,library:sidebar-config=reset,wait:300,library:sidebar-layout,library:sidebar,quit' '
    S=$(grep "^sidebar:" <<<"$OUT")
    sed -n 1p <<<"$S" | grep -q "view=home rows=nav-home,nav-recent,nav-favourites,nav-catalogues"; check "hiding the view that is shown falls back to Home" $?
    L=$(grep "^sidebar_layout:" <<<"$OUT")
    sed -n 1p <<<"$L" | grep -q "hidden=authors,notes,shelves"; check "groups hide as well (Shelves, Authors)" $?
    sed -n 2p <<<"$L" | grep -q "hidden= health=true"; check "Reset to Default shows everything again" $?
    sed -n 2p <<<"$S" | grep -q "rows=nav-home,nav-recent,nav-favourites,nav-notes,nav-catalogues"; check "and the rows are back" $?
    python3 -c "
import json,sys
c=json.load(open(sys.argv[1])).get(\"sidebar_layouts\",{})
sys.exit(0 if not c else 1)" "$d/cfg/bookx/config.json"; check "nothing is stored for the default layout" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# wave 10, XC-6: the standard keys of the library (F9 sidebar, Ctrl+A, Delete with Undo, Ctrl+comma, Ctrl+question, F1).
#   library:key=ACCEL   `lib_key: ACCEL -> action` (the action the key is bound to; library and book actions are run)
scenario lib10_standard_keys 'library:wide,wait:400,library:sidebar,library:key=F9,wait:400,library:sidebar,library:key=F9,wait:400,library:sidebar,library:key=<Control>a,wait:400,library:selection,library:key=Delete,wait:800,library:visible,library:key=<Control>comma,library:key=<Control>question,library:key=F1,library:key=<Shift>F9,quit' '
    K=$(grep "^lib_key:" <<<"$OUT"); S=$(grep "^sidebar:" <<<"$OUT")
    sed -n 1p <<<"$K" | grep -q "F9 -> lib.toggle-sidebar"; check "F9 is bound to showing and hiding the sidebar" $?
    sed -n 1p <<<"$S" | grep -q "panel=true"; check "the sidebar is shown to begin with" $?
    sed -n 2p <<<"$S" | grep -q "panel=false"; check "F9 hides the sidebar" $?
    sed -n 3p <<<"$S" | grep -q "panel=true"; check "F9 shows it again" $?
    grep -q "lib_key: <Control>a -> lib.select-all" <<<"$OUT"; check "Ctrl+A selects all books" $?
    grep "^selection:" <<<"$OUT" | head -1 | grep -q "model=2"; check "both books are selected" $?
    grep -q "lib_key: Delete -> book.delete" <<<"$OUT"; check "Delete moves the selection to Trash" $?
    grep "^visible:" <<<"$OUT" | head -1 | grep -q "^visible: 0"; check "the books left the library view (the Undo toast of Move to Trash is tested in the lib8 scenarios)" $?
    grep -q "lib_key: <Control>comma -> app.preferences" <<<"$OUT"; check "Ctrl+, is Preferences" $?
    grep -q "lib_key: <Control>question -> app.shortcuts" <<<"$OUT"; check "Ctrl+? is Keyboard Shortcuts" $?
    grep -q "lib_key: F1 -> app.help" <<<"$OUT"; check "F1 is Help" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# wave 10, WIN-L8: Import Calibre Library from the + menu is a background job (Jobs panel) with a result toast; no
# Library Health window opens.   library:calibre-import=DIR   (the chooser skipped)   prints `calibre_import: imported=N ...`
scenario lib10_calibre_import 'library:wide,wait:400,jobs:dump,library:calibre-import=@D@/scalecal,wait:300,jobs:dump,wait:3000,jobs:dump,library:visible,library:menus,quit' '
    has "^calibre_import: imported=5 skipped=0 duplicates=0 copy_failed=5 reload=true"; check "the five Calibre books were imported (the fixture has no files, so they stay linked)" $?
    J=$(grep "^jobs: " <<<"$OUT")
    sed -n 1p <<<"$J" | grep -q "running=0"; check "no job runs before" $?
    sed -n 3p <<<"$J" | grep -q "Import|Done|1.00|Importing the Calibre library .*cancellable=false|log=1"; check "the import is a job of the Jobs panel, finished with its log" $?
    grep "^visible:" <<<"$OUT" | tail -1 | grep -q "visible: 7"; check "the library lists the two books and the five imported ones" $?
    has "^mainmenu: .*Library Health \\[lib.health\\]"; check "Library Health stays a menu entry of its own" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# wave 10, XC-1: removing a catalogue of the Online Catalogues page is undoable (toast with Undo, no confirmation)
#   library:opds=catalogs  `opds_catalogs: A|B`   library:opds=remove=NAME   library:opds=undo-remove (presses the toast's Undo)
scenario lib10_catalogue_remove_undo 'library:wide,wait:300,library:opds=open,wait:500,library:opds=add-catalog=Mock|https://mock.example/opds/root.xml|u|p,library:opds=add-catalog=Other|https://mock.example/opds/other.xml|,library:opds=catalogs,library:opds=remove=Mock,wait:300,library:opds=catalogs,library:opds=undo-remove,wait:300,library:opds=catalogs,quit' '
    C=$(grep "^opds_catalogs:" <<<"$OUT")
    sed -n 1p <<<"$C" | grep -q "^opds_catalogs: Mock|Other$"; check "two catalogues of the user" $?
    sed -n 2p <<<"$C" | grep -q "^opds_catalogs: Other$"; check "removing one takes it out at once, without a confirmation" $?
    sed -n 3p <<<"$C" | grep -q "^opds_catalogs: Mock|Other$"; check "Undo puts it back in its place" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|opds_error"; check "no errors or GTK warnings" $?'

# ---- wave 11 (HIG): LIB-L6 the library's selection pattern in the Needs Attention list --------------------------------
# Dev commands: `library:organize=attention-tick=N` (ticks the N-th row), `attention-cancel`, `attention-state` (`attention_sel:
# selected=N description=.. cancel=B fetch=.. visible_checks=V checks=T`).

scenario lib11_attention_select 'library:organize=attention,wait:800,library:organize=attention-state,library:organize=attention-tick=1,library:organize=attention-state,library:organize=attention-cancel,library:organize=attention-state,quit' '
    S=$(grep "^attention_sel:" <<<"$OUT")
    sed -n 1p <<<"$S" | grep -q "selected=0 .*cancel=false fetch=\"Fetch Metadata\" visible_checks=0"; check "at rest no check box shows, Cancel is hidden and Fetch Metadata takes every book" $?
    sed -n 2p <<<"$S" | grep -q "selected=1 description=\"1 selected\" cancel=true fetch=\"Fetch Metadata for 1 Book\" visible_checks=[1-9]"; check "ticking a book says 1 selected, shows Cancel and Fetch Metadata for 1 Book" $?
    sed -n 3p <<<"$S" | grep -q "selected=0 .*cancel=false fetch=\"Fetch Metadata\" visible_checks=0"; check "Cancel clears the selection" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# Conversion Options (C20) and "Convert N at a time" (C10): the dialog of the Convert To menu
scenario convert_options 'convert:open,wait:600,convert:format=epub,convert:smarten=on,convert:format=mobi,convert:format=epub,convert:parallel=3,convert:format=epub,convert:reset,convert:close,wait:300,quit' '
    C=$(grep "^convert_options:" <<<"$OUT")
    sed -n 1p <<<"$C" | grep -q "format=epub .*smarten=false .*saved=false version_visible=true toc_visible=false"; check "the dialog opens on EPUB, which has a version row and no contents-page row" $?
    sed -n 2p <<<"$C" | grep -q "smarten=true .*saved=true"; check "turning an option on saves it for the format" $?
    sed -n 3p <<<"$C" | grep -q "format=mobi .*smarten=true .*unused_css=true .*saved=false version_visible=false toc_visible=true"; check "MOBI starts from its Kindle preset (none saved), with a contents-page row and no EPUB version" $?
    sed -n 4p <<<"$C" | grep -q "format=epub .*smarten=true"; check "coming back to EPUB shows what was saved" $?
    sed -n 5p <<<"$C" | grep -q "parallel=3"; check "Convert at a time can be set" $?
    grep -q "\"parallel\": 3" "$d/cfg/bookx/convert.json"; check "and is saved" $?
    sed -n 7p <<<"$C" | grep -q "smarten=false .*saved=false"; check "Use the Built-in Options forgets the changes of the format" $?
    ! grep -q "smarten_punctuation" "$d/cfg/bookx/convert.json"; check "the format is gone from the file" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# ---- wave 11 (T3): custom columns, search and replace, records without a file, marks -----------------------------
# Dev commands: `library:customcol=add|Name|type|arg` (types: text, integer, enumeration (arg: a/b), composite (arg: template)...),
# `customcol=set|ID|label|value`, `customcol=undo`, `customcol=remove|label`, `library:customtable=label:on|off` (tick the
# column of the table), `library:customsort=label:asc|desc` (a click on its header), `library:customcols` (`customcols:
# defined=N table=[label:visible] sort=..`), `library:record=empty|Title|Authors` / `record=isbn|ISBN`,
# `library:replace=field|find|with[|regex]`, `library:marked=1,2`, `library:clearmarks`.
scenario lib12_custom_columns 'library:wide,wait:300,library:customcol=add|Pages|integer,library:customcol=add|Mood|enumeration|Calm/Tense,library:customcol=set|1|pages|412,library:customcol=set|2|pages|30,library:customcol=set|1|pages|lots,library:customcol=set|2|mood|tense,library:customcol=set|2|mood|sleepy,library:search=#pages:>100,wait:500,library:visible,library:search=#mood:tense,wait:500,library:visible,library:search=,wait:300,library:table,wait:500,library:customtable=pages:on,wait:500,library:customcols,library:customsort=pages:desc,wait:400,library:customcols,library:customcol=undo,library:customcol=remove|pages,wait:300,library:customcols,quit' '
    C=$(grep "^customcol:" <<<"$OUT")
    sed -n 1p <<<"$C" | grep -q "added pages"; check "a column is added" $?
    sed -n 2p <<<"$C" | grep -q "added mood"; check "an enumeration column is added" $?
    sed -n 3p <<<"$C" | grep -q "set pages=412"; check "a value is stored" $?
    sed -n 5p <<<"$C" | grep -q "error .*whole number"; check "a value that does not fit is refused with the reason" $?
    sed -n 7p <<<"$C" | grep -q "error .*not one of"; check "a value outside the list is refused" $?
    V=$(grep "^visible:" <<<"$OUT")
    sed -n 1p <<<"$V" | grep -q "visible: 1"; check "#pages:>100 finds the one book with 412 pages" $?
    sed -n 2p <<<"$V" | grep -q "visible: 1"; check "#mood:tense finds a book" $?
    K=$(grep "^customcols:" <<<"$OUT")
    sed -n 1p <<<"$K" | grep -q "defined=2 table=\[pages:true\]"; check "the Pages column is in the table" $?
    sed -n 3p <<<"$K" | grep -q "sort=Some(\"pages:desc\")"; check "a click on its header sorts and the sort is remembered" $?
    grep -q "customsort: first=\[" <<<"$OUT"; check "the table is sorted by it" $?
    grep -q "customcol: undone" <<<"$OUT"; check "the last edit is one undo step" $?
    sed -n 4p <<<"$K" | grep -q "defined=1 table=\[\]"; check "removing a column takes it out of the table" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario lib12_records_replace_marks 'library:wide,wait:300,library:record=empty|Ghost Book|Zed Author,library:record=isbn|0441013597,library:record=isbn|0441013598,wait:500,library:visible,library:replace=title|Ghost|Phantom,wait:400,library:search=phantom,wait:400,library:visible,library:search=ISBN,wait:400,library:visible,library:search=,library:marked=1,wait:300,library:search=marked:true,wait:400,library:visible,library:clearmarks,wait:300,library:search=marked:true,wait:400,library:visible,quit' '
    R=$(grep "^record:" <<<"$OUT")
    sed -n 1p <<<"$R" | grep -q "record: added id="; check "an empty record is added" $?
    sed -n 2p <<<"$R" | grep -q "record: added id="; check "a record is added from a valid ISBN" $?
    sed -n 3p <<<"$R" | grep -q "record: error .*not a valid ISBN"; check "an ISBN with a wrong check digit is refused" $?
    V=$(grep "^visible:" <<<"$OUT")
    sed -n 1p <<<"$V" | grep -q "visible: 4"; check "the two books and the two records are listed" $?
    grep -q "replace: changed=1 skipped=0" <<<"$OUT"; check "search and replace changed the title of one book" $?
    sed -n 2p <<<"$V" | grep -q "visible: 1"; check "and the new title is found" $?
    sed -n 3p <<<"$V" | grep -q "visible: 1"; check "the ISBN record is listed under its placeholder title" $?
    sed -n 4p <<<"$V" | grep -q "visible: 1"; check "marked:true finds the marked book" $?
    sed -n 5p <<<"$V" | grep -q "visible: 0"; check "clearing the marks empties it" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# L5: Send to Device in the actions of a shelf row opens the Devices window with the shelf's books
scenario lib12_shelf_send 'library:wide,wait:300,library:shelf=create=Trip,library:shelf=create=Empty,library:shelf=add=Trip|1+2,library:shelf=send=Empty,library:shelf=send=Trip,wait:800,devwin:dump,quit' '
    has "^shelves: send Empty ids=\[\]"; check "an empty shelf has nothing to send" $?
    has "^shelves: send Trip ids=\[[12], [12]\]"; check "the shelf sends its two books" $?
    has "^devwin: .*selected=0 books=0|^devwin: "; check "the Devices window opened" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# L12: the import folder is watched and new books are added by themselves (off by default); L33: the text index row
#   library:inboxwatch=on|off|check|FOLDER  `inboxwatch: watch=B folder=F monitors=N handled=N polling=B`
#   library:indexstatus[=rebuild]  `index_status: indexed=N books=N running=B text=".."`
scenario lib12_inbox_watch_and_index 'library:wide,wait:300,library:inboxwatch=@D@/watchme,library:inboxwatch=check,wait:800,library:visible,library:inboxwatch=on,wait:500,library:inboxwatch=check,wait:4000,library:visible,library:inboxwatch=check,library:visible,library:inboxwatch=off,wait:300,library:indexstatus,library:indexstatus=rebuild,wait:2500,library:indexstatus,quit' '
    W=$(grep "^inboxwatch:" <<<"$OUT")
    sed -n 1p <<<"$W" | grep -q "watch=false .*monitors=0"; check "the watcher is off until it is switched on" $?
    V=$(grep "^visible:" <<<"$OUT")
    sed -n 1p <<<"$V" | grep -q "visible: 2"; check "a check while it is off adds nothing" $?
    sed -n 3p <<<"$W" | grep -q "watch=true .*\(monitors=[1-9]\|polling=true\)"; check "switched on, the folder is monitored (or polled when the system has no inotify left)" $?
    sed -n 6p <<<"$W" | grep -q "watch=false .*monitors=0"; check "switched off, the monitors are gone" $?
    sed -n 2p <<<"$V" | grep -q "visible: 3"; check "the new book of the folder was added by itself" $?
    sed -n 3p <<<"$V" | grep -q "visible: 3"; check "a second check does not add it again" $?
    I=$(grep "^index_status:" <<<"$OUT")
    sed -n 1p <<<"$I" | grep -q "indexed=[0-9]* books=3"; check "the index row counts the books" $?
    sed -n 2p <<<"$I" | grep -q "text=\"[0-9] of 3 books"; check "a rebuild starts the indexing again" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# L27: a send from the Devices window is a job of the Jobs panel
scenario device_ui_job 'library:annotate,library:selectall,wait:600,library:send-to-device,wait:2500,devwin:device=SDCARD,wait:2000,jobs:dump,devwin:send,wait:4000,jobs:dump,quit' '
    J=$(grep "^jobs: " <<<"$OUT")
    sed -n 1p <<<"$J" | grep -q "running=0"; check "no job before the send" $?
    sed -n 2p <<<"$J" | grep -q "Send|Done|1.00|Sending .* to "; check "the send is a finished job of the Jobs panel" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# L18: tags written like Fiction.Fantasy show as a tree (off by default).   library:tagtree=on|off|rows
#   prints `tagtree: on=B rows=[Fiction(2) -Fantasy(1) ...] widgets=N` (the sidebar rows of the tree)
scenario lib12_tag_tree 'library:wide,wait:300,library:selectall,wait:500,library:bulkedit=tag:+Fiction.Fantasy|tag:+Fiction.Crime,wait:700,library:tagtree=rows,library:tagtree=on,wait:500,library:tagtree=rows,library:tagtree=off,wait:300,library:tagtree=rows,quit' '
    T=$(grep "^tagtree:" <<<"$OUT")
    sed -n 1p <<<"$T" | grep -q "on=false .*widgets=0"; check "the tree is off to begin with" $?
    sed -n 1p <<<"$T" | grep -q "rows=\[Fiction(2) -Crime(2) -Fantasy(2)\]"; check "the two tags nest under Fiction, which counts each book once" $?
    sed -n 2p <<<"$T" | grep -q "on=true .*widgets=[1-9]"; check "switched on, the sidebar lists the tree" $?
    sed -n 4p <<<"$T" | grep -q "on=false .*widgets=0"; check "switched off, the flat list is back" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# L19/L20: related books (Quickview) and notes about an author   library:select1, library:quickview, library:note=set|kind|name|text
scenario lib12_quickview_notes 'library:wide,wait:300,library:note=set|author|Test Author|Met at the fair,library:note=get|author|test author,library:note=set|series|S|x,library:note=set|book|x|y,library:select1,wait:400,library:quickview,wait:600,library:dialog,library:note=set|author|Test Author|,library:note=get|author|Test Author,quit' '
    N=$(grep "^note:" <<<"$OUT")
    sed -n 1p <<<"$N" | grep -q "note: saved"; check "a note about an author is saved" $?
    sed -n 2p <<<"$N" | grep -q "note: Some(\"Met at the fair\")"; check "it is found again whatever the case of the name" $?
    sed -n 4p <<<"$N" | grep -q "note: error"; check "a note about a book is refused" $?
    sed -n 6p <<<"$N" | grep -q "note: None"; check "an empty note removes it" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario wallabag_import 'library:add-web-open,wait:500,wallabag:open,wait:500,wallabag:set=https://mock.example/wallabag;MOCKTOKEN;,wallabag:import,wait:3000,wallabag:dump,library:dump,wallabag:import,wait:3000,wallabag:dump,a11y-audit:,quit' '
    W=$(grep "^wallabag: " <<<"$OUT")
    sed -n 1p <<<"$W" | grep -q "seen=2 last=ok: 2 added, 0 problems"; check "two saved Wallabag articles are imported (the token sign-in is skipped when a token is pasted)" $?
    grep -q "Saved Article One" <<<"$LIB"; check "an article becomes a book with its title" $?
    grep -q "Wally Bag" <<<"$LIB"; check "its author is kept" $?
    sed -n 2p <<<"$W" | grep -q "seen=2 last=ok: 0 added, 0 problems"; check "a second import adds nothing: the articles are remembered" $?
    has "^a11y: controls=[0-9]+ problems=0"; check "every control of the Wallabag form has a name" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# The check before sending also runs inside the Device window (Preferences -> Library switch on): a book with a
# blocking problem stops at the report; Continue sends anyway.
scenario meta_preflight_device 'library:selectall,wait:600,library:send-to-device,wait:2500,devwin:device=SDCARD,wait:2000,devwin:send,wait:2500,library:preflight=dump,devwin:dump,library:preflight=continue,wait:4000,devwin:dump,quit' '
    has "^preflight_state: shown=true .*go_label=Some\(\"Continue Anyway\"\)"; check "sending from the Device window stops at the report when a book has a blocking problem" $?
    has "^preflight_book: Broken Mimetype \| 1 blocking"; check "the report names the broken book" $?
    has "^devwin_sent: Sent [0-9]+ books? to "; check "after Continue the books are sent" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# ---- News recipes editor and site accounts (src/ui/news_recipes.rs) ------------------------------------------------
# Dev commands: `news:recipes=open;KEY|save|close|feed;N;TITLE|ADDRESS|MODE|MAX|ON|remove-feed;N|filters;INCLUDE|EXCLUDE|KEEP|
# REMOVE (~ is a new line)|issue;LANGUAGE|TEMPLATE|IMAGES|dump` (`news_recipes: open=B feeds=N modes=… on=… max=… language=…
# error=…`) and `news:recipes=sites|site-new|site-fill;HOST;USER;PASSWORD|site-save|site-edit;HOST|site-remove;HOST|site-undo|
# sites-dump` (`news_sites: open=B count=N hosts=… users=… form=B last=…`). Passwords are never printed.

scenario news_recipes_editor 'library:newsadd=Mock Daily|https://mock.example/feed.xml|manual|3,wait:300,library:newsshow,wait:900,news:recipes=open;mock-daily,wait:700,news:recipes=dump,news:recipes=feed;0;Front page|https://mock.example/feed.xml|1|5|1,news:recipes=feed;1;Second|https://mock.example/other.xml|2|0|0,news:recipes=filters;^First~Item|sponsored|article|.ad~.promo,news:recipes=issue;de|{title} {date}|0,news:recipes=dump,a11y-audit:,news:recipes=filters;(|||,news:recipes=save,wait:400,news:recipes=dump,news:recipes=filters;^First~Item|sponsored|article|.ad~.promo,news:recipes=feed;1;Second|ftp://other.example/x|2|0|0,news:recipes=save,wait:400,news:recipes=dump,news:recipes=feed;1;Second|https://mock.example/other.xml|2|0|0,news:recipes=save,wait:900,news:recipes=dump,library:newsdump,news:recipes=open;mock-daily,wait:700,news:recipes=dump,news:recipes=remove-feed;1,news:recipes=save,wait:600,library:newsdump,quit' '
    R=$(grep "^news_recipes: " <<<"$OUT")
    sed -n 1p <<<"$R" | grep -q "feeds=1 modes=0 on=1 max=0 language=en .*images=true include=0"; check "the editor shows the feed of the subscription with its defaults" $?
    sed -n 2p <<<"$R" | grep -q "feeds=2 modes=1+2 on=1+0 max=5+0 language=de template=\"{title} {date}\" images=false include=2 exclude=1 keep=1 remove=2"; check "two feeds (full articles with a limit, feed text switched off), the filters and the issue settings are collected" $?
    sed -n 3p <<<"$R" | grep -q "banner=true error=\".*invalid regex"; check "a pattern that is not valid is refused with a reason and the editor stays open" $?
    sed -n 4p <<<"$R" | grep -q "banner=true error=\".*http"; check "an address that is not http or https is refused" $?
    sed -n 5p <<<"$R" | grep -q "open=false"; check "Save closes the editor" $?
    N=$(find "$d/cfg" -name news.json | head -1)
    grep -q "Front page" "$N" && grep -q "sponsored" "$N" && grep -q "\\.promo" "$N"; check "the recipe is saved in news.json (section title, filters, selectors)" $?
    grep -q "\"language\": \"de\"" "$N" && grep -q "\"include_images\": false" "$N"; check "language and images are saved" $?
    ! grep -q "other.xml" "$N"; check "and the feed removed in the last edit is gone from it" $?
    grep "^news_sub:" <<<"$OUT" | sed -n 1p | grep -q "feeds=2 off=1"; check "after the edit: two feeds, one switched off (the switch is kept)" $?
    grep "^news_sub:" <<<"$OUT" | tail -1 | grep -q "feeds=1 off=0"; check "a feed can be removed again in the editor" $?
    sed -n 6p <<<"$R" | grep -q "feeds=2"; check "the saved feeds come back when the editor is opened again" $?
    ! has "^a11y-problem:"; check "every control of the editor has a name" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario news_sites_accounts 'library:newsshow,wait:900,news:recipes=sites,wait:600,news:recipes=sites-dump,news:recipes=site-new,wait:500,news:recipes=site-fill;https://News.Example.com/feed;reader;SENT-site-pw-5,news:recipes=site-save,wait:700,news:recipes=sites-dump,a11y-audit:,news:recipes=site-new,wait:400,news:recipes=site-fill;news.example.com;other;SENT-site-pw-6,news:recipes=site-save,wait:400,news:recipes=sites-dump,news:recipes=site-fill;intranet;me;SENT-site-pw-7,news:recipes=site-save,wait:400,news:recipes=sites-dump,dialog:close,wait:300,news:recipes=site-remove;news.example.com,wait:500,news:recipes=sites-dump,news:recipes=site-undo,wait:500,news:recipes=sites-dump,quit' '
    S=$(grep "^news_sites: " <<<"$OUT")
    sed -n 1p <<<"$S" | grep -q "open=true count=0 hosts= users= form=false"; check "the site accounts open empty" $?
    sed -n 2p <<<"$S" | grep -q "count=1 hosts=news.example.com users=reader form=false"; check "an address is reduced to its host (lower case) and saved with the user" $?
    sed -n 3p <<<"$S" | grep -q "count=1 .*form=true last=\"This site has an account already\\.\""; check "a second account for the same site is refused" $?
    sed -n 4p <<<"$S" | grep -q "count=1 .*form=true last=\"This is not a site address\\.\""; check "a name that is not a site address is refused" $?
    sed -n 5p <<<"$S" | grep -q "count=0 hosts= "; check "Remove takes the account away" $?
    sed -n 6p <<<"$S" | grep -q "count=1 hosts=news.example.com users=reader"; check "Undo puts it back" $?
    grep -q "news.example.com" "$d/cfg/bookx/config.json" && grep -q "\"user\": \"reader\"" "$d/cfg/bookx/config.json"; check "config.json keeps the host and the user name" $?
    ! grep -rqa "SENT-site-pw" "$d/cfg" "$d/cache"; check "no password is in any file (it is a secret field of the keyring)" $?
    ! has "SENT-site-pw"; check "no password is printed" $?
    ! has "^a11y-problem:"; check "every control of the site accounts has a name" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario news_recipes_win11_fit 'library:newsadd=Mock Daily|https://mock.example/feed.xml|manual|3,wait:300,app:win-size=900x600,app:text-scale=1.5,wait:600,library:newsshow,wait:900,a11y-audit:,news:recipes=open;mock-daily,wait:700,a11y-audit:,dialog:close,wait:300,news:recipes=sites,wait:500,a11y-audit:,news:recipes=site-new,wait:500,a11y-audit:,quit' '
    [ "$(grep -c "^dialog_fit: " <<<"$OUT")" -ge 4 ]; check "the window, the editor, the site accounts and the form report their size" $?
    ! grep "^dialog_fit: " <<<"$OUT" | grep -q "fits=false"; check "at 900x600 and 1.5 times the text every dialog fits the window" $?
    ! has "^a11y-problem:"; check "every control has a name" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'
