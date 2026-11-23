//! The rules behind the *Library server accounts* screen (`ui/accounts.rs`): which names and
//! passwords are accepted, how an account is added, edited and removed in the settings, and which
//! collections (tags) can be offered. No GTK in here, so it is unit tested.
//!
//! A password is hashed with `bookx_serve::keyhash::hash_key` (salted Argon2id, the same call as
//! `bookx-cli serve --account`) before it reaches [`ShareSettings`]; the settings only ever hold the
//! hash. The caller does the hashing on a worker thread (it takes a moment on purpose) and hands the
//! hash to [`add`] / [`edit`].

use crate::core::library::Book;
use crate::i18n::{tr, trf};
use crate::sharing::{MIN_PASSWORD, ShareSettings, SharedAccount, USER};

/// Longest account name, in characters.
pub const MAX_NAME: usize = 48;
/// Longest password, in characters (the hash cost does not grow with it, but a megabyte is a mistake).
pub const MAX_PASSWORD: usize = 256;
/// Most accounts one library keeps.
pub const MAX_ACCOUNTS: usize = 50;
/// Most tags offered as collections (the most used first).
pub const MAX_CHOICES: usize = 200;

/// Tags as the server compares them: trimmed, lower case, once each, in the order given.
pub fn clean_collections(list: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in list {
        let s = s.trim().to_lowercase();
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

/// Why `name` cannot be an account name, or the name cleaned up. `editing` is the account being
/// edited (it may keep its own name).
pub fn check_name(
    name: &str,
    accounts: &[SharedAccount],
    editing: Option<&str>,
) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(tr("Give the account a name."));
    }
    if name.chars().count() > MAX_NAME {
        return Err(trf(
            "The name can have at most {} characters.",
            &[&MAX_NAME],
        ));
    }
    if name.contains(':') || name.chars().any(|c| c.is_control()) {
        // the login travels as `name:password`, so a colon would split it in the wrong place
        return Err(tr("The name must not contain a colon."));
    }
    if name.eq_ignore_ascii_case(USER) {
        return Err(trf(
            "“{}” is the name of the library password; choose another name.",
            &[&USER],
        ));
    }
    let taken = accounts
        .iter()
        .any(|a| a.name.eq_ignore_ascii_case(name) && Some(a.name.as_str()) != editing);
    if taken {
        return Err(tr("There is already an account with this name."));
    }
    Ok(name.to_string())
}

/// Why this password pair is not accepted.
pub fn check_password(password: &str, confirm: &str) -> Result<(), String> {
    let n = password.chars().count();
    if n < MIN_PASSWORD {
        return Err(trf(
            "The password needs at least {} characters.",
            &[&MIN_PASSWORD],
        ));
    }
    if n > MAX_PASSWORD {
        return Err(trf(
            "The password can have at most {} characters.",
            &[&MAX_PASSWORD],
        ));
    }
    if password != confirm {
        return Err(tr("The two passwords are not the same."));
    }
    Ok(())
}

/// Hashes a password exactly like the command line does (`--account name:password`). Slow on
/// purpose: call it off the main thread.
pub fn hash_password(password: &str) -> String {
    bookx_serve::keyhash::hash_key(password)
}

/// Adds an account whose password is already hashed.
pub fn add(
    settings: &mut ShareSettings,
    name: &str,
    hash: &str,
    collections: &[String],
) -> Result<(), String> {
    if settings.accounts.len() >= MAX_ACCOUNTS {
        return Err(trf("At most {} accounts can be added.", &[&MAX_ACCOUNTS]));
    }
    let name = check_name(name, &settings.accounts, None)?;
    if !bookx_serve::keyhash::is_hashed(hash) {
        // never store something that is not a hash, whatever the caller meant
        return Err(tr("The password was not prepared. Try again."));
    }
    settings.accounts.push(SharedAccount {
        name,
        hash: hash.to_string(),
        collections: clean_collections(collections),
    });
    Ok(())
}

