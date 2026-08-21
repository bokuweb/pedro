//! The conversation about a passage, beside the page it is about.
//!
//! An answer is shown without its `## Sources` section: that section quotes
//! every passage in full, and it is already on screen as the badges under the
//! answer. A badge that resolved to a page is a button back into the book,
//! which is the whole point of asking the model to cite at all.
//!
//! Answers are markdown, because the prompt asks for headings, lists, tables
//! and fenced code and the model obliges. The reader's own words are drawn as
//! they were typed: a question is not a document, and silently reflowing
//! someone's asterisks is a surprise, not a feature.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use gpui_component::text::TextView;
use gpui_component::tooltip::Tooltip;
use gpui_component::{IconName, h_flex, v_flex};
use pedro_core::citation::strip_sources;
use pedro_core::model::{ChatMessage, Role};
use pedro_core::{Citation, CitationKind, PageLocation, PageMiss};

use crate::app::Pedro;
use crate::palette;
use crate::ui::icon;

const WIDTH: f32 = 380.;

impl Pedro {
    pub(crate) fn render_chat(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let chat = self.chat.as_ref()?;

        let turns: Vec<_> = chat
            .messages
            .iter()
            .map(|message| render_turn(message, window, cx))
            .collect();

        Some(
            v_flex()
                .w(px(WIDTH))
                .h_full()
                .flex_shrink_0()
                .border_l_1()
                .border_color(palette::border())
                .child(self.render_chat_header(chat.passage.clone(), chat.page, cx))
                .child(
                    v_flex()
                        .id("conversation")
                        .flex_1()
                        .min_h_0()
                        .p(px(12.))
                        .gap(px(12.))
                        .overflow_y_scroll()
                        .children(turns)
                        // The question is shown the moment it is asked, above
                        // an answer that has not started: a question that
                        // disappears into a spinner reads as one that was lost.
                        .children(chat.pending.clone().map(|question| {
                            render_bubble("pending", Role::User, question, Vec::new(), window, cx)
                        }))
                        .when(chat.is_answering(), |this| {
                            this.child(render_answer_in_progress(chat.visible(), window, cx))
                        })
                        .children(
                            chat.error
                                .clone()
                                .map(|why| self.render_failure(why, chat.sign_in, cx)),
                        )
                        // A passage that has been marked but not asked about
                        // yet: the panel is showing what the next question is
                        // about, and saying so is better than looking empty.
                        .when(
                            chat.messages.is_empty()
                                && !chat.is_answering()
                                && chat.error.is_none(),
                            |this| this.child(render_nothing_asked_yet()),
                        ),
                ),
        )
    }

    /// What went wrong, and the one thing that fixes it when there is one.
    fn render_failure(
        &self,
        why: SharedString,
        sign_in: Option<&'static str>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        v_flex()
            .px(px(12.))
            .py(px(9.))
            .gap(px(8.))
            .rounded(px(12.))
            .bg(palette::danger().opacity(0.14))
            .child(
                h_flex()
                    .gap(px(8.))
                    .items_start()
                    .child(icon(IconName::TriangleAlert, px(13.), palette::danger()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.))
                            .text_color(palette::danger())
                            .child(why),
                    ),
            )
            .children(sign_in.map(|command| {
                h_flex()
                    .id("sign-in")
                    .px(px(9.))
                    .py(px(4.))
                    .gap(px(6.))
                    .items_center()
                    .rounded(px(8.))
                    .bg(palette::surface())
                    .cursor_pointer()
                    .hover(|this| this.bg(palette::surface_hover()))
                    .child(icon(IconName::SquareTerminal, px(12.), palette::code()))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(palette::text())
                            .child(format!("Sign in — {command}")),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.sign_in(command, cx)))
            }))
    }

    fn render_chat_header(
        &self,
        passage: SharedString,
        page: u32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        h_flex()
            .flex_shrink_0()
            .px(px(12.))
            .py(px(10.))
            .gap(px(8.))
            .items_start()
            .border_b_1()
            .border_color(palette::separator())
            .child(icon(IconName::Star, px(13.), palette::code()))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(palette::text_faint())
                            .child(format!("p. {page}")),
                    )
                    .child(
                        div()
                            .max_h(px(66.))
                            .overflow_hidden()
                            .text_size(px(12.))
                            .text_color(palette::text_muted())
                            .child(passage),
                    ),
            )
            .child(
                div()
                    .id("close-chat")
                    .size(px(22.))
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .hover(|this| this.bg(palette::row_hover()))
                    .child(icon(IconName::Close, px(12.), palette::text_muted()))
                    .tooltip(move |window, cx| Tooltip::new("Close").build(window, cx))
                    .on_click(cx.listener(|this, _, _, cx| this.close_chat(cx))),
            )
    }
}

/// The panel with a subject and nothing said about it yet.
fn render_nothing_asked_yet() -> impl IntoElement {
    v_flex()
        .gap(px(4.))
        .child(
            div()
                .text_size(px(12.))
                .text_color(palette::text_muted())
                .child("Nothing asked about this passage yet."),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(palette::text_faint())
                .child("Type a question below. ⏎ sends it, ⇧⏎ breaks the line."),
        )
}

