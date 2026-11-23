# Scenarios of the editor human-interface wave 8 (docs/archive/human_interface_backlog.md, EDT-E1 undo toasts, EDT-E2 delete
# rules, EDT-E3 checkpoint rows, EDT-E6 Redo/Undo buttons, EDT-E22 contents undo). Sourced by ui-tests.sh (add
# `. "$HERE/ui-tests-editor8.sh"` next to ui-tests-sync.sh). Dev commands: `editor:toast-info` prints
# `editor_toast: text="…" button=<label|-> checkpoints=N`, `editor:toast-action` presses the button of the last toast
# (`editor_toast_action: pressed=<label|none>`), `editor:delete-ask=<path>|<path>` prints `editor_delete: confirm=yes|no
# alert=<heading>|<body>` (`-` when the files were deleted at once), `editor:cp-controls` prints `editor_cp_rows: rows=N
# max_controls=M subtitles=a|b`, `editor:cp-delete-undo=<id>` brings a deleted checkpoint back.

scenario editor_toast_undo 'edit:1,wait:3000,editor:addfile=stray.css,editor:tool=fix,wait:800,editor:toast-info,editor:dump,editor:toast-action,wait:800,editor:toast-info,editor:dump,editor:toast-action,quit' '
    T=$(grep "^editor_toast:" <<<"$OUT"); D=$(grep "^editor:" <<<"$OUT"); A=$(grep "^editor_toast_action:" <<<"$OUT")
    sed -n 1p <<<"$T" | grep -q "text=\"Fixed.*button=Undo checkpoints=1"; check "a tool toast says what changed (no save-or-restore sentence) and has an Undo button" $?
    ! sed -n 1p <<<"$T" | grep -q "restore the checkpoint"; check "the old long toast text is gone" $?
    sed -n 1p <<<"$A" | grep -q "pressed=Undo"; check "the button is Undo" $?
    sed -n 2p <<<"$D" | grep -q "checkpoints=0"; check "Undo removes the Before checkpoint again" $?
    grep "^editor_files:" <<<"$OUT" | tail -1 | grep -q "stray.css"; check "the file the tool removed is back" $?
    sed -n 2p <<<"$T" | grep -q "text=\"Undone\" button=-"; check "the Undone toast has no button" $?
    sed -n 2p <<<"$A" | grep -q "pressed=none"; check "pressing again does nothing" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_delete_rules 'edit:1,wait:3000,editor:addfile=stray.css,editor:delete-ask=OEBPS/chapter-1.xhtml,wait:300,editor:delete-ask=OEBPS/chapter-1.xhtml|OEBPS/chapter-2.xhtml,wait:300,editor:delete-ask=OEBPS/stray.css,wait:600,editor:toast-info,editor:toast-action,wait:600,editor:dump,quit' '
    L=$(grep "^editor_delete:" <<<"$OUT")
    sed -n 1p <<<"$L" | grep -q "confirm=yes alert=Delete “chapter-1.xhtml”?|.*other files still refer to it"; check "a file something refers to asks first and says so" $?
    sed -n 2p <<<"$L" | grep -q "confirm=yes alert=Delete 2 files?"; check "two files ask first and name the count" $?
    sed -n 3p <<<"$L" | grep -q "confirm=no alert=-"; check "one unused file is deleted at once" $?
    has "^editor_toast: text=\"Deleted 1 file.*button=Undo"; check "with an Undo toast" $?
    grep "^editor_files:" <<<"$OUT" | tail -1 | grep -q "stray.css"; check "Undo brings it back" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_checkpoint_rows 'edit:1,wait:3000,editor:cp-new=First,editor:cp-new=Second,editor:cp-dialog,wait:500,editor:cp-controls,a11y-audit:,editor:cp-pin=1,wait:300,editor:cp-controls,editor:cp-delete=2,wait:300,editor:cp-list,editor:cp-controls,editor:toast-info,editor:toast-action,wait:300,editor:cp-list,editor:cp-controls,quit' '
    R=$(grep "^editor_cp_rows:" <<<"$OUT"); L=$(grep "^editor_cp:.*list=" <<<"$OUT")
    sed -n 1p <<<"$R" | grep -q "rows=2 max_controls=2 "; check "a checkpoint row has two controls (Compare and the menu)" $?
    sed -n 2p <<<"$R" | grep -q "subtitles=.*Kept · "; check "a kept checkpoint says Kept" $?
    sed -n 3p <<<"$R" | grep -q "rows=1 "; check "delete takes the row away at once" $?
    tail -n 2 <<<"$L" | head -n 1 | grep -qv "2:Second"; check "and out of the list" $?
    has "^editor_toast: text=\"Checkpoint “Second” deleted\" button=Undo"; check "the toast offers Undo" $?
    tail -n 1 <<<"$L" | grep -q "2:Second"; check "Undo brings the checkpoint back" $?
    sed -n 4p <<<"$R" | grep -q "rows=2 "; check "and its row" $?
    ! has "^a11y-problem:"; check "the menu buttons have accessible names" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_revert_buttons 'edit:1,wait:3000,editor:addfile=stray.css,editor:tool=fix,wait:800,editor:x:undo-ckpt,wait:500,editor:toast-info,editor:toast-action,wait:500,editor:toast-info,quit' '
    T=$(grep "^editor_toast:" <<<"$OUT")
    sed -n 1p <<<"$T" | grep -q "text=\"Reverted to before .* button=Redo"; check "Revert to Before shows a Redo button" $?
    sed -n 2p <<<"$T" | grep -q "text=\"Reverted to after the change\" button=Undo"; check "and Redo shows an Undo button" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# ---- wave 9 (docs/archive/human_interface_backlog.md EDT-E4, E6, E9-E16, E18, E20-E22, E24-E26, E28-E30, E33-E35, E39, E42-E44) ----
