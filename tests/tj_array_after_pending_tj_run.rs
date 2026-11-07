//! A `TJ` array must not overtake a show-text run that is still pending.
//!
//! Span extraction buffers consecutive `Tj` operators (and same-line `Tm`
//! continuations) into one pending run that is only emitted when the pen
//! moves (`Td`, a new line, a font change, ...). A `TJ` array, however, is
//! assembled in its own buffer and emitted on the spot. When a producer
//! starts a word with `Tj` pieces and finishes it with a `TJ` array — the
//! justified-text shape where the first glyphs of a line are placed by
//! `Tm` and the rest of the word is a kerned `TJ` — two things went wrong:
//!
//! 1. the `TJ` piece was emitted before the earlier pending piece, and
//! 2. a `Tj` drawn after the `TJ` (typically the word space) was appended to
//!    the stale pending run, i.e. placed at the start of the word.
//!
//! Characters were correct; only the span text was permuted ("al pha"
//! instead of "alpha", or a later word torn out and glued to an earlier
//! span). Per ISO 32000-1:2008 §9.4.3 every show-text operator paints at
//! the current pen position in content-stream order, so the pending run
//! has to be closed before a `TJ` array emits anything.

use pdf_oxide::document::PdfDocument;

/// One line: `(a) Tj`, a same-line `Tm` continuation, `(l) Tj`, another
/// continuation, `[(pha)] TJ`, the word space as `( ) Tj`, then `Td` to the
/// next word `(beta) Tj`. Every glyph is 500/1000 em (5 pt at 10 pt), the
/// space 250/1000 em, so each `Tm` lands exactly at the pen position.
fn split_word_pdf(tj_array: &str) -> Vec<u8> {
    let content = format!(
        "BT /F1 10 Tf\n\
         1 0 0 1 100 700 Tm (a) Tj\n\
         1 0 0 1 105 700 Tm (l) Tj\n\
         1 0 0 1 110 700 Tm {tj_array} TJ\n\
         ( ) Tj\n\
         17.5 0 Td (beta) Tj\n\
         ET"
    );
    build_minimal_pdf(content.as_bytes())
}

fn span_words(pdf: Vec<u8>) -> Vec<String> {
    let doc = PdfDocument::from_bytes(pdf).expect("open");
    let spans = doc.extract_spans(0).expect("spans");
    let joined: Vec<&str> = spans.iter().map(|s| s.text.as_str()).collect();
    joined
        .join(" ")
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

#[test]
fn tj_array_after_pending_tj_run_keeps_word_whole() {
    assert_eq!(span_words(split_word_pdf("[(pha)]")), ["alpha", "beta"]);
}

#[test]
fn kerned_tj_array_after_pending_tj_run_keeps_word_whole() {
    // Sub-threshold kerning inside the array, as justified producers emit.
    assert_eq!(span_words(split_word_pdf("[(p) 1 (h) -1 (a)]")), ["alpha", "beta"]);
}

#[test]
fn space_drawn_after_tj_array_stays_after_it() {
    // The word space follows the `TJ` in the stream, so no span may start
    // left of the `TJ` text and carry the space.
    let doc = PdfDocument::from_bytes(split_word_pdf("[(pha)]")).expect("open");
    let spans = doc.extract_spans(0).expect("spans");
    for s in &spans {
        if s.text.ends_with(' ') {
            assert!(
                s.bbox.x + s.bbox.width >= 125.0 - 0.01,
                "span {:?} at x={} w={} carries the space drawn after the TJ at x=125",
                s.text,
                s.bbox.x,
                s.bbox.width
            );
        }
    }
}

/// A raised citation drawn as `Tj` `TJ` `Tj` pieces after a body word:
/// `(settings) Tj`, then at a smaller size and a raised baseline
/// `(8,9,) Tj [(21)] TJ (,22) Tj`, then `(, and) Tj` back on the baseline.
/// Each piece starts where the previous one ends.
fn raised_citation_pdf() -> Vec<u8> {
    let content = "BT /F1 10 Tf 1 0 0 1 100 700 Tm (settings) Tj\n\
                   /F1 6 Tf 1 0 0 1 140 703 Tm (8,9,) Tj [(21)] TJ (,22) Tj\n\
                   /F1 10 Tf 1 0 0 1 170 700 Tm (, and) Tj ET";
    build_minimal_pdf(content.as_bytes())
}

#[test]
fn raised_citation_pieces_stay_on_their_line_in_order() {
    let doc = PdfDocument::from_bytes(raised_citation_pdf()).expect("open");
    let spans = doc.extract_spans(0).expect("spans");
    let base_y = spans
        .iter()
        .find(|s| s.text.contains("settings"))
        .expect("base word")
        .bbox
        .y;
    for s in &spans {
        assert!(
            (s.bbox.y - base_y).abs() < 1.0,
            "span {:?} at y={} left the line at y={}",
            s.text,
            s.bbox.y,
            base_y
        );
    }
    let text: String = spans.iter().map(|s| s.text.as_str()).collect();
    let squeezed: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(
        squeezed.contains("settings8,9,21,22,and"),
        "citation pieces out of order: {text:?}"
    );
}

fn build_minimal_pdf(content: &[u8]) -> Vec<u8> {
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![0usize];

    offsets.push(pdf.len());
    pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
    offsets.push(pdf.len());
    pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
    offsets.push(pdf.len());
    pdf.extend_from_slice(
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
          /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>\nendobj\n",
    );
    offsets.push(pdf.len());
    pdf.extend_from_slice(format!("4 0 obj\n<< /Length {} >>\nstream\n", content.len()).as_bytes());
    pdf.extend_from_slice(content);
    pdf.extend_from_slice(b"\nendstream\nendobj\n");

    let mut widths = vec!["500"; 95];
    widths[0] = "250";
    offsets.push(pdf.len());
    pdf.extend_from_slice(
        format!(
            "5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica \
             /Encoding /WinAnsiEncoding /FirstChar 32 /LastChar 126 /Widths [{}] >>\nendobj\n",
            widths.join(" ")
        )
        .as_bytes(),
    );

    let xref_pos = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n", offsets.len()).as_bytes());
    pdf.extend_from_slice(b"0000000000 65535 f\r\n");
    for &off in &offsets[1..] {
        pdf.extend_from_slice(format!("{off:010} 00000 n\r\n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n",
            offsets.len()
        )
        .as_bytes(),
    );
    pdf
}
