# Scenarios of the backup wave: the real Add / Edit Place window, one writer per place (a busy place, taking
# it over, an old lock), and Change Backup Passphrase through the real window. Sourced by ui-tests.sh (uses its
# `scenario`, `check`, `has`, $ROOT, $d, $CLI; `@D@` is the scenario's own folder).
#
# Dev hooks (all `library:health=<cmd>`; values have no commas):
#   backup-edit-open[=N]          the real Add Place window (or Edit Place for the Nth place)
#   backup-edit-set=FIELD=VALUE   kind (0 folder, 1 network, 2 cloud, 3 WebDAV), name, folder, url, user, password,
#                                 dav_folder, keep, encrypt, books, covers, max_mb (1/0 for the switches)
#   backup-edit-test | -save | -close | -dump
#       dump prints `backup_edit: open=true title="…" kind=N name="…" folder="…" folder_group=B dav_group=B url="…"
#       keep=N encrypt=B books=B covers=B max_mb=N result="title | subtitle" result_visible=B save_sensitive=B`
#   backup-lock=N|HOST|MINUTES    plants another computer's lock (last sign of life MINUTES ago) in place N
#   backup-takeover=N             Take Over This Place for place N (no question in a script)
#   backup-rekey-dialog | backup-rekey-set=old|new|repeat=TEXT | backup-rekey-go | backup-rekey-dump
#       dump prints `backup_rekey_dialog: open=B go_sensitive=B status="…" old_len=N new_len=N`

