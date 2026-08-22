//! Turning what the reader pasted into a Drive file id.
//!
//! There is no one canonical Drive link. The share sheet gives you
//! `/file/d/{id}/view`, an older link gives you `?id=`, and a Google Doc gives
//! you `/document/d/{id}/edit`. All three name the same kind of thing, so all
//! three are accepted — as is the bare id, because someone who has the id in
//! hand should not have to wrap a URL around it.

/// The id in a Drive link, or the link itself if it already is one.
///
/// Returns `None` when nothing in the input looks like an id, which is what
/// tells a typo apart from a link this does not know the shape of.
pub fn file_id(input: &str) -> Option<String> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }

    if !input.contains("://") {
        return is_id(input).then(|| input.to_owned());
    }

    // `/d/{id}/` is how every current link carries it — files, documents,
    // spreadsheets and presentations alike.
    if let Some(id) = after(input, "/d/").and_then(take_id) {
        return Some(id);
    }

    // The older `?id=` form, which `uc?export=download` links still use.
    for marker in ["?id=", "&id="] {
        if let Some(id) = after(input, marker).and_then(take_id) {
            return Some(id);
        }
    }

    None
}

/// What follows the first `marker` in `haystack`.
fn after<'a>(haystack: &'a str, marker: &str) -> Option<&'a str> {
    haystack.split_once(marker).map(|(_, rest)| rest)
}

/// The id at the front of `rest`, up to whatever ends it.
fn take_id(rest: &str) -> Option<String> {
    let id: String = rest.chars().take_while(|c| is_id_char(*c)).collect();
    is_id(&id).then_some(id)
}

/// Drive ids are opaque, so this only rules out what cannot be one.
///
/// Short strings are the case that matters: `/d/1/view` would otherwise turn a
/// malformed link into a request that comes back "not found" long after the
/// point where saying "that is not a Drive link" was still useful.
fn is_id(candidate: &str) -> bool {
    candidate.len() >= 16 && candidate.chars().all(is_id_char)
}

fn is_id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "1a2B3c4D5e6F7g8H9i0JkLmNoPqRsTuV";

    #[test]
    fn a_share_link_carries_its_id() {
        assert_eq!(
            file_id(&format!(
                "https://drive.google.com/file/d/{ID}/view?usp=sharing"
            ))
            .as_deref(),
            Some(ID)
        );
    }

    #[test]
    fn a_document_link_carries_its_id() {
        assert_eq!(
            file_id(&format!(
                "https://docs.google.com/document/d/{ID}/edit#heading=h.x"
            ))
            .as_deref(),
            Some(ID)
        );
    }

    #[test]
    fn the_older_query_form_carries_its_id() {
        assert_eq!(
            file_id(&format!("https://drive.google.com/open?id={ID}")).as_deref(),
            Some(ID)
        );
    }

    #[test]
    fn a_download_link_carries_its_id() {
        assert_eq!(
            file_id(&format!(
                "https://drive.google.com/uc?export=download&id={ID}"
            ))
            .as_deref(),
            Some(ID)
        );
    }

    #[test]
    fn a_bare_id_is_taken_as_one() {
        assert_eq!(file_id(ID).as_deref(), Some(ID));
    }

    #[test]
    fn surrounding_space_is_not_part_of_the_id() {
        assert_eq!(file_id(&format!("  {ID}\n")).as_deref(), Some(ID));
    }

    #[test]
    fn a_link_with_no_id_in_it_is_not_one() {
        assert_eq!(file_id("https://drive.google.com/drive/my-drive"), None);
    }

    #[test]
    fn something_too_short_to_be_an_id_is_not_one() {
        assert_eq!(file_id("https://drive.google.com/file/d/1/view"), None);
        assert_eq!(file_id("hello"), None);
    }

    #[test]
    fn nothing_is_not_an_id() {
        assert_eq!(file_id("   "), None);
    }
}
