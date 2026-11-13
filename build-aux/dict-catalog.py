#!/usr/bin/env python3
"""Builds and checks data/dictionaries.json, the catalogue of dictionaries Bookx downloads for the reader.

  build-aux/dict-catalog.py generate   downloads every archive of the plan below (cached in
                                       ~/.cache/bookx-dict-catalog), computes SHA-256 and size and writes
                                       data/dictionaries.json
  build-aux/dict-catalog.py check      validates the file (schema, https, hashes look like hashes, every
                                       language of the plan present) and asks each server for the size
                                       (HEAD); nothing is downloaded
  build-aux/dict-catalog.py verify     the same, and downloads every archive and compares SHA-256 and size
                                       (a few hundred MB the first time, cached after)

Sources (all StarDict, all freely redistributable, see the licence field of each entry):
  * Vuizur/Wiktionary-Dictionaries on GitHub: dictionaries extracted from Wiktionary with Wiktextract/kaikki.org,
    "<Language>-English Wiktionary dictionary stardict.tar.gz" (CC BY-SA 3.0 and GFDL, like Wiktionary). The
    best choice for most languages: the headword is in the book's language, the definitions are in English.
  * WikDict (download.wikdict.com), derived from Wiktionary, CC BY-SA 4.0, small zip files. Used as the
    fallback origin, and as the first one for Dutch and Swedish, which the first source lacks.
  * Webster's Revised Unabridged Dictionary 1913 (public domain, from dict.org) as a second English one.

The archive formats the app unpacks are tar.gz, zip and tar.bz2 (the crates the app has); FreeDict is not used
because it ships .tar.xz only.
"""
import hashlib
import json
import os
import re
import sys
import urllib.parse
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "..", "data", "dictionaries.json")
CACHE = os.path.expanduser("~/.cache/bookx-dict-catalog")
UA = {"User-Agent": "Bookx-dict-catalog/1.0"}

VZ = "https://github.com/Vuizur/Wiktionary-Dictionaries/raw/master/"
WD = "https://download.wikdict.com/dictionaries/stardict/wikdict-{}-en.zip"

# code, name, name in the Vuizur repository (None: not there), WikDict pair available
PLAN = [
    ("en", "English", "English", False),
    ("es", "Spanish", "Spanish", True),
    ("fr", "French", "French", True),
    ("de", "German", "German", True),
    ("it", "Italian", "Italian", True),
    ("pt", "Portuguese", "Portuguese", True),
    ("ru", "Russian", "Russian", True),
    ("nl", "Dutch", None, True),
    ("pl", "Polish", "Polish", True),
    ("tr", "Turkish", "Turkish", True),
    ("sv", "Swedish", None, True),
    ("cs", "Czech", "Czech", True),
    ("ja", "Japanese", "Japanese", True),
    ("zh", "Chinese", "Chinese", True),
    ("ar", "Arabic", "Arabic", False),
    ("hi", "Hindi", "Hindi", False),
    ("ko", "Korean", "Korean", False),
    ("el", "Greek", "Greek", True),
    ("he", "Hebrew", "Hebrew", False),
    ("uk", "Ukrainian", "Ukrainian", False),
]

WIKT = {
    "licence": "CC BY-SA 3.0 and GFDL (as Wiktionary)",
    "licence_url": "https://en.wiktionary.org/wiki/Wiktionary:Copyrights",
}
WIKDICT = {
    "licence": "CC BY-SA 4.0",
    "licence_url": "https://creativecommons.org/licenses/by-sa/4.0/",
}


def fetch(url, head=False):
    req = urllib.request.Request(url, headers=UA, method="HEAD" if head else "GET")
    return urllib.request.urlopen(req, timeout=120)


def download(url):
    os.makedirs(CACHE, exist_ok=True)
    p = os.path.join(CACHE, hashlib.sha1(url.encode()).hexdigest())
    if not os.path.exists(p):
        print("  downloading", url, file=sys.stderr)
        with fetch(url) as r, open(p + ".part", "wb") as f:
            while True:
                b = r.read(1 << 20)
                if not b:
                    break
                f.write(b)
        os.rename(p + ".part", p)
    return p


