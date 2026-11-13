# Scenarios of the library and reader UX gaps (docs/roadmap-status.md: L36/R36 keyboard shortcuts, L5/L29 shelves,
# L11 library switcher, L27/C10 jobs, R12/R24/R26/R27/R34/R42/R44/R47 reader). Sourced by ui-tests.sh (uses its
# `scenario`, `check`, `has`, $ROOT). Dev commands: `keys:…` (src/ui/shortcuts_dialog.rs) and the ones named in each block.

scenario shortcuts_rebind 'prefs:,wait:600,keys:open,keys:dump=app.palette,keys:press=app.palette|<Control>j,keys:dump=app.palette,keys:press=app.preferences|<Control>j,wait:300,keys:settle=swap,keys:dump=app.palette,keys:dump=app.preferences,keys:press=app.about|<Control>q,keys:settle=take,keys:dump=app.quit,keys:dump=app.about,keys:press=lib.select-all|a,keys:filter=ctrl+b,keys:filter=,quit' '
    K=$(grep "^keys: app\." <<<"$OUT")
    sed -n 1p <<<"$K" | grep -q "effective=\[\"<Control><Shift>p\", \"<Control>k\"\].*gtk=\[\"<Shift><Control>p\", \"<Control>k\"\]"; check "the default palette keys are the application accelerators" $?
    sed -n 2p <<<"$K" | grep -q "app.palette effective=\[\"<Control>j\"\].*gtk=\[\"<Control>j\"\].*customised=true"; check "rebinding changes the live accelerator" $?
    has "^keys: press app.preferences <Control>j -> conflict"; check "a key that another action has is reported as a conflict" $?
    has "^keys: settle swap -> swapped"; check "the conflict can be settled by swapping" $?
    sed -n 3p <<<"$K" | grep -q "app.palette effective=\[\"<Control>comma\"\]"; check "after the swap the palette has the Preferences key" $?
    sed -n 4p <<<"$K" | grep -q "app.preferences effective=\[\"<Control>j\"\].*gtk=\[\"<Control>j\"\]"; check "and Preferences has the palette key" $?
    has "^keys: settle take -> taken"; check "the conflict can be settled by taking the key away" $?
    sed -n 5p <<<"$K" | grep -q "app.quit effective=\[\].*gtk=\[\]"; check "the action that lost its key has no accelerator" $?
    sed -n 6p <<<"$K" | grep -q "app.about effective=\[\"<Control>q\"\]"; check "the new owner has it" $?
    has "^keys: press lib.select-all a -> needs-modifier"; check "a plain letter is refused for a library action" $?
    has "^keys: filter=\"ctrl\+b\" shown=1 "; check "the filter finds the action by its keys" $?
    has "^keys: filter=\"\" shown=([2-9][0-9]|[1-9][0-9][0-9]) of="; check "an empty filter shows every action" $?
    [ -s "$d/cfg/bookx/shortcuts.json" ] && grep -q "app.quit" "$d/cfg/bookx/shortcuts.json"; check "the choices are saved in the config folder" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario shortcuts_files 'prefs:,wait:600,keys:press=reader.reload|F5,keys:press=reader.bookmark|<Control>j,keys:export=@D@/mine.json,keys:resetall,keys:dump=reader.reload,keys:import=@D@/mine.json,keys:dump=reader.reload,keys:reset=reader.reload,keys:dump=reader.reload,keys:dump=reader.bookmark,keys:clear=reader.bookmark,keys:dump=reader.bookmark,quit' '
    K=$(grep "^keys: reader\." <<<"$OUT")
    grep -q "export true" <<<"$OUT"; check "shortcuts can be exported to a file" $?
    grep -q "reader.reload" "$d/mine.json" && grep -q "F5" "$d/mine.json"; check "the exported file lists the changes" $?
    sed -n 1p <<<"$K" | grep -q "reader.reload effective=\[\"<Control>r\"\].*customised=false"; check "Reset All brings the defaults back" $?
    sed -n 2p <<<"$K" | grep -q "reader.reload effective=\[\"F5\"\].*customised=true"; check "importing the file restores the changes" $?
    sed -n 3p <<<"$K" | grep -q "reader.reload effective=\[\"<Control>r\"\].*customised=false"; check "one action can be reset alone" $?
    sed -n 4p <<<"$K" | grep -q "reader.bookmark effective=\[\"<Control>j\"\]"; check "resetting one leaves the others" $?
    sed -n 5p <<<"$K" | grep -q "reader.bookmark effective=\[\]"; check "an action can have no shortcut" $?'

