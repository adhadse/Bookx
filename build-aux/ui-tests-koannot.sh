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
    *live_ahead_off) mkdir -p "$d/cfg/bookx" && printf '{"share":{"offer_jump":false}}' >"$d/cfg/bookx/config.json" ;;
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

# Sync activity (the owner: "no UX which shows the device connected and what is being synced"): the device that
# synced is a row under Connected devices, what it sent is a row under Recent activity, a book Bookx does not have is
# listed (never silent) and a toast announces what arrived. Dev commands: `sync:activity`, `koannot:toast`.
scenario koreader_annotations_activity 'koannot:start,wait:600,action:sync,wait:800,sync:activity,koannot:device=@D@/ko/dev1.json,koannot:merge,koannot:toast,sync:activity,koannot:device=@D@/ko/other.json,koannot:merge,sync:activity,quit' '
    has "^devices=0 events=0 unmatched=0"; check "before anything synced: no device, no activity" $?
    has "^sync_activity_window: groups_connected=true groups_recent=true empty_state=true show_all=true"; check "the window has both groups and explains what to do when nothing has synced" $?
    has "^device: Kobo Libra \\| KOReader 2026\\.07\\.2 · plugin 0\\.3\\.1 · last sync just now \\| Connected now$"; check "the device is a row with its versions and a live status" $?
    has "^activity: UI Test Book \\| 3 highlights added or updated, 2 with notes, 1 bookmark · Kobo Libra · just now$"; check "what it sent is an activity row with its counts" $?
    has "^activity: UI Test Book \\| 1 highlight could not be placed in the book · Kobo Libra · just now \\| problem$"; check "a quote that is not in the book is a visible problem row" $?
    has "^koannot_toast: \"Synced 2 highlights, 1 bookmark from Kobo Libra for UI Test Book\"$"; check "a toast says what arrived, from which device, for which book" $?
    has "^koannot_lastsync: \"Last synced from Kobo Libra just now\"$"; check "the details of the book say where and when it last synced" $?
    has "^today: Today: 3 highlights, 1 bookmark, 2 notes$"; check "the header counts the day" $?
    has "^unmatched: A Book Nobody Has \\| 1 highlight not imported · from Kobo Libra, hash eeeeeeee$"; check "a book that is not in the library is listed with its hash" $?
    has "^activity: A Book Nobody Has \\| Book not found in your library \\(hash eeeeeeee\\): 1 highlight not imported · Kobo Libra · just now \\| problem$"; check "and is an activity row too" $?
    [ "$(grep -c "^device: " <<<"$OUT")" -ge 2 ] && ! grep -q "^device: .*\\. " <<<"$OUT"; check "one device row per dump" $?
    ! has "…|\\.\\.\\.\\s*$"; check "no ellipsis in the labels" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

# Live updates (docs/koreader-annotation-sync.md, "Live updates"): a device talks to the real in-process server while the
# book is open; the open reader, its sidebar, the Notes & Highlights view and the banner follow without reopening.
# `koannot:push=<book>|<text>|<colour>|<note>` and `koannot:progress=<book>|<0..1>|<device>` are PUTs over HTTP like the
# plugin's; nothing else is called (no `koannot:merge`): the server tells the app, the app merges, the reader repaints.
# `reader:live-state` prints what the reader holds and paints, `koannot:events` the events that were delivered.
LT3='Paragraph 3 of chapter 1 talks about functionality'
LT5='Paragraph 5 of chapter 3 talks about functionality'
LT7='Paragraph 7 of chapter 3 talks about functionality'

