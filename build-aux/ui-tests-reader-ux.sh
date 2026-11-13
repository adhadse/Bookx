# Scenarios of the reader UX work (docs/enhancements.md 10, 12, 13, 16 and bug 9): the sidebar, the footer
# statistics, the text selection handles and the Wikipedia picture. Sourced by ui-tests.sh (uses its `scenario`,
# `check`, `has`, $ROOT, $CLI). Dev commands: `reader:side-*`, `reader:footer*`, `reader:bulk-highlights=N`,
# `reader:handles*` (src/ui/reader.rs).

# a big EPUB (about 2000 contents entries) for the sidebar scenarios
mkdir -p "$ROOT/big"
python3 "$HERE/make-big-epub.py" "$ROOT/big/big.epub" 2000 >/dev/null


# ---- bug 9 / item 12: the sidebar of a big book (about 2000 contents entries, 500 highlights) ----
# The hidden lists are not built, the toggle costs a few milliseconds of work per frame (a budget of
# 100 ms; Broadway's frame clock itself is slow and says nothing), the sidebar lies over the page.
scenario side_perf 'read:1,wait:3000,reader:bulk-highlights=500,wait:500,reader:side-state,reader:side-time=open,wait:1500,reader:side-state,reader:side-time=close,wait:1500,reader:side-time=open,wait:1500,reader:side-tab=notes,wait:800,reader:side-state,reader:side-time=close,wait:1500,reader:side-time=open,wait:1500,reader:side-state,quit' '
    S=$(grep "^side: " <<<"$OUT")
    sed -n 1p <<<"$S" | grep -q "shown=false collapsed=true toc=20[0-9][0-9] bookmarks=0 notes_rows=0 notes=500 rebuilds=0 dirty=true"; check "a big book: 2000+ contents entries are in the model, the 500 highlights wait unbuilt while the sidebar is shut" $?
    sed -n 2p <<<"$S" | grep -q "shown=true .* rebuilds=0 .* page=toc"; check "opening the contents does not build the notes list" $?
    sed -n 3p <<<"$S" | grep -q "shown=true .* notes_rows=520 notes=500 rebuilds=1 dirty=false .* page=notes"; check "the notes list (500 highlights under 20 chapter headings) is built once, when its page is shown" $?
    sed -n 4p <<<"$S" | grep -q "rebuilds=1"; check "closing and opening the sidebar again builds nothing" $?
    T=$(grep "^side_time: " <<<"$OUT")
    [ "$(wc -l <<<"$T")" -eq 5 ]; check "every toggle was timed" $?
    awk "{for(i=1;i<=NF;i++) if (\$i ~ /^work_ms=/) {split(\$i,a,\"=\"); if (a[2]+0 >= 100) bad=1}} END {exit bad}" <<<"$T"; check "update, layout and paint of a frame stay under 100 ms while the sidebar opens and closes (budget)" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario side_notes 'read:1,wait:3000,reader:bulk-highlights=500,reader:side-tab=notes,reader:side-open,wait:900,reader:side-state,reader:side-filter=number 33,reader:side-filter=,reader:side-filter=|green||,reader:side-filter=||underline|,reader:side-filter=|||note,reader:side-filter=|yellow||note,reader:side-filter=number 33|blue|,reader:side-filter=zzz,reader:side-filter=,reader:side-act=0:recolor:green,wait:300,reader:side-filter=|green||,reader:side-act=0:note:,wait:300,reader:side-filter=|||note,reader:side-act=1:delete,wait:300,reader:side-state,reader:side-act=0:restyle:underline,wait:300,reader:side-filter=||underline|,quit' '
    F=$(grep "^side_filter: " <<<"$OUT" | sed -E "s/.* -> //")
    sed -n 1p <<<"$F" | grep -q "^14$"; check "the search finds the highlights by their text (every word must occur: 33, 133, 233, 333, 433 and 330-339)" $?
    sed -n 2p <<<"$F" | grep -q "^500$"; check "without a filter all 500 are listed" $?
    sed -n 3p <<<"$F" | grep -q "^100$"; check "the colour filter keeps the green ones" $?
    sed -n 4p <<<"$F" | grep -q "^125$"; check "the style filter keeps the underlined ones" $?
    sed -n 5p <<<"$F" | grep -q "^167$"; check "Only with a note keeps the highlights that have one" $?
    sed -n 6p <<<"$F" | grep -q "^34$"; check "colour and note filters combine" $?
    sed -n 7p <<<"$F" | grep -q "^2$"; check "a search and a colour filter combine (332 and 337 are blue)" $?
    sed -n 8p <<<"$F" | grep -q "^0$"; check "a search that nothing matches lists nothing" $?
    sed -n 10p <<<"$F" | grep -q "^101$"; check "changing the colour of a highlight from the list moves it to the other colour" $?
    sed -n 11p <<<"$F" | grep -q "^166$"; check "clearing the note of a highlight takes it out of the notes filter" $?
    has "^side: shown=true .* notes=499 "; check "deleting a highlight from the list removes it" $?
    sed -n 12p <<<"$F" | grep -q "^126$"; check "changing the style of a highlight from the list moves it to the other style" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario side_a11y 'read:1,wait:3000,reader:bulk-highlights=20,reader:side-open,wait:700,a11y-audit:,reader:side-tab=bookmarks,wait:400,a11y-audit:,reader:side-tab=notes,wait:700,a11y-audit:,quit' '
    [ "$(grep -c "^a11y:" <<<"$OUT")" -eq 3 ]; check "the audit ran on the three pages of the sidebar" $?
    # (the Style Tweaks panel of the reader has switches without names; it is not part of the sidebar)
    ! grep "^a11y-problem:" <<<"$OUT" | grep -v "AdwActionRow.header" | grep -q .; check "every control of the sidebar has an accessible name" $?'

