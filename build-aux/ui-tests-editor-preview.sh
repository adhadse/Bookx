# Scenarios of the editor preview sync, splitting, compare view, checkpoints dialog and Live CSS.
# Sourced by ui-tests.sh (uses its `scenario`, `check`, `has`, $ROOT).

scenario editor_pv_sync 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:800,editor:pv-state,editor:find-in-text=Paragraph 20 of,wait:500,editor:pv-where,editor:pv-follow=off,editor:pv-scroll-top,wait:300,editor:find-in-text=Paragraph 12 of,wait:500,editor:pv-where,editor:pv-follow=on,wait:500,editor:pv-where,editor:pv-scroll-top,wait:300,editor:pv-click-block=3,wait:500,editor:pv-where,editor:pv-find=Paragraph 7,editor:pv-copy,wait:300,editor:pv-link=OEBPS/chapter-2.xhtml,wait:700,editor:pv-state,editor:pv-link=https://example.org/x,wait:300,editor:dump,quit' '
    P=$(grep "^editor_pv:" <<<"$OUT")
    sed -n 1p <<<"$P" | grep -q "blocks=26 mapped=26 file=Some(\"OEBPS/chapter-1.xhtml\")"; check "every block of the preview is mapped to its source" $?
    sed -n 2p <<<"$P" | grep -q "caret_block=Some(20) .*visible=true"; check "the preview scrolls to the element at the cursor" $?
    sed -n 3p <<<"$P" | grep -q "caret_block=Some(12) .*visible=false"; check "with Follow cursor off the preview stays where it is" $?
    sed -n 4p <<<"$P" | grep -q "caret_block=Some(12) .*visible=true"; check "switching Follow cursor on brings the element into view" $?
    sed -n 5p <<<"$P" | grep -q "click block=3 hit=Some(3)"; check "the hit test finds the block under a point of the preview" $?
    sed -n 6p <<<"$P" | grep -q "caret_block=Some(3) .*caret_text=\"<p>Paragraph 3 of chapter "; check "a click in the preview puts the cursor at the element in the code view" $?
    sed -n 6p <<<"$P" | grep -q "visible=true"; check "the preview does not jump away after its own click" $?
    has "^editor_pv: find \"Paragraph 7\" hits=1$"; check "find in the preview counts its hits" $?
    has "^editor_pv: copy \"Chapter 1"; check "the whole preview text can be selected and copied" $?
    grep "^editor_pv: follow" <<<"$OUT" | tail -1 | grep -q "file=Some(\"OEBPS/chapter-2.xhtml\")"; check "a click on an in-book link opens that file in the editor and preview" $?
    has "^editor: .*toast=\"External link: https://example.org/x\""; check "an external link is only named, not followed" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "the preview sync runs without errors or GTK warnings" $?'

scenario editor_pv_reload 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:800,editor:pv-auto=off,editor:find-in-text=<p>Paragraph 3 of,editor:insert=<p>Extra block</p>,wait:1000,editor:pv-state,editor:pv-refresh,wait:300,editor:pv-state,editor:pv-auto=on,editor:insert=<p>Another block</p>,wait:1300,editor:pv-state,quit' '
    P=$(grep "^editor_pv: follow" <<<"$OUT")
    sed -n 1p <<<"$P" | grep -q "auto=false stale=true blocks=26"; check "with Auto reload off typing marks the preview stale and leaves it alone" $?
    sed -n 2p <<<"$P" | grep -q "stale=false blocks=27"; check "Refresh (F5) redraws the preview from the buffer" $?
    sed -n 3p <<<"$P" | grep -q "auto=true stale=false blocks=28"; check "with Auto reload on the preview follows typing" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "reloading runs without errors" $?'

