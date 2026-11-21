# Scenarios of the Sync umbrella (src/sync_model.rs, src/ui/sync_prefs.rs, src/ui/sync_window.rs): the main menu
# entry, the task-oriented Sync window (status card, three choice cards, collapsed Advanced), Preferences -> Sync
# (setup checklist, what Bookx sync does, USB, library servers, collapsed Advanced with every technical setting)
# and the two following each other. Sourced by ui-tests.sh. Dev commands: `sync:dump|primary|card=wifi|usb|opds|
# advanced=open|close` (the window), `prefs:sync-dump|server=on|opds=on|port=N|req|adv|advanced=open|plugin|ko|
# account…` (the page), `share:port0`, `action:sync`.

scenario sync_menu 'share:port0,library:menus,action:sync,wait:900,sync:dump,sync:step=2,wait:300,sync:dump,sync:step=2,wait:300,sync:dump,a11y-audit:,library:icons,sync:card=opds,wait:600,sync:dump,sync:advanced=open,wait:400,sync:dump,a11y-audit:,sync:card=opds,wait:500,sync:dump,sync:close,wait:300,dialog:state,quit' '
    has "^mainmenu: .*Sync \\[app.sync\\] \\| Devices \\[app.devices\\]"; check "the main menu has Sync, with the plural Devices entry next to it" $?
    ! has "^mainmenu: .*Share Library"; check "Share Library is gone from the main menu" $?
    has "^sync_window: title=Sync headline=\"Bookx sync is off\" primary=\"Set up Bookx sync\" cards=wifi:Start setup:Bookx sync is off\\|usb:Open device window:[^|]*\\|opds:Start OPDS server:off use_when=3 sentence=true kosync_labels=0 advanced=collapsed account_hint=true"; check "first screen: a status card (Bookx sync is off) with one big button, three choice cards with a Use this when text and one button each, no KOSync label, Advanced collapsed" $?
    has "^sync_window: .* stepper=\\[steps=4 active=none states=pending,pending,pending,pending open=false,false,false,false statuses=Off\\|Not created yet\\|Not checked\\|Not checked"; check "the Wi-Fi card shows the setup as a compact stepper of four steps, all closed" $?
    has "^sync_window: .* stepper=\\[steps=4 active=2 states=pending,active,pending,pending open=false,true,false,false"; check "a step opens when it is picked (non-linear)" $?
    has "^sync_window: .*cards=wifi:Start setup:.*\\|opds:Stop OPDS server:on .* opds=on"; check "the library server card starts the server and turns into Stop library server" $?
    has "^sync_window: .*advanced=open"; check "Advanced opens" $?
    has "^sync_window: .*cards=.*opds:Start OPDS server:off .*opds=off"; check "and the same card stops it again" $?
    has "^a11y: controls=[0-9]+ problems=0"; check "every control of the Sync window has a name (also with Advanced open)" $?
    has "^icons: checked=[0-9]+ missing=\\[\\]"; check "the icons exist" $?
    has "^dialog: open=false"; check "the window closes" $?
    ! has "/kosync"; check "no /kosync text" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario sync_prefs_ia 'prefs:,wait:900,prefs:dump,prefs:pick=Devices and Sharing,wait:300,prefs:dump,prefs:sync-req,prefs:sync-dump,prefs:sync-adv,prefs:sync-advanced=open,wait:500,prefs:sync-adv,a11y-audit:,library:icons,prefs:search=Devices and Sharing,wait:400,prefs:dump,prefs:search=privacy,wait:400,prefs:dump,prefs:search=,wait:300,prefs:sync-port=38741,prefs:sync-server=on,wait:600,prefs:sync-dump,prefs:sync-ko,prefs:sync-opds=off,wait:500,prefs:sync-dump,quit' '
    P=$(grep "^prefs: query" <<<"$OUT")
    sed -n 1p <<<"$P" | grep -q "sections=General|Library|Reading|Dictionaries|Editor|Text to Speech|Sync|E-mail|Extensions.* selected=General"; check "Preferences has ONE Sync page (no Devices and Sharing page)" $?
    sed -n 1p <<<"$P" | grep -qv "Devices and Sharing"; check "the old Devices and Sharing title is gone from the sidebar" $?
    sed -n 2p <<<"$P" | grep -q "selected=Sync"; check "the old name still picks the Sync page" $?
    sed -n 3p <<<"$P" | grep -q "sections=.*Sync.* selected=Sync"; check "searching the old name finds the Sync page" $?
    sed -n 4p <<<"$P" | grep -q "sections=.*Sync"; check "Privacy and offline mode are reachable from the Sync page (search finds it)" $?
    has "^prefs_sync_req: rows=0 titles= .*bookx_path=true kosync_text=false koreader_menu=true steps=true account_help=true not_share=true kosync_labels=1 brand=true advanced=true info_rows=0"; check "the page has the four-step setup checklist (server, account, plugin, test) as a stepper, the not-the-Share-password text, the one explanation of Bookx sync (extended KOSync) and no other KOSync label, the requirements as a row that opens a dialog, no /kosync" $?
    has "^prefs_sync_adv: expanded=false server_address=true port=true https=true matching=true device_name=true privacy=true reverse_proxy=true requirements=true firewall=true port_reachable=false"; check "Advanced is collapsed and holds the server address, port and firewall, HTTPS, matching, device name, privacy, reverse proxy and requirements" $?
    has "^prefs_sync_adv: expanded=true .* port_reachable=true"; check "opening Advanced reaches the technical fields" $?
    has "^prefs_sync: server=false opds=false wifi=off usb=(connected|not-connected) opds=off .* state=off steps=0000 next=[1-4-] primary=setup"; check "Preferences -> Sync starts with everything off" $?
    has "^prefs_sync: server=true opds=true wifi=on .* state=ready steps=1000"; check "step 1 turns Bookx sync on (ready, step 1 done)" $?
    has "^prefs_share_ko: visible=true address=http://[0-9.]+:38741/bookx$"; check "the address for the e-reader ends in /bookx" $?
    has "^prefs_sync: server=false opds=false wifi=off .* state=off"; check "stopping the library server turns Bookx sync off too" $?
    has "^a11y: controls=[0-9]+ problems=0"; check "every control of the Sync page has a name" $?
    has "^icons: checked=[0-9]+ missing=\\[\\]"; check "the icons exist" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario sync_prefs_live 'action:sync,wait:800,sync:dump,sync:prefs,wait:900,prefs:sync-port=38742,prefs:sync-server=on,wait:600,sync:dump,prefs:sync-plugin,wait:500,sync:dump,sync:card=opds,wait:500,prefs:sync-dump,dialog:close,wait:300,sync:dump,quit' '
    has "^sync_window: title=Sync headline=\"Bookx sync is off\""; check "the Sync window opens off" $?
    has "^sync_window: title=Sync headline=\"Bookx sync is ready\" primary=\"Create account\" .*state=ready steps=1000"; check "the Start setup button opens Preferences; switching step 1 on there shows Bookx sync is ready in the open Sync window" $?
    has "^prefs_plugin: copied=true"; check "step 3 copies the plugin onto the (fake) KOReader device" $?
    has "^sync_window: .* state=ready steps=1010"; check "and the Sync window shows the plugin step done" $?
    has "^prefs_sync: server=false opds=false"; check "stopping the library server in the Sync window shows in Preferences" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario koreader_annotations_sync_states 'koannot:start,wait:500,action:sync,wait:700,sync:dump,sync:close,kosync:setup=http://127.0.0.1:1/bookx;alice;pw,prefs:,wait:800,prefs:pick=Sync,prefs:sync-plugin,wait:400,dialog:close,wait:300,action:sync,wait:600,sync:dump,koannot:device=@D@/ko/dev1.json,wait:300,sync:dump,sync:primary,wait:500,sync:dump,quit' '
    has "^sync_window: title=Sync headline=\"Bookx sync is ready\" primary=\"Create account\" .*account_hint=true .* state=ready steps=1000 next=[1-4-] primary=setup"; check "state ready: the server runs, no account and no plugin yet, the big button names the first missing step" $?
    has "^sync_window: title=Sync headline=\"Bookx sync is ready\" primary=\"Sync now\" .*account_hint=false .* state=ready steps=1110 next=[1-4-] primary=sync-now"; check "with account and plugin the big button becomes Sync now" $?
    has "^sync_window: title=Sync headline=\"Bookx sync is connected\" primary=\"Sync now\" .* state=connected steps=1111"; check "once a KOReader device has sent something the card says connected and all four steps are done" $?
    has "^sync_window: .* result=\"Last sync: "; check "Sync now merges and says what happened, inline" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario sync_account_ui 'share:kosync,wait:500,prefs:,wait:900,prefs:pick=Sync,wait:300,prefs:sync-req,prefs:sync-account-dump,prefs:sync-account=alice;secret,prefs:sync-account-register,wait:1800,prefs:sync-account-dump,prefs:sync-account-register,wait:1800,prefs:sync-account-dump,prefs:sync-account-login,wait:1800,prefs:sync-account-dump,a11y-audit:,quit' '
    has "^prefs_sync_req: .*steps=true account_help=true not_share=true"; check "step 2 is on the page: Create your sync account, the instruction, and that it is not the OPDS server password" $?
    has "^prefs_account: server=\"http://127\\.0\\.0\\.1:[0-9]+/bookx\" user=\"\" register=true login=true"; check "the server address is filled in from the running Bookx server (ending in /bookx) and Register and Log in are there" $?
    has "^kosync_account: Registered and logged in"; check "Register creates the account on Bookx’s own server" $?
    has "^prefs_account: .*user=\"alice\".*status=\"Registered and logged in"; check "the result is shown inline under the fields" $?
    has "^kosync_account: .*already registered.* Choose another user name, or press Log in if this account is yours\\."; check "a taken name says to choose another name or press Log in" $?
    has "^kosync_account: Logged in"; check "Log in works with the same account" $?
    ! has "/kosync"; check "no /kosync text" $?
    has "^a11y: controls=[0-9]+ problems=0"; check "every control has a name" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario sync_account_needs_server 'prefs:,wait:900,prefs:pick=Sync,wait:300,prefs:sync-account=bob;pw,prefs:sync-account-register,wait:400,prefs:sync-account-dump,share:start:0,wait:400,prefs:sync-account-register,wait:400,prefs:sync-account-dump,quit' '
    has "^prefs_account: .*status=\"Start the server first \\(step 1\\)"; check "without a server the account step says to start it first (step 1)" $?
    has "^prefs_account: .*status=\"Turn on the Bookx sync server \\(step 1\\) first"; check "with the server on but Bookx sync off it says to turn that on" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario sync_stepper 'prefs:,wait:900,prefs:pick=Sync,wait:300,prefs:sync-stepper,prefs:sync-port=38751,prefs:sync-server=on,wait:600,prefs:sync-stepper,prefs:sync-account=alice;secret,wait:400,prefs:sync-stepper,prefs:sync-step=4,wait:400,prefs:sync-stepper,prefs:sync-step=1,wait:400,prefs:sync-stepper,a11y-audit:,library:icons,quit' '
    has "^prefs_sync_stepper: steps=4 active=1 states=active,pending,pending,pending open=true,false,false,false statuses=Off\\|Not created yet\\|Not checked\\|Not checked names=Step 1 of 4: Turn on the Bookx sync server, current, Off\\|Step 2 of 4: Create your sync account, not done, Not created yet\\|Step 3 of 4: Install the Bookx plugin on KOReader, not done, Not checked\\|Step 4 of 4: Test on the device, not done, Not checked"; check "the setup checklist is a stepper: four numbered steps with a status each, the first open, and a stable accessible name per step" $?
    has "^prefs_sync_stepper: steps=4 active=2 states=done,active,pending,pending open=false,true,false,false statuses=On\\|Not created yet"; check "turning the server on marks step 1 done and opens step 2 by itself" $?
    has "^prefs_sync_stepper: steps=4 active=3 states=done,done,active,pending open=false,false,true,false statuses=On\\|alice\\|Not checked"; check "a user name and password mark step 2 done, shows the user name as its status and opens step 3" $?
    has "^prefs_sync_stepper: steps=4 active=4 states=done,done,pending,active open=false,false,false,true"; check "any step can be opened (non-linear)" $?
    has "^prefs_sync_stepper: steps=4 active=1 states=done,done,pending,pending open=true,false,false,false"; check "a done step can be opened again and stays marked done" $?
    has "^a11y: controls=[0-9]+ problems=0"; check "every stepper header has a name" $?
    has "^icons: checked=[0-9]+ missing=\\[\\]"; check "the check icon exists" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario sync_notes_default 'prefs:,wait:900,prefs:pick=Sync,wait:300,prefs:sync-notes,prefs:sync-port=38752,prefs:sync-server=on,wait:600,prefs:sync-notes,prefs:sync-server=off,wait:400,prefs:sync-notes,prefs:sync-notes=off,prefs:sync-server=on,wait:600,prefs:sync-notes,quit' '
    N=$(grep "^prefs_notes: " <<<"$OUT")
    sed -n 1p <<<"$N" | grep -q "switch=true config=true listening=false store=false"; check "Sync highlights, notes and bookmarks is on by default and nothing listens until the server is started" $?
    sed -n 2p <<<"$N" | grep -q "switch=true config=true listening=true store=true"; check "starting the server makes annotation sync available without touching the switch" $?
    sed -n 3p <<<"$N" | grep -q "switch=true config=true listening=false store=false"; check "stopping the server stops everything; the choice stays on" $?
    sed -n 4p <<<"$N" | grep -q "switch=false config=false listening=true store=false"; check "the switch can still be turned off: the server then serves no annotation store" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario sync_restart 'wait:1200,prefs:,wait:900,prefs:pick=Sync,wait:300,prefs:sync-dump,prefs:sync-account-dump,share:kodump,quit' '
    has "^prefs_sync: server=true opds=true wifi=on .* state=ready steps=1100"; check "a server that was on when Bookx last ran is on again after the restart (switch on, account step done)" $?
    has "^share_ko: visible=true address=http://[0-9.]+:[0-9]+/bookx$"; check "and it answers on its address" $?
    has "^prefs_account: .*status_title=\"Logged in as alice\""; check "the account row says Logged in as alice instead of Log in" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario sync_proxy_settings 'prefs:,wait:900,prefs:pick=Sync,wait:300,prefs:sync-advanced=open,wait:400,prefs:sync-proxy-dump,prefs:sync-proxy=Books.Example.com:8443 nas;10.0.0.0/8 ::1;books/,wait:400,prefs:sync-proxy-dump,prefs:sync-proxy=https://bad.example/x;;,wait:300,prefs:sync-proxy-dump,prefs:sync-proxy=;0.0.0.0/0;,wait:300,prefs:sync-proxy-dump,prefs:sync-proxy=;;/a/../b,wait:300,prefs:sync-proxy-dump,a11y-audit:,quit' '
    D=$(grep "^prefs_proxy: " <<<"$OUT")
    sed -n 1p <<<"$D" | grep -q "hosts= proxies= prefix= error=none saved=||"; check "the three reverse proxy rows start empty and nothing is saved" $?
    sed -n 2p <<<"$D" | grep -q "error=none saved=Books.Example.com:8443 nas|10.0.0.0/8 ::1|/books$"; check "valid host names, proxy networks and a sub-path are saved (the sub-path normalised to /books)" $?
    sed -n 3p <<<"$D" | grep -q "error=“https://bad.example/x” is not a host name.* saved=Books.Example.com:8443 nas|10.0.0.0/8 ::1|/books$"; check "a URL as host name is refused with the item named, and the saved value stays" $?
    sed -n 4p <<<"$D" | grep -q "error=“0.0.0.0/0” is not an address of a proxy"; check "a proxy network that would trust the whole Internet (/0) is refused" $?
    sed -n 5p <<<"$D" | grep -q "error=The sub-path must be a plain path"; check "a sub-path with .. is refused" $?
    has "^a11y: controls=[0-9]+ problems=0"; check "every new row has a name" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario sync_qr_code 'prefs:,wait:900,prefs:pick=Sync,wait:300,prefs:sync-qr-dump,prefs:sync-qr=http://192.168.1.20:8080/bookx,wait:500,prefs:sync-qr-dump,dialog:state,a11y-audit:,dialog:close,wait:300,dialog:state,quit' '
    has "^qr: none"; check "no code before one is shown" $?
    has "^qr: text=http://192.168.1.20:8080/bookx version=[1-3] size=(21|25|29) dark=[0-9]{3,}"; check "the QR code of the address is drawn (version 1 to 3 for a 31 character address, hundreds of dark modules)" $?
    has "^a11y: controls=[0-9]+ problems=0"; check "the code has a text alternative and the dialog controls have names" $?
    D=$(grep "^dialog: " <<<"$OUT")
    sed -n 1p <<<"$D" | grep -q "count=2"; check "the code opens in a dialog over Preferences" $?
    tail -n 1 <<<"$D" | grep -q "open=true count=1"; check "closing it leaves Preferences open" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario sources_priority_ui 'prefs:,wait:900,prefs:pick=Library,wait:400,prefs:sources-dump,prefs:sources-set=description;openlibrary,prefs:sources-set=series;google,prefs:sources-order=as-written,wait:300,prefs:sources-dump,prefs:sources-set=description;auto,wait:200,prefs:sources-dump,a11y-audit:,quit' '
    S=$(grep "^prefs_sources: " <<<"$OUT")
    sed -n 1p <<<"$S" | grep -q "title=auto authors=auto publisher=auto published=auto description=auto tags=auto language=auto series=auto cover=auto name_order=0"; check "every field starts on Automatic and the name order too" $?
    sed -n 2p <<<"$S" | grep -q "description=openlibrary .* series=google cover=auto name_order=2"; check "a preferred source per field and the name order can be chosen" $?
    sed -n 3p <<<"$S" | grep -q "description=auto .* series=google"; check "Automatic goes back to the built-in order" $?
    has "^a11y: controls=[0-9]+ problems=0"; check "every row has a name" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

