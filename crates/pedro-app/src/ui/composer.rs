//! The ask bar under the reader.
//!
//! A marked passage rides in the field as a chip, the way an attachment does:
//! it belongs to the message being written, and it goes when the message goes.
//! What the passage *is* belongs to the panel beside the page, which is where
//! it stays for as long as the conversation about it does.
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
    StatefulInteractiveElement as _, Styled as _, div, px, relative,
};
use gpui_component::input::Input;
use gpui_component::tooltip::Tooltip;
use gpui_component::{IconName, h_flex, v_flex};

use crate::app::Pedro;
use crate::palette;
use crate::state::AgentStatus;
use crate::ui::icon;

/// How much of a marked passage the chip shows. One line of a field that is
/// mostly there for the question.
const CHIP_LENGTH: usize = 48;

/// `text` on one line, cut to `limit` characters at a word boundary where
/// there is one.
fn shorten(text: &str, limit: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= limit {
        return text;
    }

    let kept: String = text.chars().take(limit).collect();
    let kept = kept
        .rsplit_once(' ')
        .map_or(kept.as_str(), |(head, _)| head);

    format!("{kept}…")
}

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
        v_flex()
            .w_full()
            .px(px(14.))
            .py(px(8.))
            .gap(px(7.))
            .rounded(px(16.))
            .bg(palette::surface())
            .border_1()
            .border_color(palette::border())
            .children(self.render_passage_chips(cx))
            .child(self.render_ask_row(cx))
    }

    /// The passages attached to the question being written.
    fn render_passage_chips(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        if self.attached.is_empty() {
            return None;
        }

        let chips: Vec<_> = self
            .attached
            .iter()
            .enumerate()
            .map(|(at, passage)| self.render_passage_chip(at, passage, cx))
            .collect();

        Some(h_flex().flex_wrap().gap(px(5.)).children(chips))
    }

    fn render_passage_chip(
        &self,
        at: usize,
        passage: &pedro_core::model::NewHighlight,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        h_flex()
            .id(("passage-chip", at))
            .max_w(relative(1.0))
            .px(px(8.))
            .py(px(3.))
            .gap(px(6.))
            .items_center()
            .rounded(px(8.))
            .bg(palette::row_active())
            .child(icon(IconName::Star, px(11.), palette::code()))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(palette::text_faint())
                    .child(format!("p. {}", passage.page_number)),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(11.))
                    .text_color(palette::text_muted())
                    .child(shorten(&passage.selected_text, CHIP_LENGTH)),
            )
            .child(icon(IconName::Close, px(11.), palette::text_faint()))
            .tooltip(move |window, cx| Tooltip::new("Put this passage down").build(window, cx))
            .on_click(cx.listener(move |this, _, _, cx| this.detach_passage(at, cx)))
    }

    fn render_ask_row(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        h_flex()
            .w_full()
            .gap(px(8.))
            .items_center()
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
        let label: SharedString = match (&self.agent_status, self.answering_agent()) {
            (AgentStatus::Detecting, _) => "Looking…".into(),
            (_, Some(agent)) => agent.kind.display_name().into(),
            (_, None) => "No agent CLI".into(),
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
        let zoomed = ((self.zoom - 1.0).abs() > 0.001).then_some(self.zoom);
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
                    .gap(px(10.))
                    .items_center()
                    .children(self.open_document().map(|open| {
                        div()
                            .text_size(px(11.))
                            .text_color(palette::text_faint())
                            .child(format!("{} pages", open.page_count))
                    }))
                    // Only when it is not the size a page is normally drawn at:
                    // a reader who has not zoomed does not need telling.
                    .children(zoomed.map(|zoom| {
                        h_flex()
                            .gap(px(5.))
                            .items_center()
                            .child(icon(IconName::Frame, px(12.), palette::text_faint()))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(palette::text_faint())
                                    .child(format!("{:.0}%", zoom * 100.0)),
                            )
                    })),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_passage_is_left_alone() {
        assert_eq!(shorten("a short passage", 48), "a short passage");
    }

    /// A passage picked off a page arrives with the line breaks the page put in
    /// it, which are not part of the sentence.
    #[test]
    fn the_page_layout_is_squeezed_out_of_the_chip() {
        assert_eq!(shorten("two\nlines   here", 48), "two lines here");
    }

    #[test]
    fn a_long_passage_is_cut_at_a_word() {
        assert_eq!(shorten("alpha beta gamma delta", 14), "alpha beta…");
    }

    /// Japanese does not put spaces between words, so there is no boundary to
    /// cut at and the limit has to be enough on its own.
    #[test]
    fn a_passage_with_no_spaces_is_cut_at_the_limit() {
        assert_eq!(shorten("エッジで動きます", 4), "エッジで…");
    }

    #[test]
    fn cutting_counts_characters_not_bytes() {
        assert_eq!(shorten("あいうえお", 5), "あいうえお");
    }
}
