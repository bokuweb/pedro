//! The root view: application state plus the top-level layout.
//!
//! The individual regions live in [`crate::ui`], as `impl Pedro` blocks, so
//! that this file stays about state transitions rather than styling.

use std::collections::HashMap;
use std::path::PathBuf;

use futures::StreamExt as _;
use gpui::{
    App, AppContext as _, Bounds, Context, Entity, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, ParentElement as _, PathPromptOptions, Pixels, Point, Render, SharedString,
    Styled as _, Window, actions, div, px,
};
use gpui_component::input::InputState;
use gpui_component::{ActiveTheme as _, h_flex, v_flex};
use pedro_agent::DiscoveredAgent;
use pedro_core::model::{Book, ChatMessage, NewHighlight};
use pedro_core::store::Store;
use pedro_core::{Question, ask};
use pedro_pdf::{Document, PageSize};

use crate::chat::Conversation;
use crate::document::{OpenDocument, as_render_image};
use crate::library::{Library, SharedStore};
use crate::palette;
use crate::state::{AgentStatus, Entry, OpenTab, PageLayout, Panel, RailItem};

actions!(pedro, [FocusSearch, NextPage, PreviousPage]);

/// How tall a page is drawn, in logical pixels.
///
/// Fixed for now: fitting to the window means measuring it, and measuring it
/// means re-rasterising every page on every resize. A constant gets real pages
/// on screen; the zoom control is what will replace it.
pub(crate) const PAGE_HEIGHT: f32 = 640.;

pub struct Pedro {
    focus_handle: FocusHandle,
    pub(crate) search: Entity<InputState>,
    /// Where a question about the open document is typed.
    pub(crate) composer: Entity<InputState>,
    /// Whether the agent may search the web, chatbook's toggle. Per question
    /// rather than per install, so it lives beside the field.
    pub(crate) web_search: bool,
    pub(crate) active_rail: RailItem,
    pub(crate) panels: HashMap<RailItem, Panel>,
    pub(crate) tabs: Vec<OpenTab>,
    pub(crate) active_tab: Option<usize>,
    pub(crate) layout: PageLayout,
    pub(crate) agent_status: AgentStatus,
    pub(crate) library: Library,
    /// The conversation the chat panel is showing, if one is open.
    pub(crate) chat: Option<Conversation>,
    /// Whether the sidebar is showing the places that are settings rather than
    /// reading.
    pub(crate) show_secondary: bool,
    /// Where the page is on screen, recorded while it is drawn. A drag arrives
    /// in window coordinates and the page needs it as a fraction of itself.
    pub(crate) page_bounds: Option<Bounds<Pixels>>,
    /// Whether the pointer is down on the page, dragging out a passage.
    pub(crate) selecting: bool,
    /// The last thing that went wrong where the reader was looking. Shown in
    /// the sidebar rather than as a notification: a file that could not be
    /// added is about the list it is missing from.
    pub(crate) notice: Option<SharedString>,
    /// Set on mouse-down in the title strip so the next drag moves the window.
    pub(crate) window_drag_armed: bool,
}

impl Pedro {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        // Re-render as the query changes so the sidebar filter stays live.
        cx.observe(&search, |_, _, cx| cx.notify()).detach();

