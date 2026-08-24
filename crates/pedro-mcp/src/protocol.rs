//! JSON-RPC 2.0, as much of it as MCP's stdio transport uses.
//!
//! One message per line, requests answered in order, notifications answered
//! not at all. There is no framing beyond the newline, which is the reason
//! nothing but protocol may ever be written to stdout — a stray `println!`
//! is a parse error at the other end.

use serde::Deserialize;
use serde_json::{Value, json};

/// The version of the protocol this speaks, and what a client asking for
/// something unrecognised is answered with.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// The versions a client may ask for and be given back its own.
///
/// Nothing pedro serves differs between them: it offers tools and no more, and
/// tools have worked the same way since the first of these. Echoing the
/// client's own version is therefore honest, and saves a client that only
/// knows an older one from having to fall back.
pub const SUPPORTED_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;

/// A message off the wire.
///
/// Deliberately loose. `id` is whatever JSON the client chose and is echoed
/// back untouched; `params` stays a [`Value`] because every method reads a
/// different shape out of it; and a message carrying no `method` is a reply to
/// something we sent, which is nothing.
#[derive(Debug, Deserialize)]
pub struct Incoming {
    #[serde(default)]
    pub id: Option<Value>,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub params: Value,
    /// Only ever set on a reply, which is how one is told apart from a request
    /// that forgot its method. See [`Incoming::is_reply`].
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<Value>,
}

impl Incoming {
    /// Whether the client wants no answer to this.
    pub fn is_notification(&self) -> bool {
        self.id.is_none()
    }

    /// Whether this is an answer to a request rather than a request itself —
    /// which can only be an answer to something pedro never asked, and is
    /// therefore ignored rather than complained about.
    pub fn is_reply(&self) -> bool {
        self.method.is_none() && (self.result.is_some() || self.error.is_some())
    }
}

/// An answer the client asked for.
pub fn result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// A refusal. `id` is null when the message was too broken to carry one.
pub fn error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message.into() },
    })
}

/// The version to answer `initialize` with: the client's own when it is one
/// this understands, ours otherwise, which invites the client to fall back or
/// hang up.
pub fn negotiate(requested: Option<&str>) -> &'static str {
    requested
        .and_then(|asked| SUPPORTED_VERSIONS.into_iter().find(|known| *known == asked))
        .unwrap_or(PROTOCOL_VERSION)
}
