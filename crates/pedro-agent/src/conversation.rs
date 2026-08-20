//! What a question looks like on its way to an agent.
//!
//! The conversation is pedro's, not the CLI's: every request carries the whole
//! exchange and no CLI session is resumed. It costs a little more per turn than
//! resuming would, and in exchange the history is rows in pedro's database
//! rather than state inside someone else's process — and the same code drives
//! every CLI, including the ones that have no resume of their own.

use serde::{Deserialize, Serialize};

/// Who said a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

/// One turn of the conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub role: Role,
    pub content: String,
}

impl Turn {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

/// One request to an agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    /// Standing instructions. Passed as the CLI's own system prompt where there
    /// is a flag for it, and folded into the text where there is not.
    pub system: String,
    /// The conversation, ending with the question being asked now.
    pub turns: Vec<Turn>,
    /// Whether the agent may search the web. This is a capability of the CLI
    /// rather than a request parameter, so it becomes a flag rather than a line
    /// of the prompt.
    pub web_search: bool,
    /// The directory the CLI is run in. Both CLIs read project instructions out
    /// of their working directory, so this is pedro's own directory rather than
    /// wherever pedro happened to be launched from.
    pub workspace: Option<std::path::PathBuf>,
}

/// The conversation as one block of text.
///
/// A single question is sent as itself — labelling one line "Reader" would only
/// give the model something to imitate. Anything longer is labelled, because
/// without labels a past answer and the new question run together into one
/// wall of text with no indication of who said what.
pub fn render_conversation(turns: &[Turn]) -> String {
    if let [only] = turns
        && only.role == Role::User
    {
        return only.content.clone();
    }

    turns
        .iter()
        .map(|turn| {
            let speaker = match turn.role {
                Role::User => "Reader",
                Role::Assistant => "Assistant",
            };
            format!("## {speaker}\n{}", turn.content)
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_question_is_sent_as_itself() {
        assert_eq!(
            render_conversation(&[Turn::user("これはどういう意味?")]),
            "これはどういう意味?"
        );
    }

    #[test]
    fn a_longer_exchange_says_who_said_what() {
        let rendered = render_conversation(&[
            Turn::user("これは?"),
            Turn::assistant("エッジで動きます。"),
            Turn::user("では冷スタートは?"),
        ]);

        assert_eq!(
            rendered,
            "## Reader\nこれは?\n\n## Assistant\nエッジで動きます。\n\n## Reader\nでは冷スタートは?"
        );
    }

    /// A single *assistant* turn is not the shortcut case: it is not a question
    /// at all, and sending it bare would read as words the reader wrote.
    #[test]
    fn a_lone_assistant_turn_is_still_labelled() {
        assert_eq!(
            render_conversation(&[Turn::assistant("答え")]),
            "## Assistant\n答え"
        );
    }
}
