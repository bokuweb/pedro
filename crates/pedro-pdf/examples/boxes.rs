//! Checks that a page's character boxes land on the page's own ink.
//!
//! ```bash
//! cargo run --release -p pedro-pdf --example boxes -- book.pdf 18
//! ```
//!
//! Written after a book whose marks were drawn a line above its words: pdfium
//! rasterises the crop box but reports characters in media box coordinates, and
//! a page inset by trim marks has a different origin in each. The rendered page
//! is the only thing that can settle where a character really is, so this draws
//! the claim over the evidence — `/tmp/pedro-boxes.png` — and counts the ink
//! inside each box.

use pedro_pdf::Document;

/// Big enough that a glyph is tens of pixels rather than a handful, which is
/// what makes counting ink mean anything.
const SCALE: f32 = 4.0;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let (Some(path), Some(what)) = (arguments.next(), arguments.next()) else {
        eprintln!("usage: boxes <file.pdf> <page|find> [needle]");
        std::process::exit(2);
    };

    let document = Document::open(path.as_ref())?;

    if what == "find" {
        let needle = arguments.next().expect("a needle to look for");
        for page in 0..document.page_count() {
            if document.page_text(page)?.text.contains(&needle) {
                println!("page {}", page + 1);
            }
        }
        return Ok(());
    }

    let page: u32 = what.parse()?;
    let image = document.render_page(page - 1, SCALE)?;
    let text = document.page_text(page - 1)?;

    for boundary in document.page_boxes(page - 1)? {
        println!(
            "{:>8} box ({:.2}, {:.2}, {:.2}, {:.2})",
            boundary.0, boundary.1, boundary.2, boundary.3, boundary.4
        );
    }
    let outline = document.outline();
    println!("outline: {} top-level entries", outline.len());
    for chapter in outline.iter().take(5) {
        println!("  p.{:>4} {}", chapter.page_number, chapter.title);
    }

    println!(
        "{} characters, {} boxed, rendered {} x {}",
        text.text.chars().count(),
        text.chars.len(),
        image.width,
        image.height
    );

    let ink_in = |rect: pedro_pdf::Rect| {
        let x =
            |fraction: f32| (fraction * image.width as f32).clamp(0.0, image.width as f32) as u32;
        let y =
            |fraction: f32| (fraction * image.height as f32).clamp(0.0, image.height as f32) as u32;

        (y(rect.top)..y(rect.bottom))
            .flat_map(|row| (x(rect.left)..x(rect.right)).map(move |column| (row, column)))
            .filter(|(row, column)| {
                let at = ((row * image.width + column) * 4) as usize;
                image.bytes.get(at).is_some_and(|blue| *blue < 128)
            })
            .count()
    };

    for character in text.chars.iter().take(8) {
        let rect = character.rect;
        let drop = rect.height();
        let moved = pedro_pdf::Rect {
            top: rect.top + drop,
            bottom: rect.bottom + drop,
            ..rect
        };

        println!(
            "character {:>4}: ink in its box {:>4}, in the box below it {:>4}",
            character.index,
            ink_in(rect),
            ink_in(moved)
        );
    }

    // The claim drawn over the evidence.
    let mut canvas = image::RgbaImage::new(image.width, image.height);
    for (at, pixel) in image.bytes.as_chunks::<4>().0.iter().enumerate() {
        let (x, y) = (at as u32 % image.width, at as u32 / image.width);
        canvas.put_pixel(x, y, image::Rgba([pixel[2], pixel[1], pixel[0], 255]));
    }

    for character in text.chars.iter().take(80) {
        let rect = character.rect;
        let x0 = (rect.left * image.width as f32) as u32;
        let x1 = ((rect.right * image.width as f32) as u32).min(image.width - 1);
        let y0 = (rect.top * image.height as f32) as u32;
        let y1 = ((rect.bottom * image.height as f32) as u32).min(image.height - 1);

        for x in x0..=x1 {
            canvas.put_pixel(x, y0, image::Rgba([220, 0, 0, 255]));
            canvas.put_pixel(x, y1, image::Rgba([220, 0, 0, 255]));
        }
        for y in y0..=y1 {
            canvas.put_pixel(x0, y, image::Rgba([220, 0, 0, 255]));
            canvas.put_pixel(x1, y, image::Rgba([220, 0, 0, 255]));
        }
    }

    canvas.save("/tmp/pedro-boxes.png")?;
    println!("drew the boxes over the page in /tmp/pedro-boxes.png");

    Ok(())
}