scenario editor_pv_pick 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:800,editor:pv-toc,editor:pv-pick-toc=1,wait:400,editor:pv-state,editor:pv-scroll-top,wait:300,editor:pv-hover=4,editor:pv-click-block=4,wait:600,editor:pv-state,editor:pv-toc,editor:has=<p id="toc2">Paragraph 4 of,editor:pv-pick-split,wait:300,editor:pv-state,editor:pv-pick-cancel,wait:300,editor:pv-state,quit' '
    has "^editor_pv_toc: Chapter 1=OEBPS/chapter-1.xhtml\|Chapter 2=OEBPS/chapter-2.xhtml\|"; check "the Contents point at whole files to begin with" $?
    P=$(grep "^editor_pv: follow" <<<"$OUT")
    sed -n 1p <<<"$P" | grep -q "pick=Some(Toc(1)) .*banner=true"; check "Pick in the preview waits for a click and says so" $?
    has "^editor_pv: marker visible=true top=[0-9]+"; check "the green marker line follows the pointer over the preview" $?
    sed -n 2p <<<"$P" | grep -q "pick=None marker=false banner=false"; check "the click ends the pick mode and hides the marker" $?
    grep "^editor_pv_toc:" <<<"$OUT" | tail -1 | grep -q "Chapter 2=OEBPS/chapter-1.xhtml#toc2|"; check "the Contents entry now points at the clicked place" $?
    has "^editor_has: <p id=\"toc2\">Paragraph 4 of = 1"; check "an id was added to the element the entry points at" $?
    sed -n 3p <<<"$P" | grep -q "pick=Some(Split)"; check "Split by picking uses the same mode" $?
    sed -n 4p <<<"$P" | grep -q "pick=None"; check "Cancel leaves the pick mode" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "picking runs without errors" $?'

scenario editor_split_cursor 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:800,editor:find-in-text=Paragraph 13 of,editor:pv-split-cursor,wait:900,editor:dump,editor:cat=OEBPS/chapter-1.xhtml,editor:cat=OEBPS/chapter-1_split_001.xhtml,editor:cps,editor:tabs,quit' '
    D=$(grep "^editor:" <<<"$OUT"); C=$(grep "^editor_cat:" <<<"$OUT")
    sed -n 1p <<<"$D" | grep -q "toast=\"Split into 2 files"; check "Split at Cursor splits the file in two" $?
    sed -n 1p <<<"$D" | grep -q "checkpoints=1"; check "a checkpoint is taken before the split" $?
    sed -n 1p <<<"$C" | grep -q "Paragraph 12 of chapter 1" && ! sed -n 1p <<<"$C" | grep -q "Paragraph 13 of"; check "the first part ends before the cursor" $?
    sed -n 2p <<<"$C" | grep -q "<p>Paragraph 13 of chapter 1" && ! sed -n 2p <<<"$C" | grep -q "<p></p>"; check "the second part starts at the paragraph under the cursor, without an empty paragraph left behind" $?
    sed -n 1p <<<"$C" | grep -q "</body>" && sed -n 2p <<<"$C" | grep -q "<title>Chapter 1</title>"; check "both parts are complete documents" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "splitting runs without errors" $?'

scenario editor_split_pick 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:800,editor:pv-pick-split,wait:300,editor:pv-scroll-top,wait:300,editor:pv-click-block=4,wait:900,editor:dump,editor:cat=OEBPS/chapter-1_split_001.xhtml,quit' '
    has "^editor: .*toast=\"Split into 2 files"; check "clicking in the preview in split mode splits the file there" $?
    grep "^editor_cat:" <<<"$OUT" | grep -q "<body>.*<p>Paragraph 4 of chapter 1"; check "the new file starts with the clicked block" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "split mode runs without errors" $?'

scenario editor_split_dialog 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:800,editor:addheading,editor:pv-split=preview:headings:1,editor:pv-split=preview:selector://h1,editor:pv-split=preview:class:p.nomatch,editor:pv-split=preview:selector://h1[,editor:pv-dialog=0|1,wait:300,editor:pv-dialog-status,editor:pv-dialog=3||||//h1,wait:300,editor:pv-dialog-status,editor:pv-dialog=1|1|p|nomatch|,wait:300,editor:pv-dialog-status,editor:pv-dialog=0|1,wait:300,editor:pv-dialog-go,wait:900,editor:dump,editor:cat=OEBPS/chapter-1_split_001.xhtml,editor:tabs,quit' '
    S=$(grep "^editor_split:" <<<"$OUT")
    sed -n 1p <<<"$S" | grep -q "preview parts=2 titles=Chapter 1|Second Part"; check "the preview names the files a heading split would make" $?
    sed -n 2p <<<"$S" | grep -q "preview parts=2"; check "a selector like //h1 finds the same places" $?
    sed -n 3p <<<"$S" | grep -q "preview error=Nothing to split"; check "a class that does not occur says nothing would be split" $?
    sed -n 4p <<<"$S" | grep -q "preview error="; check "a broken selector is explained, not ignored" $?
    sed -n 5p <<<"$S" | grep -q "dialog status=\"2 files\" rows=2 split_enabled=true"; check "the dialog lists the resulting files" $?
    sed -n 6p <<<"$S" | grep -q "status=\"2 files\" rows=2 split_enabled=true"; check "the dialog accepts a selector" $?
    sed -n 7p <<<"$S" | grep -q "status=\"Nothing to split.*rows=0 split_enabled=false"; check "Split stays off while nothing would be split" $?
    has "^editor: .*toast=\"Split into 2 files"; check "Split in the dialog splits the file" $?
    grep "^editor_cat:" <<<"$OUT" | grep -q "<h1>Second Part</h1>"; check "the second file starts with the heading" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "the split dialog runs without errors" $?'

