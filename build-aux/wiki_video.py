#!/usr/bin/env python3
"""Short videos for the public wiki, made from real screen recordings of the app (standard library only; needs ffmpeg).

A scene of build-aux/session/scenarios_wiki_video.py records the sandboxed GNOME session continuously (Mutter screen cast ->
PipeWire -> GStreamer, 30 fps; build-aux/session/screencast.py) into `<scene>-<name>.raw.mkv`. This script turns that into
wiki/docs/assets/<name>-light.mp4 (H.264, yuv420p, faststart) and <name>-light.webm (VP9), both without an audio track, and a
<name>-poster-light.webp (the loading placeholder):

  wiki_video.py encode RAW.mkv OUT_BASE POSTER.webp   # OUT_BASE.mp4 + OUT_BASE.webm (each within its size budget) + the poster
  wiki_video.py check [ASSETS_DIR]                    # size per format, length, width, frame rate, pixel format, faststart
  wiki_video.py stats FILE...                         # distinct frames / total frames of each file (is it a slideshow?)

The pure parts (commands, budget, faststart test, frame counting) are unit-tested in build-aux/tests/test_wiki_video.py.
"""
import json
import os
import subprocess
import sys

MAX_SECONDS = 12.0
MAX_WIDTH = 960
FPS = 30
MIN_FPS = 24
MAX_CLIP_BYTES = 900 * 1024  # per file, so per format
MAX_TOTAL_BYTES = 6 * 1000 * 1000  # all clips, both formats, posters not counted
FORMATS = ("mp4", "webm")
CRF = {"mp4": 26, "webm": 34}
MAX_CRF = {"mp4": 38, "webm": 46}


def video_filter(width=MAX_WIDTH, fps=FPS):
    """Scale down to at most `width` px (even size, 4:2:0 needs it), constant frame rate, 4:2:0."""
    return f"scale='min({width},iw)':-2:flags=lanczos,fps={fps},format=yuv420p"


def ffmpeg_command(src, out, fmt, crf=None, seconds=MAX_SECONDS):
    """H.264 MP4 (faststart: the index first, so it plays while it loads) or VP9 WebM; never an audio track or metadata."""
    if fmt not in FORMATS:
        raise ValueError(fmt)
    crf = CRF[fmt] if crf is None else crf
    base = ["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-i", src, "-an", "-map_metadata", "-1",
            "-vf", video_filter(), "-t", f"{seconds:.1f}"]
    if fmt == "mp4":
        return base + ["-c:v", "libx264", "-preset", "slow", "-crf", str(crf), "-profile:v", "high", "-pix_fmt", "yuv420p",
                       "-movflags", "+faststart", out]
    if fmt == "webm":
        return base + ["-c:v", "libvpx-vp9", "-b:v", "0", "-crf", str(crf), "-row-mt", "1", "-deadline", "good",
                       "-cpu-used", "1", out]
    raise ValueError(fmt)


def poster_command(src, out, at):
    """One frame at `at` seconds as WebP (the placeholder shown while the video loads)."""
    return ["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-ss", f"{at:.2f}", "-i", src, "-frames:v", "1",
            "-vf", video_filter(), "-c:v", "libwebp", "-quality", "80", out]


def next_crf(fmt, crf, size, max_bytes=MAX_CLIP_BYTES):
    """None when `size` fits; else a higher CRF (smaller file), None again when the quality floor is reached."""
    if size <= max_bytes:
        return None
    step = 3 if size < 1.6 * max_bytes else 6
    nxt = crf + step
    return nxt if crf < MAX_CRF[fmt] else None


def moov_before_mdat(head):
    """True when the MP4 index (`moov`) comes before the media data (`mdat`) in `head`: the file is a faststart file."""
    moov, mdat = head.find(b"moov"), head.find(b"mdat")
    return moov != -1 and (mdat == -1 or moov < mdat)


def distinct_frames(md5_lines):
    """(distinct, total) from the lines of `ffmpeg -f framemd5` (comments skipped): a slideshow of stills has few distinct
    frames, a screen recording of something moving has many."""
    hashes = [ln.rsplit(",", 1)[-1].strip() for ln in md5_lines if ln.strip() and not ln.startswith("#")]
    return len(set(hashes)), len(hashes)


