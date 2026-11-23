//! Custom columns on a library: definitions, editing with undo, composites, sorting, search.

use std::path::{Path, PathBuf};

use bookx_core::custom::{ColumnDef, ColumnKind, register_composite_renderer};
use bookx_core::library::{AddOutcome, Book, Library, MemoryBackend, SortKey};
use bookx_core::query::Query;

fn tmp() -> PathBuf {
    let d = std::env::temp_dir().join(format!("bookx-custom-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn add(lib: &mut Library, scratch: &Path, title: &str) -> u64 {
    let p = scratch.join(format!("{title}.md"));
    std::fs::write(
        &p,
        format!("---\ntitle: \"{title}\"\nauthor: \"A\"\n---\n# One\n\nHello {title}.\n"),
    )
    .unwrap();
    match lib.add_path(&p, false).unwrap() {
        AddOutcome::Added(id) => id,
        AddOutcome::Duplicate(_) => panic!("duplicate"),
    }
}

fn col(label: &str, kind: ColumnKind) -> ColumnDef {
    ColumnDef {
        label: label.into(),
        name: label.to_uppercase(),
        kind,
        ..Default::default()
    }
}

fn render(tpl: &str, b: &Book) -> String {
    let mut s = tpl.replace("{title}", &b.meta.title);
    for (k, v) in &b.custom {
        s = s.replace(&format!("{{#{k}}}"), v);
    }
    s
}

fn ids(lib: &Library, q: &str) -> Vec<u64> {
    lib.search(q, SortKey::Title, true)
}

#[test]
fn definitions_values_search_sort_and_undo() {
    register_composite_renderer(render);
    let dir = tmp();
    let backend = MemoryBackend::new();
    let mut lib = Library::with_backend(&dir.join("lib"), Box::new(backend.clone())).unwrap();
    let a = add(&mut lib, &dir, "Alpha");
    let b = add(&mut lib, &dir, "Beta");
    let c = add(&mut lib, &dir, "Gamma");
    assert!(lib.columns().is_empty());

    let mut mood = col("mood", ColumnKind::Enumeration);
    mood.values = vec!["Calm".into(), "Tense".into()];
    let mut label = col("label", ColumnKind::Composite);
    label.template = "{title}/{#mood}".into();
    lib.set_columns(vec![
        mood,
        col("pages", ColumnKind::Integer),
        col("done", ColumnKind::Bool),
        label,
    ])
    .unwrap();
    assert_eq!(lib.columns().len(), 4);

    // values are validated and stored normalised
    lib.set_custom(a, "pages", "300").unwrap();
    lib.set_custom(b, "pages", "1,200").unwrap();
    lib.set_custom(c, "pages", "45").unwrap();
    lib.set_custom(a, "mood", "tense").unwrap();
    lib.set_custom(b, "done", "yes").unwrap();
    assert_eq!(lib.get(a).unwrap().custom_value("mood"), "Tense");
    assert_eq!(lib.get(b).unwrap().custom_value("pages"), "1200");
    assert!(lib.set_custom(a, "mood", "sleepy").is_err());
    assert!(lib.set_custom(a, "pages", "many").is_err());
    assert!(lib.set_custom(a, "nope", "x").is_err());
    assert!(
        lib.set_custom(a, "label", "x").is_err(),
        "composites are computed"
    );
    assert_eq!(
        lib.get(a).unwrap().custom_value("pages"),
        "300",
        "a bad value changes nothing"
    );

    // composites follow the other values
    assert_eq!(lib.get(a).unwrap().custom_value("label"), "Alpha/Tense");
    assert_eq!(lib.get(c).unwrap().custom_value("label"), "Gamma/{#mood}");

    // search, as numbers
    assert_eq!(ids(&lib, "#pages:>100"), vec![a, b]);
    assert_eq!(ids(&lib, "#pages:<100"), vec![c]);
    assert_eq!(ids(&lib, "#done:true"), vec![b]);
    assert_eq!(ids(&lib, "#mood:tense"), vec![a]);
    assert_eq!(ids(&lib, "#label:alpha"), vec![a]);
    assert_eq!(ids(&lib, "#mood:empty"), vec![b, c]);

    // sorted by a column: numbers numerically, empty values last
    let mut v = vec![a, b, c];
    lib.sort_by_custom(&mut v, "pages", true);
    assert_eq!(v, vec![c, a, b]);
    lib.sort_by_custom(&mut v, "pages", false);
    assert_eq!(v, vec![b, a, c]);
    lib.sort_by_custom(&mut v, "mood", true);
    assert_eq!(v[0], a);

    // one undo step per edit
    lib.set_custom(a, "pages", "").unwrap();
    assert_eq!(lib.get(a).unwrap().custom_value("pages"), "");
    lib.undo_last().unwrap();
    assert_eq!(lib.get(a).unwrap().custom_value("pages"), "300");

    // bulk edit: one undo takes all back
    let n = lib
        .edit_custom(
            "Edited 3 books".into(),
            &[
                (a, vec![("done".into(), "no".into())]),
                (b, vec![("done".into(), "no".into())]),
                (c, vec![("done".into(), "no".into())]),
            ],
        )
        .unwrap();
    assert_eq!(n, 3);
    assert_eq!(ids(&lib, "#done:false").len(), 3);
    lib.undo_last().unwrap();
    assert_eq!(ids(&lib, "#done:true"), vec![b]);
    assert!(ids(&lib, "#done:false").is_empty());

    // values and definitions survive a reopen
    drop(lib);
    let mut lib = Library::with_backend(&dir.join("lib"), Box::new(backend)).unwrap();
    assert_eq!(lib.columns().len(), 4);
    assert_eq!(lib.get(b).unwrap().custom_value("pages"), "1200");
    assert_eq!(lib.custom_values("mood"), vec![("Tense".to_string(), 1)]);

    // a type cannot change; removing a column removes its values
    let mut changed = lib.columns().to_vec();
    changed[1].kind = ColumnKind::Text;
    assert!(lib.set_columns(changed).is_err());
    let rest: Vec<ColumnDef> = lib
        .columns()
        .iter()
        .filter(|c| c.label != "pages")
        .cloned()
        .collect();
    lib.set_columns(rest).unwrap();
    assert!(!lib.get(b).unwrap().custom.contains_key("pages"));
    assert!(!Query::parse("#pages:>1").matches(lib.get(b).unwrap()));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn bad_definitions_are_refused() {
    let dir = tmp();
    let mut lib = Library::in_memory(&dir.join("lib")).unwrap();
    assert!(
        lib.set_columns(vec![col("title", ColumnKind::Text)])
            .is_err()
    );
    assert!(
        lib.set_columns(vec![col("a", ColumnKind::Text), col("a", ColumnKind::Text)])
            .is_err()
    );
    assert!(
        lib.set_columns(vec![col("e", ColumnKind::Enumeration)])
            .is_err()
    );
    assert!(lib.columns().is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn search_and_replace_previews_applies_and_undoes() {
    use bookx_core::replace::{Rule, Target, apply, preview};
    let dir = tmp();
    let mut lib = Library::in_memory(&dir.join("lib")).unwrap();
    let a = add(&mut lib, &dir, "The Alpha");
    let b = add(&mut lib, &dir, "The Beta");
    let c = add(&mut lib, &dir, "Gamma");
    lib.set_columns(vec![
        col("note", ColumnKind::Text),
        col("pages", ColumnKind::Integer),
    ])
    .unwrap();
    lib.set_custom(a, "note", "lent to Sam").unwrap();
    lib.set_custom(b, "pages", "12").unwrap();
    let ids = [a, b, c];

    let r = Rule {
        target: Target::Title,
        find: "the ".into(),
        replace: String::new(),
        regex: false,
        case_sensitive: false,
    };
    let p = preview(&lib, &ids, &r, 1).unwrap();
    assert_eq!(
        (p.total, p.problems, p.changes.len()),
        (2, 0, 1),
        "the limit cuts the list, not the count"
    );
    assert_eq!(p.changes[0].before, "The Alpha");
    assert_eq!(p.changes[0].after, "Alpha");
    assert_eq!(
        lib.get(a).unwrap().meta.title,
        "The Alpha",
        "a preview changes nothing"
    );
    // case matters when asked
    let strict = Rule {
        case_sensitive: true,
        ..r.clone()
    };
    assert_eq!(preview(&lib, &ids, &strict, 9).unwrap().total, 0);

    assert_eq!(apply(&mut lib, &ids, &r).unwrap(), (2, 0));
    assert_eq!(lib.get(a).unwrap().meta.title, "Alpha");
    assert_eq!(lib.get(b).unwrap().meta.title, "Beta");
    assert_eq!(lib.get(c).unwrap().meta.title, "Gamma");
    // one undo takes both back
    lib.undo_last().unwrap();
    assert_eq!(lib.get(a).unwrap().meta.title, "The Alpha");
    assert_eq!(lib.get(b).unwrap().meta.title, "The Beta");

    // regular expressions with groups
    let rx = Rule {
        target: Target::Authors,
        find: r"^(\w)$".into(),
        replace: "Author $1".into(),
        regex: true,
        case_sensitive: true,
    };
    assert_eq!(apply(&mut lib, &ids, &rx).unwrap().0, 3);
    assert_eq!(lib.get(a).unwrap().meta.authors, vec!["Author A"]);
    assert_eq!(lib.get(a).unwrap().meta.author_sort, "A, Author");

    // an emptied title is refused and skipped, not applied
    let wipe = Rule {
        target: Target::Title,
        find: ".*".into(),
        replace: String::new(),
        regex: true,
        case_sensitive: false,
    };
    let p = preview(&lib, &ids, &wipe, 9).unwrap();
    assert_eq!((p.total, p.problems), (3, 3));
    assert_eq!(apply(&mut lib, &ids, &wipe).unwrap(), (0, 3));
    assert_eq!(lib.get(a).unwrap().meta.title, "The Alpha");

    // custom columns: the column's own rules decide what is allowed
    let note = Rule {
        target: Target::Custom("note".into()),
        find: "Sam".into(),
        replace: "Alex".into(),
        regex: false,
        case_sensitive: true,
    };
    assert_eq!(apply(&mut lib, &ids, &note).unwrap(), (1, 0));
    assert_eq!(lib.get(a).unwrap().custom_value("note"), "lent to Alex");
    lib.undo_last().unwrap();
    assert_eq!(lib.get(a).unwrap().custom_value("note"), "lent to Sam");
    let bad = Rule {
        target: Target::Custom("pages".into()),
        find: "12".into(),
        replace: "twelve".into(),
        regex: false,
        case_sensitive: true,
    };
    let p = preview(&lib, &ids, &bad, 9).unwrap();
    assert_eq!((p.total, p.problems), (1, 1));
    assert!(
        p.changes[0]
            .problem
            .as_deref()
            .unwrap()
            .contains("whole number")
    );
    assert_eq!(apply(&mut lib, &ids, &bad).unwrap(), (0, 1));
    assert_eq!(lib.get(b).unwrap().custom_value("pages"), "12");
    let _ = std::fs::remove_dir_all(dir);
}
