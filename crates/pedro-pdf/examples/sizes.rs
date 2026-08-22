//! Times reading every page's size, the two ways it can be read.
//!
//! ```bash
//! cargo run -p pedro-pdf --example sizes -- book.pdf
//! ```

use pedro_pdf::Document;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("usage: sizes <pdf>")?;
    let document = Document::open(std::path::Path::new(&path))?;
    println!("{} pages", document.page_count());

    let started = std::time::Instant::now();
    let sizes = document.page_sizes()?;
    println!(
        "all at once:  {:?} for {} sizes",
        started.elapsed(),
        sizes.len()
    );

    let started = std::time::Instant::now();
    let some = document.page_count().min(50);
    for index in 0..some {
        document.page_size(index)?;
    }
    println!("one at a time: {:?} for {some}", started.elapsed());

    let sideways = sizes.iter().filter(|s| s.width > s.height).count();
    println!(
        "{sideways} of {} pages are wider than they are tall",
        sizes.len()
    );

    Ok(())
}
