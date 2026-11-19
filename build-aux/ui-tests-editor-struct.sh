# Scenarios of the editor's structure tools: Contents drag and drop (CE-120), merge by dragging (CE-028), cover (CE-129),
# fonts (CE-420), Transform HTML (CE-421), jacket (CE-426), accessibility metadata (CE-427), books of other formats
# (CE-429). Sourced by ui-tests.sh (uses its `scenario`, `check`, `has`, $ROOT, $CLI).
# Dev commands are `editor:x:<name>=<args>` (src/ui/editor/struct_hub.rs and the files it names); a comma in a text is
# written §.

export BOOKX_FONT_DIRS="$ROOT/xt/fonts"
mkdir -p "$ROOT/xt/fonts" "$ROOT/xt/struct"
XPS='http://www.w3.org/1999/xhtml'

# books in other formats, made with the app's own converter from the generated markdown book
for f in fb2 azw3 mobi htmlz; do
    "$CLI" convert "$ROOT/src/book.md" "$ROOT/xt/struct/book.$f" >/dev/null 2>&1
done
"$CLI" convert "$ROOT/src/book.md" "$ROOT/xt/struct/book.kepub.epub" >/dev/null 2>&1
for f in fb2 azw3 htmlz; do cp "$ROOT/xt/struct/book.$f" "$ROOT/xt/struct/keep.$f"; done

