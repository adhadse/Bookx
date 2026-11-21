# Scenarios of highlight anchoring (docs/bugs.md "imported highlights painted at the wrong place"): a highlight is
# anchored by its text in the reader's own numbering of the chapter. Sourced by ui-tests.sh (uses its `scenario`,
# `check`, `has`, $ROOT, $CLI). Dev command: `reader:hl-paint` prints `hl_text_numbering: same=..` (does the canvas
# number the chapter as `Doc::reader_text` does) and, per stored highlight, `hl_paint: text=.. painted=.. same=..
# unanchored=..` (the text read back from the page at the stored range against the stored text).

# the first offset of "Paragraph K of chapter 1": in the reader's text (one line break per block) or in the export text
# (blank lines between blocks, the numbering the bug used): `hl_offset reader|export K`
hl_offset() {
    python3 -c '
import sys
mode, k = sys.argv[1], int(sys.argv[2])
line = lambda j: "Paragraph %d of chapter 1 talks about functionality, readers and the world around them in plain words." % j
sep = 1 if mode == "reader" else 2
print(len("Chapter 1") + sep + sum(len(line(j)) + sep for j in range(1, k)))' "$1" "$2"
}

# Kindle clippings: three late paragraphs of chapter 1 that devices write a little differently, and one that is not in the book
hl_anchor_fixture() {
    mkdir -p "$1/imp"
    printf '\xef\xbb\xbfUI Test Book (Test Author)\r\n- Your Highlight on page 3 | Location 30-31 | Added on Monday, 3 March 2014 10:42:11\r\n\r\nParagraph 20 of chapter 1 talks about functionality, readers and the world around them in plain words.\r\n==========\r\nUI Test Book (Test Author)\r\n- Your Highlight on Location 40 | Added on Monday, 3 March 2014 10:50:00\r\n\r\nParagraph 24 of chapter 1 talks about functionality,   readers and the world around them in plain words.\r\n==========\r\nUI Test Book (Test Author)\r\n- Your Highlight on Location 41 | Added on Monday, 3 March 2014 10:51:00\r\n\r\nparagraph 9 of chapter 1 TALKS about functionality, readers and the world around them in plain words.\r\n==========\r\nUI Test Book (Test Author)\r\n- Your Highlight on Location 50 | Added on Monday, 3 March 2014 10:52:00\r\n\r\nMISSING quote that the book never says anywhere\r\n==========\r\n' >"$1/imp/My Clippings.txt"
}

# a library as an older Bookx left it: three highlights of chapter 1, one with the export numbering (the bug), one right,
# one whose text is not in the book at all
hl_repair_fixture() {
    local d=$1
    "$CLI" --library "$d/lib" library add "$ROOT/src/book.md" >/dev/null 2>&1
    python3 - "$d/lib" "$(hl_offset reader 5)" "$(hl_offset export 20)" "$(hl_offset reader 20)" <<'PY'
import glob, json, sqlite3, sys, zipfile, re
lib, real5, old20, real20 = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4])
epub = glob.glob(lib + "/**/*.epub", recursive=True)[0]
z = zipfile.ZipFile(epub)
opf = [n for n in z.namelist() if n.endswith(".opf")][0]
x = z.read(opf).decode()
base = opf.rsplit("/", 1)[0] + "/" if "/" in opf else ""
items = dict(re.findall(r'<item[^>]*?id="([^"]+)"[^>]*?href="([^"]+)"', x)) or {}
items.update({i: h for h, i in re.findall(r'<item[^>]*?href="([^"]+)"[^>]*?id="([^"]+)"', x)})
spine = [items[i] for i in re.findall(r'<itemref[^>]*?idref="([^"]+)"', x)]
idx = next(i for i, h in enumerate(spine) if b"Paragraph 20 of chapter 1" in z.read(base + h))
line = lambda j: "Paragraph %d of chapter 1 talks about functionality, readers and the world around them in plain words." % j
mk = lambda i, s, a, b, t, n: {"id": i, "spine": s, "start": a, "end": b, "color": "yellow", "style": "", "note": n, "text": t, "created": "2026-08-17T11:22:15Z"}
c = sqlite3.connect(lib + "/bookx-library.db")
(data,) = c.execute("select data from books where id=1").fetchone()
d = json.loads(data)
d["highlights"] = [
    mk("bx-shifted", idx, old20, old20 + len(line(20)), line(20), "shifted note"),
    mk("bx-right", idx, real5, real5 + len(line(5)), line(5), ""),
    mk("bx-lost", idx, 30, 50, "A sentence that this book never had at all, anywhere.", "lost note"),
]
c.execute("update books set data=? where id=1", (json.dumps(d),))
c.commit()
PY
}

