# Scenarios of wave 8 (reader): the search popover (status line, empty state, size that fits the window), the Go to
# dialog wording, the new Ctrl+D / Ctrl+L keys, translated popover strings. Sourced by ui-tests.sh. Dev commands:
# `reader:search=TEXT` (type a query and open the popover), `reader:dump` (`search_empty: <placeholder shown> width=W
# height=H`), `reader:goto=TEXT` (checks the Go to text), `keys:dump=ACTION`.

scenario search_no_results_page 'read:1,wait:2500,reader:search=qzxqzxqzx,wait:3500,reader:dump,reader:search=the,wait:3500,reader:dump,quit' '
    S=$(grep "^search_status: " <<<"$OUT")
    sed -n 1p <<<"$S" | grep -q "^search_status: No results$"; check "a search that finds nothing says No results" $?
    E=$(grep "^search_empty: " <<<"$OUT")
    sed -n 1p <<<"$E" | grep -q "^search_empty: true"; check "and shows the No Results page in the list" $?
    sed -n 2p <<<"$S" | grep -qE "^search_status: [0-9]+ results"; check "a search with hits counts them" $?
    sed -n 2p <<<"$E" | grep -q "^search_empty: false"; check "and the empty page is gone" $?
    sed -n 2p <<<"$E" | grep -qE "width=[0-9]+ height=[0-9]+"; check "the popover has a size" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario reader_new_keys 'keys:dump=reader.bookmark,keys:dump=reader.goto,quit' '
    has "^keys: reader.bookmark effective=\\[.*<Control>d"; check "Ctrl+D is a default of the bookmark" $?
    has "^keys: reader.goto effective=\\[.*<Control>l"; check "Ctrl+L is a default of Go to" $?
    has "^keys: reader.goto effective=\\[.*<Control>g"; check "Ctrl+G still goes to a place" $?'

# ---- wave 9 (reader) -------------------------------------------------------------------------------------------
# Dev hooks added in wave 9: `reader:menu-dump` (`reader_menu: entries=N sections=N submenus=N narrow=B | Label [action] | ...`),
# `reader:menu-popup|menu-popdown`, `reader:ruler-state` (`ruler_state: action=Some(B) setting=B`), `reader:text-dump`
# (`text_pop: natural_height=H window_height=H hyphenate=B more_open=B`), `reader:text-more=open|close`,
# `reader:text-hyphen=on|off`, `reader:header-dump` (`reader_header: width=W title=W buttons=N clipped=N ...`),
# `reader:goto-hint=TEXT` (`goto_hint: ok LINE | error WHY`), `app:win-size=WxH`, `app:win-dump`, `app:toast-undo-test`,
# `app:toast-undo-press`, `app:fail-open`, `app:open-error`.