# ---- item 13: the statistics in the footer ----
scenario footer_config 'read:1,wait:2500,reader:footer,reader:footer-preset=minimal,reader:footer,reader:footer-preset=everything,reader:footer-tick,reader:footer,reader:footer-toggle=author,reader:footer-toggle=title,reader:footer-move=0:3,reader:footer,reader:footer-own=on,reader:footer-preset=standard,reader:footer,reader:footer-own=off,reader:footer,reader:footer-align=spread,reader:footer,quit' '
    L=$(grep "^footer: " <<<"$OUT")
    sed -n 1p <<<"$L" | grep -qE "^footer:  \| [0-9]+ of [0-9]+  ·  Chapter 1  ·  .*minute.* \|  bar=true own=false items=page,chapter,left_chapter,bar align=Centered"; check "the default footer is one centred line (page, chapter, time left) with the bar" $?
    sed -n 2p <<<"$L" | grep -qE "^footer:  \| [0-9]+ of [0-9]+ \|  bar=false .* align=Centered"; check "Minimal shows just the page, centred, without the bar" $?
    sed -n 3p <<<"$L" | grep -q "^footer:  | .*[0-9]* of [0-9]*  ·  .* in chapter  ·  .*%  ·  .*minute.*Test Author.*UI Test Book\|^footer:  | .*UI Test Book.*Test Author"; check "Everything lists the page, chapter page, percent, time left, the author and the title in one line" $?
    sed -n 3p <<<"$L" | grep -q "this session  ·  .* today"; check "and the session time and today reading" $?
    sed -n 3p <<<"$L" | grep -q "items=page,chapter_page,percent,left_chapter,left_book,speed,clock,battery,chapter,title,author,bar,chapters,session,today"; check "in the order of the list" $?
    sed -n 4p <<<"$L" | grep -qE "^footer:  \| .*[0-9]+ of [0-9]+.*bar=true"; check "items can be switched off and moved (the page moved behind others)" $?
    ! sed -n 4p <<<"$L" | grep -q "Test Author"; check "an item that was switched off is gone" $?
    sed -n 5p <<<"$L" | grep -q "own=true .* align=Centered"; check "a footer of this book only can differ from the common one" $?
    sed -n 6p <<<"$L" | grep -q "own=false .* align=Centered"; check "taking the book-only footer away brings the common one back" $?
    sed -n 7p <<<"$L" | grep -qE "^footer: [^|]+ \|  \| .* align=Spread"; check "the left and right position can still be chosen" $?
    grep -q "\"footer\"" "$d/cfg/bookx/config.json" && grep -q "\"footer_books\"" "$d/cfg/bookx/config.json"; check "the footer settings are saved in the config" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario footer_popover 'read:1,wait:2500,reader:footer-open,wait:700,a11y-audit:,reader:footer-open,quit' '
    has "^a11y: controls="; check "the footer popover was audited" $?
    ! grep "^a11y-problem:" <<<"$OUT" | grep -v "AdwActionRow.header" | grep -q .; check "its controls have accessible names" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# ---- item 10: the selection handles ----
