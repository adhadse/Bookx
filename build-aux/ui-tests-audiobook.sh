# Scenarios of audiobooks (docs/pain-points-ebook-managers.md PP-G-25). Sourced by ui-tests.sh (uses its
# `scenario`, `check`, `has`, $ROOT, $CLI, $HERE).
#
# The audio of these scenarios is generated here (silent 8 kHz WAV parts and the tiny M4B of the crate's
# fixtures: no copyrighted audio) and played through BOOKX_AUDIO_SINK=null:20, a sink that plays nothing but
# takes a twentieth of the time the audio would take, so a minute of book plays in three seconds. The
# libraries of the scenarios hold two books (ids 1 and 2), so the first audiobook is id 3.
# Dev commands: `audio:add=PATH`, `audio:open=ID`, `audio:close`, `audio:stats`, and for the open player
# `audio:play|pause|toggle|skip=SECONDS|seek=MS|chapter=N|next|prev|speed=X|volume=X|sleep=MINUTES|sleep=chapter|
# sleep-secs=N|bookmark=NOTE|gobookmark=N|rmbookmark=N|undo|dump`; `library:audiobadges`.

export BOOKX_AUDIO_SINK=null:20

AUD="$ROOT/audio"
mkdir -p "$AUD/Tale"
python3 - "$AUD" <<'PY'
import math, os, struct, sys, wave, zipfile
root = sys.argv[1]
def wav(path, secs, hz=0):
    with wave.open(path, "wb") as w:
        w.setnchannels(1); w.setsampwidth(2); w.setframerate(8000)
        n = int(8000 * secs)
        w.writeframes(b"".join(struct.pack("<h", int(3000 * math.sin(i / 8000 * hz * 6.2832)) if hz else 0) for i in range(n)))
for name, secs, hz in (("01 - The Gate.wav", 20, 220), ("02 - The Road.wav", 30, 330), ("03 - The End.wav", 10, 440)):
    wav(os.path.join(root, "Tale", name), secs, hz)
with zipfile.ZipFile(os.path.join(root, "zipped.zip"), "w", zipfile.ZIP_STORED) as z:
    for name in sorted(os.listdir(os.path.join(root, "Tale"))):
        z.write(os.path.join(root, "Tale", name), "Zipped Tale/" + name)
PY
cp "$HERE/../crates/bookx-audio/tests/fixtures/tiny.m4b" "$AUD/Big Book.m4b"
cp "$HERE/../crates/bookx-audio/tests/fixtures/tiny.opus" "$AUD/Only Opus.opus"

# the catalogue's view of one book: `audio_book <json> <id> <expression on b>`
audio_book() { python3 -c 'import json,sys; b=[x for x in json.loads(sys.argv[1])["books"] if x["id"]==int(sys.argv[2])][0]; sys.exit(0 if eval(sys.argv[3]) else 1)' "$1" "$2" "$3"; }

