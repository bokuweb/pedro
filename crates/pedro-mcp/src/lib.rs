//! The library pedro has read, offered to other agents as a local MCP server.
//!
//! pedro borrows the credentials of a coding agent CLI to answer questions
//! about a book. This is the same relationship the other way round: the agent
//! keeps its own reasoning, and pedro lends it the thing the agent has no way
//! to get at otherwise — a hybrid index over documents somebody chose to read,
//! with page numbers that mean something because the reader can turn to them.
//!
//! It speaks MCP over stdio, which is why it is a binary of its own rather than
//! something inside `pedro-app`: a client starts it, talks to it down a pipe,
//! and stops it. It reads the same library the reader does, so a book added in
//! one is there in the other; SQLite is in WAL mode, which is what makes both
//! open at once safe.
//!
//! ```no_run
//! let store = pedro_core::store::Store::open_default()?;
//! pedro_mcp::serve(store, std::io::stdin().lock(), std::io::stdout().lock())?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod protocol;
pub mod server;
pub mod tools;

pub use protocol::PROTOCOL_VERSION;
pub use server::{Session, serve};
