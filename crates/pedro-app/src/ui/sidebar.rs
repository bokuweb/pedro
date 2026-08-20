//! Everything to the left of the page: the window controls, where in the
//! application you are, what that place holds, and which agent is answering.
//!
//! There is no icon rail beside this. A column of icons whose labels are only
//! ever a tooltip is a second navigation for the same six places this one
//! already names, and it costs 64 points of every window to say less.
//!
//! Rows are separated by space rather than by hairlines, and the one you are on
//! is a rounded fill inset from both edges. A list of documents is read by
//! scanning titles, and rules between them turn that into a table.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Context, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use gpui_component::input::Input;
use gpui_component::tooltip::Tooltip;
use gpui_component::{IconName, h_flex, v_flex};

use crate::app::Pedro;
use crate::palette;
use crate::state::{AgentStatus, Entry, RailItem, Section, Status};
use crate::ui::icon;

const WIDTH: f32 = 300.;

/// How far the rows are inset from the panel edges. The fill on the active row
/// stops here, which is what makes it read as a card rather than a band.
const INSET: f32 = 8.;

/// Room for the macOS traffic lights, which live in this panel now that there
/// is no title bar and no rail for them to live in.
const LIGHTS: f32 = 84.;

impl Pedro {
    pub(crate) fn render_sidebar(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        v_flex()
            .w(px(WIDTH))
            .h_full()
            .flex_shrink_0()
            .bg(palette::sidebar())
            .border_r_1()
            .border_color(palette::border())
            .child(self.render_window_row(window, cx))
            .child(self.render_navigation(cx))
            .child(self.render_search())
            .children(self.render_notice())
            .child(self.render_sections(cx))
            .child(self.render_agent_footer())
    }