scenario audio_import "audio:add=$AUD/Tale,audio:add=$AUD/Big Book.m4b,audio:add=$AUD/zipped.zip,wait:600,library:audiobadges,quit" '
    has "^audio: added id=3"; check "a folder of parts is imported as an audiobook" $?
    has "^audio: added id=4"; check "a single M4B is imported" $?
    has "^audio: added id=5"; check "a zip of parts is imported" $?
    audio_book "$LIB" 3 "b[\"meta\"][\"title\"]==\"Tale\" and \"Audiobook\" in b[\"meta\"][\"tags\"]"; check "the book is named after its folder and tagged Audiobook" $?
    audio_book "$LIB" 3 "any(r[\"kind\"]==\"audiobook\" for r in b[\"resources\"]) and any(f[\"format\"]==\"wav\" for f in b[\"formats\"])"; check "the audio is linked and its format is listed (format:wav finds it)" $?
    audio_book "$LIB" 4 "b[\"meta\"][\"title\"]==\"Tiny Tale\" and b[\"meta\"][\"authors\"]==[\"Ann Author\"] and any(f[\"format\"]==\"m4b\" for f in b[\"formats\"])"; check "the M4B tags give title, author and the m4b format" $?
    audio_book "$LIB" 5 "b[\"meta\"][\"title\"]==\"zipped\""; check "a zip is named after the zip when its tags say nothing" $?
    has "^audiobadges: visible=[3-9] sidebar_row=true"; check "the covers show the audiobook badge and the sidebar has the Audiobooks entry" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario audio_filters "audio:add=$AUD/Tale,audio:add=$AUD/Big Book.m4b,wait:400,library:search=format:m4b,wait:500,library:dump,library:search=audiobook:yes,wait:500,library:dump,library:search=,quit" '
    L=$(grep -E "^title:" <<<"$OUT")
    grep -q "^title: Search results | 1 of 4 books" <<<"$L"; check "format:m4b finds the audiobook by its format" $?
    grep -q "^title: Search results | 2 of 4 books" <<<"$L"; check "audiobook:yes finds both audiobooks and not the e-books" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario audio_player "audio:add=$AUD/Tale,audio:open=3,wait:600,audio:dump,audio:play,wait:900,audio:pause,wait:300,audio:dump,audio:seek=25000,audio:dump,audio:next,audio:dump,audio:prev,audio:dump,audio:chapter=0,audio:skip=30,audio:dump,audio:skip=-15,audio:dump,quit" '
    A=$(grep "^audio: state" <<<"$OUT")
    sed -n 1p <<<"$A" | grep -q "state=Paused pos=0 chapter=1/3 .*total=60000"; check "the player opens paused at the start of a three chapter, one minute book" $?
    sed -n 2p <<<"$A" | grep -Eq "state=Paused pos=[1-9][0-9]{3,}"; check "playing moves the position and pause stops it" $?
    sed -n 3p <<<"$A" | grep -q "pos=25000 chapter=2/3"; check "seeking lands in the second chapter" $?
    sed -n 4p <<<"$A" | grep -q "pos=50000 chapter=3/3"; check "next chapter goes to the start of the third" $?
    sed -n 5p <<<"$A" | grep -q "pos=20000 chapter=2/3"; check "previous chapter at the start of a chapter goes back one" $?
    sed -n 6p <<<"$A" | grep -q "pos=30000 chapter=2/3"; check "forward 30 seconds" $?
    sed -n 7p <<<"$A" | grep -q "pos=15000 chapter=1/3"; check "back 15 seconds crosses into the chapter before" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario audio_speed "audio:add=$AUD/Tale,audio:open=3,wait:400,audio:seek=30000,audio:dump,audio:speed=2,audio:dump,audio:speed=9,audio:dump,audio:speed=0.1,audio:dump,audio:play,wait:300,audio:speed=1.25,wait:400,audio:pause,audio:dump,quit" '
    A=$(grep "^audio: state" <<<"$OUT")
    sed -n 1p <<<"$A" | grep -q "speed=1 .*progress=\"50 % · 1 min left\""; check "the progress line shows percent and time left at normal speed" $?
    sed -n 2p <<<"$A" | grep -q "speed=2 "; check "speed 2x" $?
    sed -n 3p <<<"$A" | grep -q "speed=3 "; check "speeds above 3x are held at 3x" $?
    sed -n 4p <<<"$A" | grep -q "speed=0.5 "; check "speeds below 0.5x are held at 0.5x" $?
    sed -n 5p <<<"$A" | grep -q "speed=1.25 .*error=$"; check "the speed changes while playing without an error" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario audio_sleep "audio:add=$AUD/Tale,audio:open=3,wait:400,audio:sleep=chapter,audio:dump,audio:play,wait:2200,audio:dump,audio:seek=0,audio:sleep-secs=1,audio:play,wait:2500,audio:dump,quit" '
    A=$(grep "^audio: state" <<<"$OUT")
    sed -n 1p <<<"$A" | grep -q "sleep=chapter"; check "the end-of-chapter sleep timer is set" $?
    sed -n 2p <<<"$A" | grep -q "state=Paused pos=20000 chapter=2/3 .*sleep=off"; check "it stops exactly at the end of the chapter and switches itself off" $?
    sed -n 3p <<<"$A" | grep -Eq "state=Paused pos=[0-9]+ .*sleep=off"; check "a timer of one second stops playback" $?
    P=$(sed -n 3p <<<"$A" | sed -E "s/.* pos=([0-9]+) .*/\1/")
    [ "${P:-60000}" -lt 59000 ]; check "...before the end of the book" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario audio_bookmarks "audio:add=$AUD/Tale,audio:open=3,wait:400,audio:seek=5000,audio:bookmark=First note,audio:seek=30000,audio:bookmark=,audio:dump,audio:gobookmark=0,audio:dump,audio:rmbookmark=1,audio:dump,audio:close,quit" '
    A=$(grep "^audio: state" <<<"$OUT")
    sed -n 1p <<<"$A" | grep -q "bookmarks=2"; check "two bookmarks, one with a note and one without" $?
    sed -n 2p <<<"$A" | grep -q "pos=5000 "; check "a bookmark jumps to its position" $?
    sed -n 3p <<<"$A" | grep -q "bookmarks=1"; check "a bookmark is deleted" $?
    S=$(cat "$d"/cfg/bookx/audiobooks/*.state.json 2>/dev/null)
    grep -q "First note" <<<"$S" && grep -q "\"position_ms\": 5000" <<<"$S"; check "the bookmark and its note are saved" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario audio_bookmark_undo "audio:add=$AUD/Tale,audio:open=3,wait:400,audio:seek=5000,audio:bookmark=First note,audio:dump,audio:rmbookmark=0,audio:dump,audio:undo,audio:dump,audio:close,quit" '
    A=$(grep "^audio: state" <<<"$OUT")
    sed -n 1p <<<"$A" | grep -q "bookmarks=1"; check "one bookmark with a note" $?
    sed -n 2p <<<"$A" | grep -q "bookmarks=0"; check "deleting it removes it at once, without a question" $?
    sed -n 3p <<<"$A" | grep -q "bookmarks=1"; check "Undo of the Bookmark deleted toast puts it back" $?
    S=$(cat "$d"/cfg/bookx/audiobooks/*.state.json 2>/dev/null)
    grep -q "First note" <<<"$S" && grep -q "\"position_ms\": 5000" <<<"$S"; check "with its note and position, saved" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'


scenario audio_resume "audio:add=$AUD/Tale,audio:open=3,wait:400,audio:seek=33000,audio:speed=1.5,wait:300,audio:close,wait:800,audio:open=3,wait:600,audio:dump,audio:close,quit" '
    A=$(grep "^audio: state" <<<"$OUT")
    sed -n 1p <<<"$A" | grep -q "pos=31000 chapter=2/3 speed=1.5"; check "reopening resumes two seconds before where it stopped, at the same speed" $?
    audio_book "$LIB" 3 "abs(b[\"progress\"]-0.55)<0.01 and b[\"position\"][\"spine\"]==1"; check "the library shows 55 percent and the chapter" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario audio_stats "audio:add=$AUD/Tale,audio:open=3,wait:400,audio:play,wait:600,stats:skew:600,audio:pause,wait:600,audio:close,wait:300,audio:stats,quit" '
    T=$(grep "^audio_stats:" <<<"$OUT" | sed -E "s/.* total=([0-9]+) .*/\1/")
    [ "${T:-0}" -ge 590 ]; check "the listening time is recorded as a session" $?
    has "^audio_stats: .*top=\[\(3, "; check "the book is in the most listened list" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario audio_finish "audio:add=$AUD/Tale,audio:open=3,wait:400,audio:seek=57000,audio:play,wait:1500,audio:dump,audio:close,wait:900,audio:stats,quit" '
    sed -n 1p <<<"$(grep "^audio: state" <<<"$OUT")" | grep -q "state=Ended pos=60000"; check "the book plays to its end" $?
    audio_book "$LIB" 3 "b[\"progress\"]>=0.98"; check "the library marks it finished" $?
    has "^audio_stats: .*finished_year=1"; check "a finished audiobook counts in the statistics" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario audio_unplayable "audio:add=$AUD/Only Opus.opus,audio:open=3,wait:500,audio:play,wait:600,audio:dump,quit" '
    has "^audio: added id=3"; check "an Opus audiobook is imported" $?
    has "^audio: state=Paused .*error=Bookx cannot play opus"; check "it says that it cannot play Opus instead of failing silently" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario audio_a11y_icons "audio:add=$AUD/Tale,audio:open=3,wait:600,a11y-audit:,library:icons,quit" '
    has "^a11y: controls=[0-9]+ problems=0"; check "every control of the player has an accessible name" $?
    has "^icons: checked=[0-9]+ missing=\[\]"; check "the icons exist" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario audio_narrow_900x600 "audio:add=$AUD/Tale,audio:open=3,wait:600,quit" '
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "the player opens in a 900x600 window without GTK warnings" $?'
