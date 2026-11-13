//! The Streamable HTTP transport (one endpoint, `POST /mcp`), with a bearer token.
//!
//! Every request is answered with a single JSON body (no event streams), so progress and log
//! notifications are not delivered over HTTP. Sessions (`Mcp-Session-Id`) are optional: a client
//! that sends none is served statelessly. The listener is the hardened one of `bookx-serve`
//! (size limits, timeouts, connection cap, throttling of wrong tokens, optional TLS).

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use bookx_serve::http::{constant_time_eq, Request, Response};
use bookx_serve::net::{self, Limits};
use bookx_serve::server::TlsConfig;
use serde_json::{json, Value};

use crate::app::lock;
use crate::server::{
    error_message, RpcError, Server, Session, INVALID_REQUEST, PARSE_ERROR, SUPPORTED_VERSIONS,
};

#[derive(Clone)]
pub struct HttpConfig {
    pub bind: String,
    /// `0` picks a free port.
    pub port: u16,
    /// The bearer token every request must carry.
    pub token: String,
    pub tls: Option<Arc<TlsConfig>>,
    /// Origins besides loopback ones that may call (browser based clients).
    pub allowed_origins: Vec<String>,
}

impl HttpConfig {
    pub fn new(token: &str) -> HttpConfig {
        HttpConfig {
            bind: "127.0.0.1".into(),
            port: 0,
            token: token.into(),
            tls: None,
            allowed_origins: Vec::new(),
        }
    }
}

const MAX_SESSIONS: usize = 64;

type Shared = Arc<Mutex<Session>>;

struct Sessions {
    list: Mutex<VecDeque<(String, Shared)>>,
}

impl Sessions {
    fn get(&self, id: &str) -> Option<Shared> {
        lock(&self.list)
            .iter()
            .find(|(i, _)| i == id)
            .map(|(_, s)| s.clone())
    }

    fn add(&self, s: Session) -> Shared {
        let id = s.id.clone();
        let shared = Arc::new(Mutex::new(s));
        let mut l = lock(&self.list);
        l.push_back((id, shared.clone()));
        shared
    }

    /// Adds, returning the id of a session pushed out by the cap.
    fn add_capped(&self, s: Session) -> (Shared, Option<String>) {
        let shared = self.add(s);
        let mut l = lock(&self.list);
        let evicted = if l.len() > MAX_SESSIONS {
            l.pop_front().map(|(i, _)| i)
        } else {
            None
        };
        (shared, evicted)
    }

    fn remove(&self, id: &str) -> bool {
        let mut l = lock(&self.list);
        let before = l.len();
        l.retain(|(i, _)| i != id);
        l.len() != before
    }
}

/// The request handler, separate from the listener so tests can call it directly.
pub struct Endpoint {
    pub server: Arc<Server>,
    cfg: HttpConfig,
    sessions: Sessions,
}

fn json_response(status: u16, v: &Value) -> Response {
    Response::json(status, v).with_header("Cache-Control", "no-store")
}

fn http_error(status: u16, code: i64, message: &str) -> Response {
    json_response(
        status,
        &error_message(&Value::Null, &RpcError::new(code, message)),
    )
}

