//! A whole question, from a highlight to a stored answer with pages to jump to.
//!
//! The CLI is a stand-in that prints recorded JSONL, so this exercises every
//! step pedro owns — choosing the context, building the conversation, reading
//! the answer, resolving its sources — without credentials or a network.

use std::path::{Path, PathBuf};

use pedro_agent::fixtures::fake_cli;
use pedro_agent::{AgentKind, Cancellation, DiscoveredAgent};
use pedro_core::chat::{ChatError, Question, ask};
use pedro_core::model::{Highlight, NewHighlight, Role};
use pedro_core::store::Store;
use pedro_core::{Citation, CitationKind, PageLocation, PageMiss};
use pedro_core::{CitedBook, Conversation, Subject};
use pedro_pdf::{Rect, fixtures::pdf_with_pages};

/// A library holding one book, with one passage of it marked.
fn reading(name: &str, pages: &[&str], marked: &str, page: u32) -> (Store, Highlight) {
    let root = std::env::temp_dir().join(format!("pedro-chat-{name}"));
    let _ = std::fs::remove_dir_all(&root);

    let mut store = Store::open(&root).expect("a writable library");
    let source = root.join("book.pdf");
    std::fs::write(&source, pdf_with_pages(pages)).expect("a writable file");

    let book = store.add_document(&source).expect("a readable pdf");
    let highlight = store
        .add_highlight(
            &book.id,
            NewHighlight {
                selected_text: marked.to_owned(),
                page_number: page,
                rects: vec![Rect {
                    left: 0.1,
                    top: 0.2,
                    right: 0.5,
                    bottom: 0.24,
                }],
            },
        )
        .expect("a stored book");

    (store, highlight)
}

fn agent(program: PathBuf) -> DiscoveredAgent {
    DiscoveredAgent {
        kind: AgentKind::ClaudeCode,
        program,
        version: None,
    }
}

/// A CLI that streams `answer` one line at a time and reports it as its result.
fn answering(name: &str, answer: &str) -> PathBuf {
    let deltas: String = answer
        .lines()
        .map(|line| {
            let text = serde_json::to_string(&format!("{line}\n")).expect("a string encodes");
            // printf rather than echo: /bin/sh on macOS expands backslash
            // escapes in echo, which would turn the \n inside this JSON into a
            // real newline and split one event across two lines.
            format!(
                "printf '%s\\n' '{{\"type\":\"stream_event\",\"event\":{{\"type\":\"content_block_delta\",\
                 \"delta\":{{\"type\":\"text_delta\",\"text\":{text}}}}}}}'\n"
            )
        })
        .collect();

    let result = serde_json::to_string(answer).expect("a string encodes");
    fake_cli(
        name,
        &format!(
            "{deltas}printf '%s\\n' '{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\
             \"result\":{result}}}'"
        ),
    )
}

/// A CLI that answers nothing in particular but writes down what it was given:
/// the whole command line, and separately the last argument, which is where
/// both CLIs carry the conversation.
fn recording(name: &str) -> PathBuf {
    fake_cli(
        name,
        r#"
for argument in "$@"; do last="$argument"; done
directory="$(dirname "$0")"
printf '%s' "$*" > "$directory/command.txt"
printf '%s' "$last" > "$directory/conversation.txt"
printf '%s
' '{"type":"result","subtype":"success","is_error":false,"result":"ok"}'
"#,
    )
}

fn written(cli: &Path, name: &str) -> String {
    std::fs::read_to_string(cli.with_file_name(name)).expect("a recording")
}

fn question(highlight: &Highlight, text: &str) -> Question {
    Question {
        about: Subject::Passages(vec![highlight.id.clone()]),
        text: text.to_owned(),
        web_search: false,
    }
}

/// What was asked and what the stand-in was told, so a test can check that the
/// context really did reach the CLI.
fn asked(store: &Store, highlight: &Highlight, cli: PathBuf, text: &str) -> Vec<String> {
    let mut streamed = Vec::new();
    ask(
        store,
        &agent(cli),
        &question(highlight, text),
        &Cancellation::new(),
        &mut |delta| streamed.push(delta.to_owned()),
    )
    .expect("an answering agent");

    streamed
}

