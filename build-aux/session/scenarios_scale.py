"""A library of 20,000 books in a real GNOME session (PP-C-11, PP-G-12): the cover grid is scrolled by a frame
callback (real frame clock, `library:grid-bench`) and then with the real mouse wheel; the app must stay responsive.
Imported by scenarios.py."""
import os
import re
import sqlite3
import random
import subprocess
import time

from harness import scenario

N = 20000


def _seed(c):
    """20,000 books through the Calibre importer (catalogue rows only, no files)."""
    cal = os.path.join(c.dir, "scalecal")
    os.makedirs(cal, exist_ok=True)
    random.seed(7)
    db = sqlite3.connect(os.path.join(cal, "metadata.db"))
    db.executescript("""
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

    def w():
        return "".join(random.choice(syl) for _ in range(random.randint(2, 3))).capitalize()

    authors = [f"{w()} {w()}" for _ in range(N // 8)]
    db.executemany("INSERT INTO authors VALUES(?,?)", [(i + 1, a) for i, a in enumerate(authors)])
    for i in range(1, N + 1):
        t = f"{w()} {w()} {i}"
        a = random.randrange(len(authors))
        db.execute("INSERT INTO books VALUES(?,?,?,?,?,?,?,?,?,0)",
                   (i, t, t, "2020-01-01 00:00:00+00:00", "2020-01-01 00:00:00+00:00",
                    "0101-01-01 00:00:00+00:00", 1.0, authors[a], f"x/{i}"))
        db.execute("INSERT INTO data VALUES(?,?,?,?,?)", (i, i, "EPUB", 1000, f"b{i}"))
        db.execute("INSERT INTO books_authors_link VALUES(?,?,?)", (i, i, a + 1))
    db.commit()
    db.close()
    r = subprocess.run([c.cli, "library", "import-calibre", cal], capture_output=True, text=True, env=c.sb.env)
    if r.returncode != 0:
        raise RuntimeError(f"import-calibre: {r.stdout}{r.stderr}")


@scenario
def library_scroll_20k(c):
    script = "wait:3000,library:grid,wait:1500,library:requery-time,library:grid-bench=120,wait:8000"
    c.start(seed=_seed, extra_env={"BOOKX_DEV_SCRIPT": script})
    log = os.path.join(c.dir, "app.log")

    def said(rx):
        try:
            return re.search(rx, open(log, errors="replace").read())
        except OSError:
            return None

    if os.environ.get("PROF") == "1":  # stack samples of the app during the first slow frame (needs gdb)
        c.wait(lambda: said(r"requery_time: books=2000\d"), 60)
        for i in range(4):
            time.sleep(3)
            out = subprocess.run(["gdb", "-p", str(c.app.pid), "-batch", "-ex", "thread 1", "-ex", "bt 40"],
                                 capture_output=True, text=True).stdout
            open(os.path.join(c.dir, f"stack{i}.txt"), "w").write(out)
            os.makedirs(os.environ["SESSION_RESULTS"], exist_ok=True)
            open(os.path.join(os.environ["SESSION_RESULTS"], f"prof-stack{i}.txt"), "w").write(out)
    c.check("the library lists 20 000 books", c.wait(lambda: said(r"requery_time: books=2000\d"), 60))
    m = c.wait(lambda: said(r"grid_bench: frames=120 avg_ms=([\d.]+) max_ms=([\d.]+) wall_ms=(\d+) moved=(\d+)"), 40)
    m = said(r"grid_bench: frames=120 avg_ms=([\d.]+) max_ms=([\d.]+) wall_ms=(\d+) moved=(\d+)")
    c.check("the grid scrolled 120 frames with the real frame clock", m)
    if m:
        avg, mx, wall, moved = float(m.group(1)), float(m.group(2)), int(m.group(3)), int(m.group(4))
        c.note(f"frames: avg {avg} ms, worst {mx} ms, wall {wall} ms, scrolled {moved} px (debug build)")
        c.check("the scroll moved the grid", moved > 1000, str(moved))
        # the first frame lays out the 20 000 item model (about 0.6 s in a debug build; it was 17 s while every
        # bound tile set a tooltip text, which makes GTK pick the whole window)
        c.check("no frame took longer than 3 s while scrolling 20 000 books (debug build)", mx < 3000, f"{mx} ms")
        c.check("the average frame stays under 300 ms (debug build, a screenful of tiles rebound every frame)",
                avg < 300, f"{avg} ms")
    # the real wheel over the grid, then the app must answer within a second
    t = time.time()
    c.sb.scroll(40, 640, 420)
    took = time.time() - t
    t = time.time()
    alive = c.is_alive() and c.tree().find("frame")
    answer = time.time() - t
    c.note(f"40 wheel clicks took {took:.1f} s (50 ms apart by the harness); the accessibility tree answered in {answer:.2f} s")
    c.check("the app answers after scrolling with the wheel", alive and answer < 5, f"{answer:.2f} s")
    c.shot("scrolled")
