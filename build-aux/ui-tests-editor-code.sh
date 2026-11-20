# Scenarios of the code-editing helpers (docs/calibre-editor-gap-analysis.md CE-333, CE-336, CE-339, CE-341..CE-346,
# CE-348, CE-350, CE-352, CE-354..CE-356). Sourced by ui-tests.sh (uses its `scenario`, `check`, `has`, $ROOT).
# Dev commands are `editor:code:<name>=<args>` (src/ui/editor/format_actions.rs, navigate.rs, markup_hints.rs,
# insert_dialogs.rs); a comma in a text is written §. Fixture pages are written with `editor:x:put`.

CH1='<html><head><title>T</title><link rel="stylesheet" href="style.css"/></head><body><h1 id="top">Top</h1><p class="note big">See <a href="chapter-2.xhtml#sec">two</a> and <a href="#top">top</a> and <a href="nowhere.xhtml">gone</a> <a href="chapter-2.xhtml#nope">no</a> <img src="images/missing.png" alt=""/>.</p><p class="big">Again</p></body></html>'
CH2='<html><head><title>U</title></head><body><p>x</p><h2 id="sec">Section</h2><p id="sub">The second paragraph has words</p></body></html>'
CSS1='p { color: red }\n.note { font-weight: bold }\n.big { margin: 0 }\n'

scenario editor_code_nav "edit:1,wait:3000,editor:x:put=OEBPS/chapter-1.xhtml|$CH1,editor:x:put=OEBPS/chapter-2.xhtml|$CH2,editor:x:put=OEBPS/style.css|$CSS1,editor:open=OEBPS/chapter-1.xhtml,wait:500,editor:code:ctrlclick=chapter-2.xhtml#sec|3,editor:open=OEBPS/chapter-1.xhtml,editor:code:ctrlclick=href=\"#top\"|8,editor:code:ctrlclick=href=\"style.css\"|8,editor:open=OEBPS/chapter-1.xhtml,editor:code:ctrlclick=note big|1,editor:open=OEBPS/chapter-1.xhtml,editor:code:ctrlclick=big\">Again|1,editor:open=OEBPS/chapter-1.xhtml,editor:code:ctrlclick=nowhere.xhtml|2,editor:code:ctrlclick=chapter-2.xhtml#nope|3,editor:open=OEBPS/chapter-1.xhtml,editor:code:ctrlclick=See |1,editor:open=OEBPS/style.css,wait:300,editor:code:ctrlclick=.note|2,editor:open=OEBPS/style.css,editor:code:menu=.big|2,editor:code:help=margin|2,editor:open=OEBPS/chapter-1.xhtml,editor:code:menu=note big|1,editor:code:menu=<p class|1,editor:code:help=<p class|1,quit" '
    J=$(grep "^editor_code: jumped" <<<"$OUT")
    sed -n 1p <<<"$J" | grep -q "current=OEBPS/chapter-2.xhtml"; check "Ctrl+click on a link with an anchor opens that page" $?
    sed -n 1p <<<"$J" | grep -q "at=49 "; check "and puts the caret on the element that has the id" $?
    sed -n 2p <<<"$J" | grep -q "current=OEBPS/chapter-1.xhtml"; check "a link to #id stays in the file" $?
    sed -n 3p <<<"$J" | grep -q "current=OEBPS/style.css"; check "Ctrl+click on the href of a style sheet opens the style sheet" $?
    sed -n 4p <<<"$J" | grep -q "current=OEBPS/style.css"; check "Ctrl+click on a class name goes to its CSS rule" $?
    has "^editor_code: selected .*⟦note⟧"; check "and selects the class name in the rule" $?
    sed -n 5p <<<"$J" | grep -q "current=OEBPS/style.css"; check "a class used twice still leads to its rule" $?
    sed -n 6p <<<"$J" | grep -q "toast=Nothing found for OEBPS/nowhere.xhtml"; check "a link to a missing file says so and stays" $?
    sed -n 7p <<<"$J" | grep -q "toast=Nothing found for #nope"; check "a missing anchor is reported" $?
    sed -n 8p <<<"$J" | grep -q "toast=There is nothing to jump to here"; check "plain text has nothing to jump to" $?
    sed -n 9p <<<"$J" | grep -q "current=OEBPS/chapter-1.xhtml at=113 "; check "Ctrl+click on a selector in a style sheet jumps to a use of the class" $?
    has "^editor_code: menu Rename Class “big” | Go to Definition$"; check "the context menu of a class offers rename and go to definition" $?
    has "^editor_code: menu Rename Class “note” | Go to Definition$"; check "the same on a class in a page" $?
    has "^editor_code: help https://developer.mozilla.org/en-US/docs/Web/CSS/margin$"; check "right-click on a CSS property offers the MDN page" $?
    has "^editor_code: help https://developer.mozilla.org/en-US/docs/Web/HTML/Element/p$"; check "right-click on a tag offers the MDN page of the element" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "navigation runs without errors or GTK warnings" $?'

