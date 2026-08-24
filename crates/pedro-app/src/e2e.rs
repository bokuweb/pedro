//! The application driven the way a reader drives it.
//!
//! gpui's test harness opens a real window with a real element tree, and
//! delivers clicks and keystrokes through the same dispatch the reader's go
//! through. Nothing here is a stand-in for the shell: the store is a real
//! SQLite library in a temporary directory and the books are real PDFs that
//! pdfium really reads.
//!
//! What it cannot do is look. It can say which page the reader is on, which
//! rows the pages are in, and where a thing was laid out — not whether the
//! result is legible.

use std::ops::Deref as _;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use gpui::{AppContext as _, TestAppContext, VisualTestContext};

use crate::app::Pedro;
use crate::state::AgentStatus;
use pedro_agent::{AgentKind, DiscoveredAgent};
use pedro_core::model::NewHighlight;

/// `PEDRO_LIBRARY_PATH` is process-wide, so a test that sets it holds this for
/// as long as it is looking at that library.
static ONE_LIBRARY_AT_A_TIME: Mutex<()> = Mutex::new(());

/// A library of its own, and the application looking at it.
struct Reader {
    _library: MutexGuard<'static, ()>,
    root: PathBuf,
}

impl Reader {
    /// Writes `books` into a fresh library and points the application at it.
    fn with(name: &str, books: &[(&str, Vec<u8>)]) -> Self {
        let library = ONE_LIBRARY_AT_A_TIME
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let root = std::env::temp_dir().join(format!("pedro-e2e-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("a writable directory");

        // Safety: the guard above is what keeps a second test from reading this
        // while it is set for this one.
        unsafe { std::env::set_var("PEDRO_LIBRARY_PATH", &root) };

        // Added through the store rather than through the window, so the
        // application finds a library that already has books in it — which is
        // the state every launch after the first is in.
        let mut store = pedro_core::store::Store::open(&root).expect("a writable library");
        for (file_name, bytes) in books {
            let path = root.join(file_name);
            std::fs::write(&path, bytes).expect("a writable file");
            store.add_document(&path).expect("a readable pdf");
        }

        Self {
            _library: library,
            root,
        }
    }

    /// Opens a window the way `main` does — the shell inside a `Root`, because
    /// what a keystroke reaches depends on what is above it.
    fn open<'a>(
        &self,
        cx: &'a mut TestAppContext,
    ) -> (gpui::Entity<Pedro>, &'a mut VisualTestContext) {
        // The shell's own log, for a test that wants to see what it did.
        // `RUST_LOG=pedro=debug cargo test … -- --nocapture`.
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "warn".into()),
            )
            .try_init();

        cx.update(crate::install);

        let mut shell = None;
        let window = cx.add_window(|window, cx| {
            let pedro = cx.new(|cx| Pedro::new(window, cx));
            shell = Some(pedro.clone());

            // Named rather than inferred, the same way `main` has to name it.
            let view: gpui::AnyView = pedro.into();

            gpui_component::Root::new(view, window, cx)
        });

        let pedro = shell.expect("the shell was built");
        let cx = VisualTestContext::from_window(*window.deref(), cx).into_mut();
        // The library opens on a background thread and the books arrive after
        // it: the reader watches this happen, and a test has to wait for it.
        cx.run_until_parked();