scenario shortcuts_loaded 'read:1,wait:2500,reader:key=<Control>r,reader:key=F5,reader:key=Right,keys:dump=app.quit,quit' '
    has "^reader_key: <Control>r -> disabled"; check "the default key of a rebound reader action does nothing" $?
    has "^reader_key: F5 -> reader.reload"; check "the key from the saved file runs the action in the reader" $?
    has "^reader_key: Right -> default reader.next-page"; check "keys that were not changed keep their default (the built-in handling runs them)" $?
    has "^keys: app.quit effective=\[\] .*gtk=\[\]"; check "the shortcuts file is read at start-up (Quit has no key)" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario shelves 'library:shelf=create=Favourites,library:shelf=create=To Read,library:shelf=create=favourites,library:shelf=create=  ,library:shelf=drop=Favourites|1+2,library:shelf=drop=Favourites|1,library:shelf=drop=Favourites|junk,library:shelf=add=To Read|2,library:shelf=dump,library:shelf=open=Favourites,wait:700,library:visible,library:shelf=open=To Read,wait:700,library:visible,library:search=shelf:Favourites -shelf:"To Read",wait:700,library:visible,library:search=shelf:Nope,wait:700,library:visible,library:search=,library:shelf=bookup=Favourites|2,library:shelf=bookup=Favourites|1,library:shelf=open=Favourites,wait:700,library:visible,library:shelf=rename=To Read|Later,library:shelf=up=Later,library:shelf=dump,library:shelf=remove=Favourites|2,library:shelf=menu,library:shelf=delete=Later,library:shelf=dump,library:selectall,wait:600,library:shelf=addsel=Favourites,library:shelf=dump,quit' '
    S=$(grep "^shelves: " <<<"$OUT")
    sed -n 1p <<<"$S" | grep -q "created Favourites"; check "a shelf can be created" $?
    sed -n 3p <<<"$S" | grep -q "error.*already exists"; check "a second shelf with the same name (other case) is refused" $?
    sed -n 4p <<<"$S" | grep -q "error.*Give the shelf a name"; check "an empty name is refused" $?
    sed -n 5p <<<"$S" | grep -q "drop handled=true"; check "dropping books on a shelf is handled" $?
    has "^shelves: drop handled=true said=Added 2 books to .Favourites"; check "the drop adds both books" $?
    has "^shelves: drop handled=true said=Already on .Favourites"; check "dropping them again says they are already there" $?
    has "^shelves: drop handled=false"; check "a drop that is not a book list is refused" $?
    has "^shelves: list Favourites=2:1\+2 ; To Read=1:2"; check "shelves list their books and counts" $?
    V=$(grep "^visible:" <<<"$OUT")
    sed -n 1p <<<"$V" | grep -q "^visible: 2 "; check "the shelf search shows the two books on Favourites" $?
    sed -n 2p <<<"$V" | grep -q "^visible: 1 | \[2\]"; check "To Read shows just its book" $?
    sed -n 3p <<<"$V" | grep -q "^visible: 1 | \[1\]"; check "-shelf: removes the books of a shelf" $?
    sed -n 4p <<<"$V" | grep -q "^visible: 0 "; check "a shelf that does not exist shows nothing" $?
    sed -n 5p <<<"$V" | grep -q "^visible: 2 | \[1, 2\]"; check "a shelf shows its books in the order the user gave" $?
    has "^shelves: list .*Later=1:2 ; Favourites"; check "shelves can be moved, and renaming keeps the books" $?
    has "^shelves: Removed 1 book from .Favourites"; check "a book can be taken off a shelf" $?
    has "^shelves: menu Later\|Favourites\|New Shelf"; check "the book menu lists the shelves and offers a new one" $?
    has "^shelves: list Favourites=1:1$"; check "deleting a shelf keeps the other shelf and the books" $?
    has "^shelves: Added 1 book to .Favourites.* already there|^shelves: Added 1 book to .Favourites"; check "the selected books can be put on a shelf" $?
    [ -s "$d/cfg/bookx/shelves.json" ]; check "the order is saved in the config folder" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario library_switcher 'library:libs=dump,library:libs=folder=@D@/lib2,wait:3500,library:dump,library:libs=dump,library:libs=rename=lib2|Second,library:libs=rename=lib|Second,library:libs=dump,library:libs=switch=lib,wait:3500,library:dump,library:libs=remove=Second,library:libs=remove=lib,library:libs=dump,library:libs=new=Fresh|@D@/newparent,wait:3500,library:dump,library:libs=dump,library:libs=add=Ghost|@D@/ghost,library:libs=switch=Ghost,library:libs=new=Fresh|@D@/newparent,quit' '
    L=$(grep "^libs: " <<<"$OUT")
    sed -n 1p <<<"$L" | grep -q "^libs: list lib\* label=lib"; check "the library in use is listed and current, and named on the button" $?
    sed -n 2p <<<"$L" | grep -q "added lib2"; check "a folder is added to the list under its own name" $?
    T=$(grep "^title:" <<<"$OUT")
    sed -n 1p <<<"$T" | grep -q "1 book"; check "switching opens the other library (one book)" $?
    sed -n 3p <<<"$L" | grep -q "list lib2\* ; lib label=lib2"; check "the new library is current, the old one stays in the list" $?
    sed -n 4p <<<"$L" | grep -q "renamed Second"; check "a library can be renamed" $?
    sed -n 5p <<<"$L" | grep -q "error .*already a library called"; check "renaming to a name in use is refused" $?
    sed -n 6p <<<"$L" | grep -q "list Second\* ; lib label=Second"; check "the button shows the new name" $?
    sed -n 2p <<<"$T" | grep -q "2 books"; check "switching back shows the first library again" $?
    has "^libs: .*Second. removed from the list; its files were not touched"; check "removing from the list says the files stay" $?
    [ -f "$d/lib2/bookx-library.db" ]; check "the removed library is still on disk" $?
    has "^libs: The library in use stays in the list"; check "the library in use cannot be removed from the list" $?
    has "^libs: created Fresh"; check "a new library is made in a folder of its own" $?
    [ -d "$d/newparent/Fresh" ]; check "the folder exists" $?
    sed -n 3p <<<"$T" | grep -q "0 books"; check "the new library is empty and open" $?
    has "^libs: list Fresh\* ; lib label=Fresh"; check "the new library is current" $?
    has "^libs: The folder of .Ghost. is not available"; check "a library whose folder is gone says so instead of failing" $?
    has "^libs: error There is already a library called .Fresh"; check "a new library cannot take a name in use" $?
    [ -s "$d/cfg/bookx/libraries.json" ]; check "the list is saved in the config folder" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario jobs_panel 'jobs:dump,jobs:start=convert|Convert Book|cancel,jobs:start=send|Send Other|no,jobs:progress=2|0.4|2 of 5,jobs:log=2|first line,jobs:cancel=3,jobs:cancel=2,jobs:dump,jobs:finish=2|ok,jobs:finish=3|boom,jobs:start=news|Still going|cancel,jobs:progress=4|0.5|half,jobs:open,wait:600,jobs:rows,jobs:close,jobs:dump,jobs:clear,jobs:dump,quit' '
    J=$(grep "^jobs: " <<<"$OUT")
    sed -n 1p <<<"$J" | grep -q "running=0 1|Import|Done|1.00|Importing 2 files|cancellable=true|log=[0-9]"; check "the import at start-up is listed as a finished job with its log" $?
    has "^jobs: cancel 3 accepted=false"; check "a job that cannot be stopped refuses to be cancelled" $?
    has "^jobs: cancel 2 accepted=true"; check "a job that can be stopped accepts" $?
    has "^jobs: running=2 .*2\|Conversion\|Cancelling…\|0.40\|Convert Book\|cancellable=true\|log=2"; check "a cancel request shows while the job winds down, with progress and its log" $?
    J2=$(grep "^jobs: running=" <<<"$OUT")
    has "^jobs: running=1 .*2\|Conversion\|Cancelled\|0.40"; check "a cancelled job ends as Cancelled" $?
    has "^jobs: running=1 .*3\|Send\|Failed: boom"; check "a failed job shows the reason" $?
    has "^jobs: rows .*Still going / Running · half"; check "the open panel lists the running job with its step" $?
    has "^jobs: rows .*Send Other / Failed: boom"; check "and the failed one" $?
    has "^jobs: running=1 4\|News\|Running\|0.50\|Still going"; check "clearing keeps what is still running" $?
    ! grep "^jobs: running=1 4" <<<"$OUT" | grep -q "Convert Book"; check "and removes the finished jobs" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# a book with printed pages (EPUB page-list): two chapters, page i and ii in the first, page 1 in the second