scenario editor_code_rename "edit:1,wait:3000,editor:x:put=OEBPS/chapter-1.xhtml|$CH1,editor:x:put=OEBPS/chapter-2.xhtml|$CH2.<p class=\"big note\"/>,editor:x:put=OEBPS/style.css|$CSS1,editor:open=OEBPS/chapter-1.xhtml,wait:400,editor:code:rename-preview=big|large,editor:code:rename-preview=big|1x,editor:code:rename-preview=big|note,editor:code:rename=big|large,wait:500,editor:cat=OEBPS/chapter-1.xhtml,editor:cat=OEBPS/style.css,editor:cps,editor:undo,editor:undo,wait:300,editor:cat=OEBPS/chapter-1.xhtml,quit" '
    has "^editor_code: rename-preview total=[3-9] files=[23] collides=false"; check "the preview counts the uses in pages and style sheets" $?
    has "^editor_code: rename-preview invalid"; check "an invalid new name is refused" $?
    has "^editor_code: rename-preview total=.* collides=true"; check "renaming into an existing class warns about the merge" $?
    C=$(grep "^editor_cat:" <<<"$OUT")
    sed -n 1p <<<"$C" | grep -q "class=\"note large\"" && sed -n 1p <<<"$C" | grep -q "class=\"large\">Again"; check "every use in the page is renamed" $?
    sed -n 2p <<<"$C" | grep -q "\.large { margin: 0 }" && sed -n 2p <<<"$C" | grep -q "\.note { font-weight"; check "the rule in the style sheet is renamed, others stay" $?
    has "^editor_cps: .*Before: Rename class big"; check "a checkpoint is taken first" $?
    has "^editor_code: rename uses=[3-9] files="; check "the rename reports its numbers" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "renaming runs without errors or GTK warnings" $?'

scenario editor_code_format "edit:1,wait:3000,editor:x:put=OEBPS/chapter-1.xhtml|<html><body><p>one two three</p><p><b>alpha beta gamma</b></p><p>four <i>five</i> six</p><p>red <em>green</em> blue</p><p>seven eight</p></body></html>,editor:open=OEBPS/chapter-1.xhtml,wait:400,editor:find-in-text=two|sel,editor:code:format=bold,editor:code:format=bold,editor:find-in-text=beta|sel,editor:code:format=bold,editor:find-in-text=five|sel,editor:code:format=italic,editor:find-in-text=red <em>green</em> blue|sel,editor:code:format=bold,editor:find-in-text=green</em> blue|sel,editor:code:format=underline,editor:find-in-text=eight,editor:code:split,editor:find-in-text=seven|sel,editor:code:format=sup,editor:find-in-text=alpha|sel,editor:code:format=strike,editor:code:format=code,quit" '
    F=$(grep "^editor_code: format" <<<"$OUT")
    sed -n 1p <<<"$F" | grep -q "<p>one <b>⟦two⟧</b> three</p>"; check "bold wraps the selection and keeps it selected" $?
    sed -n 2p <<<"$F" | grep -q "<p>one ⟦two⟧ three</p>"; check "bold again on the bold selection takes the tag off" $?
    sed -n 3p <<<"$F" | grep -q "<b>alpha </b>⟦beta⟧<b> gamma</b>"; check "bold on a word inside bold text splits the bold element" $?
    sed -n 4p <<<"$F" | grep -q "<p>four ⟦five⟧ six</p>"; check "italic on italic text removes the element" $?
    sed -n 5p <<<"$F" | grep -q "<b>⟦red <em>green</em> blue⟧</b>"; check "a selection with whole inner elements can be formatted" $?
    sed -n 6p <<<"$F" | grep -q "<b>red <em>⟦green</em> blue⟧</b>" && sed -n 6p <<<"$F" | grep -q "toast=The selection cuts through tags"; check "a selection that cuts through tags is refused and the text is unchanged" $?
    sed -n 7p <<<"$F" | grep -q "<sup>⟦seven⟧</sup>"; check "superscript wraps in <sup>" $?
    sed -n 8p <<<"$F" | grep -q "<strike>\|<s>⟦alpha⟧</s>\|<s>"; check "strikethrough wraps in <s>" $?
    sed -n 9p <<<"$F" | grep -q "<code>"; check "code wraps in <code>" $?
    S=$(grep "^editor_code: split" <<<"$OUT")
    grep -q "<p>seven </p><p>|eight</p>" <<<"$S"; check "Split Tag at the caret closes the paragraph and opens a copy" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "formatting runs without errors or GTK warnings" $?'

