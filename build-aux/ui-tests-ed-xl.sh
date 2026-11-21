# Scenarios of editor plugins (CE-019) and EPUB 3 narration (CE-422). Sourced by ui-tests.sh (uses its `scenario`,
# `check`, `has`, $ROOT). The mock voice ($BOOKX_TTS_MOCK is always set) makes silent audio whose length follows the
# text, so the scenarios never need a downloaded voice. Dev commands: `editor:mo:<name>` (ui/editor/narration.rs).

XPL='http://www.w3.org/1999/xhtml'
EDP=org.bookx.example.trim-whitespace

scenario editor_plugin_trim "plugins:enable=$EDP,wait:300,edit:1,wait:3000,editor:x:put=OEBPS/chapter-1.xhtml|<html xmlns=\"$XPL\"><head><title>T</title></head><body>\n<p>Trailing</p>   \n<p>clean</p>\n<p>tabs</p>  \n</body></html>,editor:x:put=OEBPS/chapter-2.xhtml|<html xmlns=\"$XPL\"><head><title>T</title></head><body><p>nothing to do</p></body></html>,editor:tool=plugin:action:$EDP:trim,wait:800,editor:cat=OEBPS/chapter-1.xhtml,editor:cat=OEBPS/chapter-2.xhtml,editor:cps,editor:dump,editor:restore=last,wait:500,editor:cat=OEBPS/chapter-1.xhtml,editor:tool=plugin:action:$EDP:nope,wait:300,editor:dump,editor:cmdpalette,wait:500,palette:type=Trim Trailing,wait:300,palette:dump,palette:close,a11y-audit:,quit" '
    C=$(grep "^editor_cat:" <<<"$OUT")
    sed -n 1p <<<"$C" | grep -q "<body>⏎<p>Trailing</p>⏎<p>clean</p>⏎<p>tabs</p>⏎</body>"; check "the plugin removed trailing spaces and tabs from the chapter" $?
    sed -n 2p <<<"$C" | grep -q "<p>nothing to do</p>"; check "a chapter without trailing whitespace is left alone" $?
    has "^editor_cps: Before: Trim Trailing Whitespace$"; check "the whole action is one checkpoint named after it" $?
    has "toast=\"Trimmed [0-9]+ files?"; check "the plugin message is shown" $?
    sed -n 3p <<<"$C" | grep -q "<p>Trailing</p>   ⏎"; check "restoring the checkpoint undoes the plugin action" $?
    has "^palette: .*Trim Trailing Whitespace"; check "the action is in the command palette" $?
    has "is not enabled"; check "an unknown action is refused with a message" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario editor_plugin_off "edit:1,wait:3000,editor:tool=plugin:action:$EDP:trim,wait:500,editor:dump,editor:cmdpalette,wait:500,palette:type=Trim Trailing,wait:300,palette:dump,palette:close,quit" '
    has "is not enabled"; check "a plugin that is not switched on offers nothing and cannot be run" $?
    ! has "^palette: .*Trim Trailing Whitespace"; check "a disabled plugin is not in the command palette" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario editor_narration "edit:1,wait:3000,editor:mo:files,editor:tool=plugin:narration:start,wait:6000,editor:mo:files,editor:mo:check,editor:mo:total,editor:cps,editor:cat=OEBPS/chapter-1.xhtml,editor:save,wait:800,editor:tool=plugin:narration:add,wait:800,editor:dump,editor:tool=plugin:narration:remove,wait:800,editor:mo:files,editor:mo:check,editor:cps,editor:restore=last,wait:600,editor:mo:files,a11y-audit:,quit" '
    M=$(grep "^editor_mo: " <<<"$OUT")
    sed -n 1p <<<"$M" | grep -q "smil=0 wav=0 overlay_attrs=0 chapters=3 narrated=0 spans=0"; check "the test book starts without narration" $?
    sed -n 2p <<<"$M" | grep -q "smil=3 wav=3 overlay_attrs=3 chapters=3 narrated=3 spans=[1-9]"; check "every chapter got its audio, its SMIL file, an overlay attribute and sentence spans" $?
    sed -n 3p <<<"$M" | grep -q "check=$"; check "the overlay validates: every text and audio reference exists and the durations match" $?
    sed -n 4p <<<"$M" | grep -qE "total=0:0[0-9]:[0-9]{2}\.[0-9]{3}"; check "the package carries the total duration" $?
    has "^editor_cps: Before: Add Narration$"; check "the run is one checkpoint" $?
    grep "^editor_cat:" <<<"$OUT" | sed -n 1p | grep -q "<span id=\"bxs1\">Chapter 1</span>"; check "sentences are wrapped in spans with ids" $?
    has "Every chapter is narrated already"; check "running it again finds nothing to narrate" $?
    sed -n 5p <<<"$M" | grep -q "smil=0 wav=0 overlay_attrs=0 chapters=3 narrated=0 spans=0"; check "Remove Narration takes everything out again" $?
    sed -n 6p <<<"$M" | grep -q "check=$"; check "a book without narration has no overlay findings" $?
    sed -n 7p <<<"$M" | grep -q "smil=3 wav=3"; check "restoring the checkpoint brings the narration back" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario reader_narration "edit:1,wait:3000,editor:tool=plugin:narration:start,wait:6000,editor:save,wait:800,read:1,wait:3500,reader:tts-play,wait:300,reader:tts-dump,wait:1500,reader:tts-dump,reader:tts-stop,wait:300,reader:tts-dump,quit" '
    T=$(grep "^tts: " <<<"$OUT")
    sed -n 1p <<<"$T" | grep -q "engine_text=\"Voice: Narration of the book\""; check "a book with narration is read with its own audio" $?
    sed -n 1p <<<"$T" | grep -q "state=Playing range=Some((0, 9)) text=\"Chapter 1\""; check "the first narrated sentence is highlighted" $?
    sed -n 2p <<<"$T" | grep -q "state=Playing" && ! sed -n 2p <<<"$T" | grep -q "text=\"Chapter 1\""; check "the highlight follows the narration to the next sentence" $?
    sed -n 3p <<<"$T" | grep -q "state=Stopped"; check "stop works as for synthesized speech" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'