scenario selection_handles 'read:1,wait:2500,reader:ui-dblclick,wait:700,reader:handles,reader:handle-drag=end:70:0,wait:600,reader:handles,reader:handle-drag=start:25:0,wait:600,reader:handles,reader:handle-drag=start:400:0,wait:600,reader:handles,reader:handle-clear,wait:300,reader:handles,quit' '
    H=$(grep "^handles: " <<<"$OUT")
    sed -n 1p <<<"$H" | grep -q "^handles: n=2 .* popover=true"; check "a double click selects a word and shows two handles with the action popover" $?
    len() { sed -E "s/.* sel=\"(.*)\"$/\1/" <<<"$1" | wc -c; }
    A=$(len "$(sed -n 1p <<<"$H")"); B=$(len "$(sed -n 2p <<<"$H")"); C=$(len "$(sed -n 3p <<<"$H")")
    [ "$B" -gt "$A" ]; check "dragging the end handle to the right makes the selection longer" $?
    has "^handle_drag: true"; check "the handle was picked up" $?
    sed -n 2p <<<"$H" | grep -q "popover=true"; check "the action popover is back near the selection after the drag" $?
    [ "$C" -lt "$B" ]; check "dragging the start handle to the right makes the selection shorter" $?
    sed -n 4p <<<"$H" | grep -q "^handles: n=2 "; check "a start handle dragged past the end swaps the two ends (still a selection with two handles)" $?
    sed -n 5p <<<"$H" | grep -q "^handles: n=0 .* sel=\"\""; check "the handles go when the selection is cleared" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario selection_handles_columns 'read:1,wait:2500,reader:two,wait:1500,reader:ui-dblclick,wait:700,reader:handles,reader:handle-drag=end:40:0,wait:600,reader:handles,quit' '
    H=$(grep "^handles: " <<<"$OUT")
    sed -n 1p <<<"$H" | grep -q "^handles: n=2 "; check "two columns: a selected word has two handles" $?
    sed -n 2p <<<"$H" | grep -q "moves=[1-9]"; check "and a handle can be dragged" $?'

scenario selection_handles_vertical "library:import=$ROOT/vertical-epub/lib,wait:2500,read:3,wait:3500,reader:ui-vsel:5:60,reader:handles,reader:handle-drag=end:0:60,wait:600,reader:handles,reader:handle-drag=start:0:-20,wait:600,reader:handles,quit" '
    H=$(grep "^handles: " <<<"$OUT")
    sed -n 1p <<<"$H" | grep -q "^handles: n=2 "; check "vertical text: a selection has two handles" $?
    len() { sed -E "s/.* sel=\"(.*)\"$/\1/" <<<"$1" | wc -c; }
    [ "$(len "$(sed -n 2p <<<"$H")")" -gt "$(len "$(sed -n 1p <<<"$H")")" ]; check "dragging the end handle down the column makes the selection longer" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# ---- item 16: the Wikipedia picture ----
scenario wiki_image 'read:1,wait:2500,reader:ui-dblclick,wait:800,reader:ui-wiki-btn,wait:2500,reader:dump,quit' '
    has "^wiki: <b>Mock Article</b>"; check "the summary shows" $?
    has "^wiki_image: frame=true shows=picture size=48x32"; check "the lead picture of the article shows above the text" $?
    [ -n "$(ls "$d/cache/bookx/wiki-images" 2>/dev/null)" ]; check "the picture is kept in the cache folder" $?
    has "^dict: wiki "; check "the Wikipedia page is still showing (the popover did not close)" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario wiki_image_cached 'read:1,wait:2500,reader:ui-dblclick,wait:800,reader:ui-wiki-btn,wait:1500,reader:dump,quit' '
    has "^wiki_image: frame=true shows=picture size=48x32"; check "offline, a picture that was fetched before still shows from the cache" $?'

scenario wiki_image_offline 'read:1,wait:2500,reader:ui-dblclick,wait:800,reader:ui-wiki-btn,wait:1500,reader:dump,quit' '
    has "^wiki: <b>Mock Article</b>"; check "offline without a cached picture the text still shows" $?
    has "^wiki_image: frame=false "; check "and the picture frame is gone, no broken placeholder" $?
    [ -z "$(ls "$d/cache/bookx/wiki-images" 2>/dev/null)" ]; check "nothing was downloaded" $?'

scenario wiki_image_missing 'read:1,wait:2500,reader:ui-dblclick,wait:800,reader:ui-wiki-btn,wait:2500,reader:dump,quit' '
    has "^wiki: <b>Mock Article</b>"; check "a picture that fails to load leaves the text" $?
    has "^wiki_image: frame=false "; check "and takes its frame away" $?'