/// Whether an `Origin` header names this machine (or an origin the user allowed).
pub fn origin_allowed(origin: &str, extra: &[String]) -> bool {
    if extra.iter().any(|o| o == origin) {
        return true;
    }
    let host = origin
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(origin)
        .split('/')
        .next()
        .unwrap_or("");
    let host = if let Some(v6) = host.strip_prefix('[') {
        v6.split(']').next().unwrap_or("")
    } else {
        host.split(':').next().unwrap_or("")
    };
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

impl Endpoint {
    pub fn new(server: Arc<Server>, cfg: HttpConfig) -> Arc<Endpoint> {
        Arc::new(Endpoint {
            server,
            cfg,
            sessions: Sessions {
                list: Mutex::new(VecDeque::new()),
            },
        })
    }

    fn authorized(&self, req: &Request) -> bool {
        req.header("authorization")
            .and_then(|h| {
                h.strip_prefix("Bearer ")
                    .or_else(|| h.strip_prefix("bearer "))
            })
            .is_some_and(|t| {
                !self.cfg.token.is_empty()
                    && constant_time_eq(t.trim().as_bytes(), self.cfg.token.as_bytes())
            })
    }

    pub fn handle(&self, req: &Request) -> Response {
        if let Some(origin) = req.header("origin") {
            if !origin_allowed(origin, &self.cfg.allowed_origins) {
                return http_error(403, INVALID_REQUEST, "origin not allowed");
            }
        }
        if !self.authorized(req) {
            return Response::json(
                401,
                &json!({"error":"unauthorized","message":"send the token as `Authorization: Bearer <token>`"}),
            )
            .with_header("WWW-Authenticate", "Bearer realm=\"bookx-mcp\"");
        }
        if req.path != "/mcp" && req.path != "/mcp/" && req.path != "/" {
            return Response::text(404, "not found");
        }
        if let Some(v) = req.header("mcp-protocol-version") {
            if !SUPPORTED_VERSIONS.contains(&v) {
                return http_error(
                    400,
                    INVALID_REQUEST,
                    &format!("unsupported protocol version {v}"),
                );
            }
        }
        match req.method.as_str() {
            "POST" => self.post(req),
            "DELETE" => {
                let Some(id) = req.header("mcp-session-id") else {
                    return Response::empty(400);
                };
                if self.sessions.remove(id) {
                    self.server.app.client_gone(id);
                    Response::empty(200)
                } else {
                    Response::empty(404)
                }
            }
            _ => Response::text(
                405,
                "this server answers POST (and DELETE to end a session)",
            )
            .with_header("Allow", "POST, DELETE"),
        }
    }

    fn post(&self, req: &Request) -> Response {
        let msg: Value = match serde_json::from_slice(&req.body) {
            Ok(v) => v,
            Err(e) => return http_error(400, PARSE_ERROR, &format!("parse error: {e}")),
        };
        if !msg.is_object() {
            return http_error(
                400,
                INVALID_REQUEST,
                "send one JSON-RPC message per request (batches are not supported)",
            );
        }
        let sid_header = req.header("mcp-session-id").map(str::to_string);
        let is_init = msg.get("method").and_then(Value::as_str) == Some("initialize");
        let (shared, new_id) = match (&sid_header, is_init) {
            (Some(id), false) => match self.sessions.get(id) {
                Some(s) => (s, None),
                None => {
                    return http_error(404, INVALID_REQUEST, "unknown session; initialize again")
                }
            },
            (_, true) => {
                let mut s = Session::new(true);
                s.initialized = false;
                let id = s.id.clone();
                let (shared, evicted) = self.sessions.add_capped(s);
                if let Some(old) = evicted {
                    self.server.app.client_gone(&old);
                }
                (shared, Some(id))
            }
            (None, false) => {
                let mut s = Session::new(true);
                s.initialized = true;
                s.client_name = "http client".into();
                (Arc::new(Mutex::new(s)), None)
            }
        };
        let sid = lock(&shared).id.clone();
        if self.server.try_cancel(&sid, &msg) {
            return Response::empty(202);
        }
        let is_request = msg.get("method").is_some() && msg.get("id").is_some();
        let sink = |_: Value| {};
        let answer = {
            let mut s = lock(&shared);
            self.server.handle(&mut s, msg, &sink)
        };
        if sid_header.is_none() && new_id.is_none() {
            self.server.app.client_gone(&sid);
        }
        match answer {
            Some(v) => {
                let mut r = json_response(200, &v);
                if let Some(id) = new_id {
                    r = r.with_header("Mcp-Session-Id", &id);
                }
                r
            }
            None if is_request => Response::empty(204),
            None => Response::empty(202),
        }
    }
}

pub struct HttpHandle {
    addr: SocketAddr,
    tls: bool,
    running: net::Running,
    pub endpoint: Arc<Endpoint>,
}

impl HttpHandle {
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// `http://127.0.0.1:PORT/mcp` (`https` with TLS).
    pub fn url(&self) -> String {
        format!(
            "{}://{}/mcp",
            if self.tls { "https" } else { "http" },
            self.addr
        )
    }

    /// Clients with a live session (for a status display).
    pub fn endpoint_clients(&self) -> Vec<crate::app::ClientStat> {
        self.endpoint.server.app.clients(None)
    }

    pub fn shutdown(mut self) {
        self.running.stop();
    }
}

pub fn start(server: Arc<Server>, cfg: HttpConfig) -> Result<HttpHandle> {
    anyhow::ensure!(
        cfg.token.len() >= 16,
        "the token must be at least 16 characters"
    );
    let limits = Limits {
        max_body: 1024 * 1024,
        ..Limits::default()
    };
    let (bind, port, tls) = (cfg.bind.clone(), cfg.port, cfg.tls.clone());
    let endpoint = Endpoint::new(server, cfg);
    let ep = endpoint.clone();
    let handler: net::Handler = Arc::new(move |req| ep.handle(req));
    let running = net::Running::start_tls(&bind, port, limits, handler, tls.clone())?;
    Ok(HttpHandle {
        addr: running.addr(),
        tls: tls.is_some(),
        running,
        endpoint,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins() {
        assert!(origin_allowed("http://localhost:3000", &[]));
        assert!(origin_allowed("http://127.0.0.1", &[]));
        assert!(origin_allowed("http://[::1]:8080", &[]));
        assert!(!origin_allowed("https://evil.example", &[]));
        assert!(!origin_allowed("http://localhost.evil.example", &[]));
        assert!(origin_allowed(
            "https://app.example",
            &["https://app.example".into()]
        ));
    }
}