        // Grows with the question rather than scrolling inside two lines: a
        // passage quoted back at the agent is easily a paragraph.
        let composer = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .auto_grow(1, 6)
                .placeholder("Ask about this document…")
        });

        let agent_status = AgentStatus::Detecting;
        let library = Library::Opening;
        let panels = RailItem::all()
            .map(|item| (item, Panel::for_rail_item(item, &agent_status, &library)))
            .collect();

        Self::detect_agents(cx);
        Self::open_library(cx);

        Self {
            focus_handle: cx.focus_handle(),
            search,
            composer,
            web_search: true,
            active_rail: RailItem::Library,
            panels,
            tabs: Vec::new(),
            active_tab: None,
            layout: PageLayout::Single,
            agent_status,
            library,
            chat: None,
            show_secondary: false,
            page_bounds: None,
            selecting: false,
            notice: None,
            window_drag_armed: false,
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
        self.panels
            .insert(RailItem::Agents, Panel::agents(&self.agent_status));
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
                    Ok::<_, String>((store, books))
                })
                .await;

            this.update(cx, |this, cx| this.library_opened(opened, cx))
                .ok();
        })
        .detach();
    }

    fn library_opened(
        &mut self,
        opened: Result<(Store, Vec<Book>), String>,
        cx: &mut Context<Self>,
    ) {
        self.library = match opened {
            Ok((store, books)) => {
                tracing::info!(books = books.len(), "opened the library");
                Library::Ready {
                    store: SharedStore::new(store),
                    books,
                }
            }
            Err(why) => {
                tracing::error!(why, "could not open the library");
                Library::Failed(why.into())
            }
        };

        self.refresh_library_panel(cx);
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
                    let store = store.lock();
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

        self.refresh_library_panel(cx);
    }

    fn refresh_library_panel(&mut self, cx: &mut Context<Self>) {
        self.panels
            .insert(RailItem::Library, Panel::library(&self.library));
        cx.notify();
    }

    pub(crate) fn panel(&self) -> &Panel {
        self.panels
            .get(&self.active_rail)
            .expect("every rail item has a panel")
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
        if let Some(panel) = self.panels.get_mut(&self.active_rail)
            && let Some(section) = panel.sections.get_mut(index)
        {
            section.expanded = !section.expanded;
            cx.notify();
        }
    }

    /// Opens an entry as a tab, or focuses it when it is already open.
    pub(crate) fn open_entry(&mut self, entry: &Entry, cx: &mut Context<Self>) {
        if !entry.openable {
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
                    let path = store.lock().document_path(&book);
                    let document = Document::open(&path).map_err(|err| err.to_string())?;
                    // Every page is laid out against the first one's size, so
                    // it is read here rather than on the UI thread later.
                    let size = document.page_size(0).map_err(|err| err.to_string())?;
                    Ok::<_, String>((document, size))
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
        opened: Result<(Document, PageSize), String>,
        page: u32,
        cx: &mut Context<Self>,
    ) {
        // The reader may have closed the tab while the book was being read.
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
            return;
        };

        match opened {
            Ok((document, size)) => {
                tab.error = None;
                tab.document = Some(OpenDocument::new(document, size, page));
            }
            Err(why) => {
                tracing::error!(why, tab_id, "could not open the book");
                tab.error = Some(why.into());
            }
        }

        self.render_visible_page(cx);
        cx.notify();
    }

    /// Rasterises the page the reader is on, unless it is already drawn.
    fn render_visible_page(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.active_tab.and_then(|index| self.tabs.get(index)) else {
            return;
        };
        let Some(open) = &tab.document else {
            return;
        };
        if open.visible().is_some() {
            return;
        }

        let tab_id = tab.id.clone();
        let page = open.page;
        let document = open.document.clone();
        let scale = open.scale_for(PAGE_HEIGHT);

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

                    Ok::<_, String>((image, text))
                })
                .await;

            this.update(cx, |this, cx| {
                this.page_rendered(&tab_id, page, rendered, cx)
            })
            .ok();
        })
        .detach();
    }

    fn page_rendered(
        &mut self,
        tab_id: &str,
        page: u32,
        rendered: Result<(pedro_pdf::PageImage, pedro_pdf::PageText), String>,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
            return;
        };
        let Some(open) = &mut tab.document else {
            return;
        };

        match rendered {
            Ok((image, text)) => {
                open.text = Some(text);
                open.rendered =
                    as_render_image(image).map(|image| crate::document::Rendered { page, image });
            }
            Err(why) => {
                tracing::error!(why, page, "could not render a page");
                tab.error = Some(why.into());
            }
        }

        cx.notify();
    }

    /// Records where the page is being drawn, so a drag can be read against it.
    ///
    /// Called while the page is laid out, which is the only place the answer is
    /// known — and only stored when it changes, since storing it every frame
    /// would ask for another frame every frame.
    pub(crate) fn page_drawn_at(&mut self, bounds: Bounds<Pixels>) {
        if self.page_bounds != Some(bounds) {
            self.page_bounds = Some(bounds);
        }
    }

    /// Where a window-space point falls on the page, as fractions of it.
    fn on_the_page(&self, position: Point<Pixels>) -> Option<(f32, f32)> {
        let bounds = self.page_bounds?;
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

    pub(crate) fn begin_selection(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some((x, y)) = self.on_the_page(position) else {
            return;
        };
        let Some(open) = self.document_mut() else {
            return;
        };

        open.begin_selection(x, y);
        self.selecting = true;
        cx.notify();
    }

    pub(crate) fn extend_selection(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if !self.selecting {
            return;
        }
        let Some((x, y)) = self.on_the_page(position) else {
            return;
        };
        let Some(open) = self.document_mut() else {
            return;
        };

        open.extend_selection(x, y);
        cx.notify();
    }

    pub(crate) fn finish_selection(&mut self, cx: &mut Context<Self>) {
        if self.selecting {
            self.selecting = false;
            cx.notify();
        }
    }

    /// The passage a question would quote, if the reader has marked one.
    pub(crate) fn selected_text(&self) -> Option<String> {
        self.open_document()?.selected_text()
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

        if open.turn(by) {
            self.render_visible_page(cx);
            cx.notify();
        }
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
        if index < self.tabs.len() {
            self.active_tab = Some(index);
            // Its page may never have been drawn, or may have been drawn at a
            // page the reader has since turned away from in another tab.
            self.render_visible_page(cx);
            cx.notify();
        }
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

    pub(crate) fn set_layout(&mut self, layout: PageLayout, cx: &mut Context<Self>) {
        if self.layout != layout {
            self.layout = layout;
            cx.notify();
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
            return;
        }
        if self.chat.as_ref().is_some_and(Conversation::is_answering) {
            return;
        }

        let Some(store) = self.library.store().cloned() else {
            return;
        };
        let Some(agent) = self.first_agent() else {
            self.notice = Some("No agent CLI was found. Install claude or codex.".into());
            cx.notify();
            return;
        };
        let Some((book_id, highlight)) = self.marked_passage() else {
            self.notice = Some("Drag across the page to choose a passage first.".into());
            cx.notify();
            return;
        };

        let conversation = self.chat.get_or_insert_with(|| {
            Conversation::about(highlight.selected_text.clone(), highlight.page_number)
        });
        conversation.asked(question.clone());
        let cancellation = conversation.cancellation.clone();

        self.composer
            .update(cx, |composer, cx| composer.set_value("", window, cx));

        let web_search = self.web_search;
        // Deltas arrive on the agent's thread and the view lives on this one,
        // so they travel by channel rather than by lock.
        let (deltas, mut arriving) = futures::channel::mpsc::unbounded::<String>();

        let answering = cx.background_executor().spawn(async move {
            let store = store.lock();
            let stored = store
                .add_highlight(&book_id, highlight)
                .map_err(|err| err.to_string())?;

            let answer = ask(
                &store,
                &agent,
                &Question {
                    highlight_id: stored.id.clone(),
                    text: question,
                    web_search,
                },
                &cancellation,
                &mut |delta| {
                    // A closed receiver means the reader moved on; the run is
                    // left to finish and store its answer regardless.
                    let _ = deltas.unbounded_send(delta.to_owned());
                },
            );

            match answer {
                Ok(_) => store.messages(&stored.id).map_err(|err| err.to_string()),
                Err(err) => Err(err.to_string()),
            }
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

    /// The passage to ask about, and the book it is in.
    fn marked_passage(&self) -> Option<(String, NewHighlight)> {
        let tab = self.active_tab()?;
        let book_id = tab.id.strip_prefix("book:")?.to_owned();
        let open = tab.document.as_ref()?;

        Some((
            book_id,
            NewHighlight {
                selected_text: open.selected_text()?,
                page_number: open.page,
                rects: open.selection_rects(),
            },
        ))
    }

    fn first_agent(&self) -> Option<pedro_agent::DiscoveredAgent> {
        match &self.agent_status {
            AgentStatus::Done(agents) => agents.first().cloned(),
            AgentStatus::Detecting => None,
        }
    }

    fn answer_arriving(&mut self, delta: &str, cx: &mut Context<Self>) {
        if let Some(chat) = &mut self.chat {
            chat.streaming.push_str(delta);
            cx.notify();
        }
    }

    fn answered(&mut self, finished: Result<Vec<ChatMessage>, String>, cx: &mut Context<Self>) {
        if let Some(chat) = &mut self.chat {
            match finished {
                Ok(messages) => chat.answered(messages),
                Err(why) => {
                    tracing::error!(why, "the agent did not answer");
                    chat.failed(why);
                }
            }
        }

        cx.notify();
    }

    /// Stops an answer that is still being written.
    pub(crate) fn stop_answering(&mut self, cx: &mut Context<Self>) {
        if let Some(chat) = &self.chat {
            chat.cancellation.cancel();
            cx.notify();
        }
    }

    pub(crate) fn close_chat(&mut self, cx: &mut Context<Self>) {
        if let Some(chat) = &self.chat {
            chat.cancellation.cancel();
        }
        self.chat = None;
        cx.notify();
    }

    /// Jumps to the page a citation names.
    pub(crate) fn show_page(&mut self, page: u32, cx: &mut Context<Self>) {
        let Some(open) = self.document_mut() else {
            return;
        };

        let by = page as i64 - open.page as i64;
        self.turn_page(by, cx);
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
        v_flex()
            .id("pedro")
            .key_context("Pedro")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::focus_search))
            .on_action(cx.listener(Self::next_page))
            .on_action(cx.listener(Self::previous_page))
            .size_full()
            .text_color(cx.theme().foreground)
            .text_size(px(15.))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_sidebar(window, cx))
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
                                    .children(self.render_chat(cx)),
                            )
                            .child(self.render_composer(cx)),
                    ),
            )
    }
}