# ---- Library server accounts (Preferences -> Sync -> Advanced, src/ui/accounts.rs) ---------------------------------
# Dev commands: `prefs:accounts-open|close|new|edit=NAME|remove=NAME|undo|fill=NAME;PW;REPEAT;tag+tag|save|guests=tag+tag|
# verify=NAME;PW|dump` (`prefs_accounts: count=N accounts=name[tags]hashed=B guests=… restarts=N window=B form=B rows=N last=…`).
# Passwords and hashes are never printed.

scenario accounts_screen 'library:select1,wait:500,library:tags=+Kids;Public,wait:900,prefs:,wait:900,prefs:pick=Sync,wait:400,prefs:sync-port=38747,prefs:sync-server=on,wait:900,prefs:sync-advanced=open,wait:500,prefs:accounts-open,wait:600,prefs:accounts-dump,a11y-audit:,prefs:accounts-new,wait:600,prefs:accounts-fill=kid;SENT-acct-pw-9;SENT-acct-mismatch;kids,prefs:accounts-save,wait:600,prefs:accounts-dump,a11y-audit:,prefs:accounts-fill=kid;short;short;kids,prefs:accounts-save,wait:400,prefs:accounts-dump,prefs:accounts-fill=bookx;SENT-acct-pw-9;SENT-acct-pw-9;kids,prefs:accounts-save,wait:400,prefs:accounts-dump,prefs:accounts-fill=kid;SENT-acct-pw-9;SENT-acct-pw-9;kids,prefs:accounts-save,wait:3500,prefs:accounts-dump,prefs:accounts-verify=kid;SENT-acct-pw-9,prefs:accounts-verify=kid;SENT-acct-wrong,prefs:accounts-new,wait:500,prefs:accounts-fill=Kid;SENT-acct-pw-7;SENT-acct-pw-7;,prefs:accounts-save,wait:500,prefs:accounts-dump,dialog:close,wait:400,prefs:accounts-edit=kid,wait:600,prefs:accounts-fill=kid;;;kids+public,prefs:accounts-save,wait:2500,prefs:accounts-dump,prefs:accounts-verify=kid;SENT-acct-pw-9,prefs:accounts-remove=kid,wait:2500,prefs:accounts-dump,prefs:accounts-undo,wait:2500,prefs:accounts-dump,prefs:accounts-guests=public,wait:2500,prefs:accounts-dump,a11y-audit:,prefs:accounts-guests=,wait:2500,prefs:accounts-dump,quit' '
    A=$(grep "^prefs_accounts: " <<<"$OUT")
    sed -n 1p <<<"$A" | grep -q "count=0 accounts= guests= restarts=0 window=true form=false rows=1"; check "the window opens with no accounts and an empty-state row" $?
    sed -n 2p <<<"$A" | grep -q "count=0 .* form=true"; check "two passwords that differ are refused and nothing is added" $?
    sed -n 3p <<<"$A" | grep -q "count=0"; check "a password that is too short is refused" $?
    sed -n 4p <<<"$A" | grep -q "count=0"; check "the name of the shared login (bookx) is refused" $?
    sed -n 5p <<<"$A" | grep -q "count=1 accounts=kid\\[kids\\]hashed=true guests= restarts=1 window=true form=false rows=1"; check "a valid account is added, hashed, and the running server restarted once" $?
    has "^prefs_accounts_verify: name=kid ok=true"; check "the stored hash accepts the password (the hash of the command line)" $?
    has "^prefs_accounts_verify: name=kid ok=false"; check "and refuses another one" $?
    sed -n 6p <<<"$A" | grep -q "count=1 .*form=true"; check "a second account whose name differs only in case is refused" $?
    sed -n 7p <<<"$A" | grep -q "count=1 accounts=kid\\[kids+public\\]hashed=true .*restarts=2"; check "editing changes the collections, keeps the password when none is typed and restarts the server" $?
    has "^prefs_accounts_verify: name=kid ok=true"; check "the old password still works after the edit" $?
    sed -n 8p <<<"$A" | grep -q "count=0 .*rows=1"; check "Remove takes the account away at once" $?
    sed -n 9p <<<"$A" | grep -q "count=1 accounts=kid\\[kids+public\\]hashed=true"; check "Undo brings it back with its collections and password" $?
    sed -n 10p <<<"$A" | grep -q "guests=public"; check "guests can be allowed for a collection" $?
    sed -n 11p <<<"$A" | grep -q "guests= "; check "and switched off again" $?
    grep -qF "a2id" "$d/cfg/bookx/config.json"; check "config.json holds an Argon2id hash" $?
    ! grep -rqa "SENT-acct-pw" "$d/cfg" "$d/cache"; check "no password is in any file" $?
    ! has "SENT-acct"; check "no password or hash is printed" $?
    ! has "^a11y-problem:"; check "every control of the accounts window and form has a name" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario accounts_win11_fit 'prefs:,wait:900,prefs:pick=Sync,wait:300,prefs:sync-advanced=open,wait:400,app:win-size=900x600,app:text-scale=1.5,wait:800,prefs:accounts-open,wait:700,a11y-audit:,prefs:accounts-new,wait:600,a11y-audit:,quit' '
    [ "$(grep -c "^dialog_fit: " <<<"$OUT")" -ge 2 ]; check "both accounts dialogs report their size" $?
    ! grep "^dialog_fit: " <<<"$OUT" | grep -q "fits=false"; check "at 900x600 and 1.5 times the text both dialogs fit the window" $?
    ! has "^a11y-problem:"; check "every control has a name at 1.5 times the text" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'