scenario editor_code_hints "edit:1,wait:3000,editor:x:put=OEBPS/chapter-2.xhtml|$CH2,editor:x:put=OEBPS/chapter-1.xhtml|<html><body><h1>Head</h1><p>open <b>x</p><a href=\"gone.xhtml\">g</a> <a href=\"chapter-2.xhtml#nope\">n</a> <a href=\"chapter-2.xhtml#sec\">ok</a> a & b <i>it</i></body></html>,editor:x:put=OEBPS/style.css|p { color: red; background: #00ff00 }\n.a { border: 1px solid rgb(0§0§255); font-family: orange }\nh1 { color: hsl(0§100%§50%) }\n,editor:open=OEBPS/chapter-1.xhtml,wait:900,editor:code:hints,editor:code:hints-tags,editor:code:tooltip=gone.xhtml|2,editor:code:tooltip=<b>|1,editor:code:tooltip=a & b|2,editor:code:tooltip=ok</a>|0,editor:code:hint-opt=emphasis:on,editor:code:hints,editor:code:hints-tags,editor:code:hint-opt=invalid:off,editor:code:hint-opt=links:off,editor:code:hints,editor:open=OEBPS/style.css,wait:900,editor:code:hints,editor:code:hints-tags,editor:code:hint-opt=colors:off,editor:code:hints,quit" '
    H=$(grep "^editor_code: hints " <<<"$OUT")
    sed -n 1p <<<"$H" | grep -q "invalid=2 link_ok=1 link_missing=2 colors=0 bold=0 italic=0"; check "invalid markup, good and missing links are counted (unclosed <b>, bare &, two missing targets)" $?
    T=$(grep "^editor_code: tag " <<<"$OUT")
    grep -q "^editor_code: tag markup-error <b|&" <<<"$T"; check "the unclosed <b> start tag and the bare & get the red squiggle" $?
    grep -q "^editor_code: tag link-missing gone.xhtml|chapter-2.xhtml#nope" <<<"$T"; check "links to a missing file and a missing anchor are underlined red" $?
    grep -q "^editor_code: tag link-target chapter-2.xhtml#sec" <<<"$T"; check "a link that leads somewhere is underlined as a target" $?
    has "^editor_code: tooltip [0-9]* There is no such file or anchor: gone.xhtml"; check "the tooltip of a missing link names the target" $?
    has "^editor_code: tooltip [0-9]* <b> is never closed"; check "the tooltip of the squiggle says what is wrong" $?
    has "^editor_code: tooltip [0-9]* & must be written as &amp;"; check "the tooltip explains a bare ampersand" $?
    has "^editor_code: tooltip [0-9]* $"; check "no tooltip where nothing is wrong" $?
    sed -n 2p <<<"$H" | grep -q "colors=0 bold=1 italic=1"; check "with the option on, headings and <i> are drawn bold and italic" $?
    sed -n 3p <<<"$H" | grep -q "invalid=0 link_ok=0 link_missing=0"; check "the View menu options switch the underlines off" $?
    sed -n 4p <<<"$H" | grep -q "invalid=0 .* colors=4 "; check "colour literals of a style sheet get swatches (name, hex, rgb(), hsl(); not font-family)" $?
    grep -q "^editor_code: swatch swatch-ff0000ff red" <<<"$OUT"; check "a swatch tag carries the colour of its literal" $?
    grep -q "^editor_code: swatch swatch-00ff00ff #00ff00" <<<"$OUT"; check "hex colours get a swatch" $?
    sed -n 5p <<<"$H" | grep -q "colors=0"; check "the swatches can be switched off" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "the decorations run without errors or GTK warnings" $?'

