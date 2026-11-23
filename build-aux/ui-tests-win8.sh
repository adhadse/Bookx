# Scenarios of wave 8 (secondary windows and dialogs; docs/archive/human_interface_backlog.md WIN-L2/L3/L4/L5/L12, X9, XC-1).
# Sourced by ui-tests.sh (add `. "$HERE/ui-tests-win8.sh"` next to the ui-tests-sync.sh line). Dev commands:
# `dialog:ctrlw` (runs the Ctrl+W action of the visible dialog), `stats:state`, `library:saved=undo-delete=Name|query|index`,
# `library:opds=add-dialog|add-fill=NAME|ADDRESS|USER|PASSWORD|add-state`, `library:citations=state`,
# `library:audiobooks=unselect=N`, `vocab:list=search=Q` (prints the placeholder).

scenario win8_ctrl_w 'stats:show,wait:800,dialog:state,dialog:ctrlw,wait:700,dialog:state,quit' '
    O=$(grep "^dialog:" <<<"$OUT")
    sed -n 1p <<<"$O" | grep -q "open=true"; check "Reading Statistics is open" $?
    tail -1 <<<"$O" | grep -q "open=false"; check "Ctrl+W closes the dialog" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario win8_stats_empty 'stats:state,stats:show,wait:600,a11y-audit:,dialog:ctrlw,wait:400,quit' '
    has "^stats: empty=true"; check "a library with no reading is empty" $?
    has "^a11y: controls=[0-9]+ problems=0"; check "every control of the empty Statistics window has a name" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario win8_saved_undo 'library:saved=add=Mine|author:test,library:saved=add=Other|tag:x,library:saved=delete=Mine,library:saved=dump,library:saved=undo-delete=Mine|author:test|0,library:saved=dump,quit' '
    S=$(grep "^saved: " <<<"$OUT")
    [ "$(grep -c "^saved: Mine | author:test" <<<"$S")" = 1 ]; check "Undo puts the deleted search back" $?
    grep "^saved: " <<<"$OUT" | sed -n 2p | grep -q "^saved: Mine"; check "and puts it back in its old place (first)" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario win8_opds_add 'library:opds=open,wait:600,library:opds=add-dialog,wait:400,library:opds=add-state,library:opds=add-fill=Mine|http://host:8080/opds|u|secret,library:opds=add-state,library:opds=add-fill=Mine|https://host/opds|u|secret,library:opds=add-state,quit' '
    O=$(grep "^opds_add:" <<<"$OUT")
    sed -n 1p <<<"$O" | grep -q "add_enabled=false warning=false"; check "Add is insensitive while the address is empty" $?
    sed -n 3p <<<"$O" | grep -q "add_enabled=true warning=true"; check "a password for an http:// address shows the cleartext warning" $?
    sed -n 5p <<<"$O" | grep -q "add_enabled=true warning=false"; check "an https:// address has no warning" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario win8_citations_window 'library:citations=open,wait:3000,library:citations=state,wait:300,a11y-audit:,dialog:ctrlw,wait:400,quit' '
    has "^citations_window: save_mapped=true save_sensitive=true copy_sensitive=true refs=[0-9]+ references?"; check "Save and Copy are in the header and usable" $?
    has "^a11y: controls=[0-9]+ problems=0"; check "every control of Export Citations has a name" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario win8_vocab_placeholders 'vocab:own=Zed|the last letter,vocab:show-words,wait:700,vocab:list=search=zzzzqqq,wait:300,vocab:list=search=,wait:300,quit' '
    has "^vocab_list: rows=0 .* placeholder=\"No Results\""; check "a search with no match says No Results, not the empty-vocabulary text" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# ---- wave 9 (windows): docs/archive/human_interface_backlog.md WIN-L3/L5/L6/L7/L9/L10, WIN-R4/R72, X6, XC-2 ----
# Dev commands added: `library:trackers=window=FILE|state|press-import` (`trackers_window: ...`),
# `library:audiobooks=state` (`audiobooks_window: page= rows= link= sensitive=`), `library:opds=cancel`
# (and `search_open=`/`retry=` in `opds_view:`), `vocab:dialog=remove`, `import_review:` printed by
# `library:import-confirm`, `dialog_fit:` printed by `a11y-audit:` while a dialog is open,
# `reader:map-focus-note` (keyboard cursor on a note; `note=` in `book_map:`).

