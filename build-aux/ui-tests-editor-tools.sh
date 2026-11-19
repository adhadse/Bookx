# Scenarios of the editor's Tools dialogs, file browser extras and window features (docs/calibre-editor-gap-analysis.md
# CE-020..CE-046, CE-130..CE-146, CE-005..CE-018). Sourced by ui-tests.sh (uses its `scenario`, `check`, `has`, $ROOT).
# Dev commands are `editor:x:<name>=<args>` (src/ui/editor/ext_dev.rs); a comma in a text is written §.

# fonts and web answers the tools look for: only these, never the real ones
export BOOKX_FONT_DIRS="$ROOT/xt/fonts"
export BOOKX_EDITOR_NET_MOCK="$ROOT/xt/net"
mkdir -p "$ROOT/xt/fonts" "$ROOT/xt/net"
printf '<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Imported</title></head><body><h1>Imported page</h1><p>Words of the imported page.</p></body></html>' >"$ROOT/xt/page.html"
printf 'p { color: blue }\n' >"$ROOT/xt/net/remote.css"

XP='http://www.w3.org/1999/xhtml'

# the "other program" of Open With: it rewrites the file it is given
printf '#!/bin/sh\nsleep 0.3\nprintf "<html xmlns=\\"http://www.w3.org/1999/xhtml\\"><head><title>E</title></head><body><p>EXTERNAL EDIT</p></body></html>" > "$1"\n' >"$ROOT/xt/mock-editor.sh"
chmod +x "$ROOT/xt/mock-editor.sh"
export BOOKX_OPEN_WITH_PROGRAM="$ROOT/xt/mock-editor.sh"


