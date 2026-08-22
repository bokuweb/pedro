//! The root view: application state plus the top-level layout.
//!
//! The individual regions live in [`crate::ui`], as `impl Pedro` blocks, so
//! that this file stays about state transitions rather than styling.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;

use futures::StreamExt as _;
use gpui::{
    App, AppContext as _, Bounds, Context, Entity, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, ParentElement as _, PathPromptOptions, Pixels, Point, Render, ScrollStrategy,
    SharedString, Styled as _, Window, actions, div, px,
};
use gpui_component::input::{InputEvent, InputState};
use gpui_component::{ActiveTheme as _, h_flex, v_flex};
use pedro_agent::DiscoveredAgent;
use pedro_core::chat::{Question, Subject, prepare, record, run};
use pedro_core::model::{Book, ChatMessage, Folder, Highlight, NewHighlight, ReadingState};
use pedro_core::store::Store;
use pedro_pdf::{Document, PageSize};

use crate::chat::Conversation;
use crate::document::{OpenDocument, Page, Spotlight, as_render_image};
use crate::library::{Library, SharedStore};
use crate::palette;
use crate::panes::Pane;
use crate::state::{AgentStatus, Entry, OpenTab, Panel, RailItem, ShelfMenu, Shown};
use gpui::UniformListScrollHandle;
use pedro_agent::AgentError;
use pedro_core::Conversation as CoreConversation;

actions!(
    pedro,
    [
        FocusSearch,
        ToggleSidebar,
        ToggleChat,
        NextPage,
        PreviousPage,
        NextTab,
        PreviousTab,
        ZoomIn,
        ZoomOut,
        ZoomReset
    ]
);

/// What the composer says it will ask about, which follows the tab.
const ASK_A_DOCUMENT: &str = "Ask about this document… (⏎ to send, ⇧⏎ for a line)";
const ASK_A_SHELF: &str = "Ask across these books… (⏎ to send, ⇧⏎ for a line)";

/// How tall a page is drawn at 100%, in logical pixels.
///
/// A page fills a comfortable window at this size. It is not fitted to the
/// window, because fitting means re-rasterising every page on every resize;
/// the zoom is how a reader who wants a different size asks for one.
pub(crate) const PAGE_HEIGHT: f32 = 640.;

/// The command that would fix a failure, when the failure is a CLI that is
/// installed but signed out.
fn sign_in_command(err: &pedro_core::chat::ChatError) -> Option<&'static str> {
    match err {
        pedro_core::chat::ChatError::Agent(AgentError::NotSignedIn { command, .. }) => {
            Some(command)
        }
        _ => None,
    }
}

/// What zoom can be set to. A book is read at one of a few sizes, and a
/// continuous zoom would rasterise a new page for every step of the way.
const ZOOM_STEPS: [f32; 7] = [0.6, 0.8, 1.0, 1.25, 1.5, 2.0, 3.0];

pub struct Pedro {
    focus_handle: FocusHandle,
    pub(crate) search: Entity<InputState>,
    /// Where a question about the open document is typed.
    pub(crate) composer: Entity<InputState>,
    /// Whether the agent may search the web, chatbook's toggle. Per question
    /// rather than per install, so it lives beside the field.
    pub(crate) web_search: bool,
    pub(crate) active_rail: RailItem,
    /// The sections the reader has shut. Panels are rebuilt every frame, so
    /// what survives between them lives here.
    pub(crate) collapsed: HashSet<(RailItem, usize)>,
    pub(crate) tabs: Vec<OpenTab>,
    pub(crate) active_tab: Option<usize>,
    pub(crate) agent_status: AgentStatus,
    pub(crate) library: Library,
    /// The conversation the chat panel is showing, if one is open.
    pub(crate) chat: Option<Conversation>,
    /// The panel of books, and the panel of the conversation.
    pub(crate) sidebar: Pane,
    pub(crate) chat_pane: Pane,

    /// Whether the sidebar is showing the places that are settings rather than
    /// reading.
    pub(crate) show_secondary: bool,
    /// Which installed CLI answers a question. `None` means whichever was found
    /// first, which is what a reader with one installed never has to think
    /// about.
    pub(crate) answering: Option<pedro_agent::AgentKind>,
    /// How large a page is drawn, as a multiple of [`PAGE_HEIGHT`].
    pub(crate) zoom: f32,
    /// The row whose remove button has been pressed once.
    ///
    /// Removing a book takes its highlights and conversations with it, so it
    /// asks twice. A second press on the same row does it; a press anywhere
    /// else changes its mind.
    pub(crate) confirming_removal: Option<SharedString>,
    /// The book whose shelf menu is open, and where to draw it.
    ///
    /// Hand-rolled rather than a popup component, because every other
    /// affordance in this panel is — the remove button asks its question in
    /// place — and because the menu's items are shelves, which are data rather
    /// than actions to register.
    pub(crate) shelf_menu: Option<ShelfMenu>,
    /// Where each page was drawn, recorded while the frame is laid out. A drag
    /// arrives in window coordinates and a page needs it as a fraction of
    /// itself.
    ///
    /// A cell rather than plain state: it is written during layout, and asking
    /// the view to change then would ask for another frame, every frame.
    pub(crate) page_bounds: Rc<RefCell<HashMap<u32, Bounds<Pixels>>>>,
    /// Whether the pointer is down on the page, dragging out a passage.
    pub(crate) selecting: bool,
    /// The passages the next question is about, in the order they were marked.
    ///
    /// A question can be about two places in a book at once — "how does this
    /// square with that" is the reason to have a reader that can be asked
    /// anything — so marking a passage adds to this rather than replacing it.
    pub(crate) attached: Vec<NewHighlight>,
    /// What a search across the books found, and what it was looking for.
    ///
    /// The query is kept beside the hits because a result that arrives after
    /// the reader has typed more is a result for a question nobody is asking
    /// any more.
    pub(crate) hits: Vec<pedro_search::Hit>,
    pub(crate) searched_for: String,
    /// The passage a book being opened should land on, when it was opened by
    /// something that knows where it wants to go — a search hit, which names a
    /// page and the words that were found on it.
    open_at: Option<Spotlight>,
    /// The last thing that went wrong where the reader was looking. Shown in
    /// the sidebar rather than as a notification: a file that could not be
    /// added is about the list it is missing from.
    pub(crate) notice: Option<SharedString>,
    /// Set on mouse-down in the title strip so the next drag moves the window.
    pub(crate) window_drag_armed: bool,

    /// Where a Google Drive link is pasted, and whether that row is showing.
    ///
    /// Hidden until it is asked for: most books come off the disk, and a field
    /// for the ones that do not should not cost a row of the panel to every
    /// reader who never uses it.
    pub(crate) drive: Entity<InputState>,
    pub(crate) drive_open: bool,
    /// Whether a fetch is in flight. One at a time: the first thing a fetch
    /// may do is open a browser, and two of those at once is not a sign-in.
    pub(crate) drive_busy: bool,
    /// Where the open shelf's name is edited.
    pub(crate) shelf_name: Entity<InputState>,
    /// What the composer is currently telling the reader it will ask about.
    ///
    /// Kept here so that following the tab costs one comparison a frame rather
    /// than a window handle at every place a tab can change — several of which,
    /// like opening the last-read book at startup, have no window to give.
    composer_hint: &'static str,
}

impl Pedro {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search your books"));
        // Re-render as the query changes, and go and look for it.
        cx.observe(&search, |this: &mut Self, _, cx| {
            this.search_books(cx);
            cx.notify();
        })
        .detach();