    /// The row the window controls sit in, with the panel's own actions beside
    /// them. Dragging it moves the window, the way a title bar would.
    fn render_window_row(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let hint = self.active_rail.hint();
        let indent = if window.is_fullscreen() {
            INSET
        } else {
            LIGHTS
        };

        let row = h_flex()
            .id("window-row")
            .h(px(46.))
            .pl(px(indent))
            .pr(px(INSET))
            .gap(px(2.))
            .items_center()
            .child(self.render_add_button(cx))
            .child(
                div()
                    .id("panel-hint")
                    .size(px(24.))
                    .rounded(px(7.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .hover(|this| this.bg(palette::row_hover()))
                    .child(icon(IconName::Info, px(14.), palette::text_faint()))
                    .tooltip(move |window, cx| Tooltip::new(hint).build(window, cx)),
            )
            .child(div().flex_1());

        self.draggable(row, cx)
    }

    /// Where in the application the reader is.
    ///
    /// The four places a reader goes are always listed; the two that are
    /// settings rather than reading are behind "More", which is where a thing
    /// you touch twice a month belongs.
    fn render_navigation(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let primary: Vec<_> = RailItem::PRIMARY
            .into_iter()
            .map(|item| self.render_navigation_row(item, cx))
            .collect();
        let secondary: Vec<_> = if self.show_secondary {
            RailItem::SECONDARY
                .into_iter()
                .map(|item| self.render_navigation_row(item, cx))
                .collect()
        } else {
            Vec::new()
        };

        let chevron = if self.show_secondary {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        };

        v_flex()
            .flex_shrink_0()
            .pb(px(6.))
            .children(primary)
            .child(
                h_flex()
                    .id("more")
                    .h(px(30.))
                    .mx(px(INSET))
                    .px(px(8.))
                    .gap(px(8.))
                    .items_center()
                    .rounded(px(8.))
                    .cursor_pointer()
                    .hover(|this| this.bg(palette::row_hover()))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_secondary(cx)))
                    .child(icon(chevron, px(13.), palette::text_faint()))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(palette::text_faint())
                            .child("More"),
                    ),
            )
            .children(secondary)
    }

    fn render_navigation_row(
        &self,
        item: RailItem,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let active = self.active_rail == item;

        h_flex()
            .id(("navigate", item as usize))
            .h(px(32.))
            .mx(px(INSET))
            .px(px(8.))
            .gap(px(9.))
            .items_center()
            .rounded(px(8.))
            .cursor_pointer()
            .when(active, |this| this.bg(palette::row_active()))
            .when(!active, |this| {
                this.hover(|this| this.bg(palette::row_hover()))
            })
            .on_click(cx.listener(move |this, _, _, cx| this.select_rail(item, cx)))
            .child(icon(
                item.icon(),
                px(15.),
                if active {
                    palette::text()
                } else {
                    palette::text_muted()
                },
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.))
                    .text_color(if active {
                        palette::text()
                    } else {
                        palette::text_muted()
                    })
                    .child(item.title()),
            )
    }

    /// The last thing that went wrong, where the reader was looking when it
    /// did. A file that could not be added is about the list it is missing
    /// from, so it belongs here rather than in a notification that floats away.
    fn render_notice(&self) -> Option<impl IntoElement + use<>> {
        let notice = self.notice.clone()?;

        Some(
            div()
                .mx(px(INSET))
                .mb(px(6.))
                .px(px(10.))
                .py(px(7.))
                .rounded(px(8.))
                .bg(palette::danger().opacity(0.14))
                .text_size(px(11.))
                .text_color(palette::danger())
                .child(notice),
        )
    }

    /// The plus in the panel header.
    fn render_add_button(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        div()
            .id("panel-add")
            .size(px(24.))
            .rounded(px(7.))
            .flex()
            .items_center()
            .justify_center()
            .hover(|this| this.bg(palette::row_hover()))
            .child(icon(IconName::Plus, px(15.), palette::text_muted()))
            .tooltip(move |window, cx| Tooltip::new("Add a document").build(window, cx))
            .on_click(cx.listener(|this, _, _, cx| {
                cx.stop_propagation();
                this.pick_documents(cx);
            }))
    }

    fn render_search(&self) -> impl IntoElement + use<> {
        div().px(px(INSET)).pb(px(6.)).child(
            Input::new(&self.search)
                .h(px(34.))
                .prefix(icon(IconName::Search, px(15.), palette::text_faint()))
                .suffix(
                    div()
                        .text_size(px(11.))
                        .text_color(palette::text_faint())
                        .child("⌘K"),
                ),
        )
    }

    fn render_sections(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let query = self.search_query(cx);
        let panel = self.panel();

        // A search hides sections that have nothing left in them; with no
        // search every section stays visible, collapsed or not.
        let visible: Vec<(usize, &Section, Vec<&Entry>)> = panel
            .sections
            .iter()
            .enumerate()
            .map(|(index, section)| (index, section, section.matching(&query)))
            .filter(|(_, _, entries)| query.is_empty() || !entries.is_empty())
            .collect();

        let empty_message: SharedString = if query.is_empty() {
            panel.empty_message.clone()
        } else {
            format!("Nothing matches “{query}”.").into()
        };

        let is_empty = visible.is_empty();

        // Rendered up front so that only one borrow of `cx` is live at a time;
        // builder temporaries would otherwise all survive to the end of the
        // statement.
        let mut rendered = Vec::with_capacity(visible.len());
        for (index, section, entries) in visible {
            // While searching, matches are always worth showing even if the
            // section was shut by hand.
            let expanded = self.is_expanded(index) || !query.is_empty();
            let header = self.render_section_header(index, section, expanded, cx);

            let rows: Vec<_> = if expanded {
                entries
                    .into_iter()
                    .map(|entry| self.render_entry(entry, cx))
                    .collect()
            } else {
                Vec::new()
            };

            rendered.push(v_flex().child(header).children(rows));
        }

        v_flex()
            .id("sections")
            .flex_1()
            .min_h_0()
            .pb(px(8.))
            .overflow_y_scroll()
            .when(is_empty, |this| {
                this.child(
                    div()
                        .px(px(16.))
                        .py(px(24.))
                        .text_size(px(13.))
                        .text_color(palette::text_faint())
                        .child(empty_message),
                )
            })
            .children(rendered)
    }

    /// A quiet label with the chevron on the trailing edge, so the eye lands on
    /// the rows rather than on the headers between them.
    fn render_section_header(
        &self,
        index: usize,
        section: &Section,
        expanded: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let chevron = if expanded {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        };

        h_flex()
            .id(("section", index))
            .h(px(32.))
            .mx(px(INSET))
            .px(px(8.))
            .mt(px(10.))
            .gap(px(8.))
            .items_center()
            .rounded(px(8.))
            .cursor_pointer()
            .hover(|this| this.bg(palette::row_hover()))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_section(index, cx)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.))
                    .text_color(palette::text_faint())
                    .child(section.title.clone()),
            )
            .child(icon(chevron, px(13.), palette::text_faint()))
    }

    fn render_entry(&self, entry: &Entry, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let active = entry.current || self.active_tab().is_some_and(|tab| tab.id == entry.id);
        let clickable = entry.openable;
        let on_open = entry.clone();

        let row = div()
            .id(entry.id.clone())
            .mx(px(INSET))
            .mt(px(2.))
            .rounded(px(10.))
            .bg(if active {
                palette::row_active()
            } else {
                palette::row()
            })
            .when(clickable, |this| {
                this.cursor_pointer()
                    .when(!active, |this| {
                        this.hover(|this| this.bg(palette::row_hover()))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.open_entry(&on_open, cx)))
            });

        if entry.is_compact() {
            return row.child(render_compact(entry, active));
        }

        row.child(render_full(entry))
    }

    /// Which agent is answering, at the foot of the panel.
    ///
    /// The reference design puts the signed-in account here. Pedro has no
    /// account; the equivalent fact — whose credentials are about to answer a
    /// question — is the CLI it found.
    fn render_agent_footer(&self) -> impl IntoElement + use<> {
        let (name, detail): (SharedString, SharedString) =
            match (&self.agent_status, self.answering_agent()) {
                (AgentStatus::Detecting, _) => ("Looking…".into(), "for an agent CLI".into()),
                (_, Some(agent)) => (
                    agent.kind.display_name().into(),
                    agent
                        .version
                        .clone()
                        .unwrap_or_else(|| "installed".to_owned())
                        .into(),
                ),
                (_, None) => ("No agent".into(), "install claude or codex".into()),
            };

        // The first letter, the way the reference design badges an account.
        let initial: SharedString = name.chars().next().unwrap_or('?').to_string().into();

        h_flex()
            .h(px(58.))
            .px(px(14.))
            .gap(px(10.))
            .items_center()
            .flex_shrink_0()
            .border_t_1()
            .border_color(palette::separator())
            .child(
                div()
                    .size(px(30.))
                    .rounded_full()
                    .bg(palette::accent())
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(13.))
                    .text_color(palette::text())
                    .child(initial),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .text_size(px(13.))
                            .text_color(palette::text())
                            .child(name),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(palette::text_faint())
                            .child(detail),
                    ),
            )
    }
}

