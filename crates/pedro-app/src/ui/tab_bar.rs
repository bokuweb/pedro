//! Open documents, one tab each, with the page layout toggles on the trailing
//! edge.
//!
//! The toggles used to have a bar of their own. Two horizontal bands above the
//! page is one more than the reading area can spare, and the toggles are about
//! the document the tabs name anyway.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use gpui_component::tooltip::Tooltip;
use gpui_component::{IconName, h_flex};

use crate::app::Pedro;
use crate::palette;
use crate::state::{OpenTab, PageLayout};
use crate::ui::{icon, square_button, vertical_rule};

const HEIGHT: f32 = 44.;

impl Pedro {
    pub(crate) fn render_tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tabs: Vec<_> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| self.render_tab(index, tab, cx))
            .collect();

        h_flex()
            .id("tab-bar")
            .h(px(HEIGHT))
            .flex_shrink_0()
            .items_center()
            .bg(palette::chrome())
            .border_b_1()
            .border_color(palette::border())
            .child(
                h_flex()
                    .id("tabs")
                    .h_full()
                    .flex_1()
                    .min_w_0()
                    .overflow_x_scroll()
                    .children(tabs),
            )
            .child(vertical_rule(px(20.)))
            .child(
                h_flex()
                    .flex_shrink_0()
                    .px(px(8.))
                    .gap(px(2.))
                    .child(self.render_layout_toggle(PageLayout::Single, cx))
                    .child(self.render_layout_toggle(PageLayout::Spread, cx)),
            )
    }

    fn render_layout_toggle(
        &self,
        layout: PageLayout,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let active = self.layout == layout;
        let label = layout.label();

        square_button(px(28.), active)
            .id(("layout", layout as usize))
            .child(icon(
                layout.icon(),
                px(15.),
                if active {
                    palette::text()
                } else {
                    palette::text_faint()
                },
            ))
            .tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
            .on_click(cx.listener(move |this, _, _, cx| this.set_layout(layout, cx)))
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
