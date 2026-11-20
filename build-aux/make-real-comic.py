#!/usr/bin/env python3
"""A real public-domain comic for the wiki screenshots: five Sunday pages of "Little Nemo in Slumberland".

  build-aux/make-real-comic.py OUT.cbz [CACHE_DIR]     # prints OUT.cbz on success, exits 1 (with a message) when the pages
                                                       # are not cached and cannot be downloaded (NET=0 or offline)

Winsor McCay (1869-1934) drew the strip for the New York Herald in 1905-1911; the pages below were published in 1905, so
they are in the public domain in the United States (published before 1931) and worldwide (the author died more than 70
years ago). The scans are the ones on Wikimedia Commons (from archive.org), tagged "Public domain" there:
  https://commons.wikimedia.org/wiki/Category:Little_Nemo_in_Slumberland
Nothing is committed to the repository: the pages go to the cache folder (default ~/.cache/bookx-testbooks/real-comic),
every file is checked against the SHA-256 below, and the CBZ (with a ComicInfo.xml) is built from them. See
docs/test-books.md (Comic fixtures) and NOTICE.md. The unit and Broadway tests never use this; they keep the synthetic
comics of `cargo run -p bookx-comic --example make_test_comics`.

Network use: HTTPS GETs of the fixed upload.wikimedia.org URLs below, nothing else; the answer is only accepted when its
SHA-256 matches, so a changed or hostile answer is dropped.
"""
import hashlib
import os
import sys
import urllib.request
import zipfile

PAGES = [  # (file name, URL, sha256)
    ("little-nemo-1905-10-15.jpg", "https://upload.wikimedia.org/wikipedia/commons/3/38/Little_Nemo_1905-10-15.jpg",
     "339d7b1228ca24bbffa55afe46efb4203db484afb6875eaa5888684694fce9d6"),
    ("little-nemo-1905-10-29.jpg", "https://upload.wikimedia.org/wikipedia/commons/1/11/Little_Nemo_1905-10-29.jpg",
     "858c2158e678dceb4ab25feea7a9e67973c85bd13b6102fd0d5bd1d43679c7ab"),
    ("little-nemo-1905-11-05.jpg", "https://upload.wikimedia.org/wikipedia/commons/e/ed/Little_Nemo_1905-11-05.jpg",
     "16586b20d7940cee3123f2dcb81b1efa10edcf4bbbbf5352e492fff157d56a2d"),
    ("little-nemo-1905-11-12.jpg", "https://upload.wikimedia.org/wikipedia/commons/c/ca/Little_Nemo_1905-11-12.jpg",
     "8eae9b9e60c695ce5da0bd136231b6a9e9a8abe52dd82d7e6bee379cfe37d20a"),
    ("little-nemo-1905-11-19.jpg", "https://upload.wikimedia.org/wikipedia/commons/c/c9/Little_Nemo_1905-11-19.jpg",
     "7de67d3a65bcc70c1f557c3ddaafd5527d6f4790d4ae95baf626154047a636d0"),
]
MAX_BYTES = 5_000_000
COMIC_INFO = (
    '<?xml version="1.0"?><ComicInfo><Title>Little Nemo in Slumberland</Title><Series>Little Nemo in Slumberland</Series>'
    "<Number>1</Number><Writer>Winsor McCay</Writer><Penciller>Winsor McCay</Penciller><Year>1905</Year>"
    "<LanguageISO>en</LanguageISO><Summary>Sunday pages of 15 October to 19 November 1905, public domain "
    "(Wikimedia Commons).</Summary>"
    '<Pages><Page Image="0" Type="FrontCover"/></Pages></ComicInfo>'
)


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def page_bytes(name, url, digest, cache, download=True):
    """The bytes of one page from the cache, or downloaded (and cached) when its hash matches; None when impossible."""
    path = os.path.join(cache, name)
    if os.path.exists(path):
        with open(path, "rb") as f:
            data = f.read()
        if sha256(data) == digest:
            return data
        os.remove(path)  # a damaged cache file is fetched again
    if not download:
        return None
    req = urllib.request.Request(url, headers={"User-Agent": "bookx-wiki-fixture/1.0 (https://bookx.adhadse.com)"})
    try:
        with urllib.request.urlopen(req, timeout=90) as r:
            data = r.read(MAX_BYTES + 1)
    except OSError as e:
        print(f"real comic: {url}: {e}", file=sys.stderr)
        return None
    if len(data) > MAX_BYTES or sha256(data) != digest:
        print(f"real comic: {name} does not match the pinned SHA-256, ignored", file=sys.stderr)
        return None
    with open(path + ".part", "wb") as f:
        f.write(data)
    os.replace(path + ".part", path)
    return data


def build(out, cache, download=True):
    os.makedirs(cache, exist_ok=True)
    pages = []
    for name, url, digest in PAGES:
        data = page_bytes(name, url, digest, cache, download)
        if data is None:
            return False
        pages.append(data)
    with zipfile.ZipFile(out + ".part", "w", zipfile.ZIP_STORED) as z:
        for i, data in enumerate(pages, 1):
            z.writestr(f"p{i:03}.jpg", data)
        z.writestr("ComicInfo.xml", COMIC_INFO)
    os.replace(out + ".part", out)
    return True


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    out = sys.argv[1]
    cache = sys.argv[2] if len(sys.argv) > 2 else os.path.join(os.path.dirname(out) or ".", "real-comic")
    if not build(out, cache, download=os.environ.get("NET", "1") != "0"):
        print("real comic: not cached and not downloadable (NET=0?); the synthetic comic is used instead", file=sys.stderr)
        return 1
    print(out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
