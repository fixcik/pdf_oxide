//! Letter-spaced (tracked) text must not gain spaces between its letters.
//!
//! HTML-to-PDF producers render CSS `letter-spacing` by positioning every
//! glyph on its own (`dx 0 Td <glyph> Tj`), adding the same tracking amount to
//! every advance and drawing word spaces as real space glyphs. With 0.2 em of
//! tracking the uniform inter-letter gap exceeds the fixed geometric space
//! threshold, so `CONTACTS` came out as `C O N T A C T S`. When the producer
//! splits the glyphs over several font resources (one subset per few glyphs),
//! the same gap also exceeds the cross-font glue ceiling and every letter
//! becomes its own span.
//!
//! ISO 32000-1:2008 §9.3.2: character spacing is added to *every* glyph
//! advance, so it is part of the line's letter spacing, not a word boundary.
//! The space decision must measure a gap against the run's own tracking.

use pdf_oxide::document::PdfDocument;

/// Helvetica advance widths (thousandths of an em) for the glyphs used here.
fn helvetica_width(c: char) -> f64 {
    match c {
        'A' | 'B' | 'E' | 'S' => 667.0,
        'C' | 'D' | 'N' | 'O' | 'U' => 722.0,
        'F' | 'T' => 611.0,
        'a' | 'b' | 'd' | 'e' => 556.0,
        'c' => 500.0,
        'f' | ' ' => 278.0,
        _ => panic!("no width for {c:?} in this fixture"),
    }
}

/// One page, one `BT … ET`, each glyph drawn by its own `Tj` and positioned by
/// a relative `Td` that is the glyph advance plus `tracking` points. Glyphs
/// alternate between font resources `/F1` and `/F2` (same metrics, different
/// style) when `alternate_fonts`.
/// A `' '` in `text` is drawn as a space glyph unless `space_glyphs` is false,
/// in which case the pen just advances by the space width plus tracking.
fn make_tracked_pdf(
    text: &str,
    size: f64,
    tracking: f64,
    alternate_fonts: bool,
    space_glyphs: bool,
) -> Vec<u8> {
    let mut stream = format!("BT /F1 {size} Tf 1 0 0 1 72 700 Tm\n");
    let mut pending = 0.0;
    let mut font = 1;
    for c in text.chars() {
        if c == ' ' && !space_glyphs {
            pending += helvetica_width(c) * size / 1000.0 + tracking;
            continue;
        }
        if alternate_fonts {
            stream.push_str(&format!("/F{font} {size} Tf\n"));
            font = 3 - font;
        }
        let escaped = if c == '(' || c == ')' {
            format!("\\{c}")
        } else {
            c.to_string()
        };
        stream.push_str(&format!("{pending:.4} 0 Td ({escaped}) Tj\n"));
        pending = helvetica_width(c) * size / 1000.0 + tracking;
    }
    stream.push_str("ET\n");
    build_pdf(&stream)
}

/// One page drawing `text` with a single `Tj` under `Tc` character spacing.
fn make_tc_pdf(text: &str, size: f64, char_spacing: f64) -> Vec<u8> {
    build_pdf(&format!(
        "BT /F1 {size} Tf {char_spacing} Tc 1 0 0 1 72 700 Tm ({text}) Tj ET\n"
    ))
}

