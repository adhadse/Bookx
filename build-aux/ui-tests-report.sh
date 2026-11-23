# Scenarios of Report a Problem (docs: wiki Troubleshooting → Report a Problem). Sourced by ui-tests.sh (same
# `scenario NAME 'dev script' 'checks'` format as ui-tests-lib8.sh).
#
# Each scenario has its own state folder ($d/state = XDG_STATE_HOME: the log and the crash files). The version of
# the app is replaced by BOOKX_TEST_VERSION in `report_beta*` (0.3.0-beta.1) and `report_stable*` (0.3.0); all other
# scenarios never see the beta notice (it is off under a dev script unless a test version is set).
#
# Dev hooks (all `report:<cmd>`):
#   open | open-crash        open the dialog (the second fills in "Bookx closed unexpectedly.")
#   type=TEXT                the "What happened?" text      log=on|off | library=on|off | titles=on|off | host=on|off   the switches
#   open-from=A|B            open the dialog as the Sync and Devices windows do: the sections titled A and B come first
#   preview                  `report_preview: <the preview, newlines as ⏎>`
#   dump                     `report_dump: len= lines= log= library= titles= host= focus_on_entry= hint=`
#   copy                     `report_clipboard: len= same= first=` (read back from the clipboard)
#   save=PATH                writes the report like Save Report (`report_saved: PATH`)
#   github                   copies, then prints `report_open: <url>` instead of opening a browser
#   folder                   `report_folder: <log folder>`
#   log-line=TEXT            writes a WARN line (no commas: they separate dev commands)
#   crash-sim=TEXT           writes a crash file through the panic hook's writer (`report_crash: Some(Ok("…"))`)
#   notices                  `report_notices: beta=<shown> crash=<shown> seen="…" due=<bool>`
#   recheck | dismiss-beta | dismiss-crash | report-from-beta | report-from-crash

scenario report_menu_palette 'library:wide,wait:300,library:menus,action:report-problem,wait:900,report:dump,report:preview,dialog:close,wait:400,palette:open,wait:700,palette:type=report a prob,wait:300,palette:dump,palette:pick,wait:900,report:dump,quit' '
    has "^mainmenu: .*Help \\[app.help\\] \\| Report a Problem \\[app.report-problem\\] \\| About Bookx"; check "the main menu lists Report a Problem between Help and About" $?
    [ "$(grep -c "^report_dump: " <<<"$OUT")" -eq 2 ]; check "the dialog opens from the menu action and from the palette" $?
    has "^palette: query=\"report a prob\" selected=0 results=.*Report a Problem"; check "the palette finds the command" $?
    has "^report_dump: .*focus_on_entry=true"; check "the What happened entry has the focus when the dialog opens" $?
    has "^report_preview: <!-- Bookx problem report.*### Bookx⏎⏎- Version: "; check "the preview starts with the version" $?
    has "^report_preview: .*- Installed as: .*### System.*- OS: .*- GTK [0-9.]+, libadwaita [0-9.]+"; check "the preview names the install, the system and the toolkit versions" $?
    has "^report_preview: .*### What happened⏎⏎$"; check "the preview ends with an empty What happened section" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario report_privacy 'library:wide,wait:300,report:log-line=opening SENT-Title Book for sentuser with SENT-pw-4242 on sentinel-host.example at /home/sentuser/x token=SENT-tok-77,wait:300,report:open,wait:800,report:type=It broke when I pressed SENT-USERTEXT,wait:300,report:preview,report:copy,wait:600,report:save=@D@/saved.md,wait:300,report:github,wait:600,report:folder,quit' '
    P=$(grep "^report_preview:" <<<"$OUT")
    [ -n "$P" ]; check "the preview is printed" $?
    for bad in "SENT-Title" "SENT-Author" "sentuser" "sentinel-host" "sentinel-mail" "SENT-pw-4242" "SENT-mail-5555" "SENT-tok-77" "$HOME"; do
        ! grep -qF -- "$bad" <<<"$P"; check "the preview has no $bad" $?
    done
    grep -qF "SENT-USERTEXT" <<<"$P"; check "the text typed under What happened is in the preview as typed" $?
    grep -q "Recent log.*WARN.*opening <title> for <account> with <secret> on <host>" <<<"$P"; check "the logged warning is in the preview with title, account, password and host removed" $?
    grep -q "bookx_crumb: library opened: " <<<"$P"; check "the start-up breadcrumbs are in the log part" $?
    has "^report_clipboard: len=[1-9][0-9]* same=true first=<!-- Bookx problem report"; check "Copy Report puts the whole report on the clipboard" $?
    [ -f "$d/saved.md" ] && [ "$(stat -c %a "$d/saved.md")" = 600 ]; check "Save Report writes a file only the owner can read" $?
    ! grep -qE "SENT-Title|SENT-pw|sentuser|sentinel-host|$HOME" "$d/saved.md"; check "the saved file has no title, password, account, host or home folder" $?
    grep -q "SENT-USERTEXT" "$d/saved.md"; check "and it has the typed text" $?
    has "^report_open: https://github.com/adhadse/Bookx/issues/new\\?template=bug_report.yml&title=Bug%3A%20$"; check "Open GitHub Issue opens the bug form with only a short title (nothing opened in the test)" $?
    has "^report_github: clipboard_len=[1-9][0-9]* hint=true"; check "the report was copied first and the hint says to paste it" $?
    L=$d/state/bookx/bookx.log
    [ -f "$L" ] && [ "$(stat -c %a "$L")" = 600 ] && [ "$(stat -c %a "$d/state/bookx")" = 700 ]; check "the log file is 0600 in a 0700 folder" $?
    ! grep -qE "SENT-Title|SENT-pw|sentuser|sentinel-host|SENT-tok|$HOME" "$L"; check "the log file on disk has no title, password, account, host, token or home folder" $?
    grep -q "WARN.*opening <title> for <account>" "$L"; check "the warning is in the log file, cleaned" $?
    grep -q "bookx_crumb: start Bookx " "$L" && grep -q "bookx_crumb: library opened: " "$L"; check "the log file has the start and library breadcrumbs" $?
    has "^report_folder: $d/state/bookx\$"; check "the log folder is under the state folder" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario report_toggles 'report:open,wait:800,report:log=off,report:library=off,report:dump,report:preview,report:log=on,report:library=on,report:preview,quit' '
    P=$(grep "^report_preview:" <<<"$OUT")
    ! sed -n 1p <<<"$P" | grep -q "### Recent log\\|### Library⏎"; check "with both switches off the preview has no log and no library part" $?
    sed -n 1p <<<"$P" | grep -q "### Recent jobs"; check "the rest stays" $?
    sed -n 2p <<<"$P" | grep -q "### Library⏎⏎- Books: [0-9]" && sed -n 2p <<<"$P" | grep -q "### Recent log"; check "switching them on brings both back" $?
    has "^report_dump: .*log=false library=false"; check "the switches report their state" $?'

