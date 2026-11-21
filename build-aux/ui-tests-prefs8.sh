# Scenarios of wave 8 (Preferences and Sync): the focusable count of the Privacy list, the reading ruler rows that
# follow their switch, the AI assistants page (token confirmation, port insensitive while the server runs), the empty
# patch list, translated labels of the cache buttons, search through the choices of a combo row, the library-folder
# confirmation and the network banner of the Sync window. Sourced by ui-tests.sh. Dev commands:
# `prefs:privacy-dump` (`prefs_privacy: rows=N focusable=K`), `prefs:ruler-dump` (`prefs_ruler: on=B lines_sensitive=B
# dim_sensitive=B`), `prefs:patches-dump` (`prefs_patches: rows=N max_controls=K menus=A|B empty_state=B`),
# `prefs:metadata-clear` (`prefs_metadata_clear: label=L`), `prefs:libdir=PATH` (`prefs_libdir: confirm_shown=true
# folder=PATH`), `prefs:dict-remove=NAME` / `prefs:dict-undo` (`prefs_dict_remove: ...`, `prefs_dict_undo: ...`),
# `mcp:token=regen-confirm` (`mcp_token_confirm: shown=1 token_unchanged=1`), `mcp:dump` (adds `port_sensitive`,
# `copy_sensitive`), `tts:labels` (`tts_labels: model_choices=A|B clear_label=L ...`), `sync:dump` (adds `lan_banner=B`).

scenario prefs8_privacy_rows 'prefs:,wait:900,prefs:pick=Privacy,wait:300,prefs:privacy-dump,a11y-audit:,quit' '
    has "^prefs_privacy: rows=[1-9][0-9]* focusable=0$"; check "the connection list has information rows that Tab skips" $?
    ! has "^a11y-problem:"; check "every control of the Privacy page has a name" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario prefs8_ruler_rows 'prefs:,wait:900,prefs:pick=Reading,wait:300,prefs:ruler-off,prefs:ruler-dump,prefs:ruler-on,prefs:ruler-dump,prefs:ruler-off,prefs:ruler-dump,quit' '
    R=$(grep "^prefs_ruler: " <<<"$OUT")
    sed -n 1p <<<"$R" | grep -q "on=false lines_sensitive=false dim_sensitive=false"; check "with the ruler off its band settings are insensitive" $?
    sed -n 2p <<<"$R" | grep -q "on=true lines_sensitive=true dim_sensitive=true"; check "switching it on enables them" $?
    sed -n 3p <<<"$R" | grep -q "on=false lines_sensitive=false"; check "and off disables them again" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario prefs8_mcp_page 'prefs:,wait:900,prefs:pick=AI Assistants,wait:300,mcp:dump,mcp:token=regen-confirm,wait:400,mcp:dump,dialog:state,mcp:port=@MCPPORT@,mcp:enable=on,wait:500,mcp:dump,mcp:enable=off,wait:300,mcp:dump,a11y-audit:,quit' '
    M=$(grep "^mcp: " <<<"$OUT")
    sed -n 1p <<<"$M" | grep -q "running=0 .*port_sensitive=1 copy_sensitive=0"; check "off: the port can be changed and there is no address to copy" $?
    has "^mcp_token_confirm: shown=1 token_unchanged=1"; check "making a new token asks first" $?
    sed -n 3p <<<"$M" | grep -q "running=1 .*port_sensitive=0 copy_sensitive=1"; check "running: the port is insensitive and the address can be copied" $?
    sed -n 4p <<<"$M" | grep -q "running=0 .*port_sensitive=1 copy_sensitive=0"; check "stopped again: the port can be changed" $?
    ! has "^a11y-problem:"; check "every control of the page has an accessible name" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario prefs8_patches_empty 'prefs:,wait:900,prefs:pick=Patches,wait:300,prefs:patches-dump,prefs:pick=Extensions,wait:300,a11y-audit:,quit' '
    has "^prefs_patches: rows=0 max_controls=0 menus= empty_state=true"; check "an empty patch list shows a placeholder, not a row" $?
    ! has "^a11y-problem:"; check "every control has a name, with the placeholder buttons" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario prefs8_cache_buttons 'prefs:,wait:900,prefs:pick=Library,wait:300,prefs:metadata-clear,wait:300,prefs:pick=Text to Speech,wait:300,tts:labels,a11y-audit:,quit' '
    has "^prefs_metadata_clear: label=\"Clear\""; check "the source cache has a Clear button" $?
    has "^tts_labels: model_choices=Small\\|Full precision clear_label=\"Clear\""; check "the Kokoro model choices are plain words and the audio cache has Clear" $?
    ! has "^a11y-problem:"; check "both Clear buttons have accessible names" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario prefs8_search_choices 'prefs:,wait:900,prefs:search=dark,wait:400,prefs:dump,prefs:search=,quit' '
    sed -n 1p <<<"$(grep "^prefs: query" <<<"$OUT")" | grep -qE "sections=General.* hits=[1-9]"; check "searching a choice of a combo row (dark) finds its row" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario prefs8_library_folder_confirm 'prefs:,wait:900,prefs:pick=Library,wait:300,prefs:libdir=@D@/other,wait:500,dialog:state,quit' '
    has "^prefs_libdir: confirm_shown=true folder=.*/other"; check "choosing another library folder asks first" $?
    has "^dialog: open=true"; check "the confirmation dialog is really open" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario prefs8_sync_banner 'share:port0,action:sync,wait:800,sync:dump,sync:card=wifi,wait:900,prefs:sync-server=on,wait:700,sync:dump,prefs:sync-server=off,wait:600,sync:dump,quit' '
    W=$(grep "^sync_window: " <<<"$OUT")
    sed -n 1p <<<"$W" | grep -q "lan_banner=false"; check "no network banner while the server is off" $?
    sed -n 2p <<<"$W" | grep -q "lan_banner=true"; check "step 1 opens the server to the network and the window says so" $?
    sed -n 3p <<<"$W" | grep -q "lan_banner=false"; check "turning step 1 off removes the banner" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# ---- wave 9 (Preferences, Sync, Health, Statistics) --------------------------------------------------------------
