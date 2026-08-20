//! The icon column on the far left: the brand mark and the panel switcher.

use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use gpui_component::tooltip::Tooltip;
use gpui_component::{IconName, v_flex};

use crate::app::Pedro;
use crate::palette;
use crate::state::RailItem;
use crate::ui::{icon, square_button};

const WIDTH: f32 = 56.;
const BUTTON: f32 = 36.;

impl Pedro {
    pub(crate) fn render_rail(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        // Built up front rather than inline: two `.children(..map(|..| ..cx))`
        // calls in one chain would hold two borrows of `cx` at once, because
        // the temporaries only drop at the end of the statement.
        let primary: Vec<_> = RailItem::PRIMARY
            .into_iter()
            .map(|item| self.render_rail_button(item, cx))
            .collect();
        let secondary: Vec<_> = RailItem::SECONDARY
            .into_iter()
            .map(|item| self.render_rail_button(item, cx))
            .collect();

        v_flex()
            .w(px(WIDTH))
            .h_full()
            .flex_shrink_0()
            .items_center()
            .gap(px(6.))
            .py(px(12.))
            .bg(palette::rail())
            .border_r_1()
            .border_color(palette::border())
            .child(render_logo())
            .child(div().h(px(8.)))
            .children(primary)
            .child(div().flex_1())
            .children(secondary)
    }

    fn render_rail_button(
        &self,
        item: RailItem,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let active = self.active_rail == item;
        let title = item.title();

        square_button(px(BUTTON), active)
            .id(("rail", item as usize))
            .child(icon(
                item.icon(),
                px(18.),
                if active {
                    palette::text()
                } else {
                    palette::text_muted()
                },
            ))
            .tooltip(move |window, cx| Tooltip::new(title).build(window, cx))
            .on_click(cx.listener(move |this, _, _, cx| this.select_rail(item, cx)))
    }
}

fn render_logo() -> impl IntoElement {
    div()
        .size(px(34.))
        .rounded(px(10.))
        .bg(palette::accent())
        .flex()
        .items_center()
        .justify_center()
        .child(icon(IconName::BookOpen, px(18.), palette::text()))
}