scenario editor_struct_toc "edit:1,wait:3000,editor:toc-dump,editor:toc-select=1,wait:300,editor:x:toc-op=in,editor:toc-dump,editor:x:toc-drop=2|0|into,editor:toc-dump,editor:toc-select=2,wait:300,editor:x:toc-op=up,editor:toc-dump,editor:x:toc-op=out,editor:toc-dump,editor:x:toc-drop=2|0|before,editor:toc-dump,editor:x:toc-drop=0|0|after,editor:dump,editor:toc-dump,editor:x:toc-drop=1|0|into,editor:toc-dump,editor:x:toc-op=delete,editor:toc-dump,editor:x:toc-problems,editor:save,wait:800,editor:cat=OEBPS/nav.xhtml,quit" '
    T=$(grep "^editor_toc:" <<<"$OUT")
    sed -n 1p <<<"$T" | grep -q "titles=0:Chapter 1/0:Chapter 2/0:Chapter 3$"; check "the contents start flat" $?
    sed -n 2p <<<"$T" | grep -q "titles=0:Chapter 1/1:Chapter 2/0:Chapter 3$"; check "indent makes an entry a child of the one before" $?
    sed -n 3p <<<"$T" | grep -q "titles=0:Chapter 1/1:Chapter 2/1:Chapter 3$"; check "dropping an entry into another makes it the last child" $?
    sed -n 4p <<<"$T" | grep -q "titles=0:Chapter 1/1:Chapter 3/1:Chapter 2$"; check "move up swaps with the previous sibling" $?
    sed -n 5p <<<"$T" | grep -q "titles=0:Chapter 1/1:Chapter 2/0:Chapter 3$"; check "outdent moves an entry out, after its parent" $?
    sed -n 6p <<<"$T" | grep -q "titles=0:Chapter 3/0:Chapter 1/1:Chapter 2$"; check "dropping before an entry puts it first" $?
    has "^editor: .*toast=\"That entry cannot go there\""; check "a drop onto itself is refused" $?
    sed -n 7p <<<"$T" | grep -q "titles=0:Chapter 3/0:Chapter 1/1:Chapter 2$"; check "a drop onto itself changes nothing" $?
    sed -n 8p <<<"$T" | grep -q "titles=0:Chapter 3/1:Chapter 1/2:Chapter 2$"; check "dropping an entry with its children moves the whole subtree" $?
    sed -n 9p <<<"$T" | grep -q "titles=0:Chapter 3/1:Chapter 2$"; check "delete moves the children of the deleted entry up" $?
    has "^editor_x: toc-problems: $"; check "the contents have no problems" $?
    C=$(grep "^editor_cat:" <<<"$OUT" | head -1)
    grep -q "Chapter 3.*Chapter 2" <<<"$C" && ! grep -q "Chapter 1" <<<"$C" && grep -q "<ol>.*<li>.*<ol>" <<<"$C"; check "the saved navigation document nests the entries" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_struct_merge_drag "edit:1,wait:3000,editor:x:merge-zone=20|40,editor:x:merge-zone=2|40,editor:x:drop-merge=OEBPS/style.css|OEBPS/chapter-1.xhtml,editor:dump,editor:x:drop-merge-pending,editor:x:drop-merge=OEBPS/chapter-2.xhtml|OEBPS/chapter-1.xhtml,editor:x:drop-merge-pending,editor:x:drop-merge-accept,wait:600,editor:x:drop-merge-pending,editor:dump,editor:cat=OEBPS/chapter-1.xhtml,editor:cps,quit" '
    has "^editor_x: merge-zone: true"; check "the middle of a row means merge" $?
    has "^editor_x: merge-zone: false"; check "the edge of a row means reorder" $?
    has "^editor: .*toast=\"Only pages can be merged\""; check "a stylesheet cannot be merged by dragging" $?
    sed -n 1p <<<"$(grep "^editor_x: drop-merge-pending:" <<<"$OUT")" | grep -q "None"; check "nothing waits after a refused drop" $?
    sed -n 2p <<<"$(grep "^editor_x: drop-merge-pending:" <<<"$OUT")" | grep -q "chapter-2.xhtml.*chapter-1.xhtml"; check "a page dropped on a page waits for confirmation" $?
    sed -n 3p <<<"$(grep "^editor_x: drop-merge-pending:" <<<"$OUT")" | grep -q "None"; check "accepting uses the answer up" $?
    F=$(grep "^editor_files:" <<<"$OUT" | tail -1)
    ! grep -q "chapter-2.xhtml" <<<"$F" && grep -q "chapter-1.xhtml" <<<"$F"; check "the dragged page is merged into the target" $?
    grep "^editor_cat:" <<<"$OUT" | head -1 | grep -q "Paragraph 1 of chapter 2"; check "its text follows the target page" $?
    has "^editor_cps: Before: Merge Files$"; check "a checkpoint was taken before the merge" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_struct_cover "edit:1,wait:3000,editor:x:write-png=$ROOT/xt/struct/pic.png,editor:x:add-png=pic.png,editor:x:cover-state,editor:x:cover=existing:OEBPS/images/pic.png,wait:500,editor:x:cover-state,editor:x:cover=gen;crop=1;size=3,wait:800,editor:x:cover-state,editor:dump,editor:x:cover=file:$ROOT/xt/struct/pic.png;page=0,wait:500,editor:x:cover-state,editor:x:cover=file:/nonexistent/x.png,editor:dump,editor:x:cover=file:$ROOT/src/book.md,editor:dump,editor:x:cover=remove,wait:500,editor:x:cover-state,editor:cps,quit" '
    S=$(grep "^editor_x: cover-state:" <<<"$OUT")
    sed -n 2p <<<"$S" | grep -q "image=OEBPS/images/pic.png page=OEBPS/cover.xhtml semantics=OEBPS/cover.xhtml .*first-spine=OEBPS/cover.xhtml"; check "an image of the book becomes the cover with a cover page, guide and landmark entry" $?
    sed -n 2p <<<"$S" | grep -q "cover-image-items=[12] "; check "the package marks one cover image" $?
    sed -n 3p <<<"$S" | grep -q "image=OEBPS/images/cover.png page=OEBPS/cover.xhtml"; check "a drawn cover replaces it and the same page shows it" $?
    sed -n 3p <<<"$S" | grep -qv "pic.png,"; check "the old cover image is deleted when nothing uses it" $?
    has "^editor_files: .*OEBPS/images/cover.png"; check "the new cover is in the file list" $?
    ! grep "^editor_files:" <<<"$OUT" | head -1 | grep -q "OEBPS/images/pic.png"; check "the old image is gone from the file list" $?
    sed -n 4p <<<"$S" | grep -q "image=OEBPS/images/cover-2.png"; check "a file from the computer becomes the cover" $?
    has "^editor: .*toast=\"cannot read /nonexistent"; check "a missing file is refused with a message" $?
    has "^editor: .*toast=\".*not an image"; check "a file that is not an image is refused" $?
    sed -n 5p <<<"$S" | grep -q "semantics= "; check "removing the cover clears the guide and landmarks" $?
    has "^editor_cps: Before: Set Cover/Before: Set Cover/Before: Set Cover/Before: Remove Cover$"; check "every cover change took a checkpoint" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_struct_fonts "edit:1,wait:3000,editor:x:write-font=$ROOT/xt/fonts/tiny.ttf,editor:x:put=OEBPS/chapter-2.xhtml|<html xmlns=\"$XPS\"><head><title>T</title></head><body><p style=\"font-family: Tiny Test\">ABC</p></body></html>,editor:x:embed=Tiny Test|subset,wait:500,editor:x:fonts-manage=list,editor:x:fonts-manage=rename=Tiny Test|Pocket Serif,wait:500,editor:cat=OEBPS/style.css,editor:cat=OEBPS/chapter-2.xhtml,editor:x:fonts-manage=list,editor:x:fonts-manage=sample=OEBPS/fonts/tiny-test-Regular.ttf|ABC,editor:x:fonts-manage=rename=Pocket Serif|serif,editor:dump,editor:x:fonts-manage=remove=OEBPS/fonts/tiny-test-Regular.ttf,wait:500,editor:cat=OEBPS/style.css,editor:x:fonts-manage=list,editor:cps,quit" '
    L=$(grep "^editor_x: fonts-manage: " <<<"$OUT")
    sed -n 1p <<<"$L" | grep -q "[Tt]iny [Tt]est|OEBPS/fonts/tiny-test-Regular.ttf|[0-9]*|[1-9][0-9]*|false"; check "the embedded font is listed with its family, size and use" $?
    grep "^editor_cat:" <<<"$OUT" | sed -n 1p | grep -q "Pocket Serif" && ! grep "^editor_cat:" <<<"$OUT" | sed -n 1p | grep -q "Tiny Test"; check "renaming changes the @font-face rule" $?
    grep "^editor_cat:" <<<"$OUT" | sed -n 2p | grep -q "Pocket Serif"; check "and the inline style that used the family" $?
    sed -n 2p <<<"$L" | grep -q "^editor_x: fonts-manage: Pocket Serif|"; check "the list shows the new name" $?
    has "^editor_x: fonts-manage: sample=[1-9][0-9]* texture=(true|false)"; check "a vector sample of the real glyphs is made" $?
    has "^editor: .*toast=.*is a generic family"; check "a generic family name is refused" $?
    grep "^editor_cat:" <<<"$OUT" | sed -n 3p | grep -qv "@font-face"; check "removing a font deletes its @font-face rule" $?
    has "^editor_x: fonts-manage: $"; check "no font is left" $?
    has "^editor_cps: Before: Embed Fonts?.*Before: Rename Font/Before: Remove Font$|Before: Rename Font/Before: Remove Font$"; check "rename and remove took checkpoints" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_struct_html "edit:1,wait:3000,editor:x:put=OEBPS/chapter-1.xhtml|<html xmlns=\"$XPS\" xmlns:epub=\"http://www.idpf.org/2007/ops\"><head><title>T</title></head><body><p><b class=\"x\">bold</b> &amp; <i>it</i></p><div class=\"ad\">AD</div><span class=\"s\">keep me</span><section epub:type=\"chapter\"><p>in</p></section></body></html>,editor:x:put=OEBPS/chapter-3.xhtml|<html><body><p>broken<b></body>,editor:x:html-rules=rename|b|strong;;remove|div.ad;;unwrap|span;;set-attr|p|lang|en~~preview,editor:x:html-rules=rename|b|strong;;remove|div.ad;;unwrap|span;;set-attr|p|lang|en;;replace-class|strong|x|y,wait:500,editor:cat=OEBPS/chapter-1.xhtml,editor:x:html-rules=set-attr|//section|epub:type|bodymatter chapter~~current,wait:400,editor:cat=OEBPS/chapter-1.xhtml,editor:x:html-rules=rename|b|<script>,editor:dump,editor:x:html-rules=rename|b|em,editor:x:html-ruleset=save|Mine|add-class|p|k;;wrap|img|div,editor:x:html-ruleset=list,editor:x:html-ruleset=load|Mine,wait:400,editor:cat=OEBPS/chapter-1.xhtml,editor:x:html-ruleset=delete|Mine,editor:x:html-ruleset=list,editor:cps,quit" '
    has "^editor_x: html-preview: changed=2 skipped=1 total=[1-9]"; check "the dry run lists the changed page and the page it cannot read, and changes nothing" $?
    C=$(grep "^editor_cat:" <<<"$OUT")
    sed -n 1p <<<"$C" | grep -q "<strong class=\"y\">bold</strong> &amp; <i>it</i>"; check "rename keeps entities and other tags, class rules follow" $?
    sed -n 1p <<<"$C" | grep -q "<p lang=\"en\">" && ! sed -n 1p <<<"$C" | grep -q "AD</div>" && ! sed -n 1p <<<"$C" | grep -q "<span"; check "remove, unwrap and set attribute worked" $?
    sed -n 1p <<<"$C" | grep -q "keep me"; check "unwrap kept the content" $?
    sed -n 2p <<<"$C" | grep -q "epub:type=\"bodymatter chapter\""; check "a namespaced attribute is set as written" $?
    has "^editor: .*toast=.*is not a tag name"; check "a bad tag name is refused" $?
    has "^editor_x: html-rulesets: Mine\(2\)$"; check "a rule set is saved" $?
    sed -n 3p <<<"$C" | grep -q "<p lang=\"en\" class=\"k\">"; check "a loaded rule set is applied" $?
    has "^editor_x: html-rulesets: $"; check "a rule set can be deleted" $?
    has "^editor_cps: Before: Transform HTML/Before: Transform HTML/Before: Transform HTML$"; check "each run took a checkpoint" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_struct_jacket "edit:1,wait:3000,editor:x:jacket=state,editor:x:jacket=add|comments=0,wait:400,editor:x:jacket=state,editor:cat=OEBPS/bookx-jacket.xhtml,editor:x:jacket=set-title|A New Title,wait:600,editor:x:jacket=state,editor:cat=OEBPS/bookx-jacket.xhtml,editor:x:jacket=remove,wait:400,editor:x:jacket=state,editor:dump,editor:cps,quit" '
    S=$(grep "^editor_x: jacket:" <<<"$OUT")
    sed -n 1p <<<"$S" | grep -q "page= "; check "a book starts without a jacket" $?
    sed -n 2p <<<"$S" | grep -q "page=OEBPS/bookx-jacket.xhtml first-spine=OEBPS/bookx-jacket.xhtml stale=false options=cover=1 series=1 tags=1 rating=1 comments=0"; check "the jacket is added first with the chosen options remembered in its marker" $?
    grep "^editor_cat:" <<<"$OUT" | sed -n 1p | grep -q "UI Test Book" && grep "^editor_cat:" <<<"$OUT" | sed -n 1p | grep -q "Test Author"; check "it shows the title and author" $?
    grep "^editor_cat:" <<<"$OUT" | sed -n 2p | grep -q "A New Title"; check "changing the metadata in the editor updates the jacket" $?
    sed -n 3p <<<"$S" | grep -q "stale=false"; check "the updated jacket is current" $?
    sed -n 4p <<<"$S" | grep -q "page= first-spine=OEBPS/chapter-1.xhtml"; check "removing it restores the reading order" $?
    ! grep "^editor_files:" <<<"$OUT" | grep -q jacket; check "and leaves no file behind" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_struct_a11y "edit:1,wait:3000,editor:x:a11y-form=suggest,editor:x:a11y-form=preset|text,wait:400,editor:x:a11y-form=read,editor:x:a11y-form=validate,editor:x:a11y-form=set|features=alternativeText:MathML|hazards=none:sound|by=Lab,wait:300,editor:x:a11y-form=set|features=alternativeText|hazards=noSoundHazard|by=Access Lab|cred=Certified|report=https://example.org/r.html|conf=EPUB Accessibility 1.1 - WCAG 2.1 Level AA,wait:400,editor:x:a11y-form=read,editor:x:a11y-form=validate,editor:x:a11y-form=set|features=bogus,editor:dump,editor:cps,quit" '
    has "^editor_x: a11y-form: summary=This publication is text only.*modes=textual .*features=.*readingOrder.*hazards=noSoundHazard"; check "suggestions come from the book (text only, with a reading order)" $?
    has "^editor_notes|^editor_x: a11y-notes: .*cannot be detected"; check "the suggestions say what cannot be detected" $?
    has "^editor_x: a11y-form: summary=This publication is a text-only book.*features=.*tableOfContents"; check "a preset is written" $?
    has "^editor_x: a11y-form: .*by=Access Lab report=https://example.org/r.html"; check "the certifier is written and read back" $?
    has "^editor_x: a11y-validate: errors=false .*alternativeText.*no images"; check "a claim the book contradicts is reported" $?
    has "^editor: .*toast=.*accessibilityFeature: .*bogus.* is not"; check "a value outside the vocabulary is refused" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_struct_foreign "edit:1,wait:3000,editor:x:foreign-open=$ROOT/xt/struct/book.fb2,wait:1500,editor:x:foreign-status,editor:x:put=OEBPS/chapter-1.xhtml|<html xmlns=\"$XPS\"><head><title>T</title></head><body><p>EDITED IN FB2 SESSION</p></body></html>,editor:x:foreign-save=prompt,editor:x:foreign-save=epub|$ROOT/xt/struct/out-fb2,wait:600,editor:x:foreign-save=back,wait:1500,editor:x:foreign-status,editor:x:foreign-open=$ROOT/xt/struct/book.kepub.epub,wait:1500,editor:x:foreign-status,editor:x:foreign-open=$ROOT/xt/struct/book.azw3,wait:2500,editor:x:foreign-status,editor:x:foreign-open=$ROOT/xt/struct/nothing.pdf,wait:300,editor:dump,quit" '
    S=$(grep "^editor_x: foreign:" <<<"$OUT")
    sed -n 1p <<<"$S" | grep -q "origin=.*book.fb2 format=fb2"; check "an FB2 book opens for editing through a temporary EPUB" $?
    [ -f "$ROOT/xt/struct/out-fb2.epub" ]; check "Save as EPUB writes an EPUB next to the chosen name" $?
    [ "$(head -c2 "$ROOT/xt/struct/out-fb2.epub" 2>/dev/null)" = PK ]; check "and it is a real EPUB" $?
    [ -f "$ROOT/xt/struct/book.fb2.bak" ]; check "Convert back keeps the old file as a backup" $?
    grep -q "<FictionBook" "$ROOT/xt/struct/book.fb2" && grep -q "EDITED IN FB2 SESSION" "$ROOT/xt/struct/book.fb2"; check "the original now holds the edit in the original format" $?
    ! grep -q "EDITED IN FB2 SESSION" "$ROOT/xt/struct/book.fb2.bak"; check "the backup is the old file" $?
    sed -n 2p <<<"$S" | grep -q "format=fb2 dirty=false"; check "after saving the book is clean" $?
    sed -n 3p <<<"$S" | grep -q "format=kepub"; check "a KEPUB is opened as it is" $?
    sed -n 4p <<<"$S" | grep -q "format=azw3"; check "an AZW3 book opens too" $?
    has "^editor: .*toast=\"Could not open that book"; check "a file Bookx cannot edit is refused with a message" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# every dialog of this file with the accessibility audit, at two window sizes; BOOKX_SHOTS=<dir> keeps screenshots