# Dev commands (editor:...): header-info, tools-menu, book-menu, revert-state, layout, resize=WxH, find-open, replace-open,
# find-info, switcher-info, file-labels, banner-info, banner-action, save-failed=<why>, fonts-state, dialog-info,
# dialog-buttons, suff-toggle=<set>:<mode>, suff-add, suff-text, links-ui, links-cancel, narration-ui,
# narration-background, check:panel, spell:info, chars:size=WxH, chars:layout.

scenario editor_header_menus 'edit:1,wait:3000,editor:header-info,editor:tools-menu,editor:book-menu,quit' '
    H=$(grep "^editor_header:" <<<"$OUT"); T=$(grep "^editor_menu: name=tools" <<<"$OUT"); B=$(grep "^editor_menu: name=book" <<<"$OUT")
    sed -n 1p <<<"$H" | sed -E "s/.*buttons=([0-9]+).*/\1/" | { read n; [ "$n" -le 8 ]; }; check "the header has at most 8 buttons (Save, Undo, Redo, Find, Preview, Tools, Markup, Main Menu)" $?
    sed -n 1p <<<"$H" | grep -q "inside=yes"; check "every header button is inside the window" $?
    sed -n 1p <<<"$H" | grep -q "preview_tip=\"Hide Preview\""; check "the preview toggle says what it will do" $?
    sed -n 1p <<<"$T" | sed -E "s/.* top=([0-9]+) .*/\1/" | { read n; [ "$n" -le 12 ]; }; check "Tools has at most 12 top-level entries" $?
    sed -n 1p <<<"$T" | grep -q "labels=Check Book|Fix Problems Automatically|Fix & Clean|Styles|Media and Fonts|Structure|Accessibility|Links|Check Spelling|Reports|Checkpoints"; check "Tools lists Check Book, Fix Problems, the submenus, Spelling, Reports and Checkpoints" $?
    sed -n 1p <<<"$T" | grep -q " dup=-$"; check "no two Tools entries share a label" $?
    ! sed -n 1p <<<"$T" | grep -q "(plugin)"; check "plugin entries carry no (plugin) suffix" $?
    sed -n 1p <<<"$B" | sed -E "s/.* top=([0-9]+) .*/\1/" | { read n; [ "$n" -le 13 ]; }; check "the main menu has at most 13 entries" $?
    sed -n 1p <<<"$B" | grep -q "Preferences|Keyboard Shortcuts|Help|Report a Problem|About Bookx dup="; check "the main menu ends with Preferences, Keyboard Shortcuts, Help, Report a Problem and About Bookx" $?
    ! sed -n 1p <<<"$B" | grep -q "Close Other Tabs"; check "the tab commands stay in the tab menu" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_revert_enabled 'edit:1,wait:3000,editor:revert-state,editor:addfile=stray.css,editor:tool=fix,wait:800,editor:revert-state,editor:x:undo-ckpt,wait:600,editor:revert-state,quit' '
    R=$(grep "^editor_revert:" <<<"$OUT")
    sed -n 1p <<<"$R" | grep -q "undo=false redo=false"; check "with no tool run, both revert items are greyed out" $?
    sed -n 2p <<<"$R" | grep -q "undo=true redo=false"; check "after a tool, Revert to Before is available" $?
    sed -n 3p <<<"$R" | grep -q "undo=false redo=true"; check "after reverting, Revert to After is available" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_main_900 'edit:1,wait:3000,editor:resize=900x600,wait:1800,editor:find-open,editor:check,wait:1500,editor:header-info,editor:layout,editor:find-info,editor:check:panel,editor:switcher-info,editor:resize=1380x860,wait:1800,editor:layout,editor:header-info,quit' '
    H=$(grep "^editor_header:" <<<"$OUT"); L=$(grep "^editor_layout:" <<<"$OUT"); F=$(grep "^editor_find:" <<<"$OUT")
    sed -n 1p <<<"$H" | grep -q "inside=yes"; check "at 900x600 every header button is inside the window" $?
    sed -n 1p <<<"$H" | sed -E "s/.*buttons=([0-9]+).*/\1/" | { read n; [ "$n" -le 9 ]; }; check "at most 9 header buttons (8 plus the sidebar toggle)" $?
    sed -n 1p <<<"$H" | grep -q "sidebar_btn=true"; check "the collapsed sidebar has its header toggle" $?
    sed -n 1p <<<"$L" | grep -q "collapsed=true"; check "below 1000sp the sidebar slides over the code" $?
    sed -n 1p <<<"$L" | grep -q "minimap=false"; check "and the minimap goes" $?
    sed -n 1p <<<"$L" | sed -E "s/.*code_w=([0-9]+).*/\1/" | { read n; [ "$n" -ge 480 ]; }; check "the code column is at least 480 px wide at 900x600" $?
    sed -n 1p <<<"$F" | sed -E "s/.*height=([0-9]+).*/\1/" | { read n; [ "$n" -le 48 ]; }; check "the find bar is one row (48 px or less)" $?
    grep "^editor_switcher:" <<<"$OUT" | grep -q "ellipsized=0"; check "no sidebar tab title is ellipsized" $?
    sed -n 2p <<<"$L" | grep -q "collapsed=false"; check "back at 1380x860 the sidebar is beside the code" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_find_rows 'edit:1,wait:3000,editor:find-open,wait:700,editor:find-info,editor:replace-open,wait:700,editor:find-info,editor:find-open,wait:700,editor:find-info,quit' '
    F=$(grep "^editor_find:" <<<"$OUT")
    sed -n 1p <<<"$F" | grep -q "open=true replace=false"; check "Ctrl+F opens the bar without the replace row" $?
    sed -n 1p <<<"$F" | sed -E "s/.*height=([0-9]+).*/\1/" | { read n; [ "$n" -le 48 ]; }; check "the find bar is at most 48 px high" $?
    sed -n 2p <<<"$F" | grep -q "open=true replace=true"; check "Ctrl+H adds the replace row" $?
    sed -n 3p <<<"$F" | grep -q "open=false replace=false"; check "closing the bar folds the replace row too" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_panels_one 'edit:1,wait:3000,editor:check,wait:1500,editor:check:panel,editor:find,wait:700,editor:check:panel,editor:check,wait:1500,editor:check:panel,quit' '
    P=$(grep "^editor_check_panel:\|^check_panel:" <<<"$OUT")
    sed -n 1p <<<"$P" | grep -q "problems=true results=false"; check "Check Book shows only the Problems panel" $?
    sed -n 2p <<<"$P" | grep -q "problems=false results=true"; check "opening the results closes the Problems panel" $?
    sed -n 3p <<<"$P" | grep -q "problems=true results=false"; check "checking again swaps back" $?
    sed -n 1p <<<"$P" | grep -q "controls=[1-5]$"; check "the Problems header has at most five controls" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_spell_undo 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:500,editor:find-in-text=Paragraph 2 of,editor:insert=<p>Wrods and tehe</p>,editor:spell:info,editor:spell:open,wait:1500,editor:spell:info,editor:spell:select=Wrods,wait:300,editor:spell:info,editor:spell:add=Personal,wait:900,editor:spell:dump,editor:toast-info,editor:toast-action,wait:1500,editor:spell:dump,editor:spell:select=tehe,wait:300,editor:spell:ignore,wait:900,editor:spell:dump,editor:toast-info,editor:toast-action,wait:1500,editor:spell:dump,quit' '
    U=$(grep "^editor_spell_ui:" <<<"$OUT"); P=$(grep "^editor_spell_panel:" <<<"$OUT"); T=$(grep "^editor_toast:" <<<"$OUT")
    sed -n 1p <<<"$U" | grep -q "placeholder=true detail=false"; check "an empty spelling pane shows the Select a Word placeholder" $?
    sed -n 3p <<<"$U" | grep -q "placeholder=false detail=true"; check "selecting a word shows its details" $?
    sed -n 1p <<<"$T" | grep -q "added to Personal.*button=Undo"; check "Add to Dictionary offers Undo" $?
    ! sed -n 1p <<<"$P" | grep -q "Wrods"; check "the added word leaves the list" $?
    sed -n 2p <<<"$P" | grep -q "Wrods"; check "Undo reports the word again" $?
    ! grep -qx "Wrods" "$d/cfg/bookx/spelling-words.txt" 2>/dev/null; check "and takes it out of the Personal dictionary file" $?
    sed -n 2p <<<"$T" | grep -q "ignored until the editor closes.*button=Undo"; check "Ignore offers Undo" $?
    ! sed -n 3p <<<"$P" | grep -q "tehe"; check "an ignored word leaves the list" $?
    sed -n 4p <<<"$P" | grep -q "tehe"; check "Undo reports the ignored word again" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_sidebar_names 'edit:1,wait:3000,editor:addfile=archive_scan_of_the_old_book_one.css,editor:addfile=archive_scan_of_the_old_book_two.css,editor:file-labels,quit' '
    L=$(grep "^editor_file_labels:" <<<"$OUT")
    grep -q "…one.css=OEBPS/archive_scan_of_the_old_book_one.css" <<<"$L"; check "files that share a long prefix show only the part that differs, with the full path as tooltip" $?
    grep -q "chapter-1.xhtml=OEBPS/chapter-1.xhtml" <<<"$L"; check "other names stay whole" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_fonts_empty 'edit:1,wait:3000,editor:x:fonts-manage=open,wait:500,editor:fonts-state,quit' '
    has "^editor_fonts: empty_page=true sample_group=false"; check "a book without fonts shows the status page and no sample group" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_banner 'edit:1,wait:3000,editor:banner-info,editor:save-failed=disk is full,editor:banner-info,editor:banner-action,wait:900,editor:banner-info,quit' '
    B=$(grep "^editor_banner:" <<<"$OUT")
    sed -n 1p <<<"$B" | grep -q "revealed=false"; check "no banner while all is well" $?
    sed -n 2p <<<"$B" | grep -q "revealed=true title=\"Could not save the book: disk is full\" button=Try Again"; check "a failed save shows a banner with Try Again" $?
    sed -n 3p <<<"$B" | grep -q "revealed=false"; check "Try Again saves and the banner goes away" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_dialog_density_900 'edit:1,wait:3000,editor:resize=900x600,wait:1800,editor:x:dialog=smarten,wait:1200,editor:dialog-info,editor:x:dialog-click=preview,wait:300,editor:dialog-info,editor:dialog-close,wait:500,editor:x:dialog=styles,wait:600,editor:dialog-info,quit' '
    D=$(grep "^editor_dialog:" <<<"$OUT")
    sed -n 1p <<<"$D" | grep -q "card_open=false .*visible=yes"; check "the first option row is visible at 900x600 and the What will change card is folded" $?
    sed -n 2p <<<"$D" | grep -q "card_open=true"; check "Preview opens the card" $?
    sed -n 3p <<<"$D" | grep -q "ellipsized=0"; check "the rule kinds of Transform Styles are not cut off" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_a11y_sets 'edit:1,wait:3000,editor:tool=upgrade,wait:900,editor:x:dialog=a11y-form,wait:600,editor:suff-text,editor:suff-toggle=1:textual,editor:suff-text,editor:suff-toggle=1:visual,editor:suff-text,editor:suff-add,editor:suff-toggle=2:textual,editor:suff-text,quit' '
    S=$(grep "^editor_suff:" <<<"$OUT")
    sed -n 1p <<<"$S" | grep -q "boxes=4"; check "one set of four mode boxes at first" $?
    sed -n 2p <<<"$S" | grep -q "text=\"textual\""; check "ticking Textual writes the value" $?
    sed -n 3p <<<"$S" | grep -q "text=\"textual,visual\""; check "ticking Visual makes a combined set" $?
    sed -n 4p <<<"$S" | grep -q "text=\"textual,visual; textual\" boxes=8"; check "Add Alternative Set adds a second set" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_chars_narrow 'edit:1,wait:3000,editor:chars:size=900x560,wait:1500,editor:chars:layout,editor:chars:size=500x500,wait:1500,editor:chars:layout,quit' '
    L=$(grep "^editor_chars_layout:" <<<"$OUT")
    sed -n 1p <<<"$L" | grep -q "list=yes drop=no"; check "the wide window lists the character groups" $?
    sed -n 2p <<<"$L" | grep -q "list=no drop=yes"; check "below 720sp the groups become a drop-down" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_links_progress 'edit:1,wait:3000,editor:x:put=OEBPS/chapter-1.xhtml|<html xmlns="http://www.w3.org/1999/xhtml"><head><title>T</title></head><body><p><a href="https://mock.example/ok.html">ok</a><a href="https://mock.example/gone">gone</a></p></body></html>,editor:x:dialog=check-links,wait:400,editor:x:dialog-click=apply,wait:200,editor:links-ui,wait:2500,editor:links-ui,quit' '
    L=$(grep "^editor_links:" <<<"$OUT")
    sed -n 1p <<<"$L" | grep -q "progress=true cancel=true"; check "a running link check shows a progress bar and Cancel" $?
    sed -n 2p <<<"$L" | grep -q "progress=false cancel=false"; check "both go away when it is finished" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_narration_background 'edit:1,wait:3000,editor:tool=plugin:narration:start,editor:narration-ui,editor:narration-background,wait:500,editor:narration-ui,wait:6000,editor:mo:files,editor:toast-info,quit' '
    N=$(grep "^editor_narration:" <<<"$OUT")
    sed -n 1p <<<"$N" | grep -q "dialog=true background=true cancel=true"; check "the narration dialog offers Run in Background and Cancel Narration" $?
    sed -n 2p <<<"$N" | grep -q "dialog=false"; check "Run in Background closes the dialog" $?
    has "^editor_mo: smil=3 wav=3"; check "the narration still finishes and is added" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_cover_dialog 'edit:1,wait:3000,editor:x:dialog=cover,wait:600,editor:dialog-buttons,quit' '
    has "^editor_dialog_buttons: Close|Set Cover$"; check "the Set Cover footer has only Close and Set Cover (Remove Cover is a row in the dialog)" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_snippet_form 'edit:1,wait:3000,editor:snippets-form=Foot|bad trigger!|<i>x</i>,wait:400,editor:snippets-form=Foot|ft|<i>x</i>,wait:300,editor:dialog-buttons,quit' '
    F=$(grep "^editor_snippet_form:" <<<"$OUT")
    sed -n 1p <<<"$F" | grep -q "trigger_error=true save=false problem=\"The trigger may only contain"; check "an invalid trigger is reported inline and Save is off" $?
    sed -n 2p <<<"$F" | grep -q "trigger_error=false save=true"; check "a valid trigger turns Save on" $?
    has "^editor_dialog_buttons: Cancel|Save"; check "the form is a dialog with Cancel and Save" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_foreign_banner "edit:1,wait:3000,editor:x:foreign-open=$ROOT/xt/struct/book.fb2,wait:1500,editor:banner-info,quit" '
    has "^editor_banner: revealed=true title=\"Editing a copy of book.fb2. Save asks where to write\" button=-"; check "a book of another format shows a persistent banner that Save asks where to write" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_preview_head_900 'edit:1,wait:3000,editor:resize=900x600,wait:1800,editor:layout,editor:spell:open,wait:1200,editor:layout,editor:spell:close,wait:600,editor:layout,editor:resize=640x560,wait:1800,editor:layout,quit' '
    P=$(grep "^editor_preview_head:" <<<"$OUT"); L=$(grep "^editor_layout:" <<<"$OUT")
    sed -n 1p <<<"$P" | sed -E "s/.*buttons=([0-9]+).*/\1/" | { read n; [ "$n" -le 4 ]; }; check "the preview header has at most four buttons (colours, options, refresh, find)" $?
    sed -n 1p <<<"$P" | grep -q "preview_visible=true"; check "the preview is shown at 900x600" $?
    sed -n 2p <<<"$P" | grep -q "preview_visible=false"; check "opening the spelling pane in a narrow window replaces the preview" $?
    sed -n 3p <<<"$P" | grep -q "preview_visible=true"; check "closing it brings the preview back" $?
    sed -n 4p <<<"$L" | grep -q "vertical=true"; check "below 700sp the preview sits under the code" $?
    sed -n 4p <<<"$L" | sed -E "s/.*code_w=([0-9]+).*/\1/" | { read n; [ "$n" -ge 320 ]; }; check "and the code keeps at least 320 px" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# ---- Problems list rows and the main menu of the editor (owner issues UX4) ----
