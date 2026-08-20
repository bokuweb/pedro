//! Turning a CLI's JSONL output into events pedro understands.
//!
//! The two CLIs report their work in different shapes, both recorded from the
//! installed versions rather than guessed:
//!
//! ```text
//! claude -p --output-format stream-json --include-partial-messages --verbose
//!   {"type":"system","subtype":"init","session_id":…}
//!   {"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":…}}}
//!   {"type":"assistant","message":{"content":[{"type":"text","text":…}]}}
//!   {"type":"result","subtype":"success","is_error":false,"result":…}
//!
//! codex exec --json
//!   {"type":"thread.started","thread_id":…}
//!   {"type":"turn.started"}
//!   {"type":"item.completed","item":{"type":"agent_message","text":…}}
//!   {"type":"turn.completed"}
//!   {"type":"turn.failed","error":{"message":…}}
//! ```
//!
//! Both parsers are deliberately forgiving: a line that matches nothing is
//! skipped rather than failed on. These formats move between CLI releases, and
//! a reader must not lose the ability to ask a question because a CLI added a
//! field.

use serde_json::Value;

/// What an agent is doing, as pedro's reader sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentEvent {
    /// The CLI is up. Carries its own session identifier where it has one,
    /// which is worth logging when a run has to be explained after the fact.
    Started { session: Option<String> },
    /// More of the answer. The reader sees these as they arrive.
    Delta(String),
    /// A whole message at once, from a CLI that does not stream deltas.
    Message(String),
    /// The run finished. `text` is the final answer where the CLI states it
    /// outright, which is more trustworthy than the deltas we stitched.
    Finished { text: Option<String> },
    /// The CLI reported a failure of its own — not logged in, out of quota, a
    /// model it cannot reach.
    Failed(String),
}

/// Reads one line of `claude --output-format stream-json`.
pub fn parse_claude_line(line: &str) -> Option<AgentEvent> {
    let value: Value = serde_json::from_str(line).ok()?;

    match value.get("type")?.as_str()? {
        "system" => Some(AgentEvent::Started {
            session: string_at(&value, "session_id"),
        }),

        // Partial messages carry the answer as it is written. Everything else
        // in this stream — tool calls, annotations, thinking — is not the
        // answer, and one of those landing in it is how "[object Object]" ends
        // up in a stored reply.
        "stream_event" => {
            let event = value.get("event")?;
            if event.get("type")?.as_str()? != "content_block_delta" {
                return None;
            }

            let delta = event.get("delta")?;
            if delta.get("type")?.as_str()? != "text_delta" {
                return None;
            }

            Some(AgentEvent::Delta(string_at(delta, "text")?))
        }

        // The complete assistant message. Only useful when partial messages
        // were unavailable, so the runner keeps it as a fallback rather than
        // appending it to what the deltas already said.
        "assistant" => {
            let blocks = value.get("message")?.get("content")?.as_array()?;
            let text: String = blocks
                .iter()
                .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect();

            (!text.is_empty()).then_some(AgentEvent::Message(text))
        }

        // `is_error` rather than `subtype`: "Not logged in" arrives as
        // {"subtype":"success","is_error":true}, and reading the subtype alone
        // stores that sentence as the answer to the reader's question.
        "result" => {
            let text = string_at(&value, "result");
            if value.get("is_error").and_then(Value::as_bool) == Some(true) {
                Some(AgentEvent::Failed(
                    text.unwrap_or_else(|| "the CLI reported an error".to_owned()),
                ))
            } else {
                Some(AgentEvent::Finished { text })
            }
        }

        _ => None,
    }
}

/// Reads one line of `codex exec --json`.
pub fn parse_codex_line(line: &str) -> Option<AgentEvent> {
    let value: Value = serde_json::from_str(line).ok()?;

    // Older releases wrap everything in an envelope: {"id":…,"msg":{"type":…}}.
    // Reading both shapes costs one branch and saves the reader from having to
    // upgrade a CLI that works.
    if let Some(message) = value.get("msg") {
        return parse_codex_legacy(message);
    }

    match value.get("type")?.as_str()? {
        "thread.started" => Some(AgentEvent::Started {
            session: string_at(&value, "thread_id"),
        }),

        "item.completed" => {
            let item = value.get("item")?;
            match item.get("type")?.as_str()? {
                "agent_message" => Some(AgentEvent::Message(string_at(item, "text")?)),
                // Codex reports warnings as error items — a fallback model, a
                // missing cache — and keeps working. A failure that ends the
                // run arrives as turn.failed instead.
                _ => None,
            }
        }

        "turn.completed" => Some(AgentEvent::Finished { text: None }),
        "turn.failed" => Some(AgentEvent::Failed(
            value
                .get("error")
                .and_then(|error| string_at(error, "message"))
                .unwrap_or_else(|| "the turn failed".to_owned()),
        )),
        "error" => Some(AgentEvent::Failed(
            string_at(&value, "message").unwrap_or_else(|| "the CLI reported an error".to_owned()),
        )),

        _ => None,
    }
}

fn parse_codex_legacy(message: &Value) -> Option<AgentEvent> {
    match message.get("type")?.as_str()? {
        "agent_message_delta" => Some(AgentEvent::Delta(string_at(message, "delta")?)),
        "agent_message" => Some(AgentEvent::Message(string_at(message, "message")?)),
        "task_complete" => Some(AgentEvent::Finished { text: None }),
        "error" => Some(AgentEvent::Failed(
            string_at(message, "message").unwrap_or_else(|| "the CLI reported an error".to_owned()),
        )),
        _ => None,
    }
}