scenario win9_trackers_preview 'library:trackers=state,wait:600,library:trackers=window=@D@/goodreads.csv,wait:1500,library:trackers=state,wait:1500,a11y-audit:,library:trackers=press-import,wait:1500,library:trackers=state,wait:1500,quit' '
    T=$(grep "^trackers_window:" <<<"$OUT")
    sed -n 1p <<<"$T" | grep -q "trackers_window: closed"; check "no Trackers window before it is opened" $?
    sed -n 2p <<<"$T" | grep -q "apply=\"Import 2 Books\" apply_sensitive=true preview_visible=true matched=\"2\" changes=\"2\" not_found=\"Not Found (1)\" not_found_rows=1"; check "the preview counts the books, names the ones not found and the button says how many it imports" $?
    has "^dialog_fit: title=\"Reading Trackers\" .* fits=true default_visible=true .*clipped_buttons=0"; check "the Import button is in the header and on screen" $?
    has "^a11y: controls=[0-9]+ problems=0"; check "every control of the Trackers window has a name" $?
    sed -n 3p <<<"$T" | grep -q "trackers_window: closed"; check "Import writes the changes and closes the window" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario win9_audiobooks_rows 'library:audiobooks=open=@D@/audio,wait:1500,library:audiobooks=state,wait:800,library:audiobooks=unselect=0,wait:1500,a11y-audit:,wait:300,dialog:ctrlw,wait:400,quit' '
    A=$(grep "^audiobooks_window:" <<<"$OUT")
    sed -n 1p <<<"$A" | grep -q "page=list rows=1 link=\"Link Selected (1)\" sensitive=true"; check "the search runs in the background and then lists the pairing in place" $?
    sed -n 2p <<<"$A" | grep -q "page=list rows=1 link=\"Link Selected (0)\" sensitive=false"; check "unticking the only row leaves nothing to link" $?
    has "^dialog_fit: title=\"Audiobooks\" .* fits=true default_visible=true"; check "Link Selected is on screen" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario win9_audiobooks_empty 'library:audiobooks=link=@D@/audio,wait:800,library:audiobooks=open,wait:1500,library:audiobooks=state,wait:1500,quit' '
    has "^audiobooks_window: page=empty rows=0 link=\"Link Selected \(0\)\" sensitive=false"; check "with nothing to link the window shows No Audiobooks Found" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario opds_win9 'library:opds=open,wait:500,library:opds=add-catalog=Mock|https://mock.example/opds/root.xml|u|p,library:opds=add-catalog=Broken|https://mock.example/opds/missing.xml|,library:opds=catalog=Mock,library:opds=cancel,wait:1200,library:opds=dump,library:opds=catalog=Mock,wait:1200,library:opds=dump,library:opds=back,library:opds=catalog=Broken,wait:1500,library:opds=dump,quit' '
    D=$(grep "^opds_view:" <<<"$OUT")
    sed -n 1p <<<"$D" | grep -q "page=list .*back=false .*search=false"; check "Cancel on the loading page returns to the catalogues and drops the late answer" $?
    sed -n 2p <<<"$D" | grep -q "back=true .*search=true | search_open=true"; check "a catalogue has a search toggle in the header and its search bar is open" $?
    sed -n 3p <<<"$D" | grep -q "page=status .*retry=true"; check "a page that cannot be loaded offers Try Again" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario win9_vocab_remove_undo 'vocab:own=Zed|the last letter,vocab:show-words,wait:700,vocab:list=edit=Zed,wait:300,vocab:dialog=remove,wait:400,vocab:list=search=Zed,wait:300,vocab:dialog=undo-remove,wait:300,vocab:list=search=Zed,wait:300,quit' '
    L=$(grep "^vocab_list:" <<<"$OUT")
    sed -n 2p <<<"$L" | grep -q "^vocab_list: rows=0 "; check "removing the definition from the dialog takes the term out of the list" $?
    has "^vocab_dialog: remove undone"; check "the Undo of the toast is offered" $?
    sed -n 3p <<<"$L" | grep -q "^vocab_list: rows=1 \[Zed\[mine\]=the last letter\]"; check "Undo puts the definition back" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# the Book Map (WIN-R4, WIN-R72): it is not bigger than a 900x600 window, and the keyboard cursor shows the note of its cell
scenario win9_book_map "library:import=$ROOT/mapbook/lib,wait:3000,read:3,wait:3500,reader:hl-add=2|50|red|strikethrough|a note about the map,wait:300,reader:map,wait:900,reader:map-focus-note,wait:500,reader:map-dump,wait:500,a11y-audit:,dialog:ctrlw,wait:400,quit" '
    has "^book_map: .* card=true .* note=\"a note about the map\""; check "moving the cursor with the keys shows the note of the cell, not only hovering" $?
    has "^dialog_fit: title=\"Book Map\" dialog=7[0-9][0-9]x5[0-9][0-9] .* fits=true"; check "the Book Map is not bigger than a 900x600 window" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# the Vocabulary window: Add Term and the paste/import/export menu are in the header
scenario win9_vocab_header 'vocab:own=Zed|the last letter,vocab:show-words,wait:900,a11y-audit:,wait:300,dialog:ctrlw,wait:400,quit' '
    has "^dialog_fit: title=\"Vocabulary\" .* fits=true"; check "the Vocabulary window fits" $?
    has "^a11y: controls=[0-9]+ problems=0"; check "Add Term and More in the header have names" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# the lint (ui/i18n_lint.rs) is the test of XC-2: `cargo test lint`

