#!/usr/bin/env bash
# End-to-end UI tests, run IN PARALLEL.
#
# Every scenario gets its own headless Broadway display, its own D-Bus session (GApplication is
# single-instance per session), its own config/cache and its own library, so scenarios cannot
# interfere with each other. They drive the real GTK widgets through BOOKX_DEV_SCRIPT (see
# _developer/docs.md) and assert on what the app prints or persists.
#
#   build-aux/ui-tests.sh            # build (debug) and run all scenarios concurrently
#   NET=0 build-aux/ui-tests.sh      # skip the scenarios that need internet
#   JOBS=16 build-aux/ui-tests.sh    # scenarios running at once (default: number of CPU cores; 0 = all)
#   ONLY='notes translate' build-aux/ui-tests.sh   # run just these scenarios
#   SHOW=notes ONLY=notes build-aux/ui-tests.sh     # also print everything the app said in that scenario
#
# Why one *process* per scenario: GTK is single threaded, and `#[gtk::test]` runs all GTK tests
# serially on one main thread. Relm4's own tests are likewise one test binary (= one process) each.
# Separate processes with separate displays and D-Bus sessions are the way to use many cores.
set -uo pipefail
cd "$(dirname "$(readlink -f "$0")")/.."

for t in gtk4-broadwayd dbus-run-session; do
    command -v "$t" >/dev/null || { echo "$t not found — skipping UI tests"; exit 0; }
done
cargo build --quiet -p bookx -p bookx-cli || exit 1
cargo build --quiet -p bookx-tts --example fake_install || exit 1
cargo build --quiet -p bookx-plugin --example make_test_plugin --example make_test_plugins || exit 1
PLUG=${CARGO_TARGET_DIR:-$PWD/target}/debug/examples/make_test_plugin
FAKEPACK=${CARGO_TARGET_DIR:-$PWD/target}/debug/examples/fake_install
PLUGS=${CARGO_TARGET_DIR:-$PWD/target}/debug/examples/make_test_plugins
JOBS=${JOBS:-$(nproc 2>/dev/null || echo 4)}
[ "$JOBS" = 0 ] && JOBS=100000
BIN=${CARGO_TARGET_DIR:-$PWD/target}/${PROFILE:-debug}/bookx # PROFILE=release runs the performance scenarios on the release build
# the catalogue is a SQLite database; the CLI dumps it as JSON for the checks
CLI=${CARGO_TARGET_DIR:-$PWD/target}/debug/bookx-cli
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(mktemp -d)
RES=$ROOT/results
mkdir -p "$RES"
trap 'pkill -P $$ 2>/dev/null; rm -rf "$ROOT"' EXIT

# A small book, generated through the app's own Markdown importer.
mkdir -p "$ROOT/src"
{
    printf -- '---\ntitle: "UI Test Book"\nauthor: "Test Author"\nlanguage: en\n---\n'
    for c in 1 2 3; do
        printf '# Chapter %s\n\n' "$c"
        for p in $(seq 1 25); do
            printf 'Paragraph %s of chapter %s talks about functionality, readers and the world around them in plain words.\n\n' "$p" "$c"
        done
    done
} >"$ROOT/src/book.md"

# an EPUB whose look comes from its own CSS classes
EP="$ROOT/css-epub"
mkdir -p "$EP/src/META-INF" "$EP/src/OEBPS" "$EP/lib"
printf 'application/epub+zip' >"$EP/src/mimetype"
cat >"$EP/src/META-INF/container.xml" <<'XML'
<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>
XML
cat >"$EP/src/OEBPS/content.opf" <<'XML'
<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">css-test</dc:identifier><dc:title>Styled Book</dc:title><dc:creator>Css Author</dc:creator><dc:language>en</dc:language></metadata><manifest><item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/><item id="css" href="s.css" media-type="text/css"/></manifest><spine><itemref idref="c1"/></spine></package>
XML
cat >"$EP/src/OEBPS/s.css" <<'CSS'
.it { font-style: italic }
.bd { font-weight: bold }
.mid { text-align: center }
.gone { display: none }
body { background: #fdf6e3 }
.red { color: #cc0000 }
.ind { text-indent: 2em; margin: 0 }
CSS
cat >"$EP/src/OEBPS/c1.xhtml" <<'XML'
<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml"><head><title>c</title><link rel="stylesheet" href="s.css" type="text/css"/></head><body><h1>Styled Chapter</h1><p>Plain <span class="it">slanted words</span> and <span class="bd">heavy words</span>.</p><p class="mid">A centred line of text.</p><p class="gone">THIS TEXT IS HIDDEN BY THE BOOK</p><p class="ind">An indented paragraph of the book, packed tight.</p><p class="ind">A second indented paragraph right after it.</p><p class="red">Red words of the book.</p><p>The end.</p></body></html>
XML
(cd "$EP/src" && zip -q -X -0 "$EP/lib/styled.epub" mimetype && zip -q -X -r "$EP/lib/styled.epub" META-INF OEBPS)

mkdir -p "$ROOT/extra"
printf 'p { margin: 0 }\n' >"$ROOT/extra/extra.css"

# an EPUB with a drop cap and a floated image
FL="$ROOT/float-epub"
mkdir -p "$FL/src/META-INF" "$FL/src/OEBPS" "$FL/lib"
printf 'application/epub+zip' >"$FL/src/mimetype"
cp "$EP/src/META-INF/container.xml" "$FL/src/META-INF/"
sed 's/css-test/float-test/; s/Styled Book/Floating Book/; s/Css Author/Float Author/; s/<item id="css"[^>]*\/>/<item id="css" href="s.css" media-type="text\/css"\/><item id="img" href="p.png" media-type="image\/png"\/>/' "$EP/src/OEBPS/content.opf" >"$FL/src/OEBPS/content.opf"
cat >"$FL/src/OEBPS/s.css" <<'CSS'
p.intro::first-letter { float: left; font-size: 3em; font-weight: bold }
img.pic { float: left; margin-right: 1em }
CSS
python3 - "$FL/src/OEBPS/p.png" <<'PY'
import struct, sys, zlib
w = h = 90
raw = b"".join(b"\x00" + b"\x30\x60\xc0" * w for _ in range(h))
def chunk(t, d):
    c = struct.pack(">I", len(d)) + t + d
    return c + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
open(sys.argv[1], "wb").write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))
PY
{
    printf '<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml"><head><title>c</title><link rel="stylesheet" href="s.css" type="text/css"/></head><body><h1>Floating Chapter</h1>'
    printf '<p class="intro">Once upon a time there was a long paragraph that needs several lines of text so that the big initial letter can sit beside the first of them and the text wraps around it nicely, the way that books in print do it.</p>'
    printf '<p><img class="pic" src="p.png" alt="pic"/>%s</p>' "The picture floats to the left of this paragraph and the words of the paragraph flow around it, line after line, until the picture ends and the text goes back to the full width of the column. This needs quite a lot of words to reach that far down the page."
    for i in 1 2 3; do printf '<p>A following paragraph number %s, long enough to need more than one line in the column of the reader, so that the layout has something to do after the picture.</p>' "$i"; done
    printf '</body></html>'
} >"$FL/src/OEBPS/c1.xhtml"
(cd "$FL/src" && zip -q -X -0 "$FL/lib/floats.epub" mimetype && zip -q -X -r "$FL/lib/floats.epub" META-INF OEBPS)

# a right-to-left EPUB (Arabic and Hebrew, page-progression-direction="rtl")
RT="$ROOT/rtl-epub"
mkdir -p "$RT/src/META-INF" "$RT/src/OEBPS" "$RT/lib"
printf 'application/epub+zip' >"$RT/src/mimetype"
cp "$EP/src/META-INF/container.xml" "$RT/src/META-INF/"
cat >"$RT/src/OEBPS/content.opf" <<'XML'
<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">rtl-test</dc:identifier><dc:title>Right To Left Book</dc:title><dc:creator>Rtl Author</dc:creator><dc:language>ar</dc:language></metadata><manifest><item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/></manifest><spine page-progression-direction="rtl"><itemref idref="c1"/></spine></package>
XML
python3 - "$RT/src/OEBPS/c1.xhtml" <<'PY'
import sys
ar = "هذا نص تجريبي طويل للقراءة من اليمين إلى اليسار، ويجب أن تنتقل الصفحات في هذا الكتاب من اليسار إلى اليمين كما في الكتب العربية."
he = "זהו טקסט ניסיון ארוך לקריאה מימין לשמאל, והדפים בספר הזה מתהפכים בכיוון ההפוך כמו בספרים בעברית."
body = "".join(f'<p>{i}. {ar if i % 2 else he}</p>' for i in range(1, 60))
open(sys.argv[1], "w").write(f'<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml" dir="rtl" lang="ar"><head><title>c</title></head><body dir="rtl"><h1>الفصل الأول</h1>{body}</body></html>')
PY
(cd "$RT/src" && zip -q -X -0 "$RT/lib/rtl.epub" mimetype && zip -q -X -r "$RT/lib/rtl.epub" META-INF OEBPS)

# vertical writing: a Japanese book (vertical-rl, ruby, tate-chu-yoko, rtl pages), a Chinese chapter (vertical-lr), a
# horizontal chapter with ruby; and the same at a hundred thousand characters for the budget
VT="$ROOT/vertical-epub"
mkdir -p "$VT/lib" "$VT/long"
python3 "$HERE/make-vertical-epubs.py" synthetic "$VT/lib/vertical.epub"
python3 "$HERE/make-vertical-epubs.py" synthetic-long "$VT/long/vertical-long.epub"

# a fixed-layout (pre-paginated) EPUB: two pages that are pictures
FX="$ROOT/fixed-epub"
mkdir -p "$FX/src/META-INF" "$FX/src/OEBPS" "$FX/lib"
printf 'application/epub+zip' >"$FX/src/mimetype"
cp "$EP/src/META-INF/container.xml" "$FX/src/META-INF/"
cat >"$FX/src/OEBPS/content.opf" <<'XML'
<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">fixed-test</dc:identifier><dc:title>Picture Book</dc:title><dc:creator>Fixed Author</dc:creator><dc:language>en</dc:language><meta property="rendition:layout">pre-paginated</meta></metadata><manifest><item id="p1" href="p1.xhtml" media-type="application/xhtml+xml"/><item id="p2" href="p2.xhtml" media-type="application/xhtml+xml"/><item id="i1" href="i1.png" media-type="image/png"/><item id="i2" href="i2.png" media-type="image/png"/></manifest><spine><itemref idref="p1"/><itemref idref="p2"/></spine></package>
XML
python3 - "$FX/src/OEBPS" <<'PY'
import struct, sys, zlib
def png(path, w, h, rgb):
    raw = b"".join(b"\x00" + bytes(rgb) * w for _ in range(h))
    def chunk(t, d):
        c = struct.pack(">I", len(d)) + t + d
        return c + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    open(path, "wb").write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))
d = sys.argv[1]
png(d + "/i1.png", 400, 300, (200, 60, 60))
png(d + "/i2.png", 300, 400, (60, 160, 90))
for n in (1, 2):
    open(d + "/p%d.xhtml" % n, "w").write('<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml"><head><title>p</title><meta name="viewport" content="width=400, height=300"/></head><body><img src="i%d.png" alt="page %d"/></body></html>' % (n, n))
PY
(cd "$FX/src" && zip -q -X -0 "$FX/lib/picture.epub" mimetype && zip -q -X -r "$FX/lib/picture.epub" META-INF OEBPS)

# books in other formats, imported through the format readers
mkdir -p "$ROOT/fb2/nested"
cat >"$ROOT/fb2/nested/solaris.fb2" <<'FB2'
<?xml version="1.0" encoding="utf-8"?>
<FictionBook xmlns="http://www.gribuser.ru/xml/fictionbook/2.0"><description><title-info>
<author><first-name>Stan</first-name><last-name>Lem</last-name></author>
<book-title>Solaris Imported</book-title><lang>en</lang></title-info></description>
<body><section><title><p>Arrival</p></title><p>The ocean was alive.</p></section></body></FictionBook>
FB2
printf 'broken' >"$ROOT/fb2/broken.fb2"

# a second book, so multi-selection can be tested
sed 's/UI Test Book/Second Book/; s/Test Author/Other Author/' "$ROOT/src/book.md" >"$ROOT/src/second.md"

# a book in Arabic: right-to-left paragraphs
mkdir -p "$ROOT/rtl"
{
    printf -- '---\ntitle: "Arabic Book"\nauthor: "Test Author"\nlanguage: ar\n---\n# الفصل الأول\n\n'
    for p in $(seq 1 12); do
        printf 'هذه فقرة رقم %s من الكتاب العربي وهي طويلة بما يكفي لتمتد على عدة أسطر في عمود القارئ حتى نتأكد من أن النص يُعرض من اليمين إلى اليسار بشكل صحيح وأن الأسطر تلتصق بالحافة اليمنى من الصفحة.\n\n' "$p"
    done
} >"$ROOT/rtl/arabic.md"

# be32 N: four big-endian bytes (for the StarDict index)
be32() { printf "$(printf '\\x%02x\\x%02x\\x%02x\\x%02x' $(($1 >> 24 & 255)) $(($1 >> 16 & 255)) $(($1 >> 8 & 255)) $(($1 & 255)))"; }
# make_dict DIR: a two-word StarDict dictionary ("chapter", "paragraph")
make_dict() {
    local dir=$1/mini w1=chapter d1="A main division of a book." w2=paragraph d2="A distinct section of writing."
    mkdir -p "$dir"
    printf '%s' "$d1$d2" >"$dir/mini.dict"
    { printf '%s\0' "$w1"; be32 0; be32 ${#d1}; printf '%s\0' "$w2"; be32 ${#d1}; be32 ${#d2}; } >"$dir/mini.idx"
    printf "StarDict's dict ifo file\nversion=2.4.2\nwordcount=2\nidxfilesize=%s\nbookname=Mini Test Dict\nsametypesequence=m\n" \
        "$(stat -c %s "$dir/mini.idx")" >"$dir/mini.ifo"
}

# make_hunspell DIR: a tiny en_US dictionary that knows the words of the generated test book
make_hunspell() {
    local dir=$1 words="Paragraph of chapter Chapter talks about functionality readers and the world around them in plain words quick brown fox Contents"
    mkdir -p "$dir"
    printf 'SET UTF-8\n' >"$dir/en_US.aff"
    { echo "$(wc -w <<<"$words")"; tr ' ' '\n' <<<"$words"; } >"$dir/en_US.dic"
}

# fake_devices MOUNTS: a Kobo with a library database (one book read to 60%), a Boox with two
# books (one finished in KOReader) and an SD card
fake_devices() {
    local m=$1
    mkdir -p "$m/Boox/Books/Second Book.sdr" "$m/SDCARD/Books" "$m/KOBOeReader/Library"
    printf 'PK-fake' >"$m/Boox/Books/Alpha.epub"
    printf 'PK-fake' >"$m/Boox/Books/Second Book.epub"
    printf 'return { ["percent_finished"] = 1.0, ["summary"] = { ["status"] = "complete" } }' >"$m/Boox/Books/Second Book.sdr/metadata.epub.lua"
    printf 'PK-fake' >"$m/SDCARD/Books/On Card.epub"
    printf 'PK-fake' >"$m/KOBOeReader/Library/UI Test Book.epub"
    python3 - "$m/KOBOeReader/.kobo/KoboReader.sqlite" <<'PY'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
c.executescript("""
CREATE TABLE content (ContentID TEXT NOT NULL, ContentType TEXT NOT NULL, Title TEXT, Attribution TEXT, ___PercentRead INTEGER, ReadStatus INTEGER, DateLastRead TEXT, PRIMARY KEY(ContentID));
CREATE TABLE Shelf (CreationDate TEXT, Id TEXT, InternalName TEXT, LastModified TEXT, Name TEXT, Type TEXT, _IsDeleted BOOL, _IsVisible BOOL, _IsSynced BOOL, PRIMARY KEY(Id));
CREATE TABLE ShelfContent (ShelfName TEXT, ContentId TEXT, DateModified TEXT, _IsDeleted BOOL, _IsSynced BOOL, PRIMARY KEY(ShelfName, ContentId));
INSERT INTO content VALUES ('file:///mnt/onboard/Library/UI Test Book.epub','6','UI Test Book','Test Author',60,1,'2026-11-01T10:00:00Z');
INSERT INTO Shelf VALUES ('d','1','Favourites','d','Favourites','UserTag','false','true','false');
INSERT INTO ShelfContent VALUES ('Favourites','file:///mnt/onboard/Library/UI Test Book.epub','d','false','false');
""")
c.commit()
PY
}

# fake_devices_ui MOUNTS: the devices of fake_devices plus a Boox that has a book with a real cover, and
# a copy of the library's "UI Test Book" with a KOReader notes file (70 % read, one highlight with a
# note that is also in the book's text)
fake_devices_ui() {
    local m=$1
    fake_devices "$m"
    python3 - "$m/Boox/Books/Covered Book.epub" <<'PY'
import struct, sys, zlib, zipfile
w, h = 40, 60
raw = b"".join(b"\x00" + b"\xc0\x30\x30" * w for _ in range(h))
def chunk(t, d):
    c = struct.pack(">I", len(d)) + t + d
    return c + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")
z = zipfile.ZipFile(sys.argv[1], "w")
z.writestr("mimetype", "application/epub+zip", zipfile.ZIP_STORED)
z.writestr("META-INF/container.xml", '<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>')
z.writestr("OEBPS/content.opf", '<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">cov</dc:identifier><dc:title>Covered Book</dc:title><dc:creator>Cover Author</dc:creator><dc:language>en</dc:language><meta name="cover" content="cover"/></metadata><manifest><item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/><item id="cover" href="cover.png" media-type="image/png" properties="cover-image"/></manifest><spine><itemref idref="c1"/></spine></package>')
z.writestr("OEBPS/c1.xhtml", '<?xml version="1.0"?><html xmlns="http://www.w3.org/1999/xhtml"><head><title>c</title></head><body><p>Hello.</p></body></html>')
z.writestr("OEBPS/cover.png", png)
z.close()
PY
    mkdir -p "$m/Boox/Books/UI Test Book.sdr"
    printf 'PK-fake' >"$m/Boox/Books/UI Test Book.epub"
    cat >"$m/Boox/Books/UI Test Book.sdr/metadata.epub.lua" <<'LUA'
return {
    ["percent_finished"] = 0.7,
    ["summary"] = { ["status"] = "reading" },
    ["doc_props"] = { ["title"] = "UI Test Book" },
    ["bookmarks"] = {
        [1] = {
            ["chapter"] = "Chapter 1",
            ["datetime"] = "2026-11-01 10:00:00",
            ["note"] = "from the device",
            ["page"] = "/body/DocFragment[1]",
            ["text"] = "Paragraph 3 of chapter 1 talks about functionality",
        },
    },
}
LUA
}

# make_calibre DIR: a Calibre library with one EPUB book ("Calibre Dune") and one PDF-only book
make_calibre() {
    local dir=$1 scratch
    scratch=$(mktemp -d "$ROOT/cal.XXXXXX")
    printf -- '---\ntitle: "Calibre Dune"\nauthor: "Frank Herbert"\n---\n# One\n\nSand.\n' >"$scratch/cd.md"
    "$CLI" --library "$scratch/lib" library add "$scratch/cd.md" >/dev/null 2>&1
    mkdir -p "$dir/Frank Herbert/Calibre Dune (1)"
    "$CLI" --library "$scratch/lib" library export 1 "$dir/Frank Herbert/Calibre Dune (1)/Calibre Dune - Frank Herbert.epub" >/dev/null 2>&1
    python3 - "$dir" <<'PY'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1] + "/metadata.db")
c.executescript("""
CREATE TABLE books(id INTEGER PRIMARY KEY, title TEXT, sort TEXT, timestamp TEXT, last_modified TEXT, pubdate TEXT, series_index REAL, author_sort TEXT, path TEXT, has_cover INTEGER);
CREATE TABLE data(id INTEGER PRIMARY KEY, book INTEGER, format TEXT, uncompressed_size INTEGER, name TEXT);
CREATE TABLE authors(id INTEGER PRIMARY KEY, name TEXT);
CREATE TABLE books_authors_link(id INTEGER PRIMARY KEY, book INTEGER, author INTEGER);
CREATE TABLE tags(id INTEGER PRIMARY KEY, name TEXT);
CREATE TABLE books_tags_link(id INTEGER PRIMARY KEY, book INTEGER, tag INTEGER);
CREATE TABLE series(id INTEGER PRIMARY KEY, name TEXT);
CREATE TABLE books_series_link(id INTEGER PRIMARY KEY, book INTEGER, series INTEGER);
CREATE TABLE publishers(id INTEGER PRIMARY KEY, name TEXT);
CREATE TABLE books_publishers_link(id INTEGER PRIMARY KEY, book INTEGER, publisher INTEGER);
CREATE TABLE comments(id INTEGER PRIMARY KEY, book INTEGER, text TEXT);
CREATE TABLE languages(id INTEGER PRIMARY KEY, lang_code TEXT);
CREATE TABLE books_languages_link(id INTEGER PRIMARY KEY, book INTEGER, lang_code INTEGER, item_order INTEGER);
CREATE TABLE ratings(id INTEGER PRIMARY KEY, rating INTEGER);
CREATE TABLE books_ratings_link(id INTEGER PRIMARY KEY, book INTEGER, rating INTEGER);
CREATE TABLE identifiers(id INTEGER PRIMARY KEY, book INTEGER, type TEXT, val TEXT);
INSERT INTO books VALUES(1,'Calibre Dune','Calibre Dune','2020-01-01 00:00:00+00:00','2020-01-01 00:00:00+00:00','1965-08-01 00:00:00+00:00',1.0,'Herbert, Frank','Frank Herbert/Calibre Dune (1)',0);
INSERT INTO books VALUES(2,'Only A PDF','Only A PDF','2020-01-01 00:00:00+00:00','2020-01-01 00:00:00+00:00','0101-01-01 00:00:00+00:00',1.0,'Nobody','Nobody/Only A PDF (2)',0);
INSERT INTO data VALUES(1,1,'EPUB',100,'Calibre Dune - Frank Herbert');
INSERT INTO data VALUES(2,2,'PDF',100,'Only A PDF');
INSERT INTO authors VALUES(1,'Frank Herbert'),(2,'Nobody');
INSERT INTO books_authors_link VALUES(1,1,1),(2,2,2);
""")
c.commit()
PY
}

# make_opds_mock DIR: the canned pages of an OPDS catalogue (served at https://mock.example/opds/<file>)
make_opds_mock() {
    local dir=$1 scratch
    mkdir -p "$dir"
    scratch=$(mktemp -d "$ROOT/opds.XXXXXX")
    printf -- '---\ntitle: "Mocked Tale"\nauthor: "Mock Writer"\n---\n# One\n\nOnce upon a mock.\n' >"$scratch/t.md"
    "$CLI" --library "$scratch/lib" library add "$scratch/t.md" >/dev/null 2>&1
    "$CLI" --library "$scratch/lib" library export 1 "$dir/mock-tale.epub" >/dev/null 2>&1
    local head='<?xml version="1.0" encoding="utf-8"?><feed xmlns="http://www.w3.org/2005/Atom" xmlns:dc="http://purl.org/dc/terms/">'
    local acq='http://opds-spec.org/acquisition'
    cat >"$dir/root.xml" <<XML
$head<id>root</id><title>Mock Root</title>
<link rel="self" href="root.xml" type="application/atom+xml;profile=opds-catalog;kind=navigation"/>
<link rel="search" href="search.xml?q={searchTerms}" type="application/atom+xml"/>
<entry><id>f</id><title>Fiction</title><content>Made-up stories</content>
<link rel="subsection" href="fiction.xml" type="application/atom+xml;profile=opds-catalog;kind=acquisition"/></entry></feed>
XML
    cat >"$dir/fiction.xml" <<XML
$head<id>fic</id><title>Fiction</title>
<link rel="next" href="page2.xml" type="application/atom+xml;profile=opds-catalog;kind=acquisition"/>
<entry><id>b1</id><title>Mock Tale</title><author><name>Mock Writer</name></author><summary>A tale.</summary>
<link rel="$acq" href="mock-tale.epub" type="application/epub+zip"/></entry></feed>
XML
    cat >"$dir/page2.xml" <<XML
$head<id>fic2</id><title>Fiction 2</title>
<entry><id>b2</id><title>Second Tale</title><author><name>Other Writer</name></author>
<link rel="$acq" href="second-tale.epub" type="application/epub+zip"/></entry></feed>
XML
    cat >"$dir/search.xml" <<XML
$head<id>s</id><title>Results</title>
<entry><id>b3</id><title>Found Tale</title><author><name>Mock Writer</name></author>
<link rel="$acq" href="mock-tale.epub" type="application/epub+zip"/></entry></feed>
XML
}

BW_BASE=${BROADWAY_BASE:-$((1000 + (RANDOM * 7 + $$) % 20000))}
PREAMBLE="wait:1500,library:import=$ROOT/src,wait:3500"
SCENARIOS=()
N=0

# scenario NAME 'dev script' 'checks (shell, sees $OUT and $LIB)'
scenario() {
    local name=$1 script=$2 checks=$3
    N=$((N + 1))
    [ -n "${ONLY:-}" ] && [[ " $ONLY " != *" $name "* ]] && return 0
    while [ "$(jobs -rp | wc -l)" -ge "$JOBS" ]; do wait -n; done # throttle to $JOBS at once
    SCENARIOS+=("$name")
    (
        d=$ROOT/$name
        mkdir -p "$d"
        script=${script//@D@/$d} # the scenario's own folder, known only here
        if [ "$name" = migrate_json ]; then
            # a library of the old kind: a catalogue in bookx-library.json next to the book files
            "$CLI" --library "$d/lib" library add "$ROOT/src/book.md" >/dev/null 2>&1
            "$CLI" --library "$d/lib" library dump >"$d/legacy.json" 2>/dev/null
            rm -f "$d/lib/bookx-library.db" "$d/lib/bookx-library.db-wal" "$d/lib/bookx-library.db-shm"
            cp "$d/legacy.json" "$d/lib/bookx-library.json"
        fi
        if [ "$name" = duplicates ] || [ "$name" = pain_apart ]; then
            # the same book three times in the library, plus a different one
            for i in 1 2 3; do "$CLI" --library "$d/lib" library add --allow-duplicates "$ROOT/src/book.md" >/dev/null 2>&1; done
            "$CLI" --library "$d/lib" library add "$ROOT/src/second.md" >/dev/null 2>&1
        fi
        case "$name" in
        secrets_migrate)
            # a settings file of an older version, with the password in plain text
            mkdir -p "$d/cfg/bookx"
            printf '{"kosync":{"enabled":true,"server":"http://127.0.0.1:1","user":"u","password":"legacy-pw-123"}}' >"$d/cfg/bookx/config.json"
            ;;
        profiles_migrate)
            # a settings file of an older version: one profile pinned to book 1, one not used
            mkdir -p "$d/cfg/bookx"
            printf '{"reader":{"font_size":15.0},"profiles":{"list":[{"name":"Big","font_size":24.0},{"name":"Spare","margin":10.0}],"books":{"1":"Big"}}}' >"$d/cfg/bookx/config.json"
            ;;
        health_calibre | health_calibre_link) make_calibre "$d/calibre" ;;
        health_check)
            # a book whose file is gone, and the book of the preamble stays a duplicate of it
            "$CLI" --library "$d/lib" library add "$ROOT/src/book.md" >/dev/null 2>&1
            rm -f "$d/lib/Test Author/UI Test Book (1)/"*.epub
            ;;
        health_trash)
            "$CLI" --library "$d/lib" library add "$ROOT/fb2/nested/solaris.fb2" >/dev/null 2>&1
            "$CLI" --library "$d/lib" library remove 1 --delete-files >/dev/null 2>&1
            ;;
        health_layout_linked)
            "$CLI" --library "$d/lib" library add "$ROOT/src/second.md" >/dev/null 2>&1
            "$CLI" --library "$d/lib" library layout linked >/dev/null 2>&1
            ;;
        health_inbox)
            mkdir -p "$d/watch"
            "$CLI" --library "$d/lib" library add "$ROOT/src/second.md" >/dev/null 2>&1
            cp "$ROOT/src/second.md" "$d/watch/Second Book.md"
            sed 's/Second Book/Fresh Arrival/; s/Other Author/New Writer/' "$ROOT/src/second.md" >"$d/watch/Fresh Arrival.md"
            "$CLI" --library "$d/lib" library inbox set "$d/watch" >/dev/null 2>&1
            ;;
        health_tidy | meta_tidy_provenance)
            for n in 1 2 3; do
                a="Frank Herbert"
                [ "$n" = 1 ] && a="Herbert, Frank"
                printf -- '---\ntitle: "Dune %s"\nauthor: "%s"\n---\n# One\n\nSand %s.\n' "$n" "$a" "$n" >"$d/dune$n.md"
                "$CLI" --library "$d/lib" library add "$d/dune$n.md" >/dev/null 2>&1
            done
            ;;
        health_readonly)
            mkdir -p "$d/lib"
            sleep 300 &
            echo $! >"$d/holder.pid"
            printf '{"pid":%s,"host":"%s","since":1700000000,"app":"Bookx"}' "$(cat "$d/holder.pid")" "$(cat /proc/sys/kernel/hostname)" >"$d/lib/.bookx-lock"
            ;;
        health_stale_lock)
            mkdir -p "$d/lib"
            printf '{"pid":4194000,"host":"%s","since":1700000000,"app":"Bookx"}' "$(cat /proc/sys/kernel/hostname)" >"$d/lib/.bookx-lock"
            ;;
        health_sync_warning) touch "$d/.stfolder" ;;
        grid_reflow*)
            # enough books for several rows
            for n in 1 2 3 4 5 6 7 8 9 10; do
                "$CLI" --library "$d/lib" library add --allow-duplicates "$ROOT/src/book.md" >/dev/null 2>&1
            done
            ;;
        library_switcher)
            # a second library with one other book
            "$CLI" --library "$d/lib2" library add "$ROOT/src/second.md" >/dev/null 2>&1
            ;;
        shortcuts_loaded)
            # the shortcuts file of an earlier session: Reload on F5, Quit without a key
            mkdir -p "$d/cfg/bookx"
            printf '{"version":1,"bindings":{"reader.reload":["F5"],"app.quit":[]}}' >"$d/cfg/bookx/shortcuts.json"
            ;;
        esac
        # ---- metadata review, series, works, preflight (docs/metadata-notes.md) ----
        case "$name" in
        meta_series*)
            # three books of one series (numbers 1, 3 and 4: #2 is missing), one of them with a second edition
            for n in 1 3 4; do
                sed "s/UI Test Book/Saga $n/" "$ROOT/src/book.md" >"$d/saga$n.md"
                "$CLI" convert "$d/saga$n.md" "$d/saga$n.epub" --author "Sage Writer" --series "The Saga" --series-index "$n" >/dev/null 2>&1
                "$CLI" --library "$d/lib" library add "$d/saga$n.epub" >/dev/null 2>&1
            done
            ;;
        meta_works)
            # the same work twice (two editions from different publishers), plus one other book
            sed 's/UI Test Book/Edition Tale/; s/Test Author/Ed Writer/' "$ROOT/src/book.md" >"$d/ed1.md"
            "$CLI" convert "$d/ed1.md" "$d/ed1.epub" --publisher "Pub A" >/dev/null 2>&1
            "$CLI" convert "$d/ed1.md" "$d/ed2.epub" --publisher "Pub B" >/dev/null 2>&1
            "$CLI" --library "$d/lib" library add --allow-duplicates "$d/ed1.epub" "$d/ed2.epub" >/dev/null 2>&1
            ;;
        meta_fetch_health)
            # more books, so that three failures in a row are seen; Google Books is down
            for n in 1 2 3 4; do
                sed "s/UI Test Book/Second Book/; s/Test Author/Other Author/" "$ROOT/src/book.md" >"$d/h$n.md"
                sed -i "s/Second Book/Second Book $n/" "$d/h$n.md"
                "$CLI" --library "$d/lib" library add --allow-duplicates "$d/h$n.md" >/dev/null 2>&1
            done
            mkdir -p "$d/mock"
            printf 503 >"$d/mock/google.status"
            ;;
        meta_auto)
            mkdir -p "$d/cfg/bookx"
            printf '{"metadata_prefs":{"auto_apply_safe":true}}' >"$d/cfg/bookx/config.json"
            ;;
        meta_attention | meta_weak_filter)
            printf -- '---\ntitle: "scan_0042"\nauthor: "Unknown"\n---\n# a\n\ntext\n' >"$d/scan.md"
            "$CLI" convert "$d/scan.md" "$d/scan.epub" >/dev/null 2>&1
            "$CLI" meta set "$d/scan.epub" --identifier "ISBN:978-0-441-17271-9" >/dev/null 2>&1
            "$CLI" --library "$d/lib" library add "$d/scan.epub" >/dev/null 2>&1
            ;;
        meta_preflight*)
            # an EPUB whose mimetype entry is wrong (a blocking, fixable problem)
            "$CLI" convert "$ROOT/src/book.md" "$d/bad.epub" --title "Broken Mimetype" --author "Bad Writer" >/dev/null 2>&1
            python3 - "$d/bad.epub" <<'PY'
import sys, zipfile, shutil
src = sys.argv[1]
tmp = src + ".new"
with zipfile.ZipFile(src) as zi, zipfile.ZipFile(tmp, "w", zipfile.ZIP_DEFLATED) as zo:
    for it in zi.infolist():
        data = zi.read(it.filename)
        if it.filename == "mimetype":
            data = b"application/zip"
        zo.writestr(it.filename, data)
shutil.move(tmp, src)
PY
            "$CLI" --library "$d/lib" library add "$d/bad.epub" >/dev/null 2>&1
            [ "$name" = meta_preflight_gate ] && { mkdir -p "$d/cfg/bookx"; printf '{"metadata_prefs":{"preflight":true}}' >"$d/cfg/bookx/config.json"; }
            ;;
        esac
        make_dict "$d/dicts"
        mkdir -p "$d/webmock"
        {
            printf '<html><head><title>A Mocked Post | Blog</title><meta property="og:title" content="A Mocked Post"/><meta name="author" content="Web Writer"/></head><body><nav>Home About</nav><article><h1>A Mocked Post</h1>'
            for i in 1 2 3 4; do printf '<p>Paragraph %s of the mocked web article, with plenty of readable words so that the extraction is convinced this is the real content of the page and not some navigation.</p>' "$i"; done
            printf '</article><footer>Copyright</footer></body></html>'
        } >"$d/webmock/page.html"
        printf '%s' '<?xml version="1.0"?><rss version="2.0"><channel><title>Mock Feed</title><link>https://mock.example/</link><item><title>First Item</title><link>https://mock.example/one</link><description>&lt;p&gt;The first item of the mocked feed has a handful of words in it for the reader.&lt;/p&gt;</description></item><item><title>Second Item</title><link>https://mock.example/two</link><description>&lt;p&gt;The second item adds a few more words so the news book has some content.&lt;/p&gt;</description></item></channel></rss>' >"$d/webmock/feed.xml"
        if [ "$name" = news_opml ]; then
            printf '%s' '<?xml version="1.0"?><opml version="2.0"><head><title>t</title></head><body><outline text="Folder"><outline type="rss" text="Mock" xmlUrl="https://mock.example/feed.xml"/><outline type="rss" text="Gone" xmlUrl="https://mock.example/missing"/></outline></body></opml>' >"$d/feeds.opml"
        fi
        mkdir -p "$d/plugins" "$d/mock"
        printf '%s' '{"numFound":1,"docs":[{"key":"/works/OL1W","title":"Second Book","author_name":["Other Author"],"first_publish_year":2001,"publisher":["Mock Press"],"isbn":["9780441172719"],"subject":["Testing"],"language":["eng"]}]}' >"$d/mock/openlibrary.json"
        if [ "$name" = plugin_source ]; then
            "$PLUG" "$d/plugins" "Plugin Press"
            rm -f "$d/mock/openlibrary.json"   # every built-in source answers 404
        fi
        mkdir -p "$d/mounts/KOBOeReader/.kobo" "$d/mounts/Kindle/documents" "$d/mounts/Kindle/system"
        srvpid=
        if [[ "$name" == kosync_client* ]]; then
            # a KOSync server of its own (the CLI serves Bookx's) and a second client, "KOReader on a Kobo"
            PEER="python3 $HERE/kosync-peer.py"
            "$CLI" --library "$d/lib" library add "$ROOT/src/book.md" >/dev/null 2>&1
            BOOKFILE=$(find "$d/lib" -name '*.epub' | head -1)
            DOC=$($PEER partial-md5 "$BOOKFILE")
            port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')
            script=${script//@PORT@/$port}
            script=${script//@DOC@/$DOC}
            "$CLI" --library "$d/srvlib" library add "$ROOT/src/second.md" >/dev/null 2>&1
            "$CLI" --library "$d/srvlib" serve --port "$port" --kosync "$d/ko.json" >"$d/srv.log" 2>&1 &
            srvpid=$!
            $PEER wait "http://127.0.0.1:$port" || echo "kosync server did not start" >&2
            $PEER register "http://127.0.0.1:$port" alice secret >/dev/null
            $PEER put "http://127.0.0.1:$port" alice secret "$DOC" 0.63 "Kobo Libra" KOBO1 >/dev/null
        fi
        case "$name" in
        annotations_import)
            mkdir -p "$d/imp"
            printf '\xef\xbb\xbfUI Test Book (Test Author)\r\n- Your Highlight on page 3 | Location 30-31 | Added on Monday, 3 March 2014 10:42:11\r\n\r\nParagraph 3 of chapter 1 talks about functionality, readers and the world around them in plain words.\r\n==========\r\nUI Test Book (Test Author)\r\n- Your Note on page 3 | Location 31 | Added on Monday, 3 March 2014 10:43:00\r\n\r\nlitany from kindle\r\n==========\r\nUI Test Book (Test Author)\r\n- Your Highlight on Location 40 | Added on Monday, 3 March 2014 10:50:00\r\n\r\nParagraph 4 of chapter 2 talks about functionality,   readers and the world around them in plain words.\r\n==========\r\nUI Test Book (Test Author)\r\n- Your Highlight on Location 50 | Added on Monday, 3 March 2014 10:51:00\r\n\r\nMISSING quote that the book never says anywhere\r\n==========\r\nSome Other Book (Someone)\r\n- Your Highlight on Location 5 | Added on Monday, 3 March 2014 10:52:00\r\n\r\nA quote from a book that is not in the library\r\n==========\r\n' >"$d/imp/My Clippings.txt"
            printf '%s' '{"title":"UI Test Book","author":"Test Author","entries":[{"page":3,"time":1700000000,"text":"Paragraph 3 of chapter 1 talks about functionality, readers and the world around them in plain words.","note":"koreader note"},{"page":9,"time":1700000100,"text":"Paragraph 5 of chapter 3 talks about functionality, readers and the world around them in plain words.","note":"second from koreader"}],"version":"1.0"}' >"$d/imp/koreader.json"
            ;;
        annotations_import_devices)
            mkdir -p "$d/imp/dev/.kobo" "$d/imp/dev/Books/UI Test Book.sdr"
            python3 - "$d/imp/dev/.kobo/KoboReader.sqlite" "$HERE/../crates/bookx-serve/tests/fixtures/kobo-schema-176.sql" <<'PY'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
c.executescript(open(sys.argv[2]).read())
c.execute("INSERT INTO content (ContentID, ContentType, MimeType, Title, Attribution, ___UserID) VALUES ('file:///mnt/onboard/UI Test Book.epub','6','application/epub+zip','UI Test Book','Test Author','')")
c.execute("INSERT INTO Bookmark (BookmarkID, VolumeID, ContentID, StartContainerPath, StartContainerChildIndex, StartOffset, EndContainerPath, EndContainerChildIndex, EndOffset, Text, Annotation, DateCreated, Type) VALUES ('1','file:///mnt/onboard/UI Test Book.epub','c','p',0,0,'p',0,5,'Paragraph 7 of chapter 3 talks about functionality, readers and the world around them in plain words.','kobo note','2026-11-01T08:00:00Z','highlight')")
c.commit()
PY
            cp "$d/imp/dev/.kobo/KoboReader.sqlite" "$d/imp/copy.sqlite"
            printf 'return {\n ["annotations"] = {\n  [1] = { ["text"] = "Paragraph 8 of chapter 2 talks about functionality, readers and the world around them in plain words.", ["datetime"] = "2026-08-17 11:22:15" },\n },\n ["doc_props"] = { ["title"] = "UI Test Book" },\n}\n' >"$d/imp/dev/Books/UI Test Book.sdr/metadata.epub.lua"
            ;;
        annotations_export) mkdir -p "$d/out" "$d/out2" "$d/out3" ;;
        esac
        case "$name" in
            device_none) rm -rf "$d/mounts" && mkdir -p "$d/mounts" ;;
            device_window|device_options) fake_devices "$d/mounts" ;;
            device_ui|device_lists|device_sync_ui|device_auto_sync) fake_devices_ui "$d/mounts" ;;
        esac
        case "$name" in
        plugin_writer | plugin_transform | plugin_device) "$PLUGS" "$d/plugins" "$d/mounts" ;;
        editor_plugin*)
            # the example editor plugin (examples/plugins/trim-whitespace), prebuilt
            mkdir -p "$d/plugins/trim"
            cp "$HERE/../examples/plugins/trim-whitespace/plugin.toml" "$HERE/../examples/plugins/trim-whitespace/plugin.wasm" "$d/plugins/trim/"
            ;;
        esac
        if [[ "$name" == patches_* ]]; then
            mkdir -p "$d/patches" "$ROOT/third"
            # a book to import while the patches are on (the preamble already filled the library)
            sed 's/UI Test Book/Third Book/; s/Test Author/Third Author/' "$ROOT/src/book.md" >"$ROOT/third/third.md"
            printf 'name = "Shout"\ndescription = "functionality becomes ZZQQ"\nhook = "convert-before"\n[[regex]]\nfind = "functionality"\nreplace = "ZZQQ"\n' >"$d/patches/a-shout.toml"
            printf 'name = "Then whisper"\nhook = "convert-after"\n[[regex]]\nfind = "ZZQQ"\nreplace = "zzqq"\n' >"$d/patches/b-whisper.toml"
            printf 'name = "On import"\nhook = "import-after"\n[[regex]]\nfind = "readers"\nreplace = "IMPORTED"\n' >"$d/patches/c-import.toml"
            printf 'name = "On open"\nhook = "open-before"\n[[regex]]\nfind = "Chapter"\nreplace = "Kapitel"\n[[css]]\nselector = "p"\ndeclarations = "color: red"\n' >"$d/patches/d-open.toml"
            printf 'hook = "convert-before"\n[[regex]]\nfind = "("\n' >"$d/patches/e-broken.toml"
        fi
        # ---- several formats, DRM, authors, trackers, audiobooks, import lookup (docs/metadata-notes.md) ----
        case "$name" in
        cat_formats*)
            # the same book twice: the second is filed as an AZW3 (as another tool would have done)
            sed 's/UI Test Book/Edition Tale/; s/Test Author/Ed Writer/' "$ROOT/src/book.md" >"$d/ed1.md"
            "$CLI" convert "$d/ed1.md" "$d/ed1.epub" >/dev/null 2>&1
            "$CLI" --library "$d/lib" library add --allow-duplicates "$d/ed1.epub" "$d/ed1.epub" >/dev/null 2>&1
            ;;
        cat_import_format*)
            # the same book as an FB2 file (the preamble has imported it as EPUB)
            mkdir -p "$d/other"
            cat >"$d/other/UI Test Book.fb2" <<'FB2'
<?xml version="1.0" encoding="utf-8"?>
<FictionBook xmlns="http://www.gribuser.ru/xml/fictionbook/2.0"><description><title-info>
<author><first-name>Test</first-name><last-name>Author</last-name></author>
<book-title>UI Test Book</book-title><lang>en</lang></title-info></description>
<body><section><title><p>One</p></title><p>The same story in another format.</p></section></body></FictionBook>
FB2
            ;;
        cat_drm)
            mkdir -p "$d/drm"
            "$CLI" convert "$ROOT/src/second.md" "$d/drm/locked.epub" --title "Locked Tale" --author "Closed Writer" >/dev/null 2>&1
            python3 - "$d/drm/locked.epub" <<'PY'
import sys, zipfile, shutil
src = sys.argv[1]
tmp = src + ".tmp"
with zipfile.ZipFile(src) as zin, zipfile.ZipFile(tmp, "w", zipfile.ZIP_DEFLATED) as zout:
    for i in zin.infolist():
        zout.writestr(i, zin.read(i.filename))
    zout.writestr("META-INF/encryption.xml", '<encryption><EncryptedData><EncryptionMethod Algorithm="http://www.w3.org/2001/04/xmlenc#aes128-cbc"/></EncryptedData></encryption>')
    zout.writestr("META-INF/rights.xml", '<rights xmlns="http://ns.adobe.com/adept"/>')
shutil.move(tmp, src)
# a Kindle book with the MOBI encryption flag set
b = bytearray(400)
b[60:68] = b"BOOKMOBI"
b[76:78] = (1).to_bytes(2, "big")
b[78:82] = (200).to_bytes(4, "big")
b[212:214] = (2).to_bytes(2, "big")
open(src.replace("locked.epub", "kindle.mobi"), "wb").write(bytes(b))
PY
            ;;
        cat_authors)
            for pair in "One|Mark Twain" "Two|Samuel Clemens" "Three|S. Clemens" "Four|Someone Else"; do
                t=${pair%%|*}; a=${pair#*|}
                printf -- '---\ntitle: "%s"\nauthor: "%s"\n---\n# One\n\nText.\n' "$t" "$a" >"$d/$t.md"
                "$CLI" --library "$d/lib" library add "$d/$t.md" >/dev/null 2>&1
            done
            ;;
        pain_health)
            # a book with a small cover whose file was damaged without its date or size changing
            python3 - "$d" <<'PY'
import struct, sys, zlib
d = sys.argv[1]
w, h = 150, 220
raw = b"".join(b"\x00" + b"\x30\x60\xc0" * w for _ in range(h))
def chunk(t, x):
    c = struct.pack(">I", len(x)) + t + x
    return c + struct.pack(">I", zlib.crc32(t + x) & 0xFFFFFFFF)
open(d + "/small.png", "wb").write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))
PY
            "$CLI" convert "$ROOT/src/second.md" "$d/damaged.epub" --title "Damaged Tale" --author "Worn Writer" --cover "$d/small.png" >/dev/null 2>&1
            "$CLI" --library "$d/lib" library add "$d/damaged.epub" >/dev/null 2>&1
            "$CLI" --library "$d/lib" library fixity >/dev/null 2>&1
            python3 - "$d/lib" <<'PY'
import glob, os, sys
f = [p for p in glob.glob(sys.argv[1] + "/*/*/*.epub") if "Damaged" in p][0]
st = os.stat(f)
b = bytearray(open(f, "rb").read())
b[len(b) // 2] ^= 0xFF
open(f, "wb").write(b)
os.utime(f, ns=(st.st_atime_ns, st.st_mtime_ns))
PY
            ;;
        cat_trackers)
            for pair in "Dune|Frank Herbert" "Emma|Jane Austen"; do
                t=${pair%%|*}; a=${pair#*|}
                printf -- '---\ntitle: "%s"\nauthor: "%s"\n---\n# One\n\nText.\n' "$t" "$a" >"$d/$t.md"
                "$CLI" --library "$d/lib" library add "$d/$t.md" >/dev/null 2>&1
            done
            printf 'Book Id,Title,Author,ISBN13,My Rating,Date Read,Bookshelves,Exclusive Shelf\n1,Dune,Frank Herbert,"=""9780441172719""",5,2021/03/14,"favorites, sci-fi",read\n2,Emma,Jane Austen,"=""""",0,,classics,to-read\n3,Not Mine,Nobody,,4,,,read\n' >"$d/goodreads.csv"
            ;;
        opds_*) make_opds_mock "$d/opdsmock" ;;
        wiki_image*)
            # canned Wikipedia: a search hit, a summary with a lead picture, and the picture (a 48x32 PNG)
            mkdir -p "$d/webmock"
            printf '{"pages":[{"key":"Mock_Article","title":"Mock Article"}]}' >"$d/webmock/wiki-search.json"
            printf '{"title":"Mock Article","extract":"A mock article with a picture.","thumbnail":{"source":"https://upload.wikimedia.org/mock/thumb.png","width":48,"height":32},"content_urls":{"desktop":{"page":"https://en.wikipedia.org/wiki/Mock_Article"}}}' >"$d/webmock/wiki-summary.json"
            python3 - "$d" "$name" <<'PY'
import struct, sys, zlib, os
d, name = sys.argv[1], sys.argv[2]
w, h = 48, 32
raw = b"".join(b"\x00" + b"\x30\x60\xc0" * w for _ in range(h))
def chunk(t, x):
    c = struct.pack(">I", len(x)) + t + x
    return c + struct.pack(">I", zlib.crc32(t + x) & 0xFFFFFFFF)
png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")
if name == "wiki_image":
    open(os.path.join(d, "webmock", "wiki-thumb.png"), "wb").write(png)
if name == "wiki_image_cached":
    # the picture is in the cache already (FNV-1a of the address, as web::image_cache_name), the network is "off"
    h64 = 0xcbf29ce484222325
    for b in b"https://upload.wikimedia.org/mock/thumb.png":
        h64 = ((h64 ^ b) * 0x100000001b3) & 0xFFFFFFFFFFFFFFFF
    c = os.path.join(d, "cache", "bookx", "wiki-images")
    os.makedirs(c, exist_ok=True)
    open(os.path.join(c, "%016x.img" % h64), "wb").write(png)
PY
            ;;
        dict_*)
            # a fake dictionary server (BOOKX_DICT_CATALOG points the app at it): English and German mock dictionaries
            dictauto=on
            dictdelay=60
            dictmode=
            case "$name" in
            dict_default_on | dict_prefs_switch) dictauto= ;;
            dict_auto_off) dictauto=off ;;
            dict_failed) dictmode=down ;;
            dict_prompt_download | dict_no_source) dictauto=off ;;
            esac
            [ "$name" = dict_open_book ] && "$CLI" --library "$d/lib" library add "$ROOT/src/book.md" >/dev/null 2>&1
            python3 "$HERE/fake-dict-server.py" "$d/dict-port" "$d/dictmock" "$dictdelay" en,de "$dictmode" &
            dictsrv=$!
            for _ in $(seq 1 50); do [ -s "$d/dict-port" ] && break; sleep 0.1; done
            dictcat=$d/dictmock/catalog.json
            ;;
        side_*)
            # a big book (about 2000 contents entries) is book 1 of these scenarios
            "$CLI" --library "$d/lib" library add "$ROOT/big/big.epub" >/dev/null 2>&1
            ;;
        cat_audiobooks)
            printf -- '---\ntitle: "Dune"\nauthor: "Frank Herbert"\n---\n# One\n\nText.\n' >"$d/Dune.md"
            "$CLI" --library "$d/lib" library add "$d/Dune.md" >/dev/null 2>&1
            mkdir -p "$d/audio/Other Stuff"
            printf 'audio' >"$d/audio/Frank Herbert - Dune.m4b"
            printf 'audio' >"$d/audio/Other Stuff/Nothing Like A Book.m4b"
            ;;
        tts_installed_pack | tts_no_pack)
            mkdir -p "$d/cfg/bookx"
            # the owner's setting: the full model is wanted, but only the small one is installed
            printf '{"tts":{"kokoro_model":"fp32"}}' >"$d/cfg/bookx/config.json"
            [ "$name" = tts_installed_pack ] && "$FAKEPACK" "$d/tts" onnxruntime kokoro-int8 kokoro-voices
            ;;
        tts_prep_always | tts_prep_offline | tts_migrates_speechd)
            mkdir -p "$d/cfg/bookx"
            [ "$name" = tts_migrates_speechd ] && printf '{"tts":{"engine":"speechd","voice":"en-us"}}' >"$d/cfg/bookx/config.json"
            [ "$name" != tts_migrates_speechd ] && printf '{"tts":{"download_without_asking":true}}' >"$d/cfg/bookx/config.json"
            ;;
        cat_fetch_import | cat_cache)
            mkdir -p "$d/cfg/bookx"
            [ "$name" = cat_fetch_import ] && printf '{"metadata_prefs":{"fetch_on_import":true}}' >"$d/cfg/bookx/config.json"
            ;;
        esac
        dicts=$d/dicts
        ttsdelay=300
        [[ "$name" == tts_chapters || "$name" == tts_sleep ]] && ttsdelay=10
        [[ "$name" == tts_player_seek_chapter ]] && ttsdelay=4000
        ttsplayer=
        ttsdl= ttsespeak=
        case "$name" in
        tts_installed_pack | tts_no_pack) ttsdelay= ; ttsplayer='cat >/dev/null'; export BOOKX_ESPEAK=/bin/true ;;
        tts_prep_offline) ttsdl=offline ;;
        tts_prep_background) ttsdl=6000 ;;
        tts_prep_origin) ttsdl=switch:4000 ;;
        tts_prep_*) ttsdl=2400 ;;
        tts_needs_espeak) ttsespeak=missing ;;
        esac
        if [ "$name" = tts_server ] || [ "$name" = tts_multi_book ]; then
            # the real pipeline (look-ahead, gapless sink) against a fake OpenAI-compatible server; the "player" eats the samples
            ttsdelay=
            ttsplayer='cat >/dev/null'
            python3 "$HERE/tts-fake-server.py" "$d/tts-port" "$d/tts-requests.log" 0.6 &
            ttssrv=$!
            for _ in $(seq 1 50); do [ -s "$d/tts-port" ] && break; sleep 0.1; done
            script=${script//@TTSPORT@/$(cat "$d/tts-port" 2>/dev/null)}
        fi
        # a free port for the scenarios that run the assistant (MCP) server
        if [[ "$script" == *@MCPPORT@* ]]; then
            script=${script//@MCPPORT@/$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1])')}
        fi
        # a fake Readwise (token "rw-secret-token") for the scenarios that send highlights
        rwurl=
        if [[ "$name" == notes_readwise* ]]; then
            python3 "$HERE/fake-readwise.py" "$d/rw-port" "$d/rw-requests.log" rw-secret-token &
            rwsrv=$!
            for _ in $(seq 1 50); do [ -s "$d/rw-port" ] && break; sleep 0.1; done
            rwurl="http://127.0.0.1:$(cat "$d/rw-port" 2>/dev/null)/api/v2/highlights/"
        fi
        [[ "$name" == dictionary_missing* ]] && dicts=$d/no-dictionaries
        [[ "$name" == dict_* ]] && dicts=$d/dicts-mine # empty: the downloads of the dictionary scenarios go here
        # a display nobody uses: other test runs (other users, agents) share the runtime dir, so never delete
        # a socket that is not ours, and start from a per-run random base
        rtd=${XDG_RUNTIME_DIR:-/run/user/$UID}
        disp=$((BW_BASE + N))
        while [ -e "$rtd/broadway$((disp + 1)).socket" ]; do disp=$((disp + 211)); done
        gtk4-broadwayd ":$disp" >/dev/null 2>&1 &
        bw=$!
        for _ in $(seq 1 100); do [ -e "$rtd/broadway$((disp + 1)).socket" ] && break; sleep 0.1; done
        smtp=
        if [ "$name" = email_book ]; then
            python3 "$HERE/fake-smtp.py" "$d/smtp-port" "$d/mail.txt" &
            smtp=$!
            for _ in $(seq 1 50); do [ -s "$d/smtp-port" ] && break; sleep 0.1; done
        fi
        dav=
        if [[ "$name" == backup_webdav* ]]; then
            mkdir -p "$d/dav-root/files/bob" # the user's folder exists on a real server
            python3 "$HERE/fake-webdav.py" "$d/dav-port" "$d/dav-root" "bob:s3cret" &
            dav=$!
            for _ in $(seq 1 50); do [ -s "$d/dav-port" ] && break; sleep 0.1; done
            script=${script//@PORT@/$(cat "$d/dav-port" 2>/dev/null)}
        fi
        hun=
        if [[ "$name" == editor_live_* ]]; then
            hun=$d/hunspell
            make_hunspell "$hun"
        fi
        if [[ "$name" == editor_live_spelldicts ]]; then
            # a British dictionary that knows "colour", and a LibreOffice extension with a German one
            printf 'SET UTF-8\n' >"$hun/en_GB.aff"
            printf '6\nthe\ncolour\nfox\nquick\nbrown\nand\n' >"$hun/en_GB.dic"
            python3 - "$d/pack.oxt" <<'PY'
import sys, zipfile
z = zipfile.ZipFile(sys.argv[1], "w")
z.writestr("META-INF/manifest.xml", "<manifest/>")
z.writestr("dictionaries/de_DE.aff", "SET UTF-8\n")
z.writestr("dictionaries/de_DE.dic", "3\nder\nhund\nkatze\n")
z.writestr("dictionaries/hyph_de_DE.dic", "hyphenation")
z.close()
PY
        fi
        # passwords go to an in-memory keyring in the tests; one scenario pretends there is none
        lang=en bsize=
        [[ "$name" =~ _([0-9]+x[0-9]+)(_de)?$ ]] && bsize=${BASH_REMATCH[1]}
        [[ "$name" == *_de ]] && lang=de
        srccache=0 # the canned answers of the tests are not cached unless a scenario asks
        [ "$name" = cat_cache ] && srccache=1
        sec=memory
        [ "$name" = secrets_no_keyring ] && sec=none
        OUT=$(
            [ -n "$hun" ] && export BOOKX_HUNSPELL=$hun
            [[ "$name" == wiki_image_cached || "$name" == wiki_image_offline ]] && export BOOKX_OFFLINE=1
            [[ "$name" == dict_offline ]] && export BOOKX_OFFLINE=1
            # automatic dictionary downloads are off in every scenario but the ones that test them
            if [ -n "${dictauto-off}" ]; then export BOOKX_DICT_AUTO=${dictauto-off}; fi
            [ -n "${dictcat:-}" ] && export BOOKX_DICT_CATALOG=$dictcat
            BOOKX_HUNSPELL_USER=$d/userdicts BOOKX_SMTP_PORT=$(cat "$d/smtp-port" 2>/dev/null) GDK_BACKEND=broadway BROADWAY_DISPLAY=":$disp" XDG_CONFIG_HOME=$d/cfg XDG_CACHE_HOME=$d/cache \
                BOOKX_SECRETS=$sec BOOKX_READWISE_URL=$rwurl BOOKX_TTS_MOCK=$ttsdelay BOOKX_TTS_MOCK_DOWNLOAD=$ttsdl BOOKX_TTS_MOCK_ESPEAK=$ttsespeak BOOKX_TTS_PLAYER=$ttsplayer BOOKX_TTS_DIR=$d/tts BOOKX_LIBRARY=$d/lib BOOKX_DICTIONARIES=$dicts BOOKX_DEVICE_ROOTS=$d/mounts BOOKX_SOURCES_MOCK=$d/mock BOOKX_SOURCES_CACHE=$srccache BOOKX_WEB_MOCK=$d/webmock BOOKX_OPDS_MOCK=$d/opdsmock BOOKX_LANG=$lang BOOKX_SIZE=$bsize BOOKX_PLUGINS=$d/plugins BOOKX_PATCHES=$d/patches BOOKX_DEV_SCRIPT="$PREAMBLE,${script%quit}wait:1200,quit" \
                timeout 120 dbus-run-session -- "$BIN" 2>&1 | grep -v 'Gdk-WARNING\|^$\|dbus-daemon\|portal\|fusermount\|fuse init\|bus can.t be made\|Non-compatible\|oo7\|xdg-desktop'
        )
        kill "$bw" 2>/dev/null
        [ -n "$smtp" ] && kill "$smtp" 2>/dev/null
        [ -n "$dav" ] && kill "$dav" 2>/dev/null
        [ -n "${ttssrv:-}" ] && kill "$ttssrv" 2>/dev/null
        [ -n "${dictsrv:-}" ] && kill "$dictsrv" 2>/dev/null
        [ -n "${rwsrv:-}" ] && kill "$rwsrv" 2>/dev/null
        rm -f "$rtd/broadway$((disp + 1)).socket"
        LIB=$("$CLI" --library "$d/lib" library dump 2>/dev/null)
        [ "${SHOW:-}" = "$name" ] && sed "s/^/[$name] /" <<<"$OUT" >&2 # SHOW=name prints what the app said
        : >"$RES/$name"
        check() { # description, condition-exit-status
            if [ "$2" = 0 ]; then echo "ok|$name|$1" >>"$RES/$name"; else echo "FAIL|$name|$1" >>"$RES/$name"; echo "$OUT" | sed 's/^/        | /' >>"$RES/$name"; fi
        }
        has() { grep -Eq -- "$1" <<<"$OUT"; }
        eval "$checks"
        [ -n "$srvpid" ] && kill "$srvpid" 2>/dev/null
    ) &
}

scenario import_formats "library:import=$ROOT/fb2,wait:3000,quit" '
    grep -q "Solaris Imported" <<<"$LIB"; check "an FB2 book in a sub-folder is imported through the format readers" $?
    grep -q "Lem" <<<"$LIB"; check "its author is read from the FB2 metadata" $?
    [ "$(grep -c "\"file\":" <<<"$LIB")" -eq 3 ]; check "the broken FB2 file does not add a book (2 test books + the FB2)" $?
    has "import failed: .*broken.fb2"; check "the broken file is reported" $?'

scenario import 'quit' '
    grep -q "\"UI Test Book\"" <<<"$LIB"; check "markdown is imported as an EPUB" $?'

scenario notes 'read:1,wait:2500,reader:select,wait:600,reader:ui-note,wait:400,reader:ui-note-save,wait:800,reader:dump,quit' '
    has "ui-note: page=Some\(\"note\"\)"; check "note page opens" $?
    has "ui-note: autohide=true"; check "note popover is modal so it can take keyboard focus" $?
    grep -q "typed through the UI" <<<"$LIB"; check "note text is persisted" $?
    has "^autohide: false"; check "popover is non-modal again afterwards" $?'

scenario clean_library 'library:wide,wait:300,library:dump,library:select1,wait:600,library:dump,library:selectall,wait:800,library:dump,library:search-toggle,wait:600,library:dump,quit' '
    L=$(grep -E "^(browse_panel|details_panel|search_bar|selection_bar|banner|title):" <<<"$OUT")
    grep -q "^browse_panel: true" <<<"$L"; check "the sidebar is always there on a wide window" $?
    grep -q "^details_panel: false" <<<"$L"; check "no details panel until a book is selected" $?
    grep -q "^title: Home | 2 books" <<<"$L"; check "the content header shows the name of the view and the book count" $?
    [ "$(grep -c "^search_bar: false" <<<"$L")" -ge 3 ]; check "search stays hidden until asked for" $?
    [ "$(grep "^selection_bar:" <<<"$L" | sed -n 2p)" = "selection_bar: false" ]; check "selecting one book does not pop up an action bar" $?
    [ "$(grep "^selection_bar:" <<<"$L" | sed -n 3p)" = "selection_bar: true" ]; check "selecting several books reveals the action bar" $?
    [ "$(grep "^search_bar:" <<<"$L" | tail -1)" = "search_bar: true" ]; check "the search button reveals the search bar" $?'

scenario continue_banner 'read:1,wait:3500,reader:next,wait:800,close:,wait:1500,library:dump,quit' '
    grep -q "^banner: true" <<<"$OUT"; check "\"Continue reading\" banner appears for a book in progress" $?'

scenario reading_stats 'read:1,wait:3500,reader:next,wait:500,reader:next,wait:500,stats:skew:60,reader:next,wait:500,close:,wait:1500,stats:dump,stats:show,wait:800,stats:dump,quit' '
    grep -q "^stats_sessions: 1" <<<"$OUT"; check "a reading session is recorded when the book is closed" $?
    P=$(grep "^stats_total_pages:" <<<"$OUT" | head -1 | cut -d" " -f2)
    [ "${P:-0}" -ge 2 ]; check "page turns are counted as pages read" $?
    T=$(grep "^stats_today_secs:" <<<"$OUT" | head -1 | cut -d" " -f2)
    [ "${T:-0}" -ge 55 ]; check "the reading time of today is at least the skewed minute" $?
    grep -q "^stats_streak: 1" <<<"$OUT"; check "a reading streak of one day" $?
    grep -q "^stats_line: Today = " <<<"$OUT"; check "the statistics window lists today" $?
    grep -q "^stats_line: Reading speed = " <<<"$OUT"; check "the statistics window shows the reading speed" $?'

scenario hyphenation 'read:1,wait:3500,reader:dump,reader:ui-hyphen-off,wait:800,reader:dump,reader:ui-hyphen-on,wait:800,reader:select,wait:300,reader:dump,quit' '
    H=$(grep "^hyphens:" <<<"$OUT")
    ON=$(sed -n 1p <<<"$H"); OFF=$(sed -n 2p <<<"$H"); AGAIN=$(sed -n 3p <<<"$H")
    [ "$(cut -d" " -f2 <<<"$ON")" -gt 0 ]; check "long words get soft hyphens when hyphenation is on" $?
    [ "$(cut -d" " -f2 <<<"$OFF")" -eq 0 ]; check "no soft hyphens when hyphenation is off" $?
    [ "$(cut -d" " -f3 <<<"$ON")" = "$(cut -d" " -f3 <<<"$OFF")" ]; check "the canonical text length is the same with and without hyphenation (highlights stay valid)" $?
    [ "$ON" = "$AGAIN" ]; check "switching hyphenation back on restores the same layout" $?
    ! grep "^selected:" <<<"$OUT" | grep -q "u{ad}"; check "copied text never contains soft hyphens" $?
    grep -q "^selected: Some(\"Chapter 1" <<<"$OUT"; check "selecting text still works on a hyphenated page" $?'

scenario optimal_breaks 'read:1,wait:3500,reader:dump,reader:select,wait:300,reader:dump,reader:ui-optimal-off,wait:800,reader:dump,reader:ui-optimal-on,wait:800,reader:dump,quit' '
    B=$(grep "^breaks:" <<<"$OUT"); H=$(grep "^hyphens:" <<<"$OUT")
    ON=$(sed -n 1p <<<"$B"); OFF=$(sed -n 3p <<<"$B")
    n() { sed -E "s/.* $2=([0-9]+).*/\\1/" <<<"$1"; }
    [ "$(n "$ON" paragraphs)" -gt 0 ]; check "justified paragraphs get Knuth-Plass line breaks" $?
    [ "$(n "$OFF" paragraphs)" -eq 0 ]; check "with the setting off Pango breaks the lines itself (fallback)" $?
    L=$(n "$ON" lines); F=$(n "$ON" full)
    [ "$L" -gt 0 ] && [ $((F * 100 / L)) -ge 90 ]; check "at least 90% of the lines are justified to the full column width ($F of $L)" $?
    [ "$(sed -n 1p <<<"$B")" = "$(sed -n 4p <<<"$B")" ]; check "switching back restores the same breaks" $?
    [ "$(cut -d" " -f3 <<<"$(sed -n 1p <<<"$H")")" = "$(cut -d" " -f3 <<<"$(sed -n 3p <<<"$H")")" ]; check "the canonical text length is the same with and without optimal breaks (highlights stay valid)" $?
    grep "^selected: Some" <<<"$OUT" | head -1 | grep -q "Chapter 1"; check "selecting text on an optimally broken page works" $?
    ! grep "^selected:" <<<"$OUT" | grep -q -P "\\x{2028}|\\x{2010}"; check "copied text never contains the break markers" $?'

scenario reader_budget 'read:1,wait:3500,reader:ui-bench,quit' '
    B=$(grep "^bench:" <<<"$OUT")
    L=$(sed -E "s/.*layout_ms=([0-9]+).*/\\1/" <<<"$B"); T=$(sed -E "s/.*turns_ms=([0-9]+).*/\\1/" <<<"$B")
    [ -n "$L" ] && [ "$L" -le "$((${BOOKX_PERF_SCALE:-1} * 1500))" ]; check "laying out a chapter with optimal breaks stays within budget (${L}ms, debug build)" $?
    [ -n "$T" ] && [ "$T" -le "$((${BOOKX_PERF_SCALE:-1} * 1500))" ]; check "60 page turns stay within budget (${T}ms, debug build)" $?'

scenario drop_cap_and_float "library:import=$ROOT/float-epub/lib,wait:2500,read:3,wait:3500,reader:ui-shot:${BOOKX_SHOT:-/dev/null},reader:dump,reader:ui-selectall,reader:ui-styles-off,wait:1200,reader:dump,reader:ui-selectall,reader:ui-styles-on,wait:1200,reader:dump,quit" '
    W=$(grep "^wrap:" <<<"$OUT"); H=$(grep "^hyphens:" <<<"$OUT"); A=$(grep "^all:" <<<"$OUT")
    sed -n 1p <<<"$W" | grep -q "caps=1 "; check "the ::first-letter float of the book becomes a drop cap" $?
    sed -n 1p <<<"$W" | grep -q "floats=1 "; check "the floated image is laid out beside the text" $?
    T=$(sed -n 1p <<<"$W" | sed -E "s/.*tails=([0-9]+).*/\\1/"); [ "$T" -ge 2 ]; check "paragraphs next to the cap and the picture are laid out in a narrow head and a full-width tail ($T tails)" $?
    sed -n 2p <<<"$W" | grep -q "caps=0 floats=0 tails=0"; check "without the styling of the book there is no cap, float or split" $?
    sed -n 3p <<<"$W" | grep -q "caps=1 floats=1 "; check "switching the styling back on restores them" $?
    [ "$(sed -n 1p <<<"$H" | cut -d" " -f3)" = "$(sed -n 2p <<<"$H" | cut -d" " -f3)" ]; check "the canonical text length does not change with the cap and the split (highlights stay valid)" $?
    [ "$(sed -n 1p <<<"$A")" = "$(sed -n 2p <<<"$A")" ]; check "the text read back is identical with and without wrapping (nothing lost, nothing added)" $?
    sed -n 1p <<<"$A" | grep -q "Once upon a time"; check "the drop cap letter is still part of the text" $?'

scenario style_tweaks 'read:1,wait:3500,reader:dump,reader:ui-tweak:margin=100,reader:ui-tweak:line_height=1.9,reader:ui-tweak:spacing=1.5,wait:800,reader:ui-tweaks-state,reader:ui-tweak:css=p { text-indent: 3em },wait:1500,reader:dump,reader:ui-tweaks-state,reader:use-default,wait:300,reader:ui-tweak:margin=20,wait:800,reader:ui-tweaks-state,reader:reset-look,wait:1500,reader:ui-tweaks-state,reader:ui-tweak:optimal=off,reader:ui-tweak:hyphen=off,wait:800,reader:dump,quit' '
    T=$(grep "^tweaked:" <<<"$OUT"); I=$(grep "^indented:" <<<"$OUT"); S=$(grep "^tweaks:" <<<"$OUT")
    sed -n 1p <<<"$T" | grep -q "margin=100 line_spacing=1.9 paragraph=Some(1.5)"; check "margin, line height and paragraph spacing from the text appearance popover reach the canvas" $?
    sed -n 1p <<<"$I" | grep -q "indented: 0 of"; check "no first-line indent before the user style sheet" $?
    sed -n 2p <<<"$I" | grep -qv "indented: 0 of"; check "the user style sheet indents the paragraphs of a book that has no styling" $?
    sed -n 2p <<<"$T" | grep -q "margin=100"; check "applying the style sheet keeps the other tweaks" $?
    sed -n 3p <<<"$T" | grep -q "margin=20 "; check "a change after Use as default is the book'"'"'s own and overrides the default" $?
    sed -n 3p <<<"$S" | grep -q "general=StyleTweaks {.*margin: Some(100.0)"; check "Use as default for new books stores the tweaks as the general ones" $?
    sed -n 4p <<<"$T" | grep -q "margin=100 "; check "Reset this book to defaults brings the general tweaks back" $?
    sed -n 4p <<<"$S" | grep -q "book=None"; check "and the book has no tweaks of its own any more" $?
    sed -n 1p <<<"$S" | grep -q "general=StyleTweaks {.*margin: None.*book=Some(StyleTweaks {.*margin: Some(100.0)"; check "tweaks made in the text appearance popover are the book'"'"'s own, not the general ones" $?
    grep -q "text-indent: 3em" <<<"$(cat "$d"/cfg/*/config.json 2>/dev/null)"; check "the user style sheet is saved in the config" $?
    B=$(grep "^breaks:" <<<"$OUT" | tail -1); grep -q "paragraphs=0" <<<"$B"; check "optimal breaking can be switched off from the popover" $?
    H=$(grep "^hyphens:" <<<"$OUT" | tail -1); grep -q "^hyphens: 0 " <<<"$H"; check "hyphenation can be switched off from the popover" $?'

scenario rtl_text "library:import=$ROOT/rtl,wait:2500,read:3,wait:3500,reader:ui-shot:${BOOKX_SHOT:-/dev/null},reader:dump,reader:select,wait:300,reader:dump,quit" '
    R=$(grep "^rtl:" <<<"$OUT" | head -1)
    n() { sed -E "s/.* $2=([0-9]+).*/\\1/" <<<"$1"; }
    [ "$(n "$R" paragraphs)" -ge 12 ]; check "Arabic paragraphs are laid out right to left" $?
    [ "$(n "$R" optimal)" -eq 0 ]; check "right-to-left paragraphs keep the breaking of Pango (Knuth-Plass measures left to right)" $?
    [ "$(n "$R" right_aligned)" -ge 10 ]; check "their last lines sit against the right edge" $?
    grep "^selected: Some" <<<"$OUT" | head -1 | grep -q "الفصل"; check "selecting right-to-left text works" $?'

# ---- vertical writing (docs/vertical-writing-plan.md); the book is the third in the library (after the preamble's two)
scenario vertical_layout "library:import=$ROOT/vertical-epub/lib,wait:2500,read:3,wait:3500,reader:ui-vertical,reader:ui-shot:${BOOKX_SHOT:-/dev/null},reader:page-text,reader:ui-vsel:5:60,reader:next,wait:300,reader:ui-vertical,reader:ui-shot:${BOOKX_SHOT2:-/dev/null},reader:dump,quit" '
    V=$(grep "^vertical:" <<<"$OUT" | sed -n 1p); V2=$(grep "^vertical:" <<<"$OUT" | sed -n 2p)
    n() { sed -E "s/.* $2=([^ ]+).*/\1/" <<<"$1"; }
    W=$(sed -E "s/.* size=([0-9]+)x.*/\1/" <<<"$V")
    grep -q "mode=vertical-rl" <<<"$V"; check "a book with writing-mode: vertical-rl (also -epub-writing-mode) is laid out in vertical lines" $?
    [ "$(n "$V" pages)" -ge 4 ]; check "the chapter is cut into pages ($(n "$V" pages))" $?
    [ "$(n "$V" ruby)" -ge 40 ]; check "the ruby of the text is found ($(n "$V" ruby) annotations)" $?
    [ "$(n "$V" combine)" -ge 20 ]; check "tate-chu-yoko digits are found ($(n "$V" combine) runs)" $?
    [ "$(n "$V" kinsoku)" -eq 0 ]; check "no line of the chapter starts with a character kinsoku forbids" $?
    [ "$(n "$V" hit)" = true ]; check "a click in the middle of the first column finds the first character" $?
    P=$(grep "^vprobe: 0 " <<<"$OUT" | sed -n 1p); PX=$(sed -E "s/.*Some\(\(([0-9]+), ([0-9]+), ([0-9]+)\)\).*/\1/" <<<"$P")
    [ "$PX" -gt $((W * 6 / 10)) ]; check "the first character is at the right edge of the page (x=$PX of $W): columns run right to left" $?
    P12=$(grep "^vprobe: 12 " <<<"$OUT" | sed -n 1p); X12=$(sed -E "s/.*Some\(\(([0-9]+), ([0-9]+), ([0-9]+)\)\).*/\1/" <<<"$P12")
    Y0=$(sed -E "s/.*Some\(\(([0-9]+), ([0-9]+), ([0-9]+)\)\).*/\2/" <<<"$P"); Y12=$(sed -E "s/.*Some\(\(([0-9]+), ([0-9]+), ([0-9]+)\)\).*/\2/" <<<"$P12")
    [ "$(sed -E "s/.*Some\(\([0-9]+, [0-9]+, ([0-9]+)\)\).*/\1/" <<<"$P12")" = 12 ]; check "hit testing maps every probed point back to its character" $?
    [ "$Y12" -gt "$Y0" ] || [ "$X12" -lt "$PX" ]; check "later characters are further down or in the next column to the left" $?
    R=$(sed -E "s/.*rect=\(([0-9]+), ([0-9]+), ([0-9]+), ([0-9]+)\).*/\3 \4/" <<<"$V")
    [ "${R#* }" -gt "${R% *}" ]; check "three characters make a rectangle taller than it is wide ($R)" $?
    T=$(grep "^page_text:" <<<"$OUT")
    grep -q "春の朝、小さな図書館の窓から光が差し込んだ。" <<<"$T"; check "the text a screen reader gets is in reading order, without the ruby" $?
    ! grep -q "はる" <<<"$T"; check "the annotations are not part of the page text" $?
    S=$(grep "^vsel:" <<<"$OUT")
    grep -q "sel=Some((5, 60))" <<<"$S"; check "a drag from one character to another selects exactly that range across columns" $?
    grep -q "text=\"スト" <<<"$S"; check "the selected text is the book text in order" $?
    XS=$(grep -o "rects=.*" <<<"$S" | grep -oE "\(([0-9]+), [0-9]+, [0-9]+, [0-9]+\)" | sed -E "s/\(([0-9]+),.*/\1/")
    [ "$(sed -n 1p <<<"$XS")" -gt "$(sed -n 2p <<<"$XS")" ] && [ "$(sed -n 2p <<<"$XS")" -gt "$(sed -n 3p <<<"$XS")" ]; check "the selection rectangles go from column to column leftwards" $?
    grep -q "^labels: 3 of " <<<"$(grep "^labels:" <<<"$OUT" | tail -1)"; check "turning forward shows the next two pages (3 of ...)" $?'

scenario vertical_chapters "library:import=$ROOT/vertical-epub/lib,wait:2500,read:3,wait:3500,reader:goto=1,wait:1500,reader:ui-vertical,reader:ui-shot:${BOOKX_SHOT:-/dev/null},reader:goto=2,wait:1500,reader:ui-vertical,reader:ui-shot:${BOOKX_SHOT2:-/dev/null},reader:goto=0,wait:1000,reader:ui-vertical,quit" '
    V1=$(grep "^vertical:" <<<"$OUT" | sed -n 1p); V2=$(grep "^vertical:" <<<"$OUT" | sed -n 2p); V3=$(grep "^vertical:" <<<"$OUT" | sed -n 3p)
    grep -q "mode=vertical-lr" <<<"$V1"; check "writing-mode: vertical-lr on a chapter lays its columns out left to right" $?
    W=$(sed -E "s/.* size=([0-9]+)x.*/\1/" <<<"$V1")
    PX=$(grep "^vprobe: 0 " <<<"$OUT" | sed -n 1p | sed -E "s/.*Some\(\(([0-9]+), .*/\1/")
    [ "$PX" -lt $((W * 4 / 10)) ]; check "its first character is at the left edge (x=$PX of $W)" $?
    [ "$(sed -E "s/.* hit=([a-z]+).*/\1/" <<<"$V1")" = true ]; check "hit testing works in vertical-lr" $?
    grep -q "mode=horizontal-tb" <<<"$V2"; check "a chapter without a writing mode is horizontal again" $?
    [ "$(sed -E "s/.* ruby=([0-9]+).*/\1/" <<<"$V2")" -ge 5 ]; check "horizontal text has its ruby too" $?
    [ "$(sed -E "s/.* hit=([a-z]+).*/\1/" <<<"$V2")" = true ]; check "hit testing of horizontal text is unchanged" $?
    grep -q "mode=vertical-rl" <<<"$V3"; check "going back to the first chapter makes the lines vertical again" $?'

scenario vertical_read_aloud "library:import=$ROOT/vertical-epub/lib,wait:2500,read:3,wait:3500,reader:tts-play,wait:1500,reader:ui-vertical,reader:tts-dump,reader:tts-stop,quit" '
    V=$(grep "^vertical:" <<<"$OUT" | sed -n 1p)
    grep -q "state=Playing" <<<"$(grep "^tts: " <<<"$OUT" | sed -n 1p)"; check "the book is read aloud" $?
    [ "$(sed -E "s/.* speaking=([0-9]+).*/\1/" <<<"$V")" -ge 1 ]; check "the sentence that is read is marked on the page, in vertical lines ($(sed -E "s/.* speaking=([0-9]+).*/\1/" <<<"$V") rectangles)" $?'

scenario vertical_scroll_and_styles "library:import=$ROOT/vertical-epub/lib,wait:2500,read:3,wait:3500,reader:scroll,wait:1200,reader:ui-vertical,reader:ui-shot:${BOOKX_SHOT:-/dev/null},reader:next,wait:400,reader:ui-vertical,reader:single,wait:1000,reader:ui-vertical,reader:ui-styles-off,wait:1500,reader:ui-vertical,quit" '
    V=$(grep "^vertical:" <<<"$OUT")
    sed -n 1p <<<"$V" | grep -q "pages=0 " && sed -n 1p <<<"$V" | grep -q "mode=vertical-rl"; check "scroll mode keeps vertical lines (the chapter is one wide strip, no pages)" $?
    [ "$(grep "^vprobe: 0 " <<<"$OUT" | sed -n 1p | grep -c None)" = 0 ]; check "the start of the chapter is on screen at first" $?
    grep "^vprobe: 0 " <<<"$OUT" | sed -n 2p | grep -q None; check "a page down scrolls sideways: the start of the chapter has left the screen" $?
    sed -n 3p <<<"$V" | grep -qE "pages=[1-9][0-9]* "; check "the single page layout cuts the vertical chapter into pages again" $?
    sed -n 4p <<<"$V" | grep -q "mode=vertical-rl"; check "turning the publisher styles off keeps a vertical book vertical" $?'

scenario vertical_budget "library:import=$ROOT/vertical-epub/long,wait:2500,read:3,wait:6000,reader:ui-bench,reader:ui-vertical,quit" '
    B=$(grep "^bench:" <<<"$OUT"); V=$(grep "^vertical:" <<<"$OUT")
    L=$(sed -E "s/.*layout_ms=([0-9]+).*/\1/" <<<"$B")
    [ "$(sed -E "s/.* lines=([0-9]+).*/\1/" <<<"$V")" -gt 2000 ]; check "the long vertical chapter has thousands of columns" $?
    [ -n "$L" ] && [ "$L" -le "$((${BOOKX_PERF_SCALE:-1} * 4000))" ]; check "laying out a long vertical chapter with ruby stays within budget (${L}ms, debug build)" $?
    T=$(sed -E "s/.*turns_ms=([0-9]+).*/\1/" <<<"$B")
    [ -n "$T" ] && [ "$T" -le "$((${BOOKX_PERF_SCALE:-1} * 1500))" ]; check "60 page turns stay within budget (${T}ms, debug build)" $?'

# Save as audio: a chapter and a whole book into WAV files (the silent test voice makes a tone)
scenario tts_save_audio 'read:1,wait:3500,reader:tts-save:chapter:@D@/out/chapter.wav,wait:2500,reader:tts-save:book:@D@/out/book,wait:4500,quit' '
    python3 - "$d/out/chapter.wav" <<'"'"'PY'"'"'
import sys, wave
w = wave.open(sys.argv[1])
assert w.getnchannels() == 1 and w.getsampwidth() == 2 and w.getframerate() == 8000, w.getparams()
assert w.getnframes() > 8000, w.getnframes()
PY
    check "the chapter is a valid mono 16-bit WAV file with audio in it" $?
    n=$(ls "$d/out/book" 2>/dev/null | grep -c "\.wav$"); [ "$n" -ge 1 ]; check "the book is written as one WAV file per chapter" $?
    ls "$d/out/book" | grep -q "^01 - "; check "the chapter files are numbered in book order" $?
    has "^tts_save: files=1"; check "the chapter export is reported as done" $?
    ! has "tts_save: error"; check "no export failed" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario tts_read_aloud 'read:1,wait:3500,reader:tts-voices,reader:tts-play,reader:tts-dump,wait:1300,reader:tts-dump,reader:tts-pause,wait:300,reader:tts-dump,wait:1500,reader:tts-dump,reader:tts-play,wait:300,reader:tts-dump,reader:tts-speed:2,reader:tts-voice:mock-a,wait:200,reader:tts-dump,reader:tts-stop,wait:300,reader:tts-dump,quit' '
    T=$(grep "^tts: " <<<"$OUT")
    grep -q "tts_voices: \\[\"\", \"mock-a\"\\]" <<<"$OUT"; check "the voice list offers the default and the voices of the book language" $?
    sed -n 1p <<<"$T" | grep -q "state=Playing range=Some((0, 9)) text=\"Chapter 1\""; check "reading starts at the top of the page and the first sentence is highlighted" $?
    sed -n 1p <<<"$T" | grep -q "icon=media-playback-pause-symbolic"; check "the play button turns into pause" $?
    sed -n 2p <<<"$T" | grep -q "text=\"Paragraph "; check "the highlight moves on to the next sentence" $?
    sed -n 3p <<<"$T" | grep -q "state=Paused"; check "pause pauses" $?
    [ "$(sed -n 3p <<<"$T" | sed -E "s/.*(range=Some\\([0-9, ]+\\)).*/\\1/")" = "$(sed -n 4p <<<"$T" | sed -E "s/.*(range=Some\\([0-9, ]+\\)).*/\\1/")" ]; check "nothing is read while paused" $?
    sed -n 5p <<<"$T" | grep -q "state=Playing"; check "play resumes" $?
    sed -n 6p <<<"$T" | grep -q "speed: 2.0," && sed -n 6p <<<"$T" | grep -q "\"mock/en\": \"mock-a\""; check "speed and voice are stored" $?
    sed -n 6p <<<"$T" | grep -q "speed 2.0" && sed -n 6p <<<"$T" | grep -q "voice mock-a"; check "the engine is told the new speed and voice" $?
    sed -n 1p <<<"$T" | grep -q "spoken=2 "; check "the next sentence is queued while the first one is spoken (look-ahead)" $?
    sed -n 1p <<<"$T" | grep -q "language en"; check "the engine is told the language of the book" $?
    sed -n 1p <<<"$T" | grep -q "engine_text=\"Voice: Silent test voice\""; check "the popover names the engine" $?
    sed -n 7p <<<"$T" | grep -q "state=Stopped range=None"; check "stop removes the highlight" $?
    sed -n 7p <<<"$T" | grep -q "icon=media-playback-start-symbolic"; check "the button is a play button again" $?
    grep -q "\"mock/en\": \"mock-a\"" <<<"$(cat "$d"/cfg/*/config.json 2>/dev/null)"; check "the voice is saved in the config, per engine and language" $?'

scenario tts_chapters 'read:1,wait:3500,reader:tts-play,wait:7000,reader:tts-dump,reader:tts-stop,quit' '
    grep -q "^tts_chapter: [1-9]" <<<"$OUT"; check "reading goes on with the next chapter when one is finished" $?
    grep "^tts: " <<<"$OUT" | grep -q "state=Playing"; check "and keeps playing there" $?'

scenario tts_controls 'read:1,wait:3500,reader:tts-play,wait:500,reader:tts-dump,reader:tts-skip:8,wait:300,reader:tts-dump,reader:tts-skip:-3,wait:300,reader:tts-dump,reader:tts-pitch:1.2,wait:400,reader:tts-dump,reader:tts-stop,quit' '
    C=$(grep "^tts_chapter:" <<<"$OUT")
    at() { sed -n "$1p" <<<"$C" | sed -E "s/.*current=Some\\(\\(([0-9]+),.*/\\1/"; }
    [ "$(at 2)" -gt "$(at 1)" ]; check "skipping forward moves the highlight on by several sentences" $?
    [ "$(at 3)" -lt "$(at 2)" ] && [ "$(at 3)" -gt "$(at 1)" ]; check "skipping back moves it back" $?
    T=$(grep "^tts: " <<<"$OUT")
    sed -n 4p <<<"$T" | grep -q "pitch: 1.2"; check "the pitch is stored" $?
    sed -n 4p <<<"$T" | grep -q "pitch 1.20"; check "the engine is told the pitch" $?
    sed -n 4p <<<"$T" | grep -q "state=Playing"; check "changing the pitch goes on reading" $?
    grep -q "\"pitch\": 1.2" <<<"$(cat "$d"/cfg/*/config.json 2>/dev/null)"; check "the pitch is saved in the config" $?'

scenario tts_sleep 'read:1,wait:3500,reader:tts-sleep:chapter,reader:tts-play,wait:6000,reader:tts-dump,quit' '
    grep "^tts: " <<<"$OUT" | tail -1 | grep -q "state=Stopped"; check "the sleep timer for the end of the chapter stops the reading there" $?
    grep "^tts_chapter:" <<<"$OUT" | tail -1 | grep -q "^tts_chapter: 0 "; check "and does not go on with the next chapter" $?'

# -- the read-aloud player card (enhancement 11) and the first-voice download (enhancement 20) ---------

scenario tts_player_card 'read:1,wait:3500,reader:tts-player,reader:tts-play,wait:120,reader:tts-player,reader:tts-player-click:next,wait:100,reader:tts-player,reader:tts-player-click:collapse,reader:tts-player-click:faster,wait:100,reader:tts-player,reader:tts-player-click:play,wait:200,reader:tts-player,reader:tts-player-click:play,wait:200,reader:tts-player,reader:tts-player-click:next-chapter,wait:900,reader:tts-player,reader:tts-player-click:collapse,wait:100,reader:tts-player,reader:tts-player-click:stop,wait:200,reader:tts-player,quit' '
    P=$(grep "^tts_player:" <<<"$OUT")
    f() { sed -n "$1p" <<<"$P"; }
    f 1 | grep -q "visible=0"; check "the player card is not there while read aloud is off" $?
    f 2 | grep -q "visible=1 collapsed=1 icon=media-playback-pause-symbolic play=1"; check "reading shows the small player (the default) with a pause button" $?
    SZ=$(f 2 | sed -E "s/.*size=([0-9]+)x([0-9]+).*/\1 \2/"); [ "${SZ% *}" -le 360 ] && [ "${SZ#* }" -le 120 ]; check "the small player is at most 360 x 120 px (is $SZ)" $?
    f 2 | grep -q "text=\"[0-9]\+ / [0-9]\+\""; check "it shows the sentence number of the chapter" $?
    f 2 | grep -q "sentence=\"[A-Z][^\"]\+"; check "and the sentence being read" $?
    f 2 | grep -q "title=\"[^\"]\+\""; check "and the title of the book" $?
    f 2 | grep -q "pch=0"; check "there is no previous chapter in the first chapter" $?
    at() { f "$1" | sed -E "s/.*text=\"([0-9]+) \/ .*/\1/"; }
    [ "$(at 3)" -gt "$(at 2)" ]; check "next sentence moves on" $?
    f 4 | grep -q "collapsed=0" && f 4 | grep -q "details=1"; check "the expand button opens the full player (opt-in)" $?
    f 4 | grep -q "speed=1.25×"; check "the speed button steps to 1.25×" $?
    f 5 | grep -q "icon=media-playback-start-symbolic"; check "the play button paused the reading" $?
    f 6 | grep -q "icon=media-playback-pause-symbolic"; check "and plays again" $?
    f 7 | grep -q "visible=1" && f 7 | grep -q "pch=1"; check "next chapter keeps reading with the card shown" $?
    f 8 | grep -q "collapsed=1" && f 8 | grep -q "details=0"; check "the collapse button makes it the small player again" $?
    f 9 | grep -q "visible=0"; check "the X (top right) stops reading and closes the card" $?
    grep -q "\"speed\": 1.25" <<<"$(cat "$d"/cfg/*/config.json 2>/dev/null)"; check "the speed is saved" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario tts_player_drag 'read:1,wait:3500,reader:tts-play,wait:400,reader:tts-player-drag:0.2:0.3,wait:200,reader:tts-player,reader:tts-player-drag:9:-9,wait:200,reader:tts-player,reader:tts-player-drag:0.5:1,wait:200,reader:tts-player,reader:tts-stop,quit' '
    P=$(grep "^tts_player:" <<<"$OUT")
    sed -n 1p <<<"$P" | grep -q "pos=(0.20,0.30)"; check "the card can be placed anywhere in the window (a fraction of its size)" $?
    sed -n 2p <<<"$P" | grep -q "pos=(1.00,0.00)"; check "and stays inside the window however far it is sent" $?
    sed -n 3p <<<"$P" | grep -q "pos=(0.50,1.00)"; check "and goes back to the default place" $?
    sed -n 1p <<<"$P" | grep -Eq "size=([0-9]+)x([0-9]+)"; check "the card has a size" $?'

scenario tts_player_seek_chapter 'read:1,wait:3500,reader:tts-play,wait:400,reader:tts-player-seek:1.0,wait:700,reader:tts-player,reader:tts-player-seek:0.0,wait:700,reader:tts-player,reader:tts-stop,quit' '
    P=$(grep "^tts_player:" <<<"$OUT")
    N=$(sed -n 1p <<<"$P" | sed -E "s/.*text=\"([0-9]+) \/ ([0-9]+)\".*/\2/")
    sed -n 1p <<<"$P" | grep -q "text=\"$N / $N\""; check "seeking to the end jumps to the last sentence of the chapter" $?
    sed -n 2p <<<"$P" | grep -q "text=\"1 / "; check "seeking to the start jumps back to the first" $?'

scenario tts_prep_download 'read:1,wait:3500,reader:tts-play,wait:300,reader:tts-player,reader:tts-download:yes,wait:900,reader:tts-player,wait:2600,reader:tts-player,reader:tts-stop,quit' '
    P=$(grep "^tts_player:" <<<"$OUT")
    sed -n 1p <<<"$P" | grep -q "offered=1" && sed -n 1p <<<"$P" | grep -q "visible=0"; check "pressing Read Aloud without a voice offers the download first (dialog, no card yet)" $?
    sed -n 2p <<<"$P" | grep -q "visible=1" && sed -n 2p <<<"$P" | grep -q "status=\"Preparing voice [0-9]\+%\""; check "the card shows Preparing voice N% while it downloads" $?
    sed -n 2p <<<"$P" | grep -q "play=0"; check "the play button waits for the voice" $?
    sed -n 3p <<<"$P" | grep -q "icon=media-playback-pause-symbolic" && sed -n 3p <<<"$P" | grep -q "text=\"[0-9]\+ / "; check "reading starts by itself when the voice is ready" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario tts_prep_always 'read:1,wait:3500,reader:tts-play,wait:700,reader:tts-player,wait:2800,reader:tts-player,reader:tts-stop,quit' '
    P=$(grep "^tts_player:" <<<"$OUT")
    sed -n 1p <<<"$P" | grep -q "offered=0" && sed -n 1p <<<"$P" | grep -q "Preparing voice"; check "with always-download set the download starts without asking" $?
    sed -n 2p <<<"$P" | grep -q "icon=media-playback-pause-symbolic"; check "and the reading starts when it is done" $?'

scenario tts_prep_background 'read:1,wait:3500,reader:tts-play,reader:tts-download:yes,wait:900,close:,wait:700,jobs:dump,jobs:open,wait:600,jobs:rows,jobs:close,wait:1500,jobs:dump,wait:4500,jobs:dump,quit' '
    J=$(grep "^jobs: running=" <<<"$OUT")
    sed -n 1p <<<"$J" | grep -q "Voice download|Running|0\.[0-9]*|Downloading the default voice"; check "leaving the book keeps the voice download running as a job in the jobs list" $?
    sed -n 1p <<<"$J" | grep -q "cancellable=true"; check "and it can be cancelled from there" $?
    grep "^jobs: rows" <<<"$OUT" | grep -q "Downloading the default voice"; check "the jobs panel shows it" $?
    A=$(sed -n 2p <<<"$J" | sed -E "s/.*Voice download\|[^|]*\|([0-9.]+)\|.*/\1/")
    B=$(sed -n 1p <<<"$J" | sed -E "s/.*Voice download\|[^|]*\|([0-9.]+)\|.*/\1/")
    awk -v a="$A" -v b="$B" "BEGIN{exit !(a+0 > b+0)}"; check "its progress goes on while the library is shown" $?
    sed -n 3p <<<"$J" | grep -q "Voice download|Done"; check "it finishes there" $?
    sed -n 3p <<<"$J" | grep -q "log=[3-9]"; check "with a log" $?
    [ -f "$d/tts/.mock-voice" ]; check "and the voice is installed" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario tts_installed_pack 'read:1,wait:3500,reader:tts-play,wait:1500,reader:tts-player,quit' '
    P=$(grep "^tts_player:" <<<"$OUT")
    sed -n 1p <<<"$P" | grep -q "offered=0"; check "with a voice installed Read Aloud never offers the download (even though the setting names the other model)" $?
    ! sed -n 1p <<<"$P" | grep -q "Preparing voice"; check "and does not download anything" $?
    sed -n 1p <<<"$P" | grep -q "visible=1"; check "it goes straight to reading (the card is there)" $?
    ! has "Download a voice"; check "no dialog" $?'

scenario tts_no_pack 'read:1,wait:3500,reader:tts-play,wait:800,reader:tts-player,quit' '
    P=$(grep "^tts_player:" <<<"$OUT")
    sed -n 1p <<<"$P" | grep -q "offered=1"; check "with nothing installed Read Aloud offers the download" $?'

# The layout manager (src/card_layout.rs): the cards never sit on top of each other or of a popover
G="$HERE/cards_geom.py"
scenario cards_layout_side_by_side 'read:1,wait:3500,reader:tts-play,wait:600,reader:rsvp,wait:300,until:cards-settled,wait:200,reader:cards-rects,quit' '
    "$G" count 1 2 <<<"$OUT"; check "both cards are open" $?
    "$G" disjoint 1 <<<"$OUT"; check "the cards do not overlap" $?
    "$G" centred 1 <<<"$OUT"; check "they stand side by side (a gap of about 12 px), centred as a group, bottoms level" $?
    "$G" inside 1 <<<"$OUT"; check "inside the page area" $?
    "$G" settled 1 <<<"$OUT"; check "the slide has ended and they are where it wanted them" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario cards_layout_yield 'read:1,wait:3500,reader:tts-play,wait:600,reader:rsvp,wait:300,until:cards-settled,wait:200,reader:cards-rects,reader:rsvp-drag:0.5:1,wait:60,reader:cards-rects,wait:300,until:cards-settled,wait:200,reader:cards-rects,reader:tts-player-drag:0.5:1,wait:300,until:cards-settled,wait:200,reader:cards-rects,reader:tts-player-drag:0.5:0.5,reader:rsvp-drag:0.5:0.5,wait:300,until:cards-settled,wait:200,reader:cards-rects,quit' '
    "$G" disjoint 2 <<<"$OUT"; check "a card dragged onto the place of the other: they do not overlap" $?
    "$G" at_wish 3 1 0.5 1 <<<"$OUT"; check "the dragged card is exactly where the user put it" $?
    "$G" moved_from_wish 3 0 0.5 1 <<<"$OUT"; check "and the other one gave way" $?
    "$G" sliding 2 <<<"$OUT"; check "the other one slides away (it was still moving 60 ms later)" $?
    "$G" disjoint 3 <<<"$OUT"; check "and does not overlap when it has stopped" $?
    "$G" at_wish 4 0 0.5 1 <<<"$OUT"; check "dragging the other card over it: the dragged one stays where it is put" $?
    "$G" moved_from_wish 4 1 0.5 1 <<<"$OUT"; check "and the first now yields" $?
    "$G" disjoint 5 <<<"$OUT"; check "both dragged to the same spot: they still do not overlap" $?
    "$G" inside 5 <<<"$OUT"; check "and are inside the page area" $?
    "$G" settled 5 <<<"$OUT"; check "and the slide has ended" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario cards_layout_popover 'read:1,wait:3500,reader:tts-play,wait:600,reader:rsvp,wait:2000,reader:select,wait:800,reader:cards-rects,reader:tts-player-drag:0.0:0.2,reader:rsvp-drag:0.6:0.2,wait:300,until:cards-settled,wait:200,reader:cards-rects,reader:selkey=clear,wait:300,until:cards-settled,wait:200,reader:cards-rects,quit' '
    "$G" pop_shown 1 <<<"$OUT"; check "selecting text opens the action popover" $?
    "$G" no_pop_overlap 1 <<<"$OUT"; check "the cards (side by side, at the bottom) are clear of it" $?
    "$G" no_pop_overlap 2 <<<"$OUT"; check "cards dragged onto the popover: both keep clear of it" $?
    "$G" disjoint 2 <<<"$OUT"; check "and of each other" $?
    "$G" moved_from_wish 2 0 0 0.2 <<<"$OUT"; check "the read aloud card was pushed out of the way of the popover" $?
    "$G" no_pops 3 <<<"$OUT"; check "the popover is closed" $?
    "$G" at_wish 3 0 0 0.2 <<<"$OUT"; check "and the card returns to the place the user gave it" $?
    "$G" at_wish 3 1 0.6 0.2 <<<"$OUT"; check "so does the speed reading card" $?
    "$G" disjoint 3 <<<"$OUT"; check "still not overlapping each other" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

# leaving the book and coming back: the stored places are the user's wishes, the final ones are worked out again
scenario cards_layout_restore 'read:1,wait:3500,reader:tts-play,wait:600,reader:tts-player-drag:0.5:0.5,reader:tts-pause,reader:rsvp,wait:1500,reader:rsvp-drag:0.5:0.5,wait:300,until:cards-settled,wait:200,reader:cards-rects,reader:tts-player,reader:rsvp-dump,wait:400,close:,until:!reader,read:1,until:tts-card-paused,until:page-ready,wait:300,until:cards-settled,wait:200,reader:cards-rects,reader:tts-player,reader:rsvp-dump,quit' '
    "$G" disjoint 1 <<<"$OUT"; check "(both wishes are the same place, the cards do not overlap)" $?
    "$G" disjoint 2 <<<"$OUT"; check "back in the book the cards do not overlap" $?
    "$G" count 2 2 <<<"$OUT"; check "both cards come back" $?
    "$G" at_wish 2 1 0.5 0.5 <<<"$OUT"; check "the card that was moved last is where the user put it" $?
    "$G" inside 2 <<<"$OUT"; check "inside the page area" $?
    "$G" settled 2 <<<"$OUT"; check "settled" $?
    grep "^tts_player:" <<<"$OUT" | sed -n 2p | grep -q "pos=(0.50,0.50)"; check "the stored place of the read aloud card is still the user wish, not the place it was pushed to" $?
    grep "^rsvp:" <<<"$OUT" | tail -1 | grep -q "pos=(0.50,0.50)"; check "and so is the speed reading card" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

# e-ink: no slides, the cards are at their final places at once
scenario cards_layout_eink 'read:1,wait:3500,eink:on,wait:1500,reader:tts-play,wait:600,reader:rsvp,wait:300,until:cards-settled,wait:200,reader:cards-rects,reader:rsvp-drag:0.5:1,reader:cards-rects,quit' '
    "$G" disjoint 1 <<<"$OUT"; check "e-ink: the cards do not overlap" $?
    "$G" disjoint 2 <<<"$OUT"; check "e-ink: a card dragged onto the other: still not" $?
    "$G" settled 2 <<<"$OUT"; check "e-ink: the other one is at its place at once (no slide)" $?
    ! "$G" sliding 2 <<<"$OUT"; check "e-ink: nothing is sliding" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario cards_restore 'read:1,wait:3500,reader:tts-play,wait:300,reader:tts-player-click:collapse,reader:tts-player-drag:0.2:0.3,wait:1500,reader:tts-player-click:next,reader:tts-pause,reader:rsvp,wait:500,reader:rsvp-drag:0.7:0.15,reader:rsvp-step=40,reader:rsvp-faster,wait:300,reader:tts-player,reader:rsvp-dump,close:,wait:1500,read:1,wait:4500,reader:tts-player,reader:rsvp-dump,quit' '
    P=$(grep "^tts_player:" <<<"$OUT"); R=$(grep "^rsvp:" <<<"$OUT")
    f() { sed -n "$2p" <<<"$1" | grep -oE "$3" | head -1; }
    [ -n "$(f "$P" 1 "pos=\\([0-9.]+,[0-9.]+\\)")" ]; check "(the first dump is there)" $?
    [ "$(f "$P" 1 "pos=\\([0-9.]+,[0-9.]+\\)")" = "$(f "$P" 2 "pos=\\([0-9.]+,[0-9.]+\\)")" ]; check "the read aloud card comes back at the same place" $?
    sed -n 2p <<<"$P" | grep -q "pos=(0.20,0.30)"; check "(the place the card was dragged to)" $?
    [ "$(f "$P" 1 "collapsed=[01]")" = "$(f "$P" 2 "collapsed=[01]")" ] && sed -n 2p <<<"$P" | grep -q "collapsed=0"; check "and in the same (expanded) layout" $?
    [ "$(f "$P" 1 "text=\"[0-9]+ / [0-9]+\"")" = "$(f "$P" 2 "text=\"[0-9]+ / [0-9]+\"")" ]; check "with the same sentence number of the chapter" $?
    sed -n 2p <<<"$P" | grep -q "icon=media-playback-start-symbolic"; check "waiting to be played, not playing" $?
    [ "$(f "$P" 1 "sentence=\"[^\"]*\"")" = "$(f "$P" 2 "sentence=\"[^\"]*\"")" ]; check "showing the same sentence" $?
    sed -n 5p <<<"$R" | grep -q "^rsvp: parked"; check "the speed reading card comes back waiting" $?
    [ "$(f "$R" 4 "pos=\\([0-9.]+,[0-9.]+\\)")" = "$(f "$R" 5 "pos=\\([0-9.]+,[0-9.]+\\)")" ]; check "at the same place" $?
    [ "$(f "$R" 4 "index=[0-9]+")" = "$(f "$R" 5 "index=[0-9]+")" ]; check "at the same word" $?
    [ "$(f "$R" 4 "wpm=[0-9]+")" = "$(f "$R" 5 "wpm=[0-9]+")" ]; check "with the same speed" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario cards_restore_play 'read:1,wait:3500,reader:tts-play,wait:1500,reader:tts-pause,wait:150,reader:tts-player,wait:150,close:,wait:1500,read:1,wait:4500,reader:tts-player,reader:tts-play,wait:700,reader:tts-dump,reader:tts-player,quit' '
    P=$(grep "^tts_player:" <<<"$OUT")
    n() { sed -n "$1p" <<<"$P" | sed -E "s/.*text=\"([0-9]+) \/ .*/\1/"; }
    [ "$(n 2)" = "$(n 1)" ]; check "the sentence number is restored" $?
    [ "$(n 3)" -ge "$(n 1)" ] && [ "$(n 3)" -le $(( $(n 1) + 4 )) ]; check "play continues from that sentence (not from the start of the chapter)" $?
    sed -n 3p <<<"$P" | grep -q "icon=media-playback-pause-symbolic"; check "and the card now plays" $?'

scenario cards_closed_stay_closed 'read:1,wait:3500,reader:tts-play,wait:600,reader:rsvp,wait:400,reader:rsvp-close,reader:tts-player,wait:200,close:,wait:1500,read:1,wait:4500,reader:tts-player,reader:rsvp-dump,reader:tts-player-click:stop,wait:300,close:,wait:1500,read:1,wait:4500,reader:tts-player,quit' '
    P=$(grep "^tts_player:" <<<"$OUT"); R=$(grep "^rsvp:" <<<"$OUT")
    sed -n 2p <<<"$P" | grep -q "visible=1"; check "a card that was open comes back" $?
    sed -n 1p <<<"$R" | grep -q "closed"; check "a card that was closed with X does not come back" $?
    sed -n 3p <<<"$P" | grep -q "visible=0"; check "and neither does one closed with X later" $?'

# "Reader focus" (the bars away while reading): while read aloud or speed reading plays the idle delay is 1.5 s
# instead of 4 s; the clock is moved by a dev hook
scenario reader_focus_playing 'read:1,wait:2500,reader:chrome,reader:tts-play,wait:300,reader:chrome,reader:chrome-clock:900,reader:chrome,reader:chrome-clock:500,reader:chrome,reader:tts-player,reader:chrome-pointer:10,reader:chrome,reader:chrome-pointer:-1,reader:chrome-clock:1600,reader:chrome,reader:tts-pause,wait:200,reader:chrome-touch,reader:chrome-clock:3600,reader:chrome,reader:chrome-clock:600,reader:chrome,quit' '
    C=$(grep "^chrome:" <<<"$OUT")
    sed -n 1p <<<"$C" | grep -q "idle=false busy=false.*playing=false"; check "before reading the bars are there" $?
    sed -n 2p <<<"$C" | grep -q "idle=false.*playing=true"; check "play starts the short countdown (the bars are still there)" $?
    sed -n 3p <<<"$C" | grep -q "idle=false"; check "under 1.5 s after the last interaction they are still there" $?
    sed -n 4p <<<"$C" | grep -q "idle=true"; check "1.6 s after it only the page and the card are left (the bars faded)" $?
    grep "^tts_player:" <<<"$OUT" | sed -n 1p | grep -q "visible=1"; check "and the card stays" $?
    sed -n 5p <<<"$C" | grep -q "idle=false busy=true"; check "the pointer at the top edge brings the bars back (and keeps them)" $?
    sed -n 6p <<<"$C" | grep -q "idle=true"; check "letting go and waiting 1.6 s hides them again" $?
    sed -n 7p <<<"$C" | grep -q "playing=false" && sed -n 7p <<<"$C" | grep -q "idle=false"; check "paused: 3.6 s after the last interaction the bars are still there (the old 4 s delay is back)" $?
    sed -n 8p <<<"$C" | grep -q "idle=true"; check "and they fade after 4 s, as before" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario reader_focus_speed_reading 'read:1,wait:2500,reader:rsvp,wait:400,reader:rsvp-play,wait:300,reader:chrome,reader:chrome-clock:1600,reader:chrome,reader:rsvp-pause,wait:100,reader:chrome-touch,reader:chrome-clock:2000,reader:chrome,quit' '
    C=$(grep "^chrome:" <<<"$OUT")
    sed -n 1p <<<"$C" | grep -q "playing=true"; check "speed reading counts as playing" $?
    sed -n 2p <<<"$C" | grep -q "idle=true"; check "the bars fade after 1.5 s while it runs" $?
    sed -n 3p <<<"$C" | grep -q "idle=false.*playing=false"; check "paused: the normal delay applies again" $?'

# the first origin of the voice download is slow: the job switches to the next one and says so in its log
scenario tts_prep_origin 'read:1,wait:3500,reader:tts-play,reader:tts-download:yes,wait:600,close:,wait:600,jobs:dump,wait:4500,jobs:dump,quit' '
    J=$(grep "^jobs: running=" <<<"$OUT")
    sed -n 1p <<<"$J" | grep -q "Voice download|Running"; check "the voice download runs as a job" $?
    sed -n 2p <<<"$J" | grep -q "Voice download|Done" && sed -n 2p <<<"$J" | grep -q "log=[4-9]"; check "it finishes, and its log has the switch to the next origin (start, file, switch, done)" $?
    [ -f "$d/tts/.mock-voice" ]; check "the voice is installed" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario tts_prep_cancel 'read:1,wait:3500,reader:tts-play,reader:tts-download:yes,wait:700,reader:tts-player,reader:tts-player-click:cancel,wait:300,reader:tts-player,wait:3000,reader:tts-player,quit' '
    P=$(grep "^tts_player:" <<<"$OUT")
    sed -n 1p <<<"$P" | grep -q "Preparing voice"; check "the download is running" $?
    sed -n 2p <<<"$P" | grep -q "visible=0"; check "Cancel stops it and hides the card" $?
    sed -n 3p <<<"$P" | grep -q "visible=0"; check "and nothing starts reading afterwards" $?'

scenario tts_prep_offline 'read:1,wait:3500,reader:tts-play,wait:1200,reader:tts-player,quit' '
    P=$(grep "^tts_player:" <<<"$OUT")
    sed -n 1p <<<"$P" | grep -q "visible=1" && sed -n 1p <<<"$P" | grep -q "offline"; check "no network: the card says that you are offline" $?
    sed -n 1p <<<"$P" | grep -q "voice server"; check "and points to the external voice server" $?
    sed -n 1p <<<"$P" | grep -q "play=1"; check "and Try again is available" $?'

scenario tts_needs_espeak 'read:1,wait:3500,reader:tts-play,wait:500,reader:tts-player,quit' '
    P=$(grep "^tts_player:" <<<"$OUT")
    sed -n 1p <<<"$P" | grep -q "visible=1" && sed -n 1p <<<"$P" | grep -q "espeak-ng"; check "without espeak-ng the card explains what to install" $?'

scenario tts_migrates_speechd 'prefs:,wait:900,prefs:pick=Text to Speech,wait:300,tts:dump,quit' '
    S=$(grep "^tts_settings:" <<<"$OUT")
    grep -q "engine=kokoro" <<<"$S" || grep -q "kokoro" <<<"$S"; check "the removed basic-voice engine shows as Kokoro" $?
    ! grep -q "speechd\|speech-dispatcher" <<<"$S"; check "and no speech-dispatcher choice is left" $?
    ! grep -q "\"engine\": \"speechd\"" <<<"$(cat "$d"/cfg/*/config.json 2>/dev/null)" && grep -q "\"engine\": \"kokoro\"" <<<"$(cat "$d"/cfg/*/config.json 2>/dev/null)"; check "the config was migrated on disk" $?'


scenario tts_settings_page 'prefs:,wait:900,prefs:pick=Text to Speech,wait:300,prefs:search=Kokoro,wait:300,prefs:dump,prefs:search=,tts:engine=kokoro,tts:server=http://127.0.0.1:9,tts:dump,tts:test,tts:dump,wait:1000,tts:dump,quit' '
    P=$(grep "^prefs:" <<<"$OUT" | tail -1)
    grep -q "sections=[^ ]*Text to Speech" <<<"$P"; check "Preferences has a Text to Speech section" $?
    grep -q "selected=Text to Speech" <<<"$P"; check "it can be selected" $?
    [ "$(sed -E "s/.* hits=([0-9]+).*/\\1/" <<<"$P")" -ge 1 ]; check "the settings search finds the Kokoro rows" $?
    S=$(grep "^tts_settings:" <<<"$OUT")
    sed -n 1p <<<"$S" | grep -q "groups=1 "; check "the page is built once, every group appears once" $?
    sed -n 1p <<<"$S" | grep -q "engine=kokoro"; check "choosing an engine is stored" $?
    sed -n 1p <<<"$S" | grep -q "No voice yet"; check "an engine without a downloaded voice says that it is downloaded on first use" $?
    sed -n 1p <<<"$S" | grep -q "server=http://127.0.0.1:9"; check "the server address is stored" $?
    sed -n 2p <<<"$S" | grep -q "test_label=\"Stop\""; check "the test button turns into a stop button while the voice speaks" $?
    sed -n 3p <<<"$S" | grep -q "Played with the silent test voice"; check "the test reports when it is done" $?
    sed -n 3p <<<"$S" | grep -q "test_label=\"Play\""; check "and the button is a play button again" $?
    grep -q "\"engine\": \"kokoro\"" <<<"$(cat "$d"/cfg/*/config.json 2>/dev/null)"; check "the engine is saved in the config" $?'

scenario tts_server 'prefs:,wait:900,prefs:pick=Text to Speech,tts:engine=server,tts:server=http://127.0.0.1:@TTSPORT@,wait:300,read:1,wait:3500,reader:tts-play,wait:2200,reader:tts-dump,wait:1800,reader:tts-dump,reader:tts-stop,wait:300,reader:tts-dump,quit' '
    L=$(cat "$d/tts-requests.log" 2>/dev/null)
    grep -q "POST /v1/audio/speech .*Chapter 1" <<<"$L"; check "the first sentence is sent to the voice server" $?
    grep -q "\"response_format\":\"wav\"" <<<"$L"; check "the server is asked for WAV" $?
    [ "$(grep -c "POST /v1/audio/speech" <<<"$L")" -ge 3 ]; check "the next sentences are requested ahead of playback" $?
    T=$(grep "^tts: " <<<"$OUT")
    sed -n 1p <<<"$T" | grep -q "state=Playing"; check "the book is read through the server engine" $?
    sed -n 1p <<<"$T" | grep -q "engine_text=\"Voice: Voice server\""; check "the popover names the voice server" $?
    A=$(sed -n 1p <<<"$T" | sed -E "s/.*range=(Some\\([0-9, ]+\\)).*/\\1/"); B=$(sed -n 2p <<<"$T" | sed -E "s/.*range=(Some\\([0-9, ]+\\)).*/\\1/")
    [ "$A" != "$B" ]; check "the highlight follows the audio sentence by sentence" $?
    sed -n 3p <<<"$T" | grep -q "state=Stopped range=None"; check "stop silences and removes the highlight" $?'

# the owner's bug: the second book of a run failed with "only one engine/environment ... per process". The real
# pipeline engine (through the speech service, against the fake voice server) in book 1, back to the library,
# book 2, back, book 1 again, and a rapid switch; the cards come back per book.
scenario tts_multi_book 'prefs:,wait:900,prefs:pick=Text to Speech,tts:engine=server,tts:server=http://127.0.0.1:@TTSPORT@,wait:300,read:1,until:page-ready,wait:300,reader:tts-play,until:tts-playing,wait:1200,reader:tts-dump,reader:tts-player,wait:300,close:,until:!reader,read:2,until:page-ready,wait:300,reader:tts-play,until:tts-playing,wait:1200,reader:tts-dump,reader:tts-player,wait:300,close:,until:!reader,read:1,until:tts-card-paused,until:page-ready,wait:300,reader:tts-player,reader:tts-play,until:tts-playing,wait:1200,reader:tts-dump,reader:tts-player,wait:300,close:,until:!reader,read:2,until:page-ready,wait:300,reader:tts-play,wait:300,close:,until:!reader,read:1,until:tts-card-paused,until:page-ready,wait:300,reader:tts-play,until:tts-playing,wait:1200,reader:tts-dump,quit' '
    T=$(grep "^tts: " <<<"$OUT"); P=$(grep "^tts_player:" <<<"$OUT")
    ! has "only one|Reading aloud failed|could not be loaded|busy with"; check "no engine error in any book (the second book used to fail)" $?
    sed -n 1p <<<"$T" | grep -q "state=Playing"; check "book 1 reads aloud" $?
    sed -n 2p <<<"$T" | grep -q "state=Playing"; check "book 2 reads aloud after book 1 was left" $?
    sed -n 3p <<<"$T" | grep -q "state=Playing"; check "book 1 again, from its restored (paused) card" $?
    sed -n 2p <<<"$P" | grep -q "visible=1"; check "book 2 has its own card" $?
    sed -n 3p <<<"$P" | grep -q "icon=media-playback-start-symbolic" && sed -n 3p <<<"$P" | grep -q "visible=1"; check "book 1 comes back with its card, paused" $?
    N1=$(sed -n 1p <<<"$P" | sed -E "s/.*text=\"([0-9]+) \/ .*/\1/"); N3=$(sed -n 3p <<<"$P" | sed -E "s/.*text=\"([0-9]+) \/ .*/\1/")
    [ "$N3" -ge "$N1" ]; check "at the sentence it was left at (not the start)" $?
    sed -n 4p <<<"$T" | grep -q "state=Playing"; check "rapid switching (read, leave at once, other book, read) still plays" $?
    [ "$(grep -c "POST /v1/audio/speech" "$d/tts-requests.log")" -ge 6 ]; check "every book was voiced by the server" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario fixed_layout "library:import=$ROOT/fixed-epub/lib,wait:2500,read:3,wait:3500,reader:ui-shot:${BOOKX_SHOT:-/dev/null},reader:dump,reader:next,wait:1000,reader:dump,quit" '
    F=$(grep "^fixed:" <<<"$OUT")
    sed -n 1p <<<"$F" | grep -q "on=true pages=1 "; check "a pre-paginated book is shown one whole page at a time" $?
    n() { sed -E "s/.* $2=([^ ]+).*/\\1/" <<<"$1"; }
    W=$(n "$(sed -n 1p <<<"$F")" image | cut -dx -f1); H=$(n "$(sed -n 1p <<<"$F")" image | cut -dx -f2); A=$(n "$(sed -n 1p <<<"$F")" area)
    [ "$H" -le "$A" ] && [ "$W" -gt 400 ]; check "the picture is scaled up to fill the page (${W}x${H} in ${A}px)" $?
    [ "$((W * 3))" -ge "$((H * 4 - 8))" ] && [ "$((W * 3))" -le "$((H * 4 + 8))" ]; check "the proportions of the picture are kept" $?
    sed -n 2p <<<"$F" | grep -q "spine=1"; check "the next page is the next spine item" $?
    H2=$(n "$(sed -n 2p <<<"$F")" image | cut -dx -f2); [ "$H2" -le "$A" ] && [ "$H2" -ge "$((A * 9 / 10))" ]; check "the portrait page fills the height of the screen" $?'

scenario highlight_styles 'read:1,wait:3500,reader:select,wait:500,reader:ui-style-underline,reader:hl,wait:700,reader:dump,reader:ui-style-squiggly,wait:500,reader:dump,reader:ui-style-strike,wait:500,reader:dump,reader:ui-style-bg,wait:500,reader:dump,quit' '
    S=$(grep "^hl_styles:" <<<"$OUT"); B=$(grep "^style_buttons:" <<<"$OUT")
    [ "$(sed -n 1p <<<"$S")" = "hl_styles: [\"underline\"] next=underline" ]; check "a highlight made with the underline style is stored as underline" $?
    [ "$(sed -n 2p <<<"$S")" = "hl_styles: [\"squiggly\"] next=squiggly" ]; check "choosing squiggly restyles the selected highlight" $?
    [ "$(sed -n 3p <<<"$S")" = "hl_styles: [\"strikethrough\"] next=strikethrough" ]; check "choosing strikethrough restyles the selected highlight" $?
    [ "$(sed -n 4p <<<"$S")" = "hl_styles: [\"background\"] next=background" ]; check "choosing background restyles the selected highlight" $?
    [ "$(sed -n 2p <<<"$B")" = "style_buttons: [\"squiggly\"]" ]; check "exactly the chosen style button is shown as active" $?'

scenario dictionary 'read:1,wait:3500,reader:select,wait:1200,reader:ui-dict,wait:900,reader:dump,quit' '
    has "^dict: [a-z]+ \| .*Mini Test Dict"; check "the dictionary page names the dictionary" $?
    has "^dict: [a-z]+ \| .*A main division of a book\."; check "the definition of the selected word is shown" $?
    has "^dict: [a-z]+ \| .*<b>chapter</b>"; check "the matched headword is shown" $?'

scenario dictionary_button 'read:1,wait:3500,reader:ui-dblclick,wait:1200,reader:dump,reader:ui-dict-btn,wait:100,reader:dump,wait:900,reader:dump,wait:1500,reader:dump,quit' '
    grep "^selected:" <<<"$OUT" | head -1 | grep -q "selected: Some(\"Chapter\")"; check "a double click selects the word and the release keeps it selected (regression: bug 1)" $?
    grep "^popover:" <<<"$OUT" | head -1 | grep -q "popover: true"; check "the popover with the dictionary button stays open after the double click (regression: bug 1)" $?
    grep "^dict: dict | .*A main division of a book" <<<"$OUT" | head -1 | grep -q .; check "the real dictionary button opens the dictionary page with the definition (regression: bug 1)" $?
    [ "$(grep "^popover:" <<<"$OUT" | sed -n 2,4p | grep -c true)" -eq 3 ]; check "the popover stays open while the definition is shown, it used to vanish a moment after the answer was set (regression: bug 1)" $?
    grep "^dict: dict" <<<"$OUT" | tail -1 | grep -q "A main division"; check "and it still shows the dictionary page at the end" $?'

scenario dictionary_missing 'read:1,wait:3500,reader:select,wait:1200,reader:ui-dict,wait:900,reader:dump,quit' '
    has "^dict_prompt: state=offer lang=en "; check "without a dictionary for the language of the book the popover offers to download one (item 30)" $?
    ! has "^dict: [a-z]+ \| No dictionaries installed"; check "instead of only saying that none is installed" $?'

scenario device_none 'library:devices-scan,wait:300,library:dump,library:ctxmenu,library:send-to-device,wait:500,devwin:dump,devwin:show,wait:1200,devwin:dump,quit' '
    has "^ctxmenu: .*Send to Device"; check "the book menu always has Send to Device" $?
    has "^ctxmenu_send: enabled=false tooltip=No device connected"; check "without a device it is disabled with the hint No device connected" $?
    has "^send_to_device: disabled"; check "activating it does nothing while no device is connected" $?
    has "^devwin: not open"; check "and no Device window opens" $?
    has "^device_button: visible=false"; check "without a device the header has no Device button" $?
    has "^device_banner: revealed=false"; check "and no banner asks anything" $?
    has "^devwin: devices=0 selected=none books=0"; check "the Device window opens without a device" $?
    has "^devwin_hint: No device is connected\\."; check "the window explains how to connect a device" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario device_button 'library:ctxmenu,library:menus,wait:2500,library:dump,library:device-banner=later,wait:300,library:dump,library:device-button,wait:1500,devwin:dump,quit' '
    L=$(grep -E "^(device_button|device_banner):" <<<"$OUT")
    has "^ctxmenu: Read \\| .*Edit Metadata \\| Edit Book .*\\| Remove$"; check "the book menu lists the book actions" $?
    [ "$(grep "^ctxmenu:" <<<"$OUT" | head -1 | grep -o "Device" | wc -l)" -eq 1 ]; check "the book menu has the one Send to Device entry and no other device entry" $?
    [ "$(grep "^ctxmenu_top:" <<<"$OUT" | head -1 | sed "s/.*: //")" -le 10 ]; check "the book menu has at most 10 entries at its first level (it fits a normal window)" $?
    ! has "^(ctxmenu|mainmenu|welcome): .*…"; check "no menu entry or button ends in an ellipsis" $?
    ! has "^ctxmenu:.*(Send to (Kobo|Kindle|E-reader)|with Notes)"; check "no per-device entry and no separate with-notes entry (bug 3)" $?
    has "^ctxmenu_send: enabled=true tooltip=Open the Device window with these books ready to send"; check "with a device connected Send to Device is enabled" $?
    has "^device_button: visible=true label=.+"; check "a connected device shows the Device button in the header bar, with its name" $?
    grep -q "^device_banner: revealed=true text=.* connected — Sync reading progress and notes?" <<<"$L"; check "the banner asks to sync when a device connects" $?
    grep "^device_banner:" <<<"$L" | tail -1 | grep -q "revealed=false"; check "Not now hides the banner" $?
    has "^devwin: devices=2 "; check "the Device button opens the Device window" $?
    ! has "^mainmenu: .*(News Subscriptions|Find Duplicates|Needs Attention|Editions|Series|Notes & Highlights)"; check "the main menu does not list what moved to the sidebar, the + menu and Library Health" $?
    has "^mainmenu: .*Library Health \\[lib.health\\]"; check "Library Health is in the main menu" $?
    has "^welcome: .*News Subscriptions \\[lib.news\\]"; check "News Subscriptions is on the welcome page" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario device_ui 'library:annotate,library:selectall,wait:600,library:send-to-device,wait:2500,devwin:device=SDCARD,wait:2000,devwin:dump,devwin:view=list,wait:500,devwin:dump,devwin:click=send:0,wait:300,devwin:dump,devwin:send,wait:4000,devwin:dump,devwin:device=KOBO,wait:2000,devwin:dump,devwin:notes=off,wait:500,devwin:dump,quit' '
    has "^devwin_lists: to_send=2 checked=2 "; check "Send to Device opens the Device window with the selected books listed and ticked" $?
    has "^devwin_send_item: UI Test Book \\| EPUB \\(sent as is\\) \\| \\+ 2 notes added as a last section \\|  \\| checked=true"; check "each book shows its format, that it is sent as is, and the notes that are added" $?
    has "^devwin_send_item: Second Book \\| EPUB \\(sent as is\\) \\|  \\|  \\| checked=true"; check "a book without notes has no notes line" $?
    has "^devwin_ui: view=grid covers=[0-9]+ include_notes=true auto_sync=false send_button=Send 2 books send_enabled=true"; check "the Send N books button counts the ticked books; notes are included and automatic sync is off by default" $?
    has "^devwin_ui: view=list "; check "the books can be shown as a list" $?
    has "^devwin_lists: to_send=2 checked=1 "; check "clicking a row unticks it" $?
    has "^devwin_ui: .*send_button=Send 1 book send_enabled=true"; check "and the button follows" $?
    has "^devwin_sent: Sent 1 book to "; check "only the ticked book is sent" $?
    [ -n "$(find "$d/mounts/SDCARD" -name "UI Test Book*.epub" 2>/dev/null)" ] && [ -z "$(find "$d/mounts/SDCARD" -name "Second Book*" 2>/dev/null)" ]; check "the unticked book is not on the device" $?
    S=$(find "$d/mounts/SDCARD" -path "*.sdr/metadata.epub.lua" 2>/dev/null | head -1)
    [ -n "$S" ] && grep -q "devhighlight text" "$S"; check "a book with notes goes with its notes (bug 3)" $?
    has "^devwin_send_item: UI Test Book \\| EPUB → KEPUB \\(converted\\) \\| \\+ 2 notes added as a last section \\| Already on the device.* \\| checked=false"; check "on a Kobo the book is shown as converted to KEPUB, with a warning that it is already there, and left out" $?
    has "^devwin_ui: .*send_button=Send 0 books send_enabled=false"; check "with nothing ticked the button is disabled" $?
    has "^devwin_send_item: UI Test Book \\| EPUB → KEPUB \\(converted\\) \\|  \\|"; check "with the notes option off the notes line is gone" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario device_lists 'devwin:show,wait:1500,devwin:device=Boox,wait:2500,devwin:view=list,wait:2000,devwin:dump,devwin:view=grid,wait:300,devwin:click=device:0,wait:200,devwin:dump,devwin:click=device:2:shift,wait:200,devwin:dump,devwin:selectall,devwin:dump,devwin:none,devwin:dump,quit' '
    has "^devwin_align: on_device rows=4 check_x=\\{[0-9]+\\} cover_x=\\{[0-9]+\\} cover_w=\\{40\\} check_before_cover=true"; check "in the list every checkbox is at the same x, before the cover, and every cover slot is 40 wide whatever the cover (the owner'"'"'s alignment bug)" $?
    X=$(grep "^devwin_align: on_device" <<<"$OUT" | head -1 | sed "s/.*check_x={\\([0-9]*\\)}.*/\\1/")
    [ -n "$X" ] && [ "$X" -lt 24 ]; check "the checkbox is the far-left element of the row" $?
    has "^devwin_lists: to_send=0 checked=0 on_device=4 checked=1"; check "clicking a tile toggles it" $?
    has "^devwin_lists: to_send=0 checked=0 on_device=4 checked=3"; check "Shift-click selects the range" $?
    has "^devwin_lists: to_send=0 checked=0 on_device=4 checked=4"; check "Select All ticks everything" $?
    [ "$(grep -c "^devwin_lists: to_send=0 checked=0 on_device=4 checked=0" <<<"$OUT")" -ge 2 ]; check "None clears it (and nothing is ticked at the start)" $?
    has "^devwin_ui: view=list covers=[1-9]"; check "the covers of the books on the device are shown" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario device_sync_ui 'library:annotate,devwin:show,wait:1500,devwin:device=Boox,wait:2500,devwin:syncask,wait:3000,devwin:syncconfirm,wait:3000,devwin:device=KOBO,wait:1500,devwin:syncask,wait:3000,devwin:syncconfirm,wait:3000,devwin:syncask,wait:3000,quit' '
    has "^sync_summary: 2 books: progress moves forward from the device \\(UI Test Book 0%→70%, Second Book 0%→100%\\)"; check "the summary names the books and the progress that moves, from the device" $?
    has "^sync_summary: 1 highlight goes device→Bookx"; check "the summary counts the highlights that come from the device" $?
    has "^sync_summary: 1 highlight stays in Bookx: the device copy keeps a notes file of its own"; check "a device copy with its own notes file is not written to" $?
    has "^sync_summary: .*Nothing is deleted on either side\\."; check "every summary promises that nothing is deleted" $?
    grep -q "Paragraph 3 of chapter 1 talks about functionality" <<<"$LIB"; check "the device highlight is in the library after the sync" $?
    grep -q "from the device" <<<"$LIB"; check "with its note" $?
    grep -q "devhighlight text" <<<"$LIB"; check "the library highlight that the device lacks is still there" $?
    grep -q "\"progress\": 0.7" <<<"$LIB"; check "the progress moved forward to the device 70%" $?
    has "^sync_summary: 2 highlights go Bookx→device \\(2 with notes\\)"; check "the second sync sends both highlights to the Kobo copy, which had none" $?
    has "^sync_summary: 1 book is further along in Bookx than on the device"; check "a device that is behind is told, and keeps its own progress" $?
    K=$(find "$d/mounts/KOBOeReader" -path "*.sdr/metadata.epub.lua" 2>/dev/null | head -1)
    [ -n "$K" ] && grep -q "devhighlight text" "$K" && grep -q "Paragraph 3 of chapter 1" "$K"; check "the notes were written next to the Kobo copy as a KOReader notes file" $?
    ! grep -q "devhighlight" "$d/mounts/Boox/Books/UI Test Book.sdr/metadata.epub.lua"; check "the Boox notes file that was there is untouched" $?
    [ "$(grep -cE "^sync_summary: .* (go|goes) " <<<"$OUT")" -eq 2 ] && [ "$(grep -c "^sync_done:" <<<"$OUT")" -eq 2 ]; check "after the syncs a new plan has nothing left to do: the highlights moved once each, nothing repeats" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario annotations_import 'library:import-highlights=@D@/imp/My Clippings.txt;@D@/imp/koreader.json,wait:3500,library:import-confirm,wait:1500,library:import-highlights=@D@/imp/My Clippings.txt;@D@/imp/koreader.json,wait:3500,library:import-confirm,wait:800,quit' '
    has "^import_plan: found=5 new=3 notes=0 already=0 unplaced=1 unmatched=1"; check "the Kindle clippings and the KOReader export are read, the repeated quote counted once, one quote is not in the book and one book is not in the library" $?
    has "^import_summary: UI Test Book: 3 highlights added, 1 not found in the book’s text, left out"; check "the review says per book what will be added" $?
    has "^import_summary: Left out, not in your library: Some Other Book \\(1\\)"; check "books that are not in the library are listed, not guessed" $?
    has "^import_summary: .*no quote is added twice"; check "the review promises that nothing is duplicated or deleted" $?
    has "^import_done: highlights=3 notes=0"; check "confirming adds the three highlights" $?
    [ "$(grep -c "\"text\": \"Paragraph" <<<"$LIB")" -eq 3 ]; check "the library has exactly the three imported quotes" $?
    [ "$(grep -c "litany from kindle" <<<"$LIB")" -eq 1 ]; check "the Kindle note joined its highlight" $?
    grep -q "2014-03-03T10:42:11" <<<"$LIB"; check "the highlight keeps the date it was made" $?
    has "^import_plan: found=5 new=0 notes=0 already=3"; check "importing the same files again finds everything already there and adds nothing" $?
    ! has "^import_done: highlights=[1-9]" || [ "$(grep -c "^import_done:" <<<"$OUT")" -eq 1 ]; check "the second import is not applied" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario annotations_import_devices 'library:import-highlights=@D@/imp/copy.sqlite,wait:3000,library:import-confirm,wait:1200,library:import-highlights=@D@/imp/dev,wait:3500,library:import-confirm,wait:1000,quit' '
    has "^import_plan: found=1 new=1 notes=0 already=0 unplaced=0 unmatched=0"; check "a copied KoboReader.sqlite gives the Kobo highlight, matched by title" $?
    has "^import_plan: found=2 new=1 notes=0 already=1 unplaced=0 unmatched=0"; check "a device folder is searched for the Kobo database and the KOReader notes; the Kobo quote is already there" $?
    grep -q "kobo note" <<<"$LIB"; check "the Kobo note is in the library" $?
    [ "$(grep -c "\"text\": \"Paragraph" <<<"$LIB")" -eq 2 ]; check "two quotes in total" $?
    [ ! -e "$d/imp/dev/.kobo/KoboReader.sqlite-wal" ] && [ ! -e "$d/imp/dev/.kobo/KoboReader.sqlite-journal" ]; check "the device database was only read" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario annotations_export 'library:menus,library:annotate,library:export-all=md:@D@/out,library:export-all=readwise:@D@/out2,library:export-all=kindle:@D@/out3,wait:800,quit' '
    has "^mainmenu: .*Import Highlights \\[lib.import-highlights\\].*Export All Annotations \\[lib.export-annotations\\]"; check "Import Highlights and Export All Annotations are in the main menu" $?
    has "^export_all: format=md files=1"; check "one file is written for the one book with annotations" $?
    grep -q "devhighlight text" "$d/out/UI Test Book - annotations.md" && grep -q "devnote remember this" "$d/out/UI Test Book - annotations.md"; check "the Markdown has the highlight and its note" $?
    python3 -c "import json,sys; j=json.load(open(sys.argv[1])); h=j[\"highlights\"]; assert len(h)==1 and h[0][\"note\"]==\"devnote remember this\" and h[0][\"title\"]==\"UI Test Book\"" "$d/out2/UI Test Book - annotations.json"; check "the Readwise file is the highlights API body" $?
    grep -q "Your Highlight" "$d/out3/UI Test Book - annotations.txt" && grep -q "Your Note" "$d/out3/UI Test Book - annotations.txt"; check "the Kindle clippings file has a highlight and a note entry" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

# Notes & Highlights: every highlight of the library in one window (docs/annotation-sync-notes.md)
scenario notes_window 'library:annotate,library:annotate-more,library:notes,wait:800,library:notes:dump,library:notes:search=pdfquote,library:notes:dump,library:notes:search=,library:notes:group=color,library:notes:dump,library:notes:group=book,library:notes:color=green,library:notes:dump,library:notes:color=,library:notes:hasnote=yes,library:notes:dump,library:notes:hasnote=any,library:notes:book=2,library:notes:dump,library:notes:book=0,library:notes:from=2026-10-05,library:notes:dump,library:notes:from=,library:notes:search=nothing-like-this,library:notes:dump,library:notes:search=,quit' '
    has "^notes_state: total=4 shown=4 selected=0 sections=\\[UI Test Book · Test Author=3;Second Book · Other Author=1\\] first=Some\\(\"UI Test Book\\|Chapter 1\\|devnote remember this\"\\)"; check "the window lists every highlight of every book, grouped by book, newest first" $?
    has "^notes_state: total=4 shown=1 selected=0 sections=\\[UI Test Book · Test Author=1\\] first=Some\\(\"UI Test Book\\|page 5\\|pdfnote\"\\)"; check "searching finds the PDF highlight and shows its page" $?
    has "^notes_state: total=4 shown=4 selected=0 sections=\\[Yellow=2;Blue=1;Green=1\\]"; check "grouping by colour makes a section per colour" $?
    has "^notes_state: total=4 shown=1 .*first=Some\\(\"UI Test Book\\|Chapter 2\\|\"\\)"; check "the colour filter keeps the green one (no note, chapter 2)" $?
    has "^notes_state: total=4 shown=3 "; check "the has-a-note filter keeps the three with notes" $?
    has "^notes_state: total=4 shown=1 selected=0 sections=\\[Second Book"; check "the book filter keeps one book" $?
    has "^notes_state: total=4 shown=2 "; check "the from-date filter drops the older ones (2 and 3 Oct)" $?
    has "^notes_state: total=4 shown=0 "; check "a search with no match shows nothing" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario notes_edit_export 'library:annotate,library:annotate-more,library:notes,wait:800,library:notes:search=bluequote,wait:400,library:notes:edit-first=changed blue note,library:notes:dump,library:notes:search=bluequote,wait:400,library:notes:select-first,library:notes:search=,wait:400,library:notes:dump,library:notes:export=json:@D@/one.json,library:notes:clear,library:notes:export=md:@D@/all.md,library:notes:export=anki-apkg-basic:@D@/h.apkg,library:notes:export=anki-tsv-cloze:@D@/h.txt,library:notes:export=kindle:@D@/k.txt,library:notes:export=vocab-tsv-basic:@D@/v.txt,library:notes:search=greenquote,wait:400,library:notes:delete-first,library:notes:search=,wait:400,library:notes:dump,quit' '
    has "^notes_edit: ok=true note=\"changed blue note\""; check "editing a note saves it" $?
    has "^notes_state: total=4 shown=1 selected=0 sections=\\[Second Book · Other Author=1\\] first=Some\\(\"Second Book\\|Chapter 1\\|changed blue note\"\\)"; check "the list shows the new note" $?
    has "^notes_state: total=4 shown=4 selected=1 "; check "selecting one highlight is counted" $?
    python3 -c "import json,sys; j=json.load(open(sys.argv[1])); a=[x for b in j[\"books\"] for x in b[\"annotations\"]]; assert len(a)==1 and a[0][\"text\"]==\"bluequote in the other book\" and a[0][\"id\"].startswith(\"bx-\") and a[0][\"note\"]==\"changed blue note\"" "$d/one.json"; check "exporting with one selected exports only it, with a stable id and the edited note" $?
    [ "$(grep -c "quote" "$d/all.md")" -ge 3 ] && grep -q "devhighlight text" "$d/all.md"; check "with nothing selected everything shown is exported (Markdown, all books)" $?
    python3 - "$d/h.apkg" <<'"'"'PY'"'"'
import sys, zipfile, sqlite3, tempfile, os, json
z = zipfile.ZipFile(sys.argv[1])
assert sorted(z.namelist()) == ["collection.anki2", "media"]
t = tempfile.mkdtemp(); open(os.path.join(t, "c"), "wb").write(z.read("collection.anki2"))
c = sqlite3.connect(os.path.join(t, "c"))
n = c.execute("select count(*) from notes").fetchone()[0]
k = c.execute("select count(*) from cards").fetchone()[0]
assert n == 4 and k == 4, (n, k)
assert c.execute("select count(distinct guid) from notes").fetchone()[0] == 4
assert len(json.loads(c.execute("select models from col").fetchone()[0])) == 2
PY
    check "the Anki package opens as a collection with four notes and cards, one guid each" $?
    grep -q "^#notetype:Cloze" "$d/h.txt" && grep -q "{{c1::" "$d/h.txt"; check "the Anki text file is a cloze import with the header lines" $?
    grep -q "Your Highlight" "$d/k.txt"; check "the Kindle clippings export works from the window too" $?
    has "^notes_export_result: .*There is nothing to turn into cards"; check "an empty vocabulary says so instead of writing an empty file" $?
    has "^notes_delete: ok=true"; check "deleting a highlight removes it" $?
    has "^notes_state: total=3 shown=3 "; check "the list has one fewer" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario notes_open_passage 'library:annotate-more,library:notes,wait:800,library:notes:search=greenquote,wait:400,library:notes:open-first,wait:3500,reader:dump,quit' '
    has "^labels: .*"; check "the book opened in the reader" $?
    has "^goto_highlight: id=m1 spine=1"; check "the reader went to the chapter of the clicked highlight" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

if [ "${NET:-1}" != 0 ]; then
    scenario notes_readwise 'library:annotate,library:annotate-more,library:notes,wait:800,library:notes:token=rw-secret-token,library:notes:readwise,wait:1500,library:notes:readwise,wait:1500,library:notes:close,wait:300,quit' '
        has "^readwise: Sent 4 highlights to Readwise"; check "the first send uploads all four" $?
        has "^readwise: Readwise is up to date \\(4 already sent\\)"; check "the second send uploads nothing: it remembers what it sent" $?
        python3 -c "import json,sys; L=[json.loads(l) for l in open(sys.argv[1])]; assert len(L)==1 and len(L[0][\"highlights\"])==4, L" "$d/rw-requests.log"; check "the fake Readwise got one request with four highlights" $?
        ! grep -rq "rw-secret-token" "$d/cfg/bookx/config.json" 2>/dev/null; check "the token is in the keyring, not in the settings file" $?
        ! has "rw-secret-token"; check "the token is never printed" $?
        ! has "panicked|CRITICAL"; check "no errors" $?'
fi

if [ "${NET:-1}" != 0 ]; then
    # Bookx as a KOSync client against a KOSync server on this machine (Bookx's own, via the CLI), with a
    # second client that plays KOReader on a Kobo
    scenario kosync_client 'kosync:setup=http://127.0.0.1:@PORT@;bob;pw,kosync:register,wait:1500,kosync:setup=http://127.0.0.1:@PORT@;alice;secret,kosync:login,wait:1500,read:1,wait:3000,kosync:continue,wait:2000,reader:next,wait:6500,kosync:dump,quit' '
        has "^kosync_account: Registered and logged in"; check "Register creates the account on the server and logs in" $?
        has "^kosync_account: Logged in"; check "Log in accepts the account that another client made" $?
        has "^kosync_pull: doc=$DOC remote=0.630"; check "opening the book asks the server for the book named by its partial MD5, the id another client computed on its own" $?
        has "^kosync_prompt: Continue from Kobo Libra at 63%\\?"; check "a position further on, from another device, is offered by name and percentage" $?
        has "^kosync_goto: spine="; check "continuing moves the reader to the percentage" $?
        python3 -c "import json,sys; d=json.loads(sys.argv[1]); b=[x for x in d.get(\"books\", d) if x[\"meta\"][\"title\"]==\"UI Test Book\"][0] if isinstance(d, dict) else d[0]; assert 0.58 < b[\"progress\"] < 0.72, b[\"progress\"]" "$LIB" 2>/dev/null || python3 -c "import re,sys; m=re.search(r\"\\\"progress\\\": ([0-9.]+)\", sys.argv[1]); assert 0.58 < float(m.group(1)) < 0.72" "$LIB"; check "the library position follows to about 63%" $?
        R=$($PEER get "http://127.0.0.1:$port" alice secret "$DOC")
        set -- $R none none none
        [ "$2" = "Bookx_test" ] && python3 -c "import sys; assert 0.58 < float(sys.argv[1]) < 0.75" "$1" && [ "${3#/body/DocFragment[}" != "$3" ]; check "after a page turn the new position is on the server under this device name, with a KOReader xpointer" $?
        has "^kosync_push: pct=.* ok"; check "the push is reported" $?
        ! has "panicked|CRITICAL"; check "no errors" $?'

    # offline queue: the push fails (nothing listens), is kept on disk, and goes out when the server is back
    scenario kosync_client_queue 'kosync:setup=http://127.0.0.1:1;alice;secret;filename,wait:300,read:1,wait:3000,reader:next,wait:6500,kosync:queue-dump,kosync:setup=http://127.0.0.1:@PORT@;alice;secret;filename,kosync:queue-flush,wait:2500,kosync:queue-dump,quit' '
        has "^kosync_push: error"; check "with the server down the push fails" $?
        has "^kosync_queue: entries=1"; check "and the position is kept in the queue on disk" $?
        has "^kosync_queue_flush: sent=1 kept=0 dropped=0"; check "when the server is back the queued position is sent" $?
        has "^kosync_queue: entries=0"; check "and the queue is empty afterwards" $?
        FN=$($PEER filename-md5 "$BOOKFILE")
        R=$($PEER get "http://127.0.0.1:$port" alice secret "$FN"); set -- $R none
        [ "$1" != none ] && python3 -c "import sys; assert float(sys.argv[1]) > 0" "$1"; check "the server has the queued position under the book name" $?
        ! has "panicked|CRITICAL"; check "no errors" $?'
    scenario kosync_client_stay 'kosync:setup=http://127.0.0.1:@PORT@;alice;secret,wait:300,read:1,wait:3000,reader:next,wait:6500,kosync:stay,reader:next,wait:6500,quit' '
        has "^kosync_prompt: Continue from Kobo Libra at 63%\\?"; check "the question is asked" $?
        has "^kosync_push: pct=.* held back"; check "until the question is answered a push does not take the server backwards" $?
        R=$($PEER get "http://127.0.0.1:$port" alice secret "$DOC")
        set -- $R none none none
        [ "$2" = "Bookx_test" ] && python3 -c "import sys; assert float(sys.argv[1]) < 0.3" "$1"; check "after Stay Here the reader keeps its own place and the next push goes out" $?
        ! has "kosync_goto"; check "nothing moved the reader" $?
        ! has "panicked|CRITICAL"; check "no errors" $?'
fi

scenario device_auto_sync 'library:annotate,library:dump,library:autosync=on,library:forget-devices,library:devices-scan,wait:5000,library:dump,quit' '
    grep "^details_auto:" <<<"$OUT" | head -1 | grep -q "auto_sync: false"; check "automatic sync is off by default" $?
    grep "^details_auto:" <<<"$OUT" | tail -1 | grep -q "auto_sync: true"; check "it can be switched on" $?
    has "^sync_done: .*synced: progress of 2 books"; check "connecting a device then syncs without asking, and says what happened" $?
    grep -q "\"progress\": 0.7" <<<"$LIB"; check "the library has the device progress" $?
    grep "^device_banner:" <<<"$OUT" | tail -1 | grep -q "revealed=false"; check "no banner is shown when automatic sync is on" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario details_panel 'library:wide,wait:300,library:dump,library:select1,wait:800,library:dump,library:selectall,wait:800,library:dump,library:unselect,wait:800,library:dump,library:select1,wait:800,library:dump,quit' '
    P=$(grep "^details_panel:" <<<"$OUT")
    [ "$(sed -n 1p <<<"$P")" = "details_panel: false" ]; check "the panel starts closed" $?
    [ "$(sed -n 2p <<<"$P")" = "details_panel: true" ]; check "selecting a book opens the details panel by itself" $?
    [ "$(sed -n 3p <<<"$P")" = "details_panel: true" ]; check "selecting several books keeps it open (with the bulk actions)" $?
    [ "$(sed -n 4p <<<"$P")" = "details_panel: false" ]; check "clearing the selection closes it" $?
    [ "$(sed -n 5p <<<"$P")" = "details_panel: true" ]; check "selecting a book opens it again" $?
    [ "$(grep -c "^close_button_in_header: true" <<<"$OUT")" -eq 5 ]; check "the window close button stays in the header bar while the panel is open (bug 5)" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario details_narrow 'library:narrow,wait:400,library:select1,wait:800,library:dump,library:details-open,wait:400,library:dump,library:sidebar,quit' '
    P=$(grep "^details_panel:" <<<"$OUT")
    [ "$(sed -n 1p <<<"$P")" = "details_panel: false" ]; check "in a narrow window selecting a book does not cover the books with the panel" $?
    [ "$(sed -n 2p <<<"$P")" = "details_panel: true" ]; check "the Details button of the selection bar opens it as an overlay" $?
    has "^selection_bar: true"; check "a single selected book shows the bar with the Details button in a narrow window" $?
    has "^sidebar: .*panel=false collapsed=true"; check "the sidebar is an overlay, closed, in a narrow window" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

# the sidebar: Home, Recent, Favourites, Notes & Highlights (enhancement 15)
scenario sidebar_views 'library:sidebar,library:nav=recent,wait:300,library:visible,library:select1,wait:300,library:favourite,wait:300,library:nav=favourites,wait:300,library:visible,library:dump,library:nav=notes,wait:500,library:dump,library:sidebar,library:nav=home,wait:300,library:visible,library:sidebar,quit' '
    has "^sidebar: view=home rows=nav-home,nav-recent,nav-favourites,nav-notes counts=2,2,0,0 panel=true collapsed=false"; check "the sidebar lists Home, Recent, Favourites and Notes & Highlights with counts" $?
    V=$(grep "^visible:" <<<"$OUT" | sed "s/ |.*//")
    [ "$(sed -n 1p <<<"$V")" = "visible: 2" ]; check "Recent lists the recently added books" $?
    [ "$(sed -n 2p <<<"$V")" = "visible: 1" ]; check "Favourites lists only the starred book" $?
    [ "$(sed -n 3p <<<"$V")" = "visible: 2" ]; check "Home lists every book again" $?
    grep -q "\"favourites\": *\[" "$d/cfg/bookx/config.json"; check "the favourites are saved in the settings" $?
    grep -q "\"sidebar_view\": *\"home\"" "$d/cfg/bookx/config.json"; check "the view shown is remembered" $?
    has "^view: notes"; check "Notes & Highlights opens inside the content area" $?
    has "^sidebar: view=home rows=.* counts=2,2,1,0 "; check "the Favourites count follows" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

# bug 8: clicks, Ctrl+click, Shift+click, right-click and the empty space
scenario selection_clicks 'library:selectall,wait:200,library:selection,library:click-empty,wait:200,library:selection,library:click=0,wait:200,library:click=1:ctrl,wait:200,library:selection,library:click=0:ctrl,wait:200,library:selection,library:click=0,wait:200,library:click=1:shift,wait:200,library:selection,library:rightclick=0,wait:200,library:selection,library:click-empty,wait:200,library:rightclick=1,wait:200,library:selection,quit' '
    S=$(grep "^selection:" <<<"$OUT" | sed "s/ | model.*//")
    [ "$(sed -n 1p <<<"$S")" = "selection: [0, 1]" ]; check "Select All selects every book" $?
    [ "$(sed -n 2p <<<"$S")" = "selection: []" ]; check "a click on the empty space clears the selection" $?
    [ "$(sed -n 3p <<<"$S")" = "selection: [0, 1]" ]; check "Ctrl+click adds a book to the selection" $?
    [ "$(sed -n 4p <<<"$S")" = "selection: [1]" ]; check "Ctrl+click on a selected book takes it out" $?
    [ "$(sed -n 5p <<<"$S")" = "selection: [0, 1]" ]; check "Shift+click extends the range" $?
    [ "$(sed -n 6p <<<"$S")" = "selection: [0, 1]" ]; check "a right-click on a selected book keeps the whole selection" $?
    [ "$(sed -n 7p <<<"$S")" = "selection: [1]" ]; check "a right-click on an unselected book selects only that book" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

# enhancement 24: the cover tiles keep their size when a panel opens or the window gets narrower; the grid reflows
scenario grid_reflow_1400x900 'library:grid,wait:800,library:tiles,library:sidebar-toggle,wait:1000,library:tiles,library:sidebar-toggle,wait:1000,library:tiles,library:select1,wait:800,library:tiles,library:unselect,wait:800,library:tiles,library:narrow,wait:800,library:tiles,library:wide,wait:600,library:tiles,quit' '
    T=$(grep "^tiles:" <<<"$OUT")
    size() { sed -n "$1p" <<<"$T" | sed "s/.*size=\([0-9x]*\) .*/\1/"; }
    cols() { sed -n "$1p" <<<"$T" | sed "s/.*cols=\([0-9]*\) .*/\1/"; }
    reqs() { sed -n "$1p" <<<"$T" | sed "s/.*requeries=\([0-9]*\)/\1/"; }
    [ "$(size 1)" != "0x0" ]; check "the cover grid has tiles" $?
    for i in 2 3 4 5 6 7; do [ "$(size $i)" = "$(size 1)" ]; check "the tile keeps its size (report $i: $(size $i))" $?; done
    [ "$(reqs 2)" = "$(reqs 1)" ] && [ "$(reqs 3)" = "$(reqs 1)" ]; check "closing and opening the sidebar does not rebuild the list of covers" $?
    [ "$(cols 2)" -gt "$(cols 1)" ] && [ "$(cols 3)" = "$(cols 1)" ]; check "closing the sidebar lets another book into the row, opening it moves it back ($(cols 1) -> $(cols 2) -> $(cols 3))" $?
    [ "$(cols 4)" -lt "$(cols 1)" ]; check "the details panel takes room, so a book moves to the next row ($(cols 1) -> $(cols 4) columns)" $?
    [ "$(cols 5)" = "$(cols 1)" ]; check "and it moves back when the panel closes" $?
    [ "$(cols 6)" -ge "$(cols 1)" ]; check "a narrow window (sidebar overlay) gives the books the whole width" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

# enhancement 25: the star of a favourite, the rating, the tags, and searching by them
scenario favourites_rating_tags 'library:select1,wait:500,library:favourite,wait:600,library:favbadges,library:rate=8,wait:900,library:favbadges,library:tags=+Alpha;beta,wait:900,library:favbadges,library:tags=-alpha,wait:900,library:favbadges,library:search=fav:true,wait:500,library:visible,library:search=rating:4,wait:500,library:visible,library:search=tag:beta,wait:500,library:visible,library:search=-tag:beta,wait:500,library:visible,library:search=fav:false,wait:500,library:visible,library:search=rating:0,wait:500,library:visible,library:search=,wait:500,library:togglefav=0,wait:600,library:favbadges,quit' '
    F=$(grep "^favbadges:" <<<"$OUT")
    V=$(grep "^visible:" <<<"$OUT" | sed "s/ |.*//")
    sed -n 1p <<<"$F" | grep -q "visible=1 favourites=1"; check "a favourite shows the star badge on its cover" $?
    sed -n 2p <<<"$F" | grep -q "rating=8"; check "the rating of the selected book is saved (4 stars = 8)" $?
    sed -n 3p <<<"$F" | grep -q "tags=.*Alpha.*beta"; check "tags are added (commas or semicolons, several at once)" $?
    sed -n 4p <<<"$F" | grep -q "tags=beta]"; check "and removed, whatever their case" $?
    grep -q "\"rating\": 8" <<<"$LIB"; check "the rating is in the catalogue" $?
    [ "$(sed -n 1p <<<"$V")" = "visible: 1" ]; check "fav:true finds the starred book" $?
    [ "$(sed -n 2p <<<"$V")" = "visible: 1" ]; check "rating:4 finds the 4-star book" $?
    [ "$(sed -n 3p <<<"$V")" = "visible: 1" ]; check "tag:beta finds the tagged book" $?
    [ "$(sed -n 4p <<<"$V")" = "visible: 1" ]; check "-tag:beta finds the other book" $?
    [ "$(sed -n 5p <<<"$V")" = "visible: 1" ]; check "fav:false finds the books that are not favourites" $?
    [ "$(sed -n 6p <<<"$V")" = "visible: 1" ]; check "rating:0 finds the unrated book" $?
    sed -n 5p <<<"$F" | grep -q "visible=0 favourites=0"; check "clicking the badge (the same toggle) takes the star off again" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

# every icon name that the sidebar, the headers, the menus, the details panel and Library Health use exists in the icon theme
scenario icons_exist 'library:select1,wait:600,library:icons,library:health=open,wait:800,library:icons,library:health=tab=duplicates,library:health=tab=attention,wait:300,library:icons,library:nav=notes,wait:600,library:icons,prefs:,wait:900,prefs:icons,quit' '
    [ "$(grep -c "^icons: checked=" <<<"$OUT")" -eq 4 ]; check "the walk over the windows ran four times" $?
    ! has "^icons: .*missing=\\[\"" ; check "no widget asks for an icon that is not in the theme" $?
    has "^prefs_icons: checked=[0-9]+ .*missing=\\[\\]"; check "every section of the Preferences sidebar has an icon that exists in the theme" $?
    grep "^icons:" <<<"$OUT" | head -1 | grep -q "missing=\\[\\]"; check "the library window with its sidebar, headers and panel uses only icons that exist" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

# the Continue Reading banner (task A): hero on a cold start, strip on the visits after, a continuous layout
# in the room, the books never move while it collapses, and the setting overrides
scenario banner_session 'library:lastread,wait:1500,library:viewport=1330x2400,library:banner,library:hero-leave,library:banner,library:hero-away,library:banner,library:hero-reset,library:banner,library:viewport=500x900,library:banner,library:viewport=1330x380,library:banner,library:viewport=1330x2400,library:banner,library:banner-scroll=150,wait:300,library:banner,library:gridpos,library:banner-scroll=5000,wait:300,library:banner,library:gridpos,library:banner-scroll=-5000,wait:300,library:banner,library:banner-mode=hidden,library:banner,library:banner-mode=compact,library:banner,library:banner-mode=large,library:hero-leave,library:banner,quit' '
    B=$(grep "^banner:" <<<"$OUT")
    G=$(grep "^gridpos:" <<<"$OUT")
    val() { sed -n "$1p" <<<"$B" | sed "s/.* $2=\([^ ]*\).*/\1/"; }
    sed -n 1p <<<"$B" | grep -q "shown=true kind=Hero variant=wide"; check "a cold start shows the hero, wide on a wide area" $?
    python3 -c "import sys; p=float(sys.argv[1]); assert 30<=p<=35,p" "$(val 1 percent)"; check "the hero takes 30-35 % of a big viewport ($(val 1 percent) %)" $?
    [ "$(val 1 card_w)" -ge "$(( $(val 1 area_w) * 9 / 10 ))" ]; check "it is a full-width card, not a floating square (card $(val 1 card_w) of $(val 1 area_w) px)" $?
    sed -n 2p <<<"$B" | grep -q "kind=Strip variant=strip"; check "after the user left Home and came back: the slim strip" $?
    [ "$(val 2 full)" -le 88 ]; check "the strip is one slim row ($(val 2 full) px with margins)" $?
    sed -n 3p <<<"$B" | grep -q "kind=Hero"; check "away for half an hour: the hero again" $?
    sed -n 4p <<<"$B" | grep -q "kind=Hero"; check "a new run starts with the hero" $?
    sed -n 5p <<<"$B" | grep -q "variant=strip"; check "on a small viewport even the hero is a strip (at most 25 %: $(val 5 percent) %)" $?
    sed -n 6p <<<"$B" | grep -q "variant=mini"; check "on a very narrow area the strip is just the title and a play button" $?
    [ "$(val 8 visible)" -lt "$(val 7 visible)" ] && [ "$(val 8 off)" -gt 0 ]; check "scrolling shrinks the banner ($(val 7 visible) -> $(val 8 visible) px)" $?
    sed -n 1p <<<"$G" | grep -q "scrolled=0"; check "the books have not moved while the banner collapses" $?
    [ "$(val 9 visible)" -eq 0 ]; check "scrolled further, it slides off completely" $?
    [ "$(val 10 visible)" = "$(val 7 visible)" ]; check "back at the top it has its full size again" $?
    sed -n 11p <<<"$B" | grep -q "shown=false"; check "the Hidden setting hides it" $?
    sed -n 12p <<<"$B" | grep -q "variant=strip"; check "Compact forces the strip" $?
    sed -n 13p <<<"$B" | grep -q "variant=wide"; check "Large forces the hero" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

# the resize ladder: 12 widths of the book area, then 12 heights
LADDER=
for w in 300 380 460 560 660 760 860 960 1100 1300 1700 2400; do LADDER="$LADDER""library:viewport=1000x$w,library:banner,"; done
for h in 500 600 700 800 900 960 1000 1100 1200 1300 1400 1500; do LADDER="$LADDER""library:viewport=${h}x2000,library:banner,"; done
scenario banner_ladder "library:lastread,wait:1500,$LADDER"'quit' '
    B=$(grep "^banner:" <<<"$OUT")
    rank() { case $1 in mini) echo 0 ;; strip) echo 1 ;; medium) echo 2 ;; wide) echo 3 ;; esac; }
    prev=0; ok=0
    for i in $(seq 1 12); do
        v=$(sed -n "${i}p" <<<"$B" | sed "s/.* variant=\([a-z]*\) .*/\1/"); r=$(rank "$v")
        [ "$r" -ge "$prev" ] || ok=1; prev=$r
    done
    [ "$ok" = 0 ]; check "growing the width only ever gives a richer layout (mini, strip, medium, wide) - monotonic over 12 widths" $?
    sed -n 1p <<<"$B" | grep -q "variant=mini"; check "300 px: title and play button" $?
    sed -n 12p <<<"$B" | grep -q "variant=wide"; check "2400 px: the wide hero" $?
    prev=0; ok=0
    for i in $(seq 13 24); do
        f=$(sed -n "${i}p" <<<"$B" | sed "s/.* full=\([0-9]*\) .*/\1/")
        [ "$f" -ge "$prev" ] || ok=1; prev=$f
    done
    [ "$ok" = 0 ]; check "growing the height never makes the banner smaller - monotonic over 12 heights" $?
    printf "%s\n" "$B" | python3 "$HERE/banner-ladder-check.py"; check "the strip never takes more than a quarter of the viewport" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'


# Library Health: the tabs that replaced Find Duplicates, Editions & Formats and Needs Attention
scenario health_tabs 'library:health=open-tab=duplicates,wait:800,library:health=dump,library:health=tab=editions,library:health=dump,library:health=tab=attention,library:health=dump,library:health=tab=backups,library:health=dump,quit' '
    T=$(grep "^health_tab:" <<<"$OUT")
    [ "$(sed -n 1p <<<"$T")" = "health_tab: duplicates" ]; check "Library Health opens on the tab asked for" $?
    [ "$(sed -n 2p <<<"$T")" = "health_tab: editions" ]; check "the Editions & Formats tab" $?
    [ "$(sed -n 3p <<<"$T")" = "health_tab: attention" ]; check "the Needs Attention tab" $?
    [ "$(sed -n 4p <<<"$T")" = "health_tab: backups" ]; check "the Backups tab" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'


scenario device_window 'devwin:show,wait:1500,devwin:dump,devwin:device=SDCARD,wait:1200,devwin:dump,devwin:device=KOBO,wait:1200,devwin:dump,devwin:sync,wait:1500,devwin:dump,devwin:device=Boox,wait:1200,devwin:sync,wait:1500,devwin:select=0,devwin:remove,wait:1800,devwin:dump,quit' '
    has "^devwin: devices=4 "; check "the Boox, the Kobo, the Kindle and the SD card are listed" $?
    has "^devwin: .*selected=E-reader \\(SDCARD\\) books=1 .*summary=E-reader · SD card · [0-9.]+ [kMGT]?B free of"; check "an SD card is told from main memory and shows its free space" $?
    has "^devwin: .*selected=Kobo \\(KOBOeReader\\) books=1 .*shelf_row=true .*summary=Kobo · Main memory"; check "the Kobo shows its books and the shelf setting" $?
    has "^devwin_sync: read=1 updated=1"; check "the Kobo database gives one reading state and updates the library" $?
    has "^devwin_sync_item: UI Test Book 0% -> 60% \\(Kobo database\\)"; check "the book read to 60% on the Kobo moves forward in the library" $?
    has "^devwin_book: Library/UI Test Book.epub · .* · Shelf: Favourites · Reading 60% \\(Kobo database\\)"; check "the book list shows the Kobo shelf and the state from the device" $?
    has "^devwin_sync_item: Second Book 0% -> 100% \\(KOReader\\)"; check "a KOReader sidecar marks the finished book" $?
    grep -q "\"progress\": 0.6" <<<"$LIB"; check "the library keeps the 60%" $?
    grep -q "\"progress\": 1.0\|\"progress\": 1," <<<"$LIB"; check "and the finished book" $?
    has "^devwin_removed: ok=1 failed=0"; check "a selected book is removed from the device" $?
    [ ! -e "$d/mounts/Boox/Books/Alpha.epub" ] && [ -e "$d/mounts/Boox/Books/Second Book.epub" ]; check "only the selected file is deleted" $?
    [ -e "$d/mounts/SDCARD/Books/On Card.epub" ]; check "the SD card is untouched" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario device_options 'devwin:show,wait:1200,devwin:format=epub,devwin:template={author}/{title},devwin:shelf=Bookx,devwin:dump,library:send=KOBO,wait:800,devwin:template={nope},devwin:dump,wait:600,quit' '
    has "^devwin: .* format=epub template={author}/{title} template_error=false "; check "the chosen format and save path are shown" $?
    F=$(find "$d/mounts/KOBOeReader/Bookx" -name "*.epub" 2>/dev/null | head -1)
    [ "${F##*/Bookx/}" = "Test Author/UI Test Book.epub" ]; check "the book is sent as EPUB into the author folder named by the template" $?
    python3 - "$d/mounts/KOBOeReader/.kobo/KoboReader.sqlite" <<PY
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
rows = c.execute("SELECT ContentId FROM ShelfContent WHERE ShelfName = \"Bookx\"").fetchall()
sys.exit(0 if rows == [("file:///mnt/onboard/Bookx/Test Author/UI Test Book.epub",)] else 1)
PY
    check "the sent book is put on the Kobo shelf" $?
    [ -f "$d/mounts/KOBOeReader/.kobo/KoboReader.sqlite.bookx-backup" ]; check "the Kobo database was backed up before the change" $?
    has "^devwin: .* template={nope} template_error=true"; check "a template with an unknown field is flagged and not used" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario sharing_https 'share:https,wait:400,share:fetchtls:/opds,share:fetchtls:/opds:untrusted,share:fetchtls:/nothing,share:gen,share:stop,share:show,wait:800,quit' '
    has "^share_started: https://127.0.0.1:[0-9]+/"; check "sharing starts over HTTPS with a self-signed certificate" $?
    has "^share_fetchtls /opds untrusted=false: 200"; check "a client that trusts the certificate gets the catalogue" $?
    has "^share_fetchtls /opds untrusted=true: refused"; check "a client that does not trust it is refused" $?
    has "^share_fetchtls /nothing untrusted=false: 404"; check "unknown addresses are 404 over TLS too" $?
    has "^share_cert: true true"; check "a new certificate and key can be made" $?
    [ -n "$(find "$d/cfg" -name "bookx-share-cert.pem")" ] && [ -n "$(find "$d/cfg" -name "bookx-share-key.pem")" ]; check "the pair is kept in the data folder" $?
    ! has "panicked|CRITICAL"; check "the Share window opens without errors" $?'

scenario sharing_https_files "share:httpsown,wait:400,share:fetchtls:/opds:untrusted,share:httpsfiles:/nonexistent/c.pem:/nonexistent/k.pem,share:stop,quit" '
    has "^share_started: https://"; check "a certificate and key chosen by the user are accepted" $?
    has "^share_fetchtls /opds untrusted=true: refused"; check "and a client that does not trust that certificate is refused" $?
    has "^share_error: .*certificate cannot be used"; check "a missing key file is reported and nothing is served" $?'

mkdir -p "$ROOT/out"
scenario convert_formats "library:convertto=kepub:$ROOT/out/t.kepub.epub,library:convertto=mobi:$ROOT/out/t.mobi,library:convertto=azw3:$ROOT/out/t.azw3,library:convertto=fb2:$ROOT/out/t.fb2,library:convertto=bogus:$ROOT/out/x,quit" '
    magic() { dd if="$1" bs=1 skip=60 count=8 2>/dev/null; }
    has "convertto: mobi -> Some\(Ok"; check "the converter reports success for MOBI" $?
    [ "$(magic "$ROOT/out/t.mobi")" = "BOOKMOBI" ]; check "the MOBI file has a PalmDB/MOBI header" $?
    [ "$(magic "$ROOT/out/t.azw3")" = "BOOKMOBI" ]; check "the AZW3 file has a MOBI/KF8 header" $?
    grep -q "<FictionBook" "$ROOT/out/t.fb2"; check "the FB2 file is a FictionBook" $?
    grep -q "UI Test Book" "$ROOT/out/t.fb2"; check "the FB2 file keeps the title" $?
    if command -v unzip >/dev/null; then
        unzip -p "$ROOT/out/t.kepub.epub" "*chapter-1*" 2>/dev/null | grep -q koboSpan; check "the KEPUB file has Kobo spans" $?
    fi
    [ ! -e "$ROOT/out/x" ]; check "an unknown target writes nothing" $?'

scenario convert_more_formats "library:convertto=docx:$ROOT/out/m.docx,library:convertto=rtf:$ROOT/out/m.rtf,library:convertto=pdb:$ROOT/out/m.pdb,library:convertto=pmlz:$ROOT/out/m.pmlz,library:convertto=rb:$ROOT/out/m.rb,library:convertto=snb:$ROOT/out/m.snb,library:convertto=oeb:$ROOT/out/m.oeb,library:convertto=zip:$ROOT/out/m.zip,quit" '
    [ "$(head -c2 "$ROOT/out/m.docx" 2>/dev/null)" = PK ]; check "DOCX is written as a zip package" $?
    [ "$(head -c5 "$ROOT/out/m.rtf" 2>/dev/null)" = "{\\rtf" ]; check "RTF starts with the RTF header" $?
    [ "$(dd if="$ROOT/out/m.pdb" bs=1 skip=60 count=8 2>/dev/null)" = "PNRdPPrs" ]; check "PDB is an eReader database" $?
    [ "$(head -c2 "$ROOT/out/m.pmlz" 2>/dev/null)" = PK ]; check "PMLZ is a zip archive" $?
    [ -s "$ROOT/out/m.rb" ] && [ "$(head -c2 "$ROOT/out/m.rb" | xxd -p)" = b00c ]; check "RB has the Rocket eBook magic" $?
    [ "$(head -c4 "$ROOT/out/m.snb" 2>/dev/null)" = SNBP ]; check "SNB has the SNBP magic" $?
    [ "$(head -c2 "$ROOT/out/m.oeb" 2>/dev/null)" = PK ] && [ "$(head -c2 "$ROOT/out/m.zip" 2>/dev/null)" = PK ]; check "OEB and ZIP are zip packages" $?
    if command -v unzip >/dev/null; then
        unzip -p "$ROOT/out/m.docx" word/document.xml 2>/dev/null | grep -q "UI Test Book\|functionality"; check "the DOCX holds the book text" $?
    fi'

scenario sharing 'share:start:0,wait:400,share:fetch:/opds,share:fetch:/,share:fetch:/nothing-here,share:stop,share:show,wait:800,quit' '
    has "^share_started: http://127.0.0.1:[0-9]+/"; check "sharing starts on this computer only, on a free port" $?
    has "^share_fetch /opds: 200"; check "the OPDS catalogue answers" $?
    has "^share_fetch /: 200"; check "the web page answers" $?
    has "^share_fetch /nothing-here: 404"; check "unknown addresses are 404" $?
    has "^share_stopped"; check "sharing can be stopped" $?
    ! has "panicked"; check "the sharing window opens without errors" $?'

scenario sharing_kosync 'share:kosync,wait:500,share:show,wait:800,share:kodump,quit' '
    has "^share_ko: visible=true address=http://127\\.0\\.0\\.1:[0-9]+/kosync"; check "with the KOReader switch on the window shows the exact address to type into KOReader, ending in /kosync" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario sharing_lan 'share:lan:short,share:lan:long-enough-pw,wait:400,share:fetch:/opds,share:stop,quit' '
    has "^share_error: .*password"; check "a short password is refused before the network is opened" $?
    has "^share_started: "; check "a proper password lets sharing start" $?
    has "^share_fetch /opds: 401"; check "other devices without the password get 401" $?'

scenario vocabulary 'vocab:dump,read:1,wait:3500,reader:select,wait:1200,reader:ui-dict,wait:900,reader:dump,vocab:dump,vocab:show,wait:600,vocab:grade:4,vocab:dump,vocab:skew:90000,vocab:dump,quit' '
    V=$(grep "^vocab:" <<<"$OUT")
    [ "$(sed -n 1p <<<"$V")" = "vocab: total=0 due=0 learning=0 mastered=0" ]; check "the vocabulary starts empty" $?
    sed -n 2p <<<"$V" | grep -q "total=1 due=1"; check "a dictionary lookup adds the word, due for review" $?
    grep "^vocab_words:" <<<"$OUT" | sed -n 2p | grep -q "Chapter=A main division of a book"; check "the definition is stored with the word" $?
    has "^dict: .*Saved to your vocabulary"; check "the lookup page says the word was saved" $?
    sed -n 3p <<<"$V" | grep -q "total=1 due=0"; check "grading a card schedules it for later" $?
    sed -n 4p <<<"$V" | grep -q "total=1 due=1"; check "the card is due again after a day" $?
    ! has "panicked"; check "the vocabulary window opens without errors" $?'

scenario send_to_device 'library:devices,library:send=KOBO,library:send=Kindle,library:send=nothing-like-this,quit' '
    has "^devices: 2"; check "a Kobo and a Kindle are detected" $?
    [ -n "$(find "$d/mounts/KOBOeReader" -name "*.kepub.epub" 2>/dev/null)" ]; check "the book is sent to the Kobo as KEPUB" $?
    A=$(find "$d/mounts/Kindle/documents" -name "*.azw3" 2>/dev/null | head -1)
    [ -n "$A" ]; check "the book is sent to the Kindle as AZW3" $?
    [ "$(dd if="$A" bs=1 skip=60 count=8 2>/dev/null)" = "BOOKMOBI" ]; check "the Kindle file has a valid MOBI/KF8 header" $?
    has "^send: no device or book"; check "an unknown device sends nothing" $?'

scenario send_with_notes 'library:annotate,library:sendnotes=KOBO,library:sendnotes=Kindle,quit' '
    has "^annotate: done"; check "the first book is given a highlight, a note and a bookmark" $?
    has "^sendnotes: Ok\(.*sidecar=true"; check "the Kobo send reports a KOReader sidecar" $?
    K=$(find "$d/mounts/KOBOeReader" -name "*.kepub.epub" 2>/dev/null | head -1)
    [ -n "$K" ]; check "the book is sent to the Kobo as KEPUB" $?
    unzip -p "$K" 2>/dev/null | grep -a -q "devnote remember this"; check "the KEPUB has the note in its Annotations chapter" $?
    unzip -p "$K" 2>/dev/null | grep -a -q "devbookmark"; check "the KEPUB lists the bookmark" $?
    S=$(find "$d/mounts/KOBOeReader" -path "*.sdr/metadata.epub.lua" 2>/dev/null | head -1)
    [ -n "$S" ] && grep -q "devhighlight text" "$S" && grep -q "devnote remember this" "$S"; check "the KOReader sidecar holds the highlight and its note" $?
    [ -n "$(find "$d/mounts/Kindle/documents" -name "*.azw3" 2>/dev/null)" ]; check "the book is sent to the Kindle as AZW3" $?
    [ -z "$(find "$d/mounts/Kindle" -name "*.sdr" 2>/dev/null)" ]; check "no KOReader sidecar is written for the Kindle" $?'

scenario fulltext_search 'wait:2500,library:fulltext,library:search=text:functionality,wait:600,library:dump,library:search=text:zzzzqqqq,wait:600,library:dump,library:search=text:"talks about",wait:600,library:dump,library:search=title:second text:functionality,wait:600,library:dump,library:search=,wait:400,library:dump,quit' '
    has "^fulltext: indexed=Some\(2\)"; check "both test books are indexed in the background" $?
    T=$(grep "^title:" <<<"$OUT")
    sed -n 1p <<<"$T" | grep -q "2 of 2 books"; check "text:functionality finds the books that contain the word" $?
    sed -n 2p <<<"$T" | grep -q "0 of 2 books"; check "a word that is in no book finds nothing" $?
    sed -n 3p <<<"$T" | grep -q "2 of 2 books"; check "a quoted phrase is searched as a phrase" $?
    sed -n 4p <<<"$T" | grep -q "1 of 2 books"; check "text: combines with ordinary fields" $?
    sed -n 5p <<<"$T" | grep -q "2 books"; check "clearing the search shows every book again" $?'

scenario fetch_metadata 'library:select1,wait:800,library:editmeta,wait:2500,library:metadump,library:fetchmeta,wait:1800,library:metadump,library:fetchmeta-replace,wait:3000,library:metadump,quit' '
    D=$(grep "^metadump:" <<<"$OUT")
    has "^fetchmeta: true"; check "the Fetch Metadata button is in the edit dialog" $?
    sed -n 1p <<<"$D" | grep -q "publisher=\"\""; check "the book starts without a publisher" $?
    sed -n 2p <<<"$D" | grep -q "publisher=\"Mock Press\""; check "fetching fills the empty publisher" $?
    sed -n 2p <<<"$D" | grep -q "published=\"2001"; check "fetching fills the publication date" $?
    sed -n 2p <<<"$D" | grep -q "9780441172719"; check "fetching adds the ISBN as an identifier" $?
    sed -n 2p <<<"$D" | grep -q "status=\"Found on"; check "the dialog says where the data was found" $?
    sed -n 2p <<<"$D" | grep -q "title=\"Second Book\""; check "the title is kept" $?
    tail -1 <<<"$D" | grep -q "busy=false"; check "the button is usable again after the lookup" $?'

scenario book_styles "library:import=$ROOT/css-epub/lib,wait:2500,read:3,wait:3500,reader:dump,reader:ui-styles-off,wait:1200,reader:dump,reader:ui-styles-on,wait:1200,reader:dump,quit" '
    S=$(grep "^styled:" <<<"$OUT")
    sed -n 1p <<<"$S" | grep -q "italic=1 bold=1 centered=1"; check "the CSS classes of the book make text italic, bold and centred" $?
    sed -n 2p <<<"$S" | grep -q "italic=0 bold=0 centered=0"; check "with the styling of the book switched off the plain structure remains" $?
    sed -n 3p <<<"$S" | grep -q "italic=1 bold=1 centered=1"; check "switching the styling back on restores it" $?
    I=$(grep "^indented:" <<<"$OUT")
    sed -n 1p <<<"$I" | grep -q "indented: 2 of "; check "paragraphs with a text-indent in the CSS get an indented first line" $?
    sed -n 2p <<<"$I" | grep -q "indented: 0 of "; check "no indent without the styling of the book" $?
    sed -n 3p <<<"$I" | grep -q "indented: 2 of "; check "the indent returns when the styling is switched back on" $?'

scenario book_colors "library:import=$ROOT/css-epub/lib,wait:2500,read:3,wait:3500,reader:dump,reader:ui-colors-original,wait:1200,reader:dump,reader:ui-colors-custom,wait:1200,reader:dump,reader:ui-colors-theme,wait:1200,reader:dump,quit" '
    P=$(grep "^palette:" <<<"$OUT"); C=$(grep "^colored:" <<<"$OUT")
    sed -n 1p <<<"$C" | grep -q "colored: 0"; check "with the theme colours the text colours of the book are not used" $?
    sed -n 2p <<<"$P" | grep -q "palette: fdf6e3/"; check "Original shows the page colour of the book" $?
    sed -n 2p <<<"$C" | grep -Eq "colored: [1-9]"; check "Original shows the text colours of the book" $?
    sed -n 3p <<<"$P" | grep -q "palette: 101010/ffcc00"; check "Custom uses the colours picked by the user" $?
    sed -n 3p <<<"$C" | grep -q "colored: 0"; check "Custom ignores the text colours of the book" $?
    sed -n 4p <<<"$P" | grep -qv "fdf6e3"; check "going back to the theme drops the book page colour" $?
    sed -n 4p <<<"$C" | grep -q "colored: 0"; check "going back to the theme drops the book text colours" $?'

scenario touch_gestures 'read:1,wait:3500,reader:dump,reader:ui-swipe:-120:5:200,wait:500,reader:dump,reader:ui-swipe:120:0:200,wait:500,reader:dump,reader:ui-swipe:-120:200:200,reader:ui-swipe:-20:0:100,reader:ui-swipe:-200:0:900,wait:400,reader:dump,quit' '
    page() { grep "^labels:" <<<"$OUT" | sed -n "$1p" | sed "s/^labels: \([0-9]*\) of.*/\1/"; }
    [ "$(page 1)" = "1" ]; check "the book opens on page 1" $?
    [ "$(page 2)" = "3" ]; check "a swipe to the left turns to the next page (two columns: 1 to 3)" $?
    [ "$(page 3)" = "1" ]; check "a swipe to the right turns back" $?
    [ "$(page 4)" = "1" ]; check "vertical, short and slow movements do not turn the page" $?
    has "^swipe: Some\(Left\)"; check "a quick left movement is classified as a swipe" $?
    [ "$(grep -c "^swipe: None" <<<"$OUT")" -eq 3 ]; check "three rejected movements are classified as no swipe" $?'

scenario tap_zones 'read:1,wait:3500,reader:ui-zones:thirds,reader:ui-tap:0.9,wait:400,reader:dump,reader:ui-tap:0.5,wait:300,reader:dump,reader:ui-tap:0.1,wait:400,reader:dump,reader:ui-zones:lefthanded,reader:ui-tap:0.1,wait:400,reader:dump,reader:ui-zones:off,reader:ui-tap:0.9,wait:300,reader:dump,reader:ui-zones:wide,reader:ui-tap:0.5,wait:400,reader:dump,touch:off,touch:on,touch:off,quit' '
    page() { grep "^labels:" <<<"$OUT" | sed -n "$1p" | sed "s/^labels: \([0-9]*\) of.*/\1/"; }
    [ "$(page 1)" = "3" ]; check "thirds: tapping the right third goes forward" $?
    [ "$(page 2)" = "3" ]; check "thirds: tapping the middle does not turn the page" $?
    [ "$(page 3)" = "1" ]; check "thirds: tapping the left third goes back" $?
    [ "$(page 4)" = "3" ]; check "mirrored: tapping the left third goes forward" $?
    [ "$(page 5)" = "3" ]; check "off: taps never turn the page" $?
    [ "$(page 6)" = "5" ]; check "wide: tapping the middle goes forward" $?
    T=$(grep "^touch_ui:" <<<"$OUT")
    [ "$(sed -n 1p <<<"$T")" = "touch_ui: false" ]; check "large touch controls can be switched off" $?
    [ "$(sed -n 2p <<<"$T")" = "touch_ui: true" ]; check "large touch controls can be switched on" $?'

scenario eink_mode 'read:1,wait:3500,reader:dump,eink:on,wait:6500,reader:dump,eink:off,wait:800,reader:dump,quit' '
    P=$(grep "^palette:" <<<"$OUT"); I=$(grep "^idle:" <<<"$OUT"); E=$(grep "^eink:" <<<"$OUT")
    [ "$(sed -n 2p <<<"$P")" = "palette: ffffff/000000" ]; check "e-ink mode draws pure black on white" $?
    [ "$(sed -n 1p <<<"$P")" != "palette: ffffff/000000" ]; check "the normal theme is not pure black" $?
    [ "$(sed -n 3p <<<"$P")" = "$(sed -n 1p <<<"$P")" ]; check "switching e-ink off restores the theme" $?
    [ "$(sed -n 2p <<<"$I")" = "idle: false" ]; check "the bars do not fade away in e-ink mode (checked after 6 seconds)" $?
    sed -n 1p <<<"$E" | grep -q "eink: true animations=false window_class=true"; check "e-ink mode switches the animations off" $?
    sed -n 2p <<<"$E" | grep -q "eink: false animations=true window_class=false"; check "e-ink mode can be switched off again" $?'

scenario migrate_json 'library:dump,quit' '
    grep -q "UI Test Book" <<<"$LIB"; check "a book of the old JSON catalogue is in the database" $?
    [ -f "$d/lib/bookx-library.db" ]; check "the library is now a database" $?
    [ ! -e "$d/lib/bookx-library.json" ]; check "the old JSON catalogue is no longer in use" $?
    [ -f "$d/lib/bookx-library.json.migrated" ]; check "the old catalogue is kept as a backup" $?
    has "^title: Home \\| [0-9]+ books"; check "the library opens with its books" $?
    [ "$(grep -c "\"file\":" <<<"$LIB")" -ge 2 ]; check "the old book and the imported ones are all there" $?'

scenario editor_tools 'edit:1,wait:3000,editor:dump,editor:addfile=stray.css,editor:tool=fix,wait:800,editor:dump,editor:tool=fix,wait:600,editor:dump,editor:tool=arrange,wait:800,editor:dump,editor:compare=last,wait:400,editor:dump,editor:restore=last,wait:800,editor:dump,editor:report,wait:600,editor:checkpoint,editor:tool=nonsense,wait:400,editor:dump,quit' '
    D=$(grep "^editor:" <<<"$OUT"); F=$(grep "^editor_files:" <<<"$OUT")
    sed -n 1p <<<"$D" | grep -q "checkpoints=0"; check "the editor starts without checkpoints" $?
    sed -n 2p <<<"$D" | grep -q "toast=\"Fixed"; check "Fix Problems repairs the stray file and says so" $?
    sed -n 2p <<<"$D" | grep -q "checkpoints=1"; check "a checkpoint is taken before the tool runs" $?
    sed -n 3p <<<"$D" | grep -q "toast=\"Nothing to fix"; check "running Fix Problems again finds nothing" $?
    sed -n 3p <<<"$D" | grep -q "checkpoints=1"; check "a tool that changed nothing leaves no checkpoint behind" $?
    sed -n 4p <<<"$F" | grep -q "/Text/"; check "Arrange Files moves chapters into a Text folder" $?
    sed -n 5p <<<"$D" | grep -q "toast=\"Compared with .Before: Arrange Files in Folders.: [1-9]"; check "comparing with the checkpoint lists the files that arrange moved" $?
    sed -n 6p <<<"$F" | grep -qv "/Text/"; check "restoring the checkpoint brings the old layout back" $?
    sed -n 6p <<<"$D" | grep -q "toast=\"Restored"; check "the restore is announced" $?
    sed -n 7p <<<"$D" | grep -q "Unknown tool nonsense"; check "an unknown tool is reported, not ignored" $?
    ! has "panicked"; check "the report dialog and the menu actions work without errors" $?'

scenario duplicates 'library:dups,library:dupsshow,wait:1000,library:mergedups,wait:800,library:dups,library:dump,quit' '
    D=$(grep "^dups:" <<<"$OUT")
    sed -n 1p <<<"$D" | grep -q "dups: 1 groups"; check "three copies of one book form a single group" $?
    has "^mergedups: removed 2"; check "merging keeps one copy and removes the other two" $?
    sed -n 2p <<<"$D" | grep -q "dups: 0 groups"; check "no duplicates are left afterwards" $?
    [ "$(grep -c "\"file\":" <<<"$LIB")" -eq 2 ]; check "the library holds the merged book and the different one" $?
    has "^title: Home \\| 2 books"; check "the header shows two books" $?
    ! has "panicked|CRITICAL"; check "the duplicates window opens without errors" $?'

scenario news_basic 'library:newsadd=Mock Daily|https://mock.example/feed.xml|daily 07:00|1,library:newsdump,library:newsshow,wait:800,library:newsdialog,library:newsrun=all,wait:3500,library:newsdump,library:newsrun=all,wait:3500,library:newsdump,quit' '
    has "^newsadd: Ok\\(\"mock-daily\"\\)"; check "a subscription is added with a key made from its name" $?
    find "$d/cfg" -name news.json | xargs grep -q "Mock Daily"; check "the subscription is saved in news.json" $?
    D=$(grep "^news_sub:" <<<"$OUT")
    sed -n 1p <<<"$D" | grep -q "schedule=\"Every day at 07:00\".*attempts=0 last_run=none"; check "a new subscription has never run and shows its schedule in local time" $?
    has "^news_dialog: open=true"; check "the News Subscriptions window opens from the menu action" $?
    sed -n 2p <<<"$D" | grep -q "attempts=1 last_run=20.*status=\"Mock Daily: 2 articles"; check "Download Now records the run and the article count" $?
    grep -q "bookx-news" <<<"$LIB"; check "the issue carries the bookx-news identifier" $?
    grep -q "\"News\"" <<<"$LIB"; check "the issue is tagged News" $?
    [ "$(grep -c "\"file\":" <<<"$LIB")" -eq 3 ]; check "with keep 1 the second download replaces the first issue (2 test books + 1 issue)" $?
    sed -n 3p <<<"$D" | grep -q "attempts=2"; check "both runs are counted" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario news_scheduler 'library:newsadd=Hourly Feed|https://mock.example/feed.xml|hourly|3,library:newsadd=Manual Feed|https://mock.example/feed.xml|manual|3,library:newstick,wait:3500,library:newsdump,library:newstick,wait:1500,library:newsdump,stats:skew:3700,library:newstick,wait:3500,library:newsdump,quit' '
    D=$(grep "^news_sub: key=hourly-feed" <<<"$OUT")
    sed -n 1p <<<"$D" | grep -q "attempts=1 last_run=20"; check "a due subscription is downloaded by the scheduler tick" $?
    sed -n 2p <<<"$D" | grep -q "attempts=1"; check "it is not downloaded again before its next slot" $?
    sed -n 3p <<<"$D" | grep -q "attempts=2"; check "an hour later it is due again and runs" $?
    ! grep "^news_sub: key=manual-feed" <<<"$OUT" | grep -qv "attempts=0 last_run=none"; check "a manual subscription is never run by the scheduler" $?
    [ "$(grep -c "\"file\":" <<<"$LIB")" -eq 4 ]; check "the two scheduled runs left two issues (2 test books + 2)" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario news_prune 'library:newsadd=Prune Me|https://mock.example/feed.xml|manual|2,library:newsrun=all,wait:3000,library:newsrun=all,wait:3000,library:newsrun=all,wait:3000,library:newsrun=all,wait:3000,library:newsdump,quit' '
    [ "$(grep -c "bookx-news" <<<"$LIB")" -eq 2 ]; check "after four downloads only the two newest issues are kept" $?
    [ "$(grep -c "\"file\":" <<<"$LIB")" -eq 4 ]; check "the other books are untouched (2 test books + 2 issues)" $?
    grep "^news_sub:" <<<"$OUT" | tail -1 | grep -q "attempts=4"; check "all four runs are recorded" $?'

scenario news_failure 'library:newsadd=Broken|https://mock.example/missing|hourly|3,library:newstick,wait:3000,library:newsdump,library:newstick,wait:1500,library:newsdump,quit' '
    D=$(grep "^news_sub:" <<<"$OUT")
    sed -n 1p <<<"$D" | grep -q "attempts=1 last_run=20.*status=\"Failed"; check "a failed download is recorded with its reason" $?
    sed -n 2p <<<"$D" | grep -q "attempts=1"; check "a failed subscription is not retried at the next tick" $?
    [ "$(grep -c "\"file\":" <<<"$LIB")" -eq 2 ]; check "nothing is added to the library" $?
    has "news download failed"; check "the failure is logged" $?
    ! has "panicked|CRITICAL"; check "the app keeps running" $?'

scenario news_opml "library:newsopml=From OPML|$ROOT/news_opml/feeds.opml,library:newsfeedoff=from-opml|https://mock.example/missing,library:newsrun=all,wait:3500,library:newsdump,quit" '
    has "^newsopml: Ok"; check "the feeds of an OPML file become one subscription" $?
    has "^news_sub: .*feeds=2 off=1"; check "a feed can be switched off and stays in the list" $?
    grep "^news_sub:" <<<"$OUT" | tail -1 | grep -q "status=\"From OPML: 2 articles\""; check "the switched-off feed is skipped, the other one is downloaded without problems" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario add_from_web 'library:addweb=mock.example/post,wait:2500,library:addweb=https://mock.example/feed.xml,wait:2500,library:addweb=https://mock.example/missing,wait:1500,library:addweb=not a url,wait:800,quit' '
    grep -q "A Mocked Post" <<<"$LIB"; check "a web page is saved as a book with its title" $?
    grep -q "Web Writer" <<<"$LIB"; check "the author of the page is kept" $?
    grep -q "Mock Feed" <<<"$LIB"; check "a feed is saved as a news book" $?
    [ "$(grep -c "\"file\":" <<<"$LIB")" -eq 4 ]; check "exactly the two web books were added to the two test books" $?
    has "import failed: .*(404|mock.example/missing)"; check "an address that cannot be downloaded is reported" $?
    has "import failed: .*(spaces|valid address)"; check "a malformed address is refused" $?'

scenario plugin_source 'plugins:dump,plugins:enable=test.source,library:select1,wait:800,library:editmeta,wait:2500,library:fetchmeta,wait:2500,library:metadump,plugins:show,wait:1000,quit' '
    has "^plugin: test.source enabled=false error=None"; check "an installed plugin is listed and starts switched off" $?
    has "^plugin_enable: Ok"; check "the plugin can be enabled" $?
    has "^plugins_active: sources=1 readers=0"; check "an enabled metadata plugin becomes a source" $?
    M=$(grep "^metadump:" <<<"$OUT" | tail -1)
    grep -q "publisher=\"Plugin Press\"" <<<"$M"; check "Fetch Metadata uses the answer of the plugin when every built-in source fails" $?
    grep -q "status=\"Found on plugin test.source" <<<"$M"; check "the status line names the plugin" $?
    grep -q "from a plugin" <<<"$M"; check "the tags of the plugin are filled in" $?
    ! has "panicked|CRITICAL"; check "the Preferences window with the plugin list opens without errors" $?'

scenario plugin_writer "plugins:dump,wait:400,library:targets,wait:400,plugins:enable=org.bookx.example.rawcopy,wait:400,library:targets,library:convertto=plugin:org.bookx.example.rawcopy:$ROOT/out/pw.epubcopy,wait:1500,plugins:disable=org.bookx.example.rawcopy,wait:400,library:targets,library:convertto=plugin:org.bookx.example.rawcopy:$ROOT/out/pw-off.epubcopy,wait:800,plugins:show,wait:1000,quit" '
    has "^plugin: org.bookx.example.rawcopy enabled=false error=None"; check "a writer plugin is listed and starts switched off" $?
    T=$(grep "^targets:" <<<"$OUT")
    ! sed -n 1p <<<"$T" | grep -q "rawcopy"; check "a disabled writer is not offered in Convert To" $?
    sed -n 2p <<<"$T" | grep -q "plugin:org.bookx.example.rawcopy=.*epubcopy.*plugin"; check "an enabled writer appears in Convert To with its extension" $?
    has "plugins_active: .*writers=1"; check "the writer is counted as active" $?
    has "^convertto: plugin:org.bookx.example.rawcopy -> Some\(Ok"; check "converting with the plugin writer succeeds" $?
    [ "$(head -c2 "$ROOT/out/pw.epubcopy" 2>/dev/null)" = PK ] && unzip -l "$ROOT/out/pw.epubcopy" 2>/dev/null | grep -q container.xml; check "the file written by the plugin is the book (an EPUB container)" $?
    ! sed -n 3p <<<"$T" | grep -q "rawcopy"; check "switching the plugin off removes the format again" $?
    [ ! -e "$ROOT/out/pw-off.epubcopy" ]; check "a disabled writer converts nothing" $?
    ! has "panicked|CRITICAL"; check "the Preferences window lists the plugin with its permissions without errors" $?'

scenario plugin_transform "plugins:enable=test.transform,wait:400,library:targets,library:convertto=plugin-tool:test.transform:$ROOT/out/ptool.epub,wait:1500,plugins:auto=test.transform,wait:300,library:convertto=fb2:$ROOT/out/ptool.fb2,wait:1500,edit:1,wait:3000,editor:tool=plugin:test.transform,wait:800,editor:dump,quit" '
    has "^targets: .*plugin-tool:test.transform=Apply Name of test.transform|^targets: .*plugin-tool:test.transform=Apply Test Transform"; check "an enabled transform is offered in the library menu" $?
    [ "$(head -c2 "$ROOT/out/ptool.epub" 2>/dev/null)" = PK ]; check "applying the transform from the library saves a new EPUB" $?
    has "^plugin_auto: Ok"; check "a transform can be chosen as a conversion option" $?
    has "^convertto: fb2 -> Some\(Ok"; check "conversion with a plugin transform in the options still works" $?
    grep "^editor:" <<<"$OUT" | grep -q "Changed 2 files"; check "the editor Tools menu runs the plugin transform and says what it changed" $?
    ! has "panicked|CRITICAL"; check "the transform plugin runs without errors" $?'

scenario plugin_device 'plugins:enable=test.device,wait:400,library:devices,library:send=PluginReader,wait:1500,quit' '
    has "^devices: 3"; check "a device plugin adds its device next to the Kobo and the Kindle" $?
    F="$d/mounts/PluginReader/plugin-books/last-sent.bin"
    [ -s "$F" ] && [ "$(head -c2 "$F")" = PK ]; check "the book is written where the plugin said (an EPUB, the format it asked for)" $?
    [ -z "$(find "$d/mounts/Kindle/documents" -type f 2>/dev/null)" ]; check "other devices are left alone" $?'

scenario patches_manage 'patches:dump,patches:enable=a-shout,patches:enable=e-broken,patches:down=a-shout,patches:dump,patches:preview=a-shout,patches:preview=e-broken,patches:disable=a-shout,patches:dump,patches:show,wait:1200,quit' '
    P=$(grep "^patch:" <<<"$OUT")
    [ "$(sed -n 1,5p <<<"$P" | grep -c "enabled=false")" -eq 5 ]; check "patches start switched off" $?
    sed -n 1p <<<"$P" | grep -q "^patch: a-shout"; check "patches are listed by file name at first" $?
    has "^patch_enable: Ok"; check "a patch can be switched on" $?
    has "^patch_enable: Err.*problem"; check "a broken patch cannot be switched on" $?
    sed -n 16p <<<"$P" | grep -q "^patch: b-whisper"; check "moving a patch later changes the order" $?
    sed -n 17p <<<"$P" | grep -q "^patch: a-shout enabled=true"; check "the moved patch keeps its place and its switch" $?
    has "^patch: e-broken enabled=false hook=None error=Some"; check "the broken patch is listed with its error" $?
    has "^patch_preview: a-shout changes=[1-9]"; check "the dry run reports what the patch would change in the first book" $?
    has "^patch_preview: e-broken error="; check "a broken patch has no dry run" $?
    has "^patches_active: 1"; check "only enabled patches are active" $?
    has "^patches_active: 0"; check "switching it off deactivates it" $?
    unzip -p "$(find "$d/lib" -name "*.epub" | head -1)" "*.xhtml" | grep functionality >/dev/null; check "a dry run leaves the stored book alone" $?
    ! has "panicked|CRITICAL"; check "the Patches list in Preferences opens without errors" $?'

scenario patches_convert "patches:enable=a-shout,patches:enable=b-whisper,wait:400,library:convertto=fb2:$ROOT/out/patched.fb2,wait:2000,patches:disable=a-shout,patches:disable=b-whisper,wait:400,library:convertto=fb2:$ROOT/out/plain.fb2,wait:2000,quit" '
    grep -q "zzqq" "$ROOT/out/patched.fb2"; check "convert-before and convert-after patches ran in order (functionality, ZZQQ, zzqq)" $?
    ! grep -q "ZZQQ" "$ROOT/out/patched.fb2"; check "the second patch saw the result of the first" $?
    grep -q "functionality" "$ROOT/out/plain.fb2" && ! grep -qi "zzqq" "$ROOT/out/plain.fb2"; check "with the patches off the conversion is unchanged" $?
    unzip -p "$(find "$d/lib" -name "*.epub" | head -1)" "*.xhtml" | grep functionality >/dev/null; check "the stored book is never changed by a conversion patch" $?'

scenario patches_import "patches:enable=c-import,library:import=$ROOT/third,wait:3500,quit" '
    F=$(find "$d/lib" -path "*Third*" -name "*.epub" | head -1)
    [ -n "$F" ]; check "the book was imported" $?
    unzip -p "$F" "*.xhtml" | grep IMPORTED >/dev/null; check "the import patch changed the book as it was added" $?
    F2=$(find "$d/lib" -path "*UI*" -name "*.epub" | head -1)
    ! unzip -p "$F2" "*.xhtml" | grep IMPORTED >/dev/null; check "books that were already in the library are untouched" $?'

scenario patches_open 'patches:enable=d-open,read:1,wait:3500,reader:select,wait:600,reader:dump,quit' '
    grep "^selected:" <<<"$OUT" | grep -q "Kapitel"; check "the open patch changes the text the reader shows" $?
    ! unzip -p "$(find "$d/lib" -name "*.epub" | head -1)" "*.xhtml" | grep Kapitel >/dev/null; check "the book on disk keeps its text" $?'

scenario editor_check 'edit:1,wait:3000,editor:addfile=stray.css,editor:check,wait:600,editor:dump,editor:tool=fix,wait:800,editor:check,wait:600,editor:dump,quit' '
    D=$(grep "^editor:" <<<"$OUT")
    sed -n 1p <<<"$D" | grep -Eq "problems=[1-9]"; check "Check lists the stray file (quick checks and EPUB validator together)" $?
    sed -n 2p <<<"$D" | grep -q "problems=2"; check "after Fix Problems the stray file is listed but unused, and the missing cover is a note" $?
    ! has "panicked"; check "the checker runs without errors" $?'

# the Problems panel of the book check (CE-160..CE-165): list, help text, Fix with a checkpoint, ignore a kind, next/previous
scenario editor_problems 'edit:1,wait:3000,editor:addfile=stray.css,editor:open=OEBPS/chapter-1.xhtml,wait:300,editor:find-in-text=Paragraph 2 of,editor:insert=<p style="colour: red">odd</p>,wait:300,editor:check,wait:500,editor:check:dump,editor:check:select-id=OPF-003,editor:check:help,editor:check:fix,wait:900,editor:check:dump,editor:dump,editor:check:select-id=BKX-011,editor:check:run,editor:check:dump,editor:insert=<!-- edited -->,wait:300,editor:check:dump,editor:check:select-id=CHK-003,editor:check:help,editor:check:ignore,wait:300,editor:check:dump,editor:check:prefs,editor:check:restore=CHK-003,editor:check:dump,editor:check:select=0,editor:check:next,editor:check:dump,editor:check:prev,editor:check:prev,editor:check:dump,editor:cur,editor:check:a11y=on,editor:check:dump,editor:check:a11y=off,editor:check:dump,quit' '
    L=$(grep "^check:" <<<"$OUT"); I=$(grep "^check_item:" <<<"$OUT"); H=$(grep "^check_help:" <<<"$OUT")
    sed -n 1p <<<"$L" | grep -Eq "problems=[3-9] .*open=true"; check "Check opens the Problems panel with the findings" $?
    grep -q "check_item: [0-9]*|OPF-003|warning|OEBPS/stray.css" <<<"$I"; check "the stray file is listed with its id, severity and file" $?
    grep -q "check_item: [0-9]*|BKX-011|warning|OEBPS/chapter-1.xhtml" <<<"$I"; check "the unknown CSS property in the style attribute is found" $?
    sed -n 1p <<<"$H" | grep -q "not listed in the manifest" && sed -n 1p <<<"$H" | grep -q "fix=true"; check "selecting a problem shows its help text and a Fix button" $?
    grep "^editor:" <<<"$OUT" | sed -n 1p | grep -q "checkpoints=1"; check "Fix takes a checkpoint first" $?
    ! sed -n 2p <<<"$L" | grep -q "OPF-003"; check "the problem is gone after the fix and the list was checked again" $?
    sed -n 3p <<<"$L" | grep -q "selected=0 "; check "checking again keeps the selected problem selected" $?
    sed -n 4p <<<"$L" | grep -q "stale=1 "; check "editing a file marks its entries as out of date" $?
    sed -n 5p <<<"$L" | grep -q "ignored=CHK-003" && sed -n 5p <<<"$L" | grep -q "hidden=1"; check "Ignore this kind hides the kind, lists it as ignored and counts it as hidden" $?
    grep "^check_prefs:" <<<"$OUT" | grep -q "CHK-003"; check "the ignored kind is remembered in the editor settings" $?
    sed -n 6p <<<"$L" | grep -q "ignored= "; check "restoring the kind shows it again" $?
    sed -n 7p <<<"$L" | grep -q "selected=1 "; check "F8 moves to the next problem" $?
    sed -n 8p <<<"$L" | grep -q "selected=2 "; check "Shift+F8 from the first problem wraps to the last" $?
    grep "^editor_cur:" <<<"$OUT" | grep -q "file=OEBPS/content.opf"; check "going to a problem opens its file in the editor" $?
    ! sed -n 6p <<<"$L" | grep -q "a11y=true" && sed -n 9p <<<"$L" | grep -q "a11y=true"; check "Also check accessibility switches the option on" $?
    sed -n 9p <<<"$L" | grep -Eq "problems=([4-9]|[1-9][0-9])" && grep -q "|ACC-90[0-9]|" <<<"$I"; check "the accessibility findings appear only then" $?
    sed -n 10p <<<"$L" | grep -q "problems=3 "; check "and go away again when it is off" $?
    ! has "panicked"; check "the panel runs without errors" $?'

scenario editor_spelling 'edit:1,wait:3000,editor:spelling,wait:600,editor:dump,quit' '
    grep "^editor:" <<<"$OUT" | grep -Eq "results=[1-9]|No spelling"; check "Check Spelling lists mistakes or says why it cannot" $?
    ! has "panicked"; check "spell checking runs without errors" $?'

# ---- the spelling panel, reports window and special characters (CE-270..CE-281, CE-300..CE-311) -------
scenario editor_live_spellpanel 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:500,editor:find-in-text=Paragraph 2 of,editor:insert=<p>Wrods and tehe wrods</p>,editor:open=OEBPS/chapter-2.xhtml,wait:300,editor:find-in-text=Paragraph 2 of,editor:insert=<p>Wrods and fox</p>,editor:spell:dump,editor:spell:open,wait:1500,editor:spell:dump,editor:spell:select=Wrods,wait:300,editor:spell:sel,editor:spell:jump=Wrods,wait:300,editor:spell:sel,editor:spell:select=wrods,wait:300,editor:spell:dump,editor:open=OEBPS/chapter-1.xhtml,editor:cursor=0,editor:spell:f8,editor:spell:sel,editor:spell:f8,editor:spell:sel,editor:spell:f8,editor:spell:sel,editor:spell:f8,editor:spell:sel,editor:spell:shift-f8,editor:spell:sel,editor:spell:select=wrods,wait:200,editor:spell:change=words,wait:1500,editor:spell:dump,editor:open=OEBPS/chapter-1.xhtml,editor:has=<p>Wrods and tehe words</p>,editor:dump,editor:spell:select=tehe,wait:200,editor:spell:ignore,wait:1500,editor:spell:dump,editor:spots,editor:spell:select=Wrods,wait:200,editor:spell:add=Personal,wait:1500,editor:spell:dump,editor:find-in-text=Paragraph 3 of,editor:insert=<p>zzqx</p>,editor:spell:dump,editor:spell:refresh,wait:1500,editor:spell:dump,editor:spell:all=on,wait:500,editor:spell:dump,editor:spell:filter=zzq,wait:300,editor:spell:dump,quit' '
    P=$(grep "^editor_spell_panel:" <<<"$OUT")
    sed -n 1p <<<"$P" | grep -q "open=false"; check "the panel is closed until Check Spelling is used" $?
    sed -n 2p <<<"$P" | grep -q "rows=3 words=Wrods/en/2|tehe/en/1|wrods/en/1 "; check "the panel lists the misspelled words of the whole book with counts and language, most frequent first (CE-270)" $?
    S=$(grep "^editor_spell_sel:" <<<"$OUT")
    sed -n 1p <<<"$S" | grep -q "chapter-1.xhtml text=Wrods"; check "selecting a word selects its next occurrence in the editor (CE-272)" $?
    sed -n 2p <<<"$S" | grep -q "chapter-2.xhtml text=Wrods"; check "activating it again goes on to the following occurrence, in the next file" $?
    sed -n 3p <<<"$P" | grep -q "sugg=.*words"; check "the panel offers suggestions for the selected word" $?
    sed -n 3p <<<"$P" | grep -q "entry=wrods"; check "the replacement field starts with the word itself" $?
    sed -n 3p <<<"$S" | grep -q "text=Wrods" && sed -n 4p <<<"$S" | grep -q "text=tehe" && sed -n 5p <<<"$S" | grep -q "text=wrods"; check "F8 walks through the misspellings of the file (CE-280)" $?
    sed -n 6p <<<"$S" | grep -q "chapter-2.xhtml text=Wrods"; check "F8 at the end of a file continues in the next file with a mistake" $?
    sed -n 7p <<<"$S" | grep -q "chapter-1.xhtml text=wrods"; check "Shift+F8 goes back, across files" $?
    sed -n 4p <<<"$P" | grep -q "words=Wrods/en/2|tehe/en/1 "; check "Change All replaced the word everywhere and the list was rebuilt (CE-273)" $?
    has "^editor_has: <p>Wrods and tehe words</p> = 1"; check "the change is in the chapter text" $?
    grep "^editor:" <<<"$OUT" | grep -Eq "checkpoints=[1-9]"; check "Change All took a checkpoint first" $?
    sed -n 5p <<<"$P" | grep -q "words=Wrods/en/2 "; check "Ignore removes the word from the list (CE-274)" $?
    ! grep "^editor_spots:" <<<"$OUT" | tail -1 | grep -q "tehe"; check "the squiggles follow (the ignored word is not underlined)" $?
    sed -n 6p <<<"$P" | grep -q "rows=0"; check "Add to Dictionary removes the word from the list (CE-275)" $?
    grep -qx "Wrods" "$d/cfg/bookx/spelling-words.txt"; check "the added word is saved in the Personal dictionary file" $?
    sed -n 7p <<<"$P" | grep -q "rows=0"; check "typing a new mistake does not change the list by itself" $?
    sed -n 8p <<<"$P" | grep -q "words=zzqx/en/1"; check "Refresh finds the new mistake (CE-277)" $?
    sed -n 9p <<<"$P" | grep -Eq "rows=[1-9][0-9] "; check "Show all words lists every word of the book with its count (CE-276)" $?
    sed -n 10p <<<"$P" | grep -q "rows=1 words=zzqx/en/1"; check "the filter narrows the list" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "the spelling panel runs without errors or GTK warnings" $?'

scenario editor_live_spellpanel_scope 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:500,editor:find-in-text=Paragraph 2 of,editor:insert=<p>Wrods fox</p>,editor:open=OEBPS/chapter-2.xhtml,wait:300,editor:find-in-text=Paragraph 2 of,editor:insert=<p>Wrods quick tehe</p>,editor:spell:open,wait:1500,editor:spell:dump,editor:spell:exclude,wait:1500,editor:spell:dump,editor:spots,editor:spell:excluded-dialog,wait:300,editor:spell:include=OEBPS/chapter-2.xhtml,wait:1500,editor:spell:dump,editor:open=OEBPS/chapter-1.xhtml,editor:find-in-text=<p>Wrods fox,editor:insert=<!-- bookx-no-spell-check -->,editor:spell:refresh,wait:1500,editor:spell:dump,editor:spell:select=tehe,wait:200,editor:spell:ignore-book,wait:1500,editor:spell:dump,quit' '
    P=$(grep "^editor_spell_panel:" <<<"$OUT")
    sed -n 1p <<<"$P" | grep -q "words=Wrods/en/2|tehe/en/1 "; check "both chapters are checked at first" $?
    sed -n 2p <<<"$P" | grep -q "words=Wrods/en/1 .*excluded=OEBPS/chapter-2.xhtml"; check "an excluded file is left out of the list (CE-278)" $?
    grep "^editor_spots:" <<<"$OUT" | head -1 | grep -q " n=0 "; check "and its words are not underlined any more" $?
    sed -n 3p <<<"$P" | grep -q "words=Wrods/en/2|tehe/en/1 .*excluded= "; check "including the file again checks it again" $?
    sed -n 4p <<<"$P" | grep -q "words=tehe/en/1|Wrods/en/1 .*3 files checked · 1 left out"; check "a bookx-no-spell-check comment leaves the whole file out" $?
    sed -n 5p <<<"$P" | grep -q "words=Wrods/en/1 "; check "Ignore in Book removes the word" $?
    grep -q "tehe" "$d"/cfg/bookx/spelling-books/*.json; check "Ignore in Book is remembered for the book in the config folder" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "excluding files runs without errors or GTK warnings" $?'

scenario editor_live_spelldicts 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:500,editor:find-in-text=Paragraph 2 of,editor:insert=<p lang="en-GB">The colour fox</p><p lang="en-US">The colour fox</p><p lang="de">Der Hunt</p>,wait:300,editor:spell:open,wait:1500,editor:spell:dump,editor:spell:import-oxt=@D@/pack.oxt,wait:1800,editor:spell:dump,editor:spell:new-dict=Fantasy,wait:300,editor:spell:select=colour,wait:300,editor:spell:add=Fantasy,wait:1500,editor:spell:dump,editor:spell:manage,wait:600,editor:spell:new-dict=Temp,wait:500,editor:spell:dump,editor:spell:remove-dict=Temp,wait:500,editor:spell:dump,quit' '
    P=$(grep "^editor_spell_panel:" <<<"$OUT")
    sed -n 1p <<<"$P" | grep -q "colour/en-us/1"; check "text marked en-US is checked with the American dictionary (CE-271)" $?
    ! sed -n 1p <<<"$P" | grep -q "colour/en-gb"; check "the same word in en-GB text is fine with the British dictionary" $?
    sed -n 1p <<<"$P" | grep -q "no dictionary for de" && ! sed -n 1p <<<"$P" | grep -q "Hunt/de"; check "before a German dictionary exists, German words are unchecked, not wrong" $?
    grep -q "editor_spell_dict: ok de_DE" <<<"$OUT"; check "a LibreOffice extension (.oxt) imports its dictionaries (CE-279)" $?
    [ -f "$d/userdicts/de_DE.dic" ] && [ ! -e "$d/userdicts/hyph_de_DE.dic" ]; check "the German dictionary lands in the user folder, hyphenation is skipped" $?
    sed -n 2p <<<"$P" | grep -q "Hunt/de/1"; check "after the import the German text is checked with it" $?
    ! sed -n 2p <<<"$P" | grep -q "Der/de"; check "and a correct German word is no mistake" $?
    sed -n 3p <<<"$P" | grep -q "dicts=Personal|Fantasy"; check "a named user dictionary can be created and chosen (CE-275)" $?
    grep -qx "colour" "$d/cfg/bookx/spelling-dictionaries/Fantasy.txt"; check "Add to Dictionary writes into the chosen dictionary file" $?
    ! sed -n 3p <<<"$P" | grep -q "colour/en-us"; check "the word is accepted afterwards" $?
    sed -n 4p <<<"$P" | grep -q "dicts=Personal|Fantasy|Temp " && sed -n 5p <<<"$P" | grep -q "dicts=Personal|Fantasy "; check "a user dictionary can be created and deleted again" $?
    [ ! -e "$d/cfg/bookx/spelling-dictionaries/Temp.txt" ] && [ -e "$d/cfg/bookx/spelling-dictionaries/Fantasy.txt" ]; check "deleting removes only the file of that dictionary" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "the dictionary dialog runs without errors or GTK warnings" $?'

scenario editor_reports 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:500,editor:addfile=unused.png,editor:find-in-text=Paragraph 2 of,editor:insert=<p><a href="nope.xhtml">bad</a> caf&#233;</p>,editor:reports:open,wait:1500,editor:reports:dump,editor:reports:filter=chapter,wait:400,editor:reports:dump,editor:reports:filter=,wait:400,editor:reports:sort=1|asc,editor:reports:dump,editor:reports:sort=1|desc,editor:reports:dump,editor:reports:csv=@D@/files.csv,editor:reports:tab=words,wait:500,editor:reports:dump,editor:reports:tab=images,wait:500,editor:reports:dump,editor:reports:tab=rules,wait:500,editor:reports:dump,editor:reports:tab=classes,wait:500,editor:reports:dump,editor:reports:tab=chars,wait:500,editor:reports:dump,editor:reports:filter=acute,wait:400,editor:reports:dump,editor:reports:csv=@D@/chars.csv,editor:open=OEBPS/chapter-3.xhtml,editor:reports:activate=0,editor:reports:dump,editor:open=OEBPS/chapter-3.xhtml,editor:reports:tab=links,editor:reports:filter=broken,wait:400,editor:reports:dump,editor:reports:activate=0,editor:reports:dump,editor:reports:tab=images,editor:reports:delete-unused,wait:1500,editor:reports:dump,editor:dump,editor:reports:close,quit' '
    R=$(grep "^editor_reports:" <<<"$OUT")
    sed -n 1p <<<"$R" | grep -q "tab=files rows=[0-9]* shown=[0-9]* cols=5 "; check "Tools → Reports opens on the Files tab with Folder, Name, Size, Type and Words (CE-304)" $?
    sed -n 2p <<<"$R" | grep -Eq "shown=[1-9] " && ! sed -n 2p <<<"$R" | grep -q "shown=$(sed -n 1p <<<"$R" | sed "s/.*rows=\([0-9]*\).*/\1/") "; check "the filter narrows the rows" $?
    [ "$(sed -n 3p <<<"$R" | sed 's/.*first=//')" != "$(sed -n 4p <<<"$R" | sed 's/.*first=//')" ]; check "clicking a column header sorts the rows both ways" $?
    [ "$(head -1 "$d/files.csv" | tr -d "\r")" = "Folder,Name,Size (KB),Type,Words" ] && [ "$(wc -l <"$d/files.csv")" -ge 8 ]; check "CSV export writes a header and one line per row shown (CE-311)" $?
    sed -n 5p <<<"$R" | grep -q "tab=words .*cols=3 .*first=chapter/en/"; check "the Words tab lists every word with language and count (CE-305)" $?
    sed -n 6p <<<"$R" | grep -q "tab=images rows=1 .*unused.png/0.0/0/"; check "the Images tab lists images with the times each is used (CE-306)" $?
    sed -n 7p <<<"$R" | grep -q "tab=rules .*cols=4"; check "the Style rules tab shows rules with their number of matches (CE-309)" $?
    sed -n 8p <<<"$R" | grep -q "tab=classes .*cols=4"; check "the Style classes tab shows classes with use counts (CE-310)" $?
    sed -n 9p <<<"$R" | grep -q "tab=chars .*cols=4 "; check "the Characters tab lists code point, name and count (CE-308)" $?
    sed -n 10p <<<"$R" | grep -q "LATIN SMALL LETTER E WITH ACUTE"; check "characters carry their Unicode names" $?
    grep -q "^Character,Code point,Name,Count" "$d/chars.csv" && grep -q "U+00E9" "$d/chars.csv"; check "the characters export holds the rows that were filtered" $?
    sed -n 11p <<<"$R" | grep -q "current=OEBPS/chapter-1.xhtml"; check "double-clicking a character row opens the file where it occurs" $?
    sed -n 12p <<<"$R" | grep -q "tab=links .*Broken"; check "the Links tab marks broken links (CE-307)" $?
    sed -n 13p <<<"$R" | grep -q "current=OEBPS/chapter-1.xhtml"; check "activating a link row goes to the link in its file" $?
    sed -n 14p <<<"$R" | grep -q "tab=images rows=0"; check "Delete Unused removes the images nothing refers to" $?
    ! grep "^editor_files:" <<<"$OUT" | tail -1 | grep -q "unused.png"; check "and the file is gone from the book" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "the reports window runs without errors or GTK warnings" $?'

scenario editor_chars 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:500,editor:find-in-text=Paragraph 2 of,editor:chars:open,wait:500,editor:chars:dump,editor:chars:category=Currency Symbols,wait:300,editor:chars:dump,editor:chars:search=acute,wait:600,editor:chars:dump,editor:chars:insert=€,wait:300,editor:has=€,editor:chars:dump,editor:chars:open,editor:chars:insert=é|keep,wait:300,editor:chars:dump,editor:chars:fav=€,editor:chars:move=€|-99,editor:chars:dump,editor:chars:close,editor:chars:focus-find,editor:chars:open,editor:chars:insert=ß,wait:300,editor:chars:field,editor:open=OEBPS/chapter-1.xhtml,editor:insert=x00e9,editor:chars:altx,editor:has=xé,editor:insert=a&nbsp,editor:chars:semicolon,editor:chars:entities=on,editor:insert=b&eacute,editor:chars:semicolon,editor:insert=&amp,editor:chars:semicolon,editor:has=bé,editor:has=a&nbsp;,editor:has=&amp;,editor:chars:dump,quit' '
    C=$(grep "^editor_chars:" <<<"$OUT")
    sed -n 1p <<<"$C" | grep -q "open=true .*favourites=.*—"; check "the dialog opens with the starter favourites (CE-300)" $?
    sed -n 2p <<<"$C" | grep -q "title=Currency Symbols" && sed -n 2p <<<"$C" | grep -q "shown=[0-9]*:.*€"; check "a block lists its characters" $?
    sed -n 3p <<<"$C" | grep -q "title=[0-9]* results for .*acute" && sed -n 3p <<<"$C" | grep -q "shown=[0-9]*:.*é"; check "searching by name finds the characters" $?
    has "^editor_has: € = 1"; check "a click inserts the character at the caret" $?
    sed -n 4p <<<"$C" | grep -q "open=false"; check "and closes the dialog" $?
    sed -n 4p <<<"$C" | grep -q "recent=€"; check "the character joins the recently used ones" $?
    sed -n 5p <<<"$C" | grep -q "open=true"; check "Ctrl+click inserts and keeps the dialog open" $?
    sed -n 6p <<<"$C" | grep -q "favourites=€"; check "a character can be made a favourite and moved to the front" $?
    grep -q "€" "$d/cfg/bookx/special-chars.json"; check "favourites and recent characters are saved in the config folder" $?
    grep -q "editor_chars_field: .*ß" <<<"$OUT"; check "with the find field focused the character goes into the field" $?
    has "^editor_has: xé = 1"; check "Alt+X converts the typed hex code (CE-301)" $?
    grep "^editor_chars_semicolon:" <<<"$OUT" | sed -n 1p | grep -q "converted=false"; check "typing ; after &nbsp leaves it alone while the setting is off (CE-302)" $?
    grep "^editor_chars_semicolon:" <<<"$OUT" | sed -n 2p | grep -q "converted=true"; check "with the setting on, &eacute; becomes the character" $?
    grep "^editor_chars_semicolon:" <<<"$OUT" | sed -n 3p | grep -q "converted=false"; check "&amp; stays an entity, the markup needs it" $?
    has "^editor_has: bé = 1" && has "^editor_has: a&nbsp; = 1" && has "^editor_has: &amp; = 1"; check "the text has the converted and the unconverted entities" $?
    grep -q '"convert_entities": true' "$d/cfg/bookx/editor.json"; check "the setting is remembered" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "the special character dialog runs without errors or GTK warnings" $?'

scenario editor_live_a11y_spelling 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:500,editor:find-in-text=Paragraph 2 of,editor:insert=<p>Wrods</p>,wait:300,editor:spell:open,wait:1500,editor:spell:select=Wrods,wait:300,a11y-audit:,editor:spell:manage,wait:600,a11y-audit:,editor:spell:excluded-dialog,wait:400,a11y-audit:,editor:reports:open,wait:1000,a11y-audit:,editor:reports:tab=links,wait:400,a11y-audit:,editor:chars:open,wait:600,a11y-audit:,quit' '
    [ "$(grep -c "^a11y:" <<<"$OUT")" -eq 6 ]; check "the audit ran with the spelling panel, its dialogs, the reports window and the special characters window" $?
    ! has "^a11y-problem:"; check "every control of the spelling panel, reports and special characters has an accessible name" $?'

scenario editor_colors 'edit:1,wait:3000,editor:palette,editor:colors-custom,wait:500,editor:palette,editor:colors-original,wait:500,editor:palette,editor:colors-theme,wait:500,editor:palette,quit' '
    P=$(grep "^editor_palette:" <<<"$OUT")
    sed -n 2p <<<"$P" | grep -q "fbf7ee/2b2b2b"; check "the preview shows the custom colours" $?
    sed -n 3p <<<"$P" | grep -q "ffffff/"; check "Original puts the preview on a plain light page" $?
    [ "$(sed -n 4p <<<"$P")" = "$(sed -n 1p <<<"$P")" ]; check "back to Theme the preview has its theme colours again" $?
    ! has "panicked|CRITICAL"; check "the colour menu of the preview works without errors" $?'

scenario editor_rename_refs 'edit:1,wait:3000,editor:addfile=a.css,editor:addfile=data.css,editor:rename=OEBPS/chapter-1.xhtml|intro.xhtml,wait:600,editor:dump,editor:cat=OEBPS/content.opf,editor:cat=OEBPS/nav.xhtml,editor:rename=OEBPS/style.css|book.css,wait:600,editor:cat=OEBPS/chapter-2.xhtml,editor:rename=OEBPS/a.css|b.css,wait:600,editor:dump,editor:cps,quit' '
    D=$(grep "^editor:" <<<"$OUT"); F=$(grep "^editor_files:" <<<"$OUT"); C=$(grep "^editor_cat:" <<<"$OUT")
    sed -n 1p <<<"$F" | grep -q "OEBPS/intro.xhtml" && ! sed -n 1p <<<"$F" | grep -q "chapter-1.xhtml"; check "renaming a chapter moves the file" $?
    sed -n 1p <<<"$C" | grep -q "href=\"intro.xhtml\"" && ! sed -n 1p <<<"$C" | grep -q "chapter-1.xhtml"; check "the package manifest follows the rename" $?
    sed -n 2p <<<"$C" | grep -q "intro.xhtml" && ! sed -n 2p <<<"$C" | grep -q "chapter-1.xhtml"; check "the navigation document follows the rename" $?
    sed -n 3p <<<"$C" | grep -q "href=\"book.css\"" && ! sed -n 3p <<<"$C" | grep -q "style.css"; check "renaming the stylesheet repairs the link in other chapters" $?
    sed -n 2p <<<"$F" | grep -q "OEBPS/b.css" && sed -n 2p <<<"$F" | grep -q "OEBPS/data.css" && ! sed -n 2p <<<"$F" | grep -q "OEBPS/a.css"; check "renaming a.css leaves data.css alone" $?
    grep "^editor_cps:" <<<"$OUT" | grep -q "Before: Rename File/Before: Rename File/Before: Rename File"; check "every rename takes a checkpoint first" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "renaming runs without errors or GTK warnings" $?'

scenario editor_checkpoints 'edit:1,wait:3000,editor:dump,editor:rename=OEBPS/style.css|book.css,wait:500,editor:delete=OEBPS/chapter-3.xhtml,wait:500,editor:select-files=OEBPS/chapter-1.xhtml,wait:400,editor:move=down,wait:500,editor:addpaths='"$ROOT"'/extra/extra.css,wait:500,editor:title=A New Title,wait:500,editor:tocgen=headings|2|replace,wait:500,editor:cps,editor:dump,quit' '
    grep -q "^editor: .*checkpoints=0" <<<"$OUT"; check "the editor starts without checkpoints" $?
    grep "^editor_cps:" <<<"$OUT" | grep -q "^editor_cps: Before: Rename File/Before: Delete File/Before: Move File/Before: Add Files/Before: Metadata Change/Before: Generate Contents$"; check "rename, delete, move, add files, metadata change and Contents generate each take a Before checkpoint" $?
    grep "^editor:" <<<"$OUT" | tail -1 | grep -q "checkpoints=6"; check "six checkpoints exist afterwards" $?
    grep "^editor_files:" <<<"$OUT" | tail -1 | grep -q "extra.css"; check "the added file is in the book" $?
    ! grep "^editor_files:" <<<"$OUT" | tail -1 | grep -q "chapter-3"; check "the deleted file is gone" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_goto_mark 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:500,editor:goto=12,wait:200,editor:cur,editor:goto=0,wait:200,editor:dump,editor:goto=99999,wait:200,editor:cur,editor:find-in-text=Paragraph 3 of|sel,editor:mark,editor:cur,editor:cursor=0,editor:mark,editor:cur,quit' '
    K=$(grep "^editor_cur:" <<<"$OUT")
    sed -n 1p <<<"$K" | grep -q "line=12 col=1 "; check "Go to line 12 puts the cursor at the start of line 12" $?
    grep "^editor:" <<<"$OUT" | grep -q "toast=\"Enter a line number from 1 to"; check "line 0 is refused with a hint" $?
    ! sed -n 2p <<<"$K" | grep -q "line=99999"; check "a number past the end goes to the last line" $?
    sed -n 3p <<<"$K" | grep -q "sel=\"Paragraph 3 of\" marks=1"; check "Mark selected text keeps one marked region" $?
    sed -n 4p <<<"$K" | grep -q "marks=0"; check "marking without a selection clears the marked text" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "go to line and marking run without errors" $?'

scenario editor_tags 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:500,editor:find-in-text=h1>Chapter,editor:tag=select,editor:cur,editor:tag=jump,editor:cur,editor:find-in-text=Chapter 1</h1>,editor:tag=heading2,editor:has=<h2>Chapter 1</h2>,editor:has=<h1>Chapter 1</h1>,editor:tag=heading0,editor:has=<p>Chapter 1</p>,editor:tag=heading1,editor:has=<h1>Chapter 1</h1>,editor:find-in-text=Paragraph 1 of,editor:tag=rename|div,editor:has=<div>Paragraph 1 of,editor:has=</div>,editor:find-in-text=Paragraph 2 of|sel,editor:tag=wrap|span class="note",editor:has=<span class="note">Paragraph 2 of</span>,editor:find-in-text=Paragraph 2 of,editor:tag=unwrap,editor:has=<span,editor:has=Paragraph 2 of,editor:find-in-text=Paragraph 4 of|end,editor:insert=<em>x,editor:tag=close,editor:has=<em>x</em>,editor:find-in-text=Paragraph 5|sel,editor:tag=upper,editor:has=PARAGRAPH 5,editor:find-in-text=Paragraph 6 of chapter 1|sel,editor:tag=comment,editor:has=<!-- Paragraph 6 of chapter 1 -->,editor:tag=comment,editor:has=<!--,editor:cursor=0,editor:tag=unwrap,editor:dump,quit' '
    K=$(grep "^editor_cur:" <<<"$OUT"); H=$(grep "^editor_has:" <<<"$OUT")
    sed -n 1p <<<"$K" | grep -q "sel=\"<h1>Chapter 1</h1>\""; check "Select to Matching Tag selects the whole heading" $?
    sed -n 2p <<<"$K" | grep -q "line=10 col=14 sel=\"\""; check "Jump to Matching Tag goes from the opening to the closing tag" $?
    sed -n 1p <<<"$H" | grep -q "<h2>Chapter 1</h2> = 1"; check "Heading 2 turns the heading into an h2" $?
    sed -n 2p <<<"$H" | grep -q "<h1>Chapter 1</h1> = 0"; check "...and the h1 is gone" $?
    sed -n 3p <<<"$H" | grep -q "<p>Chapter 1</p> = 1"; check "Paragraph turns it into a paragraph" $?
    sed -n 4p <<<"$H" | grep -q "<h1>Chapter 1</h1> = 1"; check "Heading 1 restores it" $?
    sed -n 5p <<<"$H" | grep -q "= 1" && sed -n 6p <<<"$H" | grep -q "= 1"; check "Rename Tag changes the opening and the closing tag" $?
    sed -n 7p <<<"$H" | grep -q "= 1"; check "Wrap Selection in Tag keeps the attributes on the opening tag only" $?
    sed -n 8p <<<"$H" | grep -q "<span = 0" && sed -n 9p <<<"$H" | grep -q "= 1"; check "Remove Surrounding Tag keeps the content" $?
    sed -n 10p <<<"$H" | grep -q "<em>x</em> = 1"; check "Close the Open Tag closes the innermost open element" $?
    sed -n 11p <<<"$H" | grep -q "PARAGRAPH 5 = 1"; check "Change Case to upper case works on the selection" $?
    sed -n 12p <<<"$H" | grep -q "= 1" && sed -n 13p <<<"$H" | grep -q "<!-- = 0"; check "Toggle Comment comments the selection out and back in" $?
    grep "^editor:" <<<"$OUT" | grep -q "toast=\"The cursor is not in a matching block"; check "an action that cannot apply says so" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "the tag actions run without errors" $?'

scenario editor_files_multi 'edit:1,wait:3000,editor:dump,editor:select-files=OEBPS/chapter-1.xhtml|OEBPS/chapter-2.xhtml,wait:400,editor:merge-selected,wait:800,editor:dump,editor:newfile=interlude|chapter|link,wait:800,editor:cat=OEBPS/interlude.xhtml,editor:dump,editor:select-files=OEBPS/interlude.xhtml|OEBPS/chapter-3.xhtml,wait:400,editor:bulk=sec|1,wait:800,editor:dump,editor:cat=OEBPS/nav.xhtml,editor:addfile=extra.css,editor:select-files=OEBPS/sec1.xhtml|OEBPS/sec2.xhtml,wait:400,editor:linkcss,wait:600,editor:cat=OEBPS/sec1.xhtml,editor:export='"$ROOT"'/export-multi,wait:400,editor:delete=OEBPS/sec1.xhtml|OEBPS/sec2.xhtml,wait:800,editor:dump,editor:cps,quit' '
    D=$(grep "^editor:" <<<"$OUT"); F=$(grep "^editor_files:" <<<"$OUT"); C=$(grep "^editor_cat:" <<<"$OUT")
    n() { sed -n "$1p" <<<"$D" | sed "s/.*files=\([0-9]*\).*/\1/"; }
    [ "$(n 2)" -eq $(( $(n 1) - 1 )) ]; check "Merge Selected joins two chapters into one file" $?
    sed -n 2p <<<"$D" | grep -q "toast=\"Merged 2 files"; check "the merge is announced with the count" $?
    [ "$(n 3)" -eq $(( $(n 2) + 1 )) ]; check "New File adds one page" $?
    sed -n 1p <<<"$C" | grep -q "<h1>interlude</h1>" && sed -n 1p <<<"$C" | grep -q "style.css"; check "the new page has its heading and the linked stylesheet" $?
    sed -n 4p <<<"$F" | grep -q "OEBPS/sec1.xhtml" && sed -n 4p <<<"$F" | grep -q "OEBPS/sec2.xhtml"; check "Bulk Rename numbers the selected files" $?
    ! sed -n 2p <<<"$C" | grep -q "chapter-3.xhtml"; check "the navigation document follows the bulk rename" $?
    sed -n 3p <<<"$C" | grep -q "href=\"extra.css\""; check "Link Stylesheets adds the stylesheet to the selected pages" $?
    [ -f "$ROOT/export-multi/sec1.xhtml" ] && [ -f "$ROOT/export-multi/sec2.xhtml" ]; check "Export Selected writes the files into the chosen folder" $?
    grep "^editor_cps:" <<<"$OUT" | grep -q "Before: Merge Files/Before: New File/Before: Bulk Rename/Before: Link Stylesheets/Before: Delete Files"; check "every step took a checkpoint first" $?
    ! grep "^editor_files:" <<<"$OUT" | tail -1 | grep -q "sec1.xhtml"; check "Delete Selected removes several files at once" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "the multi-file actions run without errors" $?'

scenario editor_palette 'edit:1,wait:3000,editor:cmdpalette,wait:600,palette:type=split,wait:300,palette:dump,palette:type=save,wait:300,palette:dump,palette:type=chapter-2,wait:300,palette:dump,palette:pick,wait:700,editor:tabs,editor:cmdpalette,wait:500,palette:type=check spell,wait:300,palette:dump,palette:pick,wait:900,editor:cmdpalette,wait:400,palette:type=zzqq,wait:300,palette:dump,palette:close,quit' '
    P=$(grep "^palette: query" <<<"$OUT")
    sed -n 1p <<<"$P" | grep -q "Command|Split File at Chapter Headings"; check "the editor palette lists the Tools menu items" $?
    sed -n 2p <<<"$P" | grep -q "results=Command|Save"; check "and the editor commands (Save first)" $?
    sed -n 3p <<<"$P" | grep -q "results=File|chapter-2.xhtml"; check "and the files of the book" $?
    has "^palette: picked chapter-2.xhtml -> File"; check "choosing a file picks it" $?
    grep "^editor_tabs:" <<<"$OUT" | grep -q "chapter-2.xhtml"; check "the chosen file is open in the editor" $?
    sed -n 4p <<<"$P" | grep -q "results=Command|Check Spelling"; check "a Tools item is found with a few letters of each word" $?
    has "^palette: picked Check Spelling -> Action"; check "choosing a menu item runs its action" $?
    sed -n 5p <<<"$P" | grep -q "results=$"; check "nothing matches nonsense" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no GTK warnings" $?'
scenario editor_tabs 'edit:1,wait:3000,editor:tabs,editor:open=OEBPS/chapter-2.xhtml,wait:300,editor:step=1,wait:300,editor:tabs,editor:quickopen=nav,wait:300,editor:tabs,editor:open=OEBPS/chapter-2.xhtml,wait:300,editor:tab-close,wait:500,editor:tabs,editor:rename=OEBPS/chapter-3.xhtml|c3.xhtml,wait:600,editor:tabs,editor:quickopen-popup,wait:500,editor:quickopen=c3,wait:300,editor:tabs,quit' '
    T=$(grep "^editor_tabs:" <<<"$OUT")
    sed -n 1p <<<"$T" | grep -q "n=1 selected=OEBPS/chapter-1.xhtml"; check "the editor opens with a tab for the first chapter" $?
    sed -n 2p <<<"$T" | grep -q "n=3 selected=OEBPS/chapter-3.xhtml"; check "each file that is opened gets a tab, and Next File (Ctrl+Alt+Down) follows the reading order" $?
    grep "^editor_quickopen: nav ->" <<<"$OUT" | grep -q "OEBPS/nav.xhtml"; check "quick open finds a file by part of its name" $?
    sed -n 3p <<<"$T" | grep -q "n=4 selected=OEBPS/nav.xhtml"; check "the quick-opened file is shown in a new tab" $?
    sed -n 4p <<<"$T" | grep -q "n=3 " && ! sed -n 4p <<<"$T" | grep -q "paths=.*chapter-2.xhtml.*chapter-2"; check "closing a tab leaves the others" $?
    ! sed -n 5p <<<"$T" | grep -q "chapter-3.xhtml"; check "the tab of a renamed file does not linger under the old name" $?
    grep "^editor_quickopen: c3 ->" <<<"$OUT" | grep -q "OEBPS/c3.xhtml"; check "quick open knows the new name" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|Gtk-CRITICAL"; check "tabs and quick open run without errors or GTK warnings" $?'

scenario editor_toc 'edit:1,wait:3000,editor:toc-dump,editor:toc-go=1,wait:400,editor:cur,editor:open=OEBPS/chapter-1.xhtml,wait:300,editor:find-in-text=<p>Paragraph 15 of,editor:insert=<h2 id="mid">Middle</h2>,editor:cur,editor:open=OEBPS/chapter-3.xhtml,wait:300,editor:tocgen=headings|2|replace,wait:400,editor:toc-dump,editor:toc-go=1,wait:400,editor:cur,editor:tocgen=files|1|replace,wait:300,editor:toc-dump,editor:tocgen=headings|1|append,wait:300,editor:toc-dump,editor:tocgen=headings|1|replace,wait:300,editor:toc-dump,editor:save,wait:600,editor:cat=OEBPS/nav.xhtml,quit' '
    T=$(grep "^editor_toc:" <<<"$OUT"); K=$(grep "^editor_cur:" <<<"$OUT")
    sed -n 1p <<<"$T" | grep -q "entries=3"; check "the book starts with the three chapters in its contents" $?
    sed -n 1p <<<"$K" | grep -q "chapter-2.xhtml line=1 "; check "selecting a contents entry opens its file" $?
    sed -n 2,3p <<<"$T" | head -1 | grep -q "entries=4 dirty=true"; check "Generate from headings up to level 2 finds the extra heading" $?
    sed -n 2p <<<"$T" | grep -q "1:Middle"; check "the sub heading is a child entry" $?
    L=$(sed -n 2p <<<"$K" | sed "s/.*line=\([0-9]*\) .*/\1/")
    sed -n 3p <<<"$K" | grep -q "chapter-1.xhtml line=$L "; check "selecting an entry with a fragment scrolls to the anchor" $?
    sed -n 3p <<<"$T" | grep -q "entries=3"; check "one entry per file gives three entries" $?
    sed -n 4p <<<"$T" | grep -q "entries=3"; check "adding to the contents does not duplicate entries that exist" $?
    sed -n 5p <<<"$T" | grep -q "entries=3"; check "replacing with level 1 gives the chapters only" $?
    grep "^editor_cat:" <<<"$OUT" | tail -1 | grep -q "Chapter 2"; check "the generated contents is written on Save" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "the contents tab runs without errors" $?'

scenario editor_split_merge 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:500,editor:dump,editor:addheading,editor:split,wait:800,editor:dump,editor:merge,wait:800,editor:dump,quit' '
    D=$(grep "^editor:" <<<"$OUT")
    n() { sed -n "$1p" <<<"$D" | sed "s/.*files=\([0-9]*\).*/\1/"; }
    [ "$(n 2)" -eq $(( $(n 1) + 1 )) ]; check "Split File at Chapter Headings adds a file" $?
    sed -n 2p <<<"$D" | grep -q "toast=\"Split into"; check "the split is announced" $?
    [ "$(n 3)" = "$(n 1)" ]; check "Merge with Next File puts it back" $?
    sed -n 3p <<<"$D" | grep -q "toast=\"Merged"; check "the merge is announced" $?
    ! has "panicked|CRITICAL"; check "split and merge run without errors" $?'

scenario email_book 'library:mailcfg,library:select1,wait:500,library:emailsel,wait:2500,quit' '
    MAIL=$(cat "$d/mail.txt" 2>/dev/null)
    grep -q "RCPT TO:<friend@example.org>" <<<"$MAIL"; check "the book is mailed to the address from the settings" $?
    grep -q "MAIL FROM:<me@example.com>" <<<"$MAIL"; check "the sender is the address from the settings" $?
    grep -q "Content-Disposition: attachment" <<<"$MAIL"; check "the book is attached" $?
    grep -q "\.epub" <<<"$MAIL"; check "the attachment keeps its .epub name" $?
    ! has "panicked|CRITICAL"; check "sending by e-mail works without errors" $?'

scenario note_tooltip 'read:1,wait:3500,reader:select,wait:500,reader:note,wait:1000,reader:ui-tooltip,quit' '
    has "^tooltip: Some\(\"Great opening"; check "hovering a highlighted passage with a note yields the note as tooltip" $?'

scenario note_hover_and_touch 'read:1,wait:3500,reader:select,wait:500,reader:note,wait:1000,reader:ui-hover,reader:dump,wait:300,reader:ui-hl-popover,wait:800,reader:dump,quit' '
    grep -q "^hover_note: Some(\"Great opening" <<<"$OUT"; check "the note appears instantly while hovering its highlight (no tooltip delay)" $?
    grep -q "^popover_note: true \"Great opening" <<<"$OUT"; check "the quick-options popover shows the note (touch screens have no hover)" $?'

scenario popover_note_from_highlight 'read:1,wait:3500,reader:demo-highlights,wait:800,reader:ui-note-popover,wait:800,reader:dump,quit' '
    grep -q "^popover_note: true \"Where the story really starts." <<<"$OUT"; check "opening a highlighted passage shows its note in the popover (the wiki screenshot hook)" $?'

scenario popover_without_note 'read:1,wait:3500,reader:select,wait:500,reader:hl,wait:800,reader:ui-hl-popover,wait:800,reader:dump,quit' '
    grep -q "^popover_note: false" <<<"$OUT"; check "a highlight without a note shows just the buttons" $?'

scenario popover_dismiss 'read:1,wait:3500,reader:select,wait:500,reader:hl,wait:800,reader:ui-hl-popover,wait:800,reader:dump,reader:ui-click-away,wait:800,reader:dump,quit' '
    P=$(grep "^popover:" <<<"$OUT")
    [ "$(sed -n 1p <<<"$P")" = "popover: true" ]; check "clicking a highlight opens its popover" $?
    [ "$(sed -n 2p <<<"$P")" = "popover: false" ]; check "clicking anywhere in the reading view dismisses it" $?'

scenario highlights 'read:1,wait:4000,reader:select,wait:1000,reader:hl,wait:1500,reader:dump,quit' '
    has "bookmarks_notes: [1-9]"; check "a highlight is stored" $?'

scenario status_line 'read:1,wait:2500,reader:dump,quit' '
    has "labels: [0-9]+ of [0-9]+  ·  .*minute"; check "the footer reads N of M with chapter and time, in one centred line" $?'

scenario search_persists 'read:1,wait:2500,reader:search,wait:4000,reader:search-close,wait:500,reader:dump,quit' '
    has "search_status: [0-9]+ results"; check "search results are listed" $?
    has "^search_query: the"; check "query survives closing the dropdown" $?'

scenario chrome_fades 'read:1,wait:9000,reader:dump,quit' '
    has "^idle: true"; check "bars fade after a few idle seconds" $?'

scenario chrome_stays_with_dropdown 'read:1,wait:2500,wait:5000,reader:search,wait:6000,reader:dump,quit' '
    has "^idle: false"; check "bars stay while the search dropdown is open" $?'

scenario chrome_stays_with_read_aloud 'read:1,wait:2500,reader:tts-open,wait:700,reader:chrome,wait:9000,reader:chrome,reader:tts-play,wait:5000,reader:chrome,reader:tts-close,wait:600,reader:chrome,wait:9000,reader:chrome,quit' '
    C=$(grep "^chrome:" <<<"$OUT")
    sed -n 1p <<<"$C" | grep -q "idle=false busy=true tts_popover=true"; check "opening the read-aloud popover keeps the bars (busy)" $?
    sed -n 2p <<<"$C" | grep -q "idle=false busy=true"; check "the bars are still there after longer than the idle timeout with the popover open" $?
    sed -n 3p <<<"$C" | grep -q "idle=false busy=true"; check "and while it is reading aloud with the popover open" $?
    sed -n 4p <<<"$C" | grep -q "idle=false busy=false tts_popover=false"; check "closing the popover ends the interaction" $?
    sed -n 5p <<<"$C" | grep -q "idle=true"; check "the idle countdown restarts after the interaction and the bars fade" $?'

# -- library health: backups, check, Calibre import, layout, lock, trash, undo, import folder ------------

scenario health_backup_restore 'library:health=open,wait:800,library:health=backup,wait:1500,library:import='"$ROOT"'/fb2,wait:3000,library:health=restore1,wait:1500,library:health=dump,quit' '
    has "^health: backup ok books=2"; check "Back up now writes a snapshot of the 2 books" $?
    has "^health: restored books=2"; check "restoring brings back the catalogue of the backup (the FB2 import is gone)" $?
    ! grep -q "Solaris Imported" <<<"$LIB"; check "the restored catalogue on disk does not list the later import" $?
    grep -q "UI Test Book" <<<"$LIB"; check "the books of the backup are there" $?
    [ "$(ls "$d/lib/.bookx-backups" | grep -c prerestore)" -ge 1 ]; check "the catalogue before the restore was snapshotted too" $?
    has "^health_snapshots: [3-9]"; check "the window lists the snapshots (startup, manual, before-restore)" $?'

scenario backup_destinations 'library:health=open,wait:800,library:health=backup-add=USB drive|@D@/usb,library:health=backup-add=Offsite|@D@/offsite|encrypt,library:health=backup-add=Gone|@D@/nope/deeper,library:health=backup-passphrase=secret-phrase-77,library:health=backup,wait:6000,library:health=dump,quit' '
    has "^health: backup ok books=2"; check "Back up now still writes the local snapshot" $?
    has "^backup_place: USB drive \| OK \| enabled=true \| encrypt=false"; check "the plain folder destination is OK" $?
    has "^backup_place: Offsite \| OK \| enabled=true \| encrypt=true"; check "the encrypted destination is OK" $?
    has "^backup_place: Gone \| Unreachable"; check "a destination that is not there is reported unreachable, the others still got their copy" $?
    has "^backup: copied .*encrypted=true verified=true"; check "the encrypted copy was verified after writing" $?
    ls "$d/usb" | grep -q "\.db$"; check "the plain copy is a .db file on the drive" $?
    ls "$d/offsite" | grep -q "\.bookxbackup$"; check "the encrypted copy is a .bookxbackup file" $?
    ! grep -aq "UI Test Book" "$d"/offsite/*.bookxbackup; check "the encrypted file does not contain the catalogue in clear" $?
    head -c 15 "$d"/usb/*.db | grep -aq "SQLite format 3"; check "the plain copy is a SQLite database" $?
    ! grep -rq "secret-phrase-77" "$d/cfg"; check "the backup passphrase is not in the settings file" $?
    has "^backup_pass_set: true"; check "but the app still has the passphrase" $?'

scenario backup_restore_from_destination 'library:health=open,wait:800,library:health=backup-add=USB drive|@D@/usb,library:health=backup,wait:3000,library:import='"$ROOT"'/fb2,wait:3000,library:health=backup-reload,wait:2500,library:health=dump,library:health=backup-restore-remote1,wait:3000,library:health=dump,quit' '
    has "^backup_remote_buttons: [1-9]"; check "the backups at the destination are listed with Restore buttons" $?
    has "^health: restored books=2"; check "restoring from the destination brings back the 2-book catalogue" $?
    ! grep -q "Solaris Imported" <<<"$LIB"; check "the later import is gone after restoring from the drive" $?
    [ "$(ls "$d/lib/.bookx-backups" | grep -c prerestore)" -ge 1 ]; check "the catalogue before the restore was snapshotted" $?
    [ ! -d "$d/lib/.bookx-backups/restore-tmp" ]; check "the downloaded copy was cleaned up" $?'

scenario backup_restore_encrypted 'library:health=open,wait:800,library:health=backup-passphrase=an-offsite-secret,library:health=backup-add=Offsite|@D@/offsite|encrypt,library:health=backup,wait:6000,library:import='"$ROOT"'/fb2,wait:3000,library:health=backup-reload,wait:2500,library:health=backup-restore-remote1,wait:5000,library:health=dump,quit' '
    ls "$d/offsite" | grep -q "\.bookxbackup$"; check "the off-site copy is encrypted" $?
    has "^health: restored books=2"; check "restore decrypts the copy with the saved passphrase" $?
    ! grep -q "Solaris Imported" <<<"$LIB"; check "the catalogue is the one from the encrypted backup" $?'

scenario backup_remove 'library:health=open,wait:800,library:health=backup-add=A|@D@/a,library:health=backup-add=B|@D@/b,library:health=backup-remove1,wait:500,library:health=dump,quit' '
    has "^backup_places: 1"; check "removing a destination leaves the other one" $?
    has "^backup_place: B "; check "the right destination was removed" $?'

scenario secrets_migrate 'wait:500,kosync:dump,wait:500,quit' '
    has "^kosync_state: enabled=true ready=true"; check "the migrated password still works (it came out of the keyring)" $?
    ! grep -rq "legacy-pw-123" "$d/cfg"; check "the plain text password was removed from the settings file" $?'

scenario secrets_in_keyring 'kosync:setup=http://127.0.0.1:1;bob;hunter2-pw,wait:500,library:health=open,wait:500,library:health=backup-add-dav=Cloud|https://c.example/dav|bob|dav-pw-555|Bookx backups,wait:500,quit' '
    ! grep -rq "hunter2-pw" "$d/cfg"; check "the KOSync password is not in the settings file" $?
    ! grep -rq "dav-pw-555" "$d/cfg"; check "the WebDAV backup password is not in the settings file" $?
    grep -rq "Cloud" "$d/cfg"; check "the rest of the destination is saved" $?'

scenario secrets_no_keyring 'kosync:setup=http://127.0.0.1:1;bob;hunter2-pw,wait:500,quit' '
    grep -rq "hunter2-pw" "$d/cfg"; check "without a keyring the password stays in the settings file (nothing is lost)" $?'

scenario backup_webdav 'library:health=open,wait:800,library:health=backup-add-dav=Nextcloud|http://127.0.0.1:@PORT@/files/bob/|bob|s3cret|Bookx backups|encrypt,library:health=backup-passphrase=dav-phrase-42,library:health=backup,wait:7000,library:import='"$ROOT"'/fb2,wait:3000,library:health=backup-reload,wait:3000,library:health=backup-restore-remote1,wait:6000,library:health=dump,quit' '
    has "^backup_place: Nextcloud \| OK"; check "the WebDAV destination is OK" $?
    ls "$d/dav-root/files/bob/Bookx backups" | grep -q "\.bookxbackup$"; check "the encrypted copy arrived on the WebDAV server in the chosen folder" $?
    [ -n "$(ls "$d/dav-root/files/bob/Bookx backups")" ] && ! grep -aq "UI Test Book" "$d"/dav-root/files/bob/Bookx\ backups/*; check "the server only sees ciphertext" $?
    has "^health: restored books=2"; check "restoring from the WebDAV server downloads, decrypts and restores" $?
    ! grep -q "Solaris Imported" <<<"$LIB"; check "the catalogue is the one from the server" $?
    ! grep -rq "s3cret\|dav-phrase-42" "$d/cfg"; check "neither the WebDAV password nor the passphrase is in the settings file" $?'

scenario backup_webdav_wrong_password 'library:health=open,wait:800,library:health=backup-add-dav=Nextcloud|http://127.0.0.1:@PORT@/files/bob/|bob|wrong|Bookx backups,library:health=backup,wait:4000,library:health=dump,quit' '
    has "^backup_place: Nextcloud \| Failed.*refused the user name or password"; check "a wrong password is reported as such, not as a vague failure" $?
    [ -z "$(ls "$d/dav-root/files/bob" 2>/dev/null)" ]; check "nothing was written to the server" $?'

scenario backup_on_exit 'library:health=open,wait:800,library:health=backup-add=USB drive|@D@/usb,wait:300,quit' '
    ls "$d/usb" | grep -q "\.db$"; check "closing Bookx copies the newest snapshot to the destinations" $?
    has "^backup: exit copy to USB drive ok"; check "the exit copy is reported" $?'

scenario health_check 'library:health=open,wait:800,library:health=check,wait:1500,library:health=dump,library:health=fix1,wait:1000,library:health=check,wait:1500,library:health=dump,quit' '
    has "^health: check books=[0-9]+ findings=1"; check "the check finds the book whose file is missing" $?
    has "^health_summary: .*1 problem"; check "the result line says one problem" $?
    has "^health_trash: 1"; check "fixing the entry puts it in the trash instead of erasing it" $?
    has "^health: check books=[0-9]+ findings=0"; check "a second check is clean" $?
    ! grep -q "\"title\": \"UI Test Book\"" <<<"$LIB"; check "the broken entry is gone from the catalogue" $?'

scenario health_calibre 'library:health=open,wait:800,library:health=calibre=@D@/calibre,wait:3500,library:health=dump,quit' '
    has "^health: calibre imported=1 skipped=1"; check "the Calibre import reports one imported and one skipped (no EPUB)" $?
    has "^health_calibre: 1 book\(s\) imported.*Only A PDF"; check "the report names the skipped book" $?
    grep -q "Calibre Dune" <<<"$LIB"; check "the Calibre book is in the catalogue" $?
    ! grep -Eq "\"file\": *\"/" <<<"$LIB"; check "its file was copied into the library (no absolute paths)" $?
    [ -f "$d/calibre/metadata.db" ] && [ -f "$d/calibre/Frank Herbert/Calibre Dune (1)/Calibre Dune - Frank Herbert.epub" ]; check "the Calibre library is untouched" $?'

scenario health_calibre_link 'library:health=open,wait:800,library:health=calibre=@D@/calibre|nocopy,wait:3500,library:health=dump,quit' '
    has "^health: calibre imported=1"; check "the import runs without copying" $?
    grep -Eq "\"file\": *\"/.*calibre/Frank Herbert/Calibre Dune" <<<"$LIB"; check "the book stays linked to the Calibre folder" $?'

scenario health_layout 'library:health=layout=flat,wait:2000,quit' '
    has "^health: layout One flat folder moved=2"; check "changing to the flat layout moves both books" $?
    [ -f "$d/lib/Test Author - UI Test Book.epub" ] && [ ! -d "$d/lib/Test Author" ]; check "the files are in the library folder and the author folders are gone" $?
    grep -Eq "\"file\": *\"Other Author - Second Book.epub\"" <<<"$LIB"; check "the catalogue follows the move" $?
    grep -q Flat "$d/lib/.bookx/layout.json"; check "the layout is remembered in the library" $?'

scenario health_layout_template 'library:health=layout=template|{author}/{series}/{title},wait:2000,library:health=layout=template|{author}/{title},wait:2000,quit' '
    has "^health: layout Custom template moved=2"; check "a template layout moves the books" $?
    [ -f "$d/lib/Test Author/UI Test Book.epub" ]; check "the file follows the template" $?
    ! has "layout failed"; check "no failure" $?'

scenario health_layout_linked 'library:import='"$ROOT"'/css-epub/lib,wait:3000,quit' '
    grep -Eq "\"file\": *\"/.*css-epub/lib/styled.epub\"" <<<"$LIB"; check "with the linked layout an EPUB is listed where it is, not copied" $?
    [ ! -e "$d/lib/Css Author" ]; check "no copy was made in the library folder" $?
    [ -f "$ROOT/css-epub/lib/styled.epub" ]; check "the original is untouched" $?'

scenario health_readonly 'library:health=open,wait:800,library:health=backup,library:health=dump,quit' '
    has "^health_read_only: true"; check "a library held by another running Bookx opens read-only" $?
    has "^health_backup_sensitive: false"; check "the backup button is switched off" $?
    has "^health: button is disabled"; check "pressing it does nothing" $?
    ! grep -q "UI Test Book" <<<"$LIB"; check "an import is refused, the catalogue stays untouched" $?
    grep -q "\"pid\":$(cat "$d/holder.pid")" "$d/lib/.bookx-lock"; check "the other process keeps its lock" $?
    kill "$(cat "$d/holder.pid")" 2>/dev/null; true'

scenario health_stale_lock 'library:health=open,wait:800,library:health=dump,quit' '
    has "^health_read_only: false"; check "the lock of a dead process is taken over" $?
    [ ! -e "$d/lib/.bookx-lock" ]; check "the lock file is removed when Bookx exits" $?
    grep -q "UI Test Book" <<<"$LIB"; check "and the library is writable" $?'

scenario health_sync_warning 'library:health=open,wait:800,library:health=dump,quit' '
    has "^health_warnings: 1"; check "a library inside a synced folder is reported" $?
    grep -rq "$d/lib" "$d/cfg"; check "the note is remembered so it is shown only once" $?'

scenario health_trash 'library:health=open,wait:800,library:health=dump,library:health=trash-restore1,wait:1000,library:health=dump,quit' '
    has "^health_trash: 1"; check "the deleted book waits in the trash" $?
    has "^health: trash restored #1"; check "restoring puts the book back" $?
    has "^health_trash: 0"; check "the trash is empty afterwards" $?
    grep -q "Solaris Imported" <<<"$LIB"; check "the restored book is in the catalogue" $?'

scenario health_undo_empty 'library:health=open,wait:800,library:health=undo,wait:1200,library:health=dump,library:health=trash-empty,wait:1000,library:health=dump,quit' '
    has "^health: undone Imported 2 books"; check "undo takes back the whole import" $?
    has "^health_trash: 2"; check "the undone books are in the trash, not erased" $?
    has "^health: trash emptied 2"; check "emptying the trash erases them" $?
    ! grep -q "UI Test Book" <<<"$LIB"; check "the catalogue is empty" $?
    [ ! -d "$d/lib/.bookx-trash" ]; check "the trash folder is gone from disk" $?'

scenario health_inbox 'library:health=open,wait:1500,library:health=dump,library:health=inbox-add-all,wait:2500,library:health=dump,quit' '
    has "^health: inbox waiting=2"; check "both new files of the watched folder are queued" $?
    has "^health: inbox added=1 duplicates=1 failed=0"; check "Add All adds the new book and leaves the duplicate out" $?
    grep -q "Fresh Arrival" <<<"$LIB"; check "the new book is in the library" $?
    [ "$(grep -c "\"title\": \"Second Book\"" <<<"$LIB")" -eq 1 ]; check "the duplicate was not added twice" $?
    [ "$(grep "^health: inbox waiting=" <<<"$OUT" | tail -1)" = "health: inbox waiting=0" ]; check "nothing is left waiting" $?'

scenario health_tidy 'library:health=open,wait:800,library:health=tidy,wait:1500,quit' '
    has "^health: tidy=1"; check "one author spelling is proposed for tidying" $?
    has "^health: tidied 1"; check "applying tidies it" $?
    "$CLI" --library "$d/lib" library tidy | grep -q "^0 change"; check "afterwards nothing is left to tidy" $?'

scenario health_provenance 'library:select1,wait:800,library:editmeta,wait:2500,library:fetchmeta,wait:5000,library:metasave,wait:3000,quit' '
    has "^metasave: true"; check "Save is pressed in the edit dialog" $?
    I=$("$CLI" --library "$d/lib" library info 2)
    grep -q "^Sources: .*openlibrary: .*publisher" <<<"$I"; check "the fields the lookup filled are recorded with their source" $?
    grep -q "^Publisher: *Mock Press" <<<"$I"; check "and the data itself was saved" $?'

scenario health_auto_snapshots 'quit' '
    ls "$d/lib/.bookx-backups" | grep -q -- "-n0-auto.db"; check "a snapshot is taken at start-up" $?
    ls "$d/lib/.bookx-backups" | grep -q -- "-n2-auto.db"; check "and another on exit, with the imported books" $?'

scenario health_auto_off 'library:health=open,wait:600,library:health=auto-off,wait:300,quit' '
    [ "$(ls "$d/lib/.bookx-backups" | grep -c -- "-auto.db")" -le 1 ]; check "with automatic backups off nothing is taken on exit" $?
    ! ls "$d/lib/.bookx-backups" | grep -q -- "-n2-auto.db"; check "no snapshot of the imported books" $?'

scenario editor_view 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:500,editor:view,editor:open=OEBPS/style.css,wait:400,editor:view,editor:scheme=dark,editor:scheme=light,editor:scheme=eink,editor:open=OEBPS/content.opf,wait:300,editor:view,editor:view-opt=whitespace:on,wait:200,editor:view,editor:view-opt=minimap:off,wait:200,editor:view,editor:view-opt=minimap:on,editor:view-opt=whitespace:off,wait:200,editor:view,quit' '
    V=$(grep "^editor_view:" <<<"$OUT")
    sed -n 1p <<<"$V" | grep -q "lang=html "; check "a chapter is highlighted by GtkSourceView as HTML" $?
    sed -n 2p <<<"$V" | grep -q "file=OEBPS/style.css lang=css "; check "a stylesheet is highlighted as CSS" $?
    sed -n 3p <<<"$V" | grep -q "file=OEBPS/content.opf lang=xml "; check "the package document is highlighted as XML" $?
    sed -n 1p <<<"$V" | grep -q "fallback_tags=false"; check "the hand-written highlighter stays out of the way when a language exists" $?
    sed -n 1p <<<"$V" | grep -q "line_numbers=true current_line=true brackets=true auto_indent=true indent_tab=true smart_home=true margin=false"; check "line numbers, current-line highlight, bracket matching, auto-indent, indent on Tab and smart Home/End are on and the right margin guide is off" $?
    sed -n 1p <<<"$V" | grep -q "scheme=Adwaita "; check "the light Adwaita style scheme is used" $?
    grep "^editor_scheme: dark" <<<"$OUT" | grep -q "Adwaita-dark"; check "a dark app gets the dark Adwaita scheme" $?
    grep "^editor_scheme: light" <<<"$OUT" | grep -q -- "-> Adwaita$"; check "a light app gets the light Adwaita scheme" $?
    grep "^editor_scheme: eink" <<<"$OUT" | grep -q -- "-> classic$"; check "e-ink mode gets the high-contrast classic scheme" $?
    sed -n 1p <<<"$V" | grep -q "whitespace=false minimap=true"; check "hidden characters are off and the minimap is on by default" $?
    sed -n 4p <<<"$V" | grep -q "whitespace=true minimap=true"; check "Show Hidden Characters draws spaces and tabs" $?
    sed -n 5p <<<"$V" | grep -q "whitespace=true minimap=false"; check "the minimap can be hidden" $?
    sed -n 6p <<<"$V" | grep -q "whitespace=false minimap=true"; check "both options can be switched back" $?
    grep -q "\"minimap\": true" "$d/cfg/bookx/editor.json" && grep -q "\"show_whitespace\": false" "$d/cfg/bookx/editor.json"; check "the two view options are remembered in the config folder" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "the code view opens and restyles without errors or GTK warnings" $?'

scenario editor_search_context 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:500,editor:find-set=chapter,wait:900,editor:find-status,editor:find-opt=case,wait:700,editor:find-status,editor:find-set=Paragraph [0-9]+ of,editor:find-opt=regex,wait:900,editor:find-status,editor:find-set=(,wait:700,editor:find-status,editor:find-set=Paragraph (?<n>[0-9]+) of,wait:700,editor:find-status,editor:open=OEBPS/chapter-2.xhtml,wait:700,editor:find-status,quit' '
    F=$(grep "^editor_find:" <<<"$OUT")
    num() { sed -n "$1p" <<<"$F" | sed "s/.*count=\(-\?[0-9]*\).*/\1/"; }
    sed -n 1p <<<"$F" | grep -q "highlight=true"; check "all matches of the open file are highlighted in the text" $?
    [ "$(num 1)" -ge 26 ]; check "the occurrences count of the open file comes from the search context (case-insensitive: heading and 25 paragraphs)" $?
    [ "$(num 2)" -eq 25 ]; check "Match case narrows the count" $?
    sed -n 2p <<<"$F" | grep -q "in this file"; check "the find bar says how many matches the file has" $?
    [ "$(num 3)" -eq 25 ]; check "a regular expression is counted by the search context" $?
    sed -n 4p <<<"$F" | grep -q "Invalid regular expression"; check "an invalid expression is reported in the find bar" $?
    sed -n 5p <<<"$F" | grep -q "error=false"; check "look-around and named groups are handled by the engine (the context may differ)" $?
    sed -n 6p <<<"$F" | grep -q "highlight=true"; check "after switching files the new file is the one highlighted" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "live search runs without errors or GTK warnings" $?'

# ---- search & replace: modes, scopes, history, count, wrap (CE-061..CE-081) -----------------------------
scenario editor_search_bar 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:500,editor:search:status,editor:search:find=functionality,editor:search:scope=text,editor:search:count,editor:search:status,editor:search:find=function,editor:search:opt=word,editor:search:count,editor:search:status,editor:search:opt=no-word,editor:search:count,editor:search:status,editor:search:mode=function,editor:search:status,editor:search:mode=regex,editor:search:status,editor:search:scope=open,editor:search:find=Chapter,editor:search:count,editor:search:status,editor:search:mode=text,editor:search:scope=this,editor:search:find=Paragraph 25 of chapter 1,editor:search:next,editor:search:next,editor:search:status,editor:search:find=Paragraph 3 of chapter 1,editor:search:replace=ThreeX,editor:search:next,editor:search:sel,editor:search:replace-find,editor:search:has=OEBPS/chapter-1.xhtml|ThreeX,editor:search:has=OEBPS/chapter-1.xhtml|Paragraph 3 of chapter 1,editor:search:history,editor:search:history-use=1,editor:search:status,editor:search:special=0,editor:search:status,quit' '
    S=$(grep "^editor_search:" <<<"$OUT")
    st() { sed -n "$1p" <<<"$S"; }
    st 1 | grep -q "mode=Text" && st 1 | grep -q "fn_visible=false" && st 1 | grep -q "replace_visible=true" && st 1 | grep -q "dotall_visible=false" && st 1 | grep -q "markup_visible=true"; check "in Text mode the function list and the dot-all button are hidden" $?
    st 2 | grep -q "status=\"75 matches in 3 files\""; check "Count reports the total over the scope and the number of files (CE-065)" $?
    st 3 | grep -q "status=\"0 matches in 0 files\""; check "Whole word does not find “function” inside “functionality” (CE-069)" $?
    st 4 | grep -q "status=\"75 matches in 3 files\""; check "without Whole word the substring is found everywhere" $?
    st 5 | grep -q "mode=Function" && st 5 | grep -q "fn_visible=true" && st 5 | grep -q "replace_visible=false"; check "Function mode shows the function list and hides the replacement box" $?
    st 6 | grep -q "mode=Regex" && st 6 | grep -q "dotall_visible=true" && st 6 | grep -q "fn_visible=false"; check "Regex mode shows the dot-matches-all button (CE-067)" $?
    st 7 | grep -q "status=\"27 matches in 1 file\""; check "the scope Open files covers the files that have a tab (CE-076)" $?
    st 8 | grep -q "wrapped around"; check "the second Find Next wraps around the file and says so (CE-061)" $?
    grep -q "^editor_search_sel: \"Paragraph 3 of chapter 1\"" <<<"$OUT"; check "Find Next selects the match" $?
    grep -q "^editor_search_has: OEBPS/chapter-1.xhtml \"ThreeX\" = 1" <<<"$OUT"; check "Replace and find replaces the selected match (CE-063)" $?
    grep -q "^editor_search_has: OEBPS/chapter-1.xhtml \"Paragraph 3 of chapter 1\" = 0" <<<"$OUT"; check "the match is gone" $?
    grep -q "^editor_history: find=\[\"Paragraph 3 of chapter 1\", \"Paragraph 25 of chapter 1\"" <<<"$OUT"; check "the Find box remembers the last searches, newest first (CE-079)" $?
    st 9 | grep -q "find=\"Paragraph 25 of chapter 1\""; check "choosing an entry from the history fills the Find box" $?
    st 10 | grep -q "find=\"\\\\u{a0}Paragraph 25"; check "a special character can be inserted into the Find box (CE-081)" $?
    f=$(find "$d/cfg" -name search-history.json | head -1); [ -n "$f" ] && grep -q "ThreeX" "$f"; check "the history is saved in the config folder" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "the search bar runs without errors or GTK warnings" $?'

scenario editor_search_markup_tag 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:500,editor:find-in-text=</body>,editor:insert=<p>so on <i>a</i> word here</p>\n<img src="x.png" alt=""/>\n<img src="y.png" alt="Y"/>\n<a href="http://example.org/a">link</a>\n,wait:300,editor:search:find=on a word,editor:search:count,editor:search:status,editor:search:opt=markup,editor:search:count,editor:search:status,editor:search:replace=X,editor:search:replace-all,editor:search:has=OEBPS/chapter-1.xhtml|so X<i></i> here,editor:search:opt=no-markup,editor:search:mode=tag,editor:search:find=img[alt=""],wait:400,editor:search:status,editor:search:replace=alt=Picture,editor:search:replace-all,editor:search:has=OEBPS/chapter-1.xhtml|Picture,editor:search:find=a[href^=http://],editor:search:replace==https://,editor:search:replace-all,editor:search:has=OEBPS/chapter-1.xhtml|https://example.org,editor:search:find=img[,wait:400,editor:search:status,editor:search:find=img[!alt],editor:search:count,editor:search:status,quit' '
    S=$(grep "^editor_search:" <<<"$OUT")
    st() { sed -n "$1p" <<<"$S"; }
    st 1 | grep -q "status=\"0 matches in 0 files\""; check "a phrase split by <i> is not found by a plain search" $?
    st 2 | grep -q "status=\"1 match in 1 file\""; check "with Ignore HTML markup the phrase across the tag is found (CE-083)" $?
    grep -q "^editor_search_has: OEBPS/chapter-1.xhtml \"so X<i></i> here\" = 1" <<<"$OUT"; check "replacing keeps the tags inside the match and writes the text where the match starts" $?
    st 3 | grep -q "hits=1"; check "tag search highlights the matching tags (CE-085)" $?
    grep -q "^editor_search_has: OEBPS/chapter-1.xhtml .Picture. = 1" <<<"$OUT"; check "tag replace sets an attribute on the matching tags only" $?
    grep -q "^editor_search_has: OEBPS/chapter-1.xhtml .https://example.org. = 1" <<<"$OUT"; check "tag replace =text rewrites the matched part of an attribute value" $?
    st 4 | grep -q "Tag query"; check "a broken tag query is explained in the bar" $?
    st 5 | grep -q "status=\"0 matches in 0 files\""; check "img[!alt] finds no image once all have alt text" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "markup and tag search run without errors" $?'

scenario editor_saved_searches 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:500,editor:search:find=functionality,editor:search:replace=utility,editor:search:scope=text,editor:search:save=Utility,editor:search:find=Paragraph,editor:search:replace=Para,editor:search:save=Para,editor:search:saved,editor:search:saved-export=@D@/export.json,editor:search:saved-run=0|count,wait:500,editor:search:fn-report,editor:search:saved-run=0 1|count,wait:500,editor:search:fn-report,editor:search:saved-run=0|replace,wait:800,editor:search:fn-report,editor:search:has=OEBPS/chapter-2.xhtml|utility,editor:search:has=OEBPS/chapter-2.xhtml|functionality,editor:dump,editor:search:saved-run=1|replace,wait:800,editor:search:has=OEBPS/chapter-3.xhtml|Para 25 of,editor:search:saved-move=0|1,editor:search:saved,editor:search:saved-delete=0,editor:search:saved,editor:search:saved-import=@D@/export.json,editor:search:saved,editor:search:saved-load=2,editor:search:status,editor:search:saved-open,wait:600,a11y-audit:,quit' '
    L=$(grep "^editor_saved:" <<<"$OUT")
    grep -q "^editor_saved: 0 | Utility | Text · functionality → utility · Text files" <<<"$OUT"; check "a search is saved with its name, mode, texts and scope (CE-084)" $?
    grep -q "^editor_saved: 1 | Para | " <<<"$OUT"; check "a second search is saved after the first" $?
    grep -q "^editor_saved: count=2" <<<"$OUT"; check "two searches are in the list" $?
    ! grep -q "functionality" "$d/export.json" || grep -q "\"name\": \"Utility\"" "$d/export.json"; check "export writes JSON with the searches" $?
    f=$(find "$d/cfg" -name saved-searches.json | head -1); [ -n "$f" ] && grep -q "\"name\": \"Utility\"" "$f" && grep -q "\"scope\": \"text_files\"" "$f"; check "saved searches are stored in the config folder" $?
    grep "^editor_fn_report:" <<<"$OUT" | sed -n 1p | grep -q "Utility: 75 matches"; check "running a saved search with Count only counts" $?
    grep "^editor_fn_report:" <<<"$OUT" | sed -n 2p | grep -q "Utility: 75 matches.*Para: 75 matches"; check "several searches run in list order and are all reported" $?
    grep "^editor_fn_report:" <<<"$OUT" | sed -n 3p | grep -q "Utility: 75 replaced in 3 file"; check "Replace All from the list replaces in the scope of the saved search" $?
    grep -q "^editor_search_has: OEBPS/chapter-2.xhtml \"utility\" = 25" <<<"$OUT"; check "the other chapters were changed too" $?
    grep -q "^editor_search_has: OEBPS/chapter-2.xhtml \"functionality\" = 0" <<<"$OUT"; check "nothing is left of the old word" $?
    grep -q "checkpoints=1" <<<"$OUT"; check "replacing from the list takes a checkpoint first" $?
    grep -q "^editor_search_has: OEBPS/chapter-3.xhtml \"Para 25 of\" = 1" <<<"$OUT"; check "the second search replaced too" $?
    [ "$(grep "^editor_saved:" <<<"$OUT" | grep -c "| Para |")" -ge 2 ]; check "reordering, deleting and importing keep the list consistent" $?
    grep -q "^editor_saved: count=3" <<<"$OUT"; check "importing adds the searches again (renamed when the name is taken)" $?
    grep "^editor_search:" <<<"$OUT" | tail -1 | grep -q "find=\"Paragraph\""; check "a saved search can be loaded into the find bar" $?
    ! has "^a11y-problem:"; check "the Saved Searches dialog has accessible names" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "saved searches run without errors" $?'

scenario editor_function_mode 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:500,editor:search:fn-add=Num|Paragraph (\d+) of|// @call_after_last_match\n// @file_order spine\nfn replace(m§ number§ file§ meta§ data) {\n if m == () { print("total " + data["n"]); return "final " + data["n"] + " " + meta.title; }\n data["n"] = number;\n print(file);\n "N" + number + "-" + m.group(1) + " of"\n}\n,editor:search:fn-add=Quiet|Chapter|// @suppress_result_dialog\nfn replace(m§ number§ file§ meta§ data) { print("hush"); m.text }\n,editor:search:fn-add=Broken|Chapter|fn replace(m§ number§ file§ meta§ data) {\n let a = 1;\n nope(a)\n}\n,editor:search:fn-list,editor:search:fn-use=Num,editor:search:scope=text,editor:search:status,editor:search:replace-all,wait:800,editor:search:fn-report,editor:search:has=OEBPS/chapter-1.xhtml|N1-1 of,editor:search:has=OEBPS/chapter-2.xhtml|N26-1 of,editor:search:has=OEBPS/chapter-3.xhtml|N75-25 of,editor:search:fn-use=Broken,editor:search:replace-all,wait:600,editor:search:fn-report,editor:search:has=OEBPS/chapter-1.xhtml|Chapter 1,editor:search:fn-use=Quiet,editor:search:replace-all,wait:600,editor:search:fn-report,editor:search:save=QuietSearch,editor:search:saved,editor:search:fn-edit=Auto-number sections,wait:700,a11y-audit:,editor:search:fn-open,wait:700,a11y-audit:,quit' '
    grep "^editor_fns:" <<<"$OUT" | grep -q "Num" && grep "^editor_fns:" <<<"$OUT" | grep -q "Title-case text" ; check "built-in and saved functions are listed together (CE-112)" $?
    grep "^editor_search:" <<<"$OUT" | sed -n 1p | grep -q "mode=Function" && grep "^editor_search:" <<<"$OUT" | sed -n 1p | grep -q "find=.Paragraph (.*d+) of"; check "choosing a function switches to Function mode and fills in its expression" $?
    R=$(grep "^editor_fn_report:" <<<"$OUT")
    sed -n 1p <<<"$R" | grep -q "75 replacements"; check "Replace All calls the function for every match and counts them (CE-100, CE-101)" $?
    sed -n 1p <<<"$R" | grep -q "Final output:.*final 75 UI Test Book"; check "the call after the last match gets m = () and the data kept through the run; meta.title is readable (CE-104, CE-106, CE-109)" $?
    sed -n 1p <<<"$R" | grep -q "Debug output:.*chapter-1.xhtml.*total 75"; check "print() output is shown in the Debug output after the run (CE-111)" $?
    sed -n 1p <<<"$R" | grep -q "dialogs=1"; check "the results window was shown once" $?
    grep -q "^editor_search_has: OEBPS/chapter-1.xhtml \"N1-1 of\" = 1" <<<"$OUT"; check "number starts at 1 in the first file (CE-102) and file_name is passed (CE-103)" $?
    grep -q "^editor_search_has: OEBPS/chapter-2.xhtml \"N26-1 of\" = 1" <<<"$OUT"; check "number goes on counting across files in the spine order (CE-108)" $?
    grep -q "^editor_search_has: OEBPS/chapter-3.xhtml \"N75-25 of\" = 1" <<<"$OUT"; check "the last file ends at the last number" $?
    sed -n 2p <<<"$R" | grep -q "Error: .*line 3"; check "an error in the function is shown with its line number" $?
    grep -q "^editor_search_has: OEBPS/chapter-1.xhtml \"Chapter 1\" = 2$" <<<"$OUT"; check "a failing function changes nothing" $?
    sed -n 3p <<<"$R" | grep -q "dialogs=2"; check "suppress_result_dialog hides the results window (CE-110)" $?
    grep -q "^editor_saved: 0 | QuietSearch | Function · Chapter → function Quiet" <<<"$OUT"; check "a saved search can use a function" $?
    ! has "^a11y-problem:"; check "the function dialogs have accessible names" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "function mode runs without errors" $?'

scenario editor_live_marks 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:500,editor:marks,editor:find-in-text=Paragraph 2 of,editor:insert=<b>NEW</b> ,editor:find-in-text=Paragraph 5 of,editor:insert=<p>added</p>\n,wait:300,editor:marks,editor:tip=12,editor:find-in-text=Paragraph 8 of,editor:insert=<img src="missing.png" alt="x"/>,editor:check,wait:500,editor:marks,editor:tip=19,editor:checkpoint,editor:marks,editor:find-in-text=Paragraph 12 of,editor:insert=Z,editor:marks,quit' '
    M=$(grep "^editor_marks:" <<<"$OUT")
    sed -n 1p <<<"$M" | grep -q "changed=0 findings=0"; check "a freshly opened file has no change marks" $?
    sed -n 2p <<<"$M" | grep -Eq "lines=.*[0-9]+~"; check "an edited line is marked as changed" $?
    sed -n 2p <<<"$M" | grep -Eq "lines=.*[0-9]+\+"; check "a new line is marked as added" $?
    has "^editor_tip: 12 = .*Changed since the last checkpoint"; check "hovering a changed line says so" $?
    has "^editor_tip: 19 = .*missing"; check "hovering a finding shows the text of the finding" $?
    sed -n 2p <<<"$M" | grep -Eq "12~s1"; check "a misspelled word is counted on its line next to the change mark" $?
    sed -n 3p <<<"$M" | grep -Eq "findings=[1-9]"; check "findings of the book check are marked on their lines" $?
    sed -n 3p <<<"$M" | grep -Eq "lines=.*[0-9]+[~+]?[EW]"; check "the finding carries its severity" $?
    sed -n 4p <<<"$M" | grep -q "changed=0"; check "a checkpoint resets the change marks" $?
    sed -n 5p <<<"$M" | grep -Eq "changed=1 "; check "only edits after the checkpoint are marked" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "the gutter column draws without errors or GTK warnings" $?'

scenario editor_live_spell 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:500,editor:spots,editor:find-in-text=Paragraph 2 of,editor:insert=<p>Wrods and tehe</p>,wait:300,editor:spots,editor:spell-menu=Wrods,editor:spell-menu=Paragraph,editor:spell-act=replace|Wrods|Words,editor:has=<p>Words and tehe</p>,editor:spots,editor:spell-act=ignore|tehe,editor:spots,editor:insert=<p>Wrods</p>,editor:spell-act=add|Wrods,editor:spots,editor:insert=<p>Wrods</p>,editor:spots,quit' '
    S=$(grep "^editor_spots:" <<<"$OUT")
    sed -n 1p <<<"$S" | grep -q "ready=true .* n=0 "; check "a clean chapter has no misspelled words and the dictionary is loaded" $?
    sed -n 2p <<<"$S" | grep -q "n=2 words=Wrods tehe"; check "typed misspellings are found" $?
    sed -n 2p <<<"$S" | grep -q "underlined=Wrods|tehe"; check "the misspelled words carry the squiggle tag" $?
    grep "^editor_spell_menu: Wrods" <<<"$OUT" | grep -q "Replace with\[.*Words.*\] | Ignore “Wrods” | Add to Dictionary"; check "the context menu offers Replace with, Ignore and Add to Dictionary" $?
    grep "^editor_spell_menu: Paragraph" <<<"$OUT" | grep -q -- "->\s*$"; check "a correct word gets no spelling entries" $?
    has "^editor_has: <p>Words and tehe</p> = 1"; check "Replace with changes just that word" $?
    sed -n 3p <<<"$S" | grep -q "n=1 words=tehe"; check "the replaced word is not underlined any more" $?
    sed -n 4p <<<"$S" | grep -q " n=0 "; check "Ignore removes the squiggle" $?
    sed -n 5p <<<"$S" | grep -q " n=0 "; check "Add to Dictionary removes the squiggle" $?
    grep -qi wrods "$d/cfg/bookx/spelling-words.txt"; check "the added word is saved in the personal dictionary" $?
    sed -n 6p <<<"$S" | grep -q " n=0 "; check "an added word stays accepted when typed again" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "live spelling runs without errors or GTK warnings" $?'

scenario editor_live_pairs 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:500,editor:find-in-text=h1>Chapter,editor:pair,editor:find-in-text=aragraph 1 of,editor:pair,editor:find-in-text=/p>,editor:pair,editor:open=OEBPS/style.css,wait:300,editor:cursor=3,editor:pair,quit' '
    P=$(grep "^editor_pair:" <<<"$OUT")
    sed -n 1p <<<"$P" | grep -q "editor_pair: <h1>|</h1>"; check "with the caret in a tag, the tag and its partner are highlighted" $?
    sed -n 2p <<<"$P" | grep -q "editor_pair: $"; check "no tag is highlighted while the caret is in text" $?
    sed -n 3p <<<"$P" | grep -q "editor_pair: <p>|</p>"; check "a closing tag highlights its opening tag" $?
    sed -n 4p <<<"$P" | grep -q "editor_pair: $"; check "stylesheets have no tag pairs" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "the tag-pair highlight runs without errors or GTK warnings" $?'

scenario editor_live_hidden "edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:500,editor:hidden,editor:find-in-text=Paragraph 2 of,editor:insert=x$(printf '\302\240')y$(printf '\342\200\213')z,editor:hidden,editor:view-opt=whitespace:on,wait:300,editor:hidden,editor:view-opt=whitespace:off,wait:300,editor:hidden,quit" '
    H=$(grep "^editor_hidden:" <<<"$OUT")
    sed -n 1p <<<"$H" | grep -q "editor_hidden: $"; check "nothing is highlighted while the option is off" $?
    sed -n 2p <<<"$H" | grep -q "editor_hidden: $"; check "typing a no-break space does not highlight it with the option off" $?
    sed -n 3p <<<"$H" | grep -q "editor_hidden: <nbsp>|<zwsp>"; check "Show Hidden Characters highlights no-break and zero-width spaces" $?
    sed -n 4p <<<"$H" | grep -q "editor_hidden: $"; check "switching the option off removes the highlight" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "highlighting hidden characters runs without errors" $?'

scenario editor_live_complete 'edit:1,wait:3000,editor:open=OEBPS/chapter-2.xhtml,wait:300,editor:find-in-text=Paragraph 1 of,editor:insert=<span id="zzmark"/>,wait:200,editor:open=OEBPS/chapter-1.xhtml,wait:700,editor:complete=<p>x</p><di,editor:complete=<a hr,editor:complete=<img alt="x" sr,editor:complete=<a href="chapter-,editor:complete=<a href="chapter-2.xhtml#zz,editor:complete=<p>The functio,editor:complete=<p>sec,editor:complete=<p>zzzz,editor:complete=<!-- <di,editor:complete-show,wait:700,editor:complete-last,editor:open=OEBPS/style.css,wait:700,editor:complete=p { col,editor:complete=p { color: red; mar,editor:complete=p { color: re,editor:complete=p { background: url(chap,editor:complete=p.fo,quit' '
    C=$(grep "^editor_complete:" <<<"$OUT")
    sed -n 1p <<<"$C" | grep -q "div\[tag:2\]"; check "typing <di offers div" $?
    sed -n 2p <<<"$C" | grep -q "href\[attribute:2\]"; check "inside a tag, attribute names are offered" $?
    sed -n 3p <<<"$C" | grep -q "src\[attribute:2\]"; check "attributes fit the tag (src for img)" $?
    sed -n 4p <<<"$C" | grep -q "chapter-2.xhtml\[file:8\]"; check "href completes the internal files of the book" $?
    ! sed -n 4p <<<"$C" | grep -q "chapter-1.xhtml"; check "the open file itself is not offered as a link target" $?
    sed -n 5p <<<"$C" | grep -q "zzmark\[anchor:2\]"; check "after file# the ids of that file are offered, including unsaved ones" $?
    sed -n 6p <<<"$C" | grep -q "functionality\[word:7\]"; check "words of the open file are offered as a fallback" $?
    sed -n 7p <<<"$C" | grep -q "sec  —  Section (epub:type)\[snippet:3\]"; check "snippets whose trigger matches are offered while typing" $?
    sed -n 8p <<<"$C" | grep -q "complete: n=0 "; check "nothing is offered when nothing fits" $?
    sed -n 9p <<<"$C" | grep -q "complete: n=0 "; check "no proposals inside a comment" $?
    has "^editor_popup: calls=[1-9][0-9]* last=[1-9]"; check "the real popup asks the provider and gets proposals" $?
    sed -n 10p <<<"$C" | grep -q "color\[property:3\]"; check "CSS property names are offered inside a rule" $?
    sed -n 11p <<<"$C" | grep -q "margin"; check "properties are offered after a finished declaration" $?
    sed -n 12p <<<"$C" | grep -q "complete: n=0 "; check "no property names where a value is typed" $?
    sed -n 13p <<<"$C" | grep -q "chapter-1.xhtml\[file:4\]"; check "url() completes files of the book" $?
    sed -n 14p <<<"$C" | grep -q "complete: n=0 "; check "no proposals in a selector" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "completion runs without errors or GTK warnings" $?'

scenario editor_live_snippets 'edit:1,wait:3000,editor:open=OEBPS/chapter-1.xhtml,wait:500,editor:snippets,editor:find-in-text=Paragraph 2 of,editor:insert=\n,editor:snippet=sec,editor:has=<section epub:type="chapter" id="id">,editor:has=<h2>Title</h2>,editor:cur,editor:snippet=nosuchtrigger,editor:snippet-add=Mine|zq|any|<i>${1:x}</i>$0,wait:200,editor:snippets,editor:find-in-text=Paragraph 3 of,editor:insert=\n,editor:snippet=zq,editor:has=<i>x</i>,editor:open=OEBPS/style.css,wait:300,editor:cursor=0,editor:snippet=rule,editor:has=selector {,editor:cursor=0,editor:snippet=zq,editor:snippet-add=Mine2|p|markup|<p class="mine">$0</p>,wait:200,editor:open=OEBPS/chapter-1.xhtml,wait:300,editor:find-in-text=Paragraph 4 of,editor:insert=\n,editor:snippet=p,editor:has=<p class="mine">,editor:snippets-dialog,wait:600,quit' '
    N=$(grep "^editor_snippets:" <<<"$OUT")
    sed -n 1p <<<"$N" | grep -Eq "triggers=.*\bp\b.*\bh\b.*\bimg\b.*\blink\b.*\bsec\b"; check "paragraph, heading, image, link and epub:type section are built in" $?
    grep "^editor_snippet: expanded=true" <<<"$OUT" | head -1 | grep -q true; check "a trigger followed by Tab expands" $?
    has "^editor_has: <section epub:type=\"chapter\" id=\"id\"> = 1"; check "the snippet text has its default values filled in" $?
    has "^editor_has: <h2>Title</h2> = 1"; check "tab stops hold their default text" $?
    sed -n 2p <<<"$(grep "^editor_snippet:" <<<"$OUT")" | grep -q "expanded=false"; check "an unknown trigger does nothing" $?
    sed -n 2p <<<"$N" | grep -q "triggers=.* zq"; check "a user snippet is available as soon as it is added" $?
    has "^editor_has: <i>x</i> = 1"; check "a user snippet expands with its tab stops" $?
    has "^editor_has: selector { = 1"; check "CSS snippets expand in stylesheets" $?
    sed -n 5p <<<"$(grep "^editor_snippet:" <<<"$OUT")" | grep -q "expanded=false"; check "while a snippet is active its text is not expanded again (Tab moves between the tab stops)" $?
    grep -q "\"trigger\": \"zq\"" "$d/cfg/bookx/snippets.json"; check "user snippets are stored in the config folder" $?
    has "^editor_has: <p class=\"mine\"> = 1"; check "a user snippet replaces the built-in with the same trigger" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "snippets and the Manage Snippets dialog run without errors or GTK warnings" $?'

if [ "${NET:-1}" != 0 ]; then
    scenario wikipedia 'read:1,wait:2500,reader:wiki,wait:12000,reader:dump,quit' '
        has "^wiki: <b>"; check "wikipedia summary is shown" $?'
    scenario wikipedia_button 'read:1,wait:2500,reader:ui-dblclick,wait:800,reader:ui-wiki-btn,wait:12000,reader:dump,quit' '
        has "^wiki: <b>"; check "the real Wikipedia button shows a summary (regression: bug 1)" $?
        has "^dict: wiki \|"; check "the Wikipedia page is still showing when the summary has arrived (regression: bug 1)" $?
        has "^popover: true"; check "the popover stays open when the summary arrives (regression: bug 1)" $?'
    scenario translate 'read:1,wait:2500,reader:tr,wait:12000,reader:dump,quit' '
        has "^tr: .+"; check "a translation is returned" $?
        has "^popover: true"; check "the popover stays open when the translation arrives (regression: bug 1)" $?
        ! has "^tr: Chapter"; check "it is not the English source (target switches away from the book language)" $?'
fi

# ---- accessibility, command palette, preferences search, RTL (docs/a11y-notes.md) ----------------
scenario a11y_audit_library 'a11y-audit:,wait:300,prefs:,wait:800,a11y-audit:,palette:open,wait:600,a11y-audit:,quit' '
    A=$(grep "^a11y:" <<<"$OUT")
    [ "$(wc -l <<<"$A")" -eq 3 ]; check "the audit ran for the library, with Preferences open and with the palette open" $?
    ! has "^a11y-problem:"; check "every control has an accessible name and icon-only buttons have tooltips" $?'
scenario a11y_audit_windows 'action:share,wait:600,a11y-audit:,action:devices,wait:900,a11y-audit:,action:vocab,wait:600,a11y-audit:,action:stats,wait:600,a11y-audit:,action:shortcuts,wait:500,a11y-audit:,edit:1,wait:3000,a11y-audit:,quit' '
    [ "$(grep -c "^a11y:" <<<"$OUT")" -eq 6 ]; check "the audit ran with the dialogs and the editor open" $?
    ! has "^a11y-problem:"; check "every control of the dialogs and the editor has an accessible name" $?'
scenario a11y_audit_editor_problems 'edit:1,wait:3000,editor:addfile=stray.css,editor:check,wait:600,editor:check:select-id=OPF-003,wait:300,a11y-audit:,quit' '
    [ "$(grep -c "^a11y:" <<<"$OUT")" -eq 1 ]; check "the audit ran with the Problems panel open" $?
    ! has "^a11y-problem:"; check "every control of the Problems panel has an accessible name" $?'
scenario a11y_audit_reader 'read:1,wait:2500,a11y-audit:,reader:toc,wait:500,a11y-audit:,quit' '
    A=$(grep "^a11y:" <<<"$OUT")
    [ "$(wc -l <<<"$A")" -eq 2 ]; check "the audit ran in the reader" $?
    ! has "^a11y-problem:"; check "every control of the reader has an accessible name" $?'

scenario palette_library 'palette:open,wait:700,palette:dump,palette:type=pref,wait:300,palette:dump,palette:type=ui test,wait:300,palette:dump,palette:type=hyphen,wait:300,palette:dump,palette:type=zzqq,wait:300,palette:dump,palette:type=pref,wait:300,palette:pick,wait:1500,palette:dump,quit' '
    P=$(grep "^palette: query" <<<"$OUT")
    sed -n 1p <<<"$P" | grep -q "results=Command|Add Books"; check "an empty palette lists the commands of the library, shortcuts first" $?
    sed -n 2p <<<"$P" | grep -q "results=Command|Preferences"; check "typing pref puts the Preferences command first" $?
    sed -n 3p <<<"$P" | grep -Eq "Book\|UI Test Book"; check "a book is found by its title" $?
    sed -n 4p <<<"$P" | grep -q "Setting|Hyphenate words"; check "a setting of Preferences is found by a part of its name" $?
    sed -n 5p <<<"$P" | grep -q "results=$"; check "nothing matches nonsense" $?
    has "^palette: picked Preferences -> Action"; check "Enter runs the chosen command" $?
    has "^palette: closed"; check "the palette closes after a choice" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no GTK warnings" $?'

scenario palette_reader 'read:1,wait:2500,reader:dump,palette:open,wait:600,palette:type=next page,wait:300,palette:dump,palette:pick,wait:900,reader:dump,palette:open,wait:600,palette:type=theme dark,wait:300,palette:pick,wait:1200,reader:dump,palette:registry,palette:open,wait:600,palette:type=hyphen,wait:300,palette:pick,wait:1800,quit' '
    has "^palette: query=\"next page\" selected=0 results=Command\|Next Page"; check "in the reader the palette offers the reader commands (Next Page first)" $?
    L=$(grep "^labels:" <<<"$OUT")
    [ "$(sed -n 1p <<<"$L")" != "$(sed -n 2p <<<"$L")" ]; check "choosing Next Page turned the page" $?
    PL=$(grep "^palette: [0-9a-f]\{6\}/" <<<"$OUT")
    [ "$(sed -n 1p <<<"$PL")" != "$(sed -n 3p <<<"$PL")" ]; check "choosing Theme: Dark changed the page colours" $?
    has "^palette: library_missing=\[\] reader_missing=\[\]"; check "every command of the registry has its action in the library, the reader and the application" $?
    has "^prefs_focus: page=Reading row=Hyphenate words focused=true"; check "choosing a setting opens Preferences on its page and focuses its row" $?'

scenario keyboard_selection 'read:1,wait:2500,reader:ksel=word+,wait:300,reader:ksel=word+,wait:300,reader:dump,reader:ksel=char-,wait:300,reader:dump,reader:ksel=sent+,wait:300,reader:dump,reader:selkey=commit,wait:700,reader:dump,reader:selkey=highlight,wait:700,reader:dump,reader:selkey=clear,wait:300,reader:dump,quit' '
    S=$(grep "^selected:" <<<"$OUT")
    sed -n 1p <<<"$S" | grep -q "Some(\"[^\"]\+\")"; check "Shift+Ctrl+Right twice selects two words from the start of the page" $?
    A=$(sed -n 1p <<<"$S" | wc -c); B=$(sed -n 2p <<<"$S" | wc -c)
    [ "$B" -lt "$A" ]; check "Shift+Left shortens the selection by a character" $?
    C=$(sed -n 3p <<<"$S" | wc -c)
    [ "$C" -gt "$B" ]; check "Shift+Down extends it to the end of the sentence" $?
    sed -n 4p <<<"$S" | grep -q "Some"; check "Enter keeps the selection (and opens the actions)" $?
    has "^popover: true"; check "Enter opens the actions popover for the keyboard selection" $?
    has "^bookmarks_notes: 1"; check "the h key highlights the keyboard selection" $?'

scenario page_text_for_readers 'read:1,wait:2500,reader:page-text,reader:next,wait:800,reader:page-text,reader:next,wait:800,reader:page-text,quit' '
    T=$(grep "^page_text:" <<<"$OUT")
    [ "$(wc -l <<<"$T")" -eq 3 ]; check "the reader exposes the text of the page" $?
    [ "$(sed -n 1p <<<"$T")" != "$(sed -n 2p <<<"$T")" ] && [ "$(sed -n 2p <<<"$T")" != "$(sed -n 3p <<<"$T")" ]; check "the text changes with every page turn" $?
    [ "$(sed -n 3p <<<"$T" | wc -c)" -gt 200 ]; check "a text page holds its paragraphs, not just a title" $?'

scenario prefs_search 'prefs:,wait:800,palette:open,wait:600,palette:type=sync,wait:300,palette:dump,palette:type=mail,wait:300,palette:dump,palette:type=plugin,wait:300,palette:dump,quit' '
    P=$(grep "^palette: query" <<<"$OUT")
    sed -n 1p <<<"$P" | grep -q "Setting|"; check "settings of the Devices page are in the search index" $?
    sed -n 2p <<<"$P" | grep -q "Setting|"; check "settings of the E-mail section are in the search index" $?
    sed -n 3p <<<"$P" | grep -q "Setting|\|Command|"; check "plugin settings are in the search index" $?'

scenario prefs_sidebar 'prefs:,wait:900,prefs:dump,prefs:search=hyphen,wait:400,prefs:dump,prefs:search=mail server,wait:400,prefs:dump,prefs:search=sync,wait:400,prefs:dump,prefs:search=zzqq,wait:400,prefs:dump,prefs:search=,wait:400,prefs:dump,prefs:pick=Patches,wait:300,prefs:dump,quit' '
    P=$(grep "^prefs: query" <<<"$OUT")
    sed -n 1p <<<"$P" | grep -q "sections=General|Library|Reading|Devices and Sharing|Editor|E-mail|Sync|Extensions|Patches.* selected=General"; check "Preferences lists its sections in the sidebar, General first" $?
    sed -n 2p <<<"$P" | grep -q "sections=Reading selected=Reading hits=[1-9]"; check "searching filters the sidebar to the matching section, selects it and highlights the hit" $?
    sed -n 3p <<<"$P" | grep -q "sections=E-mail.* selected=E-mail"; check "a search finds a row of another section" $?
    sed -n 4p <<<"$P" | grep -q "sections=.*Sync.* selected"; check "the search also matches section titles" $?
    sed -n 5p <<<"$P" | grep -q "empty=true"; check "nonsense shows the empty state" $?
    sed -n 6p <<<"$P" | grep -q "sections=General|Library|Reading|Devices and Sharing|Editor|E-mail|Sync|Extensions|Patches.* selected=.* hits=0"; check "clearing the search brings every section back" $?
    sed -n 7p <<<"$P" | grep -q "selected=Patches"; check "a section can be picked" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no GTK warnings" $?'

scenario rtl_book "library:import=$ROOT/rtl-epub/lib,wait:2500,read:3,wait:3500,reader:page-text,reader:dump,reader:next,wait:600,reader:next,wait:600,reader:dump,reader:tap=0.9,wait:700,reader:dump,reader:tap=0.1,wait:700,reader:dump,reader:swipe-left,wait:700,reader:dump,quit" '
    has "^book_rtl: true"; check "page-progression-direction=rtl in the spine is read" $?
    has "^page_text: .*الفصل الأول"; check "the page text for the screen reader holds the Arabic paragraphs" $?
    has "^rtl: paragraphs=[1-9]"; check "the paragraphs are laid out right to left" $?
    has "^tap: x=0.9 action=PrevPage"; check "a tap on the right edge goes back in a right-to-left book" $?
    has "^tap: x=0.1 action=NextPage"; check "a tap on the left edge goes forward in a right-to-left book" $?
    has "^swipe: swipe-left"; check "a swipe is handled" $?
    L=$(grep "^labels:" <<<"$OUT")
    P=$(grep "^labels:" <<<"$OUT" | sed "s/labels: \([0-9]*\) of.*/\1/" | tr "\n" " ")
    [ "$P" = "1 5 3 5 3 " ]; check "two forward turns, a right tap goes back, a left tap forward, a left swipe back (pages: $P)" $?'
scenario rtl_footer "library:import=$ROOT/rtl-epub/lib,wait:2500,read:3,wait:3500,reader:nav-layout,quit" '
    has "^nav_layout: next_left_of_prev=true sidebar_end=true rtl=true"; check "in a right-to-left book the Next arrow is left of Previous and the contents open on the right" $?'
scenario ltr_footer "read:1,wait:2500,reader:nav-layout,quit" '
    has "^nav_layout: next_left_of_prev=false sidebar_end=false rtl=false"; check "in a left-to-right book the footer and the contents stay as they were" $?'
scenario ltr_book_tap "read:1,wait:2500,reader:tap=0.9,wait:500,reader:tap=0.1,wait:500,reader:page-text,quit" '
    has "^tap: x=0.9 action=NextPage"; check "a tap on the right edge goes forward in a left-to-right book" $?
    has "^tap: x=0.1 action=PrevPage"; check "a tap on the left edge goes back in a left-to-right book" $?
    has "^book_rtl: false"; check "an ordinary book is left to right" $?'

# ---- metadata review, series, works, import quality, preflight (docs/metadata-notes.md) ---------
scenario meta_series 'library:organize=series-dump,library:organize=series,wait:800,library:organize=series-dump,quit' '
    [ "$(grep -c "^series: The Saga | 0 of 3 read | missing #2 | order=1=Saga 1,3=Saga 3,4=Saga 4" <<<"$OUT")" -eq 2 ]; check "the Series view lists the series in order with the gap, before and after its window opened" $?
    [ "$(grep -c "^series:" <<<"$OUT")" -eq 2 ]; check "books without a series are not a series" $?'

scenario meta_series_number 'library:search=Saga 3,wait:600,library:select1,wait:600,library:editmeta,wait:2500,library:seriesdump,library:seriestext=Book Two,wait:300,library:seriesdump,library:seriestext=IV,wait:300,library:seriesdump,library:seriestext=soon,wait:300,library:seriesdump,library:seriestext=Book Two,wait:300,library:metasave,wait:2500,quit' '
    D=$(grep "^seriesdump:" <<<"$OUT")
    sed -n 1p <<<"$D" | grep -q "number=3 .*hint_visible=false"; check "the dialog starts with the number of the book and no preview" $?
    sed -n 2p <<<"$D" | grep -q "number=2 hint=\"= 2 · sorts between “Saga 1” and “Saga 4”\""; check "Book Two is read as 2 and the sort position is shown" $?
    sed -n 3p <<<"$D" | grep -q "number=4 .*same number as “Saga 4”"; check "IV is read as 4 and the clash is pointed out" $?
    sed -n 4p <<<"$D" | grep -q "number=4 .*not a number I can read"; check "text that is no number is explained and leaves the number alone" $?
    grep -q "^Series: *The Saga #2" <<<"$("$CLI" --library "$d/lib" library info 2)"; check "the saved book has series number 2" $?
    grep -q "^Sources: .*you: .*series" <<<"$("$CLI" --library "$d/lib" library info 2)"; check "the series fields are recorded as typed by the user" $?'

scenario meta_works 'library:organize=works-dump,library:organize=works,wait:800,library:organize=works-dump,quit' '
    [ "$(grep -c "^works: 1$" <<<"$OUT")" -eq 2 ]; check "one work with several editions is found" $?
    has "^work: Edition Tale \| 2 editions · epub \| ids="; check "the work lists its editions and formats" $?'

scenario meta_bulk_review 'library:selectall,wait:500,library:bulkfetch,wait:3500,library:bulkfetch=dump,quit' '
    has "^bulkfetch: state=review rows=[1-9][0-9]* accepted=0 applied_batches=0"; check "the review table has rows and nothing is accepted by default" $?
    has "^bulkrow: [0-9]+ book=\"Second Book\" field=publisher current=\"\" proposed=\"Mock Press\" source=\"openlibrary\" conf=[0-9]+ safe=true accepted=false"; check "a row shows book, field, current, proposed, source and confidence" $?
    has "^bulkfetch: 1 of 2 books found.*1 not found"; check "the summary counts the book the sources did not know" $?
    has "^bulkfetch_health: .*Open Library: ok"; check "the source health is shown" $?
    ! grep -q "Mock Press" <<<"$LIB"; check "nothing is written without review" $?'

scenario meta_bulk_apply 'library:selectall,wait:500,library:bulkfetch,wait:3500,library:bulkfetch=accept-field=publisher,library:bulkfetch=dump,library:bulkfetch=apply,wait:800,library:bulkfetch=dump,quit' '
    has "^bulkfetch: state=review rows=[0-9]+ accepted=1 "; check "accepting one field accepts only its rows" $?
    has "^bulkfetch: applied books=1 fields=1 ops=1 errors=0"; check "only the accepted field was written" $?
    grep -q "^Publisher: *Mock Press" <<<"$("$CLI" --library "$d/lib" library info 2)"; check "the publisher is in the catalogue" $?
    ! grep -q "^Published: *2001" <<<"$("$CLI" --library "$d/lib" library info 2)"; check "the other proposed fields were not applied" $?
    grep -q "^Sources: .*openlibrary: publisher" <<<"$("$CLI" --library "$d/lib" library info 2)"; check "the applied field records its source" $?
    grep -q "Edited \"Second Book\"" <<<"$("$CLI" --library "$d/lib" library history)"; check "the edit is in the operation log" $?'

scenario meta_bulk_undo 'library:selectall,wait:500,library:bulkfetch,wait:3500,library:bulkfetch=accept-all,library:bulkfetch=apply,wait:800,library:bulkfetch=undo,wait:800,library:bulkfetch=dump,quit' '
    has "^bulkfetch: undone"; check "Undo is offered after applying" $?
    ! grep -q "^Publisher: *Mock Press" <<<"$("$CLI" --library "$d/lib" library info 2)"; check "undoing through the operation log takes the values back" $?
    ! grep -q "^Sources: .*openlibrary" <<<"$("$CLI" --library "$d/lib" library info 2)"; check "and their recorded source" $?
    grep -q "(undone)" <<<"$("$CLI" --library "$d/lib" library history)"; check "the log shows the edit as undone" $?'

scenario meta_auto 'library:selectall,wait:500,library:bulkfetch,wait:3500,library:bulkfetch=dump,quit' '
    has "^bulkfetch: state=review .*applied_batches=1 auto=true"; check "with automatic safe changes on, the fill-in batch was applied" $?
    grep -q "^Publisher: *Mock Press" <<<"$("$CLI" --library "$d/lib" library info 2)"; check "an empty field was filled without a review" $?
    ! has "safe=true accepted=false"; check "no safe row is left waiting" $?'

scenario meta_fetch_health 'library:selectall,wait:500,library:bulkfetch,wait:4000,library:bulkfetch=dump,quit' '
    has "^bulkfetch_health: .*Google Books: failing.*paused"; check "a source that keeps failing is marked failing and paused (backoff)" $?
    has "^bulkfetch_health: .*Open Library: ok"; check "the healthy source is not affected" $?
    has "^bulkfetch: state=review rows=[1-9]"; check "the books are still looked up with the other sources" $?'

scenario meta_attention 'library:organize=attention-dump,library:organize=attention,wait:800,library:organize=idclean,wait:600,library:organize=attention-dump,quit' '
    has "^attention_book: scan_0042 \| no author, title looks like a file name, no cover"; check "a book with no author and a file-name title is listed with its reasons" $?
    has "^attention_id: [0-9]+ scan_0042 \| isbn:978-0-441-17271-9 → isbn:9780441172719 \| fixable=true"; check "the ISBN cleanup is suggested" $?
    [ "$(grep -c "^attention_id:" <<<"$OUT")" -eq 1 ]; check "the suggestion is gone after the cleanup" $?
    grep -q "isbn:9780441172719" <<<"$("$CLI" --library "$d/lib" library info 1)"; check "the cleaned identifier is saved" $?
    grep -q "^Sources: .*identifier cleanup: identifiers" <<<"$("$CLI" --library "$d/lib" library info 1)"; check "the cleanup is recorded in the provenance" $?
    grep -q "Edited \"scan_0042\"" <<<"$("$CLI" --library "$d/lib" library history)"; check "and it is in the operation log (undoable)" $?'

scenario meta_weak_filter 'library:search=weak:author,wait:700,library:dump,library:search=weak:title,wait:700,library:dump,library:search=,wait:700,library:dump,quit' '
    T=$(grep "^title:" <<<"$OUT")
    sed -n 1p <<<"$T" | grep -q "1 of 3 books"; check "weak:author finds the book without an author" $?
    sed -n 2p <<<"$T" | grep -q "1 of 3 books"; check "weak:title finds the book whose title looks like a file name" $?
    sed -n 3p <<<"$T" | grep -q "| 3 books"; check "clearing the search shows everything" $?'

scenario meta_tidy_provenance 'library:health=open,wait:800,library:health=tidy,wait:1500,quit' '
    grep -q "^Sources: .*tidy names: authors" <<<"$("$CLI" --library "$d/lib" library info 1)"; check "tidying names records its source for the changed field" $?'

scenario meta_preflight 'library:search=Broken,wait:700,library:select1,wait:500,library:preflight,wait:2500,library:preflight=dump,library:preflight=fix,wait:2500,library:preflight=dump,quit' '
    P=$(grep "^preflight_book:" <<<"$OUT")
    sed -n 1p <<<"$P" | grep -q "Broken Mimetype | 1 blocking.* fixable=true | .*PKG-007"; check "the broken mimetype is a blocking problem with a fix" $?
    ! sed -n 2p <<<"$P" | grep -q "PKG-007"; check "the fix removes the problem" $?
    ls "$d"/lib/.bookx/preflight-backups/*.epub >/dev/null 2>&1; check "the original file is kept before it is rewritten" $?
    has "^preflight_state: shown=true fix_enabled=true"; check "Fix All is offered while a fixable problem exists" $?'

scenario meta_preflight_gate 'library:search=Broken,wait:700,library:select1,wait:500,library:emailsel,wait:2500,library:preflight=dump,quit' '
    has "^preflight_state: shown=true .*go_label=Some\(\"Continue Anyway\"\)"; check "with the check switched on, e-mailing a book with a blocking problem stops at the report" $?'

scenario meta_conversion_report 'library:select1,library:convertto=azw3:@D@/out.azw3,wait:3000,quit' '
    [ -s "$d/out.azw3" ]; check "the converted file is written" $?
    grep -q "^Conversion report: UI Test Book" "$d/out.azw3.report.txt"; check "a report is saved next to it" $?
    grep -q "^Target: AZW3" "$d/out.azw3.report.txt"; check "it names the target" $?'

scenario patches_provenance "patches:enable=c-import,library:import=$ROOT/third,wait:3500,quit" '
    grep -q "^Sources: .*patch: c-import: content" <<<"$("$CLI" --library "$d/lib" library info 3)"; check "a book changed by an import patch records that patch as the source of its content" $?'


# ---- several formats in one book ----
scenario cat_formats 'library:formats=retype=2:azw3,wait:300,library:formats=dump,library:formats=merge-preview=1|2,wait:700,library:formats=merge=1|2,wait:600,library:formats=dump,library:formats=rows=1,library:search=format:azw3,wait:500,library:undo=1,wait:500,library:formats=dump,quit' '
    has "^formats: preview Result: Edition Tale: epub \+ azw3 \| Merge \"Edition Tale\" \(azw3"; check "the preview names the result and what is merged" $?
    has "^formats: preview .*Highlights, bookmarks and the reading position of the merged copies stay in the Trash record"; check "the preview says what is not carried over" $?
    has "^formats: merged 1"; check "the merge ran" $?
    [ "$(grep -c "^formats: book=[0-9]* title=\"Edition Tale\"" <<<"$OUT")" -eq 5 ]; check "two books before, one book after the merge, two after undo" $?
    has "^formats: book=1 title=\"Edition Tale\" files=epub,azw3$"; check "one book holds both formats" $?
    has "^formats: row book=1 format=azw3 main=false exists=true in_bookx=false"; check "the details list shows the AZW3 file as a further format that opens outside Bookx" $?
    has "^formats: row book=1 format=epub main=true exists=true in_bookx=true"; check "and the EPUB as the main file" $?
    grep -q "\"UI Test Book\"" <<<"$LIB" && [ "$(grep -c "\"title\": *\"Edition Tale\"" <<<"$LIB")" -eq 2 ]; check "after undo both books are back in the catalogue" $?'

scenario cat_formats_remove 'library:formats=retype=2:azw3,library:formats=merge=1|2,wait:400,library:formats=dump,library:formats=remove=1:azw3,wait:400,library:formats=dump,library:undo=1,wait:400,library:formats=dump,quit' '
    has "^formats: removed azw3"; check "one format can be taken off a book" $?
    L=$(grep "^formats: book=1 " <<<"$OUT")
    sed -n 1p <<<"$L" | grep -q "files=epub,azw3"; check "merged: two formats" $?
    sed -n 2p <<<"$L" | grep -q "files=epub$"; check "after removing one: the EPUB only" $?
    sed -n 3p <<<"$L" | grep -q "files=epub,azw3"; check "undo puts the removed format back" $?'

scenario cat_import_format "library:formats=offers=add,library:import=@D@/other,wait:3500,library:formats=dump,library:undo=1,wait:500,library:formats=dump,quit" '
    has "^formats: offer answered add"; check "importing the same book in another format asks instead of adding a second book" $?
    has "^formats: book=[0-9]+ title=\"UI Test Book\" files=epub,fb2$"; check "answering Add as Another Format puts the FB2 file into the existing book" $?
    [ "$(grep -c "^formats: book=[0-9]* title=\"UI Test Book\"" <<<"$OUT")" -eq 2 ]; check "no second book was created, and undo takes the format off again" $?
    ls "$d"/lib/Test*/*/*.fb2 >/dev/null 2>&1 || [ "$(grep -c "files=epub,fb2" <<<"$OUT")" -ge 1 ]; check "the original file stays where it was" $?'

scenario cat_import_format_separate "library:formats=offers=separate,library:import=@D@/other,wait:3500,library:formats=dump,quit" '
    [ "$(grep -c "^formats: book=[0-9]* title=\"UI Test Book\"" <<<"$OUT")" -eq 2 ]; check "Import as Separate Books adds the book as before" $?'

# ---- copy protection ----
scenario cat_drm "library:import=@D@/drm,wait:3000,library:formats=drm,quit" '
    has "^formats: protected book=[0-9]+ title=\"Locked Tale\" drm=\"Adobe ADEPT\" message=\"Protected \(Adobe ADEPT\): Bookx cannot open it; Bookx does not remove DRM\.\""; check "a protected EPUB is imported and marked with its scheme" $?
    has "^formats: drm-search matches 1$"; check "drm: finds the protected books" $?
    has "import failed: .*kindle.mobi.*Kindle DRM.*does not remove DRM"; check "a Kindle file with the encryption flag is refused with the clear message" $?
    [ "$(grep -c "\"file\":" <<<"$LIB")" -eq 3 ]; check "the refused file adds no book (2 test books + the protected EPUB)" $?
    ! has "formats: protected book=[0-9]+ title=\"Second Book\""; check "unprotected books carry no badge" $?'

# ---- authors ----
scenario cat_authors 'library:authors=dump,library:authors=merge=Samuel Clemens|S. Clemens>>Mark Twain,wait:600,library:authors=dump,library:authors=open,wait:700,library:authors=rename-preview=Mark Twain>>Mark Twain Jr,wait:500,library:undo=1,wait:500,library:authors=dump,quit' '
    has "^authors: Samuel Clemens \| books=1"; check "before: the pen name is a person of its own" $?
    has "^authors: Mark Twain \| books=3 \| variants="; check "after the merge the browser groups the three books under one name" $?
    has "^authors_aliases: 2$"; check "both old spellings are remembered as aliases" $?
    has "^authors: preview 3 books will list \"Mark Twain Jr\""; check "the rename dialog previews how many books change before anything is written" $?
    grep -q "\"authors\": *\[ *\"Samuel Clemens\"" <<<"$LIB" || grep -q "Samuel Clemens" <<<"$LIB"; check "one undo takes the whole merge back" $?
    [ "$(grep -c "^authors: Mark Twain | books=3" <<<"$OUT")" -ge 2 ]; check "the aliases still group the old spellings after the undo" $?'

# ---- trackers ----
scenario cat_trackers "library:trackers=plan=@D@/goodreads.csv,library:trackers=import=@D@/goodreads.csv,wait:500,library:trackers=export=goodreads:@D@/out-gr.csv,library:trackers=export=storygraph:@D@/out-sg.csv,library:undo=1,wait:400,quit" '
    has "^trackers: 3 rows, 2 matched with your books, 2 would change, 1 not in the library"; check "the preview counts matches and changes" $?
    has "^trackers_update: Dune rating=Some\(10\) status=Some\(\"Read\"\) date=Some\(\"2021-03-14\"\) tags=favorites,sci-fi"; check "a Goodreads row gives rating, status, date and shelves" $?
    has "^trackers_update: Emma rating=None status=None date=None tags=classics,to-read"; check "a to-read row gives its shelf and a to-read tag" $?
    has "^trackers: applied books=2 ratings=1 tags=4 statuses=1 dates=1"; check "applying reports what it wrote" $?
    grep -q "^Title,Author,Additional Authors,ISBN,ISBN13,My Rating,Date Read,Bookshelves,Exclusive Shelf" "$d/out-gr.csv"; check "the Goodreads export has the layout the services import" $?
    grep -q "^Dune,Frank Herbert,,,\"\"=\"\"\"\"\"\"\"\"\|^Dune,Frank Herbert" "$d/out-gr.csv" && grep -q "read$" "$d/out-gr.csv"; check "it carries the reading status" $?
    grep -q "^Title,Authors,ISBN/UID,Read Status,Star Rating,Last Date Read,Tags" "$d/out-sg.csv"; check "the StoryGraph export has its own layout" $?
    ! grep -q "^Rating:" <<<"$("$CLI" --library "$d/lib" library info 1)"; check "one undo takes back the imported rating" $?'

# ---- audiobooks ----
scenario cat_audiobooks 'library:audiobooks=find=@D@/audio,library:audiobooks=link=@D@/audio,wait:500,library:audiobooks=dump,library:audiobooks=open=@D@/audio,wait:600,library:search=audiobook:,wait:400,library:undo=1,wait:400,library:audiobooks=dump,quit' '
    has "^audiobook: book=\"Dune\" path=.*Frank Herbert - Dune.m4b reason=\"same title and author\""; check "an .m4b named like the book is paired with it" $?
    ! has "audiobook: book=.*Nothing Like A Book"; check "an unrelated audio file is not paired" $?
    has "^audiobook: 1 found"; check "exactly one pairing is proposed" $?
    has "^audiobook_link: book=\"Dune\" Frank Herbert - Dune.m4b · "; check "the link shows in the details with its size" $?
    [ "$(grep -c "^audiobook_link:" <<<"$OUT")" -eq 1 ]; check "undo takes the link off again" $?'

# ---- fetch at import, and the response cache ----
scenario cat_fetch_import 'wait:6000,library:bulkfetch=dump,quit' '
    has "^bulkfetch: import lookup found=1 of 2"; check "with fetch-on-import on, the new books are looked up without a window" $?
    grep -q "^Publisher: *Mock Press" <<<"$("$CLI" --library "$d/lib" library info 2)"; check "an empty field was filled" $?
    grep -q "^Sources: .*openlibrary: .*publisher" <<<"$("$CLI" --library "$d/lib" library info 2)" || grep -q "^Sources: .*publisher" <<<"$("$CLI" --library "$d/lib" library info 2)"; check "and its source is recorded" $?
    ! has "^bulkrow:"; check "the review table was never needed" $?'

scenario cat_cache 'library:selectall,wait:500,library:bulkfetch,wait:3500,library:bulkfetch=cache,library:bulkfetch=dump,quit' '
    has "^bulkfetch_cache: hits=[0-9]+ misses=[1-9]"; check "the lookups went through the cache" $?'


# ---- provenance of the other edit paths ----
scenario meta_bulk_provenance 'library:selectall,wait:500,library:bulkedit=publisher:Bulk Press|tag:+extra,wait:700,quit' '
    has "^bulkedit: 2 books"; check "the bulk editor changed both books" $?
    grep -q "^Sources: .*bulk edit: .*publisher" <<<"$("$CLI" --library "$d/lib" library info 1)"; check "a field changed by the bulk editor records bulk edit as its source" $?
    grep -q "^Sources: .*bulk edit: .*tags\|^Sources: .*bulk edit: .*publisher, tags" <<<"$("$CLI" --library "$d/lib" library info 2)"; check "also on the other book, with the tags" $?
    [ "$(grep -c "(undone)" <<<"$("$CLI" --library "$d/lib" library history)")" -eq 0 ]; check "the edits are in the operation log" $?'

scenario meta_editor_provenance 'edit:1,wait:3000,editor:meta-apply=publisher:Editor Press|series:Editor Saga,wait:700,quit' '
    grep -q "^Sources: .*editor: .*publisher" <<<"$("$CLI" --library "$d/lib" library info 1)"; check "applying the editor Metadata tab records editor as the source of the changed fields" $?
    grep -q "^Sources: .*editor: .*series" <<<"$("$CLI" --library "$d/lib" library info 1)"; check "for each changed field" $?'


# ---- saved searches / virtual libraries, online catalogues, language ----
scenario saved_searches 'library:saved=add=Mine|author:test,library:saved=add=Others|author:other,library:saved=add=Both|search:Mine OR search:Others,library:saved=add=Loop|search:Loop,library:saved=dump,library:search=search:Mine,wait:700,library:visible,library:search=search:Both,wait:700,library:visible,library:search=-search:Mine,wait:700,library:visible,library:search=,library:saved=vl=Others,wait:700,library:visible,library:saved=dump,library:saved=vl-off,wait:600,library:visible,library:saved=vl=Others,library:saved=delete=Others,wait:300,library:saved=dump,library:saved=open,wait:600,quit' '
    [ "$(grep -c "^saved: " <<<"$OUT")" -eq 8 ]; check "three searches are saved (a circular one is refused), and two are left after the delete" $?
    has "^saved_error: .*circle"; check "a search that refers to itself is refused" $?
    V=$(grep "^visible:" <<<"$OUT")
    sed -n 1p <<<"$V" | grep -q "^visible: 1 "; check "search:Mine finds the one book" $?
    sed -n 2p <<<"$V" | grep -q "^visible: 2 "; check "a saved search made of two others (OR) finds both" $?
    sed -n 3p <<<"$V" | grep -q "^visible: 1 "; check "-search:Mine excludes it" $?
    sed -n 4p <<<"$V" | grep -q "^visible: 1 "; check "the virtual library narrows the whole library" $?
    grep -q "1 of 2 books · Others" <<<"$OUT"; check "the window shows which virtual library is active" $?
    sed -n 5p <<<"$V" | grep -q "^visible: 2 "; check "switching it off shows everything again" $?
    has "^saved_vl: Others"; check "the virtual library is remembered in the settings" $?
    ! grep -q "^saved_vl: Others" <<<"$(grep "^saved_vl:" <<<"$OUT" | tail -1)"; check "deleting a search that is the virtual library switches it off" $?
    grep -q "\"saved_searches\"" "$d/cfg/bookx/config.json"; check "searches are stored in the settings file" $?'

scenario opds_browse 'library:opds=open,wait:500,library:opds=dump,library:opds=add-catalog=Mock|https://mock.example/opds/root.xml|u|p,library:opds=add-catalog=Bad|ftp://x/y,library:opds=dump,library:opds=catalog=Mock,wait:1200,library:opds=dump,library:opds=go=Fiction,wait:1200,library:opds=dump,library:opds=more,wait:1200,library:opds=dump,library:opds=add=Mock Tale,wait:3000,library:opds=back,wait:1200,library:opds=back,wait:1200,library:opds=dump,library:opds=catalog=Mock,wait:1200,library:opds=search=mock,wait:1500,library:opds=dump,quit' '
    D=$(grep "^opds_view:" <<<"$OUT")
    has "^opds_catalogs: Project Gutenberg;Standard Ebooks$"; check "the two built-in catalogues are listed first" $?
    has "^opds_catalogs: Project Gutenberg;Standard Ebooks;Mock$"; check "an added catalogue is listed" $?
    has "^opds_error: not added"; check "an address that is not http(s) is refused" $?
    sed -n 3p <<<"$D" | grep -q "title=Mock .*back=true .*search=true .*rows=Fiction|folder"; check "opening a catalogue shows its folders, a back button and search" $?
    sed -n 4p <<<"$D" | grep -q "rows=Mock Tale|book"; check "a folder lists books" $?
    sed -n 5p <<<"$D" | grep -q "rows=Mock Tale|book;Second Tale|book"; check "Load more appends the next page" $?
    grep -q "Mocked Tale" <<<"$LIB"; check "Add downloads the book and imports it into the library" $?
    sed -n 6p <<<"$D" | grep -q "title=Online Catalogues .*back=false"; check "Back from the first page returns to the list of catalogues" $?
    sed -n 7p <<<"$D" | grep -q "title=Search: mock .*rows=Found Tale|book"; check "a search runs through the catalogue search link" $?
    grep -q "\"opds_catalogs\"" "$d/cfg/bookx/config.json"; check "the catalogue is stored in the settings file" $?'

scenario opds_errors 'library:opds=open,wait:400,library:opds=add-catalog=Broken|https://mock.example/opds/missing.xml,library:opds=catalog=Broken,wait:1500,library:opds=dump,library:opds=back,wait:400,library:opds=dump,quit' '
    D=$(grep "^opds_view:" <<<"$OUT")
    sed -n 1p <<<"$D" | grep -q "page=status"; check "a catalogue that cannot be loaded shows an explanation instead of an empty list" $?
    sed -n 2p <<<"$D" | grep -q "back=false"; check "Back still returns to the catalogue list" $?'

scenario language 'library:language=es,library:menus-tr,library:opds=open,wait:500,library:opds=dump,library:language=xx,library:language=de,library:menus-tr,quit' '
    has "^language: es \| es"; check "Spanish is chosen" $?
    has "^mainmenu_tr: .*Catálogos en línea"; check "the main menu entries are translated" $?
    has "^opds_view: title=Catálogos en línea"; check "a window opened after the change uses the language" $?
    has "^language: xx \| en"; check "an unknown language falls back to English" $?
    has "^mainmenu_tr: .*Online-Kataloge"; check "German too" $?
    grep -q "\"language\": \"de\"" "$d/cfg/bookx/config.json"; check "the choice is stored in the settings file" $?'

# editor preview sync, splitting, compare view, Live CSS (own file to keep this one short)
. "$HERE/ui-tests-editor-preview.sh"
. "$HERE/ui-tests-pdf.sh"
. "$HERE/ui-tests-editor-tools.sh"
. "$HERE/ui-tests-ed-xl.sh"
. "$HERE/ui-tests-editor-struct.sh"
. "$HERE/ui-tests-editor-prefs.sh"
. "$HERE/ui-tests-editor-code.sh"
. "$HERE/ui-tests-ux-gaps.sh"
. "$HERE/ui-tests-reader-ux.sh"
. "$HERE/ui-tests-dictionaries.sh"
. "$HERE/ui-tests-pain.sh"
. "$HERE/ui-tests-i18n.sh"


# an EPUB with a footnote (aside, epub:type) and a link to a heading
FN="$ROOT/footnote-epub"
mkdir -p "$FN/src/META-INF" "$FN/src/OEBPS" "$FN/lib"
printf 'application/epub+zip' >"$FN/src/mimetype"
cp "$EP/src/META-INF/container.xml" "$FN/src/META-INF/"
cat >"$FN/src/OEBPS/content.opf" <<'XML'
<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">fn-test</dc:identifier><dc:title>Footnoted Book</dc:title><dc:creator>Note Author</dc:creator><dc:language>en</dc:language></metadata><manifest><item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c1"/></spine></package>
XML
{
    printf '<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>c</title></head><body><h1>One</h1>'
    printf '<p>The sentence carries a note<a epub:type="noteref" href="c1.xhtml#n1">1</a> and then goes on, with <a href="c1.xhtml#part2">a link to part two</a>.</p>'
    for i in $(seq 1 40); do printf '<p>Filler paragraph %s with enough words to make the chapter longer than one page of the reader.</p>' "$i"; done
    printf '<h2 id="part2">Part Two</h2><p>The second part.</p>'
    printf '<aside epub:type="footnote" id="n1"><p>1. The footnote says something short. <a href="c1.xhtml">↩</a></p></aside>'
    printf '</body></html>'
} >"$FN/src/OEBPS/c1.xhtml"
(cd "$FN/src" && zip -q -X -0 "$FN/lib/footnote.epub" mimetype && zip -q -X -r "$FN/lib/footnote.epub" META-INF OEBPS)

scenario footnote_popup "library:import=$ROOT/footnote-epub/lib,wait:2500,read:3,wait:3500,reader:link=OEBPS/c1.xhtml#n1,wait:600,reader:dump,reader:link=OEBPS/c1.xhtml#part2,wait:800,reader:dump,reader:footnotes-off,reader:link=OEBPS/c1.xhtml#n1,wait:800,reader:dump,quit" '
    F=$(grep "^footnote_popup:" <<<"$OUT")
    sed -n 1p <<<"$F" | grep -q "visible=true text=The footnote says something short\.$"; check "a link to a footnote shows the note in a popup, without its number and back arrow" $?
    sed -n 2p <<<"$F" | grep -q "none"; check "a link to a heading is followed instead, and the popup goes away" $?
    sed -n 3p <<<"$F" | grep -q "none"; check "with footnote popups switched off the link is followed" $?'

scenario speed_reading 'read:1,wait:3000,reader:rsvp,wait:600,reader:rsvp-dump,reader:rsvp-play,wait:1500,reader:rsvp-pause,reader:rsvp-faster,reader:rsvp-step=5,reader:rsvp-back,reader:rsvp-close,wait:500,reader:rsvp-dump,quit' '
    R=$(grep "^rsvp:" <<<"$OUT")
    sed -n 1p <<<"$R" | grep -q "playing=false wpm=300 "; check "speed reading starts paused at 300 words per minute" $?
    sed -n 1p <<<"$R" | grep -q "word=Chapter\|word=Paragraph"; check "it starts with the words of the open page" $?
    sed -n 1p <<<"$R" | grep -q "card=1"; check "the card is shown" $?
    SZ=$(sed -n 1p <<<"$R" | sed -E "s/.*size=([0-9]+)x([0-9]+).*/\1 \2/"); [ "${SZ% *}" -le 360 ] && [ "${SZ#* }" -le 120 ]; check "the card is at most 360 x 120 px (is $SZ)" $?
    sed -n 1p <<<"$R" | grep -q "marks=Some(((" ; check "the word and its focus letter are marked on the page itself" $?
    sed -n 2p <<<"$R" | grep -q "playing=true"; check "play starts the words" $?
    I=$(sed -n 4p <<<"$R" | sed -E "s/.* index=([0-9]+) .*/\1/")
    [ "${I:-0}" -ge 2 ]; check "words advance by themselves while playing" $?
    sed -n 4p <<<"$R" | grep -q "playing=false wpm=325"; check "faster adds one step (25) and pausing stops it" $?
    grep -q "\"rsvp_wpm\": 325" "$d/cfg/bookx/config.json"; check "the speed is remembered for next time" $?
    sed -n 7p <<<"$R" | grep -q "closed"; check "the X button ends speed reading" $?
    sed -n 8p <<<"$R" | grep -q "closed"; check "and it stays closed" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario speed_reading_page 'read:1,wait:3000,reader:rsvp,wait:300,reader:rsvp-dump,reader:rsvp-step=100,reader:rsvp-step=100,reader:rsvp-step=100,reader:rsvp-next,reader:rsvp-drag:0.2:0.3,reader:rsvp-drag:0.5:0.8,reader:rsvp-close,quit' '
    R=$(grep "^rsvp:" <<<"$OUT")
    at() { sed -n "$1p" <<<"$R" | sed -E "s/.*page_start=(-?[0-9]+).*/\1/"; }
    [ "$(at 4)" -gt "$(at 1)" ]; check "the page turns by itself to keep the word in view" $?
    sed -n 5p <<<"$R" | grep -q "marks=Some(((" ; check "and the word is marked on the new page" $?
    sed -n 6p <<<"$R" | grep -q "pos=(0.20,0.30)"; check "the card can be placed anywhere" $?
    sed -n 7p <<<"$R" | grep -q "pos=(0.50,0.80)"; check "and put back" $?'

scenario speed_reading_keys 'read:1,wait:3000,reader:rsvp,wait:400,reader:rsvp-key:space,wait:700,reader:rsvp-key:up,reader:rsvp-key:space,reader:rsvp-key:right,reader:rsvp-key:escape,reader:rsvp-dump,quit' '
    R=$(grep "^rsvp:" <<<"$OUT")
    sed -n 1p <<<"$R" | grep -q "playing=true"; check "space starts" $?
    sed -n 2p <<<"$R" | grep -q "wpm=325"; check "up is faster" $?
    sed -n 3p <<<"$R" | grep -q "playing=false"; check "space pauses" $?
    I=$(sed -n 4p <<<"$R" | sed -E "s/.* index=([0-9]+) .*/\1/"); J=$(sed -n 3p <<<"$R" | sed -E "s/.* index=([0-9]+) .*/\1/")
    [ "$I" -eq $((J + 1)) ]; check "right steps one word" $?
    sed -n 5p <<<"$R" | grep -q "closed"; check "Escape closes it" $?'

scenario speed_reading_vs_read_aloud 'read:1,wait:3000,reader:rsvp,wait:400,reader:rsvp-play,wait:400,reader:tts-play,wait:600,reader:rsvp-dump,reader:tts-dump,reader:rsvp-play,wait:400,reader:tts-dump,reader:rsvp-dump,reader:rsvp-close,reader:tts-dump,quit' '
    R=$(grep "^rsvp:" <<<"$OUT"); T=$(grep "^tts: " <<<"$OUT")
    sed -n 2p <<<"$R" | grep -q "card=1" && sed -n 2p <<<"$R" | grep -q "playing=false"; check "starting read aloud pauses speed reading and leaves its card open" $?
    sed -n 1p <<<"$T" | grep -q "state=Playing"; check "and read aloud plays" $?
    sed -n 3p <<<"$R" | grep -q "playing=true"; check "speed reading plays again when started" $?
    sed -n 2p <<<"$T" | grep -q "state=Paused"; check "and that pauses read aloud, which keeps its place" $?
    I1=$(sed -n 2p <<<"$R" | sed -E "s/.* index=([0-9]+) .*/\1/"); I2=$(sed -n 4p <<<"$R" | sed -E "s/.* index=([0-9]+) .*/\1/")
    [ "$I2" -gt "$I1" ]; check "speed reading went on from where it had been paused" $?
    R1=$(sed -n 1p <<<"$T" | sed -E "s/.*range=Some\(\(([0-9]+),.*/\1/"); R2=$(sed -n 2p <<<"$T" | sed -E "s/.*range=Some\(\(([0-9]+),.*/\1/")
    [ -n "$R2" ] && [ "$R2" -ge "$R1" ]; check "read aloud did not lose its place" $?
    sed -n 3p <<<"$T" | grep -q "state=Paused"; check "closing speed reading leaves read aloud as it was" $?'

scenario reading_goals 'stats:goal:1:2,read:1,wait:3500,reader:next,wait:500,reader:next,wait:500,stats:skew:60,reader:next,wait:500,close:,wait:1500,stats:goal:1:2,stats:show,wait:800,stats:dump,stats:goal:0:0,quit' '
    has "^stats_goal_daily: Goal reached: [0-9]+ of 1 min today"; check "a daily goal that is met says so" $?
    has "^stats_goal_yearly: 0 of 2 books this year"; check "a yearly goal shows the count against the target" $?
    [ "$(grep -c "^stats_goal_daily: $" <<<"$OUT")" -ge 1 ]; check "with the goals set to 0 there is nothing to show" $?
    has "^stats_review: Days you read = 1"; check "the yearly review counts the day read" $?
    has "^stats_review: Pages turned = [1-9]"; check "and the pages turned" $?
    grep -q "\"daily_minutes\": 0" "$d/cfg/bookx/config.json"; check "goals are stored in the settings file" $?'

scenario mcp_settings_page 'prefs:,wait:900,prefs:pick=AI Assistants (MCP),wait:300,prefs:dump,mcp:dump,a11y-audit:,mcp:snippet=claude-code,mcp:snippet=opencode,mcp:snippet=opencode:http,mcp:copied,mcp:port=@MCPPORT@,mcp:enable=on,wait:500,mcp:dump,mcp:ping,wait:200,mcp:dump,mcp:token=regen,wait:500,mcp:ping,mcp:write=on,wait:300,mcp:ping,mcp:dump,mcp:enable=off,wait:300,mcp:ping,mcp:dump,quit' '
    P=$(grep "^prefs:" <<<"$OUT" | head -1)
    grep -q "sections=.*AI Assistants (MCP)" <<<"$P" && grep -q "selected=AI Assistants (MCP)" <<<"$P"; check "Preferences has an AI Assistants (MCP) section" $?
    M=$(grep "^mcp: " <<<"$OUT")
    sed -n 1p <<<"$M" | grep -q "running=0 .*enabled=0 .*write=0"; check "the server is off and read-only by default" $?
    ! has "^a11y-problem:"; check "every control of the page has an accessible name" $?
    S=$(grep "^mcp_snippet:" <<<"$OUT")
    sed -n 1p <<<"$S" | grep -q "first=\"claude mcp add bookx -- "; check "the Claude Code snippet is a claude mcp add command" $?
    sed -n 2p <<<"$S" | grep -q "has_token=0 .*json=1"; check "the opencode command snippet is JSON without a token" $?
    sed -n 3p <<<"$S" | grep -q "json=1" && sed -n 3p <<<"$S" | grep -q "has_token=1" && sed -n 3p <<<"$S" | grep -q "has_url=1"; check "the server snippet has the address and the token" $?
    grep -q "^mcp_copied: .*has_token=1" <<<"$OUT"; check "copying puts the snippet on the clipboard" $?
    sed -n 2p <<<"$M" | grep -q "running=1 url=http://127.0.0.1:[0-9]*/mcp enabled=1" && sed -n 2p <<<"$M" | grep -q "status=\"Listening\""; check "switching it on starts the server and shows its address" $?
    G=$(grep "^mcp_ping:" <<<"$OUT")
    sed -n 1p <<<"$G" | grep -q "init=200 search=200 total=[1-9][0-9]* no_token=401 write=no-such-tool"; check "a real assistant request works with the token, is refused without it, and has no write tools" $?
    sed -n 3p <<<"$M" | grep -q "clients=\".*ui-test-assistant 1.0: [1-9]"; check "the page lists the connected assistant" $?
    grep -q "^mcp_token: changed=1 len=64" <<<"$OUT"; check "a new token can be made" $?
    sed -n 2p <<<"$G" | grep -q "init=200 search=200"; check "the server restarts with the new token" $?
    sed -n 3p <<<"$G" | grep -q "write=done"; check "with changes allowed the assistant can tag a book" $?
    sed -n 4p <<<"$M" | grep -q "write=1"; check "the write switch is remembered" $?
    "$CLI" --library "$d/lib" search "tag:from-assistant" 2>/dev/null | grep -q .; check "the change reached the library" $?
    sed -n 4p <<<"$G" | grep -q "^mcp_ping: off"; check "after switching off nothing is listening" $?
    ! grep "^mcp: " <<<"$OUT" | grep -q "leaked=1"; check "the token is never shown in full" $?
    grep -q "\"enabled\": false" "$d/cfg/bookx/mcp.json" && ! grep -Eq "[0-9a-f]{40}" "$d/cfg/bookx/mcp.json"; check "the settings are saved without the token" $?
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "the page runs without errors or GTK warnings" $?'

START=$(date +%s)
wait
echo "(ran up to $JOBS scenarios at once)"
PASS=0
FAIL=0
for s in "${SCENARIOS[@]}"; do
    while IFS= read -r line; do
        case $line in
        ok\|*) PASS=$((PASS + 1)); IFS='|' read -r _ n d <<<"$line"; printf '  ok    %-24s %s\n' "$n" "$d" ;;
        FAIL\|*) FAIL=$((FAIL + 1)); IFS='|' read -r _ n d <<<"$line"; printf '  FAIL  %-24s %s\n' "$n" "$d" ;;
        *) echo "$line" ;;
        esac
    done <"$RES/$s"
done
echo
echo "passed: $PASS, failed: $FAIL  (${#SCENARIOS[@]} scenarios in parallel, $(($(date +%s) - START))s after start-up)"
[ "$FAIL" = 0 ]