        // Grows with the question rather than scrolling inside two lines: a
        // passage quoted back at the agent is easily a paragraph.
        let composer = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .auto_grow(1, 6)
                .placeholder(ASK_A_DOCUMENT)
        });
        cx.subscribe_in(&composer, window, |this, _, event, window, cx| {
            // Enter is bound to the secondary variant in `main`, which is what
            // tells it apart from the shift-enter that only breaks the line.
            if matches!(event, InputEvent::PressEnter { secondary: true }) {
                this.ask(window, cx);
            }
        })
        .detach();

        let drive = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Paste a Google Drive link and press ⏎")
        });
        cx.subscribe_in(&drive, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.add_from_drive(window, cx);
            }
        })
        .detach();

        let agent_status = AgentStatus::Detecting;
        let library = Library::Opening;

        // A shelf is named after it is made, so the field it is named in is
        // part of the shelf's own view rather than a dialog in front of it.
        let shelf_name = cx.new(|cx| InputState::new(window, cx).placeholder("Name this shelf"));
        cx.subscribe_in(&shelf_name, window, |this, _, event, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.commit_shelf_name(cx);
            }
        })
        .detach();

        Self::detect_agents(cx);
        Self::open_library(cx);

        Self {
            focus_handle: cx.focus_handle(),
            search,
            composer,
            shelf_name,
            composer_hint: ASK_A_DOCUMENT,
            web_search: true,
            active_rail: RailItem::Library,
            collapsed: HashSet::new(),
            tabs: Vec::new(),
            active_tab: None,
            agent_status,
            library,
            chat: None,
            sidebar: Pane::open(crate::ui::SIDEBAR_WIDTH),
            chat_pane: Pane::shut(crate::ui::CHAT_WIDTH),
            show_secondary: false,
            answering: None,
            zoom: 1.0,
            confirming_removal: None,
            shelf_menu: None,
            page_bounds: Rc::new(RefCell::new(HashMap::new())),
            selecting: false,
            attached: Vec::new(),
            hits: Vec::new(),
            searched_for: String::new(),
            open_at: None,
            notice: None,
            window_drag_armed: false,
            drive,
            drive_open: false,
            drive_busy: false,
        }
    }

    /// Looks for installed agent CLIs off the UI thread.
    fn detect_agents(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let agents = cx
                .background_executor()
                .spawn(async { pedro_agent::discover() })
                .await;

            this.update(cx, |this, cx| this.agents_detected(agents, cx))
                .ok();
        })
        .detach();
    }

    fn agents_detected(&mut self, agents: Vec<DiscoveredAgent>, cx: &mut Context<Self>) {
        self.agent_status = AgentStatus::Done(agents);
        cx.notify();
    }

    /// Opens the library off the UI thread: it touches the disk, and on a first
    /// run it creates the database.
    fn open_library(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let opened = cx
                .background_executor()
                .spawn(async {
                    // Errors are flattened to strings here rather than carried
                    // out of the thread: what the shell does with any of them
                    // is the same, and it saves demanding `Send` of every error
                    // type in two other crates.
                    let store = Store::open_default().map_err(|err| err.to_string())?;
                    let books = store.books().map_err(|err| err.to_string())?;
                    let shelves = store.folders().map_err(|err| err.to_string())?;
                    Ok::<_, String>((store, books, shelves))
                })
                .await;

            this.update(cx, |this, cx| this.library_opened(opened, cx))
                .ok();
        })
        .detach();
    }

    fn library_opened(
        &mut self,
        opened: Result<(Store, Vec<Book>, Vec<Folder>), String>,
        cx: &mut Context<Self>,
    ) {
        self.library = match opened {
            Ok((store, books, shelves)) => {
                tracing::info!(
                    books = books.len(),
                    shelves = shelves.len(),
                    "opened the library"
                );
                Library::Ready {
                    root: store.root().to_path_buf(),
                    store: SharedStore::new(store),
                    books,
                    shelves,
                }
            }
            Err(why) => {
                tracing::error!(why, "could not open the library");
                Library::Failed(why.into())
            }
        };

        self.reopen_last_book(cx);
        self.index_missing_books(cx);
        cx.notify();
    }

    /// Indexes the books that were added before there was an index.
    ///
    /// After the library is handed over rather than before it, so that the
    /// shelf is on screen while this happens: a book of five hundred pages is
    /// a few thousand passages to cut and write.
    fn index_missing_books(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.library.store().cloned() else {
            return;
        };

        cx.background_executor()
            .spawn(async move {
                match store.lock().index_missing() {
                    Ok(0) => {}
                    Ok(done) => tracing::info!(books = done, "indexed books added earlier"),
                    Err(err) => tracing::warn!(?err, "could not index the older books"),
                }
            })
            .detach();
    }

    /// Opens the book the reader was last in.
    ///
    /// A reader who closed the window in the middle of a chapter meant to come
    /// back to it, and an empty reader on every launch makes them find their
    /// way there again.
    fn reopen_last_book(&mut self, cx: &mut Context<Self>) {
        if !self.tabs.is_empty() {
            return;
        }

        // `books` is ordered by when each was last touched, so the first with a
        // place saved is the one that was being read.
        let Some(book) = self
            .library
            .books()
            .iter()
            .find(|book| book.reading.is_some())
            .cloned()
        else {
            return;
        };

        self.open_entry(
            &Entry::opening(format!("book:{}", book.id), crate::library::title_of(&book)),
            cx,
        );
    }

    /// Asks for PDFs and adds whatever comes back.
    pub(crate) fn pick_documents(&mut self, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Add".into()),
        });

        cx.spawn(async move |this, cx| {
            // A cancelled prompt and a failed one both mean nothing was chosen.
            let Ok(Ok(Some(paths))) = chosen.await else {
                return;
            };

            this.update(cx, |this, cx| this.add_documents(paths, cx))
                .ok();
        })
        .detach();
    }

    /// Adds documents to the library and reloads the list.
    ///
    /// Reading a book takes as long as extracting the text of every page, so
    /// this runs in the background and the list simply arrives when it is done.
    pub(crate) fn add_documents(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let Some(store) = self.library.store().cloned() else {
            return;
        };

        self.notice = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let added = cx
                .background_executor()
                .spawn(async move {
                    let mut store = store.lock();
                    // One bad file does not cost the reader the others: it is
                    // reported, and the rest are still added.
                    let mut failures = Vec::new();
                    for path in paths {
                        if let Err(err) = store.add_document(&path) {
                            failures.push(format!("{}: {err}", path.display()));
                        }
                    }

                    store
                        .books()
                        .map_err(|err| err.to_string())
                        .map(|books| (books, failures))
                })
                .await;

            this.update(cx, |this, cx| this.documents_added(added, cx))
                .ok();
        })
        .detach();
    }

    /// Shows or hides the row a Google Drive link is pasted into.
    pub(crate) fn toggle_drive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.drive_open = !self.drive_open;

        if self.drive_open {
            self.drive.update(cx, |drive, cx| drive.focus(window, cx));
        }

        cx.notify();
    }

    /// Fetches whatever the Drive field names, and adds it to the library.
    ///
    /// The whole of it — the sign-in, the download, reading the document —
    /// happens on a background thread, because the first fetch of a session
    /// waits for a browser window the reader has to come back from.
    pub(crate) fn add_from_drive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.drive_busy {
            return;
        }

        let link = self.drive.read(cx).value().trim().to_owned();
        if link.is_empty() {
            return;
        }

        let Some(store) = self.library.store().cloned() else {
            return;
        };

        // Nothing can be fetched without a client id, and saying so before the
        // browser opens is the only place the reader can act on it.
        let Some(credentials) = pedro_drive::Credentials::from_env() else {
            self.notice = Some(pedro_drive::DriveError::NotConfigured.to_string().into());
            cx.notify();
            return;
        };

        self.drive_busy = true;
        // Whether this needs a browser is a question for the keychain, and the
        // keychain is not something to ask on the thread drawing frames — it
        // can put a dialog of its own up first. One message covers both.
        self.notice = Some("Fetching from Google Drive… (a browser may open to sign in)".into());
        self.drive
            .update(cx, |drive, cx| drive.set_value("", window, cx));
        cx.notify();

        cx.spawn(async move |this, cx| {
            let added = cx
                .background_executor()
                .spawn(async move {
                    let directory = pedro_drive::scratch();
                    let mut failures = Vec::new();

                    match pedro_drive::fetch(&credentials, &link, &directory) {
                        Ok(fetched) => {
                            let mut store = store.lock();
                            if let Err(err) = store.add_document(&fetched.path) {
                                failures.push(format!("{}: {err}", fetched.name));
                            }
                        }
                        Err(err) => failures.push(err.to_string()),
                    }

                    // The library has taken its own copy under a content hash
                    // by now, so what was fetched is nothing but a temporary
                    // file in the way.
                    let _ = std::fs::remove_dir_all(&directory);

                    store
                        .lock()
                        .books()
                        .map_err(|err| err.to_string())
                        .map(|books| (books, failures))
                })
                .await;

            this.update(cx, |this, cx| {
                this.drive_busy = false;
                this.documents_added(added, cx);
            })
            .ok();
        })
        .detach();
    }

    fn documents_added(
        &mut self,
        added: Result<(Vec<Book>, Vec<String>), String>,
        cx: &mut Context<Self>,
    ) {
        match added {
            Ok((books, failures)) => {
                if let Library::Ready { books: held, .. } = &mut self.library {
                    *held = books;
                }
                self.notice = failures.first().map(|failure| {
                    tracing::warn!(failure, "could not add a document");
                    failure.clone().into()
                });
            }
            Err(why) => {
                tracing::error!(why, "could not reload the library");
                self.notice = Some(why.into());
            }
        }

        cx.notify();
    }

    /// Rereads the books and the shelves after something moved between them.
    ///
    /// Both together: a book joining a shelf changes the shelf's count as well
    /// as the book's own row, and reading one without the other draws a shelf
    /// that says two and holds three.
    fn reload_shelves(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.library.store() else {
            return;
        };

        let read = {
            let store = store.lock();
            store.books().and_then(|books| {
                let shelves = store.folders()?;
                Ok((books, shelves))
            })
        };

        match read {
            Ok((books, shelves)) => {
                if let Library::Ready {
                    books: held,
                    shelves: on,
                    ..
                } = &mut self.library
                {
                    *held = books;
                    *on = shelves;
                }
            }
            Err(err) => {
                tracing::warn!(%err, "could not reread the shelves");
                self.notice = Some(err.to_string().into());
            }
        }

        cx.notify();
    }

    /// Makes a shelf and opens it, so the reader lands somewhere they can name
    /// it and put something on it rather than on a row that has just appeared.
    pub(crate) fn create_shelf(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(store) = self.library.store() else {
            return;
        };

        // Numbered rather than blank: an untitled row among untitled rows is
        // not a thing the reader can point at.
        let taken = self.library.shelves().len();
        let name = format!("Shelf {}", taken + 1);

        let made = store.lock().create_folder(&name);
        match made {
            Ok(shelf) => {
                self.reload_shelves(cx);
                self.open_shelf(&shelf.id, &shelf.name, window, cx);
            }
            Err(err) => {
                tracing::warn!(%err, "could not make a shelf");
                self.notice = Some(err.to_string().into());
                cx.notify();
            }
        }
    }

    pub(crate) fn rename_shelf(&mut self, shelf_id: &str, name: &str, cx: &mut Context<Self>) {
        let Some(store) = self.library.store() else {
            return;
        };
        if name.trim().is_empty() {
            return;
        }

        if let Err(err) = store.lock().rename_folder(shelf_id, name) {
            tracing::warn!(%err, "could not rename a shelf");
            self.notice = Some(err.to_string().into());
            cx.notify();
            return;
        }

        // The open tab carries the old name in its title.
        let tab_id = format!("shelf:{shelf_id}");
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) {
            tab.label = name.trim().to_owned().into();
        }

        self.reload_shelves(cx);
    }

    /// Removes a shelf, and closes it if it was open. The books stay.
    pub(crate) fn delete_shelf(&mut self, shelf_id: &str, cx: &mut Context<Self>) {
        let Some(store) = self.library.store() else {
            return;
        };

        if let Err(err) = store.lock().remove_folder(shelf_id) {
            tracing::warn!(%err, "could not remove a shelf");
            self.notice = Some(err.to_string().into());
            cx.notify();
            return;
        }

        let tab_id = format!("shelf:{shelf_id}");
        if let Some(index) = self.tabs.iter().position(|tab| tab.id == tab_id) {
            self.close_tab(index, cx);
        }

        self.reload_shelves(cx);
    }

    /// Puts a book on a shelf, or takes it off one with `None`.
    pub(crate) fn shelve_book(
        &mut self,
        book_id: &str,
        shelf_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let Some(store) = self.library.store() else {
            return;
        };

        if let Err(err) = store.lock().move_book(book_id, shelf_id) {
            tracing::warn!(%err, "could not shelve a book");
            self.notice = Some(err.to_string().into());
            cx.notify();
            return;
        }

        self.reload_shelves(cx);
    }

    /// Opens a shelf as a tab of its own, which is where it is asked about.
    pub(crate) fn open_shelf(
        &mut self,
        shelf_id: &str,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tab_id = format!("shelf:{shelf_id}");
        let existing = self.tabs.iter().position(|tab| tab.id == tab_id);

        self.active_tab = Some(existing.unwrap_or_else(|| {
            self.tabs.push(OpenTab::new(tab_id, name.to_owned()));
            self.tabs.len() - 1
        }));

        let name = name.to_owned();
        self.shelf_name
            .update(cx, |field, cx| field.set_value(&name, window, cx));

        // A shelf is a thing to ask, so the panel it is asked in opens with it
        // rather than waiting to be found.
        self.chat_pane.set_open(true);
        self.open_shelf_conversation(shelf_id, &name, cx);
        cx.notify();
    }

    /// Renames the open shelf to whatever is in the field.
    pub(crate) fn commit_shelf_name(&mut self, cx: &mut Context<Self>) {
        let Some(shelf) = self.active_shelf().cloned() else {
            return;
        };

        let typed = self.shelf_name.read(cx).value().trim().to_string();
        if typed.is_empty() || typed == shelf.name {
            return;
        }

        self.rename_shelf(&shelf.id, &typed, cx);
    }

    /// The shelf a tab is showing, when the active tab is a shelf.
    pub(crate) fn active_shelf(&self) -> Option<&Folder> {
        let id = self.active_tab()?.id.strip_prefix("shelf:")?;

        self.library.shelves().iter().find(|shelf| shelf.id == id)
    }

    /// Loads what has already been said to a shelf into the chat panel.
    fn open_shelf_conversation(&mut self, shelf_id: &str, name: &str, cx: &mut Context<Self>) {
        let stored = self.library.store().map(|store| {
            store
                .lock()
                .messages(&CoreConversation::Folder(shelf_id.to_owned()))
        });

        let mut chat = Conversation::about(name.to_owned(), 0);
        chat.about = Some(CoreConversation::Folder(shelf_id.to_owned()));
        match stored {
            Some(Ok(messages)) => chat.messages = messages,
            Some(Err(err)) => tracing::warn!(%err, "could not read a shelf conversation"),
            None => {}
        }

        self.chat = Some(chat);
        cx.notify();
    }

    /// Looks for what is in the search box, across every book.
    ///
    /// One query per change of the box rather than one per keystroke debounced:
    /// the index answers in well under a frame, and a result that lands after
    /// the reader has typed more is thrown away by comparing the query it was
    /// for.
    fn search_books(&mut self, cx: &mut Context<Self>) {
        let query = self.search_query(cx);
        if query == self.searched_for {
            return;
        }

        self.searched_for = query.clone();
        if query.is_empty() {
            self.hits.clear();
            return;
        }

        let Some(store) = self.library.store().cloned() else {
            return;
        };

        cx.spawn(async move |this, cx| {
            let asked = query.clone();
            let found = cx
                .background_executor()
                .spawn(async move { store.lock().search(&query).map_err(|err| err.to_string()) })
                .await;

            this.update(cx, |this, cx| {
                // The reader has typed more since; this answer is stale.
                if this.searched_for != asked {
                    return;
                }

                match found {
                    Ok(hits) => this.hits = hits,
                    Err(why) => tracing::warn!(why, "the search failed"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The panel for wherever the reader is, built from what is on screen now.
    pub(crate) fn panel(&self) -> Panel {
        let open = self.open_document();
        let book = self.active_tab().and_then(|tab| {
            let id = tab.id.strip_prefix("book:")?;
            self.library.books().iter().find(|book| book.id == id)
        });

        Panel::for_rail_item(
            self.active_rail,
            &Shown {
                library: &self.library,
                status: &self.agent_status,
                outline: book.map(|book| book.outline.as_slice()).unwrap_or(&[]),
                page: open.map_or(1, |open| open.page),
                highlights: open.map(|open| open.highlights.as_slice()).unwrap_or(&[]),
                chat: self.chat.as_ref(),
                answering: self.answering_kind(),
                query: &self.searched_for,
                hits: &self.hits,
                library_path: self.library.path(),
                zoom: self.zoom,
            },
        )
    }

    /// Whether a section of the current panel is open.
    pub(crate) fn is_expanded(&self, index: usize) -> bool {
        !self.collapsed.contains(&(self.active_rail, index))
    }

    pub(crate) fn search_query(&self, cx: &App) -> String {
        self.search.read(cx).value().trim().to_string()
    }

    pub(crate) fn select_rail(&mut self, item: RailItem, cx: &mut Context<Self>) {
        if self.active_rail != item {
            self.active_rail = item;
            cx.notify();
        }
    }

    pub(crate) fn toggle_section(&mut self, index: usize, cx: &mut Context<Self>) {
        let section = (self.active_rail, index);
        if !self.collapsed.remove(&section) {
            self.collapsed.insert(section);
        }

        cx.notify();
    }

    /// Acts on a sidebar row: a book opens as a tab, a chapter turns to its
    /// page, a marked passage reopens the conversation about it.
    pub(crate) fn open_entry(&mut self, entry: &Entry, cx: &mut Context<Self>) {
        // Pressing the row rather than its remove button is an answer too.
        if self.confirming_removal.is_some() {
            self.confirming_removal = None;
            cx.notify();
        }
        self.close_shelf_menu(cx);

        if !entry.openable {
            return;
        }

        if let Some(page) = entry.id.strip_prefix("page:") {
            if let Ok(page) = page.parse() {
                self.show_page(page, cx);
            }
            return;
        }

        if let Some(highlight_id) = entry.id.strip_prefix("highlight:") {
            self.open_marked_passage(highlight_id, cx);
            return;
        }

        if let Some(hit) = entry.id.strip_prefix("hit:") {
            // `hit:{index}:{book}:{page}` (see `Panel::found`). The place is
            // read out of the row's own name so that it is right whatever the
            // search has done since; the index is only how the passage itself
            // is fetched back, and is checked against the place before it is
            // believed.
            if let Some((index, rest)) = hit.split_once(':')
                && let Some((book_id, page)) = rest.rsplit_once(':')
                && let Ok(page) = page.parse::<u32>()
            {
                let passage = index
                    .parse::<usize>()
                    .ok()
                    .and_then(|index| self.hits.get(index))
                    .filter(|hit| hit.book_id == book_id && hit.page_number == page)
                    .map(|hit| hit.text.clone());

                self.open_found(book_id, page, passage, cx);
            }
            return;
        }

        if let Some(program) = entry.id.strip_prefix("agent:") {
            self.choose_agent(program, cx);
            return;
        }

        let existing = self.tabs.iter().position(|tab| tab.id == entry.id);
        self.active_tab = Some(existing.unwrap_or_else(|| {
            self.tabs
                .push(OpenTab::new(entry.id.clone(), entry.label.clone()));
            self.tabs.len() - 1
        }));

        // Already open means already read: only a new tab has a book to load.
        if existing.is_none()
            && let Some(book_id) = entry.id.strip_prefix("book:")
        {
            self.load_book(book_id.to_owned(), cx);
        }

        cx.notify();
    }

    /// Reads a book off disk and rasterises its first page.
    fn load_book(&mut self, book_id: String, cx: &mut Context<Self>) {
        let Some(store) = self.library.store() else {
            return;
        };
        let Some(book) = self
            .library
            .books()
            .iter()
            .find(|book| book.id == book_id)
            .cloned()
        else {
            return;
        };

        let store = store.clone();
        let page = book.reading.as_ref().map_or(1, |reading| reading.page);
        let tab_id: SharedString = format!("book:{book_id}").into();

        cx.spawn(async move |this, cx| {
            let opened = cx
                .background_executor()
                .spawn(async move {
                    // Locked here rather than on the UI thread: an answer being
                    // written holds this same lock for as long as the agent
                    // takes, and the window must not wait on that.
                    let (path, highlights) = {
                        let store = store.lock();
                        let path = store.document_path(&book);
                        let highlights =
                            store.highlights(&book.id).map_err(|err| err.to_string())?;

                        (path, highlights)
                    };

                    let document = Document::open(&path).map_err(|err| err.to_string())?;
                    // Every page is laid out against the first one's size, so
                    // it is read here rather than on the UI thread later.
                    let size = document.page_size(0).map_err(|err| err.to_string())?;

                    // A book stored with no table of contents may simply have
                    // been read by a pedro that could not see the one it has.
                    if book.outline.is_empty() {
                        let outline = document.outline();
                        if !outline.is_empty()
                            && let Err(err) = store.lock().set_outline(&book.id, &outline)
                        {
                            tracing::warn!(?err, "could not store a recovered outline");
                        }
                    }

                    Ok::<_, String>((document, size, highlights))
                })
                .await;

            this.update(cx, |this, cx| this.book_loaded(&tab_id, opened, page, cx))
                .ok();
        })
        .detach();
    }

    fn book_loaded(
        &mut self,
        tab_id: &str,
        opened: Result<(Document, PageSize, Vec<Highlight>), String>,
        page: u32,
        cx: &mut Context<Self>,
    ) {
        // The reader may have closed the tab while the book was being read.
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
            return;
        };

        match opened {
            Ok((document, size, highlights)) => {
                tab.error = None;
                let mut open = OpenDocument::new(document, size, page);
                open.highlights = highlights;
                tab.document = Some(open);

                // Where it was asked to go, or where the reader left off.
                let target = self.open_at.take();
                let page = target.as_ref().map_or(page, |target| target.page);
                if let Some(open) = &mut tab.document {
                    open.page = page.clamp(1, open.page_count.max(1));
                    // Pointed at now, found later: the page it is on is several
                    // trips through pdfium away.
                    if let Some(target) = target {
                        open.spotlight(target);
                    }
                }

                // The list holds the position now, so continuing where the
                // reader left off is a scroll rather than a page number.
                tab.scroll
                    .scroll_to_item(page as usize - 1, ScrollStrategy::Top);
            }
            Err(why) => {
                tracing::error!(why, tab_id, "could not open the book");
                tab.error = Some(why.into());
            }
        }

        cx.notify();
    }

    /// Rasterises the pages the list is about to draw.
    ///
    /// Called from the list's own item builder, which is the one place that
    /// knows what is on screen; the page at the top of the range is also the
    /// page the reader is on, which is what a question quotes and what the
    /// stored place points at.
    pub(crate) fn pages_in_view(&mut self, range: &Range<usize>, cx: &mut Context<Self>) {
        let Some(tab) = self.active_tab else {
            return;
        };
        let Some(open) = self.tabs.get_mut(tab).and_then(|tab| tab.document.as_mut()) else {
            return;
        };

        let top = range.start as u32 + 1;
        let moved = open.page != top;
        open.page = top;

        // One page beyond the range in each direction, so scrolling by a page
        // never waits for pdfium.
        let first = top.saturating_sub(1).max(1);
        let last = (range.end as u32 + 1).min(open.page_count);

        let wanted: Vec<u32> = (first..=last).filter(|page| open.wants(*page)).collect();
        for page in &wanted {
            open.requested.insert(*page);
        }

        let tab_id = self.tabs[tab].id.clone();
        for page in wanted {
            self.rasterise(tab_id.clone(), page, cx);
        }

        if moved {
            tracing::debug!(
                top,
                drawn_at = ?self.page_bounds.borrow().get(&top),
                "the page in view changed"
            );
            self.save_reading_position(cx);
        }
    }

    /// Asks pdfium for one page, and for the text on it.
    fn rasterise(&mut self, tab_id: SharedString, page: u32, cx: &mut Context<Self>) {
        let Some(open) = self.open_document() else {
            return;
        };
        let document = open.document.clone();
        let scale = open.scale_for(self.page_height());
        let generation = open.generation;

        cx.spawn(async move |this, cx| {
            let rendered = cx
                .background_executor()
                .spawn(async move {
                    // The pixels and the text of a page are wanted at the same
                    // moment and cost one trip into pdfium each, so they are
                    // fetched together rather than as two round trips through
                    // the UI thread.
                    let image = document
                        .render_page(page - 1, scale)
                        .map_err(|err| err.to_string())?;
                    let text = document
                        .page_text(page - 1)
                        .map_err(|err| err.to_string())?;
                    let size = document
                        .page_size(page - 1)
                        .map_err(|err| err.to_string())?;

                    Ok::<_, String>((image, text, size))
                })
                .await;

            this.update(cx, |this, cx| {
                this.page_rendered(&tab_id, page, generation, rendered, cx)
            })
            .ok();
        })
        .detach();
    }

    fn page_rendered(
        &mut self,
        tab_id: &str,
        page: u32,
        generation: u64,
        rendered: Result<
            (
                pedro_pdf::PageImage,
                pedro_pdf::PageText,
                pedro_pdf::PageSize,
            ),
            String,
        >,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
            return;
        };
        let Some(open) = &mut tab.document else {
            return;
        };

        // Rasterised for a size the reader has since changed.
        if open.generation != generation {
            return;
        }

        match rendered {
            Ok((image, text, size)) => {
                open.requested.remove(&page);
                if let Some(image) = as_render_image(image) {
                    open.store(page, Page { image, size, text });
                }
            }
            Err(why) => {
                open.requested.remove(&page);
                tracing::error!(why, page, "could not render a page");
                tab.error = Some(why.into());
            }
        }

        cx.notify();
    }

    /// Where a window-space point falls on `page`, as fractions of it.
    fn on_the_page(&self, page: u32, position: Point<Pixels>) -> Option<(f32, f32)> {
        let bounds = *self.page_bounds.borrow().get(&page)?;
        let (width, height) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        if width <= 0.0 || height <= 0.0 {
            return None;
        }

        Some((
            (f32::from(position.x) - f32::from(bounds.origin.x)) / width,
            (f32::from(position.y) - f32::from(bounds.origin.y)) / height,
        ))
    }

    fn document_mut(&mut self) -> Option<&mut OpenDocument> {
        self.active_tab
            .and_then(|index| self.tabs.get_mut(index))
            .and_then(|tab| tab.document.as_mut())
    }

    pub(crate) fn begin_selection(
        &mut self,
        page: u32,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some((x, y)) = self.on_the_page(page, position) else {
            tracing::debug!(page, ?position, known = ?self.page_bounds.borrow().keys().collect::<Vec<_>>(), "a press landed on no page");
            return;
        };
        tracing::debug!(page, x, y, "press");

        // A reader touching the page has found what they were sent to.
        if let Some(open) = self.document_mut() {
            open.clear_spotlight();
        }

        // A passage already marked is a conversation, not a place to start a
        // new selection: pressing on one reopens what was said about it.
        if let Some(highlight) = self
            .open_document()
            .and_then(|open| open.highlight_at(page, x, y))
            .cloned()
        {
            self.open_highlight(&highlight, cx);
            return;
        }

        let Some(open) = self.document_mut() else {
            return;
        };

        open.begin_selection(page, x, y);
        tracing::debug!(
            page,
            characters = open.page(page).map(|held| held.chars_len()),
            selection = ?open.selection,
            "selection started"
        );
        self.selecting = true;
        cx.notify();
    }

    pub(crate) fn extend_selection(
        &mut self,
        page: u32,
        held: Option<gpui::MouseButton>,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if !self.selecting {
            return;
        }

        // A release outside the page never reaches the page's own handler, so
        // the first move afterwards is what tells us the drag is over. Without
        // this the selection keeps growing under a button nobody is holding.
        if held != Some(gpui::MouseButton::Left) {
            self.finish_selection(cx);
            return;
        }
        let Some((x, y)) = self.on_the_page(page, position) else {
            return;
        };
        let Some(open) = self.document_mut() else {
            return;
        };

        open.extend_selection(page, x, y);
        cx.notify();
    }

    pub(crate) fn finish_selection(&mut self, cx: &mut Context<Self>) {
        if !self.selecting {
            return;
        }

        self.selecting = false;

        // A marked passage joins the question being written, and becomes what
        // the panel is about. The one exception is a panel busy writing an
        // answer, which is not a place to put something else — and sending is
        // refused while it is anyway.
        let busy = self.chat.as_ref().is_some_and(Conversation::is_answering);
        if let Some(passage) = self.marked_passage().filter(|_| !busy) {
            let subject = passage.selected_text.clone();
            let page = passage.page_number;

            // Marking the same words twice is a reader adjusting a drag, not
            // asking about the passage twice.
            self.attached
                .retain(|held| held.selected_text != passage.selected_text);
            self.attached.push(passage);

            if self
                .chat
                .as_ref()
                .is_none_or(|chat| chat.messages.is_empty())
            {
                self.chat = Some(Conversation::about(subject, page));
            }
        }

        cx.notify();
    }

    /// Puts one attached passage down.
    ///
    /// When it was the last one, the conversation it opened goes with it —
    /// unless something has been said in it, since an answer already given is
    /// not something a passing drag should be able to close.
    pub(crate) fn detach_passage(&mut self, at: usize, cx: &mut Context<Self>) {
        if at < self.attached.len() {
            self.attached.remove(at);
        }

        if let Some(open) = self.document_mut() {
            open.selection = None;
        }

        if self.attached.is_empty()
            && self
                .chat
                .as_ref()
                .is_some_and(|chat| chat.messages.is_empty() && !chat.is_answering())
        {
            self.chat = None;
        }

        cx.notify();
    }

    /// Moves `by` pages and draws what that lands on.
    pub(crate) fn turn_page(&mut self, by: i64, cx: &mut Context<Self>) {
        let Some(open) = self
            .active_tab
            .and_then(|index| self.tabs.get_mut(index))
            .and_then(|tab| tab.document.as_mut())
        else {
            return;
        };

        if let Some(page) = open.turn(by) {
            // The list is what holds the position, so a page turn is a scroll.
            if let Some(scroll) = self.page_scroll() {
                scroll.scroll_to_item(page as usize - 1, ScrollStrategy::Top);
            }
            cx.notify();
        }
    }

    /// Writes down where the reader is, so the book opens here next time.
    ///
    /// Fire and forget: a place that fails to save is worth a log line and
    /// nothing more, since the reader is still looking at the page.
    fn save_reading_position(&self, cx: &mut Context<Self>) {
        let Some(store) = self.library.store().cloned() else {
            return;
        };
        let Some(tab) = self.active_tab() else {
            return;
        };
        let Some(book_id) = tab.id.strip_prefix("book:").map(str::to_owned) else {
            return;
        };
        let Some(page) = tab.document.as_ref().map(|open| open.page) else {
            return;
        };

        cx.background_executor()
            .spawn(async move {
                let state = ReadingState {
                    page,
                    highlight_id: None,
                    // Nothing here has said either way about the panels, and
                    // saying nothing is what leaves them as the reader left
                    // them (see `Store::save_reading_state`).
                    outline_open: None,
                    chat_panel_open: None,
                };

                if let Err(err) = store.lock().save_reading_state(&book_id, &state) {
                    tracing::warn!(?err, book_id, page, "could not save the place");
                }
            })
            .detach();
    }

    /// Opens the book a search hit is in, at the page it is on, and points at
    /// the passage when the caller knows which one it was. A citation arrives
    /// here knowing only the page, and lands on it with nothing lit.
    pub(crate) fn open_found(
        &mut self,
        book_id: &str,
        page: u32,
        passage: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(book) = self
            .library
            .books()
            .iter()
            .find(|book| book.id == book_id)
            .cloned()
        else {
            return;
        };

        let tab_id: SharedString = format!("book:{}", book.id).into();
        let already_open = self.tabs.iter().any(|tab| tab.id == tab_id);
        self.open_entry(&Entry::opening(tab_id, crate::library::title_of(&book)), cx);

        let target = passage.map_or_else(
            || Spotlight::page(page),
            |passage| Spotlight::found(page, passage),
        );

        // A book that had to be read first turns to the page when it lands;
        // one that was already open can turn now.
        if already_open {
            self.show_page(page, cx);
            self.point_at(target, cx);
        } else {
            self.open_at = Some(target);
        }
    }

    /// Turns to a marked passage and reopens what was asked about it.
    fn open_marked_passage(&mut self, highlight_id: &str, cx: &mut Context<Self>) {
        let Some(highlight) = self
            .open_document()
            .and_then(|open| {
                open.highlights
                    .iter()
                    .find(|highlight| highlight.id == highlight_id)
            })
            .cloned()
        else {
            return;
        };

        self.show_page(highlight.page_number, cx);
        self.point_at(
            Spotlight::marked(highlight.page_number, highlight.rects.clone()),
            cx,
        );
        self.open_highlight(&highlight, cx);
        self.show_chat(true, cx);
    }

    /// Reopens the conversation behind a marked passage.
    fn open_highlight(&mut self, highlight: &Highlight, cx: &mut Context<Self>) {
        let Some(store) = self.library.store().cloned() else {
            return;
        };

        let mut conversation =
            Conversation::about(highlight.selected_text.clone(), highlight.page_number);
        conversation.about = Some(CoreConversation::Highlight(highlight.id.clone()));
        self.chat = Some(conversation);

        self.show_chat(true, cx);

        let highlight_id = highlight.id.clone();
        cx.spawn(async move |this, cx| {
            let messages = cx
                .background_executor()
                .spawn(async move {
                    store
                        .lock()
                        .messages(&CoreConversation::Highlight(highlight_id.clone()))
                        .map_err(|err| err.to_string())
                })
                .await;

            this.update(cx, |this, cx| {
                if let (Some(chat), Ok(messages)) = (&mut this.chat, messages) {
                    chat.answered(messages);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();

        cx.notify();
    }

    fn next_page(&mut self, _: &NextPage, _: &mut Window, cx: &mut Context<Self>) {
        self.turn_page(1, cx);
    }

    fn previous_page(&mut self, _: &PreviousPage, _: &mut Window, cx: &mut Context<Self>) {
        self.turn_page(-1, cx);
    }

    /// The book the reader is looking at, if any.
    pub(crate) fn open_document(&self) -> Option<&OpenDocument> {
        self.active_tab()?.document.as_ref()
    }

    pub(crate) fn activate_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }

        if self.active_tab == Some(index) {
            return;
        }

        self.active_tab = Some(index);
        // Pages of the book being left are drawn in the same coordinates as
        // pages of the one being opened, and only the latter will repaint.
        self.page_bounds.borrow_mut().clear();

        // A passage belongs to the book it was marked in, and so does a
        // conversation: citations in one point at pages of the other. An answer
        // still being written is left to finish and store itself, and is
        // reachable afterwards from the mark that started it.
        self.attached.clear();
        self.chat = None;

        cx.notify();
    }

    /// The scroll of the book being read, which each tab keeps for itself.
    pub(crate) fn page_scroll(&self) -> Option<&UniformListScrollHandle> {
        self.active_tab().map(|tab| &tab.scroll)
    }

    pub(crate) fn close_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }

        self.tabs.remove(index);
        self.active_tab = match self.active_tab {
            _ if self.tabs.is_empty() => None,
            // Closing a tab before the active one shifts it left; closing the
            // active one falls through to its neighbour.
            Some(active) if active > index => Some(active - 1),
            Some(active) if active == index => Some(active.min(self.tabs.len() - 1)),
            other => other,
        };
        cx.notify();
    }

    pub(crate) fn active_tab(&self) -> Option<&OpenTab> {
        self.active_tab.and_then(|index| self.tabs.get(index))
    }

    /// Whether this row is waiting to be told a second time.
    /// Opens the shelf menu for a book, at the pointer.
    pub(crate) fn open_shelf_menu(
        &mut self,
        book_id: &str,
        at: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.shelf_menu = Some(ShelfMenu {
            book_id: book_id.to_owned().into(),
            at,
        });
        cx.notify();
    }

    pub(crate) fn close_shelf_menu(&mut self, cx: &mut Context<Self>) {
        if self.shelf_menu.take().is_some() {
            cx.notify();
        }
    }

    /// Arms a two-press confirmation on `id`. See [`Pedro::remove_entry`].
    pub(crate) fn confirm(&mut self, id: SharedString, cx: &mut Context<Self>) {
        self.confirming_removal = Some(id);
        cx.notify();
    }

    /// Opens a book as a tab, the way clicking its sidebar row does.
    pub(crate) fn open_book_tab(&mut self, book_id: &str, cx: &mut Context<Self>) {
        let Some(book) = self
            .library
            .books()
            .iter()
            .find(|book| book.id == book_id)
            .cloned()
        else {
            return;
        };

        let tab_id = SharedString::from(format!("book:{}", book.id));
        let existing = self.tabs.iter().position(|tab| tab.id == tab_id);

        self.active_tab = Some(existing.unwrap_or_else(|| {
            self.tabs.push(OpenTab::new(tab_id, book.file_name.clone()));
            self.tabs.len() - 1
        }));

        if existing.is_none() {
            self.load_book(book.id.clone(), cx);
        }

        cx.notify();
    }

    pub(crate) fn is_confirming(&self, id: &SharedString) -> bool {
        self.confirming_removal.as_ref() == Some(id)
    }

    /// Asks, then does it.
    pub(crate) fn remove_entry(&mut self, entry: &Entry, cx: &mut Context<Self>) {
        if !self.is_confirming(&entry.id) {
            self.confirming_removal = Some(entry.id.clone());
            cx.notify();
            return;
        }

        self.confirming_removal = None;

        if let Some(book_id) = entry.id.strip_prefix("book:") {
            self.remove_book(book_id, cx);
        } else if let Some(highlight_id) = entry.id.strip_prefix("highlight:") {
            self.remove_highlight(highlight_id, cx);
        }
    }

    /// Forgets a book, its marks, its conversations and its bytes.
    fn remove_book(&mut self, book_id: &str, cx: &mut Context<Self>) {
        let Some(store) = self.library.store().cloned() else {
            return;
        };

        // Its tab goes with it: a tab of a book that is gone has nothing to
        // draw and nothing to close it for.
        let tab_id: SharedString = format!("book:{book_id}").into();
        if let Some(index) = self.tabs.iter().position(|tab| tab.id == tab_id) {
            self.close_tab(index, cx);
        }
        if self.chat.is_some() {
            self.close_chat(cx);
        }

        let book_id = book_id.to_owned();
        cx.spawn(async move |this, cx| {
            let books = cx
                .background_executor()
                .spawn(async move {
                    let store = store.lock();
                    store.remove_book(&book_id).map_err(|err| err.to_string())?;
                    store.books().map_err(|err| err.to_string())
                })
                .await;

            this.update(cx, |this, cx| {
                this.documents_added(books.map(|books| (books, Vec::new())), cx)
            })
            .ok();
        })
        .detach();
    }

    /// Forgets a marked passage and the conversation about it.
    fn remove_highlight(&mut self, highlight_id: &str, cx: &mut Context<Self>) {
        let Some(store) = self.library.store().cloned() else {
            return;
        };

        if self.chat.as_ref().is_some_and(|chat| {
            chat.about.as_ref() == Some(&CoreConversation::Highlight(highlight_id.to_owned()))
        }) {
            self.close_chat(cx);
        }

        let highlight_id = highlight_id.to_owned();
        cx.spawn(async move |this, cx| {
            let removed = cx
                .background_executor()
                .spawn(async move {
                    store
                        .lock()
                        .remove_highlight(&highlight_id)
                        .map_err(|err| err.to_string())
                })
                .await;

            this.update(cx, |this, cx| {
                if let Err(why) = removed {
                    tracing::warn!(why, "could not remove the passage");
                    this.notice = Some(why.into());
                }
                this.reload_highlights(cx);
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn toggle_sidebar(
        &mut self,
        _: &ToggleSidebar,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sidebar.toggle();
        cx.notify();
    }

    pub(crate) fn toggle_chat(&mut self, _: &ToggleChat, _: &mut Window, cx: &mut Context<Self>) {
        self.chat_pane.toggle();
        cx.notify();
    }

    /// Opens or shuts the conversation panel.
    pub(crate) fn show_chat(&mut self, open: bool, cx: &mut Context<Self>) {
        self.chat_pane.set_open(open);
        cx.notify();
    }

    /// Moves the panes one frame towards where they are going.
    ///
    /// On the display's clock rather than a timer of its own, for the same
    /// reason the answer is written on it: it is the only rate at which a
    /// change can actually be seen, and a pane that moves between two frames
    /// has moved invisibly.
    ///
    /// Both are stepped, so opening one while the other is closing does not
    /// leave the second stranded.
    fn slide_panes(&mut self, window: &mut Window) {
        if self.sidebar.step() | self.chat_pane.step() {
            window.request_animation_frame();
        }
    }

    pub(crate) fn toggle_secondary(&mut self, cx: &mut Context<Self>) {
        self.show_secondary = !self.show_secondary;
        cx.notify();
    }

    pub(crate) fn toggle_web_search(&mut self, cx: &mut Context<Self>) {
        self.web_search = !self.web_search;
        cx.notify();
    }

    /// Asks the agent about the marked passage.
    ///
    /// The passage is stored as a highlight first: chatbook hangs a
    /// conversation off a highlight rather than off a page, so that reopening
    /// the mark reopens what was said about it.
    pub(crate) fn ask(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let question = self.composer.read(cx).value().trim().to_string();
        if question.is_empty() {
            // Enter on an empty field still broke a line before it got here,
            // and a field that fills with blank lines is a field that looks
            // broken.
            self.composer
                .update(cx, |composer, cx| composer.set_value("", window, cx));
            return;
        }
        if self.chat.as_ref().is_some_and(Conversation::is_answering) {
            return;
        }

        let Some(store) = self.library.store().cloned() else {
            return;
        };
        let Some(agent) = self.answering_agent().cloned() else {
            self.notice = Some("No agent CLI was found. Install claude or codex.".into());
            cx.notify();
            return;
        };
        // A shelf is asked as a whole, and a book is asked about a passage of
        // it. Which of the two this is comes from the tab the reader is on.
        let shelf = self
            .active_shelf()
            .map(|shelf| (shelf.id.clone(), shelf.name.clone()));

        let (book_id, passages) = match &shelf {
            Some(_) => (String::new(), Vec::new()),
            None => {
                let Some(book_id) = self
                    .active_tab()
                    .and_then(|tab| tab.id.strip_prefix("book:"))
                    .map(str::to_owned)
                else {
                    return;
                };
                if self.attached.is_empty() {
                    self.notice = Some("Drag across the page to choose a passage first.".into());
                    cx.notify();
                    return;
                }

                (book_id, std::mem::take(&mut self.attached))
            }
        };

        // The conversation belongs to the shelf, or to the first passage, which
        // is where the reader will look for it again.
        let subject = match &shelf {
            Some((_, name)) => (name.clone(), 0),
            None => passages
                .first()
                .map(|first| (first.selected_text.clone(), first.page_number))
                .unwrap_or_default(),
        };
        let conversation = self
            .chat
            .get_or_insert_with(|| Conversation::about(subject.0, subject.1));
        conversation.asked(question.clone());
        let cancellation = conversation.cancellation.clone();

        self.composer
            .update(cx, |composer, cx| composer.set_value("", window, cx));

        let web_search = self.web_search;
        // Deltas arrive on the agent's thread and the view lives on this one,
        // so they travel by channel rather than by lock.
        let (deltas, mut arriving) = futures::channel::mpsc::unbounded::<String>();

        let answering = cx.background_executor().spawn(async move {
            // Three phases, and the store is held for two of them. An agent
            // takes as long as it takes; holding a single connection across
            // that would stop every page rendering and every place being
            // saved, because those run on this same pool.
            let asked = {
                let store = store.lock();

                let about = match &shelf {
                    Some((id, _)) => Subject::Shelf(id.clone()),
                    None => {
                        // The passages are stored first: a conversation hangs
                        // off a mark, so reopening the mark reopens what was
                        // said about it.
                        let mut highlight_ids = Vec::with_capacity(passages.len());
                        for passage in passages {
                            let stored = store
                                .add_highlight(&book_id, passage)
                                .map_err(|err| (err.to_string(), None))?;
                            highlight_ids.push(stored.id);
                        }

                        Subject::Passages(highlight_ids)
                    }
                };

                prepare(
                    &store,
                    &Question {
                        about,
                        text: question,
                        web_search,
                    },
                )
                .map_err(|err| (err.to_string(), sign_in_command(&err)))?
            };

            tracing::info!(
                agent = agent.kind.display_name(),
                turns = asked.prompt.turns.len(),
                context = asked.prompt.system.len(),
                "asking"
            );

            let started = std::time::Instant::now();
            let mut pieces = 0usize;
            let answer = run(&agent, &asked, &cancellation, &mut |delta| {
                pieces += 1;
                // A closed receiver means the reader moved on; the run is
                // left to finish and store its answer regardless.
                let _ = deltas.unbounded_send(delta.to_owned());
            })
            .inspect(|answer| {
                tracing::info!(
                    seconds = started.elapsed().as_secs_f32(),
                    pieces,
                    characters = answer.len(),
                    "answered"
                );
            })
            .inspect_err(|err| {
                tracing::warn!(
                    seconds = started.elapsed().as_secs_f32(),
                    pieces,
                    %err,
                    "the agent did not answer"
                );
            })
            .map_err(|err| {
                let sign_in = match &err {
                    pedro_agent::AgentError::NotSignedIn { command, .. } => Some(*command),
                    _ => None,
                };

                (err.to_string(), sign_in)
            })?;

            let store = store.lock();
            record(&store, &asked, &answer).map_err(|err| (err.to_string(), None))?;
            store
                .messages(&asked.about)
                .map_err(|err| (err.to_string(), None))
        });

        cx.spawn(async move |this, cx| {
            while let Some(delta) = arriving.next().await {
                if this
                    .update(cx, |this, cx| this.answer_arriving(&delta, cx))
                    .is_err()
                {
                    return;
                }
            }

            let finished = answering.await;
            this.update(cx, |this, cx| this.answered(finished, cx)).ok();
        })
        .detach();

        cx.notify();
    }

    /// The passage under the drag that has just ended.
    fn marked_passage(&self) -> Option<NewHighlight> {
        let open = self.open_document()?;
        let page = open.selection()?.page;

        Some(NewHighlight {
            selected_text: open.selected_text()?,
            page_number: page,
            rects: open.selection_rects(page),
        })
    }

    /// The CLI that answers: the one the reader chose, or the first found.
    pub(crate) fn answering_agent(&self) -> Option<&pedro_agent::DiscoveredAgent> {
        let AgentStatus::Done(agents) = &self.agent_status else {
            return None;
        };

        match self.answering {
            Some(chosen) => agents
                .iter()
                .find(|agent| agent.kind == chosen)
                .or_else(|| agents.first()),
            None => agents.first(),
        }
    }

    /// Which CLI is answering, for the panel that lets it be changed.
    pub(crate) fn answering_kind(&self) -> Option<pedro_agent::AgentKind> {
        self.answering_agent().map(|agent| agent.kind)
    }

    fn choose_agent(&mut self, program: &str, cx: &mut Context<Self>) {
        let AgentStatus::Done(agents) = &self.agent_status else {
            return;
        };

        if let Some(agent) = agents.iter().find(|agent| agent.kind.program() == program) {
            self.answering = Some(agent.kind);
            cx.notify();
        }
    }

    /// Draws pages larger or smaller, one step at a time.
    ///
    /// Every page held is thrown away: they were rasterised for the old size,
    /// and drawing them at the new one is a blurry page that never sharpens.
    /// Work already in flight is not cancelled but is dropped when it lands,
    /// which is what the generation counter is for.
    fn set_zoom(&mut self, zoom: f32, cx: &mut Context<Self>) {
        if (self.zoom - zoom).abs() < f32::EPSILON {
            return;
        }

        self.zoom = zoom;
        if let Some(open) = self.document_mut() {
            open.resized();
        }

        cx.notify();
    }

    fn step_zoom(&mut self, by: i32, cx: &mut Context<Self>) {
        let here = ZOOM_STEPS
            .iter()
            .position(|step| (step - self.zoom).abs() < f32::EPSILON)
            .unwrap_or(2);
        let next = (here as i32 + by).clamp(0, ZOOM_STEPS.len() as i32 - 1) as usize;

        self.set_zoom(ZOOM_STEPS[next], cx);
    }

    /// Moves to the next book along, wrapping round.
    fn step_tab(&mut self, by: i32, cx: &mut Context<Self>) {
        if self.tabs.len() < 2 {
            return;
        }

        let count = self.tabs.len() as i32;
        let here = self.active_tab.unwrap_or(0) as i32;
        let next = (here + by).rem_euclid(count) as usize;

        self.activate_tab(next, cx);
    }

    fn next_tab(&mut self, _: &NextTab, _: &mut Window, cx: &mut Context<Self>) {
        self.step_tab(1, cx);
    }

    fn previous_tab(&mut self, _: &PreviousTab, _: &mut Window, cx: &mut Context<Self>) {
        self.step_tab(-1, cx);
    }

    fn zoom_in(&mut self, _: &ZoomIn, _: &mut Window, cx: &mut Context<Self>) {
        self.step_zoom(1, cx);
    }

    fn zoom_out(&mut self, _: &ZoomOut, _: &mut Window, cx: &mut Context<Self>) {
        self.step_zoom(-1, cx);
    }

    fn zoom_reset(&mut self, _: &ZoomReset, _: &mut Window, cx: &mut Context<Self>) {
        self.set_zoom(1.0, cx);
    }

    /// How tall a page is drawn right now.
    pub(crate) fn page_height(&self) -> f32 {
        PAGE_HEIGHT * self.zoom
    }

    fn answer_arriving(&mut self, delta: &str, cx: &mut Context<Self>) {
        let Some(chat) = &mut self.chat else {
            return;
        };

        chat.streaming.push_str(delta);
        cx.notify();
    }

    /// Walks the answer onto the screen, one drawn frame at a time.
    ///
    /// This runs from `render` rather than from a timer, and that is the whole
    /// point. At a fixed beat each step has to carry whatever the agent
    /// produced in that beat — at 33ms and a typical rate, ten characters,
    /// which is a chunk however smoothly the rate was eased into it. The
    /// display is the only clock that divides the same text into the largest
    /// number of steps a reader can actually see, so it is the one to use: on
    /// a 120Hz screen the same answer arrives two or three characters at a
    /// time.
    ///
    /// Asking for the next frame is what keeps it going; when there is nothing
    /// left to write nothing is asked for, and the window goes back to sleep.
    fn write_a_little_more(&mut self, window: &mut Window) {
        let Some(chat) = &mut self.chat else {
            return;
        };

        if chat.reveal_more() {
            window.request_animation_frame();
        } else {
            // Caught up: an agent that finished while the answer was still
            // being written has been holding its stored turns, and this is the
            // moment they can go in unnoticed.
            if chat.settle() {
                window.request_animation_frame();
            }
        }
    }

    fn answered(
        &mut self,
        finished: Result<Vec<ChatMessage>, (String, Option<&'static str>)>,
        cx: &mut Context<Self>,
    ) {
        if let Some(chat) = &mut self.chat {
            match finished {
                Ok(messages) => {
                    chat.answered(messages);
                    // The passage was stored as a highlight to hang the
                    // conversation off; the page has not been told yet.
                    self.reload_highlights(cx);
                }
                Err((why, sign_in)) => {
                    tracing::error!(why, "the agent did not answer");
                    chat.failed(why, sign_in);
                }
            }
        }

        cx.notify();
    }

    /// Opens a terminal on the command that signs a CLI in.
    ///
    /// Pedro never sees the credentials — borrowing a CLI that already has them
    /// is the whole design — so the most it can do is put the reader in front
    /// of the command with the command already typed.
    pub(crate) fn sign_in(&mut self, command: &'static str, cx: &mut Context<Self>) {
        let script = format!(r#"tell application "Terminal" to do script "{command}""#);

        cx.background_executor()
            .spawn(async move {
                let opened = std::process::Command::new("osascript")
                    .args([
                        "-e",
                        &script,
                        "-e",
                        r#"tell application "Terminal" to activate"#,
                    ])
                    .status();

                if let Err(err) = opened {
                    tracing::warn!(?err, command, "could not open a terminal to sign in");
                }
            })
            .detach();
    }

    /// Reads the book's marked passages back out of the store.
    fn reload_highlights(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.library.store().cloned() else {
            return;
        };
        let Some(tab) = self.active_tab() else {
            return;
        };
        let Some(book_id) = tab.id.strip_prefix("book:").map(str::to_owned) else {
            return;
        };
        let tab_id = tab.id.clone();

        cx.spawn(async move |this, cx| {
            let highlights = cx
                .background_executor()
                .spawn(async move {
                    store
                        .lock()
                        .highlights(&book_id)
                        .map_err(|err| err.to_string())
                })
                .await;

            this.update(cx, |this, cx| {
                let Ok(highlights) = highlights else {
                    return;
                };
                if let Some(open) = this
                    .tabs
                    .iter_mut()
                    .find(|tab| tab.id == tab_id)
                    .and_then(|tab| tab.document.as_mut())
                {
                    open.highlights = highlights;
                    // The passage is a mark now, not a pending selection.
                    open.selection = None;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Stops an answer that is still being written.
    ///
    /// What has already arrived is shown in full rather than left half-drawn:
    /// the reader asked for it to stop, not for it to be hidden.
    pub(crate) fn stop_answering(&mut self, cx: &mut Context<Self>) {
        if let Some(chat) = &mut self.chat {
            chat.cancellation.cancel();
            chat.reveal_everything();
            // Nothing is left to write, so anything the agent managed to
            // finish before the cancellation can go in now.
            chat.settle();
            cx.notify();
        }
    }

    /// Shuts the conversation panel, keeping the conversation.
    ///
    /// An answer being written is left to finish: the reader closed a panel,
    /// which is not the same as saying they no longer want the answer. Stopping
    /// it is what the Stop button is for.
    pub(crate) fn close_chat(&mut self, cx: &mut Context<Self>) {
        self.show_chat(false, cx);
    }

    /// Jumps to the page a citation names.
    pub(crate) fn show_page(&mut self, page: u32, cx: &mut Context<Self>) {
        let Some(open) = self.document_mut() else {
            return;
        };

        let by = page as i64 - open.page as i64;
        self.turn_page(by, cx);
    }

    /// Points at the passage a jump was aimed at, so the reader lands on the
    /// sentence rather than on the page it is somewhere on.
    fn point_at(&mut self, target: Spotlight, cx: &mut Context<Self>) {
        if let Some(open) = self.document_mut() {
            open.spotlight(target);
        }

        cx.notify();
    }

    /// Keeps the composer's prompt on the same subject as the tab.
    fn follow_the_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = match self.active_shelf().is_some() {
            true => ASK_A_SHELF,
            false => ASK_A_DOCUMENT,
        };

        if self.composer_hint == wanted {
            return;
        }

        self.composer_hint = wanted;
        self.composer.update(cx, |composer, cx| {
            composer.set_placeholder(wanted, window, cx)
        });
    }

    fn focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
        self.search
            .update(cx, |search, cx| search.focus(window, cx));
    }
}

impl Focusable for Pedro {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Pedro {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.write_a_little_more(window);
        self.slide_panes(window);
        self.follow_the_tab(window, cx);

        v_flex()
            .id("pedro")
            .key_context("Pedro")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::focus_search))
            .on_action(cx.listener(Self::next_page))
            .on_action(cx.listener(Self::previous_page))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::toggle_chat))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::previous_tab))
            .on_action(cx.listener(Self::zoom_in))
            .on_action(cx.listener(Self::zoom_out))
            .on_action(cx.listener(Self::zoom_reset))
            .size_full()
            .text_color(cx.theme().foreground)
            .text_size(px(15.))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .children(self.render_sidebar(window, cx))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            // `h_flex` centres its children on the cross axis,
                            // so a column without a height of its own floats in
                            // the middle of the window at the height of
                            // whatever is inside it.
                            .h_full()
                            .min_h_0()
                            // The only place the canvas is painted: the reader
                            // and the composer sit on it rather than each
                            // laying down another film of their own.
                            .bg(palette::canvas())
                            .child(self.render_tab_bar(cx))
                            .child(
                                h_flex()
                                    .flex_1()
                                    .min_h_0()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .h_full()
                                            .child(self.render_reader(cx)),
                                    )
                                    .children(self.render_chat(window, cx)),
                            )
                            .child(self.render_composer(cx)),
                    ),
            )
    }
}
