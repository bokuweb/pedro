//! Putting two rankings together.
//!
//! Keyword search and similarity search disagree, and each is right about
//! something the other cannot see: one finds the passage that says the word,
//! the other the passage that means it. Their scores are not comparable — bm25
//! counts one way and cosine another — so what is compared is the *positions*
//! they put a passage in.
//!
//! Reciprocal rank fusion: a passage scores `1 / (k + rank)` in each ranking it
//! appears in, and the scores are added. A passage both rank highly beats one
//! either ranks first alone, which is the whole point of asking twice.

use crate::index::Hit;

/// How much a low rank still counts. The usual constant; large enough that the
/// difference between first and second is not the whole answer.
const K: f32 = 60.0;

/// Merges rankings, best first.
pub fn reciprocal_rank(rankings: &[Vec<Hit>], limit: usize) -> Vec<Hit> {
    let mut fused: Vec<(Hit, f32)> = Vec::new();

    for ranking in rankings {
        for (rank, hit) in ranking.iter().enumerate() {
            let score = 1.0 / (K + rank as f32 + 1.0);

            match fused.iter_mut().find(|(held, _)| same_passage(held, hit)) {
                Some((_, total)) => *total += score,
                None => fused.push((hit.clone(), score)),
            }
        }
    }

    fused.sort_by(|left, right| right.1.total_cmp(&left.1));
    fused
        .into_iter()
        .take(limit)
        .map(|(hit, score)| Hit { score, ..hit })
        .collect()
}

/// Whether two hits are the same passage.
///
/// By where it is rather than by an identifier: the two searches read the same
/// rows, and a page of a book is what the reader is being sent to either way.
fn same_passage(left: &Hit, right: &Hit) -> bool {
    left.book_id == right.book_id && left.page_number == right.page_number && left.text == right.text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(page_number: u32, text: &str, score: f32) -> Hit {
        Hit {
            book_id: "book".to_owned(),
            page_number,
            text: text.to_owned(),
            score,
        }
    }

    /// The property worth having: a passage both searches found beats one that
    /// tops a single ranking. Nothing stronger is true — being first in one
    /// ranking and third in the other edges out being second in both, because
    /// the constant makes the gap between ranks small on purpose.
    #[test]
    fn a_passage_both_rankings_found_beats_one_either_topped() {
        let words = vec![hit(1, "only words", 9.0), hit(2, "both", 8.0)];
        let meaning = vec![hit(3, "only meaning", 0.9), hit(2, "both", 0.8)];

        let fused = reciprocal_rank(&[words, meaning], 10);

        assert_eq!(fused[0].text, "both");
    }

    #[test]
    fn a_passage_only_one_ranking_found_still_appears() {
        let words = vec![hit(1, "a", 9.0)];
        let meaning = vec![hit(9, "z", 0.9)];

        let fused = reciprocal_rank(&[words, meaning], 10);
        let texts: Vec<&str> = fused.iter().map(|hit| hit.text.as_str()).collect();

        assert!(texts.contains(&"a") && texts.contains(&"z"));
    }

    #[test]
    fn the_same_passage_is_not_returned_twice() {
        let ranking = vec![hit(1, "a", 1.0)];

        let fused = reciprocal_rank(&[ranking.clone(), ranking], 10);
        assert_eq!(fused.len(), 1);
    }

    #[test]
    fn nothing_fuses_to_nothing() {
        assert!(reciprocal_rank(&[], 10).is_empty());
        assert!(reciprocal_rank(&[vec![], vec![]], 10).is_empty());
    }

    #[test]
    fn the_limit_is_honoured() {
        let many: Vec<Hit> = (0..50).map(|n| hit(n, &n.to_string(), 1.0)).collect();

        assert_eq!(reciprocal_rank(&[many], 5).len(), 5);
    }

    /// One ranking on its own comes back in the order it arrived.
    #[test]
    fn a_single_ranking_keeps_its_order() {
        let ranking = vec![hit(1, "a", 9.0), hit(2, "b", 8.0), hit(3, "c", 7.0)];

        let fused = reciprocal_rank(&[ranking], 10);
        let texts: Vec<&str> = fused.iter().map(|hit| hit.text.as_str()).collect();

        assert_eq!(texts, vec!["a", "b", "c"]);
    }
}