scenario reader_menu_structure 'read:1,wait:2500,reader:menu-dump,reader:ruler-state,reader:ruler=toggle,wait:300,reader:ruler-state,reader:menu-popup,wait:600,a11y-audit:,reader:menu-popdown,quit' '
    M=$(grep "^reader_menu:" <<<"$OUT" | head -1)
    grep -q "entries=12 sections=3 submenus=0" <<<"$M"; check "the Main Menu has 12 entries in three sections and no submenu ($M)" $?
    grep -q "Help \\[app.help\\]" <<<"$M" && grep -q "Preferences \\[app.preferences\\]" <<<"$M" && grep -q "Keyboard Shortcuts \\[app.shortcuts\\]" <<<"$M" && grep -q "About Bookx \\[app.about\\]" <<<"$M"; check "it has Preferences, Keyboard Shortcuts, Help and About" $?
    ! grep -q "Layout" <<<"$M"; check "the Layout sub-page is gone" $?
    grep -q "Reading Ruler \\[reader.ruler\\]" <<<"$M" && grep -q "Go to \\[reader.goto\\]" <<<"$M" && grep -q "Footer Statistics \\[reader.footer\\]" <<<"$M"; check "Go to, Footer Statistics and the Reading Ruler are in it" $?
    R=$(grep "^ruler_state:" <<<"$OUT")
    sed -n 1p <<<"$R" | grep -q "action=Some(false) setting=false"; check "the Reading Ruler check item is off while the ruler is off" $?
    sed -n 2p <<<"$R" | grep -q "action=Some(true) setting=true"; check "and follows the ruler when it is turned on" $?
    has "^a11y: controls=[0-9]+ problems=0"; check "every control has an accessible name (menu open)" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario reader_text_popover 'read:1,wait:2500,reader:text-dump,reader:text-more=open,wait:700,reader:text-hyphen=off,wait:300,reader:text-dump,reader:text-more=close,wait:300,reader:text-dump,quit' '
    T=$(grep "^text_pop:" <<<"$OUT")
    sed -n 1p <<<"$T" | awk "{split(\$2,a,\"=\"); exit !(a[2] <= 560 && a[2] > 0)}"; rc=$?; check "the Text Appearance controls are at most 560 px tall (the popover scrolls at 480) ($(sed -n 1p <<<"$T"))" $rc
    sed -n 1p <<<"$T" | grep -q "hyphenate=true"; check "Hyphenate words starts on" $?
    sed -n 1p <<<"$T" | grep -q "more_open=false"; check "More Text Options starts closed" $?
    sed -n 2p <<<"$T" | grep -q "hyphenate=false more_open=true"; check "More Text Options opens and Hyphenate words changes the setting" $?
    sed -n 3p <<<"$T" | grep -q "more_open=false"; check "and closes again" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# `reader:footer-dialog=dump|toggle=KEY|up=KEY|down=KEY` drives the real widgets of the Footer Statistics dialog
# (`footer_dialog: open=B items=<keys on> order=<all keys in order>`).
scenario footer_dialog 'read:1,wait:2500,reader:footer-open,wait:900,reader:footer-dialog=dump,reader:footer-dialog=up=percent,reader:footer-dialog=dump,reader:footer-dialog=toggle=percent,reader:footer-dialog=dump,a11y-audit:,quit' '
    D=$(grep "^footer_dialog:" <<<"$OUT")
    sed -n 1p <<<"$D" | grep -q "open=true"; check "the Footer Statistics dialog opens (a dialog, not a popover)" $?
    O1=$(sed -n 1p <<<"$D" | sed -E "s/.*order=//"); O2=$(sed -n 2p <<<"$D" | sed -E "s/.*order=//")
    [ "$O1" != "$O2" ]; check "its Move up button reorders the items ($O1 → $O2)" $?
    I2=$(sed -n 3p <<<"$D" | sed -E "s/.*items=(.*) order=.*/\1/"); I3=$(sed -n 5p <<<"$D" | sed -E "s/.*items=(.*) order=.*/\1/")
    [ "$I2" != "$I3" ]; check "its check box turns an item off or on ($I2 → $I3)" $?
    has "^a11y: controls=[0-9]+ problems=0"; check "every control has an accessible name" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# `reader:hl-count` prints `marks: highlights=N notes=N bookmarks=N`; `reader:remove-highlight` presses the Remove Highlight