# Dev commands: `editor:check:rows` prints `editor_check_rows: rows=N max_row_h=H headers=K max_header_h=H gap=G`,
# `editor:book-menu-state` opens the Main Menu and prints `editor_book_menu: <entry>=on|off | … all_app=yes|no`,
# `editor:book-menu-activate=<Preferences|Keyboard Shortcuts|About Bookx|Command Palette>` runs that entry's action and
# prints `editor_book_menu_activate: <entry> activated=… dialog_over_editor=… dialog=<type>`.

scenario editor_problems_compact 'edit:1,wait:3000,editor:addfile=stray.css,editor:check,wait:1500,editor:check:rows,editor:resize=1380x860,wait:800,editor:check:rows,quit' '
    R=$(grep "^editor_check_rows:" <<<"$OUT" | tail -1)
    num() { sed -E "s/.* $1=([0-9-]+).*/\1/" <<<"$R"; }
    [ "$(num rows)" -ge 2 ]; check "the check found problems to list ($R)" $?
    [ "$(num headers)" -ge 1 ]; check "grouped under a file header row" $?
    [ "$(num max_row_h)" -le 32 ]; check "a problem row is at most 32px tall" $?
    [ "$(num max_header_h)" -le 30 ]; check "a file header row is at most 30px tall" $?
    [ "$(num gap)" -le 4 ]; check "no gap above 4px between consecutive rows" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_book_menu_enabled 'edit:1,wait:3000,editor:book-menu-state,wait:900,editor:book-menu-activate=Preferences,wait:1500,editor:book-menu-activate=Keyboard Shortcuts,wait:1500,editor:book-menu-activate=About Bookx,wait:1500,editor:book-menu-activate=Command Palette,wait:1500,quit' '
    M=$(grep "^editor_book_menu:" <<<"$OUT"); A=$(grep "^editor_book_menu_activate:" <<<"$OUT")
    grep -q "all_app=yes" <<<"$M"; check "Preferences, Keyboard Shortcuts, Help, About and Command Palette are enabled ($M)" $?
    ! grep -q "Preferences=off\|Keyboard Shortcuts=off\|About Bookx=off" <<<"$M"; check "none of the app entries is greyed" $?
    grep -q "Preferences activated=true dialog_over_editor=true" <<<"$A"; check "Preferences opens over the editor" $?
    grep -q "Keyboard Shortcuts activated=true dialog_over_editor=true" <<<"$A"; check "Keyboard Shortcuts opens over the editor" $?
    grep -q "About Bookx activated=true dialog_over_editor=true" <<<"$A"; check "About opens over the editor" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# wave 10, XC-6 and WIN-E4: F9 shows and hides the file sidebar of the editor; Preferences is the Bookx Preferences on