scenario hl_anchor 'library:import-highlights=@D@/imp/My Clippings.txt,wait:3500,library:import-confirm,wait:1500,read:1,wait:3500,reader:hl-paint,quit' '
    has "^import_plan: found=4 new=4 notes=0 already=0 unplaced=1"; check "three quotes are found in the book, the fourth is kept as a note without a place" $?
    has "^import_summary: UI Test Book: 3 highlights added, 1 not found in the book’s text, added as notes without a place"; check "the review says which quotes have no place" $?
    has "^import_review: label=\"Add 4 Highlights\""; check "the review button says how many highlights it adds" $?
    has "^hl_text_numbering: same=true"; check "the page numbers the chapter text as Doc::reader_text does" $?
    [ "$(grep -c "^hl_paint: .* painted=true same=true unanchored=false" <<<"$OUT")" -eq 3 ]; check "the text painted at the stored range of each imported highlight is the highlight text" $?
    [ "$(grep -c "^hl_paint: .* painted=false same=false unanchored=true" <<<"$OUT")" -eq 1 ]; check "the quote that is nowhere is not painted anywhere" $?
    grep -q "\"start\": $(hl_offset reader 20)," <<<"$LIB"; check "the stored start is the reader number of the paragraph, not the export number" $?
    ! grep -q "\"start\": $(hl_offset export 20)," <<<"$LIB"; check "the export numbering is not stored" $?
    grep -q "\"start\": $(hl_offset reader 24)," <<<"$LIB"; check "a quote with different spacing is placed exactly" $?
    grep -q "\"start\": $(hl_offset reader 9)," <<<"$LIB"; check "a quote with different case is placed exactly" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario hl_repair 'wait:2500,read:1,wait:3500,reader:hl-paint,quit' '
    grep -q "\"start\": $(hl_offset reader 20)," <<<"$LIB"; check "a highlight stored with the export numbering is moved to where its text is" $?
    ! grep -q "\"start\": $(hl_offset export 20)," <<<"$LIB"; check "the wrong numbers are gone from the record" $?
    grep -q "\"start\": $(hl_offset reader 5)," <<<"$LIB"; check "a highlight that was right is untouched" $?
    grep -q "\"start\": -31," <<<"$LIB" && grep -q "lost note" <<<"$LIB"; check "a highlight whose text is nowhere keeps its note and old offsets, with no place" $?
    [ "$(grep -c "^hl_paint: .* painted=true same=true unanchored=false" <<<"$OUT")" -eq 2 ]; check "the two that have a place are painted with their own text" $?
    [ "$(grep -c "^hl_paint: .* painted=false same=false unanchored=true" <<<"$OUT")" -eq 1 ]; check "the one without a place is not painted" $?
    grep -q "RepairHighlights" "$d/lib/.bookx/oplog.json"; check "the repair is one step in the operation log (undo restores the old records)" $?
    [ -f "$d/lib/.bookx/highlight-anchors-v1" ]; check "the pass runs once per library" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# Colours and styles (docs/bugs.md bug 12): a Kobo database with a highlight in each of its four colours (and a dog-ear) and a
