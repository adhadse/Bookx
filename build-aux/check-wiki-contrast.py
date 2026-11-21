#!/usr/bin/env python3
"""Checks the wiki's colour pairs for WCAG AA (build-aux/wiki.sh).

  check-wiki-contrast.py

Reads the palettes from wiki/docs/assets/extra.css (the `--bx-*` tokens of the dark scheme, the `--md-*` tokens of the
light one, the dark code-highlighting colours) and checks the text/background pairs that matter: 4.5:1 for text, 3:1
for large text and interface parts. Exits 1 and lists the failing pairs otherwise.
"""
import os
import re
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
CSS = os.path.join(ROOT, "wiki", "docs", "assets", "extra.css")


def block(css, scheme):
    return re.search(r'\[data-md-color-scheme="%s"\]\s*\{(.*?)\n\}' % scheme, css, re.S).group(1)


def tokens(text):
    return dict(re.findall(r"(--[\w-]+):\s*([^;]+);", text))


def resolve(tok, value, depth=0):
    value = value.strip()
    m = re.fullmatch(r"var\((--[\w-]+)\)", value)
    if m and depth < 8:
        return resolve(tok, tok[m.group(1)], depth + 1)
    return value


def rgb(value):
    m = re.fullmatch(r"#([0-9a-fA-F]{6})", value)
    if m:
        v = m.group(1)
        return tuple(int(v[i:i + 2], 16) for i in (0, 2, 4))
    m = re.fullmatch(r"rgba\((\d+),\s*(\d+),\s*(\d+),\s*([\d.]+)\)", value)
    if m:
        return tuple(int(m.group(i)) for i in (1, 2, 3))
    raise ValueError(value)


def lum(c):
    def f(v):
        v /= 255
        return v / 12.92 if v <= 0.03928 else ((v + 0.055) / 1.055) ** 2.4
    return 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2])


def ratio(a, b):
    la, lb = sorted((lum(a), lum(b)), reverse=True)
    return (la + 0.05) / (lb + 0.05)


def checks(tok, pairs):
    rows = []
    for fg, bg, need, what in pairs:
        r = ratio(rgb(resolve(tok, tok[fg])), rgb(resolve(tok, tok[bg])))
        rows.append((what, fg, bg, r, need))
    return rows


def main():
    css = open(CSS, encoding="utf8").read()
    dark = tokens(block(css, "slate"))
    light = tokens(block(css, "default"))
    dark_pairs = [
        ("--bx-text", "--bx-page", 4.5, "body text on page"),
        ("--bx-text", "--bx-surface", 4.5, "text on code/admonition surface"),
        ("--bx-text", "--bx-raised", 4.5, "text on raised surface (inline code, table head)"),
        ("--bx-muted", "--bx-page", 4.5, "muted text on page"),
        ("--bx-muted", "--bx-bar", 4.5, "muted text on header/nav bar"),
        ("--bx-muted", "--bx-surface", 4.5, "muted text on surface"),
        ("--bx-faint", "--bx-page", 3, "faint (quote marks, decoration) on page"),
        ("--bx-accent", "--bx-page", 4.5, "links/accent on page"),
        ("--bx-accent", "--bx-surface", 4.5, "accent on surface"),
        ("--bx-accent", "--bx-raised", 4.5, "accent on raised"),
        ("--bx-accent-hover", "--bx-page", 4.5, "link hover on page"),
        ("--bx-on-accent", "--bx-accent", 4.5, "button text on accent"),
        ("--bx-on-accent", "--bx-accent-hover", 4.5, "button text on accent hover"),
        ("--md-code-hl-number-color", "--bx-surface", 4.5, "code: numbers"),
        ("--md-code-hl-special-color", "--bx-surface", 4.5, "code: special"),
        ("--md-code-hl-function-color", "--bx-surface", 4.5, "code: functions"),
        ("--md-code-hl-constant-color", "--bx-surface", 4.5, "code: constants"),
        ("--md-code-hl-keyword-color", "--bx-surface", 4.5, "code: keywords"),
        ("--md-code-hl-string-color", "--bx-surface", 4.5, "code: strings"),
        ("--md-code-hl-operator-color", "--bx-surface", 4.5, "code: operators"),
        ("--md-code-hl-punctuation-color", "--bx-surface", 4.5, "code: punctuation"),
        ("--md-code-hl-comment-color", "--bx-surface", 4.5, "code: comments"),
        ("--md-code-fg-color", "--bx-surface", 4.5, "code: plain"),
    ]
    light_pairs = [
        ("--md-primary-bg-color", "--md-default-bg-color", 4.5, "header/body text on page"),
        ("--md-accent-fg-color", "--md-default-bg-color", 4.5, "links on page"),
    ]
    # admonition tints sit on a mix of --adm and the surface; check the tint itself against the surface as UI parts
    adm = sorted(set(re.findall(r"--adm:\s*(#[0-9a-fA-F]{6});", css)))
    bad = 0
    for name, tok, pairs in (("dark", dark, dark_pairs), ("light", light, light_pairs)):
        print(f"{name} palette")
        for what, fg, bg, r, need in checks(tok, pairs):
            ok = r >= need
            bad += not ok
            print(f"  {'ok  ' if ok else 'FAIL'} {r:5.2f}:1 (>= {need}) {what}  [{fg} on {bg}]")
    print("dark admonition accents (icons, borders; UI parts need 3:1) on the surface")
    surface = rgb(dark["--bx-surface"])
    for a in adm:
        r = ratio(rgb(a), surface)
        ok = r >= 3
        bad += not ok
        print(f"  {'ok  ' if ok else 'FAIL'} {r:5.2f}:1 {a}")
    if bad:
        print(f"{bad} colour pair(s) below WCAG AA", file=sys.stderr)
        return 1
    print("wiki colours OK (WCAG AA)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