fn render_turn(
    message: &ChatMessage,
    window: &mut Window,
    cx: &mut Context<Pedro>,
) -> impl IntoElement + use<> {
    let body: SharedString = match message.role {
        Role::Assistant => strip_sources(&message.content).into(),
        Role::User => message.content.clone().into(),
    };

    render_bubble(
        &message.id,
        message.role,
        body,
        message.citations.clone(),
        window,
        cx,
    )
}

fn render_bubble(
    id: &str,
    role: Role,
    body: SharedString,
    citations: Vec<Citation>,
    window: &mut Window,
    cx: &mut Context<Pedro>,
) -> impl IntoElement + use<> {
    let reader = matches!(role, Role::User);
    let sources: Vec<_> = citations
        .into_iter()
        .map(|citation| render_citation(citation, cx))
        .collect();

    v_flex()
        .gap(px(6.))
        .child(
            div()
                .px(px(12.))
                .py(px(9.))
                .rounded(px(12.))
                .when(reader, |this| this.bg(palette::row_active()))
                .when(!reader, |this| this.bg(palette::surface()))
                .text_size(px(13.))
                .text_color(palette::text())
                .child(render_body(id, reader, body, window, cx)),
        )
        .when(!sources.is_empty(), |this| {
            this.child(h_flex().flex_wrap().gap(px(6.)).children(sources))
        })
}

/// The words of one turn.
///
/// An answer is markdown; a question is what the reader typed. The identifier
/// has to be stable across frames — the view keeps its parsed document under
/// it — and unique between turns, or two answers share one document.
fn render_body(
    id: &str,
    reader: bool,
    body: SharedString,
    window: &mut Window,
    cx: &mut Context<Pedro>,
) -> gpui::AnyElement {
    if reader {
        return div().child(body).into_any_element();
    }

    TextView::markdown(SharedString::from(format!("turn:{id}")), body, window, cx)
        .selectable(true)
        .into_any_element()
}

/// The answer as it is being written, with somewhere to stop it.
fn render_answer_in_progress(
    streaming: &str,
    window: &mut Window,
    cx: &mut Context<Pedro>,
) -> impl IntoElement + use<> {
    let written: SharedString = streaming.to_owned().into();
    let started = !streaming.is_empty();

    v_flex()
        .gap(px(6.))
        .child(
            div()
                .px(px(12.))
                .py(px(9.))
                .rounded(px(12.))
                .bg(palette::surface())
                .text_size(px(13.))
                .text_color(if started {
                    palette::text()
                } else {
                    palette::text_faint()
                })
                .child(if started {
                    // Rendered as it arrives, half-written markdown and all:
                    // a table that assembles itself is the point of streaming.
                    render_body("streaming", false, written, window, cx)
                } else {
                    // The first token can take a few seconds. Saying so is more
                    // use than a spinner.
                    div().child("Asking…").into_any_element()
                }),
        )
        .child(
            h_flex().child(
                div()
                    .id("stop-answering")
                    .px(px(9.))
                    .py(px(3.))
                    .rounded(px(7.))
                    .bg(palette::surface())
                    .text_size(px(11.))
                    .text_color(palette::text_muted())
                    .hover(|this| this.bg(palette::surface_hover()))
                    .child("Stop")
                    .on_click(cx.listener(|this, _, _, cx| this.stop_answering(cx))),
            ),
        )
}

/// One source under an answer.
///
/// A source the book holds is a button to its page. One it does not is shown
/// with the reason, because that is the reader's only sign that the model
/// reworded the passage rather than quoting it.
fn render_citation(citation: Citation, cx: &mut Context<Pedro>) -> impl IntoElement + use<> {
    let label: SharedString = match (citation.kind, citation.page) {
        (CitationKind::Web, _) => format!("[{}] web", citation.id).into(),
        (_, Some(PageLocation::Found(page))) => format!("[{}] p. {page}", citation.id).into(),
        (_, Some(PageLocation::Missed(miss))) => format!("[{}] {}", citation.id, why(miss)).into(),
        (_, None) => format!("[{}]", citation.id).into(),
    };

    let jump = match citation.page {
        Some(PageLocation::Found(page)) => Some(page),
        _ => None,
    };
    let hint: SharedString = citation
        .url
        .clone()
        .unwrap_or_else(|| citation.text.clone())
        .into();

    div()
        .id((
            "citation",
            citation.id.as_str().len() * 31 + jump.unwrap_or(0) as usize,
        ))
        .px(px(8.))
        .py(px(3.))
        .rounded(px(7.))
        .bg(palette::surface())
        .text_size(px(11.))
        .text_color(if jump.is_some() {
            palette::code()
        } else {
            palette::text_faint()
        })
        .when(jump.is_some(), |this| {
            this.cursor_pointer()
                .hover(|this| this.bg(palette::surface_hover()))
        })
        .child(label)
        .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
        .when_some(jump, |this, page| {
            this.on_click(cx.listener(move |this, _, _, cx| this.show_page(page, cx)))
        })
}

fn why(miss: PageMiss) -> &'static str {
    match miss {
        PageMiss::NoQuote => "no quote",
        PageMiss::NotInBook => "not in the book",
        PageMiss::SinglePageBook => "one page",
    }
}
