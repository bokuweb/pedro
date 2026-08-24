//! The loop: a line in, a line out, the library in between.
//!
//! Sequential on purpose. MCP allows a client to have several calls in flight,
//! but the store is one SQLite connection and the work behind these tools is
//! measured in milliseconds — indexing a book being the exception, and that one
//! the caller is waiting on anyway. Answering in order costs nothing here and
//! removes every question about what two calls at once would do to the library.

use std::io::{BufRead, Write};

use pedro_core::store::Store;
use serde_json::{Value, json};

use crate::protocol::{
    INVALID_REQUEST, Incoming, METHOD_NOT_FOUND, PARSE_ERROR, error, negotiate, result,
};
use crate::tools;

/// What the client is told pedro is, once, at the start.
///
/// The tools describe themselves; this describes the shape of the thing they
/// reach into, which is the part a caller cannot guess: a library of documents
/// somebody chose to read, not the filesystem.
const INSTRUCTIONS: &str = "\
pedro is a reader on this machine, and this is the library it has read: PDFs \
the user added themselves, cut into passages and indexed by their words and \
by their meaning. Use it when a question is about one of those documents — a \
specification, a paper, a manual — rather than about the code at hand. Start \
with list_books to see what is there, or go straight to search_library and \
read what it names. Page numbers are the ones printed in the reader, so an \
answer can send the user to the page it came from.";

/// One client's connection, and the library it is reading.
pub struct Session {
    store: Store,
    /// What the client called itself, kept for the log line that says who
    /// connected — the only way to tell two agent CLIs apart in a log.
    client: Option<String>,
}

impl Session {
    pub fn new(store: Store) -> Self {
        Self {
            store,
            client: None,
        }
    }

    /// The name the client gave at `initialize`, once it has.
    pub fn client(&self) -> Option<&str> {
        self.client.as_deref()
    }

    /// Answers one message, or does not: a notification is a message the
    /// client has asked to hear nothing back about, and answering one anyway
    /// is a protocol violation rather than mere noise.
    pub fn handle(&mut self, line: &str) -> Option<Value> {
        if line.trim().is_empty() {
            return None;
        }

        let message: Incoming = match serde_json::from_str(line) {
            Ok(message) => message,
            Err(err) => {
                tracing::warn!(%err, "a message could not be parsed");
                return Some(error(Value::Null, PARSE_ERROR, err.to_string()));
            }
        };

        let Some(method) = message.method.clone() else {
            // A reply to something pedro never sent. Nothing to say about it.
            if message.is_reply() {
                return None;
            }

            // A request with no method, on the other hand, has to be answered:
            // it carries an id, and a client that gets nothing back for an id
            // waits for it until it gives up.
            tracing::warn!("a request named no method");
            return message
                .id
                .map(|id| error(id, INVALID_REQUEST, "a request must name a method"));
        };
        let answer = self.dispatch(&method, &message.params);

        match message.is_notification() {
            true => None,
            false => Some(match answer {
                Ok(value) => result(message.id.unwrap_or(Value::Null), value),
                Err((code, why)) => error(message.id.unwrap_or(Value::Null), code, why),
            }),
        }
    }

    fn dispatch(&mut self, method: &str, params: &Value) -> Result<Value, (i64, String)> {
        match method {
            "initialize" => Ok(self.initialize(params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools::catalogue() })),
            "tools/call" => tools::call(&mut self.store, params),

            // Notifications, which reach here only so that an unknown one is
            // distinguishable in the log from an unknown request.
            _ if method.starts_with("notifications/") => Ok(Value::Null),

            // A client that asked for prompts or resources gets told they are
            // not here, which is what the capabilities it was handed already
            // said.
            other => Err((METHOD_NOT_FOUND, format!("pedro has no method {other}"))),
        }
    }

    fn initialize(&mut self, params: &Value) -> Value {
        self.client = params
            .get("clientInfo")
            .and_then(|info| info.get("name"))
            .and_then(Value::as_str)
            .map(str::to_owned);

        let version = negotiate(params.get("protocolVersion").and_then(Value::as_str));
        tracing::info!(client = self.client, version, "a client connected");

        json!({
            "protocolVersion": version,
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "pedro", "version": env!("CARGO_PKG_VERSION") },
            "instructions": INSTRUCTIONS,
        })
    }
}

/// Reads messages until the client hangs up.
///
/// A line that is not valid JSON is answered and the loop goes on; a client
/// closing its end is how this is meant to finish, and is not an error.
pub fn serve(store: Store, input: impl BufRead, mut output: impl Write) -> std::io::Result<()> {
    let mut session = Session::new(store);

    for line in input.lines() {
        let line = match line {
            Ok(line) => line,
            // A message that is not UTF-8 cannot be answered, because the id to
            // answer it with is inside it.
            Err(err) if err.kind() == std::io::ErrorKind::InvalidData => {
                tracing::warn!(%err, "a message was not valid UTF-8 and was dropped");
                continue;
            }
            Err(err) => return Err(err),
        };

        let Some(answer) = session.handle(&line) else {
            continue;
        };

        // One message per line, and flushed: the client is a process waiting
        // on a pipe, and a buffered answer is a hang.
        writeln!(output, "{answer}")?;
        output.flush()?;
    }

    tracing::info!(client = session.client(), "the client hung up");
    Ok(())
}