#[test]
fn an_answer_is_streamed_and_stored_with_its_sources() {
    let (store, highlight) = reading(
        "sources",
        &["preface", "the runtime runs at the edge", "later chapter"],
        "the runtime",
        2,
    );

    let answer = "It runs at the edge[1].\n\n## Sources\n[1] \"the runtime runs at the edge\"";
    let streamed = asked(
        &store,
        &highlight,
        answering("chat-sources", answer),
        "これは?",
    );

    assert!(!streamed.is_empty(), "nothing was streamed");
    assert_eq!(streamed.concat().trim(), answer);

    let messages = store
        .messages(&Conversation::Highlight(highlight.id.clone()))
        .expect("a query");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, Role::User);
    assert_eq!(messages[0].content, "これは?");

    assert_eq!(messages[1].role, Role::Assistant);
    assert_eq!(messages[1].content, answer);
    assert_eq!(
        messages[1].citations,
        vec![Citation {
            id: "1".to_owned(),
            kind: CitationKind::Pdf,
            text: "the runtime runs at the edge".to_owned(),
            page: Some(PageLocation::Found(2)),
            // The passage was found, so the citation says which book it is in
            // — which matters once a shelf answers from several.
            book: Some(CitedBook {
                id: highlight.book_id.clone(),
                title: "book.pdf".to_owned(),
            }),
            url: None,
        }]
    );
}

/// The reader's only hint that the model reworded a passage instead of quoting
/// it survives all the way into the stored answer.
#[test]
fn a_quote_the_book_does_not_hold_is_stored_as_a_miss() {
    let (store, highlight) = reading("miss", &["preface", "the runtime"], "the runtime", 2);

    let answer = "Something else[1].\n\n## Sources\n[1] \"a sentence this book never printed\"";
    asked(
        &store,
        &highlight,
        answering("chat-miss", answer),
        "これは?",
    );

    let citations = &store
        .messages(&Conversation::Highlight(highlight.id.clone()))
        .expect("a query")[1]
        .citations;
    assert_eq!(
        citations[0].page,
        Some(PageLocation::Missed(PageMiss::NotInBook))
    );
}

/// The conversation is pedro's: a second question carries the first exchange,
/// with the earlier answer's `## Sources` section left off.
#[test]
fn a_second_question_carries_the_conversation_without_repeating_the_sources() {
    let (store, highlight) = reading("history", &["preface", "the runtime"], "the runtime", 2);

    let first = "It runs at the edge[1].\n\n## Sources\n[1] \"the runtime\"";
    asked(
        &store,
        &highlight,
        answering("chat-first", first),
        "これは?",
    );

    let recorder = recording("chat-history");
    asked(&store, &highlight, recorder.clone(), "では冷スタートは?");

    // The conversation alone: the system prompt names a `## Sources` section
    // too, because asking for one is its whole job.
    let sent = written(&recorder, "conversation.txt");

    assert!(sent.contains("これは?"), "the first question was dropped");
    assert!(
        sent.contains("では冷スタートは?"),
        "the new question was dropped"
    );
    assert!(
        sent.contains("It runs at the edge[1]."),
        "the answer was dropped"
    );
    assert!(
        !sent.contains("## Sources"),
        "an answer's sources were sent back as history: {sent}"
    );
}

/// The context is the chapter the highlight sits in, not the whole book.
#[test]
fn only_the_pages_around_the_highlight_are_sent() {
    let pages: Vec<String> = (1..=40).map(|page| format!("page{page}")).collect();
    let pages: Vec<&str> = pages.iter().map(String::as_str).collect();
    let (store, highlight) = reading("excerpt", &pages, "page20", 20);

    let recorder = recording("chat-excerpt");
    asked(&store, &highlight, recorder.clone(), "これは?");

    let sent = written(&recorder, "command.txt");

    assert!(sent.contains("page20"), "the highlighted page was not sent");
    assert!(sent.contains("page10"), "the window was cut short");
    assert!(
        !sent.contains("page1 "),
        "a page outside the window was sent"
    );
    assert!(!sent.contains("page40"), "the whole book was sent");
    assert!(
        sent.contains("pages 10-30 of the 40-page document"),
        "the agent was not told which pages it has: {sent}"
    );
}

