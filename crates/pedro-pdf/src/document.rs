//! An open PDF: its pages, their pixels, their text, and its outline.

use std::path::Path;

use pdfium_render::prelude::{PdfBitmapFormat, PdfDocument, PdfPage, PdfPageRenderRotation};

use crate::library::{in_use, library};
use crate::outline::OutlineItem;
use crate::text::{CharBox, PageText, Rect};
use crate::PdfError;

/// The character pages are joined with in the stored full text.
///
/// A form feed, the same delimiter chatbook's extractor uses, because the
/// citation lookup finds the page a quote came from by counting these seams.
pub const PAGE_DELIMITER: char = '\u{000C}';

/// A page's size in PDF points, which is what the reader scales from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSize {
    pub width: f32,
    pub height: f32,
}

/// How the bytes of a [`PageImage`] are ordered.
///
/// Reported rather than normalised because both orders have a consumer —
/// pdfium hands back BGRA and GPUI wants BGRA, so a crate that insisted on
/// RGBA would make every page pay for two conversions to arrive where it
/// started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Bgra8,
    Rgba8,
}

/// A rasterised page.
#[derive(Debug, Clone)]
pub struct PageImage {
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub bytes: Vec<u8>,
}

impl PageImage {
    /// Swaps the red and blue channels in place, if they are not already in
    /// the requested order.
    pub fn convert_to(&mut self, format: PixelFormat) {
        if self.format == format {
            return;
        }

        for pixel in self.bytes.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        self.format = format;
    }
}

/// An open document.
///
/// Borrows the process-wide pdfium binding for `'static`, so this is an
/// ordinary owned value that can be moved to a background thread.
///
/// pdfium cannot be *used* from two threads at once — doing so aborts the
/// process, `thread_safe` feature or not — so every method here takes a
/// process-wide lock for its duration. Callers do not have to arrange anything;
/// they only have to expect a call to wait while another page is rendering,
/// which is why none of this belongs on a UI thread.
pub struct Document {
    /// An `Option` only so that [`Drop`] can close the document while still
    /// holding the lock. Struct fields are dropped after the `drop` body
    /// returns, by which time a guard taken there is already gone — and closing
    /// a document is a call into pdfium like any other.
    inner: Option<PdfDocument<'static>>,
}

impl Drop for Document {
    fn drop(&mut self) {
        let _guard = in_use();
        drop(self.inner.take());
    }
}

impl Document {
    pub fn open(path: &Path) -> Result<Self, PdfError> {
        Self::from_bytes(std::fs::read(path)?)
    }

