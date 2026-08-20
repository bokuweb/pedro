//! An open book, and the page of it that is on screen.
//!
//! Pages are rasterised by pdfium on a background thread and handed to GPUI as
//! images. The bytes cross that boundary untouched: pdfium renders BGRA, and a
//! [`RenderImage`] is BGRA, so the one format conversion this would obviously
//! need does not exist.

use std::sync::Arc;

use gpui::{RenderImage, SharedString};
use image::{Frame, RgbaImage};
use pedro_pdf::{Document, PageImage, PageSize, PixelFormat};

/// How much detail to render per logical pixel.
///
/// Fixed at 2 rather than read from the display: a page rendered for a Retina
/// screen is merely oversampled on a screen that is not one, while the reverse
/// is a blurry page, and a book is a thing people look at closely.
const OVERSAMPLE: f32 = 2.0;

/// A book open in a tab.
pub struct OpenDocument {
    /// Shared with the background thread that rasterises pages.
    pub document: Arc<Document>,
    pub page_count: u32,
    /// The size of the first page, in points, which is what every page is laid
    /// out against. A book whose pages differ in size is rare enough that
    /// measuring each one would cost more than it is worth.
    pub size: PageSize,
    /// One-based, the way the reader counts.
    pub page: u32,
    pub rendered: Option<Rendered>,
}

/// A page that has been rasterised and is ready to draw.
pub struct Rendered {
    pub page: u32,
    pub image: Arc<RenderImage>,
}

impl OpenDocument {
    pub fn new(document: Document, size: PageSize, page: u32) -> Self {
        let page_count = document.page_count();

        Self {
            document: Arc::new(document),
            page_count,
            size,
            page: page.clamp(1, page_count.max(1)),
            rendered: None,
        }
    }

    /// The image to draw, or `None` while the page is still being rasterised.
    ///
    /// A stale page is not drawn: showing the page you just left while the new
    /// one renders looks like the page turn failed.
    pub fn visible(&self) -> Option<&Arc<RenderImage>> {
        self.rendered
            .as_ref()
            .filter(|rendered| rendered.page == self.page)
            .map(|rendered| &rendered.image)
    }

    /// How wide the page is when drawn `height` logical pixels tall.
    pub fn width_at(&self, height: f32) -> f32 {
        width_at(self.size, height)
    }

    /// The scale to rasterise at so a page drawn `height` pixels tall has a
    /// pixel of its own for every pixel of screen.
    pub fn scale_for(&self, height: f32) -> f32 {
        scale_for(self.size, height)
    }

    /// Moves `by` pages, stopping at either cover. Returns whether it moved.
    pub fn turn(&mut self, by: i64) -> bool {
        let page = turned(self.page, self.page_count, by);
        let moved = page != self.page;
        self.page = page;

        moved
    }

    /// What the tab bar and the composer's context line say about where we are.
    pub fn position(&self) -> SharedString {
        format!("p. {} of {}", self.page, self.page_count).into()
    }
}

/// How wide a page of `size` is when drawn `height` logical pixels tall.
fn width_at(size: PageSize, height: f32) -> f32 {
    match size.height {
        0.0 => height,
        page_height => height * size.width / page_height,
    }
}

/// The scale a page of `size` has to be rasterised at to fill `height` logical
/// pixels with real detail.
fn scale_for(size: PageSize, height: f32) -> f32 {
    match size.height {
        0.0 => OVERSAMPLE,
        page_height => (height * OVERSAMPLE / page_height).max(0.01),
    }
}

/// The page `by` pages from `page`, stopping at either cover.
fn turned(page: u32, page_count: u32, by: i64) -> u32 {
    (page as i64 + by).clamp(1, page_count.max(1) as i64) as u32
}

/// Wraps a rasterised page for GPUI.
///
/// The buffer is an `RgbaImage` holding BGRA, which is not a mistake: it is the
/// container `RenderImage` is built from, and what it wants inside it is BGRA.
/// Taking the bytes by value means the pixels are never copied.
pub fn as_render_image(page: PageImage) -> Option<Arc<RenderImage>> {
    debug_assert_eq!(
        page.format,
        PixelFormat::Bgra8,
        "gpui draws BGRA; anything else would need converting first"
    );

    let buffer = RgbaImage::from_raw(page.width, page.height, page.bytes)?;

    Some(Arc::new(RenderImage::new(vec![Frame::new(buffer)])))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A4-ish, in points.
    fn a4() -> PageSize {
        PageSize {
            width: 595.0,
            height: 842.0,
        }
    }

    #[test]
    fn a_page_keeps_its_aspect_ratio() {
        let width = width_at(a4(), 842.0);
        assert!((width - 595.0).abs() < 1e-3, "{width}");
    }

    #[test]
    fn a_page_is_rasterised_at_two_pixels_per_pixel() {
        assert!((scale_for(a4(), 842.0) - 2.0).abs() < 1e-6);
        assert!((scale_for(a4(), 421.0) - 1.0).abs() < 1e-6);
    }

    /// A page with no height cannot be scaled to one, and must not divide by it
    /// either: a malformed PDF should still open.
    #[test]
    fn a_page_without_height_is_not_divided_by_zero() {
        let flat = PageSize {
            width: 100.0,
            height: 0.0,
        };

        assert_eq!(width_at(flat, 640.0), 640.0);
        assert_eq!(scale_for(flat, 640.0), OVERSAMPLE);
    }

    #[test]
    fn turning_stops_at_both_covers() {
        assert_eq!(turned(1, 10, -1), 1);
        assert_eq!(turned(10, 10, 1), 10);
        assert_eq!(turned(5, 10, 3), 8);
        assert_eq!(turned(5, 10, -3), 2);
    }

    /// A book that reports no pages still has a page one to sit on.
    #[test]
    fn an_empty_book_stays_on_page_one() {
        assert_eq!(turned(1, 0, 1), 1);
    }

    #[test]
    fn a_render_image_needs_four_bytes_per_pixel() {
        let short = PageImage {
            width: 2,
            height: 2,
            format: PixelFormat::Bgra8,
            bytes: vec![0; 8],
        };

        assert!(as_render_image(short).is_none());
    }

    #[test]
    fn a_page_becomes_an_image_of_the_same_size() {
        let page = PageImage {
            width: 3,
            height: 2,
            format: PixelFormat::Bgra8,
            bytes: vec![0; 3 * 2 * 4],
        };

        let image = as_render_image(page).expect("a full buffer");
        let size = image.size(0);
        assert_eq!((u32::from(size.width), u32::from(size.height)), (3, 2));
    }
}