scenario editor_code_insert "edit:1,wait:3000,editor:x:put=OEBPS/style.css|z { a: b }\n/* c */\na { c: d }\n@font-face { src: url(x) }\nm { e: f }\nb { g: h }\n,editor:x:put=OEBPS/chapter-1.xhtml|<html><body><p>See the words here</p></body></html>,editor:open=OEBPS/chapter-1.xhtml,wait:400,editor:find-in-text=See |sel,editor:code:insert-image=../images/a.png|A \"cat\" & dog|,editor:find-in-text=words|sel,editor:code:insert-link=chapter-2.xhtml#sec||The second,editor:find-in-text=here,editor:code:insert-link=chapter-2.xhtml||,editor:code:insert-link=chapter-3.xhtml||,editor:find-in-text=the ,editor:code:insert-link=https://example.org/|Example|,editor:x:put=OEBPS/chapter-1.xhtml|<html><body></body></html>,editor:open=OEBPS/chapter-1.xhtml,wait:300,editor:find-in-text=</body>,editor:code:autoclose=<div><p>t</,editor:code:type=p,editor:code:type=>,editor:code:type=x,editor:code:autoclose=</,editor:code:hint-opt=auto-close:off,editor:code:autoclose=<span></,editor:code:hint-opt=auto-close:on,editor:open=OEBPS/style.css,wait:300,editor:code:sort-css,editor:code:sort-css,quit" '
    I=$(grep "^editor_code: insert" <<<"$OUT")
    sed -n 1p <<<"$I" | grep -q "<img src=\"../images/a.png\" alt=\"A &quot;cat&quot; &amp; dog\"/>|\?"; check "Insert Image writes an escaped img with alt text" $?
    sed -n 2p <<<"$I" | grep -q "<a href=\"chapter-2.xhtml#sec\" title=\"The second\">⟦words⟧</a>"; check "Insert Hyperlink wraps the selection and keeps the markup as link text" $?
    sed -n 3p <<<"$I" | grep -q "<a href=\"chapter-2.xhtml\">⟦chapter-2.xhtml⟧</a>"; check "without selection or text the file name becomes the link text" $?
    sed -n 4p <<<"$I" | grep -q "<a href=\"chapter-3.xhtml\">⟦chapter-2.xhtml⟧</a>" && ! sed -n 4p <<<"$I" | grep -q "<a href=\"chapter-2.xhtml\"><a"; check "inside a link the address is changed instead of nesting a second link" $?
    sed -n 5p <<<"$I" | grep -q "<a href=\"https://example.org/\">⟦Example⟧</a>"; check "a web address can be linked with its own text" $?
    A=$(grep "^editor_code: autoclose" <<<"$OUT")
    sed -n 1p <<<"$A" | grep -q "<div><p>t</p>|</body>"; check "typing </ closes the innermost open tag" $?
    sed -n 2p <<<"$A" | grep -q "<div><p>t</p></div>|</body>"; check "and the next one closes the one outside" $?
    sed -n 3p <<<"$A" | grep -q "<span></|</body>"; check "with the preference off nothing is closed" $?
    T=$(grep "^editor_code: type" <<<"$OUT")
    sed -n 1p <<<"$T" | grep -q "swallowed=true" && sed -n 2p <<<"$T" | grep -q "swallowed=true"; check "typing the name and > of a tag that </ just closed does not double them" $?
    sed -n 3p <<<"$T" | grep -q "swallowed=false"; check "any other character is typed as usual" $?
    S=$(grep "^editor_code: sort" <<<"$OUT")
    sed -n 1p <<<"$S" | grep -q "/\* c \*/\\\\na { c: d }\\\\nz { a: b }\\\\n@font-face { src: url(x) }\\\\nb { g: h }\\\\nm { e: f }"; check "Sort CSS Rules orders the rules and keeps at-rules and comments in place" $?
    sed -n 2p <<<"$S" | grep -q "a { c: d }"; check "sorting again changes nothing" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "inserting runs without errors or GTK warnings" $?'