scenario backup_place_dialog 'library:health=open,wait:800,library:health=tab=backups,wait:300,library:health=backup-edit-open,wait:500,library:health=backup-edit-dump,library:health=backup-edit-save,library:health=backup-edit-dump,library:health=backup-edit-set=kind=3,library:health=backup-edit-dump,library:health=backup-edit-set=kind=0,library:health=backup-edit-set=name=Disk,library:health=backup-edit-set=folder=@D@/disk,library:health=backup-edit-set=books=0,library:health=backup-edit-set=keep=3,library:health=backup-edit-set=max_mb=50,library:health=backup-edit-dump,library:health=backup-edit-save,wait:500,library:health=backup-edit-dump,library:health=dump,library:health=backup-edit-open=1,wait:500,library:health=backup-edit-dump,library:health=backup-edit-set=name=Disk two,library:health=backup-edit-set=encrypt=1,library:health=backup-edit-set=books=1,library:health=backup-edit-save,wait:500,library:health=dump,quit' '
    E=$(grep "^backup_edit:" <<<"$OUT")
    sed -n 1p <<<"$E" | grep -q "open=true title=\"Add a Place\" kind=0 name=\"\" folder=\"\" folder_group=true dav_group=false"; check "Add a Place opens on a folder, with nothing filled in" $?
    sed -n 1p <<<"$E" | grep -q "encrypt=false books=true covers=true max_mb=0"; check "by default books and covers are copied, no size limit, not encrypted" $?
    sed -n 2p <<<"$E" | grep -q "result=\"Cannot save | Choose a folder or enter an address first\" result_visible=true"; check "Save without a folder says what is missing and keeps the window open" $?
    sed -n 3p <<<"$E" | grep -q "kind=3 .*folder_group=false dav_group=true .*encrypt=true"; check "choosing WebDAV shows the server fields and switches encryption on for a new place" $?
    sed -n 4p <<<"$E" | grep -q "name=\"Disk\" folder=\"$d/disk\" .*keep=3 encrypt=true books=false covers=true max_mb=50"; check "the fields hold what was entered" $?
    [ "$(grep -c "^backup_edit: closed" <<<"$OUT")" = 1 ]; check "Save closes the window" $?
    has "^backup_place: Disk \| .*encrypt=true .*books=false covers=true max_mb=50"; check "the new place has the settings of the window" $?
    sed -n 6p <<<"$E" | grep -q "open=true title=\"Edit Place\" kind=0 name=\"Disk\" folder=\"$d/disk\" .*keep=3 encrypt=true books=false covers=true max_mb=50"; check "Edit Place opens with the saved values" $?
    has "^backup_places: 1"; check "editing a place does not add another" $?
    has "^backup_place: Disk two \| .*encrypt=true .*books=true"; check "the edited place has the new name and books switched back on" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario backup_webdav_place_dialog 'library:health=open,wait:800,library:health=tab=backups,wait:300,library:health=backup-edit-open,wait:500,library:health=backup-edit-set=folder=@D@/usb,library:health=backup-edit-test,wait:1500,library:health=backup-edit-dump,library:health=backup-edit-set=folder=@D@/nowhere/deeper,library:health=backup-edit-test,wait:1500,library:health=backup-edit-dump,library:health=backup-edit-set=kind=3,library:health=backup-edit-set=url=http://127.0.0.1:@PORT@/files/bob/,library:health=backup-edit-set=user=bob,library:health=backup-edit-set=password=s3cret,library:health=backup-edit-set=dav_folder=Bookx backups,library:health=backup-edit-set=encrypt=0,library:health=backup-edit-test,wait:2500,library:health=backup-edit-dump,library:health=backup-edit-close,wait:300,library:health=backup-edit-dump,quit' '
    E=$(grep "^backup_edit:" <<<"$OUT")
    sed -n 1p <<<"$E" | grep -q "result=\"Works | Can write to .*usb"; check "Test tells a working folder apart" $?
    sed -n 2p <<<"$E" | grep -q "result=\"Does not work | .*not there"; check "Test says a missing drive is not there, in words" $?
    sed -n 3p <<<"$E" | grep -q "kind=3 .*result=\"Works | Can write to http"; check "Test reaches a WebDAV server with the typed user and password" $?
    sed -n 4p <<<"$E" | grep -q "^backup_edit: closed"; check "Cancel closes the window without adding a place" $?
    ! [ -d "$d/nowhere" ]; check "testing a place that is not there did not create it" $?
    ! has "^backup_place:"; check "no place was added by testing" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario backup_busy_place_and_take_over 'library:health=open,wait:800,library:health=backup-add=NAS|@D@/nas,library:health=backup,wait:4000,library:health=backup-lock=1|kitchen-pc|1,library:health=backup,wait:3000,library:health=dump,library:health=backup-takeover=1,wait:1500,library:health=backup,wait:4000,library:health=dump,quit' '
    has "^backup: failed NAS this backup place is in use: kitchen-pc has been writing to it"; check "a fresh lock of another computer stops the copy and names the computer" $?
    has "^backup_place: NAS \| Failed.*in use"; check "the place says it is in use, not that it is broken" $?
    has "^backup_in_use: 1"; check "the window remembers which place is busy" $?
    has "^backup: took over NAS from kitchen-pc"; check "Take Over names the computer whose lock it removed" $?
    [ "$(grep -c "^backup: copied NAS" <<<"$OUT")" = 2 ]; check "the first copy and the copy after taking over both went through" $?
    has "^backup_in_use: 0"; check "afterwards nothing is busy" $?
    [ ! -e "$d/nas/Bookx Backup/lock.json" ]; check "no lock is left behind after a copy" $?
    [ "$(ls "$d/nas/Bookx Backup/snapshots" | wc -l)" -ge 1 ]; check "the backup in the place is intact" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario backup_old_lock_is_taken_over_by_itself 'library:health=open,wait:800,library:health=backup-add=NAS|@D@/nas,library:health=backup,wait:4000,library:health=backup-lock=1|kitchen-pc|45,library:health=backup,wait:3000,library:health=dump,quit' '
    [ "$(grep -c "^backup: copied NAS" <<<"$OUT")" = 2 ]; check "a lock nobody renewed for 45 minutes does not stop the copy" $?
    ! has "^backup: in use"; check "and nobody is asked anything" $?
    [ ! -e "$d/nas/Bookx Backup/lock.json" ]; check "the old lock is gone and no new one remains" $?'