        (pedro, cx)
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        // Safety: still under the guard.
        unsafe { std::env::remove_var("PEDRO_LIBRARY_PATH") };
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Lets the answer finish being written.
///
/// An answer is revealed over frames rather than all at once — that is what
/// makes it read as writing — so a test has to let those frames happen. The
/// harness draws only when it is asked to.
fn written_out(pedro: &gpui::Entity<Pedro>, cx: &mut VisualTestContext) {
    for _ in 0..MOST_FRAMES_AN_ANSWER_TAKES {
        let finished = pedro.read_with(cx, |pedro, _| {
            pedro.chat.as_ref().is_some_and(|chat| !chat.is_answering())
        });

        if finished {
            return;
        }

        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }

    panic!("the answer was still being written after {MOST_FRAMES_AN_ANSWER_TAKES} frames");
}

/// Enough frames for any answer a test writes, and few enough that a test which
/// will never finish says so rather than hanging.
const MOST_FRAMES_AN_ANSWER_TAKES: usize = 600;

/// A book of `pages` pages, each saying which book and which page it is.
///
/// Named on every page because the library is content-addressed: two books
/// whose bytes are identical are one book, and a fixture that forgets that
/// quietly adds one book twice under two names.
fn a_book(title: &str, pages: usize) -> Vec<u8> {
    let text: Vec<String> = (1..=pages)
        .map(|page| format!("{title} page {page}"))
        .collect();
    let pages: Vec<&str> = text.iter().map(String::as_str).collect();

    pedro_pdf::fixtures::pdf_with_pages(&pages)
}

/// The first thing the reader sees: their books, without asking for them.
#[gpui::test]
async fn a_library_with_books_in_it_shows_them(cx: &mut TestAppContext) {
    let reader = Reader::with("library", &[("one.pdf", a_book("one", 4))]);
    let (pedro, cx) = reader.open(cx);

    let books = pedro.read_with(cx, |pedro, _| {
        pedro
            .library
            .books()
            .iter()
            .map(|book| book.file_name.clone())
            .collect::<Vec<_>>()
    });

    assert_eq!(books, vec!["one.pdf".to_owned()]);
}

/// Opening a book from the sidebar puts the reader in it, at its first page.
#[gpui::test]
async fn opening_a_book_puts_the_reader_in_it(cx: &mut TestAppContext) {
    let reader = Reader::with("open", &[("one.pdf", a_book("one", 6))]);
    let (pedro, cx) = reader.open(cx);

    let book = pedro.read_with(cx, |pedro, _| pedro.library.books()[0].id.clone());
    pedro.update_in(cx, |pedro, _, cx| pedro.open_book_tab(&book, cx));
    // pdfium reads the book on a background thread.
    cx.run_until_parked();

    pedro.read_with(cx, |pedro, _| {
        let tab = pedro.active_tab().expect("a tab");
        assert_eq!(tab.id, format!("book:{book}"));

        let open = tab.document.as_ref().expect("an opened book");
        assert_eq!(open.page_count, 6);
        assert_eq!(open.page, 1);
    });
}

/// ⌘⇧S, pressed as a keystroke rather than called as a function, so the keymap
/// is part of what is being tested.
#[gpui::test]
async fn the_keyboard_turns_the_pages_two_at_a_time(cx: &mut TestAppContext) {
    let reader = Reader::with("spread-key", &[("one.pdf", a_book("one", 6))]);
    let (pedro, cx) = reader.open(cx);

    let book = pedro.read_with(cx, |pedro, _| pedro.library.books()[0].id.clone());
    pedro.update_in(cx, |pedro, _, cx| pedro.open_book_tab(&book, cx));
    cx.run_until_parked();

    assert!(!pedro.read_with(cx, |pedro, _| pedro.layout().is_spread()));

    cx.simulate_keystrokes("cmd-shift-s");
    cx.run_until_parked();
    assert!(
        pedro.read_with(cx, |pedro, _| pedro.layout().is_spread()),
        "cmd-shift-s did not reach the reader: is anything focused?"
    );

    cx.simulate_keystrokes("cmd-shift-s");
    cx.run_until_parked();
    assert!(!pedro.read_with(cx, |pedro, _| pedro.layout().is_spread()));
}

/// And it is remembered per book: a book closed in spreads opens in spreads.
#[gpui::test]
async fn how_a_book_was_read_is_how_it_opens(cx: &mut TestAppContext) {
    let reader = Reader::with("spread-kept", &[("one.pdf", a_book("one", 6))]);
    let (pedro, cx) = reader.open(cx);

    let book = pedro.read_with(cx, |pedro, _| pedro.library.books()[0].id.clone());
    pedro.update_in(cx, |pedro, _, cx| pedro.open_book_tab(&book, cx));
    cx.run_until_parked();

    cx.simulate_keystrokes("cmd-shift-s");
    cx.run_until_parked();

    // Closed and opened again, which is what the stored state is for.
    pedro.update_in(cx, |pedro, _, cx| {
        pedro.close_tab(0, cx);
        pedro.open_book_tab(&book, cx);
    });
    cx.run_until_parked();

    assert!(
        pedro.read_with(cx, |pedro, _| pedro.layout().is_spread()),
        "the book forgot how it was being read"
    );
}

/// A CLI that streams `answer` a line at a time and reports it as its result,
/// which is what the installed `claude` does.
fn a_cli_answering(name: &str, answer: &str) -> PathBuf {
    let deltas: String = answer
        .lines()
        .map(|line| {
            let text = serde_json::to_string(&format!("{line}\n")).expect("a string encodes");
            format!(
                "printf '%s\\n' '{{\"type\":\"stream_event\",\"event\":{{\"type\":\"content_block_delta\",\
                 \"delta\":{{\"type\":\"text_delta\",\"text\":{text}}}}}}}'\n"
            )
        })
        .collect();

    let result = serde_json::to_string(answer).expect("a string encodes");

    pedro_agent::fixtures::fake_cli(
        name,
        &format!(
            "{deltas}printf '%s\\n' '{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\
             \"result\":{result}}}'"
        ),
    )
}

/// The whole loop: a passage is marked, a question is typed and sent with the
/// return key, an agent answers it, and the source it names becomes a page.
#[gpui::test]
async fn a_question_about_a_passage_is_answered_and_its_source_resolved(cx: &mut TestAppContext) {
    let reader = Reader::with("asking", &[("one.pdf", a_book("one", 6))]);
    let (pedro, cx) = reader.open(cx);

    let book = pedro.read_with(cx, |pedro, _| pedro.library.books()[0].id.clone());
    pedro.update_in(cx, |pedro, _, cx| pedro.open_book_tab(&book, cx));
    cx.run_until_parked();

    let answer = "It is on the third page[1].\n\n## Sources\n[1] \"page 3\"";
    pedro.update_in(cx, |pedro, _, _| {
        pedro.agent_status = AgentStatus::Done(vec![DiscoveredAgent {
            kind: AgentKind::ClaudeCode,
            program: a_cli_answering("e2e-asking", answer),
            version: None,
        }]);

        // The passage the reader dragged across, which the composer shows as a
        // chip and the question is about.
        pedro.attached = vec![NewHighlight {
            selected_text: "one page 3".to_owned(),
            page_number: 3,
            rects: Vec::new(),
        }];
    });

    // Typed into the composer and sent with the return key, so the field's own
    // binding is part of what is being tested: ⏎ sends and ⇧⏎ breaks the line.
    pedro.update_in(cx, |pedro, window, cx| {
        pedro.composer.update(cx, |composer, cx| {
            composer.focus(window, cx);
        });
    });
    cx.simulate_input("where is it?");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();

    written_out(&pedro, cx);

    pedro.read_with(cx, |pedro, _| {
        let chat = pedro.chat.as_ref().expect("a conversation");

        assert!(!chat.is_answering(), "the answer never finished");
        assert_eq!(chat.messages.len(), 2, "{:?}", chat.messages.len());
        assert_eq!(chat.messages[0].content, "where is it?");
        assert_eq!(chat.messages[1].content, answer);

        // And the source it named is a page the reader can be sent to.
        let citation = &chat.messages[1].citations[0];
        assert_eq!(
            citation.page,
            Some(pedro_core::PageLocation::Found(3)),
            "{citation:?}"
        );

        // The panel is still in tow, because the reader never left the foot.
        assert!(pedro.chat_follows, "the answer was not being followed");
    });
}

/// A shelf is made, a book is put on it, and the sidebar says so.
#[gpui::test]
async fn a_book_can_be_put_on_a_shelf(cx: &mut TestAppContext) {
    let reader = Reader::with(
        "shelving",
        &[("one.pdf", a_book("one", 4)), ("two.pdf", a_book("two", 4))],
    );
    let (pedro, cx) = reader.open(cx);

    let first = pedro.read_with(cx, |pedro, _| pedro.library.books()[0].id.clone());

    let shelf = pedro.update_in(cx, |pedro, window, cx| {
        pedro.create_shelf(window, cx);
        pedro.library.shelves()[0].id.clone()
    });
    cx.run_until_parked();

    pedro.update_in(cx, |pedro, _, cx| {
        pedro.shelve_book(&first, Some(&shelf), cx)
    });
    cx.run_until_parked();

    pedro.read_with(cx, |pedro, cx| {
        let shelved = pedro.library.shelves();
        assert_eq!(shelved.len(), 1);
        assert_eq!(shelved[0].book_count, 1, "the shelf did not count its book");

        // And the sidebar draws it as a section of its own, with the book in it
        // and the other book still outside.
        let panel = crate::state::Panel::library(&pedro.library);
        let section = panel
            .sections
            .iter()
            .find(|section| section.shelf.as_ref().map(|id| id.as_ref()) == Some(shelf.as_str()))
            .expect("a section for the shelf");

        assert_eq!(section.entries.len(), 1);
        assert!(section.entries[0].id.ends_with(&first));

        let elsewhere: usize = panel
            .sections
            .iter()
            .filter(|section| section.shelf.is_none())
            .map(|section| section.entries.len())
            .sum();
        assert_eq!(elsewhere, 1, "the other book left the library");

        let _ = cx;
    });
}

/// Opening a shelf opens the panel it is asked in, and says so in the composer.
#[gpui::test]
async fn opening_a_shelf_opens_the_panel_it_is_asked_in(cx: &mut TestAppContext) {
    let reader = Reader::with("shelf-open", &[("one.pdf", a_book("one", 4))]);
    let (pedro, cx) = reader.open(cx);

    let book = pedro.read_with(cx, |pedro, _| pedro.library.books()[0].id.clone());
    let shelf = pedro.update_in(cx, |pedro, window, cx| {
        pedro.create_shelf(window, cx);
        let shelf = pedro.library.shelves()[0].id.clone();
        pedro.shelve_book(&book, Some(&shelf), cx);

        shelf
    });
    cx.run_until_parked();

    pedro.update_in(cx, |pedro, window, cx| {
        pedro.open_shelf(&shelf, "暗号", window, cx)
    });
    // A frame, because what the composer says follows the tab as it is drawn.
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    pedro.read_with(cx, |pedro, cx| {
        assert_eq!(
            pedro.active_tab().expect("a tab").id,
            format!("shelf:{shelf}")
        );
        assert!(pedro.chat_pane.is_open(), "the shelf opened with no panel");
        assert!(
            pedro.composer_hint.contains("these books"),
            "the composer is still asking about a document: {}",
            pedro.composer_hint
        );
        let _ = cx;
    });
}

/// A question put to a shelf is answered from the books on it, and the source
/// it names says which book as well as which page.
#[gpui::test]
async fn a_question_to_a_shelf_says_which_book_it_came_from(cx: &mut TestAppContext) {
    let reader = Reader::with(
        "shelf-asking",
        &[
            ("primes.pdf", a_book("primes", 4)),
            ("keys.pdf", a_book("keys", 4)),
        ],
    );
    let (pedro, cx) = reader.open(cx);

    // Both books onto one shelf, which is then opened.
    let shelf = pedro.update_in(cx, |pedro, window, cx| {
        pedro.create_shelf(window, cx);
        let shelf = pedro.library.shelves()[0].id.clone();

        let books: Vec<String> = pedro
            .library
            .books()
            .iter()
            .map(|book| book.id.clone())
            .collect();
        for book in books {
            pedro.shelve_book(&book, Some(&shelf), cx);
        }

        shelf
    });
    cx.run_until_parked();

    pedro.update_in(cx, |pedro, window, cx| {
        pedro.open_shelf(&shelf, "both", window, cx)
    });
    cx.run_until_parked();

    // The answer quotes the second book, which is the whole point: nothing tells
    // the reader which book that is except looking the quotation up.
    let answer = "It is in the other one[1].\n\n## Sources\n[1] \"keys page 2\"";
    pedro.update_in(cx, |pedro, _, _| {
        pedro.agent_status = AgentStatus::Done(vec![DiscoveredAgent {
            kind: AgentKind::ClaudeCode,
            program: a_cli_answering("e2e-shelf-asking", answer),
            version: None,
        }]);
    });

    pedro.update_in(cx, |pedro, window, cx| {
        pedro.composer.update(cx, |composer, cx| {
            composer.focus(window, cx);
        });
    });
    cx.simulate_input("which book?");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    written_out(&pedro, cx);

    pedro.read_with(cx, |pedro, _| {
        let chat = pedro.chat.as_ref().expect("a conversation");
        assert_eq!(chat.messages.len(), 2);

        let citation = &chat.messages[1].citations[0];
        assert_eq!(
            citation.page,
            Some(pedro_core::PageLocation::Found(2)),
            "{citation:?}"
        );
        assert_eq!(
            citation.book.as_ref().map(|book| book.title.as_str()),
            Some("keys.pdf"),
            "the citation did not say which book: {citation:?}"
        );
    });
}

/// Typing in the search box finds the passage, across books, and the sidebar
/// shows what was found instead of the list of books.
#[gpui::test]
async fn searching_shows_passages_rather_than_books(cx: &mut TestAppContext) {
    let reader = Reader::with(
        "searching",
        &[
            ("primes.pdf", a_book("primes", 4)),
            ("keys.pdf", a_book("keys", 4)),
        ],
    );
    let (pedro, cx) = reader.open(cx);

    pedro.update_in(cx, |pedro, window, cx| {
        pedro
            .search
            .update(cx, |search, cx| search.focus(window, cx));
    });
    cx.simulate_input("keys page 3");
    cx.run_until_parked();

    pedro.read_with(cx, |pedro, cx| {
        assert!(!pedro.hits.is_empty(), "the search found nothing");

        // Among the first few rather than first: two passages that differ by
        // one word are ranked apart by a margin not worth pinning a test to.
        let near_the_top = pedro.hits.iter().take(3);
        assert!(
            near_the_top
                .clone()
                .any(|hit| hit.text.contains("keys page 3")),
            "the passage was not found: {:?}",
            near_the_top.collect::<Vec<_>>()
        );

        // And the panel shows the passages rather than the books, because a
        // reader who has typed a query is looking for a passage. Asked of the
        // shell the way the sidebar asks it.
        let panel = pedro.panel();
        let rows: Vec<&str> = panel
            .sections
            .iter()
            .flat_map(|section| section.entries.iter())
            .map(|entry| entry.id.as_ref())
            .collect();

        assert!(!rows.is_empty(), "the sidebar went blank");
        assert!(
            rows.iter().all(|id| id.starts_with("hit:")),
            "the sidebar is still listing books: {rows:?}"
        );
        let _ = cx;
    });
}

/// Opens a book and hands back the shell, for the tests that only want to be
/// somewhere in a book.
async fn reading_a_book<'a>(
    name: &str,
    pages: usize,
    cx: &'a mut TestAppContext,
) -> (Reader, gpui::Entity<Pedro>, &'a mut VisualTestContext) {
    let reader = Reader::with(name, &[("one.pdf", a_book("one", pages))]);
    let (pedro, cx) = reader.open(cx);

    let book = pedro.read_with(cx, |pedro, _| pedro.library.books()[0].id.clone());
    pedro.update_in(cx, |pedro, _, cx| pedro.open_book_tab(&book, cx));
    cx.run_until_parked();

    (reader, pedro, cx)
}

