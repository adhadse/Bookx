# Scenarios of the password store without a keyring (BOOKX_SECRETS=none): nothing secret reaches a plain file, the
# master-password store (`secrets.vault`) holds what the user chooses to save, and passwords an older version left in
# `config.json` are scrubbed. Sourced by ui-tests.sh. Dev commands (`secrets:`): `set=KEY:VALUE`, `create=PASSWORD`,
# `unlock=PASSWORD`, `relock` (what a restart does to memory), `decline`, `reset`, `state` (`secrets_state: mode=… file=…
# declined=…`), `caption`, `has=KEY:VALUE` (`secrets_has: key=… equal=…`). The dialogs stay away while a dev script runs.

scenario secrets_no_keyring_not_in_file 'secrets:set=email-password:SENT-nokr-2222,wait:400,secrets:state,secrets:caption,secrets:has=email-password:SENT-nokr-2222,quit' '
    ! grep -rqa "SENT-nokr-2222" "$d/cfg"; check "a password typed without a keyring and without a master password is in no file" $?
    has "^secrets_caption: Not saved: it is forgotten when Bookx closes\\."; check "the field says it is not saved" $?
    has "^secrets_state: mode=Fresh file=false"; check "no encrypted store was made behind the user back" $?
    has "^secrets_has: key=email-password equal=true"; check "the password still works for this session" $?
    [ ! -e "$d/cfg/bookx/secrets.vault" ]; check "there is no secrets.vault" $?
    grep -q "\"password\": \"\"" "$d/cfg/bookx/config.json"; check "the settings file has the field empty" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario secrets_master_password_vault 'secrets:set=email-password:SENT-vault-3333,secrets:create=a long master pw,wait:400,secrets:state,secrets:caption,secrets:relock,secrets:has=email-password:SENT-vault-3333,secrets:unlock=wrong wrong wrong,secrets:unlock=a long master pw,secrets:has=email-password:SENT-vault-3333,quit' '
    V="$d/cfg/bookx/secrets.vault"
    [ -f "$V" ]; check "creating a master password makes secrets.vault" $?
    [ "$(stat -c %a "$V")" = 600 ]; check "the store is readable by the user only (0600)" $?
    [ "$(stat -c %a "$d/cfg/bookx")" = 700 ]; check "and the settings folder is private (0700)" $?
    ! grep -rqa "SENT-vault-3333" "$d/cfg"; check "the password is in no file in clear text (the store is encrypted)" $?
    ! grep -qa "master pw" "$V" "$d/cfg/bookx/config.json"; check "the master password is never stored" $?
    has "^secrets_state: mode=Vault file=true"; check "the store is open after creating it" $?
    has "^secrets_caption: Kept in Bookx.s encrypted password store\\."; check "the field says where it is kept" $?
    has "^secrets_relock: mode=Locked"; check "after a restart (relock) the store is locked" $?
    sed -n 1p <<<"$(grep "^secrets_has: " <<<"$OUT")" | grep -q "equal=false"; check "while locked the password is not there" $?
    has "^secrets_unlock: error"; check "a wrong master password is refused" $?
    sed -n 2p <<<"$(grep "^secrets_has: " <<<"$OUT")" | grep -q "equal=true"; check "the right master password brings the saved password back" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario secrets_migrate_plaintext 'wait:800,secrets:has=email-password:SENT-legacy-4444,secrets:state,secrets:create=another long master,wait:300,secrets:relock,secrets:unlock=another long master,secrets:has=email-password:SENT-legacy-4444,quit' '
    ! grep -rqa "SENT-legacy-4444" "$d/cfg" --exclude=secrets.vault; check "the plain text password was removed from config.json and its old copies" $?
    ! grep -qa "SENT-legacy-4444" "$d/cfg/bookx/secrets.vault"; check "and it is not in the store in clear text either" $?
    [ "$(stat -c %a "$d/cfg/bookx/config.json")" = 600 ]; check "config.json is now private (0600)" $?
    [ "$(stat -c %a "$d/cfg/bookx")" = 700 ]; check "the settings folder is private (0700)" $?
    sed -n 1p <<<"$(grep "^secrets_has: " <<<"$OUT")" | grep -q "equal=true"; check "the migrated password stays usable for the session (nothing is lost)" $?
    sed -n 2p <<<"$(grep "^secrets_has: " <<<"$OUT")" | grep -q "equal=true"; check "and saving it into a new encrypted store keeps it across a restart" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'
