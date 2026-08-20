//! Running an agent CLI and reading its answer as it is written.
//!
//! The CLI is invoked once per question, with the whole conversation, and its
//! JSONL output is turned into [`AgentEvent`]s (see [`crate::events`]). Tools
//! are off: pedro is asking about a book, not asking for work to be done in a
//! repository, and a coding agent left with its tools will happily go reading
//! the filesystem to answer.

use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::conversation::{Prompt, render_conversation};
use crate::discovery::{AgentKind, DiscoveredAgent};
use crate::events::{AgentEvent, parse_claude_line, parse_codex_line};

/// How often the watcher looks at whether the reader asked to stop.
const CANCEL_POLL: Duration = Duration::from_millis(100);

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("could not start {program}: {source}")]
    Spawn {
        program: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// The CLI itself said what went wrong — not logged in, no quota, a model
    /// it cannot reach. This is the message worth putting in front of the
    /// reader, because it usually names something they can fix.
    #[error("{0}")]
    Refused(String),

    #[error("{program} exited with {status}{}", format_stderr(.stderr))]
    Exited {
        program: PathBuf,
        status: String,
        stderr: String,
    },

    #[error("failed to read from the agent: {0}")]
    Read(#[source] std::io::Error),

    #[error("the question was cancelled")]
    Cancelled,

    /// The CLI ran, said nothing, and reported no error.
    #[error("the agent answered with nothing")]
    NoAnswer,
}

fn format_stderr(stderr: &str) -> String {
    match stderr.trim() {
        "" => String::new(),
        message => format!(": {message}"),
    }
}

/// A shared "stop this" flag.
///
/// Cloned into whatever holds the stop button; the run notices between lines,
/// and a watcher kills the CLI if it has gone quiet.
#[derive(Debug, Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Asks `agent` the question in `prompt`, calling `on_event` as the answer
/// arrives, and returns the finished answer.
///
/// Blocking: the caller is expected to be off the UI thread.
pub fn run(
    agent: &DiscoveredAgent,
    prompt: &Prompt,
    cancellation: &Cancellation,
    on_event: &mut dyn FnMut(AgentEvent),
) -> Result<String, AgentError> {
    let mut child = spawn(agent, prompt)?;

    let stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");

    // Read stderr on its own thread: a CLI that fills the pipe while we are
    // reading stdout would otherwise block forever waiting for us to drain it.
    let stderr_reader = thread::spawn(move || {
        let mut buffer = String::new();
        let mut stderr = stderr;
        let _ = stderr.read_to_string(&mut buffer);
        buffer
    });

    // The child is shared with the watcher only until stdout ends. It is taken
    // out before waiting, so the wait never holds the lock the watcher needs.
    let shared = Arc::new(Mutex::new(Some(child)));
    let done = Arc::new(AtomicBool::new(false));
    let watcher = watch_for_cancellation(shared.clone(), cancellation.clone(), done.clone());

    let parse = match agent.kind {
        AgentKind::ClaudeCode => parse_claude_line,
        AgentKind::Codex => parse_codex_line,
    };

    let mut answer = String::new();
    let mut refusal = None;

    for line in BufReader::new(stdout).lines() {
        let line = line.map_err(AgentError::Read)?;
        let Some(event) = parse(&line) else {
            tracing::trace!(line, "skipped an unrecognised line");
            continue;
        };

        match &event {
            AgentEvent::Delta(text) => answer.push_str(text),
            // A whole message only counts when nothing was streamed: a CLI that
            // sends both would otherwise say everything twice.
            AgentEvent::Message(text) if answer.is_empty() => answer = text.clone(),
            // The CLI's own final text, which beats the deltas we stitched.
            AgentEvent::Finished { text: Some(text) } => answer = text.clone(),
            AgentEvent::Failed(reason) => refusal = Some(reason.clone()),
            _ => {}
        }

        on_event(event);
    }

    done.store(true, Ordering::SeqCst);
    let status = shared
        .lock()
        .expect("the watcher never panics while holding the lock")
        .take()
        .expect("the child is taken exactly once")
        .wait()
        .map_err(AgentError::Read)?;
    let stderr = stderr_reader.join().unwrap_or_default();
    let _ = watcher.join();

    if cancellation.is_cancelled() {
        return Err(AgentError::Cancelled);
    }
    if let Some(reason) = refusal {
        return Err(AgentError::Refused(reason));
    }
    if !status.success() {
        return Err(AgentError::Exited {
            program: agent.program.clone(),
            status: status.to_string(),
            stderr,
        });
    }
    if answer.trim().is_empty() {
        return Err(AgentError::NoAnswer);
    }

    Ok(answer)
}

fn spawn(agent: &DiscoveredAgent, prompt: &Prompt) -> Result<Child, AgentError> {
    let mut command = command_for(agent, prompt);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    own_process_group(&mut command);

    command.spawn().map_err(|source| AgentError::Spawn {
        program: agent.program.clone(),
        source,
    })
}

/// The command line for one question.
///
/// Both CLIs are asked for JSONL and for no tools beyond web search. The
/// working directory is pedro's own, not wherever pedro was launched from:
/// both CLIs read project instructions out of the directory they run in, and a
/// question about a book should not inherit a repository's conventions.
fn command_for(agent: &DiscoveredAgent, prompt: &Prompt) -> Command {
    let mut command = Command::new(&agent.program);

    if let Some(workspace) = &prompt.workspace {
        command.current_dir(workspace);
    }

    match agent.kind {
        AgentKind::ClaudeCode => {
            command.args([
                "-p",
                "--output-format",
                "stream-json",
                // Without this the answer arrives in one piece at the end.
                "--include-partial-messages",
                // stream-json output is refused in print mode without it.
                "--verbose",
                // Nothing here is worth resuming, and a question about a book
                // should not turn up in the reader's `claude --resume` list.
                "--no-session-persistence",
            ]);
            command.args(["--system-prompt", &prompt.system]);
            command.args(["--tools", if prompt.web_search { "WebSearch" } else { "" }]);
            // An MCP server configured for coding would otherwise be started
            // for every question, and its tools offered to answer it.
            command.args(["--strict-mcp-config", "--mcp-config", r#"{"mcpServers":{}}"#]);
            command.arg("--");
            command.arg(render_conversation(&prompt.turns));
        }

        AgentKind::Codex => {
            command.args([
                "exec",
                "--json",
                // pedro's data directory is not a repository, and codex refuses
                // to run outside one without this.
                "--skip-git-repo-check",
                "-s",
                "read-only",
            ]);
            if prompt.web_search {
                command.args(["-c", "tools.web_search=true"]);
            }
            command.arg("--");
            // Codex has no flag for a system prompt, so the standing
            // instructions lead the text instead.
            command.arg(format!(
                "{}\n\n{}",
                prompt.system,
                render_conversation(&prompt.turns)
            ));
        }
    }

    command
}

/// Puts the CLI in a process group of its own, so that cancelling can end the
/// whole tree rather than one process of it.
///
/// The installed `claude` is frequently a shell script around a node process,
/// and `codex` starts children of its own. Killing only the process we spawned
/// leaves those holding the pipe we are reading, so a cancelled question keeps
/// the reader waiting for a run that is already over.
#[cfg(unix)]
fn own_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt as _;
    command.process_group(0);
}

#[cfg(not(unix))]
fn own_process_group(_command: &mut Command) {}

/// Ends the CLI and everything it started.
fn kill(child: &mut Child) {
    #[cfg(unix)]
    // Safe: the group id is the child's own pid, which `own_process_group`
    // made a group leader, and signalling a group that has already exited is
    // an error rather than undefined behaviour.
    unsafe {
        libc::killpg(child.id() as i32, libc::SIGKILL);
    }

    let _ = child.kill();
}

fn watch_for_cancellation(
    child: Arc<Mutex<Option<Child>>>,
    cancellation: Cancellation,
    done: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        while !done.load(Ordering::SeqCst) {
            if cancellation.is_cancelled() {
                if let Some(child) = child.lock().expect("no panics hold this lock").as_mut() {
                    kill(child);
                }
                return;
            }
            thread::sleep(CANCEL_POLL);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::Turn;

    fn prompt() -> Prompt {
        Prompt {
            system: "instructions".to_owned(),
            turns: vec![Turn::user("これは?")],
            web_search: false,
            workspace: None,
        }
    }

    fn arguments(kind: AgentKind, prompt: &Prompt) -> Vec<String> {
        let agent = DiscoveredAgent {
            kind,
            program: PathBuf::from("/usr/bin/false"),
            version: None,
        };

        command_for(&agent, prompt)
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn claude_is_asked_for_a_stream_with_no_tools() {
        let arguments = arguments(AgentKind::ClaudeCode, &prompt());

        assert!(arguments.windows(2).any(|pair| pair == ["--tools", ""]));
        assert!(arguments.contains(&"stream-json".to_owned()));
        assert!(arguments.contains(&"--include-partial-messages".to_owned()));
    }

    #[test]
    fn web_search_is_the_only_tool_claude_is_given() {
        let mut asking = prompt();
        asking.web_search = true;

        let arguments = arguments(AgentKind::ClaudeCode, &asking);
        assert!(
            arguments
                .windows(2)
                .any(|pair| pair == ["--tools", "WebSearch"])
        );
    }

    #[test]
    fn claudes_system_prompt_is_a_flag_rather_than_part_of_the_question() {
        let arguments = arguments(AgentKind::ClaudeCode, &prompt());

        assert!(
            arguments
                .windows(2)
                .any(|pair| pair == ["--system-prompt", "instructions"])
        );
        assert_eq!(arguments.last().unwrap(), "これは?");
    }

    /// Codex has no system prompt flag, so the instructions have to lead the
    /// text — and the question still has to be in there.
    #[test]
    fn codex_is_given_the_instructions_and_the_question_together() {
        let arguments = arguments(AgentKind::Codex, &prompt());
        let text = arguments.last().unwrap();

        assert!(text.starts_with("instructions"));
        assert!(text.ends_with("これは?"));
    }

    #[test]
    fn codex_is_told_to_search_only_when_search_is_on() {
        let mut asking = prompt();
        asking.web_search = true;

        assert!(
            arguments(AgentKind::Codex, &asking)
                .windows(2)
                .any(|pair| pair == ["-c", "tools.web_search=true"])
        );
        assert!(
            !arguments(AgentKind::Codex, &prompt())
                .iter()
                .any(|argument| argument.contains("web_search"))
        );
    }

    /// A question the reader wrote can start with a dash, which would otherwise
    /// be read as a flag.
    #[test]
    fn the_question_is_separated_from_the_flags() {
        for kind in AgentKind::ALL {
            let arguments = arguments(kind, &prompt());
            let separator = arguments
                .iter()
                .position(|argument| argument == "--")
                .expect("a separator");

            assert_eq!(separator, arguments.len() - 2, "{kind:?}");
        }
    }

    #[test]
    fn a_cancellation_starts_uncancelled_and_stays_cancelled() {
        let cancellation = Cancellation::new();
        assert!(!cancellation.is_cancelled());

        let clone = cancellation.clone();
        clone.cancel();
        assert!(cancellation.is_cancelled());
    }
}