# Dev commands: `prefs:titles` (`prefs_titles: rows=N problems=...`), `prefs:stats-dump|stats-daily=on|off|
# stats-daily-amount=N` (`stats_ui: ...`), `prefs:sync-requirements` (`prefs_sync_requirements: groups=N titles=...`),
# `library:health=dump` (adds `health_ui: ...`), `library:health=trash-heading` (`health_trash_heading: ...`), `keys:resetall` / `keys:undo`
# (`keys: resetall -> ... customised=B`, `keys: undo pressed=B customised=B`), `audio:rmbookmark=N` / `audio:undo`,
# `prefs:comic-dump` (adds `grey_sensitive`, `animate_sensitive`), `prefs:sync-account-dump` (adds `register_sensitive`,
# `login_sensitive`, `login_tip`, `status_controls`).

scenario prefs9_sidebar_titles 'prefs:,wait:900,prefs:dump,prefs:icons,prefs:titles,prefs:pick=Patches,wait:300,prefs:dump,prefs:pick=AI Assistants (MCP),wait:300,prefs:dump,quit' '
    P=$(grep "^prefs: query" <<<"$OUT")
    sed -n 1p <<<"$P" | grep -q "sections=General|Library|Reading|Dictionaries|Editor|Text to Speech|Sync|E-mail|Extensions|Privacy|Keyboard Shortcuts|AI Assistants"; check "the sidebar has twelve pages in the decided order, Dictionaries is its own page and Patches is part of Extensions" $?
    sed -n 2p <<<"$P" | grep -q "selected=Extensions"; check "the old name Patches still opens Extensions" $?
    sed -n 3p <<<"$P" | grep -q "selected=AI Assistants"; check "the old name AI Assistants (MCP) still opens the page" $?
    has "^prefs_icons: .*missing=\\[\\]"; check "every page has an icon that exists" $?
    has "^prefs_titles: rows=[1-9][0-9]* problems=$"; check "no row title is longer than 48 characters, says my or ends in an ellipsis" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario prefs9_900x600 'prefs:,wait:900,prefs:pick=Reading,wait:400,prefs:fit,prefs:pick=AI Assistants,wait:400,prefs:fit,prefs:pick=Keyboard Shortcuts,wait:400,prefs:fit,prefs:pick=Privacy,wait:400,prefs:fit,prefs:pick=Sync,wait:400,prefs:fit,quit' '
    [ "$(grep -c "^prefs_fit: .* fits=true" <<<"$OUT")" = 5 ]; check "at 900x600 the dialog fits inside the window and the longest pages need no sideways scrolling" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario prefs9_eink_and_touch 'prefs:,wait:900,prefs:search=e-ink,wait:400,prefs:dump,prefs:search=,prefs:pick=Reading,wait:300,prefs:comic-dump,prefs:eink=on,wait:300,prefs:comic-dump,prefs:eink=off,wait:300,prefs:comic-dump,quit' '
    sed -n 1p <<<"$(grep "^prefs: query" <<<"$OUT")" | grep -qE "sections=General.* hits=[1-9]"; check "E-ink mode is in General (Appearance), not Reading" $?
    C=$(grep "^comic_prefs: " <<<"$OUT")
    sed -n 1p <<<"$C" | grep -q "grey_sensitive=true animate_sensitive=true"; check "off: the comic grey and animation switches can be changed" $?
    sed -n 2p <<<"$C" | grep -q "grey=true animate=true grey_sensitive=false animate_sensitive=false"; check "with e-ink on they show on and cannot be changed" $?
    sed -n 3p <<<"$C" | grep -q "grey_sensitive=true animate_sensitive=true"; check "and they come back when e-ink is off" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario prefs9_shortcuts_reset_undo 'prefs:,wait:900,prefs:pick=Keyboard Shortcuts,wait:300,keys:press=reader.reload|F5,keys:dump=reader.reload,keys:resetall,keys:dump=reader.reload,keys:undo,keys:dump=reader.reload,a11y-audit:,quit' '
    K=$(grep "^keys: reader.reload effective" <<<"$OUT")
    sed -n 1p <<<"$K" | grep -q "customised=true"; check "a rebound shortcut is customised" $?
    sed -n 2p <<<"$K" | grep -q "customised=false"; check "Reset All Shortcuts resets it" $?
    has "^keys: undo pressed=true customised=true"; check "the Shortcuts reset toast has Undo and it brings the changed shortcut back" $?
    sed -n 3p <<<"$K" | grep -q "customised=true"; check "the restored shortcut is back in the registry" $?
    ! has "^a11y-problem:"; check "every control has a name" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario prefs9_stats_goals 'stats:show,wait:700,prefs:stats-dump,prefs:stats-daily=on,prefs:stats-dump,prefs:stats-daily-amount=45,stats:goal:0:0,prefs:stats-daily=off,prefs:stats-dump,a11y-audit:,quit' '
    S=$(grep "^stats_ui: " <<<"$OUT")
    sed -n 1p <<<"$S" | grep -q "chart_label=\"Reading time, last 7 days\" chart_desc=true daily_on=false daily_amount_visible=false"; check "the week chart has a name and a description, a goal is a switch with its amount hidden" $?
    sed -n 2p <<<"$S" | grep -q "daily_on=true daily_amount_visible=true daily_amount=30"; check "switching the daily goal on shows the amount (30 minutes)" $?
    sed -n 3p <<<"$S" | grep -q "daily_on=false daily_amount_visible=false"; check "switching it off hides the amount" $?
    ! has "^a11y-problem:"; check "every control has a name" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario prefs9_health_sidebar 'library:health=open,wait:800,library:health=dump,library:health=trash-heading,library:health=tab=backups,wait:300,library:health=dump,a11y-audit:,quit' '
    has "^health_ui: problems_visible=false calibre_visible=false check_button=\"Check Now\" status_titles=Location\\|"; check "Health opens with no empty Problems list, no Calibre import group, one Check Now button and no Books row" $?
    has "^health_trash_heading: Empty the Trash\\?"; check "an empty trash is not a count" $?
    has "^health_tab: backups"; check "a page is picked from the sidebar" $?
    ! has "^a11y-problem:"; check "every control of the sidebar and its pages has a name" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario prefs9_sync_account_gate 'prefs:,wait:900,prefs:pick=Sync,wait:300,prefs:sync-account-dump,prefs:sync-account=bob;pw;http://127.0.0.1:1/bookx,wait:300,prefs:sync-account-dump,prefs:sync-requirements,a11y-audit:,quit' '
    A=$(grep "^prefs_account: " <<<"$OUT")
    sed -n 1p <<<"$A" | grep -q "register_sensitive=false login_sensitive=false login_tip=\"Turn on the server first\" status_controls=1"; check "with no server Register and Log in are insensitive with the reason, one control per row" $?
    sed -n 2p <<<"$A" | grep -q "register_sensitive=true login_sensitive=true"; check "another server address makes them work" $?
    has "^prefs_sync_requirements: groups=5 titles=Reading progress"; check "what each sync type needs is a dialog with one group per type" $?
    ! has "^a11y-problem:"; check "every control has a name" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario prefs9_email_test 'prefs:,wait:900,prefs:pick=E-mail,prefs:email-dump,quit' '
    has "^prefs_email: titles=Server\\|Port\\|Encryption\\|User name\\|Password\\|Your address\\|Send books to\\|Send a Test Message test_sensitive=false"; check "the e-mail rows have plain titles and the test row waits for the settings" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario prefs9_email_test_sends 'library:mailcfg,prefs:,wait:900,prefs:pick=E-mail,prefs:email-dump,prefs:email-test,wait:2000,prefs:email-dump,quit' '
    has "^prefs_email: .* test_sensitive=true"; check "with server and both addresses the test row can be pressed" $?
    has "^prefs_email: .* result=\"Sent to me@example.com\""; check "the result says where the test went" $?
    MAIL=$(cat "$d/mail.txt" 2>/dev/null)
    grep -q "RCPT TO:<me@example.com>" <<<"$MAIL"; check "the server got a message for your own address" $?
    grep -q "Bookx test message" <<<"$MAIL"; check "it is the test message" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'