# the Editor page (there is no second preferences window). Dev commands: `editor:key=ACCEL` prints `editor_key: ACCEL ->
# <id>` and runs the command, `editor:sidebar-state` prints `editor_sidebar: shown=<bool> collapsed=<bool>`,
# `editor:prefs-open` is what the palette's Editor Preferences does.
scenario editor10_sidebar_key_and_prefs 'edit:1,wait:3000,editor:sidebar-state,editor:key=F9,wait:400,editor:sidebar-state,editor:key=F9,wait:400,editor:sidebar-state,editor:prefs-open,wait:1200,prefs:dump,quit' '
    K=$(grep "^editor_key:" <<<"$OUT"); S=$(grep "^editor_sidebar:" <<<"$OUT")
    sed -n 1p <<<"$K" | grep -q "F9 -> editor.sidebar"; check "F9 is bound to the editor sidebar" $?
    sed -n 1p <<<"$S" | grep -q "shown=true"; check "the file sidebar is shown to begin with" $?
    sed -n 2p <<<"$S" | grep -q "shown=false"; check "F9 hides it" $?
    sed -n 3p <<<"$S" | grep -q "shown=true"; check "F9 shows it again" $?
    has "^prefs: .*selected=Editor "; check "Editor Preferences opens the Bookx Preferences on the Editor page" $?
    has "^prefs: .*sections=General"; check "with the other pages and the search (one Preferences window for everything)" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# ---- wave 11 (T7 plugin trust, T9 launcher, safe mode, debug log). Dev commands: `plugins:sheet=ID` opens the
