//! What the agent is told, and what the conversation looks like to it.
//!
//! A port of chatbook's `buildSystemPrompt` and `buildConversation`. The
//! wording is kept as it is there: the citation rules at the end of the prompt
//! are what [`crate::citation::parse_citations`] reads back, so the two only
//! work as a pair.

pub use pedro_agent::{Role, Turn};

use crate::citation::strip_sources;
use crate::excerpt::Excerpt;

/// The conversation as the agent is given it: the earlier turns, then the new
/// question.
///
/// Past answers are sent without their `## Sources` section: it quotes the
/// passages in full, and resending it every turn pays for the same text again
/// even though the citations are already stored alongside the answer. A
/// reader's own words are left whole — they can paste an answer back to ask
/// about it, and a `## Sources` line of *theirs* is part of the question.
pub fn build_conversation(history: &[Turn], question: &str) -> Vec<Turn> {
    history
        .iter()
        .map(|turn| match turn.role {
            Role::Assistant => Turn::assistant(strip_sources(&turn.content)),
            Role::User => turn.clone(),
        })
        .chain(std::iter::once(Turn::user(question)))
        .collect()
}

/// Builds the system prompt for a question about a highlighted passage.
///
/// The document block carries an excerpt rather than the whole book. All the
/// excerpt-awareness lives in the wording *around* the DOCUMENT markers —
/// nothing is ever injected between them, so the model's quotes stay verbatim
/// substrings of the stored full text and the citation page lookup keeps
/// finding them. A whole-book excerpt produces exactly the wording this prompt
/// always had: a one-page book is never told it is looking at a fragment.
pub fn build_system_prompt(excerpt: &Excerpt, selected_text: &str, use_web_search: bool) -> String {
    let Excerpt {
        text,
        start_page,
        end_page,
        total_pages,
        is_partial,
    } = excerpt;

    let context_name = if *is_partial {
        format!("excerpt (pages {start_page}-{end_page} of the {total_pages}-page document)")
    } else {
        "document".to_owned()
    };

    // "the shown pages do" / "the document does": the subject and its verb
    // travel together so the two variants stay grammatical in every slot.
    let scope_does = if *is_partial {
        "the shown pages do"
    } else {
        "the document does"
    };

    let missing_answer_instruction = if *is_partial {
        format!(
            "- You are shown only pages {start_page}-{end_page}; the rest of the document is not \
             visible to you. When the shown pages do not contain the answer, say it is not in the \
             shown pages rather than not in the document, then provide what you know."
        )
    } else {
        "- When the document does not contain the answer, say so clearly, then provide what you \
         know."
            .to_owned()
    };

    let excerpt_or_document = if *is_partial { "excerpt" } else { "document" };
    let web_search_instruction = if use_web_search {
        format!(
            "\n\nWhen {scope_does} not contain enough information to answer the question, you may \
             use web search to find additional context. Always indicate when you are using \
             external sources."
        )
    } else {
        format!(
            "\n\nRespond using only the {excerpt_or_document} context. If {scope_does} not \
             contain the answer, say so clearly."
        )
    };

    // chatbook also asks for mermaid diagrams here. That instruction is left
    // out until the reader can render one: a diagram the reader is shown as raw
    // code is worse than the prose it replaced.
    format!(
        r###"You are a helpful AI assistant analyzing a PDF document.
Use the following {context_name} as your primary context:

--- DOCUMENT START ---
{text}
--- DOCUMENT END ---

The user has highlighted this specific passage and is asking about it:
--- HIGHLIGHTED PASSAGE ---
{selected_text}
--- END HIGHLIGHTED PASSAGE ---

Instructions:
- Answer questions based primarily on the document content.
{missing_answer_instruction}
- Keep answers concise and well-structured.
- For tabular comparisons, use a markdown table.{web_search_instruction}

When answering, follow these citation rules strictly:
1. Reference sources inline using [n] notation.
2. For PDF content: cite the exact passage you're referencing.
3. For web search results: cite the page title and URL.
4. At the end of every response, include a "## Sources" section listing all citations:
   - [n] "exact quoted text from the document"
   - [n] "exact quoted text from the page" - Page Title - URL
5. One quoted passage per entry. Do not name the section it comes from, and do not quote a second passage in the same entry — give it its own [n]. Quote Japanese passages with 「」.
6. End a web entry with its URL and write nothing after it: no parentheses around it, no trailing punctuation. A document entry carries no URL of its own.

Example:
The document states that Workers run on Cloudflare's global network[1].
キャッシュの扱いは指定できます[2]。
Service bindings connect two Workers directly[3].

## Sources
[1] "Workers execute on Cloudflare's global network across 300+ cities"
[2] 「public、privateはキャッシュを共有キャッシュとして扱ってよいかの指定に使います」
[3] "you can deploy an authentication service as its own Worker" - Service bindings · Cloudflare Workers docs - https://developers.cloudflare.com/workers/runtime-apis/bindings/service-bindings/"###
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn excerpt(is_partial: bool) -> Excerpt {
        Excerpt {
            text: "本文".to_owned(),
            start_page: if is_partial { 5 } else { 1 },
            end_page: if is_partial { 8 } else { 3 },
            total_pages: if is_partial { 12 } else { 3 },
            is_partial,
        }
    }

    #[test]
    fn drops_the_sources_section_from_a_past_answer() {
        // A reader can paste an answer back to ask about it, so a "## Sources"
        // line of their own must survive.
        let quoted_back = "この回答の出典が気になります。\n\n## Sources\n[1] 「エッジ」";
        let history = vec![
            Turn::user(quoted_back),
            Turn::assistant("エッジで動きます[1]。\n\n## Sources\n[1] 「エッジ」"),
        ];

        assert_eq!(
            build_conversation(&history, "では冷スタートはどうですか?"),
            vec![
                Turn::user(quoted_back),
                Turn::assistant("エッジで動きます[1]。"),
                Turn::user("では冷スタートはどうですか?"),
            ]
        );
    }

    #[test]
    fn a_conversation_with_no_history_is_just_the_question() {
        assert_eq!(build_conversation(&[], "質問"), vec![Turn::user("質問")]);
    }

    #[test]
    fn a_partial_excerpt_names_the_pages_it_shows() {
        let prompt = build_system_prompt(&excerpt(true), "一節", false);

        assert!(prompt.contains("excerpt (pages 5-8 of the 12-page document)"));
        assert!(prompt.contains("You are shown only pages 5-8"));
    }

    /// A one-page book is never told it is looking at a fragment.
    #[test]
    fn a_whole_excerpt_is_called_the_document() {
        let prompt = build_system_prompt(&excerpt(false), "一節", false);

        assert!(prompt.contains("Use the following document as your primary context"));
        assert!(!prompt.contains("excerpt"));
        assert!(prompt.contains("When the document does not contain the answer"));
    }

    #[test]
    fn the_passage_and_the_text_travel_in_their_own_blocks() {
        let prompt = build_system_prompt(&excerpt(true), "選んだ一節", false);

        let document = prompt
            .split("--- DOCUMENT START ---")
            .nth(1)
            .and_then(|rest| rest.split("--- DOCUMENT END ---").next())
            .expect("the document block");

        assert_eq!(document.trim(), "本文");
        assert!(prompt.contains("--- HIGHLIGHTED PASSAGE ---\n選んだ一節"));
    }

    #[test]
    fn web_search_is_offered_only_when_it_is_on() {
        assert!(build_system_prompt(&excerpt(true), "一節", true).contains("you may use web search"));
        assert!(
            build_system_prompt(&excerpt(true), "一節", false)
                .contains("Respond using only the excerpt context")
        );
    }
}
