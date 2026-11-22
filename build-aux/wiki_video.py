#!/usr/bin/env python3
"""Short videos for the public wiki, made from real frames of the app (standard library, Pillow only to read a frame size).

A scene of build-aux/session/scenarios_wiki_video.py takes a series of screenshots while a feature runs and writes, per clip,
`<scene>-<name>.fNNNN.frame` (PNG data, cut to the window without the GNOME top bar) and `<scene>-<name>.timing.json` (the
seconds since the start for every frame). This script turns them into wiki/docs/assets/<name>-light.webm (VP9, no audio
track) and <name>-poster-light.webp:

  wiki_video.py encode TIMING.json OUT.webm OUT-poster.webp      # frames next to TIMING.json
  wiki_video.py check [ASSETS_DIR]                               # the size / duration / frame-size budget of every clip

The pure parts (durations, the ffmpeg input list and command, the budget) are unit-tested in build-aux/tests/test_wiki_video.py.
"""
import json
import os
import subprocess
import sys

MAX_SECONDS = 12.0
MAX_WIDTH = 960
FPS = 15
MAX_CLIP_BYTES = 600 * 1024
MAX_TOTAL_BYTES = 4 * 1024 * 1024
MIN_FRAME_SECONDS = 1.0 / FPS


def frame_durations(times, max_seconds=MAX_SECONDS, tail=0.5):
    """How long each frame is shown: the gap to the next capture (at least one output frame), the last one `tail` seconds;
    frames after `max_seconds` are dropped. Returns a list of (index, seconds)."""
    out = []
    for i, t in enumerate(times):
        if t >= max_seconds:
            break
        nxt = times[i + 1] if i + 1 < len(times) else t + tail
        out.append((i, max(MIN_FRAME_SECONDS, min(nxt, max_seconds) - t)))
    return out


def concat_script(frames, durations):
    """The ffmpeg concat-demuxer list: `frames` are file names, `durations` come from frame_durations. The last file is
    listed twice (the demuxer ignores the duration of the final entry)."""
    lines = []
    for i, seconds in durations:
        lines.append(f"file '{frames[i]}'")
        lines.append(f"duration {seconds:.3f}")
    if durations:
        lines.append(f"file '{frames[durations[-1][0]]}'")
    return "\n".join(lines) + "\n"


def scale_filter(width, max_width=MAX_WIDTH, panel=0):
    """The video filter: the GNOME top bar (`panel` px) is already cut by the scene; scale down to at most `max_width`
    (even dimensions, as VP9 4:2:0 needs) and resample to a constant frame rate."""
    w = min(width, max_width)
    w -= w % 2
    return f"scale={w}:-2:flags=lanczos,fps={FPS},format=yuv420p"


def ffmpeg_command(concat_file, out, width, crf=36):
    """VP9, constant quality, no audio, no metadata, two passes are not needed for such short clips."""
    return ["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-f", "concat", "-safe", "0", "-i", concat_file,
            "-vf", scale_filter(width), "-an", "-c:v", "libvpx-vp9", "-b:v", "0", "-crf", str(crf), "-row-mt", "1",
            "-deadline", "good", "-cpu-used", "1", "-map_metadata", "-1", "-t", f"{MAX_SECONDS:.1f}", out]


def budget_problems(name, size_bytes, seconds, width, max_bytes=MAX_CLIP_BYTES):
    """What is wrong with a clip, as sentences (empty when it fits the budget)."""
    out = []
    if size_bytes > max_bytes:
        out.append(f"{name}: {size_bytes / 1024:.0f} KB is over the {max_bytes // 1024} KB budget of a clip")
    if seconds > MAX_SECONDS + 0.2:
        out.append(f"{name}: {seconds:.1f} s is longer than {MAX_SECONDS:.0f} s")
    if width > MAX_WIDTH:
        out.append(f"{name}: {width} px wide, at most {MAX_WIDTH} px")
    return out


def total_problems(sizes, max_total=MAX_TOTAL_BYTES):
    total = sum(sizes)
    if total > max_total:
        return [f"the clips are {total / 1e6:.1f} MB together, over the {max_total / 1e6:.1f} MB budget"]
    return []


def probe(path):
    """(seconds, width) of a video through ffprobe; None when ffprobe is not there."""
    try:
        r = subprocess.run(["ffprobe", "-v", "error", "-select_streams", "v:0", "-show_entries",
                            "stream=width:format=duration", "-of", "json", path], capture_output=True, text=True, check=True)
    except (OSError, subprocess.CalledProcessError):
        return None
    d = json.loads(r.stdout)
    return float(d.get("format", {}).get("duration", 0) or 0), int(d["streams"][0]["width"])


def encode(timing_path, out, poster):
    from PIL import Image
    base = timing_path[: -len(".timing.json")]
    times = json.load(open(timing_path))
    frames = [f"{base}.f{i + 1:04d}.frame" for i in range(len(times))]
    durations = frame_durations(times)
    if not durations:
        print(f"{timing_path}: no frames", file=sys.stderr)
        return 1
    missing = [frames[i] for i, _ in durations if not os.path.exists(frames[i])]
    if missing:
        print(f"{timing_path}: missing frames, e.g. {missing[0]}", file=sys.stderr)
        return 1
    width = Image.open(frames[0]).width
    listing = base + ".concat.txt"
    with open(listing, "w") as f:
        f.write(concat_script([os.path.abspath(p) for p in frames], durations))
    os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
    cmd = ffmpeg_command(listing, out, width)
    r = subprocess.run(cmd)
    os.remove(listing)
    if r.returncode:
        return r.returncode
    # the poster: the frame in the middle of the clip, the same size as the video
    pick = frames[durations[len(durations) // 2][0]]
    im = Image.open(pick).convert("RGB")
    if im.width > MAX_WIDTH:
        im = im.resize((MAX_WIDTH, round(im.height * MAX_WIDTH / im.width)), Image.LANCZOS)
    im.save(poster, "WEBP", quality=80, method=6)
    print(f"wrote {out} ({os.path.getsize(out) // 1024} KB, {len(durations)} frames) and {poster}")
    return 0


def check(assets):
    problems = []
    sizes = []
    for f in sorted(os.listdir(assets)):
        if not f.endswith((".webm", ".mp4")):
            continue
        p = os.path.join(assets, f)
        sizes.append(os.path.getsize(p))
        info = probe(p)
        seconds, width = info if info else (0.0, 0)
        problems += budget_problems(f, sizes[-1], seconds, width)
    problems += total_problems(sizes)
    for p in problems:
        print("  - " + p)
    print(f"wiki videos: {len(sizes)} clips, {sum(sizes) / 1024:.0f} KB, {len(problems)} problems")
    return 1 if problems else 0


def main(argv):
    if len(argv) == 5 and argv[1] == "encode":
        return encode(argv[2], argv[3], argv[4])
    if argv[1:2] == ["check"]:
        return check(argv[2] if len(argv) > 2 else os.path.join(os.path.dirname(__file__), "..", "wiki", "docs", "assets"))
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
