# Scenarios of the KOReader highlight/note sync (docs/koreader-annotation-sync.md). Sourced by ui-tests.sh (uses its
# `scenario`, `check`, `has`, `$LIB`). Bookx's server runs inside the app on a random loopback port; "the device" is a
# fake KOReader client built into the dev command `koannot:device=<file>` that makes the same HTTP calls as the Lua
# plugin (PUT /bookx/annotations/<md5> with the KOSync headers). KOReader itself is never run.
# Dev commands: `koannot:start|device=F|merge|dump|pull=ID|addhl=ID;TEXT;NOTE|editnote=ID;TEXT;NOTE|delhl=ID;TEXT|plugin=DIR`.

koannot_setup() {
    local d=$1
    mkdir -p "$d/ko" "$d/plug"
    mkdir -p "$d/mounts/KOBOeReader/.adds/koreader/plugins"
    case "$d" in
    *kind_remembered) mkdir -p "$d/cfg/bookx" && printf '{"notes_kind":"bookmarks"}' >"$d/cfg/bookx/config.json" ;;
    esac
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
# page bookmarks (KOReader's own: a page, an xpointer, a chapter, a date): placed by xpointer, with a note, by chapter
# title only, by page number only. `marks_dup` is the same page as bk-1 made again on the device; `marks_upd` is the
# same set with newer times (the server sees changes: the merge runs again over them).
def b(i, page, upd, **kw):
    d = {"id": i, "kind": "bookmark", "page": page, "created": T, "updated": upd}
    d.update(kw)
    return d
def marks(upd):
    return [
        a("ko-h", P3, "first thought", upd),
        b("bk-1", 12, upd, chapter="Chapter 2", pos0="/body/DocFragment[2]/body/p[13]/text().0"),
        b("bk-2", 40, upd, note="where I stopped", chapter="Chapter 3", pos0="/body/DocFragment[3]/body/p[20]/text().0"),
        b("bk-3", 55, upd, chapter="Chapter 3"),
        b("bk-4", 77, upd),
    ]
marks_first = dict(base, annotations=marks(T))
marks_upd = dict(base, annotations=marks(T + 900))
marks_dup = dict(base, annotations=[b("bk-5", 12, T, chapter="Chapter 2", pos0="/body/DocFragment[2]/body/p[13]/text().0")])
for name, v in [("marks", marks_first), ("marks_upd", marks_upd), ("marks_dup", marks_dup), ("dev1", first), ("dev2", second), ("dev3", third), ("colors", colors), ("byname", byname), ("other", other), ("bad", bad)]:
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

# Bookmarks from KOReader (docs/koreader-annotation-sync.md): a page bookmark becomes a Bookx bookmark at a chapter and
# how far into it. By xpointer (exact), by chapter title (the chapter's start), by page number (a guess): the guesses are
# flagged. A repeat adds nothing; the same page again is linked, not added; one made in Bookx is never changed.
scenario koreader_annotations_bookmarks 'koannot:start,wait:600,library:annotate,wait:400,koannot:device=@D@/ko/marks.json,koannot:merge,koannot:dump,koannot:merge,koannot:device=@D@/ko/marks_dup.json,koannot:merge,koannot:dump,quit' '
    has "^koannot_merge: added=1 linked=0 notes=0 removed=0 kept=0 conflicts=0 unplaced=0 bookmarks=4 "; check "the first merge adds the highlight and the four bookmarks" $?
    has "^koannot_bm: book=1 id=bm-[^ ]+ spine=1 frac=0\\.[3-6][0-9] label=\"Page 12\" source=\"KOReader \\(Kobo Libra\\)\" approx=false"; check "a bookmark with an xpointer is in its chapter, half way in, from KOReader, and exact" $?
    has "^koannot_bm: book=1 id=bm-[^ ]+ spine=2 frac=0\\.[5-9][0-9] label=\"where I stopped\" .*approx=false"; check "the words the reader wrote are the bookmark, in chapter 3" $?
    has "^koannot_bm: book=1 id=bm-[^ ]+ spine=2 frac=0\\.00 label=\"Page 55\" .*approx=true"; check "a bookmark with only a chapter title is the start of that chapter and flagged as rough" $?
    has "^koannot_bm: book=1 id=bm-[^ ]+ spine=[0-9] frac=[01]\\.[0-9]+ label=\"Page 77\" .*approx=true"; check "a bookmark with only a page number is placed by Bookx page estimate and flagged" $?
    has "^koannot_bm: book=1 id=dev-b spine=0 frac=0.25 label=\"devbookmark\" source=\"\" approx=false"; check "a bookmark made in Bookx is not touched and has no origin" $?
    has "^koannot_merge: added=0 linked=0 notes=0 removed=0 kept=0 conflicts=0 unplaced=0 bookmarks=0 unmatched=0 sent=[0-9]+ looks=0 bookmarks_skipped=0"; check "a second merge adds nothing" $?
    has "^koannot_merge: .* bookmarks=0 unmatched=0 .*bookmarks_skipped=1"; check "the same page made again on the device is linked to the bookmark, not added" $?
    [ "$(grep -c "^koannot_bm: " <<<"$OUT")" -eq 10 ]; check "five bookmarks in the library after each of the two dumps, never more" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

