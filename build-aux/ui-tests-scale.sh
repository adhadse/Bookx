# Scenarios of the Calibre export (PP-C-18) and the large-library scale checks (PP-C-11, PP-G-12). Sourced by
# ui-tests.sh (uses its `scenario`, `check`, `has`, $ROOT, $CLI). Dev commands: `library:export-calibre=DIR` (the
# Library menu entry without the folder dialog), `library:requery-time`, `library:grid-bench=FRAMES` (real frame clock only, see the end of this file).

# make_scale_calibre DIR N: a Calibre library folder with N books (EPUB rows only, no files) for the importer
make_scale_calibre() {
    mkdir -p "$1"
    python3 - "$1" "$2" <<'PY'
import sqlite3, sys, random
d, n = sys.argv[1], int(sys.argv[2])
random.seed(7)
c = sqlite3.connect(d + "/metadata.db")
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
""")
syl = "ba ko ri mu sen tal dor vi lan pe ru mar ni sol ka the gro fan wel zi".split()
w = lambda: "".join(random.choice(syl) for _ in range(random.randint(2, 3))).capitalize()
authors = [f"{w()} {w()}" for _ in range(max(n // 8, 1))]
c.executemany("INSERT INTO authors VALUES(?,?)", [(i + 1, a) for i, a in enumerate(authors)])
for i in range(1, n + 1):
    t = f"{w()} {w()} {i}"
    a = random.randrange(len(authors))
    c.execute("INSERT INTO books VALUES(?,?,?,?,?,?,?,?,?,0)", (i, t, t, "2020-01-01 00:00:00+00:00", "2020-01-01 00:00:00+00:00", "0101-01-01 00:00:00+00:00", 1.0, authors[a], f"x/{i}"))
    c.execute("INSERT INTO data VALUES(?,?,?,?,?)", (i, i, "EPUB", 1000, f"b{i}"))
    c.execute("INSERT INTO books_authors_link VALUES(?,?,?)", (i, i, a + 1))
c.commit()
PY
}

scenario calibre_export 'library:menus,library:export-calibre=@D@/calout,wait:3000,library:export-calibre=@D@/calout,wait:1000,quit' '
    has "^mainmenu: .*Import Calibre Library \\[lib.calibre\\].*Export as Calibre Library \\[lib.export-calibre\\]"; check "Export as Calibre Library is in the main menu next to the import" $?
    has "^calibre_export: books=2 files=2 covers=[0-9] problems=0 cancelled=false"; check "both books of the library were exported as a background job" $?
    has "^calibre_export: refused .*not empty"; check "a second export into the same folder is refused" $?
    python3 - "$d/calout" <<PY
import sqlite3, sys, os
d = sys.argv[1]
c = sqlite3.connect(d + "/metadata.db")
assert c.execute("pragma integrity_check").fetchone()[0] == "ok"
assert c.execute("pragma user_version").fetchone()[0] == 27
assert c.execute("select count(*) from books").fetchone()[0] == 2
titles = sorted(r[0] for r in c.execute("select title from books"))
assert titles == ["Second Book", "UI Test Book"], titles
for path, name in c.execute("select b.path, d.name from books b join data d on d.book = b.id"):
    assert os.path.getsize(f"{d}/{path}/{name}.epub") > 100, path
    assert os.path.isfile(f"{d}/{path}/metadata.opf")
PY
    check "metadata.db is a valid Calibre database (version 27) with both books, their files and metadata.opf" $?
    if command -v calibredb >/dev/null; then
        # the single-instance lock of calibre is an abstract unix socket named after the user, so a Calibre the user has running
        # blocks calibredb. Run it in its own user+network namespace (own config/cache/temp too); without unshare, a refusal
        # because of a running Calibre skips the check instead of failing it.
        mkdir -p "$d/calibre-config" "$d/calibre-cache" "$d/calibre-tmp"
        NS=""
        unshare -Urn true 2>/dev/null && NS="unshare -Urn"
        CDB=$(HOME=$d CALIBRE_CONFIG_DIRECTORY="$d/calibre-config" CALIBRE_CACHE_DIRECTORY="$d/calibre-cache" CALIBRE_TEMP_DIR="$d/calibre-tmp" \
            QT_QPA_PLATFORM=offscreen $NS calibredb list --with-library "$d/calout" --for-machine --fields title 2>&1)
        if [ -z "$NS" ] && grep -q "Another calibre program" <<<"$CDB"; then
            echo "  skip  calibre_export           calibredb: a Calibre is running and unshare is unavailable"
        else
            grep -q "UI Test Book" <<<"$CDB"
            check "calibredb opens the exported library and lists the books" $?
        fi
    fi
    ! grep -q "calout" <<<"$(find "$d/lib" -maxdepth 3 2>/dev/null)"; check "nothing was written into the Bookx library" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario scale_grid_20k 'wait:2500,library:grid,wait:1500,library:requery-time,library:requery-time,quit' '
    has "^requery_time: books=20002 "; check "20 000 books are in the grid and table" $?
    R=$(grep "^requery_time:" <<<"$OUT" | tail -1 | sed -E "s/.*ms=([0-9]+).*/\\1/")
    [ -n "$R" ] && [ "$R" -le "$((${BOOKX_PERF_SCALE:-1} * 8000))" ]; check "rebuilding the grid model of 20 000 books stays within budget (${R}ms, debug build)" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'
# 2000 books: sorting, searching and clearing the search fill the grid in idle pieces; `library:stalls` measures the
# longest block of the main loop in each (BOOKX_STALL_WATCH also reports them as `stall: N ms after ...`)
scenario grid_stalls_2000 'wait:2500,library:grid,wait:1500,library:fillstate,library:stalls,wait:8000,library:fillstate,library:fillselect,wait:3000,library:selection,library:fillstate,library:table,wait:7000,library:fillstate,quit' '
    has "^fillstate: tiles=[0-9]+ view=[0-9x]+ adj=[^ ]+ grid=(2000|2002) table=0 filling=false books=(2000|2002)"; check "all 2000 books are in the grid after a rebuild in pieces (the table stays empty in grid view)" $?
    S=$(grep "^stalls:" <<<"$OUT" | tail -1)
    has "^stalls: sort=[0-9]+ search=[0-9]+ clear=[0-9]+ sortback=[0-9]+ max=[0-9]+"; check "sort, search and clear were measured: $S" $?
    M=$(sed -E "s/.*max=([0-9]+).*/\1/" <<<"$S")
    [ -n "$M" ] && [ "$M" -le "$((${BOOKX_PERF_SCALE:-1} * 400))" ]; check "no sort, search or clear blocks the main loop for more than ${BOOKX_PERF_SCALE:-1}x400 ms (${M} ms, debug build)" $?
    has "^fillselect: filling=true loaded=[0-9]+ books=(2000|2002)"; check "select all while the list is still arriving finds it unfinished" $?
    has "^selection: .* model=(2000|2002) "; check "select all during the fill selects all 2000 books, not only the ones that had arrived" $?
    has "^fillstate: tiles=[0-9]+ view=[0-9x]+ adj=[^ ]+ grid=0 table=(2000|2002) filling=false books=(2000|2002)"; check "the table view fills the table (and only the table) with all the books" $?
    T=$(grep "^fillstate:" <<<"$OUT" | sed -n 1p | sed -E "s/^fillstate: tiles=([0-9]+) .*/\\1/")
    [ -n "$T" ] && [ "$T" -lt 700 ]; check "the grid keeps fewer than 700 tiles alive, it was 1281 at 40 columns (${T})" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'
# Scrolling is measured in the real session (build-aux/session/scenarios_scale.py, library_scroll_20k): Broadway's frame clock
# does not tick without a browser, so `library:grid-bench=N` prints nothing there.