# button; `app:toast-undo-press` presses the Undo of the toast shown last (`toast_undo: pressed` for the test toast).
scenario toast_undo_roundtrip 'app:toast-undo-test,wait:400,app:toast-undo-press,wait:200,app:toast-undo-press,quit' '
    [ "$(grep -c "^toast_undo: pressed" <<<"$OUT")" = 1 ]; check "the Undo button of a toast runs its action once" $?
    has "^toast_undo: none"; check "and a second press finds nothing left to undo" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario undo_remove_highlight 'read:1,wait:3500,reader:select,wait:500,reader:hl,wait:700,reader:note,wait:500,reader:ui-hl-popover,wait:900,reader:hl-count,reader:remove-highlight,wait:1000,reader:hl-count,wait:300,app:toast-undo-press,wait:1000,reader:hl-count,quit' '
    M=$(grep "^marks:" <<<"$OUT")
    sed -n 1p <<<"$M" | grep -q "highlights=1 notes=1"; rc=$?; check "a highlight with a note exists ($(sed -n 1p <<<"$M"))" $rc
    sed -n 2p <<<"$M" | grep -q "highlights=0 notes=0"; check "Remove Highlight takes it and its note away at once, without a dialog" $?
    sed -n 3p <<<"$M" | grep -q "highlights=1 notes=1"; check "Undo in the toast brings both back" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario undo_remove_bookmark 'read:1,wait:5000,reader:bookmark-toggle,wait:900,reader:hl-count,reader:bookmark-toggle,wait:900,reader:hl-count,wait:300,app:toast-undo-press,wait:900,reader:hl-count,quit' '
    M=$(grep "^marks:" <<<"$OUT")
    sed -n 1p <<<"$M" | grep -q "bookmarks=1"; check "Ctrl+D adds a bookmark" $?
    sed -n 2p <<<"$M" | grep -q "bookmarks=0"; check "removing it needs no confirmation" $?
    sed -n 3p <<<"$M" | grep -q "bookmarks=1"; check "Undo brings the bookmark back" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario reader_open_error 'app:fail-open,read:1,wait:1500,app:open-error,quit' '
    E=$(grep "^open_error:" <<<"$OUT")
    grep -q "shown=true title=\"Could not open the book\"" <<<"$E"; check "a book that fails to open shows a page that says so, not only a toast ($E)" $?
    grep -q "description=\"The file is damaged" <<<"$E"; check "with the reason" $?
    grep -q "button=\"Back to Library\"" <<<"$E"; check "and a Back to Library button" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario big_book_opening_page 'wait:500,read:1,wait:9000,reader:opening-state,reader:opening-demo,wait:300,reader:opening-state,wait:1800,reader:opening-state,quit' '
    O=$(grep "^opening:" <<<"$OUT")
    sed -n 3p <<<"$O" | grep -q "visible=false"; check "the Opening Book page is gone once the book is on screen" $?
    sed -n 2p <<<"$O" | grep -q "visible=true shown_once=true"; rc=$?; check "the page exists over the reader and can be shown ($(sed -n 2p <<<"$O"))" $rc
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario reader_jump_buttons 'read:1,wait:3000,reader:header-dump,quit' '
    has "^reader_header: .*back=Previous Position"; check "the jump button says Previous Position, not Back" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# `vocab:list=delete=WORD` presses the trash of the row (the term is removed at once, a toast offers Undo), `vocab:list=undo`
# presses that Undo; the dump has `max_buttons=N`, the most buttons any row has (jump + remove at most).
scenario vocab_remove_undo 'vocab:own=zorb|a rolling ball,vocab:show-words,wait:900,vocab:list=,vocab:list=delete=zorb,vocab:list=undo,vocab:own-dump,quit' '
    L=$(grep "^vocab_list:" <<<"$OUT")
    sed -n 1p <<<"$L" | grep -q "rows=1 "; check "the list shows the term" $?
    sed -n 1p <<<"$L" | grep -qE "max_buttons=[0-2] "; rc=$?; check "a row has at most two buttons (remove; jump when the term was met in the book): $(sed -n 1p <<<"$L" | sed -E "s/.*(max_buttons=[0-9]+).*/\1/")" $rc
    sed -n 2p <<<"$L" | grep -q "rows=0 "; check "Remove takes the term away at once" $?
    sed -n 3p <<<"$L" | grep -q "rows=1 \[zorb\[mine\]=a rolling ball\]"; check "Undo brings it back with its definition" $?
    has "^vocab_own_entries: .*zorb=a rolling ball"; check "and the vocabulary has it again" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# `reader:ruler=place:Y` is a tap on the page at Y, `reader:ruler=drag:Y0:DY` a finger that grabs the band at Y0 and moves