def budget_problems(name, size_bytes, seconds, width, fps=None, max_bytes=MAX_CLIP_BYTES):
    """What is wrong with a clip, as sentences (empty when it fits the budget)."""
    out = []
    if size_bytes > max_bytes:
        out.append(f"{name}: {size_bytes / 1024:.0f} KB is over the {max_bytes // 1024} KB budget of a clip")
    if seconds > MAX_SECONDS + 0.2:
        out.append(f"{name}: {seconds:.1f} s is longer than {MAX_SECONDS:.0f} s")
    if width > MAX_WIDTH:
        out.append(f"{name}: {width} px wide, at most {MAX_WIDTH} px")
    if fps is not None and not (MIN_FPS - 0.5 <= fps <= FPS + 0.5):
        out.append(f"{name}: {fps:.1f} frames per second, expected {MIN_FPS} to {FPS}")
    return out


def total_problems(sizes, max_total=MAX_TOTAL_BYTES):
    total = sum(sizes)
    if total > max_total:
        return [f"the clips are {total / 1e6:.1f} MB together, over the {max_total / 1e6:.1f} MB budget"]
    return []


def probe(path):
    """(seconds, width, fps, pix_fmt) of a video through ffprobe; None when ffprobe is not there."""
    try:
        r = subprocess.run(["ffprobe", "-v", "error", "-select_streams", "v:0", "-show_entries",
                            "stream=width,avg_frame_rate,pix_fmt:format=duration", "-of", "json", path],
                           capture_output=True, text=True, check=True)
    except (OSError, subprocess.CalledProcessError):
        return None
    d = json.loads(r.stdout)
    st = d["streams"][0]
    num, _, den = st["avg_frame_rate"].partition("/")
    fps = float(num) / float(den or 1) if float(den or 1) else 0.0
    return float(d.get("format", {}).get("duration", 0) or 0), int(st["width"]), fps, st.get("pix_fmt", "")


def encode(raw, base, poster):
    if not os.path.exists(raw):
        print(f"{raw}: not found", file=sys.stderr)
        return 1
    os.makedirs(os.path.dirname(os.path.abspath(base)), exist_ok=True)
    for fmt in FORMATS:
        out = f"{base}.{fmt}"
        crf = CRF[fmt]
        while True:
            r = subprocess.run(ffmpeg_command(raw, out, fmt, crf))
            if r.returncode:
                return r.returncode
            nxt = next_crf(fmt, crf, os.path.getsize(out))
            if nxt is None:
                break
            crf = nxt
        print(f"wrote {out} ({os.path.getsize(out) // 1024} KB, crf {crf})")
    info = probe(f"{base}.mp4")
    at = (info[0] / 2) if info else 3.0
    r = subprocess.run(poster_command(f"{base}.mp4", poster, at))
    return r.returncode


def stats(paths):
    for p in paths:
        r = subprocess.run(["ffmpeg", "-v", "error", "-i", p, "-an", "-f", "framemd5", "-"], capture_output=True, text=True)
        d, t = distinct_frames(r.stdout.splitlines())
        info = probe(p)
        secs = info[0] if info else 0
        print(f"{os.path.basename(p)}: {d} distinct of {t} frames ({secs:.1f} s)")
    return 0


def check(assets):
    problems = []
    sizes = []
    names = {}
    for f in sorted(os.listdir(assets)):
        if not f.endswith(".webm") and not f.endswith(".mp4"):
            continue
        p = os.path.join(assets, f)
        sizes.append(os.path.getsize(p))
        names.setdefault(f.rsplit(".", 1)[0], set()).add(f.rsplit(".", 1)[1])
        info = probe(p)
        seconds, width, fps, pix = info if info else (0.0, 0, None, "yuv420p")
        problems += budget_problems(f, sizes[-1], seconds, width, fps)
        if pix != "yuv420p":
            problems.append(f"{f}: pixel format {pix}, Safari needs yuv420p")
        if f.endswith(".mp4") and not moov_before_mdat(open(p, "rb").read(65536)):
            problems.append(f"{f}: not a faststart file (-movflags +faststart)")
    for n, exts in names.items():
        if exts != set(FORMATS):
            problems.append(f"{n}: needs both .mp4 and .webm, has {sorted(exts)}")
    problems += total_problems(sizes)
    for p in problems:
        print("  - " + p)
    print(f"wiki videos: {len(sizes)} files, {sum(sizes) / 1024:.0f} KB, {len(problems)} problems")
    return 1 if problems else 0


def main(argv):
    if len(argv) == 5 and argv[1] == "encode":
        return encode(argv[2], argv[3], argv[4])
    if argv[1:2] == ["check"]:
        return check(argv[2] if len(argv) > 2 else os.path.join(os.path.dirname(__file__), "..", "wiki", "docs", "assets"))
    if argv[1:2] == ["stats"] and len(argv) > 2:
        return stats(argv[2:])
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