scenario editor_x_text "edit:1,wait:3000,editor:x:put=OEBPS/chapter-1.xhtml|<html xmlns=\"$XP\"><head><title>T</title></head><body><p>\"Hello\" she said -- ok... Fine.</p><pre>\"keep\" -- this</pre></body></html>,editor:x:smarten=OEBPS/chapter-1.xhtml|smarten|de|qde,wait:500,editor:cat=OEBPS/chapter-1.xhtml,editor:x:smarten=all|unsmarten,wait:500,editor:cat=OEBPS/chapter-1.xhtml,editor:x:smarten=all|smarten|en|d,wait:500,editor:cat=OEBPS/chapter-1.xhtml,editor:x:put=OEBPS/chapter-2.xhtml|<html><body><p>Open <b>tags<p>more</body></html>,editor:x:fixhtml=OEBPS/chapter-2.xhtml|html5,wait:500,editor:cat=OEBPS/chapter-2.xhtml,editor:x:put=OEBPS/chapter-1.xhtml|<html><body><p>Sloppy<br></body></html>,editor:x:fixhtml=all,wait:500,editor:cat=OEBPS/chapter-1.xhtml,editor:x:beautify=OEBPS/chapter-3.xhtml,wait:500,editor:cat=OEBPS/chapter-3.xhtml,editor:x:beautify=OEBPS/chapter-3.xhtml,wait:400,editor:dump,editor:cps,quit" '
    C=$(grep "^editor_cat:" <<<"$OUT")
    sed -n 1p <<<"$C" | grep -q "„Hello“ she said – ok… Fine"; check "German rules: quotes, dash and ellipsis are smartened" $?
    sed -n 1p <<<"$C" | grep -q "<pre>\"keep\" -- this</pre>"; check "text in pre is never touched" $?
    sed -n 2p <<<"$C" | grep -q "\"Hello\" she said -- ok... Fine"; check "Straighten undoes the smartening" $?
    sed -n 3p <<<"$C" | grep -q "\"Hello\" she said – ok... Fine"; check "with only dashes chosen quotes and ellipsis stay as typed" $?
    sed -n 4p <<<"$C" | grep -q "</b></p><p><b>more</b></p>"; check "the HTML5 parser resolves misnested tags the way a browser does" $?
    sed -n 5p <<<"$C" | grep -q "xmlns=\"http://www.w3.org/1999/xhtml\"" && sed -n 5p <<<"$C" | grep -q "<br/>"; check "Fix HTML repairs a sloppy page and adds the namespace" $?
    sed -n 6p <<<"$C" | grep -qE "<html xmlns=\"http://www.w3.org/1999/xhtml\" lang=\"en\">⏎  <head>.*⏎  </head>⏎  <body>"; check "Beautify indents the block tags" $?
    ! sed -n 6p <<<"$C" | grep -q "plain words.⏎"; check "text inside paragraphs keeps its line" $?
    has "^editor: .*toast=\"Already nicely laid out\""; check "a second Beautify finds nothing to do" $?
    has "^editor_cps: Before: Smarten Punctuation/Before: Smarten Punctuation/Before: Smarten Punctuation/Before: Fix HTML/Before: Fix HTML/Before: Beautify"; check "every tool took its own checkpoint" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_x_styles "edit:1,wait:3000,editor:x:put=OEBPS/style.css|p { color: red; font-size: 10px; margin: 0 }\n.unused { color: blue }\nh1 { font-family: Georgia§ serif; font-style: normal }\n,editor:x:styles=change|color|red|green;;scale|2;;italic|Georgia,wait:500,editor:cat=OEBPS/style.css,editor:x:ruleset-save=Mine;;scale|1.5;;remove|margin,editor:x:ruleset-list,editor:x:ruleset-load=Mine,wait:500,editor:cat=OEBPS/style.css,editor:x:unused-css,wait:500,editor:cat=OEBPS/style.css,editor:x:filter-list,editor:x:filter=font-style,wait:500,editor:cat=OEBPS/style.css,editor:x:styles=scale|abc,editor:dump,editor:cps,quit" '
    C=$(grep "^editor_cat:" <<<"$OUT")
    sed -n 1p <<<"$C" | grep -q "color: green" && sed -n 1p <<<"$C" | grep -q "font-size: 20px"; check "rules change a value and multiply the font size" $?
    sed -n 1p <<<"$C" | grep -q "font-style: italic"; check "Make family italic replaces font-style where the family is used" $?
    has "^editor_x: rulesets: Mine\(2\)$"; check "a rule set is saved in the configuration" $?
    sed -n 2p <<<"$C" | grep -q "font-size: 30px" && ! sed -n 2p <<<"$C" | grep -q "margin"; check "a saved rule set is loaded and applied" $?
    sed -n 3p <<<"$C" | grep -q "p {" && ! sed -n 3p <<<"$C" | grep -q "unused"; check "Remove unused CSS drops the rules nothing uses" $?
    has "^editor_x: filter: .*font-size:"; check "Filter style information lists the properties with their counts" $?
    ! sed -n 4p <<<"$C" | grep -q "font-style"; check "the chosen property is removed" $?
    has "^editor: .*toast=\"Add at least one rule\"" || has "^editor: .*toast=\"The factor must be a number"; check "a rule with a bad value is refused with a hint" $?
    has "^editor_cps: Before: Transform Styles/Before: Transform Styles/Before: Remove Unused CSS/Before: Filter Style Information$"; check "each style tool took a checkpoint" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_x_media "edit:1,wait:3000,editor:x:add-png=orphan.png,editor:x:add-png=used.png,editor:x:add-font=orphan.ttf,editor:x:put=OEBPS/chapter-1.xhtml|<html xmlns=\"$XP\"><head><title>T</title></head><body><p><img src=\"used.png\" alt=\"\"/></p></body></html>,editor:x:unused-media-list,editor:x:unused-media,wait:500,editor:dump,editor:x:write-font=$ROOT/xt/fonts/tiny.ttf,editor:x:put=OEBPS/chapter-2.xhtml|<html xmlns=\"$XP\"><head><title>T</title></head><body><p style=\"font-family: Tiny Test\">ABC</p></body></html>,editor:x:fonts-list,editor:x:embed=Tiny Test|subset,wait:500,editor:dump,editor:cat=OEBPS/style.css,editor:x:fonts-list,editor:cps,quit" '
    has "^editor_x: unused-media: .*orphan.png.*orphan.ttf|^editor_x: unused-media: .*orphan.ttf.*orphan.png"; check "unused images and fonts are listed" $?
    ! has "^editor_x: unused-media: .*used.png"; check "a used image is not listed" $?
    F=$(grep "^editor_files:" <<<"$OUT")
    ! sed -n 1p <<<"$F" | grep -q "orphan"; check "the unused files are deleted" $?
    sed -n 1p <<<"$F" | grep -q "OEBPS/images/used.png"; check "the used image stays" $?
    has "^editor_x: fonts: tiny test:Embedding allowed"; check "a font used but not carried is found on the computer, with its licence flag" $?
    sed -n 2p <<<"$F" | grep -q "OEBPS/fonts/tiny-test-Regular.ttf"; check "the font is embedded into the book" $?
    has "^editor_cat: .*@font-face"; check "an @font-face rule is added" $?
    has "^editor_x: fonts: $"; check "nothing is offered after embedding" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_x_access "edit:1,wait:3000,editor:x:semantics=foreword:OEBPS/chapter-2.xhtml;title-page:OEBPS/chapter-1.xhtml,wait:400,editor:x:semantics-dump,editor:x:semantics=foreword:-,editor:x:semantics-dump,editor:x:put=OEBPS/chapter-1.xhtml|<html xmlns=\"$XP\"><head><title>T</title></head><body><p><img src=\"a.png\"/></p></body></html>,editor:x:access-dump,editor:x:access=lang:de;alt:OEBPS/chapter-1.xhtml#0=A drawing;summary:Tagged;mode:textual,wait:500,editor:x:access-dump,editor:cat=OEBPS/chapter-1.xhtml,editor:x:access=lang:de,editor:dump,quit" '
    has "^editor_x: semantics: .*foreword=OEBPS/chapter-2.xhtml"; check "Set semantics writes a foreword entry" $?
    has "^editor_x: semantics: .*title-page=OEBPS/chapter-1.xhtml"; check "and a title page entry" $?
    [ "$(grep "^editor_x: semantics:" <<<"$OUT" | sed -n 2p)" != "" ] && ! grep "^editor_x: semantics:" <<<"$OUT" | sed -n 2p | grep -q foreword; check "clearing a semantic removes only that entry" $?
    has "^editor_x: access: lang=en images=1 alts=chapter-1.xhtml#0=None"; check "the image list shows an image without a description" $?
    has "^editor_x: access: lang=de .*A drawing.*summary=\"Tagged\" modes=textual"; check "language, description and metadata are written" $?
    has "^editor_cat: .*lang=\"de\".*alt=\"A drawing\""; check "the page carries the new language and description" $?
    has "^editor: .*toast=\"Nothing changed\""; check "applying the same values again changes nothing" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_x_links "edit:1,wait:3000,editor:x:write-file=$ROOT/xt/net/ok.html|ok,editor:x:write-png=$ROOT/xt/net/pic.png,editor:x:put=OEBPS/chapter-1.xhtml|<html xmlns=\"$XP\"><head><title>T</title><link rel=\"stylesheet\" href=\"https://mock.example/remote.css\"/></head><body><p><a href=\"https://mock.example/ok.html\">ok</a><a href=\"https://mock.example/gone\">gone</a><a href=\"https://down.example/x\">down</a><img src=\"https://mock.example/pic.png\" alt=\"\"/></p></body></html>,editor:x:links-check,wait:300,editor:dump,editor:x:download,wait:800,editor:dump,editor:cat=OEBPS/chapter-1.xhtml,editor:x:put=OEBPS/chapter-2.xhtml|<html xmlns=\"$XP\"><head><title>T</title></head><body><p><a href=\"Chapter-3.xhtml\">x</a></p></body></html>,editor:x:repair-links,wait:500,editor:cat=OEBPS/chapter-2.xhtml,editor:select-files=OEBPS/chapter-3.xhtml,wait:300,editor:x:move=Text,wait:500,editor:cat=OEBPS/chapter-2.xhtml,editor:dump,editor:x:ext=html|OEBPS/chapter-2.xhtml,wait:500,editor:cat=OEBPS/nav.xhtml,editor:dump,quit" '
    has "^editor_x: links: .*https://mock.example/ok.html=Ok"; check "a reachable link is fine" $?
    has "https://mock.example/gone=NotFound"; check "a missing page is reported as not found" $?
    has "https://down.example/x=Error\(\"timed out\"\)"; check "a server that does not answer is reported" $?
    has "^editor: .*results=2 "; check "the two broken links are listed in the results with their page" $?
    C=$(grep "^editor_cat:" <<<"$OUT")
    sed -n 1p <<<"$C" | grep -q "href=\"external/remote.css\"" && sed -n 1p <<<"$C" | grep -q "src=\"external/pic.png\""; check "Download external resources copies the stylesheet and image into the book and relinks them" $?
    sed -n 1p <<<"$C" | grep -q "https://mock.example/ok.html"; check "plain links to web pages are left alone" $?
    sed -n 2p <<<"$C" | grep -q "href=\"chapter-3.xhtml\""; check "Repair broken links fixes a wrong capital letter" $?
    sed -n 3p <<<"$C" | grep -q "href=\"Text/chapter-3.xhtml\""; check "Move to folder updates the links to the file" $?
    grep "^editor_files:" <<<"$OUT" | tail -1 | grep -q "OEBPS/Text/chapter-3.xhtml"; check "the file is in the new folder" $?
    sed -n 4p <<<"$C" | grep -q "chapter-2.html"; check "Change extension updates the navigation document" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_x_files "edit:1,wait:3000,editor:x:groups,editor:x:drop=OEBPS/chapter-3.xhtml|OEBPS/chapter-1.xhtml|before,wait:500,editor:x:spine,editor:x:groups,editor:x:drop=OEBPS/chapter-3.xhtml|OEBPS/chapter-2.xhtml|after,wait:500,editor:x:spine,editor:x:drop=OEBPS/style.css|OEBPS/chapter-2.xhtml|after,editor:dump,editor:x:put=OEBPS/stray.css|p{},wait:300,editor:x:fileinfo=OEBPS/stray.css,editor:x:fileinfo=OEBPS/content.opf,editor:x:fileinfo=OEBPS/nav.xhtml,editor:x:fileinfo=OEBPS/chapter-1.xhtml,editor:open=OEBPS/chapter-2.xhtml,wait:300,editor:x:nonlinear,wait:400,editor:dump,editor:x:fileinfo=OEBPS/chapter-2.xhtml,editor:x:nonlinear,wait:400,editor:dump,editor:x:fileinfo=OEBPS/chapter-2.xhtml,editor:open=OEBPS/chapter-1.xhtml,wait:300,editor:x:mark=title,wait:400,editor:x:semantics-dump,editor:x:fileinfo=OEBPS/chapter-1.xhtml,editor:x:select-group=Text,editor:x:selected,editor:x:select-group=none,editor:x:selected,editor:x:rowlabel=OEBPS/chapter-1.xhtml,editor:x:fullpaths=on,editor:x:rowlabel=OEBPS/chapter-1.xhtml,editor:x:fullpaths=off,editor:x:write-file=$ROOT/xt/new.xhtml|<html xmlns=\"$XP\"><head><title>N</title></head><body><p>NEW REPLACED</p></body></html>,editor:x:replace=OEBPS/chapter-2.xhtml|swapped.xhtml|$ROOT/xt/new.xhtml,wait:600,editor:cat=OEBPS/swapped.xhtml,editor:cat=OEBPS/nav.xhtml,editor:x:write-template=Poem.xhtml|<html xmlns=\"$XP\"><head><title>{title}</title></head><body><h1>{title}</h1></body></html>,editor:x:templates,editor:x:newfile-user=verse|0|link,wait:600,editor:cat=OEBPS/verse.xhtml,editor:x:add-png=pic.png,editor:x:grid,wait:600,a11y-audit:,quit" '
    G=$(grep "^editor_x: groups:" <<<"$OUT")
    sed -n 1p <<<"$G" | grep -q "^editor_x: groups: Text:OEBPS/chapter-1.xhtml Text:OEBPS/chapter-2.xhtml Text:OEBPS/chapter-3.xhtml .*Styles:OEBPS/style.css"; check "the file list is grouped: text in reading order first, then styles" $?
    sed -n 2p <<<"$G" | grep -q "Text:OEBPS/chapter-3.xhtml Text:OEBPS/chapter-1.xhtml Text:OEBPS/chapter-2.xhtml"; check "dropping a row before another reorders the reading order and the list follows it" $?
    S=$(grep "^editor_x: spine:" <<<"$OUT")
    sed -n 1p <<<"$S" | grep -q "chapter-3.xhtml OEBPS/chapter-1.xhtml OEBPS/chapter-2.xhtml"; check "the spine is reordered by the drop" $?
    sed -n 2p <<<"$S" | grep -q "chapter-1.xhtml OEBPS/chapter-2.xhtml OEBPS/chapter-3.xhtml"; check "dropping after a row puts the file behind it" $?
    has "^editor: .*toast=\"Only files in the reading order can be reordered\""; check "a file outside the reading order cannot be dropped into it" $?
    I=$(grep "^editor_x: fileinfo:" <<<"$OUT")
    sed -n 1p <<<"$I" | grep -q "NotInManifest"; check "a file missing from the manifest is flagged" $?
    sed -n 2p <<<"$I" | grep -q "Package"; check "the package document has its own icon note" $?
    sed -n 3p <<<"$I" | grep -q "Nav"; check "the navigation document is marked" $?
    sed -n 4p <<<"$I" | grep -q "KB · application/xhtml+xml"; check "hovering a file shows its size and type" $?
    sed -n 5p <<<"$I" | grep -q "NonLinear" && ! sed -n 6p <<<"$I" | grep -q "NonLinear"; check "a page can be marked as not part of the main reading order and back" $?
    has "^editor_x: semantics: .*title-page=OEBPS/chapter-1.xhtml"; check "Use as Title Page sets the semantic" $?
    sed -n 7p <<<"$I" | grep -q "TitlePage"; check "and the file shows the title page mark" $?
    has "^editor_x: selected: OEBPS/chapter-1.xhtml OEBPS/chapter-2.xhtml OEBPS/chapter-3.xhtml"; check "Select all text files selects the whole category" $?
    has "^editor_x: selected: $"; check "Select none clears the selection" $?
    has "^editor_x: rowlabel: chapter-1.xhtml$" && has "^editor_x: rowlabel: OEBPS/chapter-1.xhtml$"; check "full paths can be shown in the file list" $?
    has "^editor_cat: .*NEW REPLACED"; check "Replace with file swaps the content under the new name" $?
    has "^editor_cat: .*swapped.xhtml"; check "and links follow the new name" $?
    has "^editor_x: templates: Poem.xhtml$"; check "templates in the configuration folder are found" $?
    has "^editor_cat: .*<title>verse</title>.*<h1>verse</h1>"; check "a new file from a user template uses its text" $?
    ! has "^a11y-problem:"; check "the image grid has accessible names" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_x_window "edit:1,wait:3000,editor:x:wrap=on,editor:x:wrap-status,editor:x:wrap=off,editor:x:wrap-status,editor:x:save-copy=@D@/copy.epub,wait:400,editor:x:put=OEBPS/chapter-1.xhtml|<html xmlns=\"$XP\"><head><title>T</title></head><body><p>\"Hi\" there</p></body></html>,editor:x:smarten=all|smarten,wait:400,editor:x:undo-ckpt,wait:400,editor:x:redo-ckpt,wait:400,editor:x:undo-ckpt,editor:x:undo-ckpt,editor:x:redo-ckpt,editor:cps,editor:open=OEBPS/chapter-1.xhtml,wait:200,editor:open=OEBPS/chapter-2.xhtml,wait:200,editor:open=OEBPS/chapter-3.xhtml,wait:200,editor:tabs,editor:open=OEBPS/chapter-2.xhtml,wait:300,editor:x:close-right,wait:300,editor:tabs,editor:x:close-others,wait:300,editor:tabs,editor:x:state-save,editor:x:state-dump,editor:x:open-names=chapter-3.xhtml|Paragraph 7 of,wait:300,editor:cur,editor:x:help,editor:x:status,editor:x:save-copy-edit=@D@/edit.epub,wait:1200,editor:x:status,editor:x:new-book=Fresh Start,wait:1200,editor:x:status,editor:dump,editor:x:import-file=$ROOT/xt/page.html,wait:1500,editor:x:status,editor:x:open-folder=$ROOT/css-epub/src,wait:1500,editor:x:status,editor:dump,editor:x:open-book=1,wait:800,editor:tabs,editor:x:status,quit" '
    has "^editor_x: wrap: WordChar" && has "^editor_x: wrap: None"; check "line wrapping can be switched on and off" $?
    [ -s "$d/copy.epub" ]; check "Save a copy writes the book to a file" $?
    has "^editor_cps: Before: Redo"; check "Revert to Before and After walk the checkpoints (a redo leaves a guard)" $?
    grep -q "UI Test Book" <<<"$LIB" && [ "$(grep -c "\"file\":" <<<"$LIB")" -ge 5 ]; check "Save a copy and edit it, New book, Import and Open folder add books to the library (original untouched)" $?
    T=$(grep "^editor_tabs:" <<<"$OUT")
    sed -n 1p <<<"$T" | grep -q "n=3"; check "three tabs are open" $?
    sed -n 2p <<<"$T" | grep -q "n=2"; check "Close tabs to the right keeps the open tab and those before it" $?
    sed -n 3p <<<"$T" | grep -q "n=1"; check "Close other tabs keeps only the open tab" $?
    has "^editor_x: state: tabs=.*current=Some"; check "the open tabs and cursor are remembered" $?
    has "^editor_cur: file=OEBPS/chapter-3.xhtml .*sel=\"Paragraph 7 of\""; check "opening named files can select a text" $?
    has "^editor_x: help: https://bookx.adhadse.com/editing/"; check "Help points at the wiki editing page" $?
    S=$(grep "^editor_x: status:" <<<"$OUT")
    sed -n 2p <<<"$S" | grep -q "book=[3-9]"; check "Save a copy and edit it continues with the copy" $?
    sed -n 3p <<<"$S" | grep -q "files=" ; check "a new book opens in the same window" $?
    grep "^editor_files:" <<<"$OUT" | grep -q "OEBPS/c1.xhtml"; check "an unzipped folder opens as a book" $?
    grep "^editor_tabs:" <<<"$OUT" | tail -1 | grep -q "selected=OEBPS/chapter-3.xhtml"; check "going back to a book restores its tabs" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_x_dialogs "edit:1,wait:3000,editor:x:add-png=orphan.png,editor:x:put=OEBPS/chapter-1.xhtml|<html xmlns=\"$XP\"><head><title>T</title></head><body><p style=\"color: red\">\"Hi\" there</p><a href=\"https://mock.example/ok.html\">x</a></body></html>,editor:x:dialog=smarten,wait:400,editor:x:dialog-click=preview,wait:200,editor:x:dialog-text,a11y-audit:,editor:x:dialog-click=apply,wait:600,editor:cat=OEBPS/chapter-1.xhtml,editor:x:dialog=styles,wait:400,a11y-audit:,editor:x:dialog=filter,wait:400,a11y-audit:,editor:x:dialog=unused-media,wait:400,editor:x:dialog-text,a11y-audit:,editor:x:dialog-click=apply,wait:600,editor:x:unused-media-list,editor:x:dialog=semantics,wait:400,a11y-audit:,editor:x:dialog=accessibility,wait:400,a11y-audit:,editor:x:dialog=fix-html,wait:400,a11y-audit:,editor:x:dialog=beautify,wait:400,a11y-audit:,editor:x:dialog=unused-css,wait:400,editor:x:dialog-text,editor:x:dialog=repair-links,wait:400,a11y-audit:,editor:x:dialog=check-links,wait:400,a11y-audit:,editor:x:dialog-click=apply,wait:1500,editor:x:dialog-text,editor:dump,editor:cmdpalette,wait:500,palette:type=Smarten,wait:300,palette:dump,palette:close,quit" '
    has "^editor_x: dialog: Smartened the punctuation in .* / "; check "the Smarten dialog previews the change with before and after lines" $?
    has "^editor_cat: .*“Hi” there"; check "Apply in the dialog runs the tool" $?
    has "^editor_x: dialog: .*orphan.png deleted"; check "the unused media dialog lists what it will delete" $?
    has "^editor_x: unused-media: $"; check "and deletes it on Apply" $?
    has "^editor_x: dialog: Every style rule is used|Removed unused rules"; check "the unused CSS dialog previews right away" $?
    has "^editor_x: dialog: 1 checked: 1 fine, 0 broken|^editor_x: dialog: .*checked"; check "Check external links runs in the background and reports in the dialog" $?
    ! has "^a11y-problem:"; check "every tool dialog has accessible names" $?
    has "^palette: .*Smarten Punctuation"; check "the tools are in the command palette" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_x_edit_cli "wait:2500,edit-file:$ROOT/css-epub/lib/styled.epub|c1.xhtml|slanted words,wait:2500,editor:cur,editor:tabs,quit" '
    has "^editor_cur: file=OEBPS/c1.xhtml .*sel=\"slanted words\""; check "bookx --edit FILE name --select-text TEXT opens the book in the editor, the file in a tab and selects the text" $?
    has "^editor_tabs: .*OEBPS/c1.xhtml"; check "the named file has its tab" $?
    grep -q "Styled Book" <<<"$LIB"; check "the book was added to the library" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_x_embed_dialog "edit:1,wait:3000,editor:x:write-font=$ROOT/xt/fonts/tiny.ttf,editor:x:put=OEBPS/chapter-2.xhtml|<html xmlns=\"$XP\"><head><title>T</title></head><body><p style=\"font-family: Tiny Test\">ABC</p></body></html>,wait:300,editor:x:dialog=embed-fonts,editor:x:dialog-text,wait:1200,editor:x:dialog-click=preview,wait:300,editor:x:dialog-text,a11y-audit:,editor:x:dialog-click=apply,wait:800,editor:dump,quit" '
    has "^editor_x: dialog: Searching the installed fonts|^editor_x: dialog: Press Preview"; check "the dialog opens at once while the fonts are searched in the background" $?
    has "^editor_x: dialog: Embedded 1 font"; check "when the search is done the font is offered and Preview says what would happen" $?
    has "^editor: .*toast=\"Embedded 1 font"; check "Apply embeds it" $?
    ! has "^a11y-problem:"; check "the dialog has accessible names" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_x_open_with "edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:300,editor:x:open-with-export,wait:1500,editor:x:external-pending,editor:x:external-accept,wait:600,editor:cat=OEBPS/chapter-1.xhtml,editor:dump,quit" '
    has "^editor_x: external: OEBPS/chapter-1.xhtml"; check "a file saved by the other program is noticed and waits for the answer" $?
    has "^editor_cat: .*EXTERNAL EDIT"; check "accepting puts the new content into the book" $?
    has "^editor: .*toast=\"Replaced chapter-1.xhtml|^editor: .*checkpoints=[1-9]"; check "the update is one undoable step" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'