# DY (the same code the touch gestures run); the state line has `band=TOP..BOTTOM`.
scenario ruler_touch 'read:1,wait:2500,reader:ruler=toggle,wait:1200,reader:ruler=place:300,reader:ruler=drag:300:120,reader:ruler=drag:30:200,reader:ruler=place:5000,quit' '
    B=$(grep "^ruler: " <<<"$OUT" | sed -E "s/.*band=([0-9]+)\.\.([0-9]+).*/\1 \2/")
    top() { sed -n "$1p" <<<"$B" | cut -d" " -f1; }
    [ "$(top 2)" -ge 200 ] && [ "$(top 2)" -le 320 ]; rc=$?; check "a tap puts the band there ($(sed -n 2p <<<"$B"))" $rc
    [ "$(top 3)" -gt "$(top 2)" ]; rc=$?; check "dragging the band with a finger moves it ($(sed -n 2p <<<"$B") → $(sed -n 3p <<<"$B"))" $rc
    [ "$(top 4)" = "$(top 3)" ]; check "a finger that starts far from the band does not move it" $?
    [ "$(top 5)" -gt "$(top 4)" ]; rc=$?; check "the band stays inside the page when it is put below it ($(sed -n 5p <<<"$B"))" $rc
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# `reader:esc` presses Escape and prints the action (`esc: ClearSelection`); order: popover, handle drag, selection, sidebar
# over the page, full screen, leave the book.
scenario reader_esc_order 'read:1,wait:3000,reader:select,wait:900,reader:esc,wait:700,reader:esc,wait:500,reader:esc,wait:900,quit' '
    E=$(grep "^esc: " <<<"$OUT")
    sed -n 1p <<<"$E" | grep -qE "esc: (ClosePopover|ClearSelection)"; rc=$?; check "with a selection Escape first puts the popover or the selection away ($(sed -n 1p <<<"$E"))" $rc
    sed -n 2p <<<"$E" | grep -qE "esc: (ClearSelection|Leave)"; rc=$?; check "then the selection ($(sed -n 2p <<<"$E"))" $rc
    sed -n 3p <<<"$E" | grep -q "esc: Leave"; rc=$?; check "and only with nothing left open it leaves the book ($(sed -n 3p <<<"$E"))" $rc
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# An image with an `alt` is part of the page text a screen reader gets (`reader:page-text` prints `page_text: "..."`):
# the float book of the suite has `<img alt="pic">` in its second paragraph.
scenario image_alt_in_page_text "library:import=$ROOT/float-epub/lib,wait:2500,read:3,wait:3500,reader:page-text,quit" '
    grep -q "^page_text: .*Image: pic" <<<"$OUT"; check "a picture with an alt is read as Image: pic in the page text" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# The reader at 900, 600 and 360 px: at 600 sp and less the header gives up Fullscreen and Read Aloud (they are the first entries of
