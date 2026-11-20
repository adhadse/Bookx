# Scenarios of the KOReader highlight/note sync (docs/koreader-annotation-sync.md). Sourced by ui-tests.sh (uses its
# `scenario`, `check`, `has`, `$LIB`). Bookx's server runs inside the app on a random loopback port; "the device" is a
# fake KOReader client built into the dev command `koannot:device=<file>` that makes the same HTTP calls as the Lua
# plugin (PUT /bookx/annotations/<md5> with the KOSync headers). KOReader itself is never run.
# Dev commands: `koannot:start|device=F|merge|dump|pull=ID|addhl=ID;TEXT;NOTE|editnote=ID;TEXT;NOTE|delhl=ID;TEXT|plugin=DIR`.

koannot_setup() {
    local d=$1
    mkdir -p "$d/ko" "$d/plug"
    mkdir -p "$d/mounts/KOBOeReader/.adds/koreader/plugins"
    python3 - "$d/ko" <<'PY'
import json, sys
out = sys.argv[1]
T = 1700000000
P3 = "Paragraph 3 of chapter 1 talks about functionality, readers and the world around them in plain words."
P5 = "Paragraph 5 of chapter 3 talks about functionality, readers and the world around them in plain words."
def a(i, text, note, upd, **kw):
    d = {"id": i, "kind": "highlight", "text": text, "note": note, "color": "green", "drawer": "lighten",
         "chapter": "One", "pos0": "/body/DocFragment[1]/body/p[3]/text().0", "pos1": "/body/DocFragment[1]/body/p[3]/text().90",
         "page": 3, "created": T, "updated": upd}
    d.update(kw)
    return d
base = {"device": "Kobo Libra", "title": "UI Test Book", "authors": "Test Author", "document": "auto:1"}
first = dict(base, annotations=[
    a("ko-1", P3, "first thought", T),
    a("ko-2", P5, "", T),
    a("ko-3", "A sentence that is not in this book at all.", "stray note", T),
    {"id": "ko-4", "kind": "bookmark", "page": 7, "percentage": 0.4, "created": T, "updated": T},
])
second = dict(base, annotations=[
    a("ko-1", P3, "second thought", T + 100),
    a("ko-2", "", "", T + 100, deleted=True),
])
third = dict(base, annotations=[a("ko-1", P3, "third thought", T + 200)])
# the owner's case: several annotations arrive in different colours
colors = dict(base, annotations=[
    a("ko-1", P3, "first thought", T + 400, color="purple"),
    a("ko-2", P5, "", T + 400, color="cyan"),
])
# same book, but the device names it by another digest: found by title and author
byname = dict(base, document="ffffffffffffffffffffffffffffffff", annotations=[a("ko-9", P3, "by title", T + 300)])
other = dict(base, title="A Book Nobody Has", authors="Nobody", document="eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
             annotations=[a("ko-1", "Some quote of an unknown book here.", "", T)])
bad = dict(base, annotations=[a("ko-1", "", "", T)])
for name, v in [("dev1", first), ("dev2", second), ("dev3", third), ("colors", colors), ("byname", byname), ("other", other), ("bad", bad)]:
    json.dump(v, open(f"{out}/{name}.json", "w"))
PY
}

P3='Paragraph 3 of chapter 1 talks about functionality, readers and the world around them in plain words.'
HL7='Paragraph 7 of chapter 3 talks about functionality'