fn page(pedro: &gpui::Entity<Pedro>, cx: &mut VisualTestContext) -> u32 {
    pedro.read_with(cx, |pedro, _| {
        pedro.open_document().map(|open| open.page).unwrap_or(0)
    })
}

/// The keys a reader who lives in vim reaches for.
#[gpui::test]
async fn vim_keys_turn_the_pages(cx: &mut TestAppContext) {
    let (_reader, pedro, cx) = reading_a_book("vim", 8, cx).await;
    assert_eq!(page(&pedro, cx), 1);

    cx.simulate_keystrokes("j");
    cx.run_until_parked();
    assert_eq!(page(&pedro, cx), 2, "j did not turn the page");

    cx.simulate_keystrokes("j j");
    cx.run_until_parked();
    assert_eq!(page(&pedro, cx), 4);

    cx.simulate_keystrokes("k");
    cx.run_until_parked();
    assert_eq!(page(&pedro, cx), 3);

    // The end of the book, not the last page at the top of the window: there is
    // nothing below the last page to scroll, so it can only ever be reached by
    // the window running out. What the reader is told is the page at the top of
    // the window, and at the end that is the one before the last.
    cx.simulate_keystrokes("shift-g");
    cx.run_until_parked();
    assert!(page(&pedro, cx) >= 7, "G did not go to the end");

    cx.simulate_keystrokes("home");
    cx.run_until_parked();
    assert_eq!(page(&pedro, cx), 1, "home did not go back to the start");
}

