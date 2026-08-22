//! Reading the sources off an answer, and finding the page each one came from.
//!
//! A port of the citation half of chatbook's `chatService.ts`, tests included.
//! The model is asked to end its answer with a `## Sources` section; this turns
//! that section into citations, and a quoted passage into the page it sits on,
//! which is what makes "jump back to where this came from" possible.
//!
//! Everything here is deliberately forgiving. The model does not reproduce a
//! passage character for character, and it writes links in more shapes than the
//! prompt asks for, so both lookups fall back rather than fail: a failed
//! whole-quote match is retried in fragments, and a source with no page says
//! *why* — that reason is the reader's only hint that the model reworded the
//! passage rather than quoting it.

use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::excerpt::PAGE_DELIMITER;

/// Length of the fragments used when a quote does not appear verbatim, and how
/// far apart they start. Long enough to be unique in a book, short enough to
/// survive the model rewording a clause.
const FRAGMENT_LENGTH: usize = 24;
const FRAGMENT_STEP: usize = 12;

/// Why a quoted passage has no page.
///
/// Each one is a different thing to tell the reader: a quote that is nowhere in
/// the book is a sign the model reworded it, while a book of one page simply
/// has nowhere to jump to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PageMiss {
    NoQuote,
    NotInBook,
    SinglePageBook,
}

/// Where a quoted passage lives, or why it has no page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PageLocation {
    Found(u32),
    Missed(PageMiss),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CitationKind {
    Pdf,
    Web,
}

/// A source the assistant named in its `## Sources` section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Citation {
    /// The `n` of the `[n]` in the answer's body.
    pub id: String,
    pub kind: CitationKind,
    pub text: String,
    /// Where the passage was found. `None` on web sources, and on book sources
    /// parsed without the book's text to look in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<PageLocation>,
    /// Which book the page is in, when the passage was found in one. Absent on
    /// citations stored before a conversation could span several books, which
    /// is why the reader's old conversations still load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub book: Option<CitedBook>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// A book a citation may be looked up in.
#[derive(Debug, Clone, Copy)]
pub struct BookText<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub full_text: &'a str,
    pub page_count: u32,
}

/// Which book a citation was found in.
///
/// Carried on the citation rather than left to the caller, because a question
/// put to a shelf is answered from several books at once and "page 120" is not
/// a place until it says page 120 of what.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CitedBook {
    pub id: String,
    pub title: String,
}

/// A quoted block, each opening mark closed by its own kind so a passage that
/// carries an apostrophe is not cut at it. None of them nest: the model writes
/// quotes side by side, never one inside another.
static QUOTED_BLOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new("「[^「」]+」|\"[^\"]+\"|“[^”]+”|'[^']+'").expect("a valid pattern")
});

/// The link in a Sources entry, looked for outside what the entry quotes.
///
/// The model writes the link in more shapes than the prompt asks for — after an
/// em dash, in parentheses, behind a title that carries a hyphen of its own —
/// so requiring one fixed separator misread web sources as passages of the book
/// and sent them to be looked up in it, which cost the reader the link.
///
/// A book about the web prints urls in its own body, so the quoted blocks are
/// dropped before the search: what makes a source a web one is that its url
/// stands outside the quotation marks.
static URL_OUTSIDE_QUOTES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("https?://[^\\s)）」』\"'、。]+").expect("a valid pattern"));

static SOURCES_SECTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)## Sources\n(.*)$").expect("a valid pattern"));

static SOURCE_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\[(\d+)\]\s+(.+)$").expect("a valid pattern"));

/// The trailing `## Sources` section, which the body is shown without.
static SOURCES_TAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)\n*^## Sources[ \t]*$[\s\S]*").expect("a valid pattern"));

/// Removes the trailing `## Sources` section from an answer.
///
/// The section is what the citation badges are built from, so showing it in the
/// body as well would repeat every quoted passage in full — and so would
/// sending it back as conversation history. The stored message keeps the raw
/// text; the rendering and the history handed to the agent both drop it.
pub fn strip_sources(content: &str) -> String {
    SOURCES_TAIL.replace(content, "").trim_end().to_owned()
}