SHOTS=${BOOKX_SHOTS:-$ROOT/shots}
mkdir -p "$SHOTS"
dlg() { # NAME: open the dialog, audit it, take two screenshots (1280x800 and 900x600), close it
    printf 'editor:x:win-size=1280x800,wait:500,editor:x:dialog=%s,wait:700,a11y-audit:,editor:x:dialog-shot=%s/%s-1280.png,wait:700,editor:x:dialog-close,wait:300,editor:x:win-size=900x600,wait:500,editor:x:dialog=%s,wait:700,a11y-audit:,editor:x:dialog-shot=%s/%s-900.png,wait:700,editor:x:dialog-close,wait:300,' "$1" "$SHOTS" "$1" "$1" "$SHOTS" "$1"
}
scenario editor_struct_dialogs "edit:1,wait:3000,editor:x:animations=off,editor:x:add-png=pic.png,editor:x:write-font=$ROOT/xt/fonts/tiny.ttf,editor:x:put=OEBPS/chapter-2.xhtml|<html xmlns=\"$XPS\"><head><title>T</title></head><body><p style=\"font-family: Tiny Test\">ABC</p></body></html>,editor:x:embed=Tiny Test|subset,wait:400,$(dlg cover)$(dlg fonts)$(dlg transform-html)editor:x:dialog=transform-html,wait:500,editor:x:dialog-click=preview,wait:300,editor:x:dialog-text,editor:x:dialog-close,$(dlg jacket)$(dlg a11y-form)editor:x:dialog=jacket,wait:500,editor:x:dialog-click=preview,wait:300,editor:x:dialog-text,editor:x:dialog-close,editor:x:dialog=a11y-form,wait:500,editor:x:dialog-click=preview,wait:300,editor:x:dialog-text,editor:x:dialog-close,editor:dump,quit" '
    [ "$(grep -c "^a11y:" <<<"$OUT")" -ge 10 ]; check "the audit ran with each dialog open at both sizes" $?
    ! has "^a11y-problem:"; check "every control of the structure dialogs has an accessible name" $?
    has "^editor_x: dialog: Add at least one rule"; check "the Transform HTML dialog refuses to preview without a rule" $?
    has "^editor_x: dialog: Added the book jacket OEBPS/bookx-jacket.xhtml"; check "the Book Jacket dialog previews what it will add" $?
    has "^editor_x: dialog: The accessibility metadata is already like that"; check "the Accessibility Metadata dialog previews that an untouched form changes nothing" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'
