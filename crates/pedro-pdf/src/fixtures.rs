//! A PDF written out by hand, for tests across this workspace.
//!
//! Not a checked-in binary fixture: what the tests assert is the relationship
//! between what goes onto a page and what comes back off it, and a binary
//! hides half of that.

/// Builds a valid single-font PDF with one page per string, each page carrying
/// that string as its only text.
pub fn pdf_with_pages(pages: &[&str]) -> Vec<u8> {
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

    for (index, text) in pages.iter().enumerate() {
        let contents_id = 4 + 2 * index;
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Contents {contents_id} 0 R \
             /Resources << /Font << /F1 {font_id} 0 R >> >> >>"
        ));

        let stream = format!("BT /F1 24 Tf 20 100 Td ({text}) Tj ET");
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