# ---- Preferences → Reading → Footer Stats: edits apply to the open reader at once ----
scenario footer_live 'read:1,wait:2500,reader:footer,wait:300,prefs:,wait:900,prefs:footer-toggle=left_chapter,wait:300,reader:footer,wait:300,prefs:footer-align=spread,wait:300,reader:footer,wait:300,prefs:footer-align=centered,prefs:footer-preset=everything,wait:300,reader:footer,wait:300,prefs:footer-size=130,wait:300,reader:footer,wait:300,reader:footer-own=on,wait:200,prefs:footer-preset=minimal,wait:300,reader:footer,wait:300,prefs:icons,quit' '
    L=$(grep "^footer: " <<<"$OUT")
    sed -n 1p <<<"$L" | grep -q "minutes to complete"; check "the footer starts with the time left in the chapter" $?
    ! sed -n 2p <<<"$L" | grep -q "minutes to complete"; check "switching that item off in Preferences changes the open reader at once" $?
    sed -n 2p <<<"$L" | grep -qE "^footer:  \| [0-9]+ of [0-9]+  ·  Chapter 1 \|"; check "the other items stay, in the middle" $?
    sed -n 3p <<<"$L" | grep -qE "^footer: [0-9]+ of [0-9]+ \|  \| Chapter 1 .*align=Spread"; check "choosing left and right in Preferences moves the footer live" $?
    sed -n 4p <<<"$L" | grep -q "Test Author"; check "a preset chosen in Preferences shows live (Everything)" $?
    sed -n 4p <<<"$L" | grep -q "align=Centered"; check "and the position is the middle again" $?
    sed -n 5p <<<"$L" | grep -q "size=130"; check "the text size changes live" $?
    sed -n 6p <<<"$L" | grep -q "Test Author.* own=true"; check "a book with a footer of its own keeps it when the common one changes" $?
    grep -q "\"footer\"" "$d/cfg/bookx/config.json" && ! grep -q "\"items\"[^}]*\"minimal\"" "$d/cfg/bookx/config.json"; check "the common footer is saved" $?
    has "^prefs_icons: checked=[0-9]+ .*missing=\[\]"; check "every section of the Preferences sidebar (Reading included) has an icon that exists in the theme" $?
    grep "^prefs_icons:" <<<"$OUT" | grep -q "Reading"; check "the Reading section is in the walk" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# ---- the selection popover never stays over a dialog or window (overlay_guard.rs) ----
# It is not closed by clicks away (so the page can extend the selection), which also meant that nothing closed it
# when a dialog came over the reader. Now it closes when a dialog, window or menu comes up; the selection stays,
# the popover stays closed (a new double click or Enter on the page shows it again).
scenario selection_popover_dialogs 'read:1,wait:2500,reader:ui-dblclick,wait:1200,reader:handles,wait:300,prefs:,wait:900,reader:handles,wait:300,dialog:close,wait:1000,reader:handles,wait:300,reader:ui-dblclick,wait:1200,reader:handles,wait:300,reader:properties,wait:900,reader:handles,wait:300,dialog:close,wait:1000,reader:ui-dblclick,wait:1200,reader:handles,wait:300,reader:map,wait:3000,reader:handles,wait:300,dialog:close,wait:1000,reader:ui-dblclick,wait:1200,reader:handles,wait:300,action:shortcuts,wait:1500,reader:handles,wait:300,dialog:close,wait:1000,reader:ui-dblclick,wait:1200,reader:handles,wait:300,reader:footer-open,wait:900,reader:handles,wait:300,quit' '
    H=$(grep "^handles: " <<<"$OUT")
    pop() { sed -n "$1p" <<<"$H" | sed -E "s/.* popover=([a-z]+) .*/\1/"; }
    [ "$(pop 1)" = true ]; check "a double click selects a word and shows the action popover" $?
    [ "$(pop 2)" = false ]; check "Preferences opens: the selection popover is gone, not drawn over it" $?
    sed -n 2p <<<"$H" | grep -qv "sel=\"\""; check "the selection itself is still there" $?
    [ "$(pop 3)" = false ]; check "after Preferences closes the popover stays closed (documented: it does not come back by itself)" $?
    [ "$(pop 4)" = true ]; check "a new double click shows it again" $?
    [ "$(pop 5)" = false ]; check "Properties (Ctrl+I): the popover is gone" $?
    [ "$(pop 6)" = true ]; check "and comes back with the next selection" $?
    [ "$(pop 7)" = false ]; check "the book map: the popover is gone" $?
    [ "$(pop 8)" = true ]; check "and comes back with the next selection" $?
    [ "$(pop 9)" = false ]; check "the keyboard shortcuts window: the popover is gone" $?
    [ "$(pop 10)" = true ]; check "and comes back with the next selection" $?
    [ "$(pop 11)" = false ]; check "another popover (the footer settings) opens: the selection popover gives way" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'
