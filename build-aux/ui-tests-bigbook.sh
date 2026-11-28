# Scenarios of a 10 MB EPUB with 20 000 contents entries (QA round 2, docs/archive/qa-wave5.md "Main-thread stalls"):
# opening it in the reader and in the editor must not freeze the window. Sourced by ui-tests.sh (uses its
# `scenario`, `check`, `has`, $ROOT, $HERE). `BOOKX_STALL_WATCH` (set by ui-tests.sh for big_book_*) prints
# `stall: N ms after <command>` for every stretch in which the main loop did not run. The budgets are for the
# debug build the suite runs; STALL_BUDGET_MS scales them (release numbers: docs/performance-notes.md).
mkdir -p "$ROOT/big20k"
python3 "$HERE/make-big-epub.py" "$ROOT/big20k/big.epub" 20000 >/dev/null

# the longest stall after the given dev command (0 when none was reported)
longest_stall() { # $1 = command prefix
    grep -E "^stall: [0-9]+ ms after $1" <<<"$OUT" | sed -E 's/^stall: ([0-9]+) ms.*/\1/' | sort -n | tail -1 | grep . || echo 0
}

scenario big_book_reader 'wait:500,read:1,wait:9000,reader:side-state,quit' '
    has "^side: .* toc=20[0-9][0-9][0-9] "; check "the reader has all 20 000+ contents entries in its model" $?
    L=$(longest_stall "read:1")
    [ "$L" -le "${STALL_BUDGET_MS:-4000}" ]; check "opening the 10 MB book blocks the main loop for at most ${STALL_BUDGET_MS:-4000} ms in a debug build (${L} ms; it was 1133 ms in release before the fix of the paragraph measuring)" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# X18: opening a long chapter at a saved position in the middle lays out a window around it first (the page is
# there at once, the page count is an estimate), the chapter follows in slices and replaces the estimate; a page turn
# before that completes the layout first and carries on from the exact position
scenario big_book_window 'wait:500,read:1,wait:9000,reader:goto-frac=5:0.6,wait:200,reader:layout-state,wait:200,reader:layout-state,wait:200,reader:layout-state,wait:200,reader:layout-state,wait:200,reader:layout-state,wait:200,reader:layout-state,wait:200,reader:layout-state,wait:200,reader:layout-state,wait:200,reader:layout-state,wait:200,reader:layout-state,wait:200,reader:layout-state,wait:200,reader:layout-state,wait:20000,reader:layout-state,reader:goto-frac=6:0.3,wait:150,reader:next,reader:layout-state,wait:3000,reader:layout-state,quit' '
    W=$(grep "^layout: complete=false shown=true window=true" <<<"$OUT" | head -1)
    [ -n "$W" ]; check "the chapter is shown from a window while the rest is still being laid out: $W" $?
    sed -E "s/.* frac=([0-9.]+) .*/\\1/" <<<"$W" | awk "{exit !(\$1 >= 0.58 && \$1 <= 0.62)}"; check "the window is at the saved position (60 % of the chapter, as far as the estimate goes)" $?
    N=$(grep "^layout: " <<<"$OUT" | grep -c "window=true")
    [ "$N" -ge 1 ]; check "the window was seen ${N} times before the layout completed" $?
    E=$(grep "^layout: complete=true shown=true window=false frac=0.59[0-9]" <<<"$OUT" | head -1)
    [ -n "$E" ]; check "when the layout is complete the position is the exact 60 %: $E" $?
    M=$(grep "^layout: " <<<"$OUT" | tail -2 | head -1)
    grep -q "complete=true" <<<"$M" && sed -E "s/.* frac=([0-9.]+) .*/\\1/" <<<"$M" | awk "{exit !(\$1 > 0.295 && \$1 < 0.33)}"; check "a page turn at once completes the layout and moves on from 30 %: $M" $?
    L=$(longest_stall "reader:goto-frac")
    [ "$L" -le "${STALL_BUDGET_MS:-4000}" ]; check "opening the chapter in the middle blocks the main loop for at most ${STALL_BUDGET_MS:-4000} ms in a debug build (${L} ms)" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario big_book_editor 'wait:500,edit:1,wait:12000,editor:toc-rows,editor:syntax-state,editor:toc-row=7,wait:800,editor:toc-dump,editor:toc-rows,editor:open=OEBPS/nav.xhtml,wait:3000,editor:syntax-state,editor:open=OEBPS/part2.xhtml,wait:2500,editor:syntax-state,editor:longline=reformat,wait:6000,editor:syntax-state,editor:toc-rows,quit' '
    R=$(grep "^editor_toc_rows:" <<<"$OUT" | head -1)
    grep -qE "entries=20[0-9]{3} model=20[0-9]{3} " <<<"$R"; check "the Contents list has all 20 000+ entries in its model: $R" $?
    W=$(sed -E "s/.* widgets=([0-9]+) .*/\1/" <<<"$R")
    [ -n "$W" ] && [ "$W" -lt 400 ]; check "but only the rows in view have widgets (${W}), not 20 000" $?
    grep "^editor_toc_rows:" <<<"$OUT" | sed -n 2p | grep -q "selected=7 "; check "selecting an entry selects its row in the virtual list" $?
    has "^editor_toc: entries=20[0-9]+ dirty=false selected=Some\(7\)"; check "selecting an entry changes nothing in the book" $?
    S=$(grep "^editor_syntax:" <<<"$OUT")
    sed -n 1p <<<"$S" | grep -q "page=longline .*plain=true"; rc=$?; check "a chapter that is one 450 KB line is not put into the text view, the page that offers line breaks is shown: $(sed -n 1p <<<"$S")" $rc
    sed -n 2p <<<"$S" | grep -q "file=OEBPS/nav.xhtml page=longline .*plain=true"; check "neither is the 1.2 MB navigation document" $?
    sed -n 3p <<<"$S" | grep -q "file=OEBPS/part2.xhtml page=longline"; check "nor the third chapter" $?
    sed -n 4p <<<"$S" | grep -q "file=OEBPS/part2.xhtml page=code .*language=true highlight=true plain=false"; rc=$?; check "after Add Line Breaks the file opens in the text view, coloured again: $(sed -n 4p <<<"$S")" $rc
    sed -n 4p <<<"$S" | grep -qE "chars=[0-9]{6}"; check "the text kept its content (only white space between tags was added)" $?
    L=$(longest_stall "edit:1")
    [ "$L" -le "${STALL_BUDGET_MS:-4000}" ]; check "opening the book in the editor blocks the main loop for at most ${STALL_BUDGET_MS:-4000} ms in a debug build (${L} ms; it was 2590 ms in release and 7.5 s in all at first)" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# a normal chapter keeps its colours: the small book of the suite
scenario editor_syntax_normal_file 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:500,editor:syntax-state,quit' '
    has "^editor_syntax: file=OEBPS/chapter-1.xhtml .* language=true highlight=true plain=false"; check "an ordinary file is still coloured by GtkSourceView" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'