scenario editor_compare 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:800,editor:checkpoint,editor:find-in-text=Paragraph 5 of,editor:insert=NEWWORD ,wait:200,editor:find-in-text=<p>Paragraph 20 of,editor:insert=<p>Added para</p>\n,wait:300,editor:compare=last,wait:500,editor:cmp-dump,editor:cmp-context=0,wait:400,editor:cmp-dump,editor:cmp-context=3,editor:cmp-search=NEWWORD,wait:300,editor:cmp-dump,editor:cmp-goto-text=NEWWORD,wait:400,editor:cur,editor:cmp-beautify=on,wait:400,editor:cmp-dump,editor:cmp-close,wait:300,editor:cmp-dump,editor:cmp-book='"$ROOT"'/css-epub/lib/styled.epub,wait:700,editor:cmp-dump,quit' '
    C=$(grep "^editor_cmp:" <<<"$OUT")
    sed -n 1p <<<"$C" | grep -q "visible=true .*file=Some(\"OEBPS/chapter-1.xhtml\") .*added=1 removed=0 changed=1 skips=[1-9]"; check "comparing with a checkpoint shows the changed file side by side: one changed and one added line, equal lines folded" $?
    sed -n 1p <<<"$C" | grep -q "marks=[2-9]"; check "the words that changed inside a changed line are marked" $?
    sed -n 1p <<<"$C" | grep -q "goto_left=false"; check "a double click goes to the line in the current book (the right side)" $?
    sed -n 2p <<<"$C" | grep -q "skips=3 .*context=0"; check "0 lines of context folds everything but the changes" $?
    sed -n 3p <<<"$C" | grep -q "hits=1 hit_lines=1"; check "searching the differences marks the line" $?
    grep "^editor_cur:" <<<"$OUT" | head -1 | grep -q "file=OEBPS/chapter-1.xhtml line=1[0-9]"; check "double clicking a line goes to it in the editor" $?
    sed -n 4p <<<"$C" | grep -q "beautify=true"; check "Beautify recomputes the comparison" $?
    sed -n 5p <<<"$C" | grep -q "visible=false"; check "Close returns to the code" $?
    sed -n 6p <<<"$C" | grep -q "goto_left=true"; check "comparing with another book file works too" $?
    grep "^editor_cmp_names:" <<<"$OUT" | tail -1 | grep -q "This book | styled.epub"; check "the two sides are named" $?
    grep "^editor_cmp_text:" <<<"$OUT" | sed -n 2p | grep -q "right=.*NEWWORDParagraph 5"; check "the new text appears on the right" $?
    grep "^editor_cmp_text:" <<<"$OUT" | head -1 | grep -q "unchanged lines\\\\n<p>"; check "a folded run of equal lines is a line of its own" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "the compare view runs without errors" $?'

