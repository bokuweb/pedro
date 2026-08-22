//! Searches the library from the command line.
//!
//! ```bash
//! cargo run -p pedro-core --example find -- 素数
//! ```
//!
//! The same index the reader's search box uses, without a window in the way.

use pedro_core::store::Store;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()))
        .init();

    let query = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    if query.trim().is_empty() {
        eprintln!("usage: find <query>");
        std::process::exit(2);
    }

    let store = Store::open_default()?;
    println!(
        "searching by {}",
        match store.can_search_by_meaning() {
            true => "words and meaning",
            false => "words alone (run scripts/fetch-embedding.sh for meaning)",
        }
    );
    let titles: std::collections::HashMap<String, String> = store
        .books()?
        .into_iter()
        .map(|book| (book.id, book.file_name))
        .collect();

    let hits = store.search(&query)?;
    println!("{} passages", hits.len());

    for hit in hits.iter().take(10) {
        let book = titles
            .get(&hit.book_id)
            .map(String::as_str)
            .unwrap_or("a book");
        let passage: String = hit.text.split_whitespace().collect::<Vec<_>>().join(" ");
        let passage: String = passage.chars().take(90).collect();

        println!("\n{book} p.{}  ({:.2})\n  {passage}…", hit.page_number, hit.score);
    }

    Ok(())
}
