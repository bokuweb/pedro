//! What the sidebar can switch to, what each panel lists, and which documents
//! are open.
//!
//! Panels are built when they are drawn rather than kept and invalidated. Their
//! contents follow the open book and the page it is on, and a cache of that has
//! to be swept on every event that moves either — which is every event.

use gpui::SharedString;
use gpui_component::IconName;
use pedro_agent::{AgentKind, DiscoveredAgent};
use pedro_core::model::{Book, Highlight};
use pedro_pdf::OutlineItem;

use crate::chat::Conversation;
use crate::document::OpenDocument;
use crate::library::{Library, how_long_ago, title_of};

/// A place the sidebar can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RailItem {
    Library,
    Reader,
    Highlights,
    Agents,
    Settings,
}

impl RailItem {
    /// The places a reader goes.
    ///
    /// There is no separate list of conversations: every conversation hangs off
    /// the passage that started it, so the highlights *are* that list.
    pub const PRIMARY: [RailItem; 3] = [RailItem::Library, RailItem::Reader, RailItem::Highlights];

    /// The places that are settings rather than reading.
    pub const SECONDARY: [RailItem; 2] = [RailItem::Agents, RailItem::Settings];

    pub fn icon(self) -> IconName {
        match self {
            RailItem::Library => IconName::GalleryVerticalEnd,
            RailItem::Reader => IconName::BookOpen,
            RailItem::Highlights => IconName::Star,
            RailItem::Agents => IconName::SquareTerminal,
            RailItem::Settings => IconName::Settings,
        }
    }

    /// Shown in the rail tooltip and as the sidebar panel title.
    pub fn title(self) -> &'static str {
        match self {
            RailItem::Library => "Library",
            RailItem::Reader => "Contents",
            RailItem::Highlights => "Highlights",
            RailItem::Agents => "Agents",
            RailItem::Settings => "Settings",
        }
    }

    /// One line of explanation for the panel header's info affordance.
    pub fn hint(self) -> &'static str {
        match self {
            RailItem::Library => "Documents you have added to pedro.",
            RailItem::Reader => "Table of contents for the active document.",
            RailItem::Highlights => "Passages you have marked, and what you asked about them.",
            RailItem::Agents => "Coding agent CLIs discovered on this machine.",
            RailItem::Settings => "Application preferences.",
        }
    }
}

/// What an entry is currently doing, shown as a dot and a word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Working,
    Failed,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Working => "Answering",
            Status::Failed => "Failed",
        }
    }
}

/// A single clickable entry in a sidebar section.
///
/// An entry with neither a [`meta`](Entry::meta) line nor a
/// [`detail`](Entry::detail) is drawn as one compact line instead of three,
/// which is what keeps an archive of forty rows readable next to a list of
/// four.
#[derive(Clone)]
pub struct Entry {
    pub id: SharedString,
    /// The title line.
    pub label: SharedString,
    /// The muted line above the title: where the entry came from.
    pub meta: Option<SharedString>,
    /// Right of the meta line: an age, a page, a count.
    pub trailing: Option<SharedString>,
    /// Replaces `trailing` when the entry is busy or finished.
    pub status: Option<Status>,
    /// Whether this is the row the reader is on — the chapter holding the open
    /// page, say. Drawn like a selected row, because that is what it is.
    pub current: bool,
    pub icon: IconName,
    /// The faint line below the title, beside the icon.
    pub detail: Option<SharedString>,
    /// Whether clicking the entry should open it as a tab.
    pub openable: bool,
    /// Whether the row offers to remove what it names.
    pub removable: bool,
}

impl Entry {
    /// An entry built to be acted on rather than listed — the shell opening
    /// something on the reader's behalf.
    pub fn opening(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self::new(id, label)
    }

    fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            meta: None,
            trailing: None,
            status: None,
            current: false,
            icon: IconName::File,
            detail: None,
            openable: true,
            removable: false,
        }
    }

    fn icon(mut self, icon: IconName) -> Self {
        self.icon = icon;
        self
    }

    fn meta(mut self, meta: impl Into<SharedString>) -> Self {
        self.meta = Some(meta.into());
        self
    }

    fn trailing(mut self, trailing: impl Into<SharedString>) -> Self {
        self.trailing = Some(trailing.into());
        self
    }

    fn status(mut self, status: Option<Status>) -> Self {
        self.status = status;
        self
    }

    fn current(mut self, current: bool) -> Self {
        self.current = current;
        self
    }

    fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    fn read_only(mut self) -> Self {
        self.openable = false;
        self
    }

    fn removable(mut self) -> Self {
        self.removable = true;
        self
    }

    /// Whether this draws as one line rather than three.
    pub fn is_compact(&self) -> bool {
        self.meta.is_none() && self.detail.is_none()
    }
}

/// A collapsible group of entries.
///
/// Whether it is open is not stored here: panels are rebuilt on every frame,
/// and state that survives has to live somewhere that is not.
#[derive(Clone)]
pub struct Section {
    pub title: SharedString,
    pub entries: Vec<Entry>,
}

