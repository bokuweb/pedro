//! The root view: application state plus the top-level layout.
//!
//! The individual regions live in [`crate::ui`], as `impl Pedro` blocks, so
//! that this file stays about state transitions rather than styling.

use std::collections::HashMap;

use gpui::{
    App, Context, Entity, FocusHandle, Focusable, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Styled as _, Window, actions, div, px,
};
use gpui_component::input::InputState;
use gpui_component::{ActiveTheme as _, WindowExt as _, h_flex, v_flex};
use pedro_agent::DiscoveredAgent;

use crate::palette;
use crate::state::{AgentStatus, Entry, OpenTab, PageLayout, Panel, RailItem};

actions!(pedro, [FocusSearch]);

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
        let panels = RailItem::all()
            .map(|item| (item, Panel::for_rail_item(item, &agent_status)))
            .collect();

        Self::detect_agents(cx);

        let tabs = vec![OpenTab {
            id: "book:tcp".into(),
            label: "TCP/IP Illustrated".into(),
        }];

        Self {
            focus_handle: cx.focus_handle(),
            search,
            composer,
            web_search: true,
            active_rail: RailItem::Library,
            panels,
            tabs,
            active_tab: Some(0),
            layout: PageLayout::Single,
            agent_status,
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
            self.tabs.push(OpenTab {
                id: entry.id.clone(),
                label: entry.label.clone(),
            });
            self.tabs.len() - 1
        }));
        cx.notify();
    }

    pub(crate) fn activate_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.tabs.len() {
            self.active_tab = Some(index);
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

    pub(crate) fn toggle_web_search(&mut self, cx: &mut Context<Self>) {
        self.web_search = !self.web_search;
        cx.notify();
    }

    /// Sends what is in the composer.
    ///
    /// Deliberately loud rather than silently doing nothing: the field, the
    /// agent chip and the web toggle are all real, and the only missing piece
    /// is the wiring to `pedro-core`.
    pub(crate) fn ask(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let question = self.composer.read(cx).value().trim().to_string();
        if question.is_empty() {
            return;
        }

        window.push_notification("Asking is not wired to the reader yet.", cx);
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("pedro")
            .key_context("Pedro")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::focus_search))
            .size_full()
            .bg(palette::canvas())
            .text_color(cx.theme().foreground)
            .text_size(px(15.))
            .child(self.render_title_strip(cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_rail(cx))
                    .child(self.render_sidebar(cx))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(self.render_tab_bar(cx))
                            .child(div().flex_1().min_h_0().child(self.render_reader(cx)))
                            .child(self.render_composer(cx)),
                    ),
            )
    }
}