# Notes & Highlights lists the bookmarks, with a type filter: All, Highlights, Notes, Bookmarks.
scenario koreader_annotations_bookmarks_notes 'koannot:start,wait:600,koannot:device=@D@/ko/marks.json,koannot:merge,library:notes,wait:800,library:notes:kinds,library:notes:dump,library:notes:kind=bookmarks,library:notes:dump,library:notes:kind=highlights,library:notes:dump,library:notes:kind=notes,library:notes:dump,library:notes:kind=bookmarks,library:notes:search=stopped,library:notes:dump,quit' '
    has "^notes_kinds: All\\|Highlights\\|Notes\\|Bookmarks on=all"; check "the view has the four types, All is on" $?
    has "^notes_state: total=5 shown=5 .* bookmarks=4 kind=all"; check "All lists the highlight and the four bookmarks (the sidebar count is the same total)" $?
    has "^notes_state: total=5 shown=4 .* bookmarks=4 kind=bookmarks"; check "Bookmarks lists only the four bookmarks" $?
    has "^notes_state: total=5 shown=1 .* bookmarks=0 kind=highlights"; check "Highlights hides them" $?
    has "^notes_state: total=5 shown=1 .* bookmarks=0 kind=notes"; check "Notes lists the highlight that has a note, not the bookmarks" $?
    has "^notes_state: total=5 shown=1 .*bookmarks=1 kind=bookmarks"; check "a search finds a bookmark by the words the reader wrote" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

# The choice of type is remembered: kept in the settings when it is made, and read when the view is built.
scenario koreader_annotations_bookmarks_kind_remembered 'library:notes,wait:600,library:notes:kinds,library:notes:dump,library:notes:kind=highlights,wait:300,quit' '
    has "^notes_kinds: .* on=bookmarks"; check "a view built with the choice Bookmarks in the settings starts on Bookmarks" $?
    has "^notes_state: total=0 shown=0 .* kind=bookmarks"; check "and filters by it" $?
    grep -q "\"notes_kind\": \"highlights\"" "$d/cfg/bookx/config.json"; check "a new choice is written to the settings" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

# A click on a bookmark opens the book at it: its chapter, and how far into it (not the start of the chapter).
scenario koreader_annotations_bookmarks_open 'koannot:start,wait:600,koannot:device=@D@/ko/marks.json,koannot:merge,library:notes,wait:800,library:notes:kind=bookmarks,library:notes:search=where I stopped,wait:400,library:notes:open-first,wait:3500,reader:bm-where,quit' '
    has "^goto_bookmark: id=bm-[^ ]+ spine=2 frac=0\\.[5-9]"; check "the reader goes to the bookmark: chapter 3, well into it" $?
    has "^bm_where: spine=2 page=[3-5]/[0-9]+ frac=0\\.[3-9]"; check "the book is open there (the page shown is the spread that holds 76 % of chapter 3)" $?
    ! has "bm_where: spine=2 page=1/"; check "and that is not the first page of the chapter" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

# The reader's own Bookmarks tab lists them too, with where they came from.
scenario koreader_annotations_bookmarks_reader 'koannot:start,wait:600,koannot:device=@D@/ko/marks.json,koannot:merge,read:1,wait:3000,reader:side-state,quit' '
    has "^side_bookmark: .* / Page 12 · .* · KOReader \\(Kobo Libra\\)$"; check "the sidebar lists the synced bookmark with its origin" $?
    has "^side_bookmark: .* / Page 55 · .* · KOReader \\(Kobo Libra\\) · approximate place$"; check "a rough place says so" $?
    has "^side: .* bookmarks=4 "; check "all four are listed" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

# Deleting a bookmark takes it away with an Undo; deleted for good, it is not brought back by the next sync even when
# the device sends it again (changed); nothing the device does deletes one here.
scenario koreader_annotations_bookmarks_delete 'koannot:start,wait:600,koannot:device=@D@/ko/marks.json,koannot:merge,library:notes,wait:800,library:notes:kind=bookmarks,library:notes:dump,library:notes:delete-first,wait:300,library:notes:dump,library:undo-last,wait:300,library:notes:dump,library:notes:delete-first,wait:300,koannot:device=@D@/ko/marks_upd.json,koannot:merge,koannot:dump,library:notes:dump,quit' '
    has "^toast: Bookmark removed \\[undo\\]"; check "deleting a bookmark shows a message with Undo" $?
    [ "$(grep -c "^notes_state: total=5 shown=4 .* kind=bookmarks" <<<"$OUT")" -eq 2 ]; check "all four before, and all four again after Undo" $?
    has "^notes_state: total=4 shown=3 .* kind=bookmarks"; check "one fewer after the delete" $?
    has "^koannot_merge: added=0 linked=0 notes=0 removed=0 kept=0 conflicts=0 unplaced=0 bookmarks=0 "; check "the device sends them again, changed: nothing is added" $?
    [ "$(grep -c "^koannot_bm: " <<<"$OUT")" -eq 3 ]; check "the deleted bookmark did not come back" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'
