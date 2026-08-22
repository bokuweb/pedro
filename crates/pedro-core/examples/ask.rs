//! Asks an installed agent about a passage of a real PDF, without a window.
//!
//! ```bash
//! cargo run -p pedro-core --example ask -- book.pdf 12 "この節の要点は?"
//! ```
//!
//! Everything the reader's chat panel will do happens here already: the file is
//! added to the library, a passage of the page is marked, the chapter around it
//! becomes the context, an installed CLI answers, and the sources it names are
//! resolved to pages. It writes to the real library, so a book added twice is
//! the same book.

use std::io::Write as _;
use std::path::PathBuf;

use pedro_agent::Cancellation;
use pedro_core::Subject;
use pedro_core::chat::{Question, ask};
use pedro_core::model::NewHighlight;
use pedro_core::store::Store;
use pedro_core::{CitationKind, PageLocation};
use pedro_pdf::Document;

/// How much of the page to mark. A paragraph's worth: enough for the question
/// to be about something, short enough to stay a quotation.
const PASSAGE: usize = 300;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()))
        .init();

    let mut arguments = std::env::args().skip(1);
    let (Some(path), Some(page), Some(question)) =
        (arguments.next(), arguments.next(), arguments.next())
    else {
        eprintln!("usage: ask <file.pdf> <page> <question>");
        std::process::exit(2);
    };

    let path = PathBuf::from(path);
    let page: u32 = page.parse()?;

    let agents = pedro_agent::discover();
    let Some(agent) = agents.first() else {
        eprintln!("no agent CLI found: install claude or codex and authenticate it");
        std::process::exit(1);
    };
    println!("asking {}", agent.kind.display_name());

    let mut store = Store::open_default()?;
    let book = store.add_document(&path)?;
    println!(
        "{} — {} pages, {} chapters",
        book.file_name,
        book.page_count,
        book.outline.len()
    );

    // Mark the top of the page, the way a drag across it would.
    let document = Document::open(&store.document_path(&book))?;
    let text = document.page_text(page - 1)?;
    let marked: String = text.text.chars().take(PASSAGE).collect();
    if marked.trim().is_empty() {
        eprintln!("page {page} has no text to ask about");
        std::process::exit(1);
    }

    let last = marked.chars().count().saturating_sub(1);
    let highlight = store.add_highlight(
        &book.id,
        NewHighlight {
            selected_text: marked.clone(),
            page_number: page,
            rects: text.line_rects(0, last),
        },
    )?;
    println!("\n--- marked on page {page} ---\n{}\n---\n", marked.trim());

    let answer = ask(
        &store,
        agent,
        &Question {
            about: Subject::Passages(vec![highlight.id.clone()]),
            text: question,
            web_search: false,
        },
        &Cancellation::new(),
        &mut |delta| {
            print!("{delta}");
            let _ = std::io::stdout().flush();
        },
    )?;

    println!("\n\n--- sources ---");
    for citation in &answer.citations {
        let where_from = match (citation.kind, &citation.url, citation.page) {
            (CitationKind::Web, Some(url), _) => url.clone(),
            (_, _, Some(PageLocation::Found(page))) => format!("page {page}"),
            (_, _, Some(PageLocation::Missed(miss))) => format!("not found ({miss:?})"),
            _ => "unresolved".to_owned(),
        };
        println!("[{}] {where_from} — {}", citation.id, citation.text);
    }

    Ok(())
}