def digest(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest(), os.path.getsize(path)


def origin(url, fmt):
    sha, size = digest(download(url))
    return {"url": url, "format": fmt, "sha256": sha, "size": size}


def generate():
    langs = []
    for code, name, vz, wd in PLAN:
        print(code, name, file=sys.stderr)
        dicts = []
        origins, source, lic = [], "", None
        if vz:
            origins.append(origin(VZ + urllib.parse.quote(f"{vz}-English Wiktionary dictionary stardict.tar.gz"), "tar.gz"))
            source, lic = "Wiktionary, extracted by Vuizur with Wiktextract", WIKT
        if wd:
            origins.append(origin(WD.format(code), "zip"))
            if not vz:
                source, lic = "WikDict (Wiktionary)", WIKDICT
        kind = "monolingual" if code == "en" else "bilingual"
        dicts.append({
            "id": f"wiktionary-{code}",
            "title": f"{name}{'' if code == 'en' else '–English'} (Wiktionary)",
            "kind": kind,
            "gloss": "en",
            "source": source,
            "licence": lic["licence"],
            "licence_url": lic["licence_url"],
            "origins": origins,
        })
        if code == "en":
            dicts.append({
                "id": "webster-1913",
                "title": "Webster's Revised Unabridged Dictionary, 1913",
                "kind": "monolingual",
                "gloss": "en",
                "source": "dict.org web1913, packaged for StarDict by ahacop",
                "licence": "Public domain",
                "licence_url": "https://en.wikisource.org/wiki/Webster%27s_Revised_Unabridged_Dictionary",
                "origins": [origin("https://github.com/ahacop/websters-dict-1913-stardict/raw/main/stardict-dictd-web1913-2.4.2.tgz", "tar.gz")],
            })
        for d in dicts:
            d["size"] = d["origins"][0]["size"]
        langs.append({"code": code, "name": name, "dictionaries": dicts})
    out = {"version": 1, "languages": langs}
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w") as f:
        json.dump(out, f, indent=1, ensure_ascii=False)
        f.write("\n")
    print("wrote", OUT, file=sys.stderr)


def validate(cat):
    errs = []
    if cat.get("version") != 1:
        errs.append("version must be 1")
    seen = set()
    codes = [l["code"] for l in cat.get("languages", [])]
    for code, *_ in PLAN:
        if code not in codes:
            errs.append(f"language {code} is missing")
    for l in cat.get("languages", []):
        if not re.fullmatch(r"[a-z]{2,3}", l["code"]):
            errs.append(f"bad code {l['code']}")
        if not l.get("dictionaries"):
            errs.append(f"{l['code']}: no dictionary")
        for d in l.get("dictionaries", []):
            if d["id"] in seen:
                errs.append(f"duplicate id {d['id']}")
            seen.add(d["id"])
            for k in ("title", "kind", "source", "licence", "licence_url", "size"):
                if not d.get(k):
                    errs.append(f"{d['id']}: {k} missing")
            if not d.get("origins"):
                errs.append(f"{d['id']}: no origin")
            for o in d.get("origins", []):
                if not o["url"].startswith("https://"):
                    errs.append(f"{d['id']}: not https: {o['url']}")
                if not re.fullmatch(r"[0-9a-f]{64}", o["sha256"]):
                    errs.append(f"{d['id']}: bad sha256")
                if o["format"] not in ("tar.gz", "zip", "tar.bz2"):
                    errs.append(f"{d['id']}: format {o['format']}")
                if o["size"] <= 0:
                    errs.append(f"{d['id']}: size")
    return errs


def check(full):
    cat = json.load(open(OUT))
    errs = validate(cat)
    for l in cat["languages"]:
        for d in l["dictionaries"]:
            for o in d["origins"]:
                try:
                    if full:
                        sha, size = digest(download(o["url"]))
                        if sha != o["sha256"] or size != o["size"]:
                            errs.append(f"{d['id']}: {o['url']} is now {sha} / {size}")
                    else:
                        with fetch(o["url"], head=True) as r:
                            n = int(r.headers.get("Content-Length", "0") or 0)
                            if n and n != o["size"]:
                                errs.append(f"{d['id']}: {o['url']} is {n} bytes, the catalogue says {o['size']}")
                except Exception as e:  # noqa: BLE001
                    errs.append(f"{d['id']}: {o['url']}: {e}")
    for e in errs:
        print("ERROR", e)
    print(f"{sum(len(l['dictionaries']) for l in cat['languages'])} dictionaries, {len(errs)} problems")
    return 1 if errs else 0


if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else "check"
    if cmd == "generate":
        generate()
    elif cmd in ("check", "verify"):
        sys.exit(check(cmd == "verify"))
    else:
        sys.exit(__doc__)
