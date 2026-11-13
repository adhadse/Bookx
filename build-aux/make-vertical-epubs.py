#!/usr/bin/env python3
"""Builds the EPUB test books for vertical writing (never committed, always generated).

  make-vertical-epubs.py synthetic OUT.epub        a short Japanese book (vertical-rl, page-progression rtl)
                                                  with ruby, tate-chu-yoko, punctuation, Latin text,
                                                  a picture and a vertical-lr chapter in Chinese
                                                  and a horizontal chapter with ruby
  make-vertical-epubs.py synthetic-long OUT.epub   the same with a first chapter of about 100 000 characters (budgets)
  make-vertical-epubs.py aozora SRC.html OUT.epub  chapters 1-3 of 夏目漱石「吾輩は猫である」 (Aozora Bunko,
                                                  public domain, see docs/test-books.md) as vertical-rl EPUB

The Aozora file is Shift_JIS XHTML; its ruby (`<ruby><rb>..</rb><rp>..</rp><rt>..</rt></ruby>`) is kept as it is.
"""
import html
import re
import struct
import sys
import zlib
import zipfile

CONTAINER = """<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"""

CSS_RL = """html { -epub-writing-mode: vertical-rl; -webkit-writing-mode: vertical-rl; writing-mode: vertical-rl; }
body { margin: 0; }
h1, h2 { font-weight: bold; margin: 0; }
p { margin: 0; text-indent: 1em; }
.tcy { -epub-text-combine: horizontal; -webkit-text-combine: horizontal; text-combine-upright: all; }
.latin { text-orientation: mixed; }
.up { text-orientation: upright; }
"""
CSS_LR = """html { writing-mode: vertical-lr; }
body { margin: 0; }
p { margin: 0; text-indent: 2em; }
"""


CSS_PLAIN = "p { margin: 0 0 0.6em 0; }\n"


def png(w, h, rgb):
    raw = b"".join(b"\x00" + bytes(rgb) * w for _ in range(h))

    def chunk(t, d):
        c = struct.pack(">I", len(d)) + t + d
        return c + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)

    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


def xhtml(title, body, lang="ja", css="v.css"):
    return (f'<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml" xml:lang="{lang}" lang="{lang}">'
            f'<head><title>{html.escape(title)}</title><link rel="stylesheet" type="text/css" href="{css}"/></head>'
            f'<body>{body}</body></html>')


def write_epub(out, title, author, lang, chapters, css, ppd="rtl", extra=()):
    """chapters: [(file, title, xhtml)]; css: {name: text}; extra: [(name, bytes, media-type)]"""
    manifest = "".join(f'<item id="c{i}" href="{f}" media-type="application/xhtml+xml"/>' for i, (f, _, _) in enumerate(chapters))
    manifest += "".join(f'<item id="s{i}" href="{n}" media-type="text/css"/>' for i, n in enumerate(css))
    manifest += "".join(f'<item id="x{i}" href="{n}" media-type="{m}"/>' for i, (n, _, m) in enumerate(extra))
    manifest += '<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>'
    spine = "".join(f'<itemref idref="c{i}"/>' for i in range(len(chapters)))
    opf = (f'<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">'
           f'<metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">vertical-{zlib.crc32(title.encode()) % 10**6}</dc:identifier>'
           f'<dc:title>{html.escape(title)}</dc:title><dc:creator>{html.escape(author)}</dc:creator><dc:language>{lang}</dc:language>'
           f'<meta property="dcterms:modified">2024-01-01T00:00:00Z</meta></metadata>'
           f'<manifest>{manifest}</manifest><spine page-progression-direction="{ppd}">{spine}</spine></package>')
    nav = ('<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>nav</title></head>'
           '<body><nav epub:type="toc"><ol>' + "".join(f'<li><a href="{f}">{html.escape(t)}</a></li>' for f, t, _ in chapters) + "</ol></nav></body></html>")
    with zipfile.ZipFile(out, "w") as z:
        z.writestr(zipfile.ZipInfo("mimetype"), "application/epub+zip", compress_type=zipfile.ZIP_STORED)
        z.writestr("META-INF/container.xml", CONTAINER, compress_type=zipfile.ZIP_DEFLATED)
        z.writestr("OEBPS/content.opf", opf, compress_type=zipfile.ZIP_DEFLATED)
        z.writestr("OEBPS/nav.xhtml", nav, compress_type=zipfile.ZIP_DEFLATED)
        for f, _, x in chapters:
            z.writestr("OEBPS/" + f, x, compress_type=zipfile.ZIP_DEFLATED)
        for n, text in css.items():
            z.writestr("OEBPS/" + n, text, compress_type=zipfile.ZIP_DEFLATED)
        for n, data, _ in extra:
            z.writestr("OEBPS/" + n, data, compress_type=zipfile.ZIP_STORED)