PL="$ROOT/pagelist-epub"
mkdir -p "$PL/src/META-INF" "$PL/src/OEBPS" "$PL/lib"
printf 'application/epub+zip' >"$PL/src/mimetype"
cp "$EP/src/META-INF/container.xml" "$PL/src/META-INF/"
cat >"$PL/src/OEBPS/content.opf" <<'XML'
<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">pl-test</dc:identifier><dc:title>Printed Pages Book</dc:title><dc:creator>Page Author</dc:creator><dc:language>en</dc:language></metadata><manifest><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/><item id="c2" href="c2.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c1"/><itemref idref="c2"/></spine></package>
XML
cat >"$PL/src/OEBPS/nav.xhtml" <<'XML'
<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>Nav</title></head><body>
<nav epub:type="toc"><ol><li><a href="c1.xhtml">First Chapter</a></li><li><a href="c2.xhtml">Second Chapter</a></li></ol></nav>
<nav epub:type="page-list" hidden=""><ol><li><a href="c1.xhtml#pg-i">i</a></li><li><a href="c1.xhtml#pg-ii">ii</a></li><li><a href="c2.xhtml#pg-1">1</a></li></ol></nav>
</body></html>
XML
{
    printf '<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml"><head><title>c1</title></head><body><h1 id="pg-i">First Chapter</h1>'
    for i in $(seq 1 30); do printf '<p>Filler paragraph %s of the first chapter with enough words to fill a page of the reader and then some more.</p>' "$i"; done
    printf '<p id="pg-ii">Second printed page starts here.</p>'
    for i in $(seq 31 50); do printf '<p>Filler paragraph %s of the first chapter with enough words to fill a page of the reader and then some more.</p>' "$i"; done
    printf '</body></html>'
} >"$PL/src/OEBPS/c1.xhtml"
{
    printf '<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml"><head><title>c2</title></head><body><h1 id="pg-1">Second Chapter</h1>'
    for i in $(seq 1 40); do printf '<p>Filler paragraph %s of the second chapter with enough words to fill a page of the reader and then some more.</p>' "$i"; done
    printf '</body></html>'
} >"$PL/src/OEBPS/c2.xhtml"
(cd "$PL/src" && zip -q -X -0 "$PL/lib/printed.epub" mimetype && zip -q -X -r "$PL/lib/printed.epub" META-INF OEBPS)

