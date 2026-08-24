//! pedro's library, served to a coding agent over MCP.
//!
//! Registered with a client as a command to run, and talked to down its own
//! stdin and stdout:
//!
//! ```bash
//! claude mcp add pedro -- pedro-mcp
//! ```
//!
//! Nothing may be printed to stdout but protocol, so the log goes to stderr —
//! where a client that captures it will show it, and where a client that does
//! not will drop it harmlessly.

use pedro_core::store::Store;

fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()))
        .init();

    // Opening the library is the one thing that can fail before a client is
    // listening, and a client shows a server that exited rather than one that
    // reported an error, so it is said on stderr too.
    let store = match Store::open_default() {
        Ok(store) => store,
        Err(err) => {
            eprintln!("pedro-mcp: the library could not be opened: {err}");
            std::process::exit(1);
        }
    };

    tracing::info!(library = %store.root().display(), "serving");

    // Books added before an index existed are not indexed here: a client gives
    // a server it has just started a matter of seconds to answer `initialize`,
    // and embedding a shelf of books takes longer than that. The reader does it
    // at startup, and `cargo run -p pedro-core --example reindex` does it now.
    serve(store)
}

fn serve(store: Store) -> std::io::Result<()> {
    let input = std::io::stdin().lock();
    let output = std::io::stdout().lock();

    pedro_mcp::serve(store, input, output)
}