/// A failed answer leaves the question in the conversation: the reader did ask,
/// and should not have to type it again.
#[test]
fn a_refused_question_is_still_recorded() {
    let (store, highlight) = reading("refused", &["preface", "the runtime"], "the runtime", 2);

    let refusing = fake_cli(
        "chat-refusal",
        r#"
echo '{"type":"result","subtype":"success","is_error":true,"result":"Not logged in · Please run /login"}'
exit 1
"#,
    );

    let error = ask(
        &store,
        &agent(refusing),
        &question(&highlight, "これは?"),
        &Cancellation::new(),
        &mut |_| {},
    )
    .unwrap_err();

    // The agent layer turns a signed-out CLI into the one refusal with an
    // obvious next step, and the chat layer passes it through untouched.
    assert!(
        matches!(&error, ChatError::Agent(agent) if agent.to_string().contains("not signed in")),
        "{error}"
    );

    let messages = store
        .messages(&Conversation::Highlight(highlight.id.clone()))
        .expect("a query");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, Role::User);
}

#[test]
fn a_question_about_a_highlight_that_is_not_there_says_so() {
    let (store, _highlight) = reading("missing", &["preface"], "preface", 1);

    let error = ask(
        &store,
        &agent(PathBuf::from("/nonexistent/pedro/claude")),
        &Question {
            about: Subject::Passages(vec!["nope".to_owned()]),
            text: "これは?".to_owned(),
            web_search: false,
        },
        &Cancellation::new(),
        &mut |_| {},
    )
    .unwrap_err();

    assert!(matches!(error, ChatError::NoSuchHighlight(_)), "{error}");
}

/// Kept honest about the path the fixture helper builds.
#[test]
fn the_stand_in_is_written_where_it_is_run_from() {
    let cli = answering("chat-path", "ok");
    assert!(Path::new(&cli).is_file());
}

/// A question can be about more than one passage, and what the agent is given
/// has to hold all of them and the context around each.
#[test]
fn a_question_can_be_about_two_passages_at_once() {
    let pages: Vec<String> = (1..=40).map(|page| format!("page{page}")).collect();
    let pages: Vec<&str> = pages.iter().map(String::as_str).collect();
    let (store, first) = reading("two-passages", &pages, "page5", 5);

    let book_id = first.book_id.clone();
    let second = store
        .add_highlight(
            &book_id,
            NewHighlight {
                selected_text: "page30".to_owned(),
                page_number: 30,
                rects: Vec::new(),
            },
        )
        .expect("a stored book");

    let recorder = recording("chat-two-passages");
    let mut streamed = Vec::new();
    ask(
        &store,
        &agent(recorder.clone()),
        &Question {
            about: Subject::Passages(vec![first.id.clone(), second.id.clone()]),
            text: "どう違う?".to_owned(),
            web_search: false,
        },
        &Cancellation::new(),
        &mut |delta| streamed.push(delta.to_owned()),
    )
    .expect("an answering agent");

    let sent = written(&recorder, "command.txt");

    // Both passages, numbered so an answer can say which one it means.
    assert!(sent.contains("HIGHLIGHTED PASSAGE 1 (page 5)"), "{sent}");
    assert!(sent.contains("HIGHLIGHTED PASSAGE 2 (page 30)"), "{sent}");

    // And the pages around each of them, which are two windows rather than
    // everything between the two.
    assert!(sent.contains("page5") && sent.contains("page30"));
    assert!(!sent.contains("page18"), "the gap between them was sent");

    // The conversation hangs off the first, which is where the reader will
    // look for it.
    assert_eq!(
        store
            .messages(&Conversation::Highlight(first.id.clone()))
            .expect("a query")
            .len(),
        2
    );
    assert!(
        store
            .messages(&Conversation::Highlight(second.id.clone()))
            .expect("a query")
            .is_empty()
    );
}

/// A shelf of two books, each with a distinctive passage in it.
fn shelf(name: &str) -> (Store, String) {
    let root = std::env::temp_dir().join(format!("pedro-shelf-{name}"));
    let _ = std::fs::remove_dir_all(&root);

    let mut store = Store::open(&root).expect("a writable library");
    let shelf = store.create_folder("暗号").expect("a shelf");

    // Latin text: the fixture writes a PDF whose text extraction mangles
    // anything else, so a Japanese passage would come back out as mojibake and
    // this would be a test of the fixture rather than of the shelf.
    for (file, pages) in [
        (
            "primes.pdf",
            [
                "preface",
                "a prime is divisible only by one and itself",
                "afterword",
            ],
        ),
        (
            "keys.pdf",
            [
                "introduction",
                "the key length is chosen from the work factor",
                "index",
            ],
        ),
    ] {
        let source = root.join(file);
        std::fs::write(&source, pdf_with_pages(&pages)).expect("a writable file");
        let book = store.add_document(&source).expect("a readable pdf");
        store
            .move_book(&book.id, Some(&shelf.id))
            .expect("a shelved book");
    }

    (store, shelf.id)
}