fn build_pdf(stream: &str) -> Vec<u8> {
    let mut pdf: Vec<u8> = Vec::new();
    let mut offsets = Vec::new();
    pdf.extend_from_slice(b"%PDF-1.4\n");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R /F2 6 0 R >> >> >>"
            .to_string(),
        format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len() + 1),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Oblique >>".to_string(),
    ];
    for (i, body) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \r\n", objects.len() + 1).as_bytes(),
    );
    for off in offsets {
        pdf.extend_from_slice(format!("{off:010} 00000 n \r\n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    pdf
}

/// The page's span texts joined the way a line consumer joins them.
fn line_text(pdf: Vec<u8>) -> String {
    let doc = PdfDocument::from_bytes(pdf).unwrap();
    let spans = doc.extract_spans(0).unwrap();
    spans
        .iter()
        .map(|s| s.text.trim())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn tracked_glyph_run_keeps_word_space_without_intra_word_spaces() {
    // 0.2 em tracking at 10 pt: every gap is 2 pt, above the fixed threshold.
    let text = line_text(make_tracked_pdf("ABC DEF", 10.0, 2.0, false, true));
    assert_eq!(text, "ABC DEF");
}

#[test]
fn tracked_glyph_run_without_space_glyphs_keeps_word_gap() {
    // No space glyph: the word gap is the space advance plus the tracking,
    // clearly wider than the tracked letter gap.
    let text = line_text(make_tracked_pdf("CONTACTS ABOUT", 10.0, 2.0, false, false));
    assert_eq!(text, "CONTACTS ABOUT");
}

#[test]
fn tracked_glyph_run_across_font_resources_is_one_word() {
    // Glyphs alternate between two font resources, as in producers that emit
    // many small font subsets; the tracked gap must not split the word.
    let text = line_text(make_tracked_pdf("CONTACTS", 10.0, 1.4, true, true));
    assert_eq!(text, "CONTACTS");
}

#[test]
fn char_spacing_tracked_show_keeps_word_space() {
    let text = line_text(make_tc_pdf("ABC DEF", 10.0, 2.0));
    assert_eq!(text, "ABC DEF");
}

#[test]
fn untracked_glyph_run_is_unchanged() {
    // Control: no tracking, words separated only by the space advance.
    let text = line_text(make_tracked_pdf("ABC DEF", 10.0, 0.0, false, false));
    assert_eq!(text, "ABC DEF");
    let text = line_text(make_tracked_pdf("ABC DEF", 10.0, 0.0, false, true));
    assert_eq!(text, "ABC DEF");
}

#[test]
fn per_word_shows_without_space_glyphs_keep_their_spaces() {
    // Control: one show per word, no space glyphs, uniform word-sized gaps.
    // The gaps are word gaps, not tracking, so they must stay spaces.
    let mut stream = String::from("BT /F1 10 Tf 1 0 0 1 72 700 Tm\n");
    let mut pending = 0.0;
    for word in ["ABC", "DEF", "CAB", "FED", "BAD"] {
        stream.push_str(&format!("{pending:.4} 0 Td ({word}) Tj\n"));
        pending = word.chars().map(helvetica_width).sum::<f64>() * 10.0 / 1000.0 + 2.0;
    }
    stream.push_str("ET\n");
    assert_eq!(line_text(build_pdf(&stream)), "ABC DEF CAB FED BAD");
}

#[test]
fn tracked_glyph_run_reaches_structured_regions_glued() {
    let doc =
        PdfDocument::from_bytes(make_tracked_pdf("CONTACTS ABOUT", 10.0, 2.0, true, true)).unwrap();
    let page = doc.extract_structured(0).unwrap();
    let text: Vec<&str> = page.regions.iter().map(|r| r.text.trim()).collect();
    assert_eq!(text.join(" "), "CONTACTS ABOUT");
}

#[test]
fn justified_pieces_with_single_letter_word_edges_keep_their_spaces() {
    // Control: a justified line drawn in multi-glyph pieces where the
    // single-letter pieces sit on both sides of each (uniform) word gap. Those
    // single-letter gaps are word gaps, not tracking.
    let mut stream = String::from("BT /F1 10 Tf 1 0 0 1 72 700 Tm\n");
    let mut pending = 0.0;
    for _ in 0..3 {
        for (piece, gap_after) in [("ab", 0.0), ("c", 2.0), ("d", 0.0), ("ef", 2.0)] {
            stream.push_str(&format!("{pending:.4} 0 Td ({piece}) Tj\n"));
            pending = piece.chars().map(helvetica_width).sum::<f64>() * 10.0 / 1000.0 + gap_after;
        }
    }
    stream.push_str("ET\n");
    assert_eq!(line_text(build_pdf(&stream)), "abc def abc def abc def");
}
