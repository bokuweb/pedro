//! Cutting Japanese into something FTS5 can index.
//!
//! SQLite's `unicode61` tokenizer splits on spaces and punctuation, which a
//! Japanese sentence does not have — a whole paragraph arrives as one token and
//! matches nothing. The way round it is to segment the text ourselves and hand
//! FTS5 the pieces separated by spaces, using the *same* segmentation when
//! indexing and when searching.
//!
//! The segmentation is overlapping character bigrams, which needs no
//! dictionary: 東京駅 becomes 東京 and 京駅, and a search for 京駅 finds it.
//! Runs of latin letters and digits are words already and are kept whole,
//! lowercased.
//!
//! Adapted from `ellisii-jp-tokenizer-bigram` and `-core` in the author's
//! ellisii-toolkit, with permission.

/// Hiragana, katakana, kanji: the scripts written without spaces.
pub fn is_cjk(c: char) -> bool {
    matches!(
        c,
        '\u{3040}'..='\u{309f}'      // hiragana
            | '\u{30a0}'..='\u{30ff}' // katakana
            | '\u{31f0}'..='\u{31ff}' // katakana extensions
            | '\u{3400}'..='\u{4dbf}' // CJK extension A
            | '\u{4e00}'..='\u{9fff}' // CJK unified ideographs
            | '\u{f900}'..='\u{faff}' // CJK compatibility ideographs
            | '\u{ff66}'..='\u{ff9d}' // half-width katakana
    )
}

/// Whether a character is part of a word rather than a separator.
pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || is_cjk(c)
}

/// Splits `text` into the tokens the index is built from.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut run = String::new();
    let mut run_is_cjk = false;

    for character in text.chars() {
        if !is_word_char(character) {
            flush(&mut run, run_is_cjk, &mut tokens);
            continue;
        }

        // A script boundary ends a run: 「Rustで」 is a latin word and a
        // Japanese one, not one token spanning both.
        let cjk = is_cjk(character);
        if !run.is_empty() && cjk != run_is_cjk {
            flush(&mut run, run_is_cjk, &mut tokens);
        }

        run.push(character);
        run_is_cjk = cjk;
    }

    flush(&mut run, run_is_cjk, &mut tokens);
    tokens
}

/// The tokens as one string, which is what goes into the index and into a
/// query. Indexing and searching must use this same function or they are
/// speaking different languages.
pub fn for_index(text: &str) -> String {
    tokenize(text).join(" ")
}

