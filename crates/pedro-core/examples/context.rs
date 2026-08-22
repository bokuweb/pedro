//! Shows what a question would be answered from, without asking anyone.
//!
//! ```bash
//! cargo run -p pedro-core --example context -- "鍵長はどう推定する?"
//! ```
//!
//! The passages a search turns up for the question, which are what a question
//! about the book carries with it beyond the pages the reader marked.

use pedro_core::store::Store;

/// As many as a question is given.
const PASSAGES: usize = 6;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()))
        .init();

    let question = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    if question.trim().is_empty() {
        eprintln!("usage: context <question>");
        std::process::exit(2);
    }

    let store = Store::open_default()?;
    let Some(book) = store.books()?.into_iter().next() else {
        eprintln!("the library is empty");
        std::process::exit(1);
    };

    println!(
        "{}\nasking about: {question}\nretrieval: {}\n",
        book.file_name,
        match store.can_search_by_meaning() {
            true => "words and meaning",
            false => "words alone",
        }
    );

    for passage in store.passages_for(std::slice::from_ref(&book.id), &question, PASSAGES)? {
        let text: String = passage
            .text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let text: String = text.chars().take(120).collect();

        println!(
            "p.{:<5} {:.3}\n  {text}…\n",
            passage.page_number, passage.score
        );
    }

    Ok(())
}
