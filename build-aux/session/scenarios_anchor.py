"""Highlights are painted where their text is (docs/bugs.md, bug 11). A library as an older Bookx left it holds a
highlight of a public-domain book whose stored position is wrong (the numbering of the export text, a few hundred
characters late, as the device import used to write it): opening the library repairs it, and the real reader paints
the highlight over exactly the words it holds. Screenshots: <results>/highlight_import_anchor-*.png.
Imported by scenarios.py."""
import html
import json
import os
import re
import sqlite3
import time
import zipfile

from harness import scenario


def _chapters(epub):
    """Chapter texts of the book in spine order: the paragraphs, tags removed, white space collapsed."""
    z = zipfile.ZipFile(epub)
    opf = next(n for n in z.namelist() if n.endswith(".opf"))
    x = z.read(opf).decode("utf8", "replace")
    base = opf.rsplit("/", 1)[0] + "/" if "/" in opf else ""
    ids = {}
    for tag in re.findall(r"<item\b[^>]*>", x):
        i, h = re.search(r'\bid="([^"]+)"', tag), re.search(r'\bhref="([^"]+)"', tag)
        if i and h:
            ids[i.group(1)] = h.group(1)
    out = []
    for idref in re.findall(r'<itemref\b[^>]*\bidref="([^"]+)"', x):
        src = z.read(base + ids[idref]).decode("utf8", "replace")
        paras = [html.unescape(re.sub(r"<[^>]+>", "", p)) for p in re.findall(r"<p\b[^>]*>(.*?)</p>", src, re.S)]
        out.append([" ".join(p.split()) for p in paras if p.strip()])
    return out


def _seed(c):
    """Alice with one highlight stored in the old, wrong numbering, and the reading position at its paragraph."""
    chapters = _chapters(c.books["alice"])
    spine, paras = max(enumerate(chapters), key=lambda kv: len(kv[1]) if len(kv[1]) < 400 else 0)
    k = min(len(paras) - 1, 12)
    while k < len(paras) - 1 and len(paras[k]) < 160:
        k += 1
    quote = paras[k][:150].rsplit(" ", 1)[0]
    reader_start = sum(len(p) + 1 for p in paras[:k])
    old_start = reader_start + 2 * k  # one extra line break per block before it: the export numbering
    c.quote = quote
    c.spine = spine
    con = sqlite3.connect(os.path.join(c.dir, "lib", "bookx-library.db"))
    (data,) = con.execute("select data from books order by id limit 1").fetchone()
    d = json.loads(data)
    d["highlights"] = [{
        "id": "bx-session-shifted", "spine": spine, "start": old_start, "end": old_start + len(quote),
        "color": "yellow", "style": "", "note": "", "text": quote, "created": "2026-08-17T11:22:15Z",
    }]
    d["position"] = {"spine": spine, "frac": max(0.0, (reader_start - 40) / max(1, sum(len(p) + 1 for p in paras)))}
    con.execute("update books set data=? where id=(select min(id) from books)", (json.dumps(d),))
    con.commit()
    con.close()


def _yellow(r, g, b):
    return r > 200 and g > 180 and b < 150 and r - b > 80


@scenario
def highlight_import_anchor(c):
    c.start(seed=_seed)
    time.sleep(2.5)
    c.check("the library lists the book", c.find("table cell", rx="Alice") or c.find("button", rx="Alice"))
    r = c.open_book()
    c.check("the book opens", r)
    time.sleep(2.0)
    img = c.image("page")
    box = (0, 80, img.width, img.height - 60)
    n = c.count_pixels(img, box, _yellow)
    c.check("the highlight whose position was wrong is painted on the page where the book was left open", n > 600,
            f"{n} yellow px")
    # the painted words: crop to the yellow rows and read them
    px = img.load()
    rows = [y for y in range(box[1], box[3]) if sum(1 for x in range(0, img.width, 3) if _yellow(*px[x, y])) > 4]
    if rows and os.path.exists("/usr/bin/tesseract"):
        crop = (0, max(0, rows[0] - 6), img.width, min(img.height, rows[-1] + 6))
        text = " ".join((c.ocr(img, crop) or "").split()).lower()
        words = c.quote.lower().split()[:3]
        c.check("the words under the colour are the first words of the highlight's text", all(w.strip(",.;:!?'\"“”‘’") in text for w in words),
                f"{text[:80]!r} vs {c.quote[:40]!r}")
    c.shot("painted")


def _seed_colors(c):
    """Alice with highlights of four colours (and an underline) on four paragraphs of one page, as a device import
    stores them now."""
    chapters = _chapters(c.books["alice"])
    spine, paras = max(enumerate(chapters), key=lambda kv: len(kv[1]) if len(kv[1]) < 400 else 0)
    k = 10
    while len(paras[k]) < 90 or len(paras[k + 1]) < 90 or len(paras[k + 2]) < 90 or len(paras[k + 3]) < 90:
        k += 1
    looks = [("yellow", ""), ("pink", ""), ("blue", ""), ("green", "underline")]
    hs = []
    for i, (color, style) in enumerate(looks):
        start = sum(len(p) + 1 for p in paras[:k + i])
        quote = paras[k + i][:80].rsplit(" ", 1)[0]
        hs.append({"id": f"bx-color-{i}", "spine": spine, "start": start, "end": start + len(quote), "color": color,
                   "style": style, "note": "", "text": quote, "created": "2026-08-17T11:22:15Z"})
    total = sum(len(p) + 1 for p in paras)
    con = sqlite3.connect(os.path.join(c.dir, "lib", "bookx-library.db"))
    (data,) = con.execute("select data from books order by id limit 1").fetchone()
    d = json.loads(data)
    d["highlights"] = hs
    d["position"] = {"spine": spine, "frac": max(0.0, (sum(len(p) + 1 for p in paras[:k]) - 40) / max(1, total))}
    con.execute("update books set data=? where id=(select min(id) from books)", (json.dumps(d),))
    con.commit()
    con.close()


@scenario
def highlight_colors_painted(c):
    c.start(seed=_seed_colors)
    time.sleep(2.5)
    r = c.open_book()
    c.check("the book opens", r)
    time.sleep(2.0)
    img = c.image("page")
    box = (0, 80, img.width, img.height - 60)
    kinds = {
        "yellow": lambda r, g, b: r > 230 and g > 190 and b < 140,
        "pink": lambda r, g, b: r > 220 and 130 < g < 190 and 150 < b < 230 and r - g > 40,
        "blue": lambda r, g, b: b > 220 and r < 150 and 150 < g < 200,
        "green": lambda r, g, b: g > 180 and r < 120 and b < 150,
    }
    for name, pred in kinds.items():
        n = c.count_pixels(img, box, pred)
        c.check(f"a {name} highlight is painted in {name}", n > 150, f"{n} px")
    c.shot("colors")