scenario report_crash_start 'wait:1500,report:notices,report:report-from-crash,wait:900,report:dump,report:preview,report:notices,quit' '
    has "^report_notices: beta=false crash=true"; check "a crash file from the last run shows the notice at start" $?
    has "^report_dump: .*focus_on_entry=true"; check "Report opens the dialog" $?
    P=$(grep "^report_preview:" <<<"$OUT")
    grep -q "### Last crash.*closed unexpectedly" <<<"$P"; check "the report includes the crash text" $?
    grep -q "### What happened⏎⏎Bookx closed unexpectedly\\." <<<"$P"; check "the What happened text is filled in with the first line" $?
    [ "$(grep "^report_notices:" <<<"$OUT" | tail -1 | grep -c "crash=false")" = 1 ]; check "the notice is gone once the report was opened" $?
    ls "$d"/state/bookx/crash-*.txt.seen >/dev/null 2>&1 && ! ls "$d"/state/bookx/crash-*.txt >/dev/null 2>&1; check "the crash file is kept as seen (not deleted, not shown again)" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario report_privacy_crash_hook 'wait:500,report:notices,report:crash-sim=boom in SENT-Title Book for sentuser,wait:200,report:recheck,report:notices,report:dismiss-crash,report:notices,quit' '
    has "^report_notices: beta=false crash=false"; check "no notice without a crash" $?
    has "^report_crash: Some\\(Ok\\(.*/state/bookx/crash-[0-9]{8}-[0-9]{6}\\.txt"; check "the panic hook writer wrote a crash file next to the log" $?
    [ "$(grep "^report_notices:" <<<"$OUT" | sed -n 2p | grep -c "crash=true")" = 1 ]; check "the next check shows the crash notice" $?
    [ "$(grep "^report_notices:" <<<"$OUT" | sed -n 3p | grep -c "crash=false")" = 1 ]; check "Dismiss hides it" $?
    F=$(ls "$d"/state/bookx/crash-*.txt.seen 2>/dev/null | head -1)
    [ -n "$F" ] && [ "$(stat -c %a "$F")" = 600 ]; check "the dismissed crash file is kept, 0600" $?
    ! grep -qE "SENT-Title|sentuser|$HOME" "$F"; check "the crash file has no title, account or home folder" $?
    grep -q "backtrace:" "$F"; check "it has a backtrace (without RUST_BACKTRACE)" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario report_beta_notice 'wait:1500,report:notices,report:dismiss-beta,report:notices,report:recheck,report:notices,quit' '
    N=$(grep "^report_notices:" <<<"$OUT")
    sed -n 1p <<<"$N" | grep -q "beta=true crash=false seen=\"\" due=true"; check "the first start of a beta version shows the notice" $?
    sed -n 2p <<<"$N" | grep -q "beta=false .*seen=\"0.3.0-beta.1\" due=false"; check "Dismiss hides it and remembers the version" $?
    sed -n 3p <<<"$N" | grep -q "beta=false"; check "it does not come back" $?
    grep -q "\"beta_notice_seen\": \"0.3.0-beta.1\"" "$d/cfg/bookx/config.json"; check "the settings file keeps the version" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario report_beta_seen 'wait:1500,report:notices,quit' '
    has "^report_notices: beta=false crash=false seen=\"0.3.0-beta.1\" due=false"; check "a beta version whose notice was dismissed shows nothing at start" $?'

