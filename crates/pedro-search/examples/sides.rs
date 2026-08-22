//! What each way of looking returns for a question, before they are fused.
//!
//! ```bash
//! cargo run -p pedro-search --example sides -- "素数はどうやって生成する?"
//! ```
//!
//! Reads the reader's own library. Fusion hides which side found what, and this
//! is the tool that made the difference between the two cuts visible — and that
//! showed a table of contents rising to the top because the overlap was being
//! dropped from one of them.

use pedro_search::{Embedder, index};
use rusqlite::Connection;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let question = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    index::prepare();
    let library = std::env::var("HOME")? + "/Library/Application Support/pedro/pedro.sqlite3";
    let connection = Connection::open(library)?;

    println!("=== words");
    for hit in index::search_about(&connection, &question, 5)? {
        println!(
            "  p.{:<5} {:.3}  {}",
            hit.page_number,
            hit.score,
            hit.text
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(60)
                .collect::<String>()
        );
    }

    println!("=== pairs (the search box)");
    for hit in index::search(&connection, &question, 5)? {
        println!(
            "  p.{:<5} {:.3}  {}",
            hit.page_number,
            hit.score,
            hit.text
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(60)
                .collect::<String>()
        );
    }

    if let Some(embedder) = Embedder::find() {
        println!("=== meaning");
        for hit in index::search_similar(&connection, &embedder.embed(&question)?, 5, 0.25)? {
            println!(
                "  p.{:<5} {:.3}  {}",
                hit.page_number,
                hit.score,
                hit.text
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .chars()
                    .take(60)
                    .collect::<String>()
            );
        }
    }

    Ok(())
}
