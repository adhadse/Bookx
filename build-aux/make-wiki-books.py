#!/usr/bin/env python3
"""Makes a shelf of small EPUBs with generated covers for the wiki screenshots (never committed, always generated).

  make-wiki-books.py OUTDIR

The titles are public-domain works; the covers are plain colour designs drawn here (no real cover art), the text is a
few paragraphs of filler so every book has a chapter to open.
"""
import io
import os
import sys
import zipfile

from PIL import Image, ImageDraw, ImageFont

BOOKS = [
    ("Moby-Dick", "Herman Melville", "Adventure", (30, 90, 160), "#f5f0e1"),
    ("Pride and Prejudice", "Jane Austen", "Romance", (150, 50, 90), "#fbeef3"),
    ("Frankenstein", "Mary Shelley", "Gothic", (45, 60, 55), "#dfeee8"),
    ("Dracula", "Bram Stoker", "Gothic", (120, 20, 30), "#f7e3e3"),
    ("The Time Machine", "H. G. Wells", "Science fiction", (20, 110, 120), "#e0f4f5"),
    ("Treasure Island", "R. L. Stevenson", "Adventure", (190, 120, 30), "#fff4de"),
    ("Jane Eyre", "Charlotte Bronte", "Classic", (90, 60, 130), "#efe8fa"),
    ("The Odyssey", "Homer", "Epic", (60, 110, 70), "#e6f3e8"),
    ("Walden", "Henry D. Thoreau", "Nature", (95, 125, 45), "#eef5df"),
    ("Great Expectations", "Charles Dickens", "Classic", (70, 70, 80), "#ececf1"),
    ("The Art of War", "Sun Tzu", "Philosophy", (170, 40, 40), "#fbe9e0"),
    ("Siddhartha", "Hermann Hesse", "Philosophy", (210, 150, 50), "#fdf1da"),
]

FONTS = ["/usr/share/fonts/dejavu-sans-fonts/DejaVuSans-Bold.ttf", "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
         "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"]


def font(size):
    for f in FONTS:
        if os.path.exists(f):
            return ImageFont.truetype(f, size)
    return ImageFont.load_default(size)


def wrap(draw, text, fnt, width):
    lines, cur = [], ""
    for w in text.split():
        t = (cur + " " + w).strip()
        if draw.textlength(t, font=fnt) <= width or not cur:
            cur = t
        else:
            lines.append(cur)
            cur = w
    return lines + [cur]


def cover(title, author, colour, ink):
    w, h = 600, 900
    img = Image.new("RGB", (w, h), colour)
    d = ImageDraw.Draw(img)
    for i in range(h):  # a soft vertical gradient
        k = i / h
        d.line([(0, i), (w, i)], fill=tuple(int(c * (1 - 0.35 * k)) for c in colour))
    d.rectangle((40, 40, w - 40, h - 40), outline=ink, width=3)
    y = 150
    for line in wrap(d, title.upper(), font(54), w - 140):
        d.text((w / 2, y), line, font=font(54), fill=ink, anchor="mt")
        y += 70
    d.line([(w / 2 - 60, y + 20), (w / 2 + 60, y + 20)], fill=ink, width=3)
    d.text((w / 2, h - 120), author, font=font(32), fill=ink, anchor="mt")
    buf = io.BytesIO()
    img.save(buf, "JPEG", quality=88)
    return buf.getvalue()


def make(path, n, title, author, subject, colour, ink):
    esc = lambda s: s.replace("&", "&amp;")
    paras = "".join(
        f"<p>Chapter {c} of {esc(title)}. A few quiet lines of filler text so the reader has something to show. "
        "The quick brown fox jumps over the lazy dog, and the story goes on page after page.</p>"
        for c in range(1, 30))
    files = {
        "mimetype": "application/epub+zip",
        "META-INF/container.xml": '<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">'
        '<rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>',
        "OEBPS/content.opf": f'''<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">
<metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
<dc:identifier id="id">urn:uuid:5a3c0000-0000-4000-8000-{n:012d}</dc:identifier>
<dc:title>{esc(title)}</dc:title><dc:creator>{esc(author)}</dc:creator><dc:language>en</dc:language>
<dc:subject>{subject}</dc:subject><meta property="dcterms:modified">2026-01-01T00:00:00Z</meta>
</metadata>
<manifest>
<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
<item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/>
<item id="cover" href="cover.jpg" media-type="image/jpeg" properties="cover-image"/>
</manifest>
<spine><itemref idref="c1"/></spine></package>''',
        "OEBPS/nav.xhtml": '<?xml version="1.0" encoding="UTF-8"?><html xmlns="http://www.w3.org/1999/xhtml" '
        'xmlns:epub="http://www.idpf.org/2007/ops"><head><title>Contents</title></head><body>'
        '<nav epub:type="toc"><ol><li><a href="c1.xhtml">Chapter 1</a></li></ol></nav></body></html>',
        "OEBPS/c1.xhtml": f'<?xml version="1.0" encoding="UTF-8"?><html xmlns="http://www.w3.org/1999/xhtml"><head>'
        f"<title>{esc(title)}</title></head><body><h1>{esc(title)}</h1>{paras}</body></html>",
    }
    with zipfile.ZipFile(path, "w") as z:
        z.writestr("mimetype", files.pop("mimetype"), zipfile.ZIP_STORED)
        for name, data in files.items():
            z.writestr(name, data, zipfile.ZIP_DEFLATED)
        z.writestr("OEBPS/cover.jpg", cover(title, author, colour, ink), zipfile.ZIP_STORED)


def main():
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    for n, (title, author, subject, colour, ink) in enumerate(BOOKS, 1):
        make(os.path.join(out, f"{n:02d}-{title.replace(' ', '-')}.epub"), n, title, author, subject, colour, ink)
    print(f"{len(BOOKS)} books in {out}")


if __name__ == "__main__":
    main()
