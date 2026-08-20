//! Driving a whole run, with a stand-in for the CLI.
//!
//! The unit tests cover the two JSONL dialects line by line. These cover what
//! surrounds them: stitching deltas into an answer, preferring the CLI's own
//! final text, telling a refusal apart from a crash, and stopping a run that is
//! still going.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use pedro_agent::fixtures::fake_cli;
use pedro_agent::{AgentError, AgentEvent, AgentKind, Cancellation, DiscoveredAgent, Prompt, Turn};

fn agent(kind: AgentKind, program: PathBuf) -> DiscoveredAgent {
    DiscoveredAgent {
        kind,
        program,
        version: None,
    }
}

fn prompt() -> Prompt {
    Prompt {
        system: "instructions".to_owned(),
        turns: vec![Turn::user("これは?")],
        web_search: false,
        workspace: None,
    }
}

/// Runs the fake and returns the answer alongside every event it reported.
fn ask(agent: &DiscoveredAgent) -> (Result<String, AgentError>, Vec<AgentEvent>) {
    let mut events = Vec::new();
    let answer = pedro_agent::run(agent, &prompt(), &Cancellation::new(), &mut |event| {
        events.push(event)
    });

    (answer, events)
}

#[test]
fn deltas_are_stitched_into_one_answer() {
    let cli = fake_cli(
        "claude-deltas",
        r#"
echo '{"type":"system","subtype":"init","session_id":"s1"}'
echo '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"hello "}}}'
echo '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"world"}}}'
echo '{"type":"result","subtype":"success","is_error":false,"result":"hello world"}'
"#,
    );

    let (answer, events) = ask(&agent(AgentKind::ClaudeCode, cli));

    assert_eq!(answer.unwrap(), "hello world");
    assert_eq!(
        events,
        vec![
            AgentEvent::Started {
                session: Some("s1".to_owned())
            },
            AgentEvent::Delta("hello ".to_owned()),
            AgentEvent::Delta("world".to_owned()),
            AgentEvent::Finished {
                text: Some("hello world".to_owned())
            },
        ]
    );
}

/// A CLI that streams *and* repeats itself at the end must not say everything
/// twice.
#[test]
fn a_whole_message_does_not_repeat_what_was_streamed() {
    let cli = fake_cli(
        "claude-both",
        r#"
echo '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"hello world"}}}'
echo '{"type":"assistant","message":{"content":[{"type":"text","text":"hello world"}]}}'
echo '{"type":"result","subtype":"success","is_error":false,"result":"hello world"}'
"#,
    );

    assert_eq!(ask(&agent(AgentKind::ClaudeCode, cli)).0.unwrap(), "hello world");
}

/// An older CLI without partial messages still answers.
#[test]
fn a_cli_that_never_streams_still_answers() {
    let cli = fake_cli(
        "claude-whole",
        r#"
echo '{"type":"assistant","message":{"content":[{"type":"text","text":"hello world"}]}}'
echo '{"type":"result","subtype":"success","is_error":false,"result":"hello world"}'
"#,
    );

    assert_eq!(ask(&agent(AgentKind::ClaudeCode, cli)).0.unwrap(), "hello world");
}

#[test]
fn codex_reports_its_answer_as_one_completed_item() {
    let cli = fake_cli(
        "codex-message",
        r#"
echo '{"type":"thread.started","thread_id":"t1"}'
echo '{"type":"turn.started"}'
echo '{"type":"item.completed","item":{"id":"item_1","type":"agent_message","text":"hello world"}}'
echo '{"type":"turn.completed"}'
"#,
    );

    assert_eq!(ask(&agent(AgentKind::Codex, cli)).0.unwrap(), "hello world");
}

/// The shape recorded from a CLI that is not logged in: an error reported in
/// the stream, and a non-zero exit. The message is what the reader can act on,
/// so it is the one that survives.
#[test]
fn a_refusal_is_reported_with_the_reason_the_cli_gave() {
    let cli = fake_cli(
        "claude-refusal",
        r#"
echo '{"type":"result","subtype":"success","is_error":true,"result":"Not logged in · Please run /login"}'
exit 1
"#,
    );

    let error = ask(&agent(AgentKind::ClaudeCode, cli)).0.unwrap_err();
    assert!(
        matches!(&error, AgentError::Refused(reason) if reason.contains("Not logged in")),
        "{error}"
    );
}

#[test]
fn a_crash_reports_the_status_and_what_it_printed() {
    let cli = fake_cli(
        "claude-crash",
        r#"
echo 'command not found: node' >&2
exit 127
"#,
    );

    let error = ask(&agent(AgentKind::ClaudeCode, cli)).0.unwrap_err();
    assert!(
        matches!(&error, AgentError::Exited { stderr, .. } if stderr.contains("command not found")),
        "{error}"
    );
}

#[test]
fn a_run_that_says_nothing_is_not_an_empty_answer() {
    let cli = fake_cli("claude-silent", "echo '{\"type\":\"system\",\"subtype\":\"init\"}'");

    let error = ask(&agent(AgentKind::ClaudeCode, cli)).0.unwrap_err();
    assert!(matches!(error, AgentError::NoAnswer), "{error}");
}

#[test]
fn a_missing_cli_is_reported_as_one() {
    let missing = agent(AgentKind::ClaudeCode, PathBuf::from("/nonexistent/pedro/claude"));

    let error = ask(&missing).0.unwrap_err();
    assert!(matches!(error, AgentError::Spawn { .. }), "{error}");
}

/// The reader can stop an answer that is still being written, and the CLI does
/// not outlive it.
#[test]
fn cancelling_stops_a_run_that_is_still_going() {
    let cli = fake_cli(
        "claude-slow",
        r#"
echo '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"thinking"}}}'
sleep 30
echo '{"type":"result","subtype":"success","is_error":false,"result":"too late"}'
"#,
    );

    let cancellation = Cancellation::new();
    let seen = Arc::new(Mutex::new(Vec::new()));

    let error = pedro_agent::run(
        &agent(AgentKind::ClaudeCode, cli),
        &prompt(),
        &cancellation,
        &mut |event| {
            seen.lock().expect("no poisoning").push(event);
            cancellation.cancel();
        },
    )
    .unwrap_err();

    assert!(matches!(error, AgentError::Cancelled), "{error}");
    assert_eq!(seen.lock().expect("no poisoning").len(), 1);
}
