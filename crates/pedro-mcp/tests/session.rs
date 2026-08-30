//! The server against a real library, one message at a time.
//!
//! Everything but the pipe: [`Session::handle`] is what the stdio loop calls
//! for each line, so driving it directly covers the protocol, the tools and
//! the store without a subprocess in the way.

use pedro_core::model::NewHighlight;
use pedro_core::store::Store;
use pedro_mcp::{PROTOCOL_VERSION, Session};
use pedro_pdf::{Rect, fixtures::pdf_with_pages};
use serde_json::{Value, json};

/// A session over a library of its own, emptied first so a rerun starts clean.
fn session(name: &str) -> (Session, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!("pedro-mcp-{name}"));
    let _ = std::fs::remove_dir_all(&root);
    let store = Store::open(&root).expect("a writable library");

    (Session::new(store), root)
}

/// A PDF next to the library, the way a user's file sits on disk.
fn pdf(root: &std::path::Path, name: &str, pages: &[&str]) -> std::path::PathBuf {
    let path = root.join(name);
    std::fs::create_dir_all(root).expect("a writable directory");
    std::fs::write(&path, pdf_with_pages(pages)).expect("a writable file");

    path
}

fn send(session: &mut Session, id: u32, method: &str, params: Value) -> Value {
    let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    let answer = session
        .handle(&message.to_string())
        .unwrap_or_else(|| panic!("an answer to {method}"));

    assert_eq!(answer["id"], json!(id), "the id is echoed back");
    answer
}

/// Calls a tool and hands back the text of its result, which is what a model
/// would read.
fn call(session: &mut Session, name: &str, arguments: Value) -> (String, bool) {
    let answer = send(
        session,
        99,
        "tools/call",
        json!({ "name": name, "arguments": arguments }),
    );
    assert!(
        answer.get("error").is_none(),
        "{name} failed at the protocol level: {answer}"
    );

    let result = &answer["result"];
    (
        result["content"][0]["text"]
            .as_str()
            .expect("text content")
            .to_owned(),
        result["isError"] == json!(true),
    )
}

#[test]
fn initialize_names_the_server_and_what_it_offers() {
    let (mut session, _root) = session("initialize");

    let answer = send(
        &mut session,
        1,
        "initialize",
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "claude-code", "version": "1" },
        }),
    );

    let result = &answer["result"];
    assert_eq!(result["protocolVersion"], json!(PROTOCOL_VERSION));
    assert_eq!(result["serverInfo"]["name"], json!("pedro"));
    assert!(
        result["capabilities"]["tools"].is_object(),
        "tools are what pedro offers"
    );
    assert!(
        result["instructions"].as_str().unwrap().contains("library"),
        "the client is told what it is reaching into"
    );
    assert_eq!(session.client(), Some("claude-code"));
}

#[test]
fn a_client_on_an_older_protocol_is_answered_in_its_own_version() {
    let (mut session, _root) = session("older-protocol");

    let answer = send(
        &mut session,
        1,
        "initialize",
        json!({ "protocolVersion": "2024-11-05" }),
    );

    assert_eq!(answer["result"]["protocolVersion"], json!("2024-11-05"));
}

#[test]
fn a_client_on_a_version_pedro_does_not_know_is_answered_in_pedros() {
    let (mut session, _root) = session("unknown-protocol");

    let answer = send(
        &mut session,
        1,
        "initialize",
        json!({ "protocolVersion": "1999-01-01" }),
    );

    assert_eq!(answer["result"]["protocolVersion"], json!(PROTOCOL_VERSION));
}

#[test]
fn a_notification_is_not_answered() {
    let (mut session, _root) = session("notification");

    let message = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
    assert!(session.handle(&message.to_string()).is_none());
}

#[test]
fn a_reply_to_something_pedro_never_sent_is_ignored() {
    let (mut session, _root) = session("stray-reply");

    let message = json!({ "jsonrpc": "2.0", "id": 7, "result": {} });
    assert!(session.handle(&message.to_string()).is_none());
}