scenario koreader_annotations_live_reader "koannot:start,wait:600,read:1,until:page-ready:30000,wait:300,reader:live-state,koannot:push=1|$LT3|purple|first,until:sync-repaint>=1:20000,wait:400,reader:live-state,koannot:push=1|$LT3|green|first,until:sync-repaint>=2:20000,wait:400,reader:live-state,koannot:pushdel=1|$LT3,until:sync-repaint>=3:20000,wait:400,reader:live-state,koannot:events,quit" '
    L=$(grep "^live_state: " <<<"$OUT")
    sed -n 1p <<<"$L" | grep -q "highlights=0 notes=0 .*painted=0 .*events=0 "; check "the book opens with nothing painted and nothing heard" $?
    sed -n 2p <<<"$L" | grep -q "highlights=1 notes=1 .*colours=purple painted=1 .*events=1 .*repaints=1 "; check "a highlight the e-reader made is drawn on the open page once, with its note and colour" $?
    # the app is waited for (not slept for), so a slow or busy machine cannot make the state be read too early; the speed
    # is its own check: 1.5 s from the push to the painted page, times UI_LATENCY_SLACK on a machine that is known to be slow
    ms=$(grep "^until_ms: sync-repaint>=1 " <<<"$OUT" | awk "{print \$3}")
    [ -n "$ms" ] && [ "$ms" -le $((1500 * ${UI_LATENCY_SLACK:-1})) ]; check "it was drawn within 1.5 seconds of the push (took ${ms:-?} ms)" $?
    sed -n 3p <<<"$L" | grep -q "highlights=1 .*colours=green painted=1 .*events=2 .*repaints=2 "; check "the colour changed on the e-reader changes the open page without reopening" $?
    [ "$(sed -n 2p <<<"$L" | grep -o "looks=[^ ]*")" != "$(sed -n 3p <<<"$L" | grep -o "looks=[^ ]*")" ]; check "the painted colour really differs (not only the stored one)" $?
    sed -n 4p <<<"$L" | grep -q "highlights=0 notes=0 .*painted=0 .*events=3 "; check "a highlight deleted on the e-reader disappears from the page" $?
    [ "$(grep -c "^koannot_event: book=1 " <<<"$OUT")" -eq 3 ]; check "exactly one event per change reached the reader, no duplicates" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# A burst (three highlights at once) is merged once and heard once; the sidebar Notes tab and the book map follow.
scenario koreader_annotations_live_burst "koannot:start,wait:600,read:1,until:page-ready:30000,wait:300,reader:side-open,reader:side-tab=notes,wait:500,reader:live-state,koannot:push=1|$LT3|yellow|a,koannot:push=1|$LT5|blue|b,koannot:push=1|$LT7|pink,wait:2000,reader:live-state,koannot:events,quit" '
    L=$(grep "^live_state: " <<<"$OUT")
    sed -n 1p <<<"$L" | grep -q "highlights=0 .* notes=0 "; check "the sidebar starts empty" $?
    sed -n 2p <<<"$L" | grep -q "highlights=3 notes=2 .*painted=1 .* notes=3 "; check "three highlights arrive: the page paints the one of this chapter and the Notes tab lists all three" $?
    [ "$(grep -c "^koannot_event: book=1 " <<<"$OUT")" -le 2 ] && [ "$(grep -c "^koannot_event: book=1 " <<<"$OUT")" -ge 1 ]; check "the burst is one or two events, not three" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario koreader_annotations_live_bookmarks "koannot:start,wait:600,read:1,until:page-ready:30000,wait:300,reader:side-open,reader:side-tab=bookmarks,wait:500,reader:live-state,koannot:device=@D@/ko/marks.json,wait:2000,reader:live-state,quit" '
    L=$(grep "^live_state: " <<<"$OUT")
    sed -n 1p <<<"$L" | grep -q "bookmarks=0 .* bookmarks=0 "; check "no bookmarks to start with" $?
    sed -n 2p <<<"$L" | grep -q "bookmarks=4 .* bookmarks=4 "; check "bookmarks the device sent are in the library and in the sidebar list without reopening" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# The library window: the Notes & Highlights view follows too.
scenario koreader_annotations_live_library "koannot:start,wait:600,library:notes,wait:800,library:notes:dump,koannot:push=1|$LT3|purple|n1,wait:1500,library:notes:dump,koannot:push=1|$LT5|blue,wait:1500,library:notes:dump,quit" '
    L=$(grep "^notes_state: " <<<"$OUT")
    sed -n 1p <<<"$L" | grep -q "total=0 shown=0 "; check "Notes starts empty" $?
    sed -n 2p <<<"$L" | grep -q "total=1 shown=1 "; check "a highlight from the e-reader is listed while the view is open" $?
    sed -n 3p <<<"$L" | grep -q "total=2 shown=2 "; check "the next one is listed too, the counts follow" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# A note being edited is never taken from the reader: it asks.