scenario editor_code_complete "edit:1,wait:3000,editor:x:put=OEBPS/chapter-2.xhtml|$CH2,editor:open=OEBPS/chapter-1.xhtml,wait:700,editor:complete=<a href=\"c2,editor:complete=<a href=\"s.css,editor:complete=<a href=\"chapter-2.xhtml#,editor:complete=<a href=\"chapter-2.xhtml#su,editor:complete=<img src=\"zzzz,quit" '
    C=$(grep "^editor_complete:" <<<"$OUT")
    sed -n 1p <<<"$C" | grep -q "chapter-2.xhtml\[file:2\]"; check "c2 finds chapter-2.xhtml by subsequence" $?
    sed -n 2p <<<"$C" | grep -q "style.css\[file:5\]"; check "s.css finds style.css: letters in order, not only a prefix" $?
    sed -n 3p <<<"$C" | grep -q "sec\[anchor:0\]«Section»" && sed -n 3p <<<"$C" | grep -q "sub\[anchor:0\]«The second paragraph has words»"; check "anchors after # show the text they point at" $?
    sed -n 4p <<<"$C" | grep -q "sub\[anchor:2\]«The second" && ! sed -n 4p <<<"$C" | grep -q "sec\["; check "typing after # narrows the anchors" $?
    sed -n 5p <<<"$C" | grep -q "n=0 "; check "nothing is offered when no file matches" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "completion runs without errors or GTK warnings" $?'

# Text colour, background colour and alignment (CE-350): spans and `text-align`, toggled off by the same value, with
# CSS classes (and the rule in the style sheet) when View > Use CSS Classes is on.
scenario editor_code_style "edit:1,wait:3000,editor:x:put=OEBPS/chapter-1.xhtml|<html><body><p>one two three</p><p>four five six</p><p style=\"margin:0\">seven</p></body></html>,editor:x:put=OEBPS/style.css|p { color: black }\n,editor:open=OEBPS/chapter-1.xhtml,wait:400,editor:find-in-text=two|sel,editor:code:colour=text|#ff0000,editor:code:colour=text|#ff0000,editor:find-in-text=three|sel,editor:code:colour=background|yellow,editor:code:colour=text|red;x,editor:code:colour=text|off,editor:find-in-text=five,editor:code:align=center,editor:code:align=center,editor:code:align=right,editor:code:align=off,editor:find-in-text=seven,editor:code:align=justify,editor:code:colour-classes=on,editor:find-in-text=six|sel,editor:code:colour=text|#00F,editor:find-in-text=four,editor:code:align=center,editor:cat=OEBPS/style.css,editor:open=OEBPS/style.css,wait:200,editor:code:align=left,quit" '
    S=$(grep "^editor_code: style" <<<"$OUT")
    sed -n 1p <<<"$S" | grep -q "<p>one <span style=\"color:#ff0000\">⟦two⟧</span> three</p>"; check "Text Colour wraps the selection in a span with the colour" $?
    sed -n 2p <<<"$S" | grep -q "<p>one ⟦two⟧ three</p>"; check "the same colour again takes the span off" $?
    sed -n 3p <<<"$S" | grep -q "<span style=\"background-color:yellow\">⟦three⟧</span>"; check "Background Colour writes background-color" $?
    sed -n 4p <<<"$S" | grep -q "toast=That is not a colour"; check "a value that is not a colour is refused" $?
    sed -n 5p <<<"$S" | grep -q "toast=There is no colour here to remove" && sed -n 5p <<<"$S" | grep -q "<span style=\"background-color:yellow\">⟦three⟧</span>"; check "Remove Text Colour where there is none says so and leaves the page alone" $?
    sed -n 6p <<<"$S" | grep -q "<p style=\"text-align:center\">|four five six</p>\|<p style=\"text-align:center\">four |five six</p>"; check "Align Centre sets text-align on the block around the caret" $?
    sed -n 7p <<<"$S" | grep -q "<p>four five six</p>\|<p>.*four.*five six</p>" && ! sed -n 7p <<<"$S" | grep -q "text-align"; check "the same alignment again takes it off" $?
    sed -n 8p <<<"$S" | grep -q "text-align:right"; check "another alignment replaces it" $?
    ! sed -n 9p <<<"$S" | grep -q "text-align"; check "Remove Alignment takes it off" $?
    sed -n 10p <<<"$S" | grep -q "<p style=\"margin:0;text-align:justify\">"; check "other style declarations of the block are kept" $?
    sed -n 11p <<<"$S" | grep -q "<span class=\"color-00f\">⟦six⟧</span>"; check "with CSS classes on, the colour is written as a class" $?
    sed -n 12p <<<"$S" | grep -q "<p class=\"align-center\">"; check "and the alignment as an align- class" $?
    sed -n 13p <<<"$S" | grep -q "p { color: black }.n.color-00f { color: #00F; }.n.align-center { text-align: center; }.n| toast=Colours and alignment work in HTML pages"; check "the rules of both classes are added to the style sheet, and style sheets are not styled" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "styling runs without errors or GTK warnings" $?'

