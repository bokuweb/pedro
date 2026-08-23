//! A PDF written out by hand, for tests across this workspace.
//!
//! Not a checked-in binary fixture: what the tests assert is the relationship
//! between what goes onto a page and what comes back off it, and a binary
//! hides half of that.

/// Builds a valid single-font PDF with one page per string, each page carrying
/// that string as its only text.
pub fn pdf_with_pages(pages: &[&str]) -> Vec<u8> {
    let sized: Vec<Page<'_>> = pages.iter().map(|text| Page::new(text)).collect();

    build(&sized, None)
}

/// A page of the fixture: what is written on it, and how large it is.
#[derive(Clone, Copy)]
pub struct Page<'a> {
    pub text: &'a str,
    pub width: f32,
    pub height: f32,
}

impl<'a> Page<'a> {
    /// The size every page of the plain fixture has.
    pub fn new(text: &'a str) -> Self {
        Self {
            text,
            width: 300.,
            height: 200.,
        }
    }

    pub fn sized(text: &'a str, width: f32, height: f32) -> Self {
        Self {
            text,
            width,
            height,
        }
    }
}

/// A book whose pages are not all one shape — an upright book with a plan
/// turned sideways in it, which is the case a reader laid out against its first
/// page gets wrong.
pub fn pdf_with_sizes(pages: &[Page<'_>]) -> Vec<u8> {
    build(pages, None)
}

/// The same, with a crop box inset from the media box by `inset` points on
/// every side — the shape a printed book has, and the one that tells the two
/// coordinate spaces of a page apart.
pub fn pdf_with_crop_box(pages: &[&str], inset: f32) -> Vec<u8> {
    let sized: Vec<Page<'_>> = pages.iter().map(|text| Page::new(text)).collect();

    build(&sized, Some(inset))
}

fn build(pages: &[Page<'_>], crop_inset: Option<f32>) -> Vec<u8> {
    let font_id = 3 + 2 * pages.len();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            (0..pages.len())
                .map(|index| format!("{} 0 R", 3 + 2 * index))
                .collect::<Vec<_>>()
                .join(" "),
            pages.len()
        ),
    ];

    for (index, page) in pages.iter().enumerate() {
        let contents_id = 4 + 2 * index;
        let crop = match crop_inset {
            Some(inset) => format!(
                " /CropBox [{inset} {inset} {} {}]",
                page.width - inset,
                page.height - inset
            ),
            None => String::new(),
        };

        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}]{crop} /Contents {contents_id} 0 R \
             /Resources << /Font << /F1 {font_id} 0 R >> >> >>",
            page.width, page.height
        ));

        let stream = format!("BT /F1 24 Tf 20 100 Td ({}) Tj ET", page.text);
        objects.push(format!(
            "<< /Length {} >>\nstream\n{stream}\nendstream",
            stream.len()
        ));
    }

    objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned());

    let mut pdf = String::from("%PDF-1.4\n");
    let mut offsets = Vec::with_capacity(objects.len());
    for (index, body) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.push_str(&format!("{} 0 obj\n{body}\nendobj\n", index + 1));
    }

    let xref_offset = pdf.len();
    pdf.push_str(&format!(
        "xref\n0 {}\n0000000000 65535 f \n",
        objects.len() + 1
    ));
    for offset in &offsets {
        pdf.push_str(&format!("{offset:010} 00000 n \n"));
    }
    pdf.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF",
        objects.len() + 1
    ));

    pdf.into_bytes()
}