fn string_at(value: &Value, key: &str) -> Option<String> {
    Some(value.get(key)?.as_str()?.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    mod claude {
        use super::*;

        #[test]
        fn the_init_line_carries_the_session() {
            let line = r#"{"type":"system","subtype":"init","session_id":"789f7103","tools":[]}"#;
            assert_eq!(
                parse_claude_line(line),
                Some(AgentEvent::Started {
                    session: Some("789f7103".to_owned())
                })
            );
        }

        #[test]
        fn a_text_delta_is_part_of_the_answer() {
            let line = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello"}}}"#;
            assert_eq!(
                parse_claude_line(line),
                Some(AgentEvent::Delta("hello".to_owned()))
            );
        }

        /// The stream also carries deltas for tool input, which are not the
        /// answer and used to end up inside it.
        #[test]
        fn a_delta_that_is_not_text_is_skipped() {
            let line = r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"input_json_delta","partial_json":"{\"a\":1}"}}}"#;
            assert_eq!(parse_claude_line(line), None);
        }

        #[test]
        fn an_assistant_message_joins_its_text_blocks() {
            let line = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hello "},{"type":"thinking","thinking":"…"},{"type":"text","text":"world"}]}}"#;
            assert_eq!(
                parse_claude_line(line),
                Some(AgentEvent::Message("hello world".to_owned()))
            );
        }

        #[test]
        fn a_result_states_the_final_answer() {
            let line = r#"{"type":"result","subtype":"success","is_error":false,"result":"hello world"}"#;
            assert_eq!(
                parse_claude_line(line),
                Some(AgentEvent::Finished {
                    text: Some("hello world".to_owned())
                })
            );
        }

        /// Recorded from the installed CLI without credentials: a successful
        /// subtype carrying a failure.
        #[test]
        fn a_successful_subtype_that_is_an_error_is_a_failure() {
            let line = r#"{"type":"result","subtype":"success","is_error":true,"result":"Not logged in · Please run /login"}"#;
            assert_eq!(
                parse_claude_line(line),
                Some(AgentEvent::Failed(
                    "Not logged in · Please run /login".to_owned()
                ))
            );
        }

        #[test]
        fn a_line_that_is_not_json_is_skipped() {
            assert_eq!(parse_claude_line("Reading additional input..."), None);
        }

        #[test]
        fn an_unknown_line_type_is_skipped() {
            assert_eq!(parse_claude_line(r#"{"type":"user","message":{}}"#), None);
        }
    }

    mod codex {
        use super::*;

        #[test]
        fn a_started_thread_carries_its_id() {
            let line = r#"{"type":"thread.started","thread_id":"01a01f02"}"#;
            assert_eq!(
                parse_codex_line(line),
                Some(AgentEvent::Started {
                    session: Some("01a01f02".to_owned())
                })
            );
        }

        #[test]
        fn a_completed_agent_message_is_the_answer() {
            let line = r#"{"type":"item.completed","item":{"id":"item_1","type":"agent_message","text":"hello world"}}"#;
            assert_eq!(
                parse_codex_line(line),
                Some(AgentEvent::Message("hello world".to_owned()))
            );
        }

        /// Recorded from the installed CLI: a warning about model metadata,
        /// after which the run carried on.
        #[test]
        fn an_error_item_is_a_warning_not_a_failure() {
            let line = r#"{"type":"item.completed","item":{"id":"item_0","type":"error","message":"Model metadata not found."}}"#;
            assert_eq!(parse_codex_line(line), None);
        }

        #[test]
        fn a_failed_turn_reports_why() {
            let line = r#"{"type":"turn.failed","error":{"message":"requires a newer version"}}"#;
            assert_eq!(
                parse_codex_line(line),
                Some(AgentEvent::Failed("requires a newer version".to_owned()))
            );
        }

        #[test]
        fn a_completed_turn_ends_the_run() {
            assert_eq!(
                parse_codex_line(r#"{"type":"turn.completed","usage":{}}"#),
                Some(AgentEvent::Finished { text: None })
            );
        }

        #[test]
        fn a_top_level_error_is_a_failure() {
            let line = r#"{"type":"error","message":"stream disconnected"}"#;
            assert_eq!(
                parse_codex_line(line),
                Some(AgentEvent::Failed("stream disconnected".to_owned()))
            );
        }

        #[test]
        fn the_older_envelope_still_streams() {
            let line = r#"{"id":"0","msg":{"type":"agent_message_delta","delta":"hel"}}"#;
            assert_eq!(
                parse_codex_line(line),
                Some(AgentEvent::Delta("hel".to_owned()))
            );
        }

        #[test]
        fn the_older_envelope_reports_a_whole_message() {
            let line = r#"{"id":"0","msg":{"type":"agent_message","message":"hello"}}"#;
            assert_eq!(
                parse_codex_line(line),
                Some(AgentEvent::Message("hello".to_owned()))
            );
        }

        #[test]
        fn a_line_that_is_not_json_is_skipped() {
            assert_eq!(parse_codex_line("Reading additional input from stdin..."), None);
        }
    }
}