scenario reader_goto "library:import=$ROOT/pagelist-epub/lib,wait:2500,read:3,wait:3500,reader:pagemap,reader:goto-text=ii,wait:2500,reader:anchor=pg-ii,reader:anchor=pg-i,reader:pagemap,reader:jumps,reader:goto-text=1,wait:800,reader:pagemap,reader:goto-text=99,reader:goto-text=ch 2,wait:800,reader:jumps,reader:goto-text=50%,wait:800,reader:jumps,reader:back,wait:800,reader:jumps,reader:back,wait:600,reader:back,wait:600,reader:jumps,reader:forward,wait:600,reader:jumps,reader:goto-text=loc 5,wait:600,reader:jumps,reader:goto-text=loc 99999,reader:goto-dialog,wait:600,quit" '
    P=$(grep "^reader_pagemap:" <<<"$OUT")
    sed -n 1p <<<"$P" | grep -q "printed=true pages=3 footer=\"Page i\""; check "a book with a page-list shows its printed page (Page i)" $?
    sed -n 2p <<<"$P" | grep -q "footer=\"Page ii\""; check "going to a printed page label lands on that page" $?
    sed -n 3p <<<"$P" | grep -q "footer=\"Page 1\""; check "the next printed page is in the next chapter" $?
    has "^reader_goto: ii -> Ok"; check "a printed page label is accepted" $?
    has "^reader_goto: 99 -> Err.*no page"; check "a page the book does not have is explained" $?
    has "^reader_goto: ch 2 -> Ok"; check "a chapter number goes to that chapter" $?
    has "^reader_goto: loc 99999 -> Err.*locations"; check "a location past the end is explained" $?
    J=$(grep "^reader_jumps:" <<<"$OUT")
    sed -n 1p <<<"$J" | grep -q "back=1 forward=0 .*back_btn=true forward_btn=false"; check "a jump shows the Back button" $?
    sed -n 2p <<<"$J" | grep -q "back=3 forward=0"; check "every jump is remembered" $?
    sed -n 4p <<<"$J" | grep -q "back=2 forward=1 .*forward_btn=true"; check "going back makes Forward available" $?
    sed -n 5p <<<"$J" | grep -q "back=0 forward=3 .*back_btn=false"; check "back all the way empties the back list" $?
    sed -n 6p <<<"$J" | grep -q "back=1 forward=2"; check "forward walks the way back again" $?
    sed -n 7p <<<"$J" | grep -q "back=2 forward=0"; check "a new jump drops the forward list" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# a book with a picture in the middle of a page