impl Section {
    fn new(title: impl Into<SharedString>, entries: Vec<Entry>) -> Self {
        Self {
            title: title.into(),
            entries,
        }
    }

    /// The entries whose label matches `query`, which may be empty.
    pub fn matching(&self, query: &str) -> Vec<&Entry> {
        if query.is_empty() {
            return self.entries.iter().collect();
        }

        let query = query.to_lowercase();
        self.entries
            .iter()
            .filter(|entry| entry.label.to_lowercase().contains(&query))
            .collect()
    }
}

/// Everything the sidebar shows for one rail destination.
#[derive(Clone)]
pub struct Panel {
    pub sections: Vec<Section>,
    /// Shown instead of the sections when there is nothing to list.
    pub empty_message: SharedString,
}

impl Panel {
    fn new(sections: Vec<Section>, empty_message: impl Into<SharedString>) -> Self {
        Self {
            sections,
            empty_message: empty_message.into(),
        }
    }

    /// The books on disk, split by whether the reader has been into them.
    ///
    /// Two sections rather than one because "where was I" and "what did I add"
    /// are different questions, and a library of forty books answers neither
    /// as one flat list.
    pub fn library(library: &Library) -> Self {
        let (reading, unread): (Vec<&Book>, Vec<&Book>) = library
            .books()
            .iter()
            .partition(|book| book.reading.is_some());

        let sections = [("Reading", reading), ("Recently added", unread)]
            .into_iter()
            .filter(|(_, books)| !books.is_empty())
            .map(|(title, books)| Section::new(title, books.into_iter().map(row_for).collect()))
            .collect();

        Self::new(sections, library.empty_message())
    }

    /// The book's own table of contents, as pdfium read it.
    fn reader(outline: &[OutlineItem], page: u32) -> Self {
        // The chapter being read is the last one that starts at or before this
        // page, which is the same rule the excerpt is cut by.
        let current = outline
            .iter()
            .rposition(|chapter| chapter.page_number <= page);

        let entries = outline
            .iter()
            .enumerate()
            .map(|(index, chapter)| {
                Entry::new(
                    format!("page:{}", chapter.page_number),
                    chapter.title.clone(),
                )
                .icon(IconName::Dash)
                .trailing(format!("p. {}", chapter.page_number))
                .current(current == Some(index))
            })
            .collect();

        Self::new(
            vec![Section::new("Chapters", entries)],
            "This book has no table of contents.",
        )
    }

    /// The passages marked in the open book, newest first.
    ///
    /// Each one is also a conversation: pressing it opens the page and whatever
    /// was asked about that passage.
    fn highlights(highlights: &[Highlight], chat: Option<&Conversation>) -> Self {
        let entries = highlights
            .iter()
            .rev()
            .map(|highlight| {
                // The one being answered right now says so, since the answer
                // lands in a panel the reader may not be looking at.
                let open = chat.filter(|chat| chat.highlight_id.as_deref() == Some(&highlight.id));
                let status = open.and_then(|chat| match (chat.is_answering(), &chat.error) {
                    (true, _) => Some(Status::Working),
                    (_, Some(_)) => Some(Status::Failed),
                    _ => None,
                });

                Entry::new(
                    format!("highlight:{}", highlight.id),
                    one_line(&highlight.selected_text),
                )
                .icon(IconName::Star)
                .trailing(format!("p. {}", highlight.page_number))
                .removable()
                .status(status)
                .current(open.is_some())
            })
            .collect();

        Self::new(
            vec![Section::new("Marked", entries)],
            "Nothing marked yet. Drag across a page to ask about a passage.",
        )
    }

    /// What pedro is actually doing, rather than knobs it does not have.
    ///
    /// Every row here is a fact worth checking when something is wrong: where
    /// the books are, which pdfium is drawing them, how big a page is drawn.
    fn settings(shown: &Shown<'_>) -> Self {
        let library = match shown.library {
            Library::Ready { .. } => shown
                .library_path
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "opened".to_owned()),
            Library::Opening => "opening…".to_owned(),
            Library::Failed(why) => why.to_string(),
        };

