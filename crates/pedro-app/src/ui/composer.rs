//! The ask bar under the reader.
//!
//! One rounded field with the answer's terms stated on its own trailing edge:
//! which CLI will answer, and whether it may look past the book. Both are
//! decisions a reader makes per question rather than once in a settings screen,
//! so they sit where the question is typed rather than behind a menu.
//!
//! Below it, a line of context: the document on the left, the place in it on
//! the right. Nothing there is clickable — it is the answer to "what am I
//! asking about?", which is a question the reader asks constantly and should
//! never have to click to answer.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use gpui_component::input::Input;
use gpui_component::tooltip::Tooltip;
use gpui_component::{IconName, h_flex, v_flex};

use crate::app::Pedro;
use crate::palette;
use crate::state::AgentStatus;
use crate::ui::icon;

impl Pedro {
    pub(crate) fn render_composer(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        v_flex()
            .flex_shrink_0()
            .px(px(16.))
            .pb(px(10.))
            .pt(px(6.))
            .gap(px(7.))
            .child(self.render_ask_field(cx))
            .child(self.render_context_line())
    }

    fn render_ask_field(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        h_flex()
            .w_full()
            .px(px(14.))
            .py(px(8.))
            .gap(px(8.))
            .items_center()
            .rounded(px(16.))
            .bg(palette::surface())
            .border_1()
            .border_color(palette::border())
            .child(
                // The field draws none of its own chrome: the rounded bar
                // around it is the control, and a second border inside the
                // first would read as two.
                Input::new(&self.composer)
                    .appearance(false)
                    .flex_1()
                    .min_w_0(),
            )
            .child(self.render_agent_chip())
            .child(self.render_web_search_chip(cx))
            .child(self.render_send_button(cx))
    }

    /// Which CLI is about to answer.
    fn render_agent_chip(&self) -> impl IntoElement + use<> {
        let label: SharedString = match &self.agent_status {
            AgentStatus::Detecting => "Looking…".into(),
            AgentStatus::Done(agents) => match agents.first() {
                Some(agent) => agent.kind.display_name().into(),
                None => "No agent CLI".into(),
            },
        };
        let tint = if self.agent_status.is_problem() {
            palette::danger()
        } else {
            palette::text_muted()
        };
        // The chip has room for one name; the headline has room for the fact
        // that there were three to choose from.
        let headline = self.agent_status.headline();

        h_flex()
            .id("agent-chip")
            .flex_shrink_0()
            .h(px(28.))
            .px(px(9.))
            .gap(px(6.))
            .items_center()
            .rounded(px(9.))
            .child(icon(self.agent_status.icon(), px(14.), tint))
            .child(div().text_size(px(12.)).text_color(tint).child(label))
            .tooltip(move |window, cx| Tooltip::new(headline.clone()).build(window, cx))
    }

    /// chatbook's web-search toggle. Here it decides which tools the CLI is
    /// started with rather than which endpoint is posted to.
    fn render_web_search_chip(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let on = self.web_search;

        h_flex()
            .id("web-search")
            .flex_shrink_0()
            .h(px(28.))
            .px(px(9.))
            .gap(px(6.))
            .items_center()
            .rounded(px(9.))
            .cursor_pointer()
            .when(on, |this| this.bg(palette::row_active()))
            .hover(|this| this.bg(palette::row_hover()))
            .child(icon(
                IconName::Globe,
                px(14.),
                if on {
                    palette::code()
                } else {
                    palette::text_faint()
                },
            ))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(if on {
                        palette::text()
                    } else {
                        palette::text_faint()
                    })
                    .child("Web"),
            )
            .tooltip(move |window, cx| {
                let hint = if on {
                    "The agent may search the web"
                } else {
                    "The agent answers from the book alone"
                };
                Tooltip::new(hint).build(window, cx)
            })
            .on_click(cx.listener(|this, _, _, cx| this.toggle_web_search(cx)))
    }

    fn render_send_button(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        div()
            .id("ask")
            .flex_shrink_0()
            .size(px(30.))
            .rounded_full()
            .bg(palette::text())
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(|this| this.bg(palette::code()))
            .child(icon(IconName::ArrowUp, px(16.), palette::ink()))
            .tooltip(move |window, cx| Tooltip::new("Ask").build(window, cx))
            .on_click(cx.listener(|this, _, window, cx| this.ask(window, cx)))
    }

    /// What the question is about: the document, and the place in it.
    fn render_context_line(&self) -> impl IntoElement + use<> {
        let document: SharedString = match self.active_tab() {
            // The page matters as much as the book: a question is about the
            // passage in front of the reader, not about the whole volume.
            Some(tab) => match self.open_document() {
                Some(open) => format!("{} · {}", tab.label, open.position()).into(),
                None => tab.label.clone(),
            },
            None => "No document open".into(),
        };

        h_flex()
            .w_full()
            .px(px(6.))
            .items_center()
            .justify_between()
            .child(
                h_flex()
                    .min_w_0()
                    .gap(px(6.))
                    .items_center()
                    .child(icon(IconName::BookOpen, px(12.), palette::text_faint()))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(palette::text_faint())
                            .child(document),
                    ),
            )
            .child(
                h_flex()
                    .flex_shrink_0()
                    .gap(px(6.))
                    .items_center()
                    .child(icon(self.layout.icon(), px(12.), palette::text_faint()))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(palette::text_faint())
                            .child(self.layout.label()),
                    ),
            )
    }
}
