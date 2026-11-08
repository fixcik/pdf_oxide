//! A Form XObject whose own /Font dictionary reuses a page font name
//! (`/T1_0` of the form vs `/T1_0` of the page) draws with the form's font
//! on every page, whichever page loaded the form's fonts first. The font-set
//! caches of `load_fonts` used to keep only the names the page did not
//! already have, so a later page that hit the name-keyed cache drew the
//! form's text with the page font.

use pdf_oxide::document::PdfDocument;

fn build(objs: &[String]) -> Vec<u8> {
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offs = Vec::new();
    for (i, body) in objs.iter().enumerate() {
        offs.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offs {
        pdf.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objs.len() + 1
        )
        .as_bytes(),
    );
    pdf
}

fn stream(dict: &str, data: &str) -> String {
    format!("<< {dict} /Length {} >>\nstream\n{data}\nendstream", data.len())
}

/// `pages` pages; page `i` has `/T1_0` = Helvetica and paints its own form,
/// whose inline /Font dict maps `/T1_0` to its own (content-identical)
/// Courier object.
fn shadowing_pdf(pages: usize) -> Vec<u8> {
    let per_page = 4; // page, content, form, courier
    let first = 4;
    let kids: Vec<String> = (0..pages)
        .map(|i| format!("{} 0 R", first + per_page * i))
        .collect();
    let mut objs = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!("<< /Type /Pages /Kids [{}] /Count {pages} >>", kids.join(" ")),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_string(),
    ];
    for i in 0..pages {
        let page = first + per_page * i;
        objs.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {} 0 R \
             /Resources << /Font << /T1_0 3 0 R >> /XObject << /Fm {} 0 R >> >> >>",
            page + 1,
            page + 2
        ));
        objs.push(stream(
            "",
            &format!("q /Fm Do Q BT /T1_0 12 Tf 72 400 Td (Body of page {i}) Tj ET"),
        ));
        objs.push(stream(
            &format!(
                "/Type /XObject /Subtype /Form /BBox [0 0 612 792] \
                 /Resources << /Font << /T1_0 {} 0 R >> >>",
                page + 3
            ),
            "BT /T1_0 10 Tf 72 700 Td (MMMM) Tj ET",
        ));
        objs.push(
            "<< /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding >>"
                .to_string(),
        );
    }
    build(&objs)
}

/// Width of the form's run on `page`: Courier is 600/1000 per glyph, so
/// four glyphs at 10 pt are 24 pt (Helvetica's `M` would give 33.3 pt).
fn form_run_width(doc: &PdfDocument, page: usize) -> f32 {
    let spans = doc.extract_spans(page).unwrap();
    let s = spans
        .iter()
        .find(|s| s.text.contains("MMMM"))
        .unwrap_or_else(|| panic!("page {page}: no form run in {:?}", spans));
    s.bbox.width
}

#[test]
fn form_font_shadowing_a_page_font_name_is_used_on_every_page() {
    let doc = PdfDocument::from_bytes(shadowing_pdf(3)).unwrap();
    for page in 0..3 {
        let w = form_run_width(&doc, page);
        assert!(
            (w - 24.0).abs() < 0.5,
            "page {page}: form run drawn with the page font (width {w})"
        );
    }
}

#[test]
fn form_font_shadowing_in_reverse_page_order() {
    let doc = PdfDocument::from_bytes(shadowing_pdf(3)).unwrap();
    for page in (0..3).rev() {
        let w = form_run_width(&doc, page);
        assert!(
            (w - 24.0).abs() < 0.5,
            "page {page}: form run drawn with the page font (width {w})"
        );
    }
}