/// And the ones a reader who lives in emacs does.
#[gpui::test]
async fn emacs_keys_turn_the_pages(cx: &mut TestAppContext) {
    let (_reader, pedro, cx) = reading_a_book("emacs", 8, cx).await;

    cx.simulate_keystrokes("ctrl-n ctrl-n");
    cx.run_until_parked();
    assert_eq!(page(&pedro, cx), 3, "C-n did not turn the page");

    cx.simulate_keystrokes("ctrl-p");
    cx.run_until_parked();
    assert_eq!(page(&pedro, cx), 2);

    cx.simulate_keystrokes("ctrl-v");
    cx.run_until_parked();
    assert_eq!(page(&pedro, cx), 3);

    cx.simulate_keystrokes("alt-v");
    cx.run_until_parked();
    assert_eq!(page(&pedro, cx), 2);

    cx.simulate_keystrokes("alt-shift-.");
    cx.run_until_parked();
    assert!(page(&pedro, cx) >= 7, "M-> did not go to the end");

    cx.simulate_keystrokes("alt-shift-,");
    cx.run_until_parked();
    assert_eq!(page(&pedro, cx), 1, "M-< did not go back to the start");
}

/// The reason these are bound in the shell's own context: a reader typing a
/// question has to be able to type the letters they are bound to.
#[gpui::test]
async fn a_j_typed_into_a_question_is_a_j(cx: &mut TestAppContext) {
    let (_reader, pedro, cx) = reading_a_book("typing", 8, cx).await;

    pedro.update_in(cx, |pedro, window, cx| {
        pedro.composer.update(cx, |composer, cx| {
            composer.focus(window, cx);
        });
    });
    cx.simulate_input("just checking");
    cx.run_until_parked();

    assert_eq!(page(&pedro, cx), 1, "typing turned the pages");
    pedro.read_with(cx, |pedro, cx| {
        assert_eq!(pedro.composer.read(cx).value(), "just checking");
    });
}