# The Markup menu is split into short pages and shows the keys of the registry, also after a rebinding.
scenario editor_markup_menu "edit:1,wait:3000,editor:code:menu,editor:code:rebind=editor.format-bold|<Control><Alt>x,wait:300,editor:code:menu,quit" '
    M=$(grep "^editor_menu: " <<<"$OUT")
    sed -n 1p <<<"$M" | grep -q "^editor_menu: top=Format,Insert,Structure,View "; check "the Markup menu starts with Format, Insert and Structure submenus" $?
    sed -n 1p <<<"$M" | sed -E "s/.*biggest_page=([0-9]+).*/\1/" | { read n; [ "$n" -le 12 ]; }; check "no page of the Markup menu is longer than 12 rows (fits 900x600)" $?
    has "^editor_menu_accel: Bold=<Control>b"; check "Bold shows Ctrl+B from the registry" $?
    has "^editor_menu_accel: Insert Hyperlink=<Control>k"; check "Insert Hyperlink shows its key" $?
    has "^editor_menu_accel: Go to Definition=F12"; check "Go to Definition shows F12" $?
    has "^editor_menu_accel: Bold=<Control><Alt>x"; check "after rebinding Bold the menu shows the new key" $?
    [ "$(grep -c "^editor_menu_accel: Bold=<Control>b$" <<<"$OUT")" = 1 ]; check "and the old key is only in the first listing" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "the menu builds without errors or GTK warnings" $?'

# The completion popup answers only after a short pause (one popup, no hide-and-show in the same iteration).
scenario editor_completion_debounce "edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:500,editor:complete-show,wait:700,editor:complete-show,wait:700,editor:complete-waits,quit" '
    has "^editor_popup_waits: answers=[1-9][0-9]* shortest_ms=(9[0-9]|[1-9][0-9][0-9]+) debounced=true"; check "every answer of the completion popup came after the debounce" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "the popup opens without errors or GTK warnings" $?'

# `</` closes the tag when the `/` comes from an input method commit (no key press), not only from the key.
scenario editor_code_im_commit "edit:1,wait:3000,editor:x:put=OEBPS/chapter-1.xhtml|<html><body></body></html>,editor:open=OEBPS/chapter-1.xhtml,wait:400,editor:find-in-text=</body>,editor:code:commit=<p>x</,wait:400,editor:code:dump,editor:code:commit=/,wait:400,editor:code:dump,quit" '
    D=$(grep "^editor_code: dump" <<<"$OUT")
    sed -n 1p <<<"$D" | grep -q "<body><p>x</p>|</body>"; check "a / committed by an input method after < closes the open tag" $?
    sed -n 2p <<<"$D" | grep -q "<body><p>x</p>|/</body>\|<body><p>x</p>/|</body>"; check "a / that does not follow < is just a slash" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "committing text runs without errors or GTK warnings" $?'

# Ctrl held over a link target or a class name: underlined with the hand pointer, nothing elsewhere.
scenario editor_code_ctrl_hover "edit:1,wait:3000,editor:x:put=OEBPS/chapter-1.xhtml|$CH1,editor:open=OEBPS/chapter-1.xhtml,wait:400,editor:code:hover=chapter-2.xhtml#sec|3,editor:code:hover=note big|1,editor:code:hover=See |1,editor:code:hover=id=\"top\"|5,editor:code:hover=none,quit" '
    H=$(grep "^editor_code: hover" <<<"$OUT")
    sed -n 1p <<<"$H" | grep -q "target=\[chapter-2.xhtml#sec\] underlined=[0-9]*\.\.[0-9]* cursor=pointer"; check "Ctrl over a link target underlines it and shows the hand" $?
    sed -n 2p <<<"$H" | grep -q "target=\[note\] .*cursor=pointer"; check "Ctrl over a class name underlines that class" $?
    sed -n 3p <<<"$H" | grep -q "target=\[\] underlined= cursor=text"; check "plain text has no hover" $?
    sed -n 4p <<<"$H" | grep -q "target=\[top\] .*cursor=pointer"; check "an id counts too" $?
    sed -n 5p <<<"$H" | grep -q "underlined= "; check "leaving takes the underline away" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "hovering runs without errors or GTK warnings" $?'