# KOReader notes file with colours and drawers; the reader paints each in its own colour and style.
hl_colors_fixture() {
    local d=$1
    mkdir -p "$d/imp/dev/.kobo" "$d/imp/dev/Books/UI Test Book.sdr"
    python3 - "$d/imp/dev/.kobo/KoboReader.sqlite" "$HERE/../crates/bookx-serve/tests/fixtures/kobo-schema-176.sql" <<'PY'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
c.executescript(open(sys.argv[2]).read())
c.execute("INSERT INTO content (ContentID, ContentType, MimeType, Title, Attribution, ___UserID) VALUES ('file:///mnt/onboard/UI Test Book.epub','6','application/epub+zip','UI Test Book','Test Author','')")
line = lambda j: "Paragraph %d of chapter 1 talks about functionality, readers and the world around them in plain words." % j
for i, (para, color, kind) in enumerate([(2, 0, "highlight"), (3, 1, "highlight"), (4, 2, "highlight"), (5, 3, "highlight"), (6, 0, "dogear")]):
    c.execute("INSERT INTO Bookmark (BookmarkID, VolumeID, ContentID, StartContainerPath, StartContainerChildIndex, StartOffset, EndContainerPath, EndContainerChildIndex, EndOffset, Text, Annotation, DateCreated, Type, Color) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
              (str(i), 'file:///mnt/onboard/UI Test Book.epub', 'c', 'span#kobo.%d.1' % para, 0, 12, 'span#kobo.%d.1' % para, 0, 70, line(para), '', '2026-11-01T08:00:00Z', kind, color))
c.commit()
PY
    local ent=''
    n=0
    for spec in "8 red underscore" "9 olive strikeout" "10 purple invert"; do
        set -- $spec
        n=$((n + 1))
        ent+=" [$n] = { [\"text\"] = \"Paragraph $1 of chapter 1 talks about functionality, readers and the world around them in plain words.\", [\"color\"] = \"$2\", [\"drawer\"] = \"$3\", [\"datetime\"] = \"2026-08-17 11:22:1$n\" },\n"
    done
    printf 'return {\n ["annotations"] = {\n'"$ent"' },\n ["doc_props"] = { ["title"] = "UI Test Book" },\n}\n' >"$d/imp/dev/Books/UI Test Book.sdr/metadata.epub.lua"
}

scenario hl_colors 'library:import-highlights=@D@/imp/dev,wait:3500,library:import-confirm,wait:1500,read:1,wait:3500,reader:hl-looks,quit' '
    has "^import_plan: found=7 new=7"; check "the Kobo database and the KOReader file give seven highlights (the dog-ear is not one)" $?
    L=$(grep "^hl_look:" <<<"$OUT")
    look() { grep "Paragraph $1 of" <<<"$L" | sed -E "s/.*color=([^ ]+) style=([^ ]+).*/\\1 \\2/"; }
    [ "$(look 2)" = "#ffd91f background" ]; check "Kobo colour 0 is painted yellow" $?
    [ "$(look 3)" = "#f76fb7 background" ]; check "Kobo colour 1 is painted pink" $?
    [ "$(look 4)" = "#408cff background" ]; check "Kobo colour 2 is painted blue" $?
    [ "$(look 5)" = "#33cc6b background" ]; check "Kobo colour 3 is painted green" $?
    [ "$(look 8)" = "#ff6152 underline" ]; check "a KOReader red underscore is painted red and underlined" $?
    [ "$(look 9 | cut -d" " -f2)" = "strikethrough" ] && [ "$(look 9 | cut -d" " -f1)" != "#ffd91f" ]; check "a KOReader olive strikeout is painted struck through in its own colour" $?
    [ "$(look 10)" = "#b873f2 invert" ]; check "a KOReader purple invert is painted purple, inverted" $?
    grep -q "\"color\": \"olive\"" <<<"$LIB"; check "a colour Bookx has no swatch for is stored as the device wrote it" $?
    ! grep -q "Paragraph 6 of chapter 1" <<<"$LIB"; check "the dog-ear is not imported" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'