# the Main Menu), the title keeps at least 100 px and no control sticks out (`reader:header-dump`, `reader:menu-dump`).
scenario reader_narrow_600x600 'read:1,wait:3000,app:win-size=900x600,wait:1200,reader:header-dump,reader:menu-dump,app:win-size=600x600,wait:1500,app:win-dump,reader:header-dump,reader:menu-dump,reader:menu-popup,wait:700,a11y-audit:,reader:menu-popdown,quit' '
    H=$(grep "^reader_header:" <<<"$OUT")
    sed -n 1p <<<"$H" | grep -q "fullscreen=true read_aloud=true"; rc=$?; check "at 900 px the header has Fullscreen and Read Aloud ($(sed -n 1p <<<"$H"))" $rc
    sed -n 2p <<<"$H" | grep -q "narrow=true"; check "at 600 px the header is in its narrow form" $?
    sed -n 2p <<<"$H" | grep -q "fullscreen=false read_aloud=false"; check "Fullscreen and Read Aloud left the header" $?
    W=$(grep "^window:" <<<"$OUT" | sed -E "s/window: ([0-9]+)x.*/\1/")
    MIN=$(sed -n 2p <<<"$H" | sed -E "s/.* min_header=([0-9]+) .*/\1/")
    [ -n "$W" ] && [ "$MIN" -le "$W" ]; check "the header fits the ${W} px window (it needs ${MIN} px: no control is clipped)" $?
    sed -n 2p <<<"$H" | sed -E "s/.* width=([0-9]+) title=([0-9]+) .*/\2/" | awk "{exit !(\$1 >= 100)}"; rc=$?; check "and the title keeps at least 100 px ($(sed -n 2p <<<"$H" | sed -E "s/.* title=([0-9]+) .*/\1/") px)" $rc
    M=$(grep "^reader_menu:" <<<"$OUT")
    sed -n 1p <<<"$M" | grep -q "entries=12 "; check "the wide menu has 12 entries" $?
    sed -n 2p <<<"$M" | grep -q "entries=15 .*narrow=true | Fullscreen \\[reader.fullscreen\\] | Read Aloud \\[reader.read-aloud\\] | Text Appearance \\[reader.text-appearance\\]"; check "the narrow menu starts with Fullscreen, Read Aloud and Text Appearance" $?
    has "^a11y: controls=[0-9]+ problems=0"; check "every control has an accessible name at the narrow width" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

# PDF and comic readers: the same Main Menu builder, the same narrow form at 600 px (Fullscreen moves into the menu).
scenario pdf_menu_narrow "library:import=$PDFS/story,wait:3500,read:3,wait:2500,reader:menu-dump,app:win-size=600x600,wait:1500,reader:header-dump,reader:menu-dump,quit" '
    M=$(grep "^reader_menu:" <<<"$OUT")
    sed -n 1p <<<"$M" | grep -q "entries=7 .*Go to Page \\[reader.goto\\] | Reflow as Text \\[reader.reflow\\] | Properties \\[reader.properties\\].*Help \\[app.help\\]"; rc=$?; check "the PDF reader has a Main Menu with Go to Page, Reflow, Properties and the app group ($(sed -n 1p <<<"$M" | cut -c1-60))" $rc
    sed -n 2p <<<"$M" | grep -q "narrow=true | Fullscreen \\[reader.fullscreen\\]"; check "at 600 px Fullscreen is the first entry" $?
    grep "^reader_header:" <<<"$OUT" | grep -q "fullscreen=false"; check "and the header lost its button" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario comic_menu_and_look "library:import=$COMICS/story,wait:3500,read:3,until:comic:15000,reader:menu-dump,reader:view-dump,reader:comic-fit=width,reader:comic-rtl=on,wait:900,reader:comic-state,reader:view-dump,reader:comic-reset,wait:600,reader:view-dump,reader:comic-failed-text,app:win-size=600x600,wait:1500,reader:header-dump,quit" '
    M=$(grep "^reader_menu:" <<<"$OUT")
    sed -n 1p <<<"$M" | grep -q "entries=6 .*Go to Page \\[reader.goto\\] | Properties \\[reader.properties\\] | Preferences \\[app.preferences\\] | Keyboard Shortcuts \\[app.shortcuts\\] | Help \\[app.help\\] | About Bookx \\[app.about\\]"; check "the comic reader has a Main Menu (Go to Page, Properties, then the app group, Help included)" $?
    V=$(grep "^comic_view:" <<<"$OUT")
    sed -n 1p <<<"$V" | grep -q "reset_visible=false"; check "Use the Defaults for This Book is hidden while the book follows the defaults" $?
    sed -n 2p <<<"$V" | grep -q "reset_visible=true"; check "and shows once the book differs" $?
    sed -n 3p <<<"$V" | grep -q "reset_visible=false"; check "and goes away after it was used" $?
    sed -n 1p <<<"$V" | grep -q "labels=Right to left|Guided view|Grey pages"; check "the switches have short names (Right to left, Guided view, Grey pages)" $?
    sed -n 1p <<<"$V" | grep -oE "natural_height=[0-9]+" | cut -d= -f2 | awk "{exit !(\$1 <= 560)}"; rc=$?; check "the View popover is not taller than the window allows ($(sed -n 1p <<<"$V" | cut -c1-60))" $rc
    has "^comic_failed_text: \"This page cannot be shown\""; check "a damaged page says This page cannot be shown" $?
    grep "^reader_header:" <<<"$OUT" | grep -q "fullscreen=false narrow=true"; check "at 600 px the comic header gives up Fullscreen" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# The selection popover wraps: at 360 px it is not wider than the window, the colour swatches are 32 px and a one-word
