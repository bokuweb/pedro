//! Talking to the coding agent CLIs already installed on this machine.
//!
//! Pedro drives an agent the reader has installed and authenticated
//! (`claude`, `codex`, ...) instead of asking for an API key. This crate
//! answers both questions that requires: *what is available here?*
//! ([`discover`]) and *what does it say?* ([`run`]).
//!
//! ```no_run
//! for agent in pedro_agent::discover() {
//!     println!("{} at {}", agent.kind.display_name(), agent.program.display());
//! }
//! ```

mod conversation;
mod discovery;
mod events;
pub mod fixtures;
mod process;
mod run;
mod shell_env;

pub use conversation::{Prompt, Role, Turn, render_conversation};
pub use discovery::{AgentKind, DiscoveredAgent, discover, probe_version};
pub use events::{AgentEvent, parse_claude_line, parse_codex_line};
pub use run::{AgentError, Cancellation, run};
