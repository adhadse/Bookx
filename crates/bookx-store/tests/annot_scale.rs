//! How the catalogue behaves with a lot of highlights, and what a table of their own would change (docs/design-audit.md).
//! Not a pass/fail budget: run it by hand and read the numbers.
//!   cargo test --release -p bookx-store --test annot_scale -- --ignored --nocapture
mod common;
use bookx_core::library::{Bookmark, Highlight};
use bookx_store::open_library;
use chrono::{TimeZone, Utc};
use common::book;
use std::time::Instant;

fn rss_mb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find(|l| l.starts_with("VmRSS"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0)
        / 1024
}

fn make(n: u64, hl: usize, bm: usize) -> Vec<bookx_core::library::Book> {
    (1..=n).map(|id| {
        let mut b = book(id, &format!("Title {id}"), &format!("Author {}", id % 500));
        b.highlights = (0..hl).map(|i| Highlight {
            id: format!("h{id}-{i}"), spine: i % 20, start: (i * 37) as i32, end: (i * 37 + 120) as i32,
            color: ["yellow","green","blue","pink","purple"][i % 5].into(), style: "background".into(),
            note: if i % 5 < 2 { format!("my thoughts about passage {i} of book {id}, a sentence or two long.") } else { String::new() },
            text: format!("This is the highlighted passage number {i} from book {id}; it is about one hundred and twenty characters long, give or take."),
            created: Utc.timestamp_opt(1_700_000_000 + i as i64 * 60, 0).unwrap(),
        }).collect();
        b.bookmarks = (0..bm).map(|i| Bookmark { id: format!("b{id}-{i}"), spine: i, frac: 0.1 * i as f64, label: format!("Bookmark {i}"), created: Utc.timestamp_opt(1_700_000_000, 0).unwrap() }).collect();
        b
    }).collect()
}

fn run(n: u64, hl: usize) {
    let dir = std::env::temp_dir().join(format!("annot-bench-{n}-{hl}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let total = n as usize * hl;
    println!("\n=== {n} books x {hl} highlights = {total} highlights ===");
    {
        let mut o = open_library(&dir).unwrap();
        o.library.books = make(n, hl, 5);
        let t = Instant::now();
        o.library.save().unwrap();
        println!("first save: {:?}", t.elapsed());
    }
    let db = dir.join("bookx-library.db");
    println!(
        "db size: {} MB",
        std::fs::metadata(&db).unwrap().len() / 1_048_576
    );
    let rss0 = rss_mb();
    let t = Instant::now();
    let mut o = open_library(&dir).unwrap();
    println!(
        "reopen (load all books): {:?}   RSS +{} MB",
        t.elapsed(),
        rss_mb().saturating_sub(rss0)
    );
    // one new highlight in one book
    let i = (n / 2) as usize;
    o.library.books[i].highlights.push(Highlight {
        id: "new".into(),
        spine: 0,
        start: 1,
        end: 9,
        color: "yellow".into(),
        style: "background".into(),
        note: "x".into(),
        text: "new passage".into(),
        created: Utc::now(),
    });
    let t = Instant::now();
    o.library.save().unwrap();
    println!("save after adding ONE highlight: {:?}", t.elapsed());
    // aggregate notes view: all highlights with a note containing a word, in memory
    let t = Instant::now();
    let c: usize = o
        .library
        .books
        .iter()
        .flat_map(|b| b.highlights.iter())
        .filter(|h| h.note.contains("passage 7 "))
        .count();
    println!(
        "in-memory scan of all highlights ({c} hits): {:?}",
        t.elapsed()
    );
    // the same question in SQL over the JSON column
    let conn = rusqlite::Connection::open(&db).unwrap();
    let t = Instant::now();
    let c2: i64 = conn.query_row("SELECT count(*) FROM books b, json_each(b.data, '$.highlights') h WHERE json_extract(h.value,'$.note') LIKE '%passage 7 %'", [], |r| r.get(0)).unwrap();
    println!(
        "SQL json_each over the JSON column ({c2} hits): {:?}",
        t.elapsed()
    );
    // normalized table: what a real table would cost
    conn.execute_batch("CREATE TABLE hl(id TEXT PRIMARY KEY, book INTEGER, spine INT, start INT, end_ INT, color TEXT, style TEXT, note TEXT, text TEXT, created INT); CREATE INDEX hl_book ON hl(book); CREATE INDEX hl_color ON hl(color);").unwrap();
    let t = Instant::now();
    {
        let tx = conn.unchecked_transaction().unwrap();
        {
            let mut st = tx
                .prepare("INSERT INTO hl VALUES(?,?,?,?,?,?,?,?,?,?)")
                .unwrap();
            for b in &o.library.books {
                for h in &b.highlights {
                    st.execute(rusqlite::params![
                        h.id,
                        b.id as i64,
                        h.spine as i64,
                        h.start,
                        h.end,
                        h.color,
                        h.style,
                        h.note,
                        h.text,
                        h.created.timestamp()
                    ])
                    .unwrap();
                }
            }
        }
        tx.commit().unwrap();
    }
    println!(
        "(normalized) insert all {total} highlights: {:?}",
        t.elapsed()
    );
    let t = Instant::now();
    let c3: i64 = conn
        .query_row(
            "SELECT count(*) FROM hl WHERE note LIKE '%passage 7 %'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    println!(
        "(normalized) LIKE over the note column ({c3} hits): {:?}",
        t.elapsed()
    );
    let t = Instant::now();
    let _: i64 = conn
        .query_row(
            "SELECT count(*) FROM hl WHERE book = ?",
            [n as i64 / 2],
            |r| r.get(0),
        )
        .unwrap();
    println!(
        "(normalized) all highlights of one book (indexed): {:?}",
        t.elapsed()
    );
    let t = Instant::now();
    conn.execute("UPDATE hl SET note = 'edited' WHERE id = ?", ["h5-3"])
        .unwrap();
    println!("(normalized) edit ONE highlight: {:?}", t.elapsed());
    println!(
        "db size with the extra table: {} MB",
        std::fs::metadata(&db).unwrap().len() / 1_048_576
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "a measurement, run by hand"]
fn bench() {
    run(2_000, 40);
    run(10_000, 60);
}