scenario report_beta_report 'wait:1500,report:notices,report:report-from-beta,wait:900,report:dump,report:notices,quit' '
    has "^report_notices: beta=true"; check "the beta notice is shown" $?
    has "^report_dump: .*focus_on_entry=true"; check "its Report a Problem button opens the dialog" $?
    [ "$(grep "^report_notices:" <<<"$OUT" | tail -1 | grep -c "beta=false")" = 1 ]; check "and counts as seen" $?'

scenario report_stable_no_notice 'wait:1500,report:notices,quit' '
    has "^report_notices: beta=false crash=false seen=\"\" due=false"; check "a version without a pre-release part shows no notice" $?'

scenario report_dialog_fit_win11 'app:win-size=900x600,app:text-scale=1.5,wait:600,report:open,wait:900,a11y-audit:,report:dump,quit' '
    [ "$(grep -c "^dialog_fit: title=\"Report a Problem\"" <<<"$OUT")" -ge 1 ]; check "the dialog reports its size" $?
    ! grep "^dialog_fit: " <<<"$OUT" | grep -q "fits=false"; check "at 900x600 and 1.5 times the text the dialog fits the window" $?
    grep "^dialog_fit: " <<<"$OUT" | grep -q "clipped_buttons=0"; check "no button cuts its label" $?
    ! has "^a11y-problem:"; check "every control has an accessible name" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario report_summary_entry_and_optins 'report:open-from=Sync (KOSync client)|Built-in server,wait:900,report:dump,report:preview,report:titles=on,report:host=on,wait:300,report:dump,report:preview,report:titles=off,report:host=off,report:log=off,report:library=off,report:preview,quit' '
    P=$(grep "^report_preview:" <<<"$OUT" | sed -n 1p)
    has "^report_dump: .*titles=false host=false"; check "the two opt-in switches are off when the dialog opens" $?
    has "^report_dump: .*titles=true host=true"; check "they can be switched on" $?
    grep -q -- "report_preview: <!-- Bookx problem report.*### Summary and suspected area⏎⏎- Suspected area: " <<<"$P"; check "the report opens with the summary and the suspected area" $?
    grep -qF -- "- Reported from: Sync (KOSync client), Built-in server" <<<"$P"; check "the summary says which window the report was opened from" $?
    grep -q "### Recent operations" <<<"$P"; check "recent operations come right after the summary" $?
    grep -q "### Sync (KOSync client)⏎.*### Built-in server⏎.*### Machine" <<<"$P"; check "the sections of the window it was opened from come before the others" $?
    grep -q "### Sync (KOSync client)⏎⏎- Enabled: off" <<<"$P"; check "the sync section says whether the client is on" $?
    grep -q "### Machine⏎⏎- Processors: [0-9]" <<<"$P"; check "the machine section is there" $?
    grep -q "### Library folder⏎⏎- File system: " <<<"$P"; check "the library folder section names its file system" $?
    L=$(grep "^report_preview:" <<<"$OUT" | sed -n 3p)
    ! grep -q "### Library folder\\|### Books involved" <<<"$L"; check "the library switch also removes the library folder and the books involved" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario report_privacy_titles_optin 'library:wide,wait:300,report:log-line=opening SENT-Title Book,wait:300,report:open,wait:800,report:preview,report:titles=on,wait:300,report:preview,quit' '
    A=$(grep "^report_preview:" <<<"$OUT" | sed -n 1p)
    B=$(grep "^report_preview:" <<<"$OUT" | sed -n 2p)
    ! grep -qF "SENT-Title" <<<"$A"; check "no title in the default report" $?
    ! grep -qF "SENT-Title" <<<"$B"; check "and none after switching titles on: only the books an action was about are named, and none was" $?
    ! grep -q "<host>.*<host>.*<host>.*<host>.*<host>" <<<"$A"; check "the report is not full of placeholders" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'
