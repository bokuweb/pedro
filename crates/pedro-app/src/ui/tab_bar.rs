//! Open documents, one tab each, between the two buttons that open and shut
//! the panels either side of them.
//!
//! When the sidebar is shut this row runs to the left edge of the window, where
//! the traffic lights are, so it is indented by whatever the sidebar has left
//! uncovered — which slides with the panel rather than jumping when it lands.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use gpui_component::tooltip::Tooltip;
use gpui_component::{IconName, h_flex};

use crate::app::Pedro;
use crate::palette;
use crate::state::OpenTab;
use crate::ui::icon;

const HEIGHT: f32 = 44.;

/// The room the macOS traffic lights need at the left edge of the window.
const LIGHTS: f32 = 84.;

/// How wide a tab title is allowed to grow. Book filenames run long, and
/// without a ceiling one of them pushes every other tab out of the row.
const LABEL_MAX: f32 = 180.;

impl Pedro {
    pub(crate) fn render_tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tabs: Vec<_> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| self.render_tab(index, tab, cx))
            .collect();

        // What the sidebar does not cover, this row has to leave for the
        // window controls.
        let clearance = (LIGHTS - self.sidebar.width).max(6.);

        let bar = h_flex()
            .id("tab-bar")
            .h(px(HEIGHT))
            .flex_shrink_0()
            .items_center()
            .pl(px(clearance))
            .pr(px(6.))
            .gap(px(4.))
            .bg(palette::chrome())
            .border_b_1()
            .border_color(palette::border())
            .child(self.render_panel_toggle(
                "toggle-sidebar",
                if self.sidebar.is_open() {
                    IconName::PanelLeftClose
                } else {
                    IconName::PanelLeftOpen
                },
                "Books",
                Box::new(cx.listener(|this, _, window, cx| {
                    this.toggle_sidebar(&Default::default(), window, cx)
                })),
            ))
            .child(
                h_flex()
                    .id("tabs")
                    .h_full()
                    .flex_1()
                    .min_w_0()
                    .overflow_x_scroll()
                    .children(tabs),
            )
            .child(self.render_panel_toggle(
                "toggle-chat",
                if self.chat_pane.is_open() {
                    IconName::PanelRightClose
                } else {
                    IconName::PanelRightOpen
                },
                "Conversation",
                Box::new(cx.listener(|this, _, window, cx| {
                    this.toggle_chat(&Default::default(), window, cx)
                })),
            ));

        // The other row along the top edge, and the one a reader is most likely
        // to reach for when there is no title bar to grab.
        self.draggable(bar, cx)
    }

    /// One of the two buttons that open and shut a panel.
    fn render_panel_toggle(
        &self,
        id: &'static str,
        name: IconName,
        what: &'static str,
        on_click: Box<dyn Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static>,
    ) -> impl IntoElement + use<> {
        div()
            .id(id)
            .size(px(28.))
            .flex_shrink_0()
            .rounded(px(8.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(|this| this.bg(palette::row_hover()))
            .child(icon(name, px(15.), palette::text_muted()))
            .tooltip(move |window, cx| Tooltip::new(what).build(window, cx))
            .on_click(on_click)
    }

    fn render_tab(
        &self,
        index: usize,
        tab: &OpenTab,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let active = self.active_tab == Some(index);

        h_flex()
            .id(("tab", index))
            .h_full()
            .flex_shrink_0()
            .pl(px(16.))
            .pr(px(8.))
            .gap(px(8.))
            .items_center()
            .cursor_pointer()
            .bg(if active {
                palette::canvas()
            } else {
                palette::chrome()
            })
            .when(!active, |this| {
                this.hover(|this| this.bg(palette::row_hover()))
            })
            .on_click(cx.listener(move |this, _, _, cx| this.activate_tab(index, cx)))
            .child(icon(
                IconName::File,
                px(15.),
                if active {
                    palette::text_muted()
                } else {
                    palette::text_faint()
                },
            ))
            .child(
                div()
                    .max_w(px(LABEL_MAX))
                    .min_w_0()
                    .truncate()
                    .text_size(px(14.))
                    .text_color(if active {
                        palette::text()
                    } else {
                        palette::text_muted()
                    })
                    .child(tab.label.clone()),
            )
            .child(
                div()
                    .id(("tab-close", index))
                    .size(px(22.))
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .hover(|this| this.bg(palette::surface()))
                    .child(icon(IconName::Close, px(12.), palette::text_muted()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        // Otherwise the tab underneath would activate itself
                        // on the way out.
                        cx.stop_propagation();
                        this.close_tab(index, cx);
                    })),
            )
    }
}