/// The three-line shape: where it came from, what it is, what file it is in.
/// `use<>` because the row is stored as an `AnyElement`, which is `'static`:
/// everything here is cloned out of the entry rather than borrowed from it,
/// and edition 2024 would otherwise capture the lifetime anyway.
fn render_full(entry: &Entry) -> impl IntoElement + use<> {
    v_flex()
        .px(px(10.))
        .py(px(8.))
        .gap(px(3.))
        .child(
            h_flex()
                .gap(px(8.))
                .items_center()
                .children(entry.meta.clone().map(|meta| {
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(11.))
                        .text_color(palette::text_faint())
                        .child(meta)
                }))
                .child(render_trailing(entry)),
        )
        .child(
            div()
                .truncate()
                .text_size(px(14.))
                .text_color(palette::text())
                .child(entry.label.clone()),
        )
        .children(entry.detail.clone().map(|detail| {
            h_flex()
                .gap(px(6.))
                .items_center()
                .child(icon(entry.icon.clone(), px(12.), palette::text_faint()))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(px(11.))
                        .text_color(palette::text_faint())
                        .child(detail),
                )
        }))
}

/// The one-line shape, for lists long enough that three lines each would be a
/// wall: an archive, a table of contents, the agents that were found.
fn render_compact(entry: &Entry, active: bool) -> impl IntoElement + use<> {
    h_flex()
        .h(px(34.))
        .px(px(10.))
        .gap(px(9.))
        .items_center()
        .child(icon(
            entry.icon.clone(),
            px(14.),
            if active {
                palette::text()
            } else {
                palette::text_muted()
            },
        ))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(13.))
                .text_color(palette::text())
                .child(entry.label.clone()),
        )
        .child(render_trailing(entry))
}

/// The right-hand end of a row: a status if it has one, an age or a page if it
/// does not, and nothing at all otherwise.
fn render_trailing(entry: &Entry) -> impl IntoElement + use<> {
    let status = entry.status;
    let trailing = entry.trailing.clone();

    h_flex()
        .flex_shrink_0()
        .gap(px(6.))
        .items_center()
        .children(status.map(|status| {
            div()
                .size(px(6.))
                .rounded_full()
                .bg(status_color(status))
                .into_any_element()
        }))
        .children(status.map(|status| {
            div()
                .text_size(px(11.))
                .text_color(status_color(status))
                .child(status.label())
                .into_any_element()
        }))
        .children(trailing.filter(|_| status.is_none()).map(|trailing| {
            div()
                .text_size(px(11.))
                .text_color(palette::text_faint())
                .child(trailing)
                .into_any_element()
        }))
}

fn status_color(status: Status) -> Hsla {
    match status {
        Status::Working => palette::working(),
        Status::Failed => palette::danger(),
    }
}