/// Tokens as an FTS5 query: every one quoted, any of them may match.
///
/// Quoted because a token can contain characters FTS5 reads as syntax, and
/// joined with OR because a bigram-segmented phrase rarely matches in full —
/// ranking is what decides which hits are good, not the query.
pub fn as_query(tokens: &[&str]) -> String {
    tokens
        .iter()
        .map(|token| format!("\"{}\"", token.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" OR ")
}

/// The tokens of `text` as an FTS5 query.
pub fn for_query(text: &str) -> String {
    let tokens = tokenize(text);
    let borrowed: Vec<&str> = tokens.iter().map(String::as_str).collect();

    as_query(&borrowed)
}

fn flush(run: &mut String, is_cjk: bool, tokens: &mut Vec<String>) {
    if run.is_empty() {
        return;
    }

    if is_cjk {
        let characters: Vec<char> = run.chars().collect();
        match characters.len() {
            // A lone character has no pair to make; it is its own token, so
            // that a one-character word is still findable.
            1 => tokens.push(characters[0].to_string()),
            _ => tokens.extend(
                characters
                    .windows(2)
                    .map(|pair| pair.iter().collect::<String>()),
            ),
        }
    } else {
        tokens.push(run.to_lowercase());
    }

    run.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin_words_are_kept_whole_and_lowercased() {
        assert_eq!(tokenize("Hello World"), vec!["hello", "world"]);
    }

    #[test]
    fn japanese_becomes_overlapping_pairs() {
        assert_eq!(tokenize("東京駅"), vec!["東京", "京駅"]);
    }

    /// The pairs overlap so that a search for the middle of a word finds it.
    #[test]
    fn a_search_term_inside_a_word_shares_a_token_with_it() {
        let indexed = tokenize("東京駅前");
        let searched = tokenize("京駅");

        assert!(searched.iter().all(|token| indexed.contains(token)));
    }

    #[test]
    fn a_script_boundary_ends_a_token() {
        assert_eq!(tokenize("Rustで書く"), vec!["rust", "で書", "書く"]);
    }

    #[test]
    fn a_lone_character_is_its_own_token() {
        assert_eq!(tokenize("を"), vec!["を"]);
    }

    #[test]
    fn punctuation_and_spaces_separate() {
        assert_eq!(tokenize("あ、い"), vec!["あ", "い"]);
        assert!(tokenize("   ").is_empty());
    }

    #[test]
    fn a_query_quotes_every_token_and_asks_for_any() {
        assert_eq!(for_query("東京駅"), r#""東京" OR "京駅""#);
    }

    /// A quotation mark in the text would otherwise close the quoted token and
    /// leave the rest as FTS5 syntax.
    #[test]
    fn a_query_cannot_be_broken_by_a_quotation_mark() {
        assert_eq!(for_query("say \"hi\""), r#""say" OR "hi""#);
    }

    #[test]
    fn an_empty_query_is_empty() {
        assert_eq!(for_query("  "), "");
        assert_eq!(for_index(""), "");
    }
}

/// Whether a character is one Japanese writes content in.
///
/// Japanese writes its content in kanji and katakana and its grammar in
/// hiragana, which is what makes a script boundary a usable word boundary
/// without a dictionary.
fn script_of(c: char) -> Script {
    match c {
        '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}' | '\u{F900}'..='\u{FAFF}' => {
            Script::Kanji
        }
        '\u{30A0}'..='\u{30FF}' | '\u{FF66}'..='\u{FF9F}' => Script::Katakana,
        c if c.is_alphanumeric() && c.is_ascii() => Script::Latin,
        _ => Script::Other,
    }
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Script {
    Kanji,
    Katakana,
    Latin,
    Other,
}

/// The content words of `text`: runs of kanji, runs of katakana, latin words.
///
/// The other tokenizer cuts everything into character pairs, which finds a word
/// inside a longer one but manufactures tokens that are rare and meaningless at
/// once — 「で動」 spans a particle and a verb, occurs in six passages of a
/// library of 1,836, and means nothing. No weighting can tell that from 「素数」,
/// which occurs in seventy-nine, because to a weighting they are the same shape.
/// Splitting on the script boundary can: it drops the grammar and keeps the
/// subject.
///
/// What this loses is a content word written in hiragana — ふるい, できる. The
/// pairs are still indexed beside these, and searching for a string still uses
/// them; this is for finding what a question is *about*.
pub fn content(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut run = String::new();
    let mut script = Script::Other;

    for character in text.chars() {
        let this = script_of(character);
        if this != script && !run.is_empty() {
            words.push(std::mem::take(&mut run).to_lowercase());
        }

        script = this;
        if this != Script::Other {
            run.push(character);
        }
    }

    if !run.is_empty() {
        words.push(run.to_lowercase());
    }

    words
}

/// The content words of `text` as one string, for the index.
pub fn content_for_index(text: &str) -> String {
    content(text).join(" ")
}

/// The content words of a question, as an FTS5 query.
///
/// A verb's stem is one kanji — 決める, 動く, 話す — and one kanji is a word
/// only by accident: asking 「鍵長はどうやって決める?」 with 決 in the query
/// matched every 決して in the library and ranked them above the pages about key
/// length. Single characters are dropped where the question has something longer
/// to go on, and kept where they are all it has.
pub fn content_query(text: &str) -> String {
    let mut words = content(text);
    if words.iter().any(|word| word.chars().count() > 1) {
        words.retain(|word| word.chars().count() > 1);
    }

    let borrowed: Vec<&str> = words.iter().map(String::as_str).collect();
    as_query(&borrowed)
}

#[cfg(test)]
mod content_tests {
    use super::{content, content_query};

    #[test]
    fn grammar_is_dropped_and_the_subject_kept() {
        assert_eq!(content("素数はどうやって生成する?"), vec!["素数", "生成"]);
    }

    #[test]
    fn a_katakana_word_is_one_word() {
        assert_eq!(
            content("エラトステネスのふるいで素数を生成する"),
            vec!["エラトステネス", "素数", "生成"]
        );
    }

    #[test]
    fn latin_words_survive_the_japanese_around_them() {
        assert_eq!(
            content("runtime が edge で動くという話はどの本?"),
            vec!["runtime", "edge", "動", "話", "本"]
        );
    }

    #[test]
    fn a_query_drops_the_single_characters_it_can_afford_to() {
        assert_eq!(
            content_query("runtime が edge で動くという話はどの本?"),
            "\"runtime\" OR \"edge\""
        );
    }

    /// A question that is nothing but single characters still asks something.
    #[test]
    fn a_query_of_single_characters_keeps_them() {
        assert_eq!(content_query("本と鍵"), "\"本\" OR \"鍵\"");
    }

    /// Nothing to go on rather than a query that matches everything.
    #[test]
    fn a_question_written_only_in_hiragana_has_no_content_words() {
        assert!(content("これはどうですか").is_empty());
        assert!(content_query("これはどうですか").is_empty());
    }
}
