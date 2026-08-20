//! The panel between the rail and the reader: search, a panel header, and a
//! list of collapsible sections.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use gpui_component::input::Input;
use gpui_component::tooltip::Tooltip;
use gpui_component::{IconName, WindowExt as _, h_flex, v_flex};

use crate::app::Pedro;
use crate::palette;
use crate::state::{Entry, Section};
use crate::ui::icon;

const WIDTH: f32 = 300.;
const ROW_HEIGHT: f32 = 46.;
const SECTION_HEIGHT: f32 = 44.;

impl Pedro {
    pub(crate) fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        v_flex()
            .w(px(WIDTH))
            .h_full()
            .flex_shrink_0()
            .bg(palette::sidebar())
            .border_r_1()
            .border_color(palette::border())
            .child(self.render_search())
            .child(self.render_panel_header())
            .child(self.render_sections(cx))
    }

    fn render_search(&self) -> impl IntoElement + use<> {
        div().px(px(12.)).py(px(10.)).child(
            Input::new(&self.search)
                .h(px(38.))
                .prefix(icon(IconName::Search, px(16.), palette::text_muted()))
                .suffix(
                    div()
                        .px(px(6.))
                        .py(px(2.))
                        .rounded(px(5.))
                        .bg(palette::surface_hover())
                        .text_size(px(11.))
                        .text_color(palette::text_muted())
                        .child("⌘K"),
                ),
        )
    }

    fn render_panel_header(&self) -> impl IntoElement + use<> {
        let title = self.active_rail.title();
        let hint = self.active_rail.hint();

        h_flex()
            .id("panel-header")
            .h(px(56.))
            .px(px(16.))
            .items_center()
            .justify_between()
            .bg(palette::sidebar_header())
            .border_t_1()
            .border_b_1()
            .border_color(palette::separator())
            .child(
                div()
                    .text_size(px(19.))
                    .text_color(palette::text())
                    .child(title),
            )
            .child(
                div()
                    .id("panel-hint")
                    .child(icon(IconName::Info, px(17.), palette::text_faint()))
                    .tooltip(move |window, cx| Tooltip::new(hint).build(window, cx)),
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
            // section was collapsed by hand.
            let expanded = section.expanded || !query.is_empty();
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
            .h(px(SECTION_HEIGHT))
            .px(px(16.))
            .gap(px(8.))
            .items_center()
            .bg(palette::sidebar_header())
            .border_b_1()
            .border_color(palette::separator())
            .hover(|this| this.bg(palette::row_hover()))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_section(index, cx)))
            .child(icon(chevron, px(14.), palette::text_muted()))
            .child(
                div()
                    .flex_1()
                    .text_size(px(15.))
                    .text_color(palette::text())
                    .child(section.title.clone()),
            )
            .when(section.addable, |this| this.child(render_add_button(index)))
    }

    fn render_entry(&self, entry: &Entry, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let active = self.active_tab().is_some_and(|tab| tab.id == entry.id);
        let clickable = entry.openable;
        let on_open = entry.clone();

        h_flex()
            .id(entry.id.clone())
            .h(px(ROW_HEIGHT))
            .pl(px(30.))
            .pr(px(16.))
            .gap(px(10.))
            .items_center()
            .bg(if active {
                palette::row_active()
            } else {
                palette::row()
            })
            .border_b_1()
            .border_color(palette::separator())
            .when(clickable, |this| {
                this.cursor_pointer()
                    .hover(|this| this.bg(palette::row_hover()))
                    .on_click(cx.listener(move |this, _, _, cx| this.open_entry(&on_open, cx)))
            })
            .child(icon(
                entry.icon.clone(),
                px(16.),
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
                    .text_color(palette::text())
                    .child(entry.label.clone()),
            )
            .children(entry.detail.clone().map(|detail| {
                div()
                    .flex_shrink_0()
                    .text_size(px(12.))
                    .text_color(palette::text_faint())
                    .child(detail)
            }))
    }
}

/// The circled plus in a section header.
fn render_add_button(index: usize) -> impl IntoElement {
    div()
        .id(("section-add", index))
        .size(px(22.))
        .rounded_full()
        .border_1()
        .border_color(palette::text_faint())
        .flex()
        .items_center()
        .justify_center()
        .hover(|this| this.border_color(palette::text_muted()))
        .child(icon(IconName::Plus, px(12.), palette::text_muted()))
        .tooltip(move |window, cx| Tooltip::new("Add").build(window, cx))
        .on_click(|_, window, cx| {
            // Deliberately loud: the affordance exists in the layout before
            // the document store does.
            cx.stop_propagation();
            window.push_notification("Adding documents is not implemented yet.", cx);
        })
}