#[test]
fn a_request_naming_no_method_is_answered_rather_than_left_hanging() {
    let (mut session, _root) = session("no-method");

    let message = json!({ "jsonrpc": "2.0", "id": 7, "params": {} });
    let answer = session.handle(&message.to_string()).expect("an answer");

    assert_eq!(answer["id"], json!(7));
    assert_eq!(answer["error"]["code"], json!(-32600));
}

#[test]
fn a_line_that_is_not_json_is_a_parse_error_and_the_session_goes_on() {
    let (mut session, _root) = session("parse-error");

    let answer = session.handle("{not json").expect("an answer");
    assert_eq!(answer["error"]["code"], json!(-32700));
    assert_eq!(answer["id"], Value::Null, "there was no id to echo");

    let answer = send(&mut session, 2, "ping", json!({}));
    assert!(answer["result"].is_object(), "the session survived it");
}

#[test]
fn an_empty_line_is_not_a_message() {
    let (mut session, _root) = session("empty-line");

    assert!(session.handle("").is_none());
    assert!(session.handle("   \n").is_none());
}

#[test]
fn a_method_pedro_does_not_have_says_so() {
    let (mut session, _root) = session("no-method-named");

    let answer = send(&mut session, 3, "resources/list", json!({}));
    assert_eq!(answer["error"]["code"], json!(-32601));
}

#[test]
fn every_tool_is_listed_with_a_schema_a_client_can_read() {
    let (mut session, _root) = session("tools-list");

    let answer = send(&mut session, 4, "tools/list", json!({}));
    let tools = answer["result"]["tools"].as_array().expect("tools");

    let named: Vec<&str> = tools
        .iter()
        .map(|tool| tool["name"].as_str().expect("a name"))
        .collect();
    assert_eq!(
        named,
        vec![
            "list_books",
            "search_library",
            "read_pages",
            "book_contents",
            "add_book",
        ]
    );

    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        assert!(
            tool["description"].as_str().is_some_and(|it| it.len() > 40),
            "{name} tells the model what it is for"
        );
        assert_eq!(
            tool["inputSchema"]["type"],
            json!("object"),
            "{name} takes an object"
        );
    }
}

#[test]
fn asking_for_a_tool_that_is_not_there_is_a_protocol_error() {
    let (mut session, _root) = session("no-such-tool");

    let answer = send(
        &mut session,
        5,
        "tools/call",
        json!({ "name": "ask_the_agent", "arguments": {} }),
    );
    assert_eq!(answer["error"]["code"], json!(-32602));
}

#[test]
fn a_missing_argument_is_a_protocol_error_rather_than_an_empty_search() {
    let (mut session, _root) = session("missing-argument");

    let answer = send(
        &mut session,
        6,
        "tools/call",
        json!({ "name": "search_library", "arguments": {} }),
    );
    assert_eq!(answer["error"]["code"], json!(-32602));
    assert!(
        answer["error"]["message"]
            .as_str()
            .unwrap()
            .contains("query")
    );
}

#[test]
fn an_empty_library_says_so_rather_than_failing() {
    let (mut session, _root) = session("empty-library");

    let (books, failed) = call(&mut session, "list_books", json!({}));
    assert!(!failed);
    assert!(books.contains("empty"), "{books}");

    let (found, failed) = call(&mut session, "search_library", json!({ "query": "primes" }));
    assert!(!failed, "finding nothing is an answer, not a failure");
    assert!(found.contains("Nothing"), "{found}");
}

#[test]
fn a_book_that_is_not_there_is_the_models_problem_to_solve_not_the_clients() {
    let (mut session, _root) = session("no-such-book");

    let (said, failed) = call(
        &mut session,
        "read_pages",
        json!({ "book_id": "invented", "page": 1 }),
    );
    assert!(failed, "the model is told this went wrong");
    assert!(
        said.contains("list_books"),
        "and what to do about it: {said}"
    );
}

