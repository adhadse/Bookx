#![forbid(unsafe_code)]
//! Devices and servers: USB e-reader driver, OPDS server and client, KOSync-compatible progress
//! server, a tiny content server, and send-by-email (message building and SMTP delivery).
//!
//! Everything here is GTK-free. The protocol logic is pure (`handle(&Request) -> Response`) and the
//! HTTP transport in [`server`] (hyper, optionally TLS) is a thin shell around it.

pub mod annotation_sync;
pub mod cert;
pub mod devfs;
pub mod devices;
pub mod http;
pub mod httpc;
pub mod kobo;
pub mod kosync;
pub mod kosync_client;
pub mod look_repair;
pub mod lua;
pub mod mail;
pub mod mtp;
pub mod net;
pub mod opds;
pub mod opds_client;
pub mod reading;
pub mod server;
pub mod syncplan;
pub mod webdav;
pub mod xml;