/// Reads the `## Sources` section of an answer into citations.
///
/// Book sources are resolved to a page when `book` is given; without it they
/// carry neither a page nor a reason, because nothing was looked up.
pub fn parse_citations(response_text: &str, books: &[BookText<'_>]) -> Vec<Citation> {
    let Some(sources) = SOURCES_SECTION.captures(response_text) else {
        return Vec::new();
    };

    sources[1]
        .lines()
        .filter_map(|line| SOURCE_LINE.captures(line))
        .map(|entry| {
            let id = entry[1].to_owned();
            let content = entry[2].trim();

            match url_outside_quotes(content) {
                Some(url) => Citation {
                    id,
                    kind: CitationKind::Web,
                    text: describe_web_source(content, &url),
                    page: None,
                    book: None,
                    url: Some(url),
                },
                None => {
                    let quoted = extract_quoted_text(content);
                    let (page, book) = locate(&quoted, books);
                    Citation {
                        id,
                        page,
                        book,
                        kind: CitationKind::Pdf,
                        text: quoted,
                        url: None,
                    }
                }
            }
        })
        .collect()
}

/// Finds which of `books` a quotation is in, and where.
///
/// The first book that holds it wins. A passage quoted from a shelf could in
/// principle be in two of its books — a quotation of a standard, an epigraph —
/// and picking the first is the same answer the reader would get by opening
/// them in order; the alternative is showing them two places for one source,
/// which is worse than showing them one of two.
///
/// A quotation in none of them reports the miss from the first book, which is
/// the reason the reader can act on: the model reworded the passage.
fn locate(quoted: &str, books: &[BookText<'_>]) -> (Option<PageLocation>, Option<CitedBook>) {
    let mut first_miss = None;

    for book in books {
        match find_page_number(quoted, book.full_text, book.page_count) {
            found @ PageLocation::Found(_) => {
                return (
                    Some(found),
                    Some(CitedBook {
                        id: book.id.to_owned(),
                        title: book.title.to_owned(),
                    }),
                );
            }
            missed => first_miss = first_miss.or(Some(missed)),
        }
    }

    (first_miss, None)
}

fn url_outside_quotes(entry: &str) -> Option<String> {
    let unquoted = QUOTED_BLOCK.replace_all(entry, "");
    URL_OUTSIDE_QUOTES
        .find(&unquoted)
        .map(|url| url.as_str().to_owned())
}

/// The passage a Sources entry quotes, or the entry itself when it quotes
/// nothing.
///
/// The model writes `「passage」（本書 第1章）`, so the trailing note has to be
/// dropped before the passage can be looked up. It also names the section it is
/// quoting from, and quotes more than once. Reading the entry as one block from
/// its first mark to its last stitched those together into a string the book
/// does not hold, which cost the reader the page as well as the mark on it.
///
/// The last block is the passage. The section is named before what is quoted
/// from it, so the order tells the two apart where their length does not — a
/// section title can be the longer of the two.
fn extract_quoted_text(entry: &str) -> String {
    match QUOTED_BLOCK.find_iter(entry).last() {
        Some(block) => unquote(block.as_str()).to_owned(),
        None => entry.to_owned(),
    }
}

/// What a web entry is about: the passage it quotes, or the page's title when
/// it quotes nothing.
///
/// The first block is the one to take, the opposite of a book entry: here the
/// quote comes first and the page that carries it is named after, and that
/// title may be quoted too (`「Backend for Frontend Pattern」`).
fn describe_web_source(entry: &str, url: &str) -> String {
    if let Some(block) = QUOTED_BLOCK.find(entry) {
        return unquote(block.as_str()).to_owned();
    }

    // Nothing quoted: the entry is the title, with the link and the punctuation
    // that introduced it left behind. A bracket goes only when it is the one
    // holding the link — a title of its own can close on one (`比較（…低コスト）`)
    // and reads as a sentence cut short without it.
    let Some(start) = entry.find(url) else {
        return entry.to_owned();
    };

    let before = entry[..start].trim_end();
    let after = &entry[start + url.len()..];
    let bracketed = before.ends_with(['(', '（']) && after.trim_start().starts_with([')', '）']);

    let title = if bracketed {
        &before[..before.len() - before.chars().next_back().map_or(0, char::len_utf8)]
    } else {
        before
    };

    title
        .trim_end_matches([
            ' ', '\t', '\n', '-', '—', '–', ':', '：', '、', '。', ',', '.',
        ])
        .trim()
        .to_owned()
}

/// Drops the opening and closing marks of a quoted block.
fn unquote(block: &str) -> &str {
    let mut chars = block.chars();
    chars.next();
    chars.next_back();
    chars.as_str()
}

/// The page a quoted passage sits on, found by searching each page's text.
///
/// The model rarely reproduces a passage character for character, so a failed
/// whole-quote match falls back to fragments of it. A text with no page seams
/// at all is searched as one string and the page estimated by position.
pub fn find_page_number(text: &str, full_text: &str, page_count: u32) -> PageLocation {
    let needle = normalize(text);
    if needle.is_empty() {
        return PageLocation::Missed(PageMiss::NoQuote);
    }
    if page_count <= 1 {
        return PageLocation::Missed(PageMiss::SinglePageBook);
    }

    let pages: Vec<&str> = full_text.split(PAGE_DELIMITER).collect();
    if pages.len() <= 1 {
        return locate_by_position(&needle, full_text, page_count);
    }

    let normalized: Vec<String> = pages.iter().map(|page| normalize(page)).collect();

    if let Some(page) = page_containing(&normalized, &needle) {
        return PageLocation::Found(page);
    }

    // A quote can start near the bottom of a page and finish on the next one.
    for (index, pair) in normalized.windows(2).enumerate() {
        if format!("{}{}", pair[0], pair[1]).contains(&needle) {
            return PageLocation::Found(index as u32 + 1);
        }
    }

    // Scan fragments from the start of the quote, so the first hit is the page
    // the passage begins on.
    let characters: Vec<char> = needle.chars().collect();
    for start in (0..).step_by(FRAGMENT_STEP) {
        if start + FRAGMENT_LENGTH > characters.len() {
            break;
        }

        let fragment: String = characters[start..start + FRAGMENT_LENGTH].iter().collect();
        if let Some(page) = page_containing(&normalized, &fragment) {
            return PageLocation::Found(page);
        }
    }

    PageLocation::Missed(PageMiss::NotInBook)
}

/// The one-based page whose text contains the needle.
fn page_containing(pages: &[String], needle: &str) -> Option<u32> {
    pages
        .iter()
        .position(|page| page.contains(needle))
        .map(|index| index as u32 + 1)
}

/// Where a passage sits in a text with no page seams, as a fraction of it.
///
/// Records stored before the extractor delimited pages have no seams to count,
/// and an estimate that lands on the right page most of the time is worth more
/// to the reader than no link at all.
fn locate_by_position(needle: &str, full_text: &str, page_count: u32) -> PageLocation {
    let whole = normalize(full_text);
    let Some(byte_index) = whole.find(needle) else {
        return PageLocation::Missed(PageMiss::NotInBook);
    };

    let character_index = whole[..byte_index].chars().count() as f64;
    let page_size = whole.chars().count() as f64 / page_count as f64;
    let page = (character_index / page_size).floor() as u32 + 1;

    PageLocation::Found(page.min(page_count))
}

/// Whitespace is where the quote and the extracted text diverge: the extractor
/// separates text runs, while the model quotes the passage as it reads.
fn normalize(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_text_of(pages: &[&str]) -> String {
        pages.join(&PAGE_DELIMITER.to_string())
    }

    const TITLE: &str = "A Book";

    fn book<'a>(full_text: &'a str, page_count: u32) -> [BookText<'a>; 1] {
        [BookText {
            id: "book-1",
            title: TITLE,
            full_text,
            page_count,
        }]
    }

    /// The book a found page is in. Every one-book test looks it up in the same
    /// book, so a citation that found its page names that one.
    fn from_book(page: Option<PageLocation>) -> Option<CitedBook> {
        matches!(page, Some(PageLocation::Found(_))).then(|| CitedBook {
            id: "book-1".to_owned(),
            title: TITLE.to_owned(),
        })
    }

    fn pdf(id: &str, text: &str, page: Option<PageLocation>) -> Citation {
        Citation {
            id: id.to_owned(),
            kind: CitationKind::Pdf,
            text: text.to_owned(),
            book: from_book(page),
            page,
            url: None,
        }
    }

    fn web(id: &str, text: &str, url: &str) -> Citation {
        Citation {
            id: id.to_owned(),
            kind: CitationKind::Web,
            text: text.to_owned(),
            page: None,
            book: None,
            url: Some(url.to_owned()),
        }
    }

    fn found(page: u32) -> Option<PageLocation> {
        Some(PageLocation::Found(page))
    }

    mod find_page_number {
        use super::*;

        #[test]
        fn reports_the_page_a_quoted_passage_was_found_on() {
            let full_text = full_text_of(&["まえがき", "エッジ で 動く"]);
            assert_eq!(
                find_page_number("エッジで動く", &full_text, 2),
                PageLocation::Found(2)
            );
        }

        #[test]
        fn reports_a_passage_the_book_does_not_contain() {
            let full_text = full_text_of(&["まえがき", "エッジ で 動く"]);
            assert_eq!(
                find_page_number("この本にない一文", &full_text, 2),
                PageLocation::Missed(PageMiss::NotInBook)
            );
        }

        #[test]
        fn reports_a_quote_that_is_only_whitespace_as_having_no_text() {
            let full_text = full_text_of(&["まえがき", "エッジ で 動く"]);
            assert_eq!(
                find_page_number("  \n ", &full_text, 2),
                PageLocation::Missed(PageMiss::NoQuote)
            );
        }

        #[test]
        fn blames_the_empty_quote_rather_than_the_page_count() {
            assert_eq!(
                find_page_number("  ", "エッジ で 動く", 1),
                PageLocation::Missed(PageMiss::NoQuote)
            );
        }

        #[test]
        fn says_a_book_of_one_page_has_nowhere_to_jump_to() {
            assert_eq!(
                find_page_number("エッジで動く", "エッジ で 動く", 1),
                PageLocation::Missed(PageMiss::SinglePageBook)
            );
        }

        /// A text with no seams is searched by ratio, so the passage has to sit
        /// in the second half to land on page 2.
        const UNDELIMITED: &str = "まえがき の ながい はじめに エッジ で 動く";

        #[test]
        fn estimates_the_page_of_a_passage_an_undelimited_book_holds() {
            assert_eq!(
                find_page_number("エッジで動く", UNDELIMITED, 2),
                PageLocation::Found(2)
            );
        }

        #[test]
        fn reports_a_passage_an_undelimited_book_does_not_hold() {
            assert_eq!(
                find_page_number("この本にない一文", UNDELIMITED, 2),
                PageLocation::Missed(PageMiss::NotInBook)
            );
        }
    }

    mod parse_citations {
        use super::*;

        #[test]
        fn resolves_the_page_of_a_japanese_quoted_book_citation() {
            let full_text = full_text_of(&[
                "まえがき",
                "第1章 Cloudflare Workers とは",
                "エッジ は サーバーレス 実行基盤 です",
            ]);
            let response = "エッジで動きます[1]\n\n## Sources\n[1] 「エッジはサーバーレス実行基盤です」（本書 第3章 3.1）";

            assert_eq!(
                parse_citations(response, &book(&full_text, 3)),
                vec![pdf("1", "エッジはサーバーレス実行基盤です", found(3))]
            );
        }

        #[test]
        fn resolves_a_passage_split_across_two_pages_to_the_page_it_starts_on() {
            let full_text = full_text_of(&[
                "まえがき",
                "Workers は グローバル",
                "ネットワーク で 動きます",
            ]);
            let response =
                "本文[1]\n\n## Sources\n[1] 「Workersはグローバルネットワークで動きます」";

            assert_eq!(
                parse_citations(response, &book(&full_text, 3)),
                vec![pdf(
                    "1",
                    "Workersはグローバルネットワークで動きます",
                    found(2)
                )]
            );
        }

        #[test]
        fn still_finds_the_page_when_the_model_paraphrased_part_of_the_passage() {
            let full_text = full_text_of(&[
                "まえがき",
                "TLS の ハンドシェイク では、クライアント ／ サーバ間 での ラウンドトリップ が発生するため、一定の時間が必要となります",
            ]);
            // The opening was reworded, but the rest is quoted from the page.
            let response = "本文[1]\n\n## Sources\n[1] 「TLSハンドシェイク処理では、クライアント／サーバ間でのラウンドトリップが発生するため、一定の時間が必要となります」";

            assert_eq!(
                parse_citations(response, &book(&full_text, 2)),
                vec![pdf(
                    "1",
                    "TLSハンドシェイク処理では、クライアント／サーバ間でのラウンドトリップが発生するため、一定の時間が必要となります",
                    found(2)
                )]
            );
        }

        /// A quote the book does not hold is the reader's only hint that the
        /// model reworded it, so the citation carries the reason.
        #[test]
        fn says_a_quoted_passage_is_not_in_the_document() {
            let full_text = full_text_of(&["まえがき", "第1章 Cloudflare Workers とは"]);
            let response = "本文[1]\n\n## Sources\n[1] 「この文は本文に存在しません」";

            assert_eq!(
                parse_citations(response, &book(&full_text, 2)),
                vec![pdf(
                    "1",
                    "この文は本文に存在しません",
                    Some(PageLocation::Missed(PageMiss::NotInBook))
                )]
            );
        }

        #[test]
        fn takes_the_last_quoted_block_when_an_entry_names_its_section_and_quotes_twice() {
            let full_text = full_text_of(&[
                "まえがき",
                "public 、 private は キャッシュ を 共有キャッシュ として 扱って よいか の 指定 に 使います",
                "private で あって ほしい もの には private を 付ける ように して おきましょう",
            ]);
            let response = "本文[1]\n\n## Sources\n[1] 「public、private」の節：「public、privateはキャッシュを共有キャッシュとして扱ってよいかの指定に使います」「privateであってほしいものにはprivateを付けるようにしておきましょう」";

            assert_eq!(
                parse_citations(response, &book(&full_text, 3)),
                vec![pdf(
                    "1",
                    "privateであってほしいものにはprivateを付けるようにしておきましょう",
                    found(3)
                )]
            );
        }

        /// The section is named before the passage is quoted, so the order tells
        /// them apart where the length does not.
        #[test]
        fn takes_the_quoted_passage_even_when_the_section_is_the_longer_block() {
            let full_text = full_text_of(&[
                "まえがき",
                "キャッシュ制御ヘッダ の 設計 と 運用 における 注意点",
                "private は 必ず 指定 します",
            ]);
            let response = "本文[1]\n\n## Sources\n[1] 「キャッシュ制御ヘッダの設計と運用における注意点」の節：「privateは必ず指定します」";

            assert_eq!(
                parse_citations(response, &book(&full_text, 3)),
                vec![pdf("1", "privateは必ず指定します", found(3))]
            );
        }

        /// Closing on whichever mark comes first would cut an English passage at
        /// its apostrophe, so each opening mark is paired with its own kind.
        #[test]
        fn keeps_an_apostrophe_inside_a_double_quoted_passage() {
            let full_text = full_text_of(&["preface", "The runtime doesn't ship a native canvas"]);
            let response =
                "本文[1]\n\n## Sources\n[1] \"The runtime doesn't ship a native canvas\"";

            assert_eq!(
                parse_citations(response, &book(&full_text, 2)),
                vec![pdf(
                    "1",
                    "The runtime doesn't ship a native canvas",
                    found(2)
                )]
            );
        }

        #[test]
        fn keeps_a_web_citation_as_a_url_reference_without_a_page() {
            let full_text = full_text_of(&["まえがき", "第1章"]);
            let response = "本文[1]\n\n## Sources\n[1] Cloudflare Docs - https://developers.cloudflare.com/workers/";

            assert_eq!(
                parse_citations(response, &book(&full_text, 2)),
                vec![web(
                    "1",
                    "Cloudflare Docs",
                    "https://developers.cloudflare.com/workers/"
                )]
            );
        }

        #[test]
        fn keeps_a_web_citation_whose_url_is_parenthesised_after_an_em_dashed_title() {
            let full_text = full_text_of(&["まえがき", "第1章"]);
            let response = "本文[1]\n\n## Sources\n[1] \"BFF looks up session in KV, retrieves access token\" — GitHub - neilpmas/bezzie: BFF OAuth 2.0 auth library for Cloudflare Workers (https://github.com/neilpmas/bezzie)";

            assert_eq!(
                parse_citations(response, &book(&full_text, 2)),
                vec![web(
                    "1",
                    "BFF looks up session in KV, retrieves access token",
                    "https://github.com/neilpmas/bezzie"
                )]
            );
        }

        #[test]
        fn keeps_a_web_citation_whose_title_holds_a_hyphen_of_its_own() {
            let full_text = full_text_of(&["まえがき", "第1章"]);
            let response = "本文[1]\n\n## Sources\n[1] \"A Worker-based BFF works best when the gateway owns client-facing routes\" - OneUptime Blog「Backend for Frontend Pattern」 https://raw.githubusercontent.com/OneUptime/blog/refs/heads/master/README.md";

            assert_eq!(
                parse_citations(response, &book(&full_text, 2)),
                vec![web(
                    "1",
                    "A Worker-based BFF works best when the gateway owns client-facing routes",
                    "https://raw.githubusercontent.com/OneUptime/blog/refs/heads/master/README.md"
                )]
            );
        }

        #[test]
        fn keeps_a_web_citation_separated_from_its_url_by_an_em_dash() {
            let full_text = full_text_of(&["まえがき", "第1章"]);
            let response = "本文[1]\n\n## Sources\n[1] \"Forwards these authenticated requests to the Hono API via service binding\" — Cloudflare Vite Plugin for React Router v7 · Issue #8958 — https://github.com/cloudflare/workers-sdk/issues/8958";

            assert_eq!(
                parse_citations(response, &book(&full_text, 2)),
                vec![web(
                    "1",
                    "Forwards these authenticated requests to the Hono API via service binding",
                    "https://github.com/cloudflare/workers-sdk/issues/8958"
                )]
            );
        }

        /// An entry that quotes nothing is its title, and the brackets that held
        /// the link are not part of it.
        #[test]
        fn keeps_a_web_citation_that_names_its_page_without_quoting_from_it() {
            let full_text = full_text_of(&["まえがき", "第1章"]);
            let response = "本文[1]\n\n## Sources\n[1] GitHub - neilpmas/bezzie: BFF OAuth 2.0 auth library for Cloudflare Workers (https://github.com/neilpmas/bezzie)";

            assert_eq!(
                parse_citations(response, &book(&full_text, 2)),
                vec![web(
                    "1",
                    "GitHub - neilpmas/bezzie: BFF OAuth 2.0 auth library for Cloudflare Workers",
                    "https://github.com/neilpmas/bezzie"
                )]
            );
        }

        /// Only the brackets that held the link go with it. A title closing on
        /// one of its own reads as a sentence cut short when it loses it.
        #[test]
        fn keeps_the_closing_bracket_a_web_citations_own_title_ends_on() {
            let full_text = full_text_of(&["まえがき", "第1章"]);
            let response = "本文[1]\n\n## Sources\n[1] 計算コストの比較（ハッシュ計算は高コスト、ファイル属性の読み取りは低コスト） - https://raw.githubusercontent.com/Alessandro-Pang/fe-interview/refs/heads/main/content/docs/network/network-14.md";

            assert_eq!(
                parse_citations(response, &book(&full_text, 2)),
                vec![web(
                    "1",
                    "計算コストの比較（ハッシュ計算は高コスト、ファイル属性の読み取りは低コスト）",
                    "https://raw.githubusercontent.com/Alessandro-Pang/fe-interview/refs/heads/main/content/docs/network/network-14.md"
                )]
            );
        }

        /// A book about the web prints urls in its own body. What tells a web
        /// source apart is that its url stands outside the quotation marks.
        #[test]
        fn keeps_a_book_citation_whose_quoted_passage_prints_a_url_of_its_own() {
            let full_text = full_text_of(&[
                "まえがき",
                "詳細は https://developers.cloudflare.com/workers/ を参照してください",
            ]);
            let response = "本文[1]\n\n## Sources\n[1] 「詳細は https://developers.cloudflare.com/workers/ を参照してください」（本書 4.2）";

            assert_eq!(
                parse_citations(response, &book(&full_text, 2)),
                vec![pdf(
                    "1",
                    "詳細は https://developers.cloudflare.com/workers/ を参照してください",
                    found(2)
                )]
            );
        }

        #[test]
        fn estimates_the_page_by_position_when_the_text_has_no_page_delimiters() {
            let full_text = format!(
                "{}\n{}目的の文{}",
                "a".repeat(100),
                "b".repeat(100),
                "c".repeat(100)
            );
            let response = "本文[1]\n\n## Sources\n[1] 「目的の文」";

            assert_eq!(
                parse_citations(response, &book(&full_text, 3)),
                vec![pdf("1", "目的の文", found(2))]
            );
        }

        #[test]
        fn gives_a_citation_neither_a_page_nor_a_reason_without_the_books_text() {
            let response = "本文[1]\n\n## Sources\n[1] 「引用」";
            assert_eq!(parse_citations(response, &[]), vec![pdf("1", "引用", None)]);
        }

        #[test]
        fn returns_no_citations_when_the_answer_has_no_sources_section() {
            let full_text = full_text_of(&["本文"]);
            assert!(parse_citations("出典のない回答です", &book(&full_text, 1)).is_empty());
        }
    }

    mod strip_sources {
        use super::*;

        #[test]
        fn drops_the_sources_section_and_the_blank_line_before_it() {
            let answer = "エッジで動きます[1]。\n\n## Sources\n[1] 「エッジ」";
            assert_eq!(strip_sources(answer), "エッジで動きます[1]。");
        }

        #[test]
        fn leaves_an_answer_without_sources_alone() {
            assert_eq!(strip_sources("出典のない回答です"), "出典のない回答です");
        }

        #[test]
        fn leaves_a_heading_that_is_not_a_sources_heading_alone() {
            let answer = "## Summary\n中身";
            assert_eq!(strip_sources(answer), answer);
        }
    }
}