#[test]
fn a_path_that_is_not_absolute_is_refused_before_anything_is_read() {
    let (mut session, _root) = session("relative-path");

    let answer = send(
        &mut session,
        7,
        "tools/call",
        json!({ "name": "add_book", "arguments": { "path": "book.pdf" } }),
    );
    assert_eq!(answer["error"]["code"], json!(-32602));
}

#[test]
fn a_book_is_added_listed_searched_and_read() {
    let (mut session, root) = session("whole-path");
    let path = pdf(
        &root,
        "primes.pdf",
        &[
            "A prime number has no divisors but itself and one.",
            "The sieve of Eratosthenes crosses out the multiples.",
            "Every integer factors into primes in one way only.",
        ],
    );

    let (added, failed) = call(
        &mut session,
        "add_book",
        json!({ "path": path.to_str().unwrap() }),
    );
    assert!(!failed, "{added}");
    assert!(added.contains("primes.pdf"), "{added}");

    let (books, _) = call(&mut session, "list_books", json!({}));
    assert!(books.contains("primes.pdf"), "{books}");
    assert!(books.contains("3 pages"), "{books}");

    // The id every other tool takes, read back out the way a model would.
    let book_id = book_id(&mut session);

    let (found, failed) = call(
        &mut session,
        "search_library",
        json!({ "query": "sieve of Eratosthenes", "limit": 3 }),
    );
    assert!(!failed, "{found}");
    assert!(found.contains("page 2"), "the hit names its page: {found}");
    assert!(found.contains("Eratosthenes"), "{found}");

    let (page, failed) = call(
        &mut session,
        "read_pages",
        json!({ "book_id": book_id, "page": 3 }),
    );
    assert!(!failed, "{page}");
    assert!(page.contains("factors into primes"), "{page}");
    assert!(
        !page.contains("sieve of Eratosthenes"),
        "only the page asked for: {page}"
    );

    let (range, _) = call(
        &mut session,
        "read_pages",
        json!({ "book_id": book_id, "page": 1, "until": 3 }),
    );
    assert!(range.contains("--- page 1 ---"), "{range}");
    assert!(range.contains("--- page 3 ---"), "{range}");

    let (contents, failed) = call(&mut session, "book_contents", json!({ "book_id": book_id }));
    assert!(!failed, "{contents}");
    assert!(
        contents.contains("no outline"),
        "the fixture ships none, and that is said plainly: {contents}"
    );
}

#[test]
fn a_page_past_the_end_says_how_long_the_book_is() {
    let (mut session, root) = session("past-the-end");
    let path = pdf(&root, "short.pdf", &["One page, and that is all of it."]);
    call(
        &mut session,
        "add_book",
        json!({ "path": path.to_str().unwrap() }),
    );

    let book_id = book_id(&mut session);
    let (said, failed) = call(
        &mut session,
        "read_pages",
        json!({ "book_id": book_id, "page": 40 }),
    );
    assert!(failed);
    assert!(said.contains("1 pages") || said.contains("has 1"), "{said}");
}

#[test]
fn a_search_can_be_held_to_one_book() {
    let (mut session, root) = session("one-book");
    for (name, page) in [
        ("primes.pdf", "A prime number has no divisors but itself."),
        ("ships.pdf", "A prime mover drives the propeller shaft."),
    ] {
        let path = pdf(&root, name, &[page]);
        let (said, failed) = call(
            &mut session,
            "add_book",
            json!({ "path": path.to_str().unwrap() }),
        );
        assert!(!failed, "{said}");
    }

    let (both, _) = call(&mut session, "search_library", json!({ "query": "prime" }));
    assert!(
        both.contains("primes.pdf") && both.contains("ships.pdf"),
        "{both}"
    );

    // The most recently added book is first, and that is the one to hold to.
    let only = book_id(&mut session);
    let (one, failed) = call(
        &mut session,
        "search_library",
        json!({ "query": "prime", "book_id": only }),
    );
    assert!(!failed, "{one}");
    assert!(one.contains("ships.pdf"), "{one}");
    assert!(!one.contains("primes.pdf"), "{one}");
}