        let pdfium = pedro_pdf::library_path()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "not loaded yet".to_owned());

        Self::new(
            vec![
                Section::new(
                    "Library",
                    vec![
                        Entry::new("about:library", "Where the books are")
                            .icon(IconName::Folder)
                            .detail(library)
                            .read_only(),
                        Entry::new("about:books", "Books")
                            .icon(IconName::GalleryVerticalEnd)
                            .trailing(format!("{}", shown.library.books().len()))
                            .read_only(),
                    ],
                ),
                Section::new(
                    "Reading",
                    vec![
                        Entry::new("about:zoom", "Page size")
                            .icon(IconName::Frame)
                            .trailing(format!("{:.0}%", shown.zoom * 100.0))
                            .read_only(),
                        Entry::new("about:pdfium", "Drawn by")
                            .icon(IconName::File)
                            .detail(pdfium)
                            .read_only(),
                    ],
                ),
            ],
            "",
        )
    }

    /// Built from whatever CLI discovery found.
    /// The CLIs found on this machine, and which of them answers.
    pub fn agents(status: &AgentStatus, answering: Option<AgentKind>) -> Self {
        let entries = match status {
            AgentStatus::Detecting => vec![
                Entry::new("agent:detecting", "Looking for installed CLIs…")
                    .icon(IconName::LoaderCircle)
                    .read_only(),
            ],
            AgentStatus::Done(agents) => agents
                .iter()
                .map(|agent| {
                    let entry = Entry::new(
                        format!("agent:{}", agent.kind.program()),
                        agent.kind.display_name(),
                    )
                    .icon(IconName::SquareTerminal)
                    .detail(agent.program.display().to_string())
                    .current(answering == Some(agent.kind));

                    match &agent.version {
                        Some(version) => entry.trailing(version.clone()),
                        None => entry,
                    }
                })
                .collect(),
        };

        Self::new(
            vec![Section::new("Installed", entries)],
            "No agent CLI found. Install claude or codex and restart pedro.",
        )
    }

    pub fn for_rail_item(item: RailItem, shown: &Shown<'_>) -> Self {
        match item {
            RailItem::Library => Self::library(shown.library),
            RailItem::Reader => Self::reader(shown.outline, shown.page),
            RailItem::Highlights => Self::highlights(shown.highlights, shown.chat),
            RailItem::Agents => Self::agents(shown.status, shown.answering),
            RailItem::Settings => Self::settings(shown),
        }
    }
}

/// Everything a panel is built from, gathered once per frame.
pub struct Shown<'a> {
    pub library: &'a Library,
    pub status: &'a AgentStatus,
    pub outline: &'a [OutlineItem],
    /// The page being read, so the contents can say which chapter that is in.
    pub page: u32,
    pub highlights: &'a [Highlight],
    /// The conversation that is open, so the passage behind it can say so.
    pub chat: Option<&'a Conversation>,
    /// Which CLI answers a question.
    pub answering: Option<AgentKind>,
    /// Where the books are kept, once the library is open.
    pub library_path: Option<&'a std::path::Path>,
    /// How large a page is drawn, as a multiple of its natural size.
    pub zoom: f32,
}

/// A passage as one line, for a list that has room for one.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// One book as a sidebar row: how big it is and when it was last touched,
/// its title, and the file it actually is.
fn row_for(book: &Book) -> Entry {
    let entry = Entry::new(format!("book:{}", book.id), title_of(book))
        .meta(format!("{} pages", book.page_count))
        .detail(book.file_name.clone())
        .removable();

    match &book.reading {
        Some(reading) => entry.trailing(format!("p. {}", reading.page)),
        None => entry.trailing(how_long_ago(book.updated_at)),
    }
}

/// Result of looking for locally installed agent CLIs.
#[derive(Debug, Clone)]
pub enum AgentStatus {
    Detecting,
    Done(Vec<DiscoveredAgent>),
}

impl AgentStatus {
    /// The text shown in the status pill in the top bar.
    pub fn headline(&self) -> SharedString {
        match self {
            AgentStatus::Detecting => "Looking for agent CLIs".into(),
            AgentStatus::Done(agents) => match agents.as_slice() {
                [] => "No agent CLI found".into(),
                [only] => only.kind.display_name().into(),
                many => format!("{} agent CLIs available", many.len()).into(),
            },
        }
    }

    pub fn icon(&self) -> IconName {
        match self {
            AgentStatus::Detecting => IconName::LoaderCircle,
            AgentStatus::Done(agents) if agents.is_empty() => IconName::TriangleAlert,
            AgentStatus::Done(_) => IconName::CircleCheck,
        }
    }

    pub fn is_problem(&self) -> bool {
        matches!(self, AgentStatus::Done(agents) if agents.is_empty())
    }
}

/// A document opened in the tab bar.
pub struct OpenTab {
    pub id: SharedString,
    pub label: SharedString,
    /// The book itself, once pdfium has opened it. `None` while it is being
    /// read, and on a tab that is not a book at all.
    pub document: Option<OpenDocument>,
    /// Why the book could not be opened, if it could not.
    pub error: Option<SharedString>,
}

impl OpenTab {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            document: None,
            error: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_returns_everything_for_an_empty_query() {
        let section = Section::new("s", vec![Entry::new("a", "Alpha"), Entry::new("b", "Beta")]);
        assert_eq!(section.matching("").len(), 2);
    }

    #[test]
    fn matching_is_case_insensitive() {
        let section = Section::new("s", vec![Entry::new("a", "Alpha"), Entry::new("b", "Beta")]);
        let found = section.matching("ALP");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].label, "Alpha");
    }

    #[test]
    fn headline_reports_a_missing_cli() {
        let status = AgentStatus::Done(vec![]);
        assert_eq!(status.headline().as_ref(), "No agent CLI found");
        assert!(status.is_problem());
    }

    #[test]
    fn detecting_is_not_a_problem() {
        assert!(!AgentStatus::Detecting.is_problem());
    }
}