/// Changes an account: a new name, new collections and, when `new_hash` is given, a new password.
/// The other accounts and the order do not change.
pub fn edit(
    settings: &mut ShareSettings,
    old_name: &str,
    new_name: &str,
    new_hash: Option<&str>,
    collections: &[String],
) -> Result<(), String> {
    let name = check_name(new_name, &settings.accounts, Some(old_name))?;
    if let Some(h) = new_hash {
        if !bookx_serve::keyhash::is_hashed(h) {
            return Err(tr("The password was not prepared. Try again."));
        }
    }
    let Some(a) = settings.accounts.iter_mut().find(|a| a.name == old_name) else {
        return Err(tr("This account is gone."));
    };
    a.name = name;
    if let Some(h) = new_hash {
        a.hash = h.to_string();
    }
    a.collections = clean_collections(collections);
    Ok(())
}

/// Removes an account; the position and the account come back for Undo.
pub fn remove(settings: &mut ShareSettings, name: &str) -> Option<(usize, SharedAccount)> {
    let i = settings.accounts.iter().position(|a| a.name == name)?;
    Some((i, settings.accounts.remove(i)))
}

/// Puts a removed account back where it was.
pub fn restore(settings: &mut ShareSettings, index: usize, account: SharedAccount) {
    if settings.accounts.iter().any(|a| a.name == account.name) {
        return;
    }
    let i = index.min(settings.accounts.len());
    settings.accounts.insert(i, account);
}

/// Sets what guests (visitors without a login) may read. Empty turns guests off, because a guest
/// without a collection would otherwise see everything.
pub fn set_guests(settings: &mut ShareSettings, collections: &[String]) {
    settings.guest_collections = clean_collections(collections);
}

/// What the row of an account says about its access.
pub fn access_text(collections: &[String]) -> String {
    match collections {
        [] => tr("Sees all books"),
        c if c.len() <= 3 => trf("Sees only: {}", &[&c.join(", ")]),
        c => trf(
            "Sees only: {}, and {} more",
            &[&c[..3].join(", "), &(c.len() - 3)],
        ),
    }
}

/// The tags of the library to choose from: lower case, the most used first, then by name.
pub fn tag_choices(books: &[Book]) -> Vec<(String, usize)> {
    let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
    for b in books {
        let mut seen: Vec<String> = Vec::new();
        for t in &b.meta.tags {
            let t = t.trim().to_lowercase();
            if !t.is_empty() && !seen.contains(&t) {
                *counts.entry(t.clone()).or_default() += 1;
                seen.push(t);
            }
        }
    }
    let mut v: Vec<(String, usize)> = counts.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v.truncate(MAX_CHOICES);
    v
}

/// The choices plus the collections an account already names that no book has any more (so editing
/// never drops one silently).
pub fn choices_with(selected: &[String], books: &[Book]) -> Vec<(String, usize)> {
    let mut v = tag_choices(books);
    for s in clean_collections(selected) {
        if !v.iter().any(|(t, _)| *t == s) {
            v.push((s, 0));
        }
    }
    v
}