# capability sheet and prints `plugin_sheet: title="…" lines=A|B warning=B`, `plugins:sheet-allow` presses Allow
# (`plugin_sheet: allowed state=Trusted`), `plugins:state` prints `plugin_trust: ID State`, `safe:dump`
# (`safe_mode: active=B plugins_dir=P plugins_found=N offline=B`), `launch:write=PATH|TEXT`, `launch:spawn=ARGS`
# (`launch_spawn: exit=N`) starts a second bookx as a launcher does.

scenario editor_plugin_trust_sheet "plugins:state,plugins:sheet=$EDP,wait:600,dialog:state,plugins:sheet-allow,wait:400,plugins:state,plugins:enable=$EDP,edit:1,wait:3000,editor:tool=plugin:action:$EDP:trim,wait:600,dialog:state,quit" '
    S=$(grep "^plugin_sheet:" <<<"$OUT"); T=$(grep "^plugin_trust:" <<<"$OUT")
    sed -n 1p <<<"$T" | grep -q "$EDP NeverAsked"; check "a plugin nobody allowed has never been asked about" $?
    sed -n 1p <<<"$S" | grep -q "title=\"Allow Trim trailing whitespace to run?\" lines=Read the book that is open in the editor|Change the book that is open in the editor.* warning=true"; check "the sheet lists what the plugin may do in words and warns that it is unsigned" $?
    has "^dialog: open=true"; check "the sheet is a dialog" $?
    sed -n 2p <<<"$S" | grep -q "allow_pressed=true"; check "Allow was there to press" $?
    sed -n 3p <<<"$S" | grep -q "allowed state=Trusted"; check "Allow remembers the approval" $?
    sed -n 2p <<<"$T" | grep -q "$EDP Trusted"; check "the plugin is trusted afterwards" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario launch_forward_doc 'launch:write=@D@/Forwarded.md|---\ntitle: Forwarded Book\nauthor: Zed Writer\n---\n# One\n\nA second process hands this to the first.\n,launch:spawn=--doc @D@/Forwarded.md,wait:5000,reader:dump,quit' '
    has "^launch_write: Ok"; check "the sample book was written" $?
    has "^launch_spawn: exit=Some\(0\)"; check "the second bookx handed the request over and left at once" $?
    has "^labels:"; check "the running instance imported the book and opened it in the reader" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario launch_forward_open_with 'launch:write=@D@/OpenedWith.md|---\ntitle: Open With Book\nauthor: Zed Writer\n---\n# One\n\nSent by the file manager.\n,launch:spawn=@D@/OpenedWith.md,wait:4000,launch:titles,quit' '
    has "^launch_spawn: exit=Some\(0\)"; check "Open With: the second bookx forwarded the file and left" $?
    has "^launch_titles: .*Open With Book"; check "the running instance imported it" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_plugin_safe_mode 'safe:dump,plugins:enable=org.bookx.example.trim-whitespace,prefs:eink=on,wait:300,quit' '
    has "^safe_mode: active=true .*plugins_found=0 offline=true"; check "safe mode starts without plugins and offline" $?
    has "^plugin_enable: Err"; check "a plugin cannot be switched on in safe mode" $?
    [ ! -s "$d/cfg/bookx/config.json" ]; check "nothing is written to the settings file in safe mode" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor11_preview_colours "edit:1,wait:3000,editor:prefs-set=preview_fg=#ff0000,editor:prefs-set=preview_bg=#00ff00,editor:prefs-set=preview_link=zzz,wait:500,editor:prefs-open,wait:900,a11y-audit:,quit" '
    grep -q "\"preview_fg\": \"#ff0000\"" "$d/cfg/bookx/editor-look.json"; check "the preview text colour override is saved" $?
    grep -q "\"preview_bg\": \"#00ff00\"" "$d/cfg/bookx/editor-look.json"; check "and the background override" $?
    ! has "^a11y-problem:"; check "the colour buttons have names" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'
