//! Decoding pages off the main thread. Requests replace each other: when the reader moves on, the
//! pages nobody waits for any more are dropped from the queue (a decode already running finishes,
//! the reader ignores results it no longer wants).

use std::collections::VecDeque;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use crate::decode::{decode_page, Opts, Page};

/// Where the bytes of the pages come from.
pub trait Source: Send + Sync + 'static {
    fn count(&self) -> usize;
    fn bytes(&self, page: usize) -> Option<&[u8]>;
}

/// One page to decode.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Job {
    pub page: usize,
    /// Handed back with the result (the reader uses it to drop results of an older setting).
    pub tag: u64,
    pub opts: Opts,
}

/// A decoded page (or why not).
pub struct Done {
    pub page: usize,
    pub tag: u64,
    pub result: Result<Page, String>,
}

#[derive(Default)]
struct Queue {
    pages: VecDeque<Job>,
    thumbs: VecDeque<Job>,
    stop: bool,
}

struct Shared {
    q: Mutex<Queue>,
    cv: Condvar,
}

pub struct Worker {
    shared: Arc<Shared>,
    rx: Receiver<Done>,
    threads: Vec<JoinHandle<()>>,
}

fn run(shared: Arc<Shared>, source: Arc<dyn Source>, tx: Sender<Done>) {
    loop {
        let job = {
            let mut q = shared.q.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if q.stop {
                    return;
                }
                if let Some(j) = q.pages.pop_front().or_else(|| q.thumbs.pop_front()) {
                    break j;
                }
                q = shared.cv.wait(q).unwrap_or_else(|e| e.into_inner());
            }
        };
        let result = match source.bytes(job.page) {
            Some(b) => decode_page(b, &job.opts).map_err(|e| format!("{e:#}")),
            None => Err("no such page".to_string()),
        };
        if tx
            .send(Done {
                page: job.page,
                tag: job.tag,
                result,
            })
            .is_err()
        {
            return;
        }
    }
}

impl Worker {
    pub fn new(source: Arc<dyn Source>, threads: usize) -> Worker {
        let shared = Arc::new(Shared {
            q: Mutex::new(Queue::default()),
            cv: Condvar::new(),
        });
        let (tx, rx) = channel();
        let threads = (0..threads.max(1))
            .map(|_| {
                let (s, src, tx) = (shared.clone(), source.clone(), tx.clone());
                std::thread::spawn(move || run(s, src, tx))
            })
            .collect();
        Worker {
            shared,
            rx,
            threads,
        }
    }

    /// The pages to decode now, most important first. Replaces the pages still waiting.
    pub fn request(&self, jobs: Vec<Job>) {
        let mut q = self.shared.q.lock().unwrap_or_else(|e| e.into_inner());
        q.pages = jobs.into();
        drop(q);
        self.shared.cv.notify_all();
    }

    /// Thumbnails: done when no page waits. Replaces the thumbnails still waiting.
    pub fn request_thumbs(&self, jobs: Vec<Job>) {
        let mut q = self.shared.q.lock().unwrap_or_else(|e| e.into_inner());
        q.thumbs = jobs.into();
        drop(q);
        self.shared.cv.notify_all();
    }

    pub fn try_recv(&self) -> Option<Done> {
        self.rx.try_recv().ok()
    }

    /// Waits for a result (tests).
    pub fn recv_timeout(&self, ms: u64) -> Option<Done> {
        self.rx
            .recv_timeout(std::time::Duration::from_millis(ms))
            .ok()
    }

    /// Jobs waiting (not yet started).
    pub fn waiting(&self) -> usize {
        let q = self.shared.q.lock().unwrap_or_else(|e| e.into_inner());
        q.pages.len() + q.thumbs.len()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        {
            let mut q = self.shared.q.lock().unwrap_or_else(|e| e.into_inner());
            q.stop = true;
            q.pages.clear();
            q.thumbs.clear();
        }
        self.shared.cv.notify_all();
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testgen::{png, Layout};

    struct Pages(Vec<Vec<u8>>);
    impl Source for Pages {
        fn count(&self) -> usize {
            self.0.len()
        }
        fn bytes(&self, i: usize) -> Option<&[u8]> {
            self.0.get(i).map(|v| v.as_slice())
        }
    }

    fn pages(n: usize) -> Arc<Pages> {
        Arc::new(Pages(
            (0..n)
                .map(|_| png(120, 180, Layout::Grid(2, 2), 255))
                .collect(),
        ))
    }

    #[test]
    fn decodes_requested_pages_and_hands_back_the_tag() {
        let w = Worker::new(pages(4), 2);
        let o = Opts::default();
        w.request(vec![
            Job {
                page: 2,
                tag: 7,
                opts: o,
            },
            Job {
                page: 0,
                tag: 7,
                opts: o,
            },
        ]);
        let mut got = Vec::new();
        for _ in 0..2 {
            let d = w.recv_timeout(5000).expect("a result");
            assert_eq!(d.tag, 7);
            assert_eq!(d.result.as_ref().unwrap().panels.len(), 4);
            got.push(d.page);
        }
        got.sort();
        assert_eq!(got, vec![0, 2]);
    }

    #[test]
    fn errors_come_back_as_results() {
        let w = Worker::new(pages(1), 1);
        w.request(vec![Job {
            page: 9,
            tag: 1,
            opts: Opts::default(),
        }]);
        let d = w.recv_timeout(5000).unwrap();
        assert!(d.result.is_err());
    }

    #[test]
    fn a_new_request_drops_the_old_waiting_ones() {
        let w = Worker::new(pages(50), 1);
        let o = Opts::default();
        w.request(
            (0..50)
                .map(|p| Job {
                    page: p,
                    tag: 1,
                    opts: o,
                })
                .collect(),
        );
        w.request(vec![Job {
            page: 49,
            tag: 2,
            opts: o,
        }]);
        assert!(w.waiting() <= 1);
        let mut seen = 0;
        let mut last_tag = 0;
        while let Some(d) = w.recv_timeout(800) {
            seen += 1;
            last_tag = d.tag;
        }
        assert!(seen < 50, "most of the first request never ran: {seen}");
        assert_eq!(last_tag, 2);
    }

    #[test]
    fn pages_go_before_thumbnails_and_drop_stops_the_threads() {
        let w = Worker::new(pages(6), 1);
        let o = Opts::default();
        let t = Opts {
            max_edge: 40,
            panels: None,
            ..o
        };
        w.request_thumbs(
            (0..5)
                .map(|p| Job {
                    page: p,
                    tag: 9,
                    opts: t,
                })
                .collect(),
        );
        w.request(vec![Job {
            page: 5,
            tag: 1,
            opts: o,
        }]);
        let mut n = 0;
        while let Some(d) = w.recv_timeout(2000) {
            n += 1;
            if d.tag == 9 {
                assert!(d.result.unwrap().width <= 40);
            }
            if n == 6 {
                break;
            }
        }
        assert_eq!(n, 6);
        drop(w); // joins the threads: the test would hang if they did not stop
    }
}
