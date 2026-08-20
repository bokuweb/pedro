//! The bar above the tabs: page layout toggles and the agent status pill.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use gpui_component::tooltip::Tooltip;
use gpui_component::{IconName, h_flex};

use crate::app::Pedro;
use crate::palette;
use crate::state::PageLayout;
use crate::ui::{icon, square_button, vertical_rule};

const HEIGHT: f32 = 52.;

impl Pedro {
    pub(crate) fn render_top_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        h_flex()
            .h(px(HEIGHT))
            .flex_shrink_0()
            .items_center()
            .gap(px(10.))
            .px(px(12.))
            .bg(palette::chrome())
            .border_b_1()
            .border_color(palette::border())
            .child(
                h_flex()
                    .gap(px(2.))
                    .child(self.render_layout_toggle(PageLayout::Single, cx))
                    .child(self.render_layout_toggle(PageLayout::Spread, cx)),
            )
            .child(vertical_rule(px(22.)))
            .when(!self.status_dismissed, |this| {
                this.child(self.render_status_pill(cx))
            })
    }

    fn render_layout_toggle(
        &self,
        layout: PageLayout,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let active = self.layout == layout;
        let label = layout.label();

        square_button(px(34.), active)
            .id(("layout", layout as usize))
            .child(icon(
                layout.icon(),
                px(17.),
                if active {
                    palette::text()
                } else {
                    palette::text_muted()
                },
            ))
            .tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
            .on_click(cx.listener(move |this, _, _, cx| this.set_layout(layout, cx)))
    }

    /// Mirrors the "connect your domain" pill in the reference design, but
    /// carries something pedro actually knows: which agent CLI it found.
    fn render_status_pill(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let status = &self.agent_status;
        let tint = if status.is_problem() {
            palette::danger()
        } else {
            palette::text_muted()
        };

        h_flex()
            .h(px(36.))
            .pl(px(12.))
            .pr(px(6.))
            .gap(px(8.))
            .items_center()
            .rounded(px(8.))
            .bg(palette::surface())
            .child(icon(status.icon(), px(16.), tint))
            .child(
                div()
                    .text_size(px(14.))
                    .text_color(palette::text())
                    .child(status.headline()),
            )
            .child(
                div()
                    .id("dismiss-status")
                    .size(px(24.))
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .hover(|this| this.bg(palette::surface_hover()))
                    .child(icon(IconName::Close, px(13.), palette::text_muted()))
                    .on_click(cx.listener(|this, _, _, cx| this.dismiss_status(cx))),
            )
    }
}