# XC-7 / X6: with the text 1.5 times larger in a 900x600 window every secondary window still fits and its main button is on screen
scenario win9_scale_dialogs 'app:win-size=900x600,wait:800,library:opds=open,wait:2500,a11y-audit:,library:nav=home,wait:600,library:citations=open,wait:2500,a11y-audit:,dialog:ctrlw,wait:600,library:audiobooks=open,wait:2500,a11y-audit:,dialog:ctrlw,wait:600,library:trackers=open,wait:2500,a11y-audit:,dialog:ctrlw,wait:600,vocab:show-words,wait:2500,a11y-audit:,dialog:ctrlw,wait:600,quit' '
    D=$(grep "^dialog_fit:" <<<"$OUT")
    [ "$(grep -c " fits=true " <<<"$D")" = 4 ]; check "all four windows fit in a 900x600 window with 1.5 times larger text (Online Catalogues is a page of the library now)" $?
    has "^a11y: controls=[0-9]+ problems=0"; check "the Online Catalogues page has named controls at 1.5 times larger text" $?
    ! grep -q "default_visible=false" <<<"$D"; check "the default button of every window is on screen" $?
    ! grep -q "clipped_buttons=[1-9]" <<<"$D"; check "no button cuts its label" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# ---- wave 11 (HIG): Sync window rows, one window --------------------------------------------------------------
# Dev commands: `dialog:state` prints `dialog: open=B count=N` (N = dialogs on the main window), `sync:dump` adds
# `error_banner=B`.

scenario win11_sync_rows_one_window 'share:port0,action:sync,wait:900,dialog:state,sync:dump,a11y-audit:,sync:card=wifi,wait:900,dialog:state,quit' '
    D=$(grep "^dialog: open" <<<"$OUT")
    sed -n 1p <<<"$D" | grep -q "count=1"; check "the Sync window is one dialog" $?
    has "^sync_window: .*cards=wifi:Start setup:.*usb:.*opds:.*use_when=3 .*error_banner=false"; check "the three choices are rows with their use-when text and one button each, no error banner" $?
    has "^a11y: controls=[0-9]+ problems=0"; check "every control of the Sync window has a name" $?
    sed -n 2p <<<"$D" | grep -q "count=1"; check "Start setup replaces the Sync window by Preferences: exactly one dialog is open" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# ---- wave 11 (HIG): XC-7 text 1.5 times larger at 900x600 on every surface ----------------------------------------
# Dev commands: `app:text-scale=1.5` (Xft DPI, like the accessibility setting), `a11y-audit:` prints `window_fit: window=WxH
# offscreen=N clipped_buttons=M ...` (controls outside the window that are not in a scrolled area, buttons that cut their
# label) next to `dialog_fit:`.

scenario win11_scale_library 'app:win-size=900x600,app:text-scale=1.5,wait:1000,library:annotate,library:annotate-more,a11y-audit:,library:table,wait:700,a11y-audit:,library:notes,wait:800,a11y-audit:,library:nav=home,wait:500,library:health=open,wait:1500,a11y-audit:,dialog:ctrlw,wait:600,action:devices,wait:1500,a11y-audit:,dialog:ctrlw,wait:600,prefs:,wait:1200,a11y-audit:,prefs:pick=Sync,wait:500,a11y-audit:,dialog:ctrlw,wait:600,quit' '
    W=$(grep "^window_fit:" <<<"$OUT")
    [ "$(grep -c "^window_fit:" <<<"$OUT")" -ge 7 ]; check "the grid, table, notes, Health, devices and Preferences were all measured" $?
    ! grep -q "window=[0-9]*x[0-9]* offscreen=[1-9]" <<<"$W"; check "no control is pushed outside the window by the larger text" $?
    ! grep -q "clipped_buttons=[1-9]" <<<"$W"; check "no button of the library cuts its label" $?
    D=$(grep "^dialog_fit:" <<<"$OUT")
    [ "$(grep -c " fits=true " <<<"$D")" -ge 4 ]; check "Health, Devices and Preferences fit the window" $?
    ! grep -q "default_visible=false\|clipped_buttons=[1-9]" <<<"$D"; check "their main buttons are on screen and keep their labels" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario win11_scale_reader 'app:win-size=900x600,app:text-scale=1.5,wait:800,read:1,wait:2500,a11y-audit:,reader:menu,wait:500,a11y-audit:,quit' '
    W=$(grep "^window_fit:" <<<"$OUT")
    [ "$(grep -c "^window_fit:" <<<"$OUT")" -ge 2 ]; check "the reader was measured" $?
    ! grep -q "offscreen=[1-9]" <<<"$W"; check "no reader control is pushed outside the window" $?
    ! grep -q "clipped_buttons=[1-9]" <<<"$W"; check "no reader button cuts its label" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario win11_scale_editor 'edit:1,wait:3000,editor:resize=900x600,app:text-scale=1.5,wait:1000,a11y-audit:,editor:book-menu,wait:400,a11y-audit:,quit' '
    W=$(grep "^window_fit:" <<<"$OUT")
    [ "$(grep -c "^window_fit:" <<<"$OUT")" -ge 2 ]; check "the editor was measured" $?
    ! grep -q "offscreen=[1-9]" <<<"$W"; check "no editor control is pushed outside the window" $?
    ! grep -q "clipped_buttons=[1-9]" <<<"$W"; check "no editor header button cuts its label" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'