scenario backup_change_passphrase 'library:health=open,wait:800,library:health=backup-passphrase=first-phrase-1,library:health=backup-add=Offsite|@D@/offsite|encrypt,library:health=backup,wait:9000,library:health=backup-rekey-dialog,wait:400,library:health=backup-rekey-dump,library:health=backup-rekey-set=new=second-phrase-2,library:health=backup-rekey-dump,library:health=backup-rekey-set=repeat=something-else-3,library:health=backup-rekey-dump,library:health=backup-rekey-go,library:health=backup-rekey-set=repeat=second-phrase-2,library:health=backup-rekey-dump,library:health=backup-rekey-go,wait:12000,library:health=dump,library:import='"$ROOT"'/fb2,wait:3000,library:health=backup-reload,wait:9000,library:health=backup-restore-remote1,wait:9000,library:health=dump,quit' '
    R=$(grep "^backup_rekey_dialog:" <<<"$OUT")
    sed -n 1p <<<"$R" | grep -q "open=true go_sensitive=false status=\"\" old_len=0 new_len=0"; check "the window opens empty and Change is off until the fields are right" $?
    sed -n 2p <<<"$R" | grep -q "go_sensitive=false status=\"The two new passphrases are not the same\""; check "a new passphrase without its repeat is refused with the reason" $?
    sed -n 3p <<<"$R" | grep -q "go_sensitive=false status=\"The two new passphrases are not the same\""; check "two different repeats are still refused" $?
    sed -n 4p <<<"$R" | grep -q "go_sensitive=true status=\"\""; check "matching fields switch Change on" $?
    has "^backup: rekey Offsite ok objects=[0-9]+ converted=[0-9]+ already=0 resumed=false"; check "every object of the place was re-encrypted" $?
    has "^backup: rekey done complete=true"; check "the change finished for every place" $?
    has "^backup_pass_set: true"; check "the new passphrase is the saved one" $?
    ! grep -rq "first-phrase-1\|second-phrase-2" "$d/cfg"; check "neither passphrase is in the settings file" $?
    [ ! -e "$d/offsite/Bookx Backup/rekey.json" ] && [ ! -e "$d/offsite/Bookx Backup/lock.json" ]; check "no change marker and no lock are left in the place" $?
    grep -q "\"rekeyed\"" "$d/offsite/Bookx Backup/bookx-repo.json"; check "the place records that its passphrase was changed" $?
    BOOKX_BACKUP_PASSPHRASE=second-phrase-2 "$CLI" backup list --to "$d/offsite" >/dev/null 2>&1; check "the new passphrase opens the place" $?
    ! BOOKX_BACKUP_PASSPHRASE=first-phrase-1 "$CLI" backup list --to "$d/offsite" >/dev/null 2>&1; check "the old one does not" $?
    has "^health: restored books=2"; check "restoring with the saved (new) passphrase works" $?
    ! grep -q "Solaris Imported" <<<"$LIB"; check "and gives the catalogue of the backup" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'

scenario backup_change_passphrase_refused 'library:health=open,wait:800,library:health=backup-passphrase=first-phrase-1,library:health=backup-add=Offsite|@D@/offsite|encrypt,library:health=backup,wait:9000,library:health=backup-rekey-dialog,wait:400,library:health=backup-rekey-set=new=short,library:health=backup-rekey-set=repeat=short,library:health=backup-rekey-dump,library:health=backup-rekey-set=new=first-phrase-1,library:health=backup-rekey-set=repeat=first-phrase-1,library:health=backup-rekey-dump,library:health=backup-rekey-go,library:health=backup-rekey-set=old=wrong-phrase-9,library:health=backup-rekey-set=new=second-phrase-2,library:health=backup-rekey-set=repeat=second-phrase-2,library:health=backup-rekey-go,wait:9000,library:health=dump,quit' '
    R=$(grep "^backup_rekey_dialog:" <<<"$OUT")
    sed -n 1p <<<"$R" | grep -q "go_sensitive=false status=\"Use at least 8 characters\""; check "a short passphrase is refused" $?
    sed -n 2p <<<"$R" | grep -q "go_sensitive=false status=\"The new passphrase is the same as the current one\""; check "the same passphrase is refused" $?
    has "^backup: rekey Offsite failed .*old passphrase does not open this backup"; check "a wrong current passphrase fails with that reason" $?
    has "^backup: rekey done complete=false"; check "and the change is reported as not done" $?
    BOOKX_BACKUP_PASSPHRASE=first-phrase-1 "$CLI" backup list --to "$d/offsite" >/dev/null 2>&1; check "the backups still open with the original passphrase" $?
    [ ! -e "$d/offsite/Bookx Backup/rekey.json" ]; check "a refused change leaves no marker behind" $?
    ! has "panicked|CRITICAL|Gtk-WARNING"; check "no errors or GTK warnings" $?'
