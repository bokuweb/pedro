//! The icon column on the far left: the panel switcher, under the traffic
//! lights.
//!
//! There is no brand mark. It sat at the top of this column and read as a
//! fourth button — an application does not need to tell the reader its own name
//! every time they look left, and the space is better spent on the space the
//! window controls need.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use gpui_component::tooltip::Tooltip;
use gpui_component::v_flex;

use crate::app::Pedro;
use crate::palette;
use crate::state::RailItem;
use crate::ui::{icon, square_button};

/// Wide enough to hold the macOS traffic lights, which sit in this column now
/// that there is no title bar for them to sit in. The cluster is 52 points
/// across, so `main.rs` starts it at 6 to centre it on the same axis as the
/// buttons below — an off-centre cluster is the first thing the eye picks up in
/// a column this narrow.
const WIDTH: f32 = 64.;
const BUTTON: f32 = 36.;

/// Top padding that clears the traffic lights. Fullscreen has none, so the
/// column starts where every other column does.
const LIGHTS: f32 = 40.;

impl Pedro {
    pub(crate) fn render_rail(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
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
            .pb(px(12.))
            .when(window.is_fullscreen(), |this| this.pt(px(12.)))
            .when(!window.is_fullscreen(), |this| this.pt(px(LIGHTS)))
            .bg(palette::rail())
            .border_r_1()
            .border_color(palette::border())
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