/// What a dev hook may print about the accounts: names, access and whether each is hashed. Never a
/// hash and never a password.
pub fn dump(settings: &ShareSettings) -> String {
    let rows: Vec<String> = settings
        .accounts
        .iter()
        .map(|a| {
            format!(
                "{}[{}]hashed={}",
                a.name,
                a.collections.join("+"),
                bookx_serve::keyhash::is_hashed(&a.hash)
            )
        })
        .collect();
    format!(
        "count={} accounts={} guests={}",
        settings.accounts.len(),
        rows.join(","),
        settings.guest_collections.join("+")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s() -> ShareSettings {
        ShareSettings::default()
    }

    fn hash(p: &str) -> String {
        hash_password(p)
    }

    #[test]
    fn names_are_checked() {
        let mut st = s();
        add(&mut st, "Kid", &hash("secret1"), &["kids".into()]).unwrap();
        assert_eq!(check_name("  Mum ", &st.accounts, None).unwrap(), "Mum");
        assert!(check_name("", &st.accounts, None).is_err());
        assert!(check_name("a:b", &st.accounts, None).is_err());
        assert!(check_name("tab\there", &st.accounts, None).is_err());
        assert!(check_name(&"x".repeat(MAX_NAME + 1), &st.accounts, None).is_err());
        assert!(check_name(&"x".repeat(MAX_NAME), &st.accounts, None).is_ok());
        // the shared login keeps its name, and a name is not used twice (in any case)
        assert!(check_name("bookx", &st.accounts, None).is_err());
        assert!(check_name("BOOKX", &st.accounts, None).is_err());
        assert!(check_name("kid", &st.accounts, None).is_err());
        // an account may keep its own name when edited
        assert!(check_name("Kid", &st.accounts, Some("Kid")).is_ok());
    }

    #[test]
    fn passwords_are_checked() {
        assert!(check_password("short", "short").is_err());
        assert!(check_password("longenough", "different1").is_err());
        assert!(check_password("longenough", "longenough").is_ok());
        assert!(
            check_password(&"x".repeat(MAX_PASSWORD + 1), &"x".repeat(MAX_PASSWORD + 1)).is_err()
        );
        // a colon is fine in a password (the login is split at the first one)
        assert!(check_password("pass:word", "pass:word").is_ok());
    }

    #[test]
    fn a_new_account_holds_a_hash_that_verifies_and_never_the_password() {
        let mut st = s();
        add(
            &mut st,
            "kid",
            &hash("sentinel-pw-123"),
            &[" Kids ".into(), "kids".into()],
        )
        .unwrap();
        let a = &st.accounts[0];
        assert_eq!(a.collections, vec!["kids"]);
        assert!(bookx_serve::keyhash::is_hashed(&a.hash));
        assert!(bookx_serve::keyhash::verify(
            "kid",
            &a.hash,
            "sentinel-pw-123"
        ));
        assert!(!bookx_serve::keyhash::verify(
            "kid",
            &a.hash,
            "other-password"
        ));
        // what is saved has no trace of the password
        let json = serde_json::to_string(&st).unwrap();
        assert!(!json.contains("sentinel-pw-123"));
        assert!(json.contains("$a2id$"));
        // the same hash the command line makes for the same input
        let cli = bookx_serve::accounts::parse_arg("kid:sentinel-pw-123").unwrap();
        assert!(bookx_serve::keyhash::verify(
            "kid",
            &cli.hash,
            "sentinel-pw-123"
        ));
        assert!(bookx_serve::keyhash::is_hashed(&cli.hash));
    }

    #[test]
    fn something_that_is_not_a_hash_is_refused() {
        let mut st = s();
        assert!(add(&mut st, "kid", "plain-password", &[]).is_err());
        assert!(st.accounts.is_empty());
        add(&mut st, "kid", &hash("secret1"), &[]).unwrap();
        assert!(edit(&mut st, "kid", "kid", Some("plain"), &[]).is_err());
    }

    #[test]
    fn the_server_config_gets_the_accounts_and_the_guest_list() {
        let mut st = s();
        add(&mut st, "kid", &hash("secret1"), &["kids".into()]).unwrap();
        set_guests(&mut st, &["Public".into()]);
        let provider: bookx_serve::server::LibraryProvider =
            std::sync::Arc::new(|| bookx_serve::server::LibrarySnapshot {
                root: std::path::PathBuf::new(),
                books: Vec::new(),
            });
        let cfg = crate::sharing::server_config(&st, provider, &std::env::temp_dir()).unwrap();
        assert_eq!(cfg.accounts.len(), 1);
        assert_eq!(cfg.accounts[0].name, "kid");
        assert_eq!(cfg.guest, Some(vec!["public".to_string()]));
    }

    #[test]
    fn edit_keeps_the_hash_unless_a_new_one_comes() {
        let mut st = s();
        let h = hash("secret1");
        add(&mut st, "kid", &h, &["a".into()]).unwrap();
        add(&mut st, "mum", &hash("secret2"), &[]).unwrap();
        edit(&mut st, "kid", "child", None, &["b".into(), "c".into()]).unwrap();
        assert_eq!(st.accounts[0].name, "child");
        assert_eq!(st.accounts[0].hash, h);
        assert_eq!(st.accounts[0].collections, vec!["b", "c"]);
        let h2 = hash("secret3");
        edit(&mut st, "child", "child", Some(&h2), &[]).unwrap();
        assert_eq!(st.accounts[0].hash, h2);
        assert!(st.accounts[0].collections.is_empty());
        // renaming onto another account, or a missing one, fails
        assert!(edit(&mut st, "child", "MUM", None, &[]).is_err());
        assert!(edit(&mut st, "nobody", "x", None, &[]).is_err());
        assert_eq!(st.accounts[1].name, "mum");
    }

    #[test]
    fn removing_and_restoring_keeps_the_place() {
        let mut st = s();
        for n in ["a", "b", "c"] {
            add(&mut st, n, &hash("secret1"), &[]).unwrap();
        }
        let (i, acc) = remove(&mut st, "b").unwrap();
        assert_eq!(i, 1);
        assert_eq!(st.accounts.len(), 2);
        assert!(remove(&mut st, "b").is_none());
        restore(&mut st, i, acc.clone());
        restore(&mut st, i, acc);
        let names: Vec<_> = st.accounts.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
    }

    #[test]
    fn there_is_a_limit_on_accounts() {
        let mut st = s();
        let h = hash("secret1");
        for i in 0..MAX_ACCOUNTS {
            add(&mut st, &format!("u{i}"), &h, &[]).unwrap();
        }
        assert!(add(&mut st, "one-too-many", &h, &[]).is_err());
    }

    #[test]
    fn collections_are_cleaned_and_listed_by_use() {
        use crate::core::opf::Metadata;
        let book = |tags: &[&str]| {
            let mut b = crate::core::library::Book {
                id: 1,
                file: "a.epub".into(),
                cover: None,
                meta: Metadata::default(),
                added: chrono::Utc::now(),
                modified: chrono::Utc::now(),
                last_read: None,
                size: 0,
                progress: 0.0,
                position: None,
                bookmarks: vec![],
                highlights: vec![],
                formats: vec![],
                resources: vec![],
                drm: String::new(),
                custom: Default::default(),
                marked: false,
            };
            b.meta.tags = tags.iter().map(|t| t.to_string()).collect();
            b
        };
        let books = vec![
            book(&["Kids", "kids", "Fiction"]),
            book(&["fiction"]),
            book(&["Cooking", " "]),
        ];
        let c = tag_choices(&books);
        assert_eq!(
            c,
            vec![
                ("fiction".to_string(), 2),
                ("cooking".to_string(), 1),
                ("kids".to_string(), 1)
            ]
        );
        // a collection no book has any more stays in the list of the account that names it
        let c = choices_with(&["gone".into(), "kids".into()], &books);
        assert!(c.contains(&("gone".to_string(), 0)));
        assert_eq!(c.iter().filter(|(t, _)| t == "kids").count(), 1);
    }

    #[test]
    fn the_access_text_names_the_collections() {
        assert_eq!(access_text(&[]), "Sees all books");
        assert_eq!(access_text(&["a".into(), "b".into()]), "Sees only: a, b");
        let many: Vec<String> = ["a", "b", "c", "d", "e"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(access_text(&many), "Sees only: a, b, c, and 2 more");
    }

    #[test]
    fn the_dump_never_has_a_hash_or_a_password() {
        let mut st = s();
        let h = hash("sentinel-pw-123");
        add(&mut st, "kid", &h, &["kids".into()]).unwrap();
        let d = dump(&st);
        assert!(d.contains("kid[kids]hashed=true"));
        assert!(!d.contains(&h) && !d.contains("sentinel-pw-123") && !d.contains("a2id"));
    }
}