#[test]
fn the_reading_position_the_reader_left_is_in_the_listing() {
    let (mut session, root) = session("reading-position");
    let path = pdf(&root, "manual.pdf", &["One.", "Two.", "Three."]);
    call(
        &mut session,
        "add_book",
        json!({ "path": path.to_str().unwrap() }),
    );

    // What the reader does when it closes a book, done to the same library.
    {
        let store = Store::open(&root).expect("the same library");
        let book = store.books().expect("books").remove(0);
        let highlight = store
            .add_highlight(
                &book.id,
                NewHighlight {
                    selected_text: "Two.".to_owned(),
                    page_number: 2,
                    rects: vec![Rect {
                        left: 0.1,
                        top: 0.2,
                        right: 0.5,
                        bottom: 0.24,
                    }],
                },
            )
            .expect("a highlight");
        store
            .save_reading_state(
                &book.id,
                &pedro_core::model::ReadingState {
                    page: 2,
                    highlight_id: Some(highlight.id),
                    outline_open: None,
                    chat_panel_open: None,
                    spread: None,
                },
            )
            .expect("a saved place");
    }

    let (books, _) = call(&mut session, "list_books", json!({}));
    assert!(books.contains("last read at page 2"), "{books}");
}

#[test]
fn a_search_held_to_a_book_that_has_nothing_says_what_the_library_has() {
    let (mut session, root) = session("nothing-in-this-book");
    for (name, page) in [
        ("ships.pdf", "A prime mover drives the propeller shaft."),
        ("recipes.pdf", "Simmer the onions until they are soft."),
    ] {
        let path = pdf(&root, name, &[page]);
        call(
            &mut session,
            "add_book",
            json!({ "path": path.to_str().unwrap() }),
        );
    }

    // The most recently added is first, and it is the one with no propellers.
    let recipes = book_id(&mut session);
    let (said, failed) = call(
        &mut session,
        "search_library",
        json!({ "query": "propeller shaft", "book_id": recipes }),
    );

    assert!(!failed, "finding nothing is an answer: {said}");
    assert!(said.contains("Nothing matches"), "{said}");
    assert!(
        said.contains("Other books in the library do match"),
        "and the library did have something: {said}"
    );
}

#[test]
fn one_of_something_is_never_called_one_of_them() {
    let (mut session, root) = session("plurals");
    let path = pdf(&root, "one.pdf", &["A single page, and one book."]);
    call(
        &mut session,
        "add_book",
        json!({ "path": path.to_str().unwrap() }),
    );

    let (books, _) = call(&mut session, "list_books", json!({}));
    assert!(books.contains("1 book in the library"), "{books}");

    let (found, _) = call(
        &mut session,
        "search_library",
        json!({ "query": "single page", "limit": 1 }),
    );
    assert!(found.contains("1 passage for"), "{found}");
}

#[test]
fn a_book_on_a_shelf_says_which_shelf() {
    let (mut session, root) = session("shelves");
    let path = pdf(&root, "ciphers.pdf", &["A cipher hides what it carries."]);
    call(
        &mut session,
        "add_book",
        json!({ "path": path.to_str().unwrap() }),
    );

    // What the reader does when a book is dragged onto a shelf.
    {
        let store = Store::open(&root).expect("the same library");
        let book = store.books().expect("books").remove(0);
        let shelf = store.create_folder("Cryptography", None).expect("a shelf");
        store
            .move_book(&book.id, Some(&shelf.id))
            .expect("a book on it");
    }

    let (books, _) = call(&mut session, "list_books", json!({}));
    assert!(books.contains("on shelf Cryptography"), "{books}");
}

/// The id of the first book in the library, taken the way a model would: out
/// of what `list_books` said.
fn book_id(session: &mut Session) -> String {
    let (books, _) = call(session, "list_books", json!({}));
    books
        .split_whitespace()
        .skip_while(|word| *word != "book_id")
        .nth(1)
        .expect("an id in the listing")
        .to_owned()
}
