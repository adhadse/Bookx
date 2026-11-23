# Scenarios of the editor preferences, shortcuts, snippets and the new search mode (docs/archive/calibre-editor-gap-analysis.md
# CE-357, CE-383..387, CE-400..403, CE-406, CE-424). Sourced by ui-tests.sh (uses its `scenario`, `check`, `has`).
# Dev commands: `editor:prefs-*`, `editor:keys-*`, `editor:snippet-run=`, `editor:snippets-*` (src/ui/editor/prefs_page.rs, snippets.rs).

scenario editor_prefs_live "edit:1,wait:3000,editor:prefs-dump,editor:prefs-set=tab_width=4,editor:prefs-set=insert_spaces=false,editor:prefs-set=line_numbers=false,editor:prefs-set=right_margin=true,editor:prefs-set=right_margin_column=72,editor:prefs-set=scheme=kate,editor:prefs-set=font=Fira Code 12,wait:400,editor:prefs-dump,editor:open=OEBPS/style.css,wait:300,editor:cursor=0,editor:insert=a { color: #ff0000; background: rgb(0 128 0) }\n,wait:900,editor:prefs-dump,editor:prefs-set=css_colors=false,wait:400,editor:prefs-dump,editor:prefs-set=tab_width=99,wait:300,editor:prefs-dump,editor:prefs-write-scheme=bookx-test,editor:prefs-set=scheme=bookx-test,wait:300,editor:prefs-dump,editor:prefs-open,wait:800,quit" '
    P=$(grep "^editor_prefs:" <<<"$OUT")
    sed -n 1p <<<"$P" | grep -q "tab=2 spaces=true lines=true hl=true margin=false col=80 scheme=Adwaita"; check "the defaults are the old fixed behaviour" $?
    sed -n 2p <<<"$P" | grep -q "tab=4 spaces=false lines=false"; check "tab width, tabs and line numbers follow the preferences live" $?
    sed -n 2p <<<"$P" | grep -q "margin=true col=72 scheme=kate"; check "right margin and colour scheme follow live" $?
    sed -n 2p <<<"$P" | grep -q "font_css=\".code-view { font-family: \\\\\"Fira Code\\\\\", monospace; font-size: 12pt; }\""; check "the code font becomes a style rule" $?
    sed -n 3p <<<"$P" | grep -q "colors=2"; check "colour literals of a style sheet are painted (hex and rgb())" $?
    sed -n 4p <<<"$P" | grep -q "colors=0"; check "the colour preview switches off" $?
    sed -n 5p <<<"$P" | grep -q "tab=16 "; check "an absurd tab width is clamped" $?
    has "^editor_scheme: written=true listed=true"; check "a scheme dropped into the config folder is listed" $?
    sed -n 6p <<<"$P" | grep -q "scheme=bookx-test"; check "and can be chosen" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "no errors or GTK warnings" $?'

scenario editor_prefs_persist "edit:1,wait:2500,editor:prefs-set=tab_width=6,editor:prefs-set=auto_close_tags=false,wait:300,quit" '
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "changing preferences runs without errors" $?'

scenario editor_keys_registry "edit:1,wait:2500,editor:keys-palette=Save,editor:keys-set=editor.save=<Control><Alt>x,editor:keys-palette=Save,editor:keys-set=editor.close-tab=<Control><Alt>x,editor:keys-set=editor.mark=<Control>d,editor:keys-palette=Mark Selection,editor:keys-palette=Check Book,quit" '
    K=$(grep "^editor_keys:" <<<"$OUT")
    sed -n 1p <<<"$K" | grep -q "palette Save=Ctrl+S"; check "the editor palette shows the default key" $?
    sed -n 2p <<<"$K" | grep -q "bound=true"; check "an editor action can be rebound" $?
    sed -n 3p <<<"$K" | grep -q "palette Save=Ctrl+Alt+X"; check "the palette shows the new key at once" $?
    sed -n 4p <<<"$K" | grep -q "bound=false"; check "a key used by another editor action is refused (conflict)" $?
    sed -n 5p <<<"$K" | grep -q "bound=true"; check "a free key is accepted" $?
    sed -n 6p <<<"$K" | grep -q "palette Mark Selection=Ctrl+D"; check "menu-less commands show their rebound key too" $?
    sed -n 7p <<<"$K" | grep -q "palette Check Book=F7"; check "unchanged commands keep theirs" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_snippet_engine 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:500,editor:snippet-run=p|hello,editor:snippet-run=<a|word,editor:snippet-run=<<|text,editor:snippet-run=<>|,editor:snippet-run=<i|,editor:snippet-run=Lorem|,editor:snippet-run=p|,editor:snippet-run=nosuch|,editor:snippet-run=zz|sel,wait:200,quit' '
    S=$(grep "^editor_snippet: expanded" <<<"$OUT")
    sed -n 1p <<<"$S" | grep -q "expanded=true text=\"<p>hello</p>\""; check "selected text goes into the starred stop of p" $?
    sed -n 2p <<<"$S" | grep -q "text=\"<a href=\\\\\"#\\\\\">word</a>\""; check "<a wraps the selection in a link" $?
    sed -n 3p <<<"$S" | grep -q "text=\"<p>text</p>\""; check "<< wraps the selection in a tag whose closing tag mirrors it" $?
    sed -n 4p <<<"$S" | grep -q "text=\"<br/>\""; check "<> inserts a self-closing tag" $?
    sed -n 5p <<<"$S" | grep -q "<img src=\\\\\"../Images/image.jpg\\\\\" alt=\\\\\"description\\\\\"/>"; check "<i inserts an image" $?
    sed -n 6p <<<"$S" | grep -q "Lorem ipsum dolor sit amet"; check "Lorem inserts filler paragraphs" $?
    sed -n 7p <<<"$S" | grep -q "<p>text</p>"; check "without a selection the stop shows its default" $?
    sed -n 8p <<<"$S" | grep -q "expanded=false"; check "an unknown trigger does nothing" $?
    sed -n 9p <<<"$S" | grep -q "expanded=false"; check "a selection after a word that is no trigger is left alone" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "snippets run without errors" $?'