IMG="$ROOT/image-epub"
mkdir -p "$IMG/src/META-INF" "$IMG/src/OEBPS" "$IMG/lib"
printf 'application/epub+zip' >"$IMG/src/mimetype"
cp "$EP/src/META-INF/container.xml" "$IMG/src/META-INF/"
cat >"$IMG/src/OEBPS/content.opf" <<'XML'
<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">img-test</dc:identifier><dc:title>Picture Book</dc:title><dc:creator>Pic Author</dc:creator><dc:language>en</dc:language></metadata><manifest><item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/><item id="fig" href="fig.png" media-type="image/png"/></manifest><spine><itemref idref="c1"/></spine></package>
XML
printf '<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml"><head><title>c</title></head><body><h1>Picture</h1><p>Words before the picture.</p><p><img src="fig.png" alt="A figure"/></p><p>Words after the picture.</p></body></html>' >"$IMG/src/OEBPS/c1.xhtml"
python3 - "$IMG/src/OEBPS/fig.png" <<'PY'
import struct, sys, zlib
w, h = 640, 400
raw = b"".join(b"\x00" + bytes(((x * 255) // w, (y * 255) // h, 128) [i % 3] for x in range(w) for i in range(3)) for y in range(h))
def chunk(t, d):
    c = struct.pack(">I", len(d)) + t + d
    return c + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")
open(sys.argv[1], "wb").write(png)
PY
(cd "$IMG/src" && zip -q -X -0 "$IMG/lib/picture.epub" mimetype && zip -q -X -r "$IMG/lib/picture.epub" META-INF OEBPS)

scenario image_viewer "library:import=$ROOT/image-epub/lib,wait:2500,read:3,wait:3500,reader:image-click,wait:1200,reader:image-viewer=dump,reader:image-viewer=in,reader:image-viewer=in,wait:500,reader:image-viewer=pan=40+20,wait:400,reader:image-viewer=dump,reader:image-viewer=actual,reader:image-viewer=pinch=2,reader:image-viewer=fit,reader:image-viewer=toggle,reader:image-viewer=copy,reader:image-viewer=save=@D@/saved.png,reader:image-viewer=close,wait:600,reader:image-viewer=dump,quit" '
    I=$(grep "^image_viewer:" <<<"$OUT")
    sed -n 5p <<<"$I" | grep -q "scroll=290,184"; check "dragging or scrolling pans the zoomed picture (40,20 further)" $?
    has "^reader_image: click OEBPS/fig.png"; check "a click in the middle of a picture is seen as a click on that picture" $?
    sed -n 1p <<<"$I" | grep -q "image=640x400"; check "the viewer opens on the picture at its real size" $?
    sed -n 1p <<<"$I" | grep -q "fit=true"; check "it starts fitted to the window" $?
    sed -n 3p <<<"$I" | grep -q "percent=[0-9]* .*fit=false"; check "zooming in leaves the fit" $?
    sed -n 6p <<<"$I" | grep -q "^image_viewer: actual percent=100 size=640x400"; check "actual size is 100%" $?
    sed -n 7p <<<"$I" | grep -q "^image_viewer: pinch percent=200 size=1280x800"; check "a pinch of 2x doubles the size" $?
    sed -n 8p <<<"$I" | grep -q "fit=true"; check "Fit shows the whole picture again" $?
    sed -n 9p <<<"$I" | grep -q "^image_viewer: toggle percent=100"; check "a double click toggles between fit and 100%" $?
    has "^image_viewer: copy clipboard_has_image=true"; check "the picture can be copied to the clipboard" $?
    has "^image_viewer: save true"; check "the picture can be saved" $?
    cmp -s "$d/saved.png" "$IMG/src/OEBPS/fig.png"; check "the saved file is the picture of the book, byte for byte" $?
    has "^image_viewer: closed"; check "the viewer closes" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# a long book for the book map (about 900 contents entries: some hundreds of pages)
mkdir -p "$ROOT/mapbook/lib"
python3 "$HERE/make-big-epub.py" "$ROOT/mapbook/lib/map.epub" 900 >/dev/null

# Every change to the look is the book's own; new books start from the defaults (enhancement 19)
scenario book_looks 'read:1,wait:2500,reader:profile=set=font_size|20,wait:300,reader:profile=set=margin|90,wait:300,reader:profile=dump,reader:use-default,wait:300,reader:profile=set=font_size|12,wait:300,reader:profile=dump,reader:reset-look,wait:500,reader:profile=dump,reader:profile=set=font_size|17,wait:800,close:,wait:1500,read:1,wait:2500,reader:profile=dump,close:,wait:1500,read:2,wait:2500,reader:profile=dump,quit' '
    D=$(grep "^reader_profile: own=" <<<"$OUT")
    sed -n 1p <<<"$D" | grep -q "own=true .*now=size20,margin90"; check "a change to the look is kept as the book'"'"'s own look" $?
    sed -n 2p <<<"$D" | grep -q "defaults=size20,margin90"; check "Use as default for new books makes the current look the default" $?
    sed -n 2p <<<"$D" | grep -q "now=size12"; check "later changes only touch this book" $?
    sed -n 3p <<<"$D" | grep -q "own=false .*defaults=size20.*now=size20,margin90"; check "Reset this book to defaults forgets its own look and uses the defaults" $?
    sed -n 4p <<<"$D" | grep -q "own=true .*now=size17"; check "the book opens again with its own look" $?
    grep -m1 "\"font_size\"" "$d/cfg/bookx/config.json" | grep -q ": 20"; check "the settings file holds the defaults, not the open book'"'"'s look" $?
    sed -n 5p <<<"$D" | grep -q "own=false .*now=size20,margin90"; check "another book starts from the defaults" $?
    grep -q "\"book_looks\"" "$d/cfg/bookx/config.json"; check "book looks are saved in the settings" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# The old named profiles are migrated at the first start
scenario profiles_migrate 'read:1,wait:2500,reader:profile=dump,quit' '
    has "^reader_profile: own=true .*now=size24"; check "a profile pinned to a book became that book'"'"'s own look" $?
    grep -q "Spare" "$d/cfg/bookx/reading-profiles-backup.json" && ! grep -q "Big" "$d/cfg/bookx/reading-profiles-backup.json"; check "unpinned profiles are kept in a backup file" $?
    ! grep -q "\"profiles\"" "$d/cfg/bookx/config.json"; check "the settings no longer hold profiles" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# Custom themes sit next to Light / Sepia / Dark / Black in the reader menu (enhancement 19)
scenario reader_themes 'read:1,wait:2500,reader:theme-row,reader:profile=theme-save=Paper|#222222/#f5f0e6,reader:profile=theme-save=Ink Night|#dddddd/#101010,reader:profile=theme-save=Bad|red/#000000,wait:300,reader:theme-row,reader:profile=theme-apply=Ink Night,wait:500,reader:theme-row,reader:profile=dump,reader:profile=theme-export=@D@/themes.json,reader:theme-edit=Paper,wait:800,reader:profile=theme-delete=Paper,reader:profile=theme-delete=Ink Night,wait:300,reader:theme-row,reader:profile=theme-import=@D@/themes.json,wait:300,reader:theme-row,reader:profile=theme-apply=Paper,wait:400,reader:theme-row,quit' '
    R=$(grep "^reader_theme_row:" <<<"$OUT")
    sed -n 1p <<<"$R" | grep -q "^reader_theme_row: $"; check "without custom themes the row only has the + swatch" $?
    sed -n 2p <<<"$R" | grep -q "Paper:off,Ink Night:off"; check "a saved theme gets a swatch next to the built-in ones" $?
    sed -n 3p <<<"$R" | grep -q "Paper:off,Ink Night:on"; check "applying a theme marks its swatch" $?
    has "^reader_profile: theme saved Paper"; check "a colour theme can be saved" $?
    has "^reader_profile: error Colours are written like"; check "a colour that is not a colour is refused" $?
    grep "^reader_profile: own=" <<<"$OUT" | sed -n 1p | grep -q "themes=Paper(#222222/#f5f0e6) ; Ink Night(#dddddd/#101010) now=.*colorsCustom,fg#dddddd,bg#101010"; check "applying a theme gives the reader those colours" $?
    grep -q "Ink Night" "$d/themes.json" && grep -q "#f5f0e6" "$d/themes.json"; check "themes export to a JSON file" $?
    sed -n 4p <<<"$R" | grep -q "^reader_theme_row: $"; check "deleting the themes takes their swatches away" $?
    has "^reader_profile: theme imported Ok\(2\)"; check "the exported file imports again" $?
    sed -n 5p <<<"$R" | grep -q "Paper:off,Ink Night:on"; check "imported themes get swatches (the one in use is marked)" $?
    sed -n 6p <<<"$R" | grep -q "Paper:on"; check "an imported theme can be applied" $?
    grep -q "reader_themes" "$d/cfg/bookx/config.json"; check "themes are kept in the settings, shared by all books" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# The book map: pages, time bars, markers, wheel, zoom, drag (enhancement 18)
scenario book_map "library:import=$ROOT/mapbook/lib,wait:3000,read:3,wait:3500,reader:dwell=0.30|60,reader:dwell=0.31|20,reader:bookmark,wait:300,reader:hl-add=1|50|yellow,reader:hl-add=1|60|green,reader:hl-add=1|70|blue|underline,reader:hl-add=2|50|red|strikethrough|a note,reader:hl-add=3|50|purple,wait:300,reader:map,wait:900,reader:map-dump,reader:map-wheel=1,reader:map-wheel=1,reader:map-wheel=1,reader:map-wheel=1|ctrl,reader:map-wheel=1|ctrl,reader:map-hover=40,reader:map-key=End,reader:map-key=Home,reader:map-drag=2|300,wait:1000,reader:jumps,reader:map,wait:900,reader:map-dump,reader:map-thumb=200,reader:map-thumb=200,reader:map-thumb=400,reader:map-hover=210,wait:1500,reader:map-dump,reader:map-thumb=297|/tmp/opencode/thumb297.png,reader:map-close,quit" '
    M=$(grep "^book_map:" <<<"$OUT")
    sed -n 1p <<<"$M" | grep -qE "^book_map: pages=[0-9]{3,} level=Detail ppc=1 grid=[0-9]+x[0-9]+ first_row=0 cursor=[0-9]+ current=[0-9]+ chapters=[0-9]{2,} read=[1-9][0-9]* bookmarks=1 "; check "the map shows every page of the book, the chapters, the pages that were read and the bookmark" $?
    sed -n 1p <<<"$M" | grep -q "highlights=5 notes=1 .*colours=\[33cc6b,408cff,b873f2,ff6152,ffd91f\] stack=[1-3] "; check "highlights keep their own colours (yellow, green, blue, red, purple) and a note adds a pencil" $?
    T1=$(sed -n 1p <<<"$M" | sed -E "s/.* pages=([0-9]+).*/\1/")
    sed -n 4p <<<"$M" | grep -qE "first_row=[1-9]"; check "the wheel scrolls the map by rows" $?
    sed -n 5p <<<"$M" | grep -q "level=Section"; check "Ctrl+wheel zooms out one level" $?
    sed -n 6p <<<"$M" | grep -q "level=Book"; check "and once more to the whole book" $?
    sed -n 7p <<<"$M" | grep -qE "^book_map: pages=.* card=true"; check "hovering a page shows the preview card" $?
    sed -n 7p <<<"$M" | grep -qE "preview=\"[^\"]+ · Page [0-9]+ · [0-9]+%\""; check "the preview names the chapter, the page and the place in the book" $?
    sed -n 8p <<<"$M" | grep -qE "cursor=$((T1 - 1)) "; check "End goes to the last page" $?
    sed -n 9p <<<"$M" | grep -q "cursor=0 "; check "Home goes to the first page" $?
    has "^book_map: dragged 2 to"; check "dragging through the book and letting go goes to the page under the pointer" $?
    J=$(grep "^reader_jumps:" <<<"$OUT"); sed -n 1p <<<"$J" | grep -q "back=1 forward=0 spine=[1-9]"; check "the jump moved the reader and is remembered" $?
    sed -n 11p <<<"$M" | grep -qE "history=\[1@[0-9]+\]"; check "the place you jumped from is a numbered marker (1)" $?
    sed -n 12p <<<"$M" | grep -qE "^book_map: thumb page=200 size=(200|[0-9]+)x[0-9]+ col=[0-9]/[0-9] ink=[1-9][0-9]+ ms=[0-9]+ made=[0-9]+ layout=[0-9]+ goto=[0-9]+ paint=[0-9]+ cached=(true|false) layouts=[1-9] paints=[1-9]"; check "the hover card has a real page: a thumbnail of page 200 with text on it (not blank)" $?
    W=$(sed -n 12p <<<"$M" | sed -E "s/.*size=([0-9]+)x([0-9]+).*/\1 \2/"); set -- $W; [ "$1" -ge 180 ] && [ "$1" -le 220 ] && [ "$2" -ge 100 ]; check "it is 180 to 220 pixels wide and has the shape of the reader page (wider than 100 px)" $?
    sed -n 12p <<<"$M" | sed -E "s/.* ms=([0-9]+) .*/\1/" | { read ms; [ "$ms" -lt 4000 ]; }; check "the first thumbnail of a chapter (layout included) is made within the 4 s budget of a debug build" $?
    sed -n 13p <<<"$M" | grep -qE "ms=[0-9] made=[0-9]+ .*cached=true"; check "the same page again comes from the cache at once (no new layout, no new painting)" $?
    sed -n 14p <<<"$M" | grep -qE "thumb page=400 .*ink=[1-9][0-9]+ .*cached=false layouts=[1-9] paints=[2-9]"; check "another page is made with at most one more chapter layout" $?
    sed -n 16p <<<"$M" | grep -qE "thumb=2[0-9]*:"; check "hovering a page puts its thumbnail in the card within a moment" $?
    [ -s "$d/cfg/bookx/dwell.json" ] && grep -q "buckets" "$d/cfg/bookx/dwell.json"; check "the time spent on pages is kept in a file of its own" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# Well-known keys of other readers (enhancement 21): the registry and what the reader does with them
scenario reader_new_keys 'read:1,wait:2500,reader:key=<Control>t,reader:key=m,reader:key=<Control>i,reader:key=<Control>0,reader:key=<Control>w,reader:key=Right,reader:key=<Control>F12,quit' '
    has "^reader_key: <Control>t -> default reader.contents"; check "Ctrl+T is a default of Contents" $?
    has "^reader_key: m -> default reader.map"; check "M is a default of the book map" $?
    has "^reader_key: <Control>i -> default reader.properties"; check "Ctrl+I is a default of Book Properties" $?
    has "^reader_key: <Control>0 -> default reader.reset-size"; check "Ctrl+0 is a default of Reset Text Size" $?
    has "^reader_key: <Control>w -> default reader.back"; check "Ctrl+W is a default of Back to the Library" $?
    has "^reader_key: Right -> default reader.next-page"; check "the page keys are defaults of the page actions too" $?
    has "^reader_key: <Control>F12 -> builtin"; check "a key that is nobody'"'"'s default is left alone" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# The thumbnail is laid out exactly like the reader's own page (same chapter layout fingerprint), made fast
# and ahead of time (enhancement 18 follow-up)
scenario map_thumb_faithful "library:import=$ROOT/mapbook/lib,wait:3000,read:3,wait:3500,reader:map,wait:2500,reader:map-stats,reader:map-thumb=0,reader:map-thumb=2,reader:map-thumb=7,reader:map-stats,reader:map-close,quit" '
    M=$(grep "^book_map:" <<<"$OUT")
    sed -n 1p <<<"$M" | grep -qE "^book_map: stats layouts=[1-9] paints=[0-9]+ prefetch_done=[1-9][0-9]* disk_hits=[0-9]+ queued=[0-9]+ max_slice_ms=[0-9]+ max_paint_slice_ms=[0-9]+"; check "the pages in view were made ahead in the background right after the map opened" $?
    sed -n 3p <<<"$M" | grep -q "same_layout=true"; check "a page of the reader'"'"'s own chapter is laid out like the reader'"'"'s page (same pages, lines, height and columns)" $?
    sed -n 4p <<<"$M" | grep -q "same_layout=true"; check "and another page of it" $?
    sed -n 2p <<<"$M" | grep -qE "cached=true"; check "pages the prefetch made are in the cache before they are asked for" $?
    sed -n 3p <<<"$M" | grep -qE "col=[12]/2 "; check "a thumbnail is one page of the two column view (a half of it)" $?
    sed -n 5p <<<"$M" | grep -qE "queued=[0-9]+"; check "the prefetch queue is reported" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# Speed of the book map thumbnails (budgets in docs/perf.md): a typical chapter's first thumbnail < 150 ms, a
# cached one < 5 ms, 40 pages made ahead in < 2 s without holding the main loop (paint slices < 16 ms); a debug
# build gets 12 times the room. A second map of the same book reads its pages from the disk cache.
mkdir -p "$ROOT/alicebook/lib"
if [ -s "$HOME/.cache/bookx-testbooks/alice.epub" ]; then cp "$HOME/.cache/bookx-testbooks/alice.epub" "$ROOT/alicebook/lib/"; else cp "$ROOT/mapbook/lib/map.epub" "$ROOT/alicebook/lib/"; fi
PF=1; [ "${PROFILE:-debug}" = release ] || PF=12
scenario map_thumb_perf "library:import=$ROOT/alicebook/lib,wait:4000,read:3,wait:3500,reader:map,wait:50,reader:map-thumb=100,reader:map-thumb=100,reader:map-thumb=101,wait:$((2500 * PF)),reader:map-stats,reader:map-close,wait:500,reader:map,wait:$((1500 * PF)),reader:map-thumb=100,reader:map-stats,reader:map-close,quit" '
    M=$(grep "^book_map:" <<<"$OUT")
    num() { sed -E "s/.* $2=([0-9]+).*/\1/" <<<"$1"; }
    A=$(sed -n 1p <<<"$M"); B=$(sed -n 2p <<<"$M"); C=$(sed -n 3p <<<"$M")
    [ "$(num "$A" ms)" -le $((150 * '$PF')) ]; check "the first thumbnail of a chapter (layout included) takes $(num "$A" ms) ms (budget $((150 * '$PF')) ms)" $?
    [ "$(num "$B" ms)" -le $((5 * '$PF')) ]; check "a thumbnail that is cached takes $(num "$B" ms) ms (budget $((5 * '$PF')) ms)" $?
    [ "$(num "$C" ms)" -le $((40 * '$PF')) ]; check "another page of the same chapter takes $(num "$C" ms) ms (no new layout)" $?
    S=$(sed -n 4p <<<"$M")
    grep -qE "queued=0" <<<"$S"; check "the 40 pages nearest the pointer were made ahead within $((2500 * '$PF')) ms of opening the map" $?
    [ "$(num "$S" max_paint_slice_ms)" -le $((16 * '$PF')) ]; check "a prefetch slice that only paints held the main loop at most $(num "$S" max_paint_slice_ms) ms (budget $((16 * '$PF')) ms)" $?
    S2=$(sed -n 7p <<<"$M")
    [ "$(num "$S2" disk_hits)" -ge 1 ]; check "the second map of the book reads its pages from the disk cache ($(num "$S2" disk_hits) hits)" $?
    [ "$(num "$S2" layouts)" -le 3 ]; check "and lays out at most three chapters again (pages that were not made the first time)" $?
    [ "$(num "$(sed -n 6p <<<"$M")" ms)" -le $((15 * '$PF')) ]; check "a page from the disk cache is there in $(num "$(sed -n 6p <<<"$M")" ms) ms (budget $((15 * '$PF')) ms)" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'
