#!/usr/bin/env python3
"""Builds a big EPUB for the reader sidebar tests (never committed, always generated).

  make-big-epub.py OUT.epub [ENTRIES]

ENTRIES (default 2000) is about the number of entries in the table of contents: 20 parts, each with chapters and
every chapter with 9 sections, in a nested EPUB 3 navigation document. Every section has a few paragraphs.
"""
import sys
import zipfile

CONTAINER = """<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"""

out = sys.argv[1]
entries = int(sys.argv[2]) if len(sys.argv) > 2 else 2000
parts = 20
chapters = max(1, round(entries / parts / 10))  # a part is 1 entry, a chapter 1 + 9 sections
files = []  # (name, xhtml)
nav = []  # nested list html
count = 0
for p in range(parts):
    body = [f'<h1 id="p{p}">Part {p + 1}</h1>']
    sub = []
    for c in range(chapters):
        body.append(f'<h2 id="c{p}_{c}">Chapter {p + 1}.{c + 1}</h2>')
        subsub = []
        for s in range(9):
            body.append(f'<h3 id="s{p}_{c}_{s}">Section {p + 1}.{c + 1}.{s + 1}</h3>')
            for k in range(3):
                body.append(
                    f"<p>Paragraph {k + 1} of section {p + 1}.{c + 1}.{s + 1}. The reader opens this book to test "
                    "how fast the sidebar shows a long table of contents and a long list of highlights.</p>"
                )
            subsub.append(f'<li><a href="part{p}.xhtml#s{p}_{c}_{s}">Section {p + 1}.{c + 1}.{s + 1}</a></li>')
            count += 1
        sub.append(f'<li><a href="part{p}.xhtml#c{p}_{c}">Chapter {p + 1}.{c + 1}</a><ol>{"".join(subsub)}</ol></li>')
        count += 1
    count += 1
    nav.append(f'<li><a href="part{p}.xhtml">Part {p + 1}</a><ol>{"".join(sub)}</ol></li>')
    files.append(
        (
            f"part{p}.xhtml",
            '<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml"><head><title>Part</title></head><body>'
            + "".join(body)
            + "</body></html>",
        )
    )

manifest = "".join(f'<item id="f{i}" href="{n}" media-type="application/xhtml+xml"/>' for i, (n, _) in enumerate(files))
spine = "".join(f'<itemref idref="f{i}"/>' for i in range(len(files)))
opf = (
    '<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">'
    '<metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">big-test</dc:identifier>'
    "<dc:title>The Big Book</dc:title><dc:creator>Big Author</dc:creator><dc:language>en</dc:language></metadata>"
    f'<manifest><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>{manifest}</manifest>'
    f"<spine>{spine}</spine></package>"
)
navdoc = (
    '<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">'
    '<head><title>Contents</title></head><body><nav epub:type="toc"><ol>' + "".join(nav) + "</ol></nav></body></html>"
)
with zipfile.ZipFile(out, "w") as z:
    z.writestr("mimetype", "application/epub+zip", compress_type=zipfile.ZIP_STORED)
    z.writestr("META-INF/container.xml", CONTAINER)
    z.writestr("OEBPS/content.opf", opf)
    z.writestr("OEBPS/nav.xhtml", navdoc)
    for n, x in files:
        z.writestr("OEBPS/" + n, x)
print(count)