def ruby(base, rt):
    return f"<ruby>{base}<rp>（</rp><rt>{rt}</rt><rp>）</rp></ruby>"


def synthetic(out, long=False):
    # original sentences written for this test, nothing copied from a book
    paras = [
        f"　{ruby('春', 'はる')}の{ruby('朝', 'あさ')}、{ruby('小', 'ちい')}さな{ruby('図書館', 'としょかん')}の{ruby('窓', 'まど')}から{ruby('光', 'ひかり')}が{ruby('差', 'さ')}し{ruby('込', 'こ')}んだ。",
        "　「今日は<span class=\"tcy\">12</span>月<span class=\"tcy\">24</span>日ですね。」と彼女は言った。「ええ、もう<span class=\"tcy\">2024</span>年も終わりです。」",
        "　駅までは歩いて<span class=\"tcy\">15</span>分。電車は<span class=\"tcy\">7</span>時<span class=\"tcy\">30</span>分に出る。（本当に間に合うだろうか。）彼は「「急げ」」と心の中で叫んだ。",
        "　その本には<span class=\"latin\">Hello, world</span>という英語の見出しがあり、<span class=\"latin\">Bookx</span>は縦書きでも横向きに表示される。",
        "　文字が<span class=\"up\">ABC</span>のように正立することもある。ー、ぁ、ゃ、っ、々、…… ―― といった約物は行頭に来てはならない。",
        "　長い文章は列の終わりで折り返され、禁則処理によって句読点や閉じ括弧が行頭に来ないように、前の文字が次の列へ追い出される。" * 3,
    ]
    pic = "<p style=\"text-indent:0\"><img src=\"pic.png\" alt=\"picture\"/></p>"
    ch1 = "<h1>縦書きのテスト</h1>" + "".join(f"<p>{p}</p>" for p in paras) + pic + "".join(f"<p>{p}</p>" for p in paras * (150 if long else 6))
    zh = ["　这是一个竖排的繁体中文测试段落，文字从上到下排列，列从左到右。",
          "　「你好，世界！」他说道。“这是第<span class=\"tcy\">12</span>个例子，”她回答。"]
    ch2 = "<h1>直排中文</h1>" + "".join(f"<p>{p}</p>" for p in zh * 12)
    ch3 = "<h1>横書きのルビ</h1>" + "".join(f"<p>{p}</p>" for p in paras[:1] * 6)
    write_epub(out, "縦書きテスト", "Bookx Test", "ja",
               [("c1.xhtml", "縦書きのテスト", xhtml("c1", ch1)), ("c2.xhtml", "直排中文", xhtml("c2", ch2, "zh", "h.css")),
                ("c3.xhtml", "横書きのルビ", xhtml("c3", ch3, "ja", "p.css"))],
               {"v.css": CSS_RL, "h.css": CSS_LR, "p.css": CSS_PLAIN}, extra=[("pic.png", png(120, 160, (200, 120, 60)), "image/png")])


def aozora(src, out, chapters=3):
    raw = open(src, "rb").read().decode("shift_jis", errors="replace")
    body = raw[raw.index('<div class="main_text">'):raw.index('<div class="bibliographical_information">')]
    body = re.sub(r'<img[^>]*alt="([^"]*)"[^>]*/>', r"\1", body)  # gaiji
    body = re.sub(r"<(?!/?(ruby|rb|rp|rt|br)\b)[^>]+>", "", body)  # only ruby and line breaks stay
    # chapter headings are the lines made of a single kanji numeral
    lines = [l.strip() for l in body.replace("\r", "").split("<br />")]
    chapters_out, cur_title, cur = [], None, []
    for l in lines:
        if re.fullmatch(r"[一二三四五六七八九十]+", l):
            if cur_title is not None:
                chapters_out.append((cur_title, cur))
            cur_title, cur = l, []
        elif l and cur_title is not None:
            cur.append(l)
    chapters_out.append((cur_title, cur))
    files = []
    for i, (t, ls) in enumerate(chapters_out[:chapters]):
        ps = "".join(f"<p>{l}</p>" for l in ls)
        files.append((f"c{i + 1}.xhtml", t, xhtml(t, f"<h2>{t}</h2>{ps}")))
    write_epub(out, "吾輩は猫である（縦書き）", "夏目漱石", "ja", files, {"v.css": CSS_RL})


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] in ("synthetic", "synthetic-long"):
        synthetic(sys.argv[2], sys.argv[1] == "synthetic-long")
    elif len(sys.argv) == 4 and sys.argv[1] == "aozora":
        aozora(sys.argv[2], sys.argv[3])
    else:
        sys.exit(__doc__)
