//! rsPostman core: a local-first, Postman-compatible API client engine.
#![allow(clippy::large_enum_variant, clippy::unnecessary_sort_by, clippy::redundant_closure_call, clippy::manual_strip)]

pub mod auth;
pub mod bench;
pub mod bench_report;
pub mod codegen;
pub mod cookies;
pub mod docs;
pub mod curl;
pub mod echo;
pub mod server;
pub mod engine;
pub mod http;
pub mod import;
pub mod import_ext;
pub mod mock;
pub mod model;
pub mod profile;
pub mod report;
pub mod runner;
pub mod script;
pub mod secrets;
pub mod store;
pub mod vars;
pub mod vault;
pub mod ws;

pub use engine::{Exchange, Session};
pub use model::{Auth, Body, Collection, Environment, Item, Request, Url, KV};
pub use store::Workspace;