# selection shows the words of the plus button (`reader:sel-dump`: `sel_pop: width=W height=H window=W swatch=WxH vocab_row=B`).
scenario selection_popover_narrow 'read:1,wait:3000,app:win-size=360x700,wait:1500,reader:ui-dblclick,wait:1500,reader:sel-dump,reader:esc,reader:esc,wait:500,reader:select,wait:1200,reader:sel-dump,quit' '
    S=$(grep "^sel_pop:" <<<"$OUT")
    W=$(sed -n 1p <<<"$S" | sed -E "s/.*width=([0-9]+) .*/\1/"); WIN=$(sed -n 1p <<<"$S" | sed -E "s/.* window=([0-9]+) .*/\1/")
    [ "$W" -gt 0 ] && [ "$W" -le "$WIN" ]; check "at 360 px the popover is not wider than the window (${W} of ${WIN})" $?
    sed -n 1p <<<"$S" | grep -qE "swatch=3[2-9]x3[2-9]"; check "the colour swatches are at least 32x32" $?
    sed -n 1p <<<"$S" | grep -q "vocab_row=true"; check "a one-word selection shows the Add to Vocabulary row" $?
    sed -n 2p <<<"$S" | grep -q "vocab_row=false"; check "a longer selection does not" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario goto_live_hint 'read:1,wait:3000,reader:goto-hint=5,reader:goto-hint=37%,reader:goto-hint=ch 2,reader:goto-hint=banana,reader:goto-hint=loc 3,quit' '
    G=$(grep "^goto_hint:" <<<"$OUT")
    sed -n 1p <<<"$G" | grep -qE "^goto_hint: ok Page 5 of [0-9]+"; rc=$?; check "a number says Page 5 of N ($(sed -n 1p <<<"$G"))" $rc
    sed -n 2p <<<"$G" | grep -q "^goto_hint: ok 37 percent"; check "37% says 37 percent" $?
    sed -n 3p <<<"$G" | grep -qE "^goto_hint: ok Chapter 2 of [0-9]+"; check "a chapter says Chapter 2 of N" $?
    sed -n 4p <<<"$G" | grep -q "^goto_hint: error"; check "text that leads nowhere says why" $?
    sed -n 5p <<<"$G" | grep -qE "^goto_hint: ok Location 3 of [0-9]+"; check "a location says Location 3 of N" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

# Regression: after a keyboard selection was committed (Enter) and its popover closed with Escape, extending the
# selection with Ctrl+Shift+arrows and pressing h highlighted the OLD words. `reader:ksel=word+` is Ctrl+Shift+Right,
# `reader:selkey=commit|highlight` is Enter / h; `reader:hl-count` prints the text of the last highlight (`last="..."`).
scenario keyboard_selection_extended_after_popover 'read:1,wait:3500,reader:ksel=word+,reader:ksel=word+,reader:ksel=word+,wait:300,reader:selkey=commit,wait:700,reader:esc,wait:500,reader:ksel=word+,reader:ksel=word+,wait:300,reader:selkey=highlight,wait:900,reader:hl-count,quit' '
    M=$(grep "^marks:" <<<"$OUT")
    grep -q "highlights=1 " <<<"$M"; check "one highlight was made ($M)" $?
    grep -q "last=\"Chapter 1.*Paragraph 1 of\"" <<<"$M"; check "it covers the extended selection (up to \"Paragraph 1 of\"), not the one committed before" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'
