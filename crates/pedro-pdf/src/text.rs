//! Page text, and the box around each character it is made of.
//!
//! A selection is the reader dragging across a page, which is a rectangle in
//! screen space, and a question needs the *text* under it. That translation is
//! what these boxes are for; the same boxes drawn back gives the highlight.
//!
//! Coordinates are normalised to the page box — `0.0` to `1.0`, origin at the
//! top left — rather than left in PDF points, because the caller has the page
//! at some zoom level and a fraction of the page is the only thing that stays
//! true across all of them.

use pdfium_render::prelude::PdfRect;

/// A rectangle on a page, as a fraction of the page box, origin top left.
///
/// Stored as it is measured. chatbook keeps highlight geometry in the pixels of
/// whatever width the page happened to be rendered at, and carries that width
/// along so the rectangles can be rescaled later; a fraction of the page needs
/// no such companion and cannot disagree with one.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Rect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Rect {
    pub fn width(&self) -> f32 {
        self.right - self.left
    }

    pub fn height(&self) -> f32 {
        self.bottom - self.top
    }

    /// Whether `(x, y)`, in the same normalised space, is inside this rectangle.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.left && x <= self.right && y >= self.top && y <= self.bottom
    }

    /// The smallest rectangle covering both. Used to merge the characters of
    /// one line into a single highlight rectangle.
    pub fn union(&self, other: &Rect) -> Rect {
        Rect {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }

    /// Converts a pdfium rectangle, whose origin is the bottom left of `page`.
    pub(crate) fn from_pdfium(rect: PdfRect, page: PdfRect) -> Option<Rect> {
        let width = page.width().value;
        let height = page.height().value;
        if width <= 0.0 || height <= 0.0 {
            return None;
        }

        Some(Rect {
            left: (rect.left().value - page.left().value) / width,
            right: (rect.right().value - page.left().value) / width,
            // PDF y grows upwards, so the page's top edge is the zero of ours.
            top: (page.top().value - rect.top().value) / height,
            bottom: (page.top().value - rect.bottom().value) / height,
        })
    }
}

/// One character of a page, and where it sits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CharBox {
    /// Position in [`PageText::text`], counted in `char`s rather than bytes.
    pub index: usize,
    pub rect: Rect,
}

/// The text of one page.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PageText {
    /// Built from the characters pdfium reports, in their order, so that a
    /// [`CharBox::index`] always addresses this string. Taking pdfium's own
    /// whole-page string instead would be a second extraction that is not
    /// guaranteed to line up with the first.
    pub text: String,
    /// Boxes for the characters that have one. Whitespace usually does not, so
    /// this is shorter than `text` and is why the index is carried explicitly.
    pub chars: Vec<CharBox>,
}

impl PageText {
    /// The text between two character indices, inclusive of both.
    pub fn slice(&self, from: usize, to: usize) -> String {
        let (from, to) = if from <= to { (from, to) } else { (to, from) };
        self.text
            .chars()
            .skip(from)
            .take(to.saturating_sub(from) + 1)
            .collect()
    }

    /// The character whose box holds `(x, y)`, in normalised page coordinates.
    pub fn char_at(&self, x: f32, y: f32) -> Option<usize> {
        self.chars
            .iter()
            .find(|char_box| char_box.rect.contains(x, y))
            .map(|char_box| char_box.index)
    }

    /// The boxes of the characters from `from` to `to`, merged per line, which
    /// is what a highlight is drawn from.
    ///
    /// Characters are grouped by their vertical span rather than by a line
    /// number, which the PDF does not have: two boxes belong to the same line
    /// when their vertical centres are within half a character height.
    pub fn line_rects(&self, from: usize, to: usize) -> Vec<Rect> {
        let (from, to) = if from <= to { (from, to) } else { (to, from) };

        let mut lines: Vec<Rect> = Vec::new();
        for char_box in self
            .chars
            .iter()
            .filter(|char_box| char_box.index >= from && char_box.index <= to)
        {
            let rect = char_box.rect;
            let centre = (rect.top + rect.bottom) / 2.0;

            match lines.last_mut() {
                Some(line) if on_the_same_line(line, centre) => *line = line.union(&rect),
                _ => lines.push(rect),
            }
        }

        lines
    }
}