scenario koreader_annotations_live_conflict "koannot:start,wait:600,read:1,until:page-ready:30000,wait:300,koannot:push=1|$LT3|yellow|mine0,wait:1500,reader:live-edit=0,wait:600,reader:live-type=my typing,koannot:push=1|$LT3|yellow|theirs note,wait:1500,reader:live-state,reader:live-press=primary,wait:500,reader:live-state,quit" '
    L=$(grep "^live_state: " <<<"$OUT")
    sed -n 1p <<<"$L" | grep -q "banner=This note changed on your e-reader | Use Theirs / Keep Mine editor=[^ ]*:\"my typing\""; check "the note changed on the e-reader while it is open: the editor keeps what was typed and a banner asks" $?
    sed -n 2p <<<"$L" | grep -q "banner=none editor=[^ ]*:\"theirs note\""; check "Use Theirs puts their note in the editor and the banner goes" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# Keep Mine and a note that nobody typed in.
scenario koreader_annotations_live_conflict_keep "koannot:start,wait:600,read:1,until:page-ready:30000,wait:300,koannot:push=1|$LT3|yellow|mine0,wait:1500,reader:live-edit=0,wait:600,koannot:push=1|$LT3|yellow|quiet change,wait:1500,reader:live-state,reader:live-type=typed after,koannot:push=1|$LT3|yellow|loud change,wait:1500,reader:live-state,reader:live-press=secondary,wait:500,reader:live-state,quit" '
    L=$(grep "^live_state: " <<<"$OUT")
    sed -n 1p <<<"$L" | grep -q "banner=none editor=[^ ]*:\"quiet change\""; check "nothing typed yet: the editor just shows the new note, no banner" $?
    sed -n 2p <<<"$L" | grep -q "banner=This note changed on your e-reader .* editor=[^ ]*:\"typed after\""; check "typed text is kept and the banner asks" $?
    sed -n 3p <<<"$L" | grep -q "banner=none editor=[^ ]*:\"typed after\""; check "Keep Mine keeps it and the banner goes" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# The device is ahead: a banner, never a silent move; Continue There jumps; a page turn or Dismiss ends it.
scenario koreader_annotations_live_ahead "koannot:start,wait:600,read:1,until:page-ready:30000,wait:300,reader:bm-where,koannot:progress=1|0.62|Kobo Libra,wait:1500,reader:live-state,reader:bm-where,reader:live-press=primary,wait:1200,reader:live-state,reader:bm-where,quit" '
    L=$(grep "^live_state: " <<<"$OUT")
    W=$(grep "^bm_where: " <<<"$OUT")
    sed -n 1p <<<"$L" | grep -q "banner=Your Kobo Libra is at 62% (page [0-9]*) | Continue There / Dismiss"; check "a position further along is offered with its device, percentage and page" $?
    [ "$(sed -n 1p <<<"$W")" = "$(sed -n 2p <<<"$W")" ]; check "the page did not move by itself" $?
    sed -n 2p <<<"$L" | grep -q "banner=none"; check "the banner goes after Continue There" $?
    sed -n 3p <<<"$W" | grep -q "spine=[1-9]"; check "Continue There went to the place of the device" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario koreader_annotations_live_ahead_dismiss "koannot:start,wait:600,read:1,until:page-ready:30000,wait:300,koannot:progress=1|0.5|Kobo Libra,wait:1500,reader:live-state,reader:live-press=secondary,reader:live-state,koannot:progress=1|0.5|Kobo Libra,wait:1500,reader:live-state,koannot:progress=1|0.8|Kobo Libra,wait:1500,reader:live-state,reader:next,wait:600,reader:live-state,quit" '
    L=$(grep "^live_state: " <<<"$OUT")
    sed -n 1p <<<"$L" | grep -q "banner=Your Kobo Libra is at 50%"; check "offered" $?
    sed -n 2p <<<"$L" | grep -q "banner=none"; check "Dismiss ends it" $?
    sed -n 3p <<<"$L" | grep -q "banner=none"; check "the same position is not offered again" $?
    sed -n 4p <<<"$L" | grep -q "banner=Your Kobo Libra is at 80%"; check "a new position is" $?
    sed -n 5p <<<"$L" | grep -q "banner=none"; check "turning a page takes the banner away" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario koreader_annotations_live_ahead_off "koannot:start,wait:600,read:1,until:page-ready:30000,wait:300,koannot:progress=1|0.62|Kobo Libra,wait:1500,reader:live-state,quit" '
    sed -n 1p < <(grep "^live_state: " <<<"$OUT") | grep -q "banner=none"; check "with Offer to jump switched off nothing is offered" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'
