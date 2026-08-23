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

fn a_book(pages: usize) -> Vec<u8> {
    let text: Vec<String> = (1..=pages).map(|page| format!("page {page}")).collect();
    let pages: Vec<&str> = text.iter().map(String::as_str).collect();

    pedro_pdf::fixtures::pdf_with_pages(&pages)
}

/// The first thing the reader sees: their books, without asking for them.
#[gpui::test]
async fn a_library_with_books_in_it_shows_them(cx: &mut TestAppContext) {
    let reader = Reader::with("library", &[("one.pdf", a_book(4))]);
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
    let reader = Reader::with("open", &[("one.pdf", a_book(6))]);
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
    let reader = Reader::with("spread-key", &[("one.pdf", a_book(6))]);
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
    let reader = Reader::with("spread-kept", &[("one.pdf", a_book(6))]);
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
    let reader = Reader::with("asking", &[("one.pdf", a_book(6))]);
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
            selected_text: "page 3".to_owned(),
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