fn on_the_same_line(line: &Rect, centre: f32) -> bool {
    let tolerance = line.height().max(f32::EPSILON) / 2.0;
    ((line.top + line.bottom) / 2.0 - centre).abs() <= tolerance
}

#[cfg(test)]
mod tests {
    use pdfium_render::prelude::PdfPoints;

    use super::*;

    fn points(bottom: f32, left: f32, top: f32, right: f32) -> PdfRect {
        PdfRect::new(
            PdfPoints::new(bottom),
            PdfPoints::new(left),
            PdfPoints::new(top),
            PdfPoints::new(right),
        )
    }

    fn char_box(index: usize, left: f32, top: f32) -> CharBox {
        CharBox {
            index,
            rect: Rect {
                left,
                top,
                right: left + 0.01,
                bottom: top + 0.02,
            },
        }
    }

    fn page(text: &str, chars: Vec<CharBox>) -> PageText {
        PageText {
            text: text.to_owned(),
            chars,
        }
    }

    #[test]
    fn a_slice_is_inclusive_of_both_ends() {
        let page = page("abcdef", vec![]);
        assert_eq!(page.slice(1, 3), "bcd");
    }

    #[test]
    fn a_slice_reads_the_same_dragged_backwards() {
        let page = page("abcdef", vec![]);
        assert_eq!(page.slice(3, 1), page.slice(1, 3));
    }

    #[test]
    fn a_slice_counts_characters_not_bytes() {
        let page = page("あいうえお", vec![]);
        assert_eq!(page.slice(1, 2), "いう");
    }

    #[test]
    fn a_point_outside_every_box_selects_nothing() {
        let page = page("ab", vec![char_box(0, 0.1, 0.1)]);
        assert_eq!(page.char_at(0.9, 0.9), None);
        assert_eq!(page.char_at(0.105, 0.105), Some(0));
    }

    #[test]
    fn characters_on_one_line_merge_into_one_rectangle() {
        let page = page(
            "abc",
            vec![
                char_box(0, 0.1, 0.1),
                char_box(1, 0.11, 0.1),
                char_box(2, 0.12, 0.1),
            ],
        );

        let rects = page.line_rects(0, 2);
        assert_eq!(rects.len(), 1);
        assert!((rects[0].left - 0.1).abs() < 1e-6);
        assert!((rects[0].right - 0.13).abs() < 1e-6);
    }

    #[test]
    fn a_selection_across_two_lines_yields_two_rectangles() {
        let page = page(
            "abcd",
            vec![
                char_box(0, 0.1, 0.1),
                char_box(1, 0.11, 0.1),
                char_box(2, 0.1, 0.2),
                char_box(3, 0.11, 0.2),
            ],
        );

        assert_eq!(page.line_rects(0, 3).len(), 2);
    }

    #[test]
    fn only_the_selected_characters_are_drawn() {
        let page = page(
            "abcd",
            vec![
                char_box(0, 0.1, 0.1),
                char_box(1, 0.11, 0.1),
                char_box(2, 0.1, 0.2),
                char_box(3, 0.11, 0.2),
            ],
        );

        let rects = page.line_rects(2, 3);
        assert_eq!(rects.len(), 1);
        assert!((rects[0].top - 0.2).abs() < 1e-6);
    }

    #[test]
    fn a_pdfium_rectangle_is_measured_from_the_top_left() {
        // PdfRect is (bottom, left, top, right) in points.
        let page = points(0.0, 0.0, 800.0, 600.0);
        // A box in the top left corner of the page: high y, low x.
        let rect = points(780.0, 0.0, 800.0, 60.0);

        let converted = Rect::from_pdfium(rect, page).expect("a page with area");
        assert!((converted.left - 0.0).abs() < 1e-6);
        assert!((converted.top - 0.0).abs() < 1e-6);
        assert!((converted.right - 0.1).abs() < 1e-6);
        assert!((converted.bottom - 0.025).abs() < 1e-6);
    }

    #[test]
    fn a_page_without_area_has_no_coordinates_to_offer() {
        let empty = points(0.0, 0.0, 0.0, 0.0);
        assert_eq!(Rect::from_pdfium(empty, empty), None);
    }
}