    /// Opens a document from bytes already in memory. pdfium keeps reading
    /// from the buffer as pages are visited, so it takes ownership of it.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, PdfError> {
        let _guard = in_use();
        Ok(Self {
            inner: Some(library()?.load_pdf_from_byte_vec(bytes, None)?),
        })
    }

    pub fn page_count(&self) -> u32 {
        let _guard = in_use();
        self.count()
    }

    pub fn page_size(&self, index: u32) -> Result<PageSize, PdfError> {
        let _guard = in_use();
        let page = self.page(index)?;
        Ok(PageSize {
            width: page.width().value,
            height: page.height().value,
        })
    }

    /// Rasterises one page at `scale` times its natural size.
    ///
    /// The caller decides the scale because the page's size in points and the
    /// window's zoom are both its business; this only refuses to produce a
    /// zero-pixel bitmap, which pdfium rejects.
    pub fn render_page(&self, index: u32, scale: f32) -> Result<PageImage, PdfError> {
        let _guard = in_use();
        let page = self.page(index)?;
        let width = ((page.width().value * scale).round() as i32).max(1);
        let height = ((page.height().value * scale).round() as i32).max(1);

        let bitmap = page.render(width, height, None::<PdfPageRenderRotation>)?;
        if bitmap.format()? != PdfBitmapFormat::BGRA {
            return Err(PdfError::UnsupportedBitmapFormat);
        }

        Ok(PageImage {
            width: bitmap.width() as u32,
            height: bitmap.height() as u32,
            format: PixelFormat::Bgra8,
            bytes: bitmap.as_raw_bytes(),
        })
    }

    /// The text of one page, with the box around every character that has one.
    pub fn page_text(&self, index: u32) -> Result<PageText, PdfError> {
        let _guard = in_use();
        self.text_of(index)
    }

    fn text_of(&self, index: u32) -> Result<PageText, PdfError> {
        let page = self.page(index)?;
        let bounds = page.page_size();
        let text = page.text()?;

        let mut string = String::new();
        let mut chars = Vec::new();

        for (position, character) in text.chars().iter().enumerate() {
            // A character pdfium cannot decode still occupies a position, and
            // dropping it here would shift every box after it.
            string.push(character.unicode_char().unwrap_or(char::REPLACEMENT_CHARACTER));

            // Loose bounds rather than tight ones: a highlight should cover the
            // line's height, not just the ink of the glyph, and a character
            // without ink (a space) has no tight box at all.
            if let Some(rect) = character
                .loose_bounds()
                .ok()
                .and_then(|rect| Rect::from_pdfium(rect, bounds))
            {
                chars.push(CharBox {
                    index: position,
                    rect,
                });
            }
        }

        Ok(PageText {
            text: string,
            chars,
        })
    }

    /// Every page's text, joined with [`PAGE_DELIMITER`].
    ///
    /// This is the string a question's context is cut out of and the string a
    /// citation is looked up in, so the seams are load-bearing: they are how a
    /// position in the text becomes a page number.
    pub fn full_text(&self) -> Result<String, PdfError> {
        // One lock for the whole book rather than one per page: a long book
        // would otherwise hand the library back and forth hundreds of times
        // while nothing else wants it.
        let _guard = in_use();

        let mut pages = Vec::with_capacity(self.count() as usize);
        for index in 0..self.count() {
            pages.push(self.text_of(index)?.text);
        }
        Ok(pages.join(&PAGE_DELIMITER.to_string()))
    }

    /// The top-level outline entries that name a page, in document order.
    ///
    /// Entries pointing nowhere are dropped rather than reported: an outline is
    /// an optimisation for choosing context, and a broken entry in it is not
    /// worth failing an import over.
    pub fn outline(&self) -> Vec<OutlineItem> {
        let _guard = in_use();
        let bookmarks = self.inner().bookmarks();
        let mut items = Vec::new();
        let mut current = bookmarks.root();

        while let Some(bookmark) = current {
            if let Some(title) = bookmark.title()
                && let Some(page_index) = bookmark
                    .destination()
                    .and_then(|destination| destination.page_index().ok())
            {
                let title = title.trim().to_owned();
                if !title.is_empty() {
                    items.push(OutlineItem {
                        title,
                        page_number: page_index as u32 + 1,
                    });
                }
            }

            current = bookmark.next_sibling();
        }

        items
    }

    /// The open document. Only `None` while it is being dropped.
    fn inner(&self) -> &PdfDocument<'static> {
        self.inner.as_ref().expect("the document is open")
    }

    /// The page count without taking the lock, for callers that already hold it.
    fn count(&self) -> u32 {
        self.inner().pages().len() as u32
    }

    fn page(&self, index: u32) -> Result<PdfPage<'_>, PdfError> {
        self.inner()
            .pages()
            .get(index as i32)
            .map_err(|_| PdfError::NoSuchPage {
                requested: index,
                page_count: self.count(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_conversion_swaps_red_and_blue() {
        let mut image = PageImage {
            width: 1,
            height: 1,
            format: PixelFormat::Bgra8,
            bytes: vec![1, 2, 3, 4],
        };

        image.convert_to(PixelFormat::Rgba8);
        assert_eq!(image.bytes, vec![3, 2, 1, 4]);
        assert_eq!(image.format, PixelFormat::Rgba8);
    }

    #[test]
    fn converting_to_the_format_it_already_has_does_nothing() {
        let mut image = PageImage {
            width: 1,
            height: 1,
            format: PixelFormat::Bgra8,
            bytes: vec![1, 2, 3, 4],
        };

        image.convert_to(PixelFormat::Bgra8);
        assert_eq!(image.bytes, vec![1, 2, 3, 4]);
    }
}
