//! A thread that owns the open document and does the slow work (rendering, text extraction,
//! searching) so that the UI thread never waits for it.
//!
//! The UI sends requests and polls [`Worker::try_recv`] for events. Renders are served newest
//! first and a request that was cancelled before it started is dropped; a search is cancelled by
//! starting another one. A page that makes the renderer panic is reported as failed and the
//! worker carries on.

use std::collections::{HashMap, VecDeque};
use std::ops::Range;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread::JoinHandle;

use crate::doc::{Document, Info, Link, OpenError, OutlineItem};
use crate::render::Bitmap;
use crate::text::PageText;

/// Everything cheap to know once the document is open.
#[derive(Debug, Clone)]
pub struct Summary {
    pub page_sizes: Vec<(f32, f32)>,
    pub labels: Vec<String>,
    pub outline: Vec<OutlineItem>,
    pub info: Info,
}

/// One match of a search on a page.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    /// Characters of the page text.
    pub range: Range<usize>,
    /// Text around the match for the result list and the range of the match in it.
    pub snippet: String,
    pub mark: Range<usize>,
}

/// Lets the sender of a request withdraw it before it starts.
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn new() -> Cancel {
        Cancel::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

#[derive(Debug)]
pub enum Event {
    Opened(Result<Summary, OpenError>),
    Rendered {
        page: usize,
        scale: f32,
        invert: bool,
        bitmap: Arc<Bitmap>,
    },
    RenderFailed {
        page: usize,
    },
    Page {
        page: usize,
        text: Arc<PageText>,
        links: Arc<Vec<Link>>,
    },
    SearchHits {
        id: u64,
        page: usize,
        hits: Vec<Hit>,
    },
    SearchDone {
        id: u64,
    },
    Crop {
        page: usize,
        bbox: Option<[f32; 4]>,
    },
}

enum Req {
    Render {
        page: usize,
        scale: f32,
        invert: bool,
        cancel: Cancel,
    },
    Page(usize),
    Search {
        id: u64,
        query: String,
    },
    Crop(usize),
    Quit,
}

pub struct Worker {
    tx: Sender<Req>,
    rx: Receiver<Event>,
    handle: Option<JoinHandle<()>>,
}

impl Worker {
    /// Opens `data` on a new thread. The first event is [`Event::Opened`].
    pub fn open(data: Vec<u8>, password: Option<String>) -> Worker {
        let (tx, req_rx) = channel();
        let (ev_tx, rx) = channel();
        let handle = std::thread::Builder::new()
            .name("bookx-pdf".into())
            .spawn(move || run(data, password, req_rx, ev_tx))
            .ok();
        Worker { tx, rx, handle }
    }

    pub fn try_recv(&self) -> Option<Event> {
        self.rx.try_recv().ok()
    }

    /// Waits for the next event (tests, importers).
    pub fn recv_timeout(&self, d: std::time::Duration) -> Option<Event> {
        self.rx.recv_timeout(d).ok()
    }

    pub fn render(&self, page: usize, scale: f32, invert: bool) -> Cancel {
        let cancel = Cancel::new();
        let _ = self.tx.send(Req::Render {
            page,
            scale,
            invert,
            cancel: cancel.clone(),
        });
        cancel
    }

    /// The text and links of a page.
    pub fn page(&self, page: usize) {
        let _ = self.tx.send(Req::Page(page));
    }

    pub fn search(&self, id: u64, query: &str) {
        let _ = self.tx.send(Req::Search {
            id,
            query: query.to_string(),
        });
    }

    pub fn crop(&self, page: usize) {
        let _ = self.tx.send(Req::Crop(page));
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.tx.send(Req::Quit);
        // not joined: a render in progress should not freeze closing the book
        drop(self.handle.take());
    }
}

struct Job {
    page: usize,
    scale: f32,
    invert: bool,
    cancel: Cancel,
}

struct SearchState {
    id: u64,
    query: String,
    next: usize,
}

fn run(data: Vec<u8>, password: Option<String>, rx: Receiver<Req>, tx: Sender<Event>) {
    let opened = catch_unwind(AssertUnwindSafe(|| {
        Document::open(data, password.as_deref())
    }))
    .unwrap_or(Err(OpenError::Invalid));
    let doc = match opened {
        Ok(d) => d,
        Err(e) => {
            let _ = tx.send(Event::Opened(Err(e)));
            return;
        }
    };
    let summary = catch_unwind(AssertUnwindSafe(|| Summary {
        page_sizes: doc.page_sizes(),
        labels: doc.page_labels(),
        outline: doc.outline(),
        info: doc.info(),
    }));
    let Ok(summary) = summary else {
        let _ = tx.send(Event::Opened(Err(OpenError::Invalid)));
        return;
    };
    let n = summary.page_sizes.len();
    if tx.send(Event::Opened(Ok(summary))).is_err() {
        return;
    }
    let renderer = doc.renderer();
    let mut texts: HashMap<usize, Arc<PageText>> = HashMap::new();
    let mut text_of = |page: usize| -> Arc<PageText> {
        if let Some(t) = texts.get(&page) {
            return t.clone();
        }
        let t = Arc::new(catch_unwind(AssertUnwindSafe(|| doc.text(page))).unwrap_or_default());
        if texts.len() >= 256 {
            texts.clear();
        }
        texts.insert(page, t.clone());
        t
    };
    let mut renders: Vec<Job> = Vec::new();
    let mut pages: VecDeque<usize> = VecDeque::new();
    let mut crops: VecDeque<usize> = VecDeque::new();
    let mut search: Option<SearchState> = None;

    loop {
        let idle = renders.is_empty() && pages.is_empty() && crops.is_empty() && search.is_none();
        let first = if idle {
            match rx.recv() {
                Ok(r) => Some(r),
                Err(_) => return,
            }
        } else {
            None
        };
        let mut incoming: Vec<Req> = first.into_iter().collect();
        loop {
            match rx.try_recv() {
                Ok(r) => incoming.push(r),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        for r in incoming {
            match r {
                Req::Quit => return,
                Req::Render {
                    page,
                    scale,
                    invert,
                    cancel,
                } => {
                    if page < n && !cancel.is_cancelled() {
                        renders.retain(|j| {
                            !(j.page == page && j.scale == scale && j.invert == invert)
                        });
                        renders.push(Job {
                            page,
                            scale,
                            invert,
                            cancel,
                        });
                    }
                }
                Req::Page(p) => {
                    if p < n && !pages.contains(&p) {
                        pages.push_back(p);
                    }
                }
                Req::Crop(p) => {
                    if p < n && !crops.contains(&p) {
                        crops.push_back(p);
                    }
                }
                Req::Search { id, query } => {
                    search = Some(SearchState { id, query, next: 0 });
                }
            }
        }

        if let Some(p) = pages.pop_front() {
            let text = text_of(p);
            let links =
                Arc::new(catch_unwind(AssertUnwindSafe(|| doc.links(p))).unwrap_or_default());
            if tx
                .send(Event::Page {
                    page: p,
                    text,
                    links,
                })
                .is_err()
            {
                return;
            }
            continue;
        }
        renders.retain(|j| !j.cancel.is_cancelled());
        if let Some(job) = renders.pop() {
            let out = catch_unwind(AssertUnwindSafe(|| {
                renderer.render(job.page, job.scale, job.invert)
            }));
            let ev = match out {
                Ok(Some(b)) => Event::Rendered {
                    page: job.page,
                    scale: job.scale,
                    invert: job.invert,
                    bitmap: Arc::new(b),
                },
                _ => Event::RenderFailed { page: job.page },
            };
            if tx.send(ev).is_err() {
                return;
            }
            continue;
        }
        if let Some(p) = crops.pop_front() {
            let bbox = catch_unwind(AssertUnwindSafe(|| renderer.content_box(p)))
                .ok()
                .flatten();
            if tx.send(Event::Crop { page: p, bbox }).is_err() {
                return;
            }
            continue;
        }
        if let Some(mut s) = search.take() {
            if s.next >= n {
                if tx.send(Event::SearchDone { id: s.id }).is_err() {
                    return;
                }
                continue;
            }
            let t = text_of(s.next);
            let hits: Vec<Hit> = t
                .find(&s.query)
                .into_iter()
                .map(|range| {
                    let (snippet, mark) = t.snippet(range.clone(), 28);
                    Hit {
                        range,
                        snippet,
                        mark,
                    }
                })
                .collect();
            let page = s.next;
            s.next += 1;
            let id = s.id;
            search = Some(s);
            if !hits.is_empty() && tx.send(Event::SearchHits { id, page, hits }).is_err() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testpdf::{Page, TestPdf};
    use std::time::Duration;

    const T: Duration = Duration::from_secs(20);

    fn wait_for<F: FnMut(&Event) -> bool>(w: &Worker, mut f: F) -> Event {
        loop {
            let e = w.recv_timeout(T).expect("timed out waiting for the worker");
            if f(&e) {
                return e;
            }
        }
    }

    fn sample() -> Worker {
        Worker::open(
            TestPdf::simple(&[
                &["alpha beta"],
                &["gamma delta"],
                &["alpha again", "and Alpha"],
            ])
            .title("W")
            .build(),
            None,
        )
    }

    #[test]
    fn opening_reports_the_summary() {
        let w = sample();
        let Event::Opened(Ok(s)) = w.recv_timeout(T).unwrap() else {
            panic!("not opened")
        };
        assert_eq!(s.page_sizes.len(), 3);
        assert_eq!(s.labels, ["1", "2", "3"]);
        assert_eq!(s.info.title, "W");
    }

    #[test]
    fn opening_a_bad_file_reports_the_error() {
        let w = Worker::open(b"garbage".to_vec(), None);
        assert!(matches!(
            w.recv_timeout(T).unwrap(),
            Event::Opened(Err(OpenError::Invalid))
        ));
        let enc = Worker::open(TestPdf::simple(&[&["x"]]).password("p").build(), None);
        assert!(matches!(
            enc.recv_timeout(T).unwrap(),
            Event::Opened(Err(OpenError::NeedsPassword))
        ));
    }

    #[test]
    fn a_render_comes_back_with_its_key() {
        let w = sample();
        wait_for(&w, |e| matches!(e, Event::Opened(_)));
        let _c = w.render(1, 0.25, false);
        let Event::Rendered {
            page,
            scale,
            bitmap,
            ..
        } = wait_for(&w, |e| matches!(e, Event::Rendered { .. }))
        else {
            unreachable!()
        };
        assert_eq!((page, scale), (1, 0.25));
        assert_eq!((bitmap.width, bitmap.height), (153, 198));
    }

    #[test]
    fn cancelled_renders_never_run() {
        let w = sample();
        wait_for(&w, |e| matches!(e, Event::Opened(_)));
        // first keep the worker busy with a text request so that the renders queue up
        w.page(0);
        let a = w.render(0, 1.0, false);
        let b = w.render(1, 1.0, false);
        a.cancel();
        b.cancel();
        let _keep = w.render(2, 0.25, false);
        let mut rendered = vec![];
        loop {
            if let Event::Rendered { page, .. } = w.recv_timeout(T).unwrap() {
                rendered.push(page);
                if page == 2 {
                    break;
                }
            }
        }
        assert!(
            !rendered.contains(&0) || !rendered.contains(&1),
            "{rendered:?}"
        );
        assert!(rendered.contains(&2));
    }

    #[test]
    fn a_page_request_gives_text_and_links() {
        let page = Page::new(612.0, 792.0)
            .text(72.0, 700.0, 12.0, "linked")
            .link([72.0, 690.0, 120.0, 710.0], crate::testpdf::Target::Page(0));
        let w = Worker::open(TestPdf::new().page(page).build(), None);
        wait_for(&w, |e| matches!(e, Event::Opened(_)));
        w.page(0);
        let Event::Page { text, links, .. } = wait_for(&w, |e| matches!(e, Event::Page { .. }))
        else {
            unreachable!()
        };
        assert_eq!(text.text(), "linked\n");
        assert_eq!(links.len(), 1);
    }

    #[test]
    fn search_streams_hits_page_by_page_and_ends() {
        let w = sample();
        wait_for(&w, |e| matches!(e, Event::Opened(_)));
        w.search(7, "alpha");
        let mut hits: Vec<(usize, usize)> = vec![];
        loop {
            match w.recv_timeout(T).unwrap() {
                Event::SearchHits { id, page, hits: h } => {
                    assert_eq!(id, 7);
                    assert!(h[0].snippet.to_lowercase().contains("alpha"));
                    hits.push((page, h.len()));
                }
                Event::SearchDone { id } => {
                    assert_eq!(id, 7);
                    break;
                }
                _ => {}
            }
        }
        assert_eq!(hits, vec![(0, 1), (2, 2)]);
    }

    #[test]
    fn a_new_search_replaces_the_running_one() {
        let w = sample();
        wait_for(&w, |e| matches!(e, Event::Opened(_)));
        w.search(1, "alpha");
        w.search(2, "delta");
        // the worker may finish search 1 before it sees search 2; either way the last search
        // finishes, and nothing of search 1 arrives after anything of search 2
        let mut seen = vec![];
        loop {
            match w.recv_timeout(T).unwrap() {
                Event::SearchHits { id, .. } => seen.push(id),
                Event::SearchDone { id } => {
                    seen.push(id);
                    if id == 2 {
                        break;
                    }
                }
                _ => {}
            }
        }
        let first2 = seen.iter().position(|i| *i == 2).unwrap();
        assert!(seen[first2..].iter().all(|i| *i == 2), "{seen:?}");
    }

    #[test]
    fn crop_reports_the_content_box() {
        let page = Page::new(200.0, 200.0).rect([50.0, 50.0, 150.0, 150.0], 0.0);
        let w = Worker::open(TestPdf::new().page(page).build(), None);
        wait_for(&w, |e| matches!(e, Event::Opened(_)));
        w.crop(0);
        let Event::Crop { bbox, .. } = wait_for(&w, |e| matches!(e, Event::Crop { .. })) else {
            unreachable!()
        };
        let b = bbox.unwrap();
        assert!(
            (b[0] - 0.25).abs() < 0.05 && (b[2] - 0.75).abs() < 0.05,
            "{b:?}"
        );
    }

    #[test]
    fn requests_after_an_error_do_not_hang_or_panic() {
        let w = Worker::open(b"x".to_vec(), None);
        let _ = w.recv_timeout(T);
        w.page(0);
        let _ = w.render(0, 1.0, false);
        w.search(1, "a");
        assert!(w.try_recv().is_none());
    }
}
