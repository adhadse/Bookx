# Scenarios of the dictionary downloads (docs/enhancements.md item 30): the offer in the look-up popover, the
# automatic downloads as Background Jobs, offline and failed downloads. They run against a fake dictionary server
# (build-aux/fake-dict-server.py, started by the `dict_*)` arm of ui-tests.sh; BOOKX_DICT_CATALOG points the app at it).
# Automatic downloads are switched off (BOOKX_DICT_AUTO=off) in every other scenario so none goes to the network.

# a German book
mkdir -p "$ROOT/dictde"
printf -- '---\ntitle: "Ein Buch"\nauthor: "A. Autor"\nlanguage: de\n---\n# Eins\n\nText.\n' >"$ROOT/dictde/de.md"

scenario dict_prompt_download 'read:1,wait:2500,reader:select,wait:900,reader:ui-dict,wait:900,reader:dump,reader:ui-dict-prompt-btn,wait:700,reader:dump,wait:3500,reader:dump,jobs:dump,quit' '
    P=$(grep "^dict_prompt:" <<<"$OUT")
    sed -n 1p <<<"$P" | grep -q "state=offer lang=en title=\"No dictionary for English yet\" primary=\"Download dictionary ([0-9]* [KM]B)\" enabled=true progress=None visible=true"; check "no dictionary for the language: the popover offers the download, with its size, instead of the old message" $?
    sed -n 2p <<<"$P" | grep -q "state=working .*enabled=false progress=Some"; check "one click starts it: the popover shows the progress and the button waits" $?
    sed -n 3p <<<"$P" | grep -q "state=hidden .* visible=false"; check "when it is installed the offer is gone" $?
    grep "^dict: " <<<"$OUT" | tail -1 | grep -q "^dict: dict | .*A mock definition of chapter"; check "and the definition of the word that was looked up is shown in the same popover (still on the dictionary page)" $?
    grep "^dict_service:" <<<"$OUT" | tail -1 | grep -q "installed=\[\"en:mock-en\"\]"; check "the dictionary is installed for its language" $?
    ls "$d"/dicts-mine/en-mock-en/*.ifo >/dev/null 2>&1 && [ -s "$d/dicts-mine/en-mock-en/bookx-dict.json" ]; check "in the dictionaries folder, with its marker" $?
    ! ls "$d"/dicts-mine/.downloads/*.archive 2>/dev/null | grep -q .; check "the downloaded archive was cleaned up" $?
    grep "^jobs: " <<<"$OUT" | grep -q "Dictionary for English"; check "it ran as a job of the Background Jobs panel" $?
    ! has "No dictionaries installed"; check "the old message is not shown" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario dict_auto_import "wait:4500,jobs:dump,library:import=$ROOT/dictde,wait:4500,jobs:dump,library:import=$ROOT/dictde,wait:1500,reader:none,quit" '
    J=$(grep "^jobs: " <<<"$OUT")
    grep -q "Dictionary for English" <<<"$J"; check "a book in a language without a dictionary is added: a job for it appears by itself" $?
    ls "$d"/dicts-mine/en-mock-en/*.ifo >/dev/null 2>&1; check "the English dictionary got installed" $?
    grep -q "Dictionary for German" <<<"$J"; check "a German book added later starts the job for German" $?
    ls "$d"/dicts-mine/de-mock-de/*.ifo >/dev/null 2>&1; check "and the German dictionary got installed" $?
    [ "$(grep -c "Dictionary for English" <<<"$(grep "^jobs: " <<<"$OUT" | tail -1)")" -le 1 ]; check "one job per language (importing the same book again starts nothing new)" $?
    grep -q "\"considered\"" "$d/cfg/bookx/config.json"; check "the languages that were looked at are remembered" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario dict_auto_off "wait:4000,library:import=$ROOT/dictde,wait:3000,jobs:dump,quit" '
    ! grep "^jobs: " <<<"$OUT" | grep -q "Dictionary for"; check "with automatic downloads off a new book starts no download" $?
    [ ! -d "$d/dicts-mine/de-mock-de" ]; check "and nothing is installed" $?'

scenario dict_default_on 'wait:4500,jobs:dump,quit' '
    grep "^jobs: " <<<"$OUT" | grep -q "Dictionary for English"; check "the setting is on by default (no setting, no override): the first book starts its download" $?'

scenario dict_prefs_switch "prefs:,wait:900,prefs:dict-auto=off,wait:300,library:import=$ROOT/dictde,wait:3000,jobs:dump,quit" '
    grep "^jobs: " <<<"$OUT" | grep -q "Dictionary for English"; check "(the first book started its download: the setting is on by default)" $?
    ! grep "^jobs: " <<<"$OUT" | grep -q "Dictionary for German"; check "switching the setting off in Preferences stops the automatic downloads: the German book starts none" $?
    grep -q "\"auto_download\": *false" "$d/cfg/bookx/config.json"; check "and it is saved" $?'

scenario dict_open_book 'read:1,wait:4500,jobs:dump,reader:dump,quit' '
    grep "^jobs: " <<<"$OUT" | grep -q "Dictionary for English"; check "opening a book whose language has no dictionary starts the download (the library already had it: no download then)" $?
    grep "^dict_service:" <<<"$OUT" | tail -1 | grep -q "installed=\[\"en:mock-en\"\]"; check "and it is installed" $?'

scenario dict_offline "wait:4000,library:import=$ROOT/dictde,wait:2500,jobs:dump,read:1,wait:2500,reader:select,wait:800,reader:ui-dict,wait:700,reader:ui-dict-prompt-btn,wait:900,reader:dump,quit" '
    ! grep "^jobs: " <<<"$OUT" | grep -q "Dictionary for"; check "offline: no download is started and the import is not blocked" $?
    grep -q "\"pending\"" "$d/cfg/bookx/config.json" && grep -A3 "\"pending\"" "$d/cfg/bookx/config.json" | grep -q "\"en\""; check "the language waits for the network (it is tried again later)" $?
    grep "^dict_prompt:" <<<"$OUT" | tail -1 | grep -q "state=failed .*primary=\"Try again\""; check "pressing Download offline says so and offers Try again" $?
    grep "^dict_prompt:" <<<"$OUT" | tail -1 | grep -q "title=\"No dictionary for English yet\""; check "the popover still says what is missing" $?'

scenario dict_failed 'read:1,wait:2500,reader:select,wait:900,reader:ui-dict,wait:700,reader:ui-dict-prompt-btn,wait:2500,reader:dump,quit' '
    P=$(grep "^dict_prompt:" <<<"$OUT" | tail -1)
    grep -q "state=failed" <<<"$P"; check "a download that cannot connect ends in the failed state, not a hang" $?
    grep -q "primary=\"Try again\" enabled=true" <<<"$P"; check "with the button to try again" $?
    ! grep -q "dict_service:.*installed=\[\"" <<<"$(grep "^dict_service:" <<<"$OUT" | tail -1)"; check "nothing got installed" $?'

# a language the catalogue has nothing for: the old explanation (where to put dictionaries) stays
mkdir -p "$ROOT/dictla"
printf -- '---\ntitle: "Liber"\nauthor: "A"\nlanguage: la\n---\n# Caput\n\nGallia est omnis divisa in partes tres.\n' >"$ROOT/dictla/la.md"
scenario dict_no_source "library:import=$ROOT/dictla,wait:3500,read:3,wait:3000,reader:select,wait:900,reader:ui-dict,wait:900,reader:dump,quit" '
    has "^dict_prompt: state=hidden lang=la"; check "a language the catalogue has no dictionary for: no offer" $?
    has "^dict: [a-z]+ \| No dictionaries installed"; check "the explanation of where to put your own dictionaries is shown" $?'