scenario editor_checkpoints_dialog 'edit:1,wait:3000,editor:cp-new=First,editor:cp-new=,editor:cp-dialog,wait:500,editor:cp-list,editor:cp-new=Third,wait:200,editor:cp-list,editor:cp-rename=1|Renamed,editor:cp-pin=1,wait:200,editor:cp-list,editor:cp-delete=2,wait:200,editor:cp-list,editor:cp-compare=1,wait:400,editor:cp-list,quit' '
    L=$(grep "^editor_cp:" <<<"$OUT")
    sed -n 1p <<<"$L" | grep -q "dialog=true list=1:First|2:Checkpoint 2"; check "named checkpoints are created, an empty name gets a number" $?
    sed -n 2p <<<"$L" | grep -q "dialog_rows=2"; check "the dialog lists them" $?
    sed -n 3p <<<"$L" | grep -q "list=1:First|2:Checkpoint 2|3:Third"; check "a checkpoint made while the dialog is open is added" $?
    sed -n 4p <<<"$L" | grep -q "dialog_rows=3"; check "the open dialog is refreshed" $?
    sed -n 5p <<<"$L" | grep -q "list=1:Renamed\*|2:Checkpoint 2|3:Third"; check "rename and keep (pin) work" $?
    sed -n 7p <<<"$L" | grep -q "list=1:Renamed\*|3:Third"; check "a checkpoint can be deleted" $?
    sed -n 9p <<<"$L" | grep -q "dialog=false"; check "Compare closes the dialog" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "the checkpoints dialog runs without errors" $?'

scenario editor_live_css 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:800,editor:css-show,editor:find-in-text=Paragraph 3 of,editor:css-dump,editor:css-jump=p,wait:400,editor:cur,editor:open=OEBPS/chapter-1.xhtml,wait:300,editor:find-in-text=h1>Chapter,editor:css-dump,editor:open=OEBPS/style.css,wait:300,editor:css-dump,quit' '
    E=$(grep "^editor_css:" <<<"$OUT"); R=$(grep "^editor_css_rule:" <<<"$OUT")
    sed -n 1p <<<"$E" | grep -q "element=p line=[0-9]* .*rules=1 inherited=[1-9]"; check "for a paragraph the panel shows its rule and what it inherits" $?
    grep -q "^editor_css_rule: p | OEBPS/style.css:3 | from=- | margin:0 0 .8em 0;text-align:justify" <<<"$OUT"; check "the rule names its file and line and its declarations" $?
    grep -q "^editor_css_rule: body | OEBPS/style.css:1 | from=body | .*font-family:serif" <<<"$OUT"; check "inherited rules name the ancestor they come from" $?
    grep -q "^editor_css_computed: .*font-family=serif" <<<"$OUT"; check "the resulting values include inherited ones" $?
    grep "^editor_cur:" <<<"$OUT" | head -1 | grep -q "file=OEBPS/style.css line=3"; check "clicking the origin of a rule jumps to it in the stylesheet" $?
    sed -n 2p <<<"$E" | grep -q "element=h1 "; check "the panel follows the cursor to a heading" $?
    sed -n 3p <<<"$E" | grep -q "^editor_css: none"; check "a stylesheet has no element to inspect" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "the Live CSS panel runs without errors" $?'

scenario editor_pv_a11y 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:500,editor:checkpoint,editor:find-in-text=Paragraph 5 of,editor:insert=NEWWORD ,wait:200,editor:css-show,wait:300,a11y-audit:,editor:compare=last,wait:500,a11y-audit:,editor:cmp-close,editor:cp-dialog,wait:500,a11y-audit:,editor:pv-dialog=0|1,wait:500,a11y-audit:,quit' '
    ! has "^a11y-problem:"; check "the preview header, compare view, Live CSS panel, checkpoints dialog and split dialog have accessible names" $?
    [ "$(grep -c "^a11y:" <<<"$OUT")" -ge 4 ]; check "the audit ran with each of them open" $?'

