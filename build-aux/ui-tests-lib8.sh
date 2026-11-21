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
    has "^menu_counts: main=8 library=8 add=6 "; check "main menu 8 entries, Library menu 8, + menu 6 (all within 3-12)" $?
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
    has "^jobs: icon (task-due|system-run)-symbolic"; check "the Jobs button has its own icon (not the list icon of Shelves and the table)" $?
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

scenario lib9_notes_menus 'library:annotate,library:annotate-more,library:notes,wait:800,library:notes:export-menu,library:notes:anki-id=apkg:cloze:vocab,library:notes:anki-id=csv:basic,library:notes:anki-id=tsv:basic:vocab,library:notes:anki-id=pdf,library:notes:anki-dialog=apkg,wait:400,library:dialog,library:dialog-close,wait:200,library:resize=900x600,wait:700,library:notes:toolbar,quit' '
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
scenario lib9_grid_empty_click 'library:wide,wait:1500,library:gridcap=500,wait:1800,library:gridwidth,library:gridbg,library:select1,wait:400,library:click-empty=margin,wait:400,library:works=selected,library:select1,wait:400,library:click-empty=below,wait:400,library:works=selected,library:select1,wait:400,library:click-empty=inside,wait:400,library:works=selected,quit' '
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
