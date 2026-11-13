#![forbid(unsafe_code)]
//! A Model Context Protocol server for the Bookx library, so AI assistants can search, read and
//! (when the user allows it) annotate the user's books. GTK free.
//!
//! * [`app`]: configuration, catalogue access, audit log, rate limit.
//! * [`server`]: the protocol (JSON-RPC, tools, resources, prompts, pagination, cancellation).
//! * [`stdio`] and [`http`]: the two transports (Streamable HTTP with a bearer token).
//! * [`tools`], [`tools_write`], [`resources`], [`prompts`]: what the server offers.
//!
//! The protocol is implemented here on `serde_json` instead of an async SDK: the workspace is
//! blocking code and the subset needed is small (see `docs/mcp-notes.md`).

pub mod app;
pub mod http;
pub mod prompts;
pub mod resources;
pub mod schema;
pub mod server;
pub mod stdio;
pub mod text;
pub mod tools;
pub mod tools_write;

pub use app::{App, Config, WritePolicy};
pub use server::Server;

use std::sync::Arc;

/// A server over `cfg`.
pub fn new_server(cfg: Config) -> Arc<Server> {
    Arc::new(Server::new(App::new(cfg)))
}

/// A fresh random token for the HTTP transport: 64 hex characters from two random UUIDs.
pub fn generate_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}
