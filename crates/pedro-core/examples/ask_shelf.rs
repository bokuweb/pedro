//! Puts a question to a shelf of the library, and prints what it was answered
//! from.
//!
//! ```bash
//! cargo run -p pedro-core --example ask_shelf -- "この2冊に共通する話題は?"
//! ```
//!
//! Makes a shelf and holds a conversation, so point it at a copy:
//!
//! ```bash
//! PEDRO_LIBRARY_PATH=/tmp/a-copy cargo run -p pedro-core --example ask_shelf -- "…"
//! ```

use pedro_core::chat::{Question, Subject, ask};
use pedro_core::store::Store;
use pedro_core::{Conversation, PageLocation};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "pedro=info".into()))
        .init();

    let question = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    if question.trim().is_empty() {
        eprintln!("usage: ask_shelf <question>");
        std::process::exit(2);
    }

    // This makes a shelf and clears the ones already there, so it refuses to
    // run against the library it would find on its own. A diagnostic that can
    // quietly rearrange the reader's shelves is not one worth having.
    if std::env::var_os("PEDRO_LIBRARY_PATH").is_none() {
        eprintln!(
            "This clears the shelves of whatever library it opens.\n\
             Point it at a copy:\n\n    \
             PEDRO_LIBRARY_PATH=/tmp/a-copy cargo run -p pedro-core --example ask_shelf -- \"…\""
        );
        std::process::exit(2);
    }

    let store = Store::open_default()?;
    println!("library: {}", store.root().display());

    // A shelf of everything, made fresh each run.
    for existing in store.folders()? {
        store.remove_folder(&existing.id)?;
    }
    let shelf = store.create_folder("暗号とPython", None)?;
    for book in store.books()? {
        store.move_book(&book.id, Some(&shelf.id))?;
        println!("on the shelf: {}", book.file_name);
    }

    let agent = pedro_agent::discover()
        .into_iter()
        .next()
        .ok_or("no agent CLI")?;
    println!("\nasking {} — {question}\n", agent.kind.display_name());

    let answer = ask(
        &store,
        &agent,
        &Question {
            about: Subject::Shelf(shelf.id.clone()),
            text: question,
            web_search: false,
        },
        &pedro_agent::Cancellation::new(),
        &mut |delta| {
            use std::io::Write as _;
            print!("{delta}");
            std::io::stdout().flush().ok();
        },
    )?;

    println!("\n\n--- sources ---");
    for citation in &answer.citations {
        let page = match citation.page {
            Some(PageLocation::Found(page)) => format!("p.{page}"),
            Some(PageLocation::Missed(miss)) => format!("{miss:?}"),
            None => "web".to_owned(),
        };
        let book = citation
            .book
            .as_ref()
            .map(|book| book.title.as_str())
            .unwrap_or("—");

        println!("[{}] {page:<8} {book}", citation.id);
    }

    let turns = store.messages(&Conversation::Folder(shelf.id))?;
    println!("\nthe shelf's conversation is {} turns", turns.len());

    Ok(())
}