scenario editor_scroll_sync 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:600,editor:pv-fill=150,wait:400,editor:pv-refresh,wait:800,until:editor-sync-idle:8000,editor:pv-sync-state,editor:pv-both=off,wait:200,editor:pv-code-scroll=120,until:editor-sync-idle:8000,wait:250,editor:pv-sync-state,editor:pv-code-scroll=60,until:editor-sync-idle:8000,wait:250,editor:pv-sync-state,editor:pv-code-scroll=1,until:editor-sync-idle:8000,wait:250,editor:pv-sync-state,editor:pv-preview-scroll=0.5,until:editor-sync-idle:8000,wait:250,editor:pv-sync-state,editor:pv-both=on,wait:200,editor:pv-preview-scroll=0.5,until:editor-sync-idle:8000,wait:250,editor:pv-sync-state,editor:pv-code-scroll=100,until:editor-sync-idle:8000,wait:250,editor:pv-sync-state,editor:find-in-text=Filler paragraph 20 of,wait:700,editor:pv-where,editor:pv-sync-state,editor:pv-hide=on,wait:300,editor:pv-code-scroll=10,until:editor-sync-idle:8000,wait:250,editor:pv-sync-state,quit' '
    S=$(grep "^editor_sync:" <<<"$OUT")
    f() { sed -n "$1p" <<<"$S" | sed -E "s/.* $2=(Some\()?([0-9]+).*/\2/"; }
    near() { [ "$(( $1 - $2 ))" -le 1 ] && [ "$(( $2 - $1 ))" -le 1 ]; }
    sed -n 1p <<<"$S" | grep -q "^editor_sync: both=true "; check "scrolling the preview scrolls the code is on by default (a fresh editor.json)" $?
    c=$(f 2 code_block); p=$(f 2 preview_block)
    [ "${c:-0}" -gt 40 ] && near "${p:-0}" "${c:-0}"; check "scrolling the code view moves the preview to the same block (code block $c, preview block $p)" $?
    c=$(f 3 code_block); p=$(f 3 preview_block)
    [ "${c:-0}" -gt 10 ] && [ "${c:-0}" -lt 70 ] && near "${p:-0}" "${c:-0}"; check "and back up again (code block $c, preview block $p)" $?
    c=$(f 4 code_block); p=$(f 4 preview_block)
    [ "${p:-9}" -le 1 ]; check "scrolling the code to the top brings the preview to the top (preview block $p)" $?
    sed -n 5p <<<"$S" | grep -q "code_line=1 "; check "with the option switched off, scrolling the preview does not move the code" $?
    sed -n 6p <<<"$S" | grep -q "both=true"; check "the option can be switched on" $?
    l=$(f 6 code_line); [ "${l:-1}" -gt 40 ]; check "with the option on, scrolling the preview scrolls the code to the same place (code line $l)" $?
    y1=$(f 7 preview_y); c=$(f 7 code_block); p=$(f 7 preview_block)
    near "${p:-0}" "${c:-0}"; check "with the option on there is no feedback: code and preview still agree after a code scroll (block $c / $p)" $?
    k=$(grep "^editor_pv: caret_block" <<<"$OUT" | tail -1 | sed -E "s/.*caret_block=Some\(([0-9]+)\).*/\1/"); p=$(f 8 preview_block)
    [ -n "$k" ] && [ "${p:-0}" -ge "$((k - 2))" ] && [ "${p:-0}" -le "$((k + 1))" ]; check "a cursor move wins: the preview shows the element at the cursor, not the scrolled code (cursor block $k, preview block $p)" $?
    [ "$(f 9 steps)" = "$(f 8 steps)" ]; check "with the preview hidden a code scroll does no sync work" $?
    t=$(f 7 step_us); [ "${t:-99999}" -lt 2000 ]; check "a sync step takes under 2 ms (${t} us)" $?
    grep -q "\"scroll_both\": true" "$d/cfg/bookx/editor.json"; check "the option is remembered in the editor settings" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "scroll sync runs without errors or GTK warnings" $?'

scenario editor_scroll_sync_old 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:600,editor:pv-sync-state,quit' '
    S=$(grep "^editor_sync:" <<<"$OUT")
    sed -n 1p <<<"$S" | grep -q "^editor_sync: both=true "; check "an old editor.json with the option off is switched on once" $?
    grep -q "\"scroll_both\": true" "$d/cfg/bookx/editor.json" && grep -q "\"scroll_both_v2\": true" "$d/cfg/bookx/editor.json"; check "the migration is written once (marker in editor.json)" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "no errors or GTK warnings" $?'

scenario editor_pv_click_selects 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:800,editor:pv-state,editor:pv-scroll-top,wait:300,editor:pv-click-block=3,wait:500,editor:pv-code-sel,quit' '
    has "^editor_pv: code_sel=\"<p>Paragraph 3 of chapter .* len=[1-9][0-9]{2,}$"; check "a click in the preview selects the whole element in the code" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "no errors or GTK warnings" $?'
