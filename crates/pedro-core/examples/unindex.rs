//! Throws away a library's vectors, so that indexing has work to do again.
//!
//! ```bash
//! PEDRO_LIBRARY_PATH=/tmp/a-copy cargo run -p pedro-core --example unindex
//! ```

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var_os("PEDRO_LIBRARY_PATH").is_none() {
        eprintln!("point this at a copy: PEDRO_LIBRARY_PATH=/tmp/a-copy");
        std::process::exit(2);
    }

    pedro_search::index::prepare();
    let root = std::env::var("PEDRO_LIBRARY_PATH")?;
    let connection = rusqlite::Connection::open(std::path::Path::new(&root).join("pedro.sqlite3"))?;

    let before: i64 =
        connection.query_row("SELECT count(*) FROM chunks_vec", [], |row| row.get(0))?;
    connection.execute("DELETE FROM chunks_vec", [])?;

    println!("threw away {before} vectors");
    Ok(())
}