fn about_shelf(id: &str, text: &str) -> Question {
    Question {
        about: Subject::Shelf(id.to_owned()),
        text: text.to_owned(),
        web_search: false,
    }
}

/// A question put to a shelf is answered from every book on it, and the
/// passages it is given say which book each one came from.
#[test]
fn a_question_to_a_shelf_gathers_from_all_of_its_books() {
    let (store, shelf_id) = shelf("gathers");
    let recorder = recording("chat-shelf");

    ask(
        &store,
        &agent(recorder.clone()),
        &about_shelf(&shelf_id, "prime"),
        &Cancellation::new(),
        &mut |_| {},
    )
    .expect("an answering agent");

    let sent = written(&recorder, "command.txt");

    assert!(
        sent.contains("shelf of PDF documents called \"暗号\""),
        "{sent}"
    );
    assert!(
        sent.contains("primes.pdf"),
        "the book was not named: {sent}"
    );
    assert!(
        sent.contains("a prime is divisible only by one and itself"),
        "the passage was not sent: {sent}"
    );
}

/// A source is looked up by its quotation, so the answer says which book it is
/// in without the model having to name it — and the page is the page of that
/// book, not of whichever book happened to be first.
#[test]
fn a_shelf_citation_resolves_to_the_book_it_is_in() {
    let (store, shelf_id) = shelf("citation");

    let answer = "The key length follows the work factor[1].\n\n## Sources\n[1] \"the key length is chosen from the work factor\"";
    ask(
        &store,
        &agent(answering("chat-shelf-citation", answer)),
        &about_shelf(&shelf_id, "key length"),
        &Cancellation::new(),
        &mut |_| {},
    )
    .expect("an answering agent");

    let messages = store
        .messages(&Conversation::Folder(shelf_id))
        .expect("a query");
    assert_eq!(messages.len(), 2);

    let citation = &messages[1].citations[0];
    assert_eq!(citation.page, Some(PageLocation::Found(2)));
    assert_eq!(
        citation.book.as_ref().map(|book| book.title.as_str()),
        Some("keys.pdf"),
        "{citation:?}"
    );
}

/// A shelf with nothing on it has nothing to answer from, and says so rather
/// than asking an agent a question with an empty context.
#[test]
fn a_question_to_an_empty_shelf_says_so() {
    let root = std::env::temp_dir().join("pedro-shelf-empty");
    let _ = std::fs::remove_dir_all(&root);
    let store = Store::open(&root).expect("a writable library");
    let shelf = store.create_folder("空").expect("a shelf");

    let error = ask(
        &store,
        &agent(PathBuf::from("/nonexistent/pedro/claude")),
        &about_shelf(&shelf.id, "これは?"),
        &Cancellation::new(),
        &mut |_| {},
    )
    .expect_err("an empty shelf");

    assert!(matches!(error, ChatError::EmptyShelf), "{error:?}");
}

/// The conversation is the shelf's, and a second question carries the first.
#[test]
fn a_shelf_conversation_carries_its_turns() {
    let (store, shelf_id) = shelf("turns");
    let recorder = recording("chat-shelf-turns");

    for text in ["the first question", "the second question"] {
        ask(
            &store,
            &agent(recorder.clone()),
            &about_shelf(&shelf_id, text),
            &Cancellation::new(),
            &mut |_| {},
        )
        .expect("an answering agent");
    }

    let sent = written(&recorder, "conversation.txt");
    assert!(
        sent.contains("the first question"),
        "the first turn was dropped: {sent}"
    );
    assert!(sent.contains("the second question"), "{sent}");

    assert_eq!(
        store
            .messages(&Conversation::Folder(shelf_id))
            .expect("a query")
            .len(),
        4
    );
}
