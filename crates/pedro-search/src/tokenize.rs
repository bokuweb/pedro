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