scenario editor_snippet_files "edit:1,wait:2500,editor:snippet-add=Mine|zq|any|<i>\${1:x}</i>\$0,wait:200,editor:snippets-export=@D@/snips.json,editor:snippets-import=@D@/snips.json,editor:template-save=Chapter|xhtml|<h1>{title}</h1>,editor:x:templates,editor:snippets-dialog,wait:700,quit" '
    has "^editor_snippets_export: n=1"; check "the user snippets are exported" $?
    has "^editor_snippets_import: added=1 skipped=0 total=1"; check "and imported again without duplicates" $?
    has "^editor_template: saved=true"; check "a template for new files is saved from the snippets dialog code" $?
    has "^editor_x: templates: Chapter.xhtml"; check "and New File offers it" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "the snippets dialog opens without errors or GTK warnings" $?'

scenario editor_fuzzy_mode 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:500,editor:find-in-text=</body>,editor:insert=<p>We recieve the receive and the receeve.</p>\n,wait:300,editor:search:mode=fuzzy,editor:search:find=receive,wait:500,editor:search:status,editor:search:scope=this,editor:search:replace=get,editor:search:replace-all,editor:search:has=OEBPS/chapter-1.xhtml|We get the get and the get.,editor:search:mode=text,editor:search:status,quit' '
    S=$(grep "^editor_search: status" <<<"$OUT")
    sed -n 1p <<<"$S" | grep -q "mode=Fuzzy" && sed -n 1p <<<"$S" | grep -q "hits=3"; check "the Fuzzy mode finds the word and its misspellings" $?
    has "^editor_search_has: OEBPS/chapter-1.xhtml \"We get the get and the get.\" = 1"; check "Replace All replaces the near matches too" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "no errors or GTK warnings" $?'

scenario editor_fn_functions 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:500,editor:find-in-text=</body>,editor:insert=<p>aa bb</p>\n,wait:300,editor:search:fn-add=Brackets|\b[ab]{2}\b|fn replace(m§ number§ file§ meta§ data) { "[" + m.text + "]" },editor:search:fn-add=Main|\b[ab]{2}\b|// @append_final_output_to_marked\n// @call_after_last_match\nfn replace(m§ number§ file§ meta§ data§ functions) { if m == () { return "END " + data["n"]; } data["n"] = number; functions["Brackets"].invoke(m§ number§ file§ meta§ data) + "!" },editor:search:fn-use=Main,editor:search:scope=this,editor:search:replace-all,wait:600,editor:search:has=OEBPS/chapter-1.xhtml|<p>[aa]! [bb]!</p>,editor:search:fn-report,quit' '
    grep -q "^editor_search_has: OEBPS/chapter-1.xhtml \"<p>\[aa\]! \[bb\]!</p>\" = 1" <<<"$OUT"; check "a function calls another saved function through functions[...]" $?
    grep "^editor_fn_report:" <<<"$OUT" | grep -q "Final output:.*END 2"; check "data is shared and the final output is reported" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "no errors or GTK warnings" $?'

scenario editor_marks_append "edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:500,editor:find-in-text=</body>,editor:insert=<p>aa bb</p>\n,wait:300,editor:find-in-text=<p>aa bb</p>|sel,editor:mark,editor:cur,editor:open=OEBPS/style.css,wait:300,editor:open=OEBPS/chapter-1.xhtml,wait:300,editor:cur,editor:search:fn-add=Total|\b[ab]{2}\b|// @append_final_output_to_marked\n// @call_after_last_match\n// @suppress_result_dialog\nfn replace(m§ number§ file§ meta§ data) { if m == () { return \"TOTAL \" + data[\"n\"]; } data[\"n\"] = number; () },editor:search:fn-use=Total,editor:search:scope=marked,editor:search:replace-all,wait:600,editor:search:has=OEBPS/chapter-1.xhtml|TOTAL 2,editor:cur,editor:clear-marks,editor:cur,quit" '
    C=$(grep "^editor_cur:" <<<"$OUT")
    sed -n 1p <<<"$C" | grep -q "marks=1"; check "Ctrl+Shift+M marks the selection" $?
    sed -n 2p <<<"$C" | grep -q "marks=1"; check "the mark is still there after switching to another file and back" $?
    grep -q "^editor_search_has: OEBPS/chapter-1.xhtml \"TOTAL 2\" = 1" <<<"$OUT"; check "the final output of the function is added after the marked text" $?
    sed -n 3p <<<"$C" | grep -q "marks=2"; check "and the added text is marked too" $?
    sed -n 4p <<<"$C" | grep -q "marks=0"; check "Clear Marked Text removes every mark" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "no errors or GTK warnings" $?'
