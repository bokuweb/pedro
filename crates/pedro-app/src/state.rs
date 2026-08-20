//! View state for the shell: what the rail can switch to, what each panel
//! lists, and which documents are open.
//!
//! The library contents are placeholder data for now. Once the document store
//! lands, [`Panel::library`] is the only thing that has to change.

use gpui::SharedString;
use gpui_component::IconName;
use pedro_agent::DiscoveredAgent;

/// A destination in the icon rail on the far left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RailItem {
    Library,
    Reader,
    Chat,
    Highlights,
    Agents,
    Settings,
}

impl RailItem {
    /// Destinations in the upper group of the rail.
    pub const PRIMARY: [RailItem; 4] = [
        RailItem::Library,
        RailItem::Reader,
        RailItem::Chat,
        RailItem::Highlights,
    ];

    /// Destinations pinned to the bottom of the rail.
    pub const SECONDARY: [RailItem; 2] = [RailItem::Agents, RailItem::Settings];

    pub fn icon(self) -> IconName {
        match self {
            RailItem::Library => IconName::GalleryVerticalEnd,
            RailItem::Reader => IconName::BookOpen,
            RailItem::Chat => IconName::Bot,
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
            RailItem::Chat => "Conversations",
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
            RailItem::Chat => "Questions you have asked about your documents.",
            RailItem::Highlights => "Passages you have marked while reading.",
            RailItem::Agents => "Coding agent CLIs discovered on this machine.",
            RailItem::Settings => "Application preferences.",
        }
    }

    pub fn all() -> impl Iterator<Item = RailItem> {
        Self::PRIMARY.into_iter().chain(Self::SECONDARY)
    }
}

/// A single clickable line in a sidebar section.
#[derive(Clone)]
pub struct Entry {
    pub id: SharedString,
    pub label: SharedString,
    pub icon: IconName,
    /// Trailing muted text, e.g. a page count or a CLI version.
    pub detail: Option<SharedString>,
    /// Whether clicking the entry should open it as a tab.
    pub openable: bool,
}

impl Entry {
    fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: IconName::File,
            detail: None,
            openable: true,
        }
    }

    fn icon(mut self, icon: IconName) -> Self {
        self.icon = icon;
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
}

/// A collapsible group of entries.
#[derive(Clone)]
pub struct Section {
    pub title: SharedString,
    pub expanded: bool,
    /// Whether to draw the "add" affordance in the section header.
    pub addable: bool,
    pub entries: Vec<Entry>,
}

impl Section {
    fn new(title: impl Into<SharedString>, entries: Vec<Entry>) -> Self {
        Self {
            title: title.into(),
            expanded: true,
            addable: false,
            entries,
        }
    }

    fn addable(mut self) -> Self {
        self.addable = true;
        self
    }

    fn collapsed(mut self) -> Self {
        self.expanded = false;
        self
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

    /// Placeholder library until documents are read from disk.
    pub fn library() -> Self {
        Self::new(
            vec![
                Section::new(
                    "Reading",
                    vec![
                        Entry::new("book:tcp", "TCP/IP Illustrated").detail("p. 214"),
                        Entry::new("book:sicp", "Structure and Interpretation").detail("p. 42"),
                    ],
                )
                .addable(),
                Section::new(
                    "Recently added",
                    vec![
                        Entry::new("book:crafting", "Crafting Interpreters"),
                        Entry::new("book:ddia", "Designing Data-Intensive Applications"),
                        Entry::new("book:rust", "Programming Rust"),
                    ],
                ),
                Section::new(
                    "Finished",
                    vec![Entry::new("book:pragmatic", "The Pragmatic Programmer")],
                )
                .collapsed(),
            ],
            "No documents yet. Add a PDF to get started.",
        )
    }

    fn reader() -> Self {
        Self::new(
            vec![Section::new(
                "Chapters",
                vec![
                    Entry::new("toc:1", "1. Introduction").detail("p. 1"),
                    Entry::new("toc:2", "2. The Link Layer").detail("p. 21"),
                    Entry::new("toc:3", "3. The Internet Protocol").detail("p. 63"),
                ],
            )],
            "Open a document to see its contents.",
        )
    }

    fn chat() -> Self {
        Self::new(
            vec![
                Section::new(
                    "Recent",
                    vec![
                        Entry::new("chat:1", "Why is the window scaled?").icon(IconName::Bot),
                        Entry::new("chat:2", "Explain slow start").icon(IconName::Bot),
                    ],
                )
                .addable(),
            ],
            "Select a passage while reading to ask about it.",
        )
    }

    fn highlights() -> Self {
        Self::new(
            vec![Section::new(
                "TCP/IP Illustrated",
                vec![
                    Entry::new("hl:1", "Nagle's algorithm")
                        .icon(IconName::Star)
                        .detail("p. 267"),
                    Entry::new("hl:2", "Silly window syndrome")
                        .icon(IconName::Star)
                        .detail("p. 271"),
                ],
            )],
            "Nothing highlighted yet.",
        )
    }

    fn settings() -> Self {
        Self::new(
            vec![Section::new(
                "Preferences",
                vec![
                    Entry::new("set:appearance", "Appearance").icon(IconName::Palette),
                    Entry::new("set:reading", "Reading").icon(IconName::BookOpen),
                    Entry::new("set:agents", "Agents").icon(IconName::Bot),
                ],
            )],
            "",
        )
    }

    /// Built from whatever CLI discovery found.
    pub fn agents(status: &AgentStatus) -> Self {
        let entries = match status {
            AgentStatus::Detecting => vec![
                Entry::new("agent:detecting", "Looking for installed CLIs...")
                    .icon(IconName::LoaderCircle)
                    .read_only(),
            ],
            AgentStatus::Done(agents) => agents
                .iter()
                .map(|agent| {
                    let entry = Entry::new(
                        format!("agent:{}", agent.program.display()),
                        agent.kind.display_name(),
                    )
                    .icon(IconName::SquareTerminal)
                    .read_only();

                    match &agent.version {
                        Some(version) => entry.detail(version.clone()),
                        None => entry,
                    }
                })
                .collect(),
        };

        Self::new(
            vec![Section::new("Detected", entries)],
            "No agent CLI found. Install claude or codex and restart pedro.",
        )
    }

    pub fn for_rail_item(item: RailItem, status: &AgentStatus) -> Self {
        match item {
            RailItem::Library => Self::library(),
            RailItem::Reader => Self::reader(),
            RailItem::Chat => Self::chat(),
            RailItem::Highlights => Self::highlights(),
            RailItem::Agents => Self::agents(status),
            RailItem::Settings => Self::settings(),
        }
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

/// How pages are arranged in the reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageLayout {
    Single,
    Spread,
}

impl PageLayout {
    pub fn icon(self) -> IconName {
        match self {
            PageLayout::Single => IconName::Frame,
            PageLayout::Spread => IconName::LayoutDashboard,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PageLayout::Single => "Single page",
            PageLayout::Spread => "Two-page spread",
        }
    }
}

/// A document opened in the tab bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenTab {
    pub id: SharedString,
    pub label: SharedString,
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
