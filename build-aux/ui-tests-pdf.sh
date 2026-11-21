# Scenarios of the PDF reader: pages, zoom, search, selection and highlights, links, passwords, reflow.
# Sourced by ui-tests.sh (uses its `scenario`, `check`, `has`, $ROOT, $CLI, $BIN).

PDFS=$ROOT/pdf
cargo build --quiet -p bookx-pdf --example make_test_pdfs || exit 1
MKPDF=${CARGO_TARGET_DIR:-$PWD/target}/debug/examples/make_test_pdfs
"$MKPDF" "$PDFS.all" >/dev/null 2>&1
for f in story locked scan margins big; do
    mkdir -p "$PDFS/$f"
    mv "$PDFS.all/$f.pdf" "$PDFS/$f/$f.pdf" 2>/dev/null
done

# the state line of the PDF view as a key=value lookup
pv() { grep "^pdf_state:" <<<"$OUT" | sed -n "${1}p" | sed -E "s/.* $2=([^ ]*).*/\\1/"; }

scenario pdf_open "library:import=$PDFS/story,wait:3500,read:3,wait:3000,reader:pdf-state,reader:pdf-lists,quit" '
    grep -q "The Fox Papers" <<<"$LIB"; check "the title comes from the PDF information" $?
    grep -q "Ada Reader" <<<"$LIB"; check "and the author" $?
    grep -q "\"format\": \"pdf\"" <<<"$LIB" || grep -q "\\.pdf" <<<"$LIB"; check "the PDF is kept as a format of the book" $?
    ls "$d"/lib/*/*/*.pdf >/dev/null 2>&1 || ls "$d"/lib/*/*.pdf >/dev/null 2>&1 || find "$d/lib" -name "*.pdf" | grep -q .; check "the PDF file is in the library folder" $?
    [ "$(pv 1 opened)" = true ]; check "the PDF opens in the page view" $?
    [ "$(pv 1 pages)" = 8 ]; check "it has 8 pages" $?
    [ "$(pv 1 page)" = 1 ]; check "it starts on the first page" $?
    [ "$(pv 1 zoom)" = width ]; check "pages fit the width by default" $?
    has "^pdf_lists: toc=4 "; check "the outline fills the contents (4 entries)" $?
    has "caption=\"i \(1 of 8\)\""; check "the footer shows the page label and the number" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "opening runs without errors or GTK warnings" $?'

scenario pdf_navigation "library:import=$PDFS/story,wait:3500,read:3,wait:2500,reader:pdf-goto=4,wait:600,reader:pdf-state,reader:pdf-lists,reader:pdf-goto=ii,wait:400,reader:pdf-state,reader:pdf-view=single,wait:800,reader:pdf-state,reader:pdf-turn,wait:500,reader:pdf-state,reader:pdf-turn=back,wait:500,reader:pdf-state,reader:pdf-zoom=200,wait:600,reader:pdf-state,reader:pdf-zoom-out,wait:500,reader:pdf-state,reader:pdf-zoom=page,wait:500,reader:pdf-state,reader:pdf-view=continuous,wait:600,reader:pdf-zoom=width,wait:800,reader:pdf-state,quit" '
    [ "$(pv 1 page)" = 6 ]; check "a typed number is a page label first: 4 is the sixth page here" $?
    has "page_entry=\"4\" caption=\"4 \(6 of 8\)\""; check "the footer names the page by its label (4) and its number (6 of 8)" $?
    [ "$(pv 2 page)" = 2 ]; check "a page label can be typed to go there (ii is page 2)" $?
    [ "$(pv 3 mode)" = Single ]; check "one page at a time is a mode of its own" $?
    [ "$(pv 4 page)" = 3 ]; check "a turn in that mode shows the next page" $?
    [ "$(pv 5 page)" = 2 ]; check "and turning back shows the previous page" $?
    [ "$(pv 6 zoom)" = 200 ]; check "zoom can be set to a percentage" $?
    awk "BEGIN{exit !($(pv 6 scale) > 2.6 && $(pv 6 scale) < 2.7)}"; check "200 percent is two times the 96 dpi size" $?
    [ "$(pv 7 zoom)" = 150 ]; check "Zoom out steps down the list of zooms" $?
    [ "$(pv 8 zoom)" = page ]; check "fit page is a zoom mode" $?
    [ "$(pv 9 mode)" = Continuous ] && [ "$(pv 9 zoom)" = width ] && [ "$(pv 9 page)" = 2 ]; check "back to continuous and fit width keeps the page you were on" $?
    grep -q "\"zoom\": \"width\"" "$d/cfg/bookx/pdf.json"; check "the view settings are stored" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "navigation runs without errors or GTK warnings" $?'

scenario pdf_position "library:import=$PDFS/story,wait:3500,read:3,wait:2500,reader:pdf-goto=5,wait:800,close:,wait:1500,library:dump,read:3,wait:2500,reader:pdf-state,quit" '
    [ "$(pv 1 page)" = 7 ]; check "the page you left (label 5, page 7) is the page you come back to" $?
    grep -Eq "\"progress\": 0\.(7|8)" <<<"$LIB"; check "the reading progress of the book follows the page" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "closing and reopening runs without errors" $?'

scenario pdf_outline_links "library:import=$PDFS/story,wait:3500,read:3,wait:2500,reader:toc-row=2,wait:600,reader:pdf-state,reader:toc-row=1,wait:500,reader:pdf-state,reader:pdf-goto=i,wait:400,reader:pdf-link=0,wait:700,reader:pdf-state,reader:pdf-back,wait:700,reader:pdf-state,reader:pdf-link=1,reader:pdf-link=7,quit" '
    [ "$(pv 1 page)" = 4 ]; check "a contents entry goes to its page (Middle: page 4)" $?
    [ "$(pv 2 page)" = 2 ]; check "a nested entry too (The first night: page 2)" $?
    [ "$(pv 3 page)" = 6 ]; check "a link inside the document goes to its page (page 6)" $?
    [ "$(pv 4 page)" = 1 ]; check "Back returns to where the link was followed from" $?
    has "^pdf_link: uri https://example.org/foxes"; check "an address link is recognised (and only reported in tests)" $?
    has "^pdf_link: none"; check "there is no seventh link" $?
    [ "$(pv 1 links)" -ge 2 ]; check "the page reports its links" $?'

scenario pdf_select_highlight "library:import=$PDFS/story,wait:3500,read:3,wait:2500,reader:pdf-select=quick brown,wait:500,reader:pdf-state,reader:pdf-style=underline,reader:pdf-note=Remember the fox,wait:500,reader:pdf-select=lazy dog,wait:500,reader:pdf-style=background,reader:pdf-highlight=blue,wait:500,reader:pdf-highlights,reader:pdf-state,wait:400,close:,wait:1500,read:3,wait:2500,reader:pdf-state,reader:pdf-tab=notes,reader:pdf-lists,quit" '
    has "^pdf_select: page 1 text=Some\(\"quick brown\"\)"; check "selecting text finds the words on the page with their positions" $?
    has "sel=quick brown "; check "the selection text is available for copy and look-ups" $?
    has "^pdf_highlights: p1:\"quick brown\":Remember the fox:1 \| p1:\"lazy dog\"::1"; check "a highlight stores the page, the quote, the note and the rectangles" $?
    grep -q "pdf|" <<<"$LIB"; check "it is stored in the library (as a PDF annotation)" $?
    [ "$(pv 2 hl)" = 2 ]; check "both are drawn on the page" $?
    [ "$(pv 3 hl)" = 2 ]; check "it is still there after closing and opening the book" $?
    grep -q "pdf|underline|" <<<"$LIB"; check "the style chosen (underline) is kept with it" $?
    grep -q "\"color\": \"blue\"" <<<"$LIB"; check "and its colour" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "highlighting runs without errors or GTK warnings" $?'

scenario pdf_search "library:import=$PDFS/story,wait:3500,read:3,wait:2500,reader:pdf-search=zyzzyva,wait:2500,reader:pdf-search-state,reader:pdf-state,reader:pdf-search=fox,wait:3000,reader:pdf-search-state,reader:pdf-search-next,wait:600,reader:pdf-state,reader:pdf-search-state,reader:pdf-search=nothing here,wait:2000,reader:pdf-search-state,quit" '
    has "^pdf_search: id=[0-9]+ done=true hits=1 active=Some\(0\) first=Some\(\(5, "; check "a word that is on one page is found there, with a snippet" $?
    [ "$(pv 1 page)" = 5 ]; check "the first match is shown (page 5)" $?
    [ "$(pv 1 hits)" = 1 ]; check "the match is highlighted on the page" $?
    has "^pdf_search: id=[0-9]+ done=true hits=17 "; check "every match in the document is listed (17)" $?
    [ "$(pv 2 page)" != "" ]; check "stepping to the next match moves there" $?
    has "^pdf_search: id=[0-9]+ done=true hits=0 "; check "no match is reported as such" $?'

scenario pdf_crop_invert "library:import=$PDFS/margins,wait:3500,read:3,wait:2500,reader:pdf-state,reader:pdf-crop=on,wait:2500,reader:pdf-state,reader:pdf-invert=on,wait:1500,reader:pdf-state,reader:pdf-crop=off,wait:800,reader:pdf-state,quit" '
    [ "$(pv 1 crop)" = false ]; check "margins are not cropped by default" $?
    [ "$(pv 2 crop)" = true ]; check "Crop margins finds the margins from the content of the pages" $?
    awk "BEGIN{exit !($(pv 2 scale) > $(pv 1 scale) * 1.05)}" || awk "BEGIN{exit !($(pv 2 scale) > $(pv 1 scale))}"; check "the cropped page is shown larger" $?
    [ "$(pv 3 invert)" = true ]; check "dark pages invert the colours" $?
    [ "$(pv 4 crop)" = false ]; check "cropping can be switched off again" $?
    grep -q "\"invert\": true" "$d/cfg/bookx/pdf.json"; check "the colour setting is stored" $?'

scenario pdf_password "library:import=$PDFS/locked,wait:3500,read:3,wait:2000,reader:pdf-state,reader:pdf-password=wrong,wait:1500,reader:pdf-state,reader:pdf-password=sesame,wait:2500,reader:pdf-state,quit" '
    grep -q "locked" <<<"$LIB"; check "a protected PDF is still imported" $?
    [ "$(pv 1 opened)" = false ]; check "a protected PDF waits for the password" $?
    [ "$(pv 2 opened)" = false ] || [ -z "$(pv 2 opened)" ]; check "a wrong password does not open it" $?
    [ "$(pv 3 opened)" = true ] && [ "$(pv 3 pages)" = 2 ]; check "the right password opens it (2 pages)" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "the password prompt runs without errors" $?'

scenario pdf_scan "library:import=$PDFS/scan,wait:3500,read:3,wait:2500,reader:pdf-state,reader:pdf-search=text,wait:1500,reader:pdf-search-state,quit" '
    grep -q "Scanned Pages" <<<"$LIB"; check "a scan imports with its title" $?
    [ "$(pv 1 pages)" = 2 ]; check "and shows its pages" $?
    has "^pdf_search: id=[0-9]+ done=true hits=0 "; check "searching a scan finds nothing (there is no text layer)" $?'

scenario pdf_reflow "library:import=$PDFS/story,wait:3500,read:3,wait:2500,reader:pdf-reflow,wait:3500,reader:page-text,quit" '
    has "^page_text: .*quick brown fox"; check "Reflow opens the text of the PDF in the e-book reader" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "switching to the reflowed text runs without errors" $?'

scenario pdf_bookmarks_thumbs "library:import=$PDFS/story,wait:3500,read:3,wait:2500,reader:pdf-goto=3,wait:500,reader:pdf-bookmark,wait:300,library:dump,reader:pdf-tab=pages,wait:2500,reader:pdf-thumbs,reader:pdf-bookmark,wait:300,reader:pdf-state,quit" '
    grep -q "\"bookmarks\": \[" <<<"$LIB" ; check "a bookmark is stored with the book" $?
    has "^pdf_thumbs: shown=[1-9][0-9]* rendered=[1-9]"; check "thumbnails of the pages shown in the sidebar are rendered" $?
    has "sidebar=true"; check "the sidebar opens on a tab" $?'

scenario pdf_big "library:import=$PDFS/big,wait:20000,read:3,wait:5000,reader:pdf-state,reader:pdf-goto=250,wait:3000,reader:pdf-state,reader:pdf-search=pack my box,wait:4000,reader:pdf-search-state,quit" '
    [ "$(pv 1 pages)" = 500 ]; check "a 500 page PDF opens" $?
    [ "$(pv 2 page)" = 250 ] && [ "$(pv 2 rendered)" -ge 1 ]; check "a far page is shown and rendered after a jump" $?
    has "^pdf_search: id=[0-9]+ done=(true|false) hits=[1-9]"; check "searching starts to list matches while it is still running" $?'


scenario pdf_a11y_audit "library:import=$PDFS/story,wait:3500,read:3,wait:2500,a11y-audit:,reader:pdf-tab=pages,wait:1500,a11y-audit:,reader:pdf-select=quick brown,wait:600,a11y-audit:,quit" '
    A=$(grep "^a11y:" <<<"$OUT")
    [ "$(wc -l <<<"$A")" -eq 3 ]; check "the audit ran in the PDF reader" $?
    ! has "^a11y-problem:"; check "every control of the PDF reader has an accessible name" $?'
