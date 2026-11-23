//! The stdio transport: one JSON-RPC message per line on the input, answers one per line on the
//! output. Standard output carries nothing else.
//!
//! Reading happens on its own thread so a `notifications/cancelled` is seen while a tool is still
//! running; requests themselves are answered one at a time, in order.

use std::io::{BufRead, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::Value;

use crate::app::lock;
use crate::server::{
    INVALID_REQUEST, PARSE_ERROR, RpcError, Server, Session, changes, error_message, snapshot,
};

/// Longest accepted input line.
pub const MAX_LINE: usize = 4 * 1024 * 1024;

/// Waits up to `total`, in steps of at most 50 ms, and returns early once `done` is set.
fn static_wait(done: &AtomicBool, total: Duration) {
    let end = Instant::now() + total;
    while !done.load(Ordering::Relaxed) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(50).min(total));
    }
}

fn send<W: Write>(w: &Mutex<W>, v: &Value) {
    let mut w = lock(w);
    let _ = writeln!(w, "{v}");
    let _ = w.flush();
}

pub fn serve<R, W>(server: Arc<Server>, reader: R, writer: W) -> Result<()>
where
    R: BufRead + Send + 'static,
    W: Write + Send + 'static,
{
    let writer = Arc::new(Mutex::new(writer));
    let mut session = Session::new(false);
    let sid = session.id.clone();
    let (tx, rx) = mpsc::channel::<Value>();

    let reader_thread = {
        let (server, writer) = (server.clone(), writer.clone());
        std::thread::spawn(move || {
            let mut reader = reader;
            loop {
                let mut line = Vec::new();
                let n = match (&mut reader)
                    .take(MAX_LINE as u64 + 1)
                    .read_until(b'\n', &mut line)
                {
                    Ok(n) => n,
                    Err(_) => break,
                };
                if n == 0 {
                    break;
                }
                if line.len() > MAX_LINE {
                    send(
                        &writer,
                        &error_message(
                            &Value::Null,
                            &RpcError::new(INVALID_REQUEST, "message too large"),
                        ),
                    );
                    break;
                }
                let text = String::from_utf8_lossy(&line);
                let text = text.trim();
                if text.is_empty() {
                    continue;
                }
                let msg: Value = match serde_json::from_str(text) {
                    Ok(v) => v,
                    Err(e) => {
                        send(
                            &writer,
                            &error_message(
                                &Value::Null,
                                &RpcError::new(PARSE_ERROR, format!("parse error: {e}")),
                            ),
                        );
                        continue;
                    }
                };
                if server.try_cancel(&sid, &msg) {
                    continue;
                }
                if msg.get("method").is_some() {
                    if let Some(id) = msg.get("id") {
                        server.register_request(&sid, id);
                    }
                }
                if tx.send(msg).is_err() {
                    break;
                }
            }
        })
    };

    // tell the client about library changes (list_changed, updated for subscribed resources)
    let done = Arc::new(AtomicBool::new(false));
    let watcher = {
        let interval = server.app.cfg.watch_interval;
        (!interval.is_zero()).then(|| {
            let (server, writer, done) = (server.clone(), writer.clone(), done.clone());
            let subs = session.subscriptions.clone();
            std::thread::spawn(move || {
                let books = |s: &Server| s.app.catalog.books().map(|b| snapshot(&b)).ok();
                let mut last = books(&server);
                while !done.load(Ordering::Relaxed) {
                    // short steps so shutdown is quick; the library is looked at once per interval
                    static_wait(&done, interval);
                    if done.load(Ordering::Relaxed) {
                        break;
                    }
                    let now = books(&server);
                    if let (Some(old), Some(new)) = (&last, &now) {
                        let subs = lock(&subs).clone();
                        for n in changes(old, new, &subs) {
                            send(&writer, &n);
                        }
                    }
                    if now.is_some() {
                        last = now;
                    }
                }
            })
        })
    };

    let notify_writer = writer.clone();
    let notify = move |v: Value| send(&notify_writer, &v);
    for msg in rx {
        if let Some(answer) = server.handle(&mut session, msg, &notify) {
            send(&writer, &answer);
        }
    }
    let _ = reader_thread.join();
    done.store(true, Ordering::Relaxed);
    if let Some(w) = watcher {
        let _ = w.join();
    }
    server.app.client_gone(&session.id);
    Ok(())
}
