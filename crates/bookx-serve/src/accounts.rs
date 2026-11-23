//! Accounts of the content server with a list of collections each may see, and a read-only guest.
//!
//! * `users` of [`ServerConfig`](crate::server::ServerConfig) (one shared name and password) keep
//!   full access.
//! * An [`Account`] has its own name, a salted Argon2id hash of the password (never the password:
//!   the settings file holds only the hash) and the collections it may see. A collection is a tag of
//!   the book; no collections means all books.
//! * A guest (no login) can be allowed to read the books of some collections, only with `GET` and
//!   `HEAD`. A request that presents wrong credentials is refused, never silently treated as guest,
//!   so the failed-login limit still counts it.
//!
//! The check is on the books themselves: what an account may not see is missing from the catalogue,
//! the search, the downloads and the covers (a book outside the collections is a plain `404`).

use bookx_core::library::Book;

use crate::keyhash;

/// A login of the content server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub name: String,
    /// `keyhash::hash_key` of the password.
    pub hash: String,
    /// Tags of the books this account may see; empty means all.
    pub collections: Vec<String>,
}

impl Account {
    /// Makes an account from a password (hashed here, the password is not kept).
    pub fn new(name: &str, password: &str, collections: &[String]) -> Account {
        Account {
            name: name.to_string(),
            hash: keyhash::hash_key(password),
            collections: clean_collections(collections),
        }
    }
}

/// What a request is allowed to see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Access {
    /// Every book.
    All,
    /// Books with one of these tags (lower case).
    Collections(Vec<String>),
}

impl Access {
    pub fn of_collections(c: &[String]) -> Access {
        let c = clean_collections(c);
        if c.is_empty() {
            Access::All
        } else {
            Access::Collections(c)
        }
    }

    pub fn allows(&self, book: &Book) -> bool {
        match self {
            Access::All => true,
            Access::Collections(c) => book
                .meta
                .tags
                .iter()
                .any(|t| c.contains(&t.trim().to_lowercase())),
        }
    }
}

fn clean_collections(c: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in c {
        let s = s.trim().to_lowercase();
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

/// The account whose name and password the `Authorization: Basic` header carries. The time spent
/// does not tell whether the name exists.
pub fn login<'a>(accounts: &'a [Account], header: Option<&str>) -> Option<&'a Account> {
    let (user, pass) = crate::http::parse_basic_auth(header?)?;
    match accounts.iter().find(|a| a.name == user) {
        Some(a) => keyhash::verify(&a.name, &a.hash, &pass).then_some(a),
        None => {
            keyhash::dummy_verify(&pass);
            None
        }
    }
}

/// `name:password` or `name:password:collection,collection` (a command-line argument). The password
/// must not contain a colon.
pub fn parse_arg(s: &str) -> Result<Account, String> {
    let mut it = s.splitn(3, ':');
    let name = it.next().unwrap_or("").trim();
    let pass = it.next().unwrap_or("");
    if name.is_empty() || pass.is_empty() {
        return Err(
            "an account looks like name:password or name:password:collection,collection".into(),
        );
    }
    let colls: Vec<String> = it
        .next()
        .map(|c| c.split(',').map(str::to_string).collect())
        .unwrap_or_default();
    Ok(Account::new(name, pass, &colls))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book(tags: &[&str]) -> Book {
        Book {
            id: 1,
            file: "a.epub".into(),
            cover: None,
            meta: bookx_core::opf::Metadata {
                title: "T".into(),
                tags: tags.iter().map(|t| t.to_string()).collect(),
                ..Default::default()
            },
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
            source_ids: Vec::new(),
        }
    }

    fn basic(u: &str, p: &str) -> String {
        use base64::Engine;
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("{u}:{p}"))
        )
    }

    #[test]
    fn the_hash_is_stored_not_the_password() {
        let a = Account::new(
            "kid",
            "s3cret pass",
            &["Kids".into(), " kids ".into(), "".into()],
        );
        assert!(!a.hash.contains("s3cret"));
        assert!(keyhash::is_hashed(&a.hash));
        assert_eq!(a.collections, vec!["kids"]);
    }

    #[test]
    fn login_needs_the_right_name_and_password() {
        let accts = vec![
            Account::new("kid", "pw1", &[]),
            Account::new("mum", "pw2", &[]),
        ];
        assert_eq!(
            login(&accts, Some(&basic("kid", "pw1"))).unwrap().name,
            "kid"
        );
        assert_eq!(
            login(&accts, Some(&basic("mum", "pw2"))).unwrap().name,
            "mum"
        );
        assert!(login(&accts, Some(&basic("kid", "pw2"))).is_none());
        assert!(login(&accts, Some(&basic("nobody", "pw1"))).is_none());
        assert!(login(&accts, Some("Basic !!!")).is_none());
        assert!(login(&accts, Some("Bearer x")).is_none());
        assert!(login(&accts, None).is_none());
    }

    #[test]
    fn collections_decide_what_is_visible() {
        let mut b = book(&["Fantasy", "Kids"]);
        assert!(Access::All.allows(&b));
        assert!(Access::of_collections(&["kids".into()]).allows(&b));
        assert!(Access::of_collections(&[" FANTASY ".into()]).allows(&b));
        assert!(!Access::of_collections(&["adult".into()]).allows(&b));
        assert_eq!(Access::of_collections(&[]), Access::All);
        b.meta.tags.clear();
        assert!(
            !Access::of_collections(&["kids".into()]).allows(&b),
            "untagged books are not shared"
        );
    }

    #[test]
    fn arguments() {
        let a = parse_arg("kid:pw:Kids,Comics").unwrap();
        assert_eq!((a.name.as_str(), a.collections.len()), ("kid", 2));
        assert!(parse_arg("kid:pw").unwrap().collections.is_empty());
        assert!(parse_arg("kid").is_err());
        assert!(parse_arg(":pw").is_err());
        assert!(parse_arg("kid:").is_err());
    }
}
