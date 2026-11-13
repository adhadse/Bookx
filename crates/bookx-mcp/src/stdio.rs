//! The stdio transport: one JSON-RPC message per line on the input, answers one per line on the
//! output. Standard output carries nothing else.
//!
//! Reading happens on its own thread so a `notifications/cancelled` is seen while a tool is still
//! running; requests themselves are answered one at a time, in order.

use std::io::{BufRead, Read, Write};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use serde_json::Value;

use crate::app::lock;
use crate::server::{error_message, RpcError, Server, Session, INVALID_REQUEST, PARSE_ERROR};

/// Longest accepted input line.
pub const MAX_LINE: usize = 4 * 1024 * 1024;

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

    let notify_writer = writer.clone();
    let notify = move |v: Value| send(&notify_writer, &v);
    for msg in rx {
        if let Some(answer) = server.handle(&mut session, msg, &notify) {
            send(&writer, &answer);
        }
    }
    let _ = reader_thread.join();
    server.app.client_gone(&session.id);
    Ok(())
}