scenario koreader_annotations 'koannot:start,wait:600,koannot:device=@D@/ko/dev1.json,koannot:merge,koannot:dump,koannot:device=@D@/ko/dev1.json,koannot:merge,koannot:addhl=1;Paragraph 7 of chapter 3 talks about functionality;from bookx,koannot:merge,koannot:pull=1,koannot:device=@D@/ko/dev2.json,koannot:merge,koannot:dump,quit' '
    has "^koannot_started: http://127.0.0.1:[0-9]+/"; check "the built-in server starts with the KOReader annotation sync on" $?
    has "^koannot_put: status=200 .*accepted=4 unchanged=0"; check "the device sends its four annotations over the same HTTP calls the plugin makes" $?
    has "^koannot_merge: added=2 linked=0 notes=0 removed=0 kept=0 conflicts=0 unplaced=1 bookmarks=1 unmatched=0 sent=0"; check "the merge adds the quote it can find, keeps the one it cannot find as an unanchored note, remembers the bookmark" $?
    has "^koannot_hl: book=1 id=[^ ]+ anchored=true spine=[0-9]+ start=[0-9]+ note=\"first thought\" source=\"KOReader \\(Kobo Libra\\)\" text=\"Paragraph 3 of chapter 1"; check "the found highlight sits at a verified place, has the device note and its provenance" $?
    has "^koannot_hl: book=1 id=[^ ]+ anchored=false .*note=\"stray note\" source=\"KOReader \\(Kobo Libra\\)\" text=\"A sentence that is not in this book"; check "a quote that is not in the book is kept unanchored with its note, not put anywhere" $?
    has "^koannot_ledger: incoming=4 outgoing=0 docs=1"; check "the ledger knows the four annotations and the matched book" $?
    has "^koannot_put: status=200 .*accepted=0 unchanged=4"; check "sending the same annotations again changes nothing on the server (idempotent)" $?
    [ "$(grep -c "^koannot_merge: added=0 linked=0 notes=0 removed=0 kept=0 conflicts=0 unplaced=0 bookmarks=0 unmatched=0 sent=0" <<<"$OUT")" -ge 1 ]; check "the second merge adds nothing and duplicates nothing" $?
    has "^koannot_added: ok"; check "a highlight made in Bookx is added" $?
    has "^koannot_merge: added=0 .*sent=1"; check "it is published to the device document" $?
    has "^koannot_pull: origin=bookx deleted=false note=\"from bookx\" text=\"Paragraph 7 of chapter 3 talks about functionality\""; check "the device would receive it with its note" $?
    has "^koannot_merge: added=0 linked=0 notes=1 removed=1 "; check "a newer note and a deletion from the device are merged" $?
    L=$(grep "^koannot_hl: " <<<"$OUT" | tail -3)
    grep -q "note=\"second thought\"" <<<"$L"; check "the note follows the newer version" $?
    ! grep -q "Paragraph 5 of chapter 3 talks about functionality, readers and the world around them in plain words" <<<"$L"; check "the highlight deleted on the device is gone, the others stay" $?
    [ "$(grep -c "\"text\": \"Paragraph 3 of chapter 1" <<<"$LIB")" -le 1 ]; check "no highlight is duplicated in the saved library" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario koreader_annotations_colours 'koannot:start,wait:600,koannot:device=@D@/ko/dev1.json,koannot:merge,koannot:device=@D@/ko/colors.json,koannot:merge,koannot:dump,quit' '
    has "^koannot_merge: added=2 "; check "first merge" $?
    has "^koannot_merge: .*looks=2"; check "two recolours from KOReader are counted" $?
    grep "^koannot_hl: " <<<"$OUT" | grep -q "color=\"purple\""; check "a purple highlight stays purple in the library" $?
    grep "^koannot_hl: " <<<"$OUT" | grep -q "color=\"cyan\""; check "a cyan highlight stays cyan, not turned into blue or yellow" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario koreader_annotations_conflicts 'koannot:start,wait:600,koannot:device=@D@/ko/dev1.json,koannot:merge,koannot:editnote=1;Paragraph 3 of chapter 1 talks about functionality;my own edit,koannot:device=@D@/ko/dev3.json,koannot:merge,koannot:dump,koannot:device=@D@/ko/byname.json,koannot:device=@D@/ko/other.json,koannot:merge,quit' '
    has "^koannot_merge: added=2 "; check "first merge" $?
    has "^koannot_merge: added=0 linked=0 notes=0 removed=0 kept=0 conflicts=1 "; check "a note changed on both sides is a conflict, counted, and nothing is overwritten" $?
    grep "^koannot_hl: " <<<"$OUT" | grep -q "note=\"my own edit\""; check "the library keeps the reader note" $?
    has "^koannot_merge: .*unmatched=1"; check "a book of the device that is not in the library is counted, not guessed" $?
    ! has "^koannot_error"; check "the device client had no error" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario koreader_annotations_refused 'koannot:start,wait:600,koannot:device=@D@/ko/bad.json,koannot:merge,koannot:dump,quit' '
    has "^koannot_put: status=422 .*accepted=null"; check "an invalid annotation makes the server refuse the whole request" $?
    has "^koannot_ledger: incoming=0 "; check "nothing of it reached the library" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario koreader_annotations_ui 'koannot:start,share:show,wait:900,a11y-audit:,library:icons,koannot:plugin=@D@/plug,koannot:plugins-folder=@D@/mounts/KOBOeReader,quit' '
    has "^a11y: controls=[0-9]+ problems=0"; check "every control of the sharing window has a name" $?
    has "^icons: checked=[0-9]+ missing=\\[\\]"; check "the icons exist" $?
    has "^koannot_plugin: .*bookx.koplugin files=.*LICENSE,.*_meta.lua.*bookxjson.lua.*bookxmd5.lua.*bookxsync.lua.*main.lua"; check "the plugin is copied with all its files" $?
    has "^koannot_plugins_folder: Some\\(\".*/.adds/koreader/plugins\"\\)"; check "KOReader on a Kobo is found under .adds" $?
    [ -f "$d/plug/bookx.koplugin/main.lua" ]; check "the plugin file is there" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'
