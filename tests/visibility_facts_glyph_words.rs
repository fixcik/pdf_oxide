//! Visibility facts of items 1–3 of the third fork request: per-glyph tiny
//! size facts, per-char facts on `extract_words`, and the blend mode on
//! paints and spans.

mod visibility_common;

use pdf_oxide::document::PdfDocument;
use pdf_oxide::extractors::{BlendMode, TINY_GLYPH_PT};
use visibility_common::{facts_of, open_with_facts, pdf, span_of, spans};

#[test]
fn letter_spaced_tiny_run_is_flagged_per_glyph() {
    // 0.5 pt glyphs spread 6 pt apart by Tc: the span bbox is tiny in height
    // but wide per character, so a span-level "height <= 1 and width/n <= 1"
    // check misses it.
    let content = b"BT /F1 12 Tf 72 700 Td (VISIBLE) Tj ET\n\
                    BT /F1 0.5 Tf 6 Tc 72 650 Td (PWNED) Tj ET";
    let s = spans(&open_with_facts(pdf("", "", content, &[])));
    let tiny = span_of(&s, "PWNED");
    let n = tiny.text.chars().count() as f32;
    assert!(tiny.bbox.height <= TINY_GLYPH_PT);
    assert!(tiny.bbox.width / n > TINY_GLYPH_PT, "span-level advance check passes it");
    let v = tiny.visibility.unwrap();
    assert_eq!(v.tiny_glyphs, 5);
    assert!(v.glyph_advance_max.unwrap() <= TINY_GLYPH_PT);
    assert!((v.glyph_size_min - 0.5).abs() < 1e-4);
    let vis = facts_of(&s, "VISIBLE");
    assert_eq!(vis.tiny_glyphs, 0);
    assert!(vis.glyph_advance_max.unwrap() > TINY_GLYPH_PT);
    assert!((vis.glyph_size_min - 12.0).abs() < 1e-4);
}

#[test]
fn tiny_run_merged_into_a_normal_span_keeps_its_glyph_facts() {
    // A tiny run right after a normal one on the same baseline is merged into
    // one span whose bbox (and font_size) is that of the normal run.
    let content = b"BT /F1 12 Tf 72 700 Td (Hello) Tj /F1 0.5 Tf (PWNED) Tj ET";
    let s = spans(&open_with_facts(pdf("", "", content, &[])));
    let merged = span_of(&s, "Hello");
    assert!(merged.text.contains("PWNED"), "runs merged: {:?}", merged.text);
    assert!(merged.bbox.height > TINY_GLYPH_PT);
    let v = merged.visibility.unwrap();
    assert_eq!(v.tiny_glyphs, 5);
    assert!((v.glyph_size_min - 0.5).abs() < 1e-4);
    assert!(v.glyph_advance_max.unwrap() > TINY_GLYPH_PT);
}

#[test]
fn actual_text_run_has_no_glyph_advance() {
    let content = b"BT /F1 0.5 Tf 72 700 Td /Span << /ActualText (REPL) >> BDC (x) Tj EMC ET";
    let s = spans(&open_with_facts(pdf("", "", content, &[])));
    let v = facts_of(&s, "REPL");
    assert_eq!(v.glyph_advance_max, None);
    assert_eq!(v.tiny_glyphs, 0);
}

/// A 2x2 ruled table; the bottom-right cell's word is drawn with `3 Tr`.
fn table_pdf() -> Vec<u8> {
    let content = b"0 G 1 w 72 600 200 60 re S 172 600 m 172 660 l S 72 630 m 272 630 l S\n\
                    BT /F1 10 Tf 80 640 Td (Name) Tj ET\n\
                    BT /F1 10 Tf 180 640 Td (Value) Tj ET\n\
                    BT /F1 10 Tf 80 610 Td (Alpha) Tj ET\n\
                    BT 3 Tr /F1 10 Tf 180 610 Td (SECRET) Tj ET";
    pdf("", "", content, &[])
}

#[test]
fn extract_words_carries_per_char_facts_when_on() {
    let doc = open_with_facts(table_pdf());
    let words = doc.extract_words(0).unwrap();
    assert!(words.len() >= 4, "{:?}", words.iter().map(|w| &w.text).collect::<Vec<_>>());
    for w in &words {
        assert_eq!(w.char_visibility.len(), w.chars.len(), "word {:?}", w.text);
    }
    let word = |t: &str| words.iter().find(|w| w.text == t).expect(t);
    assert!(word("SECRET")
        .char_visibility
        .iter()
        .all(|v| v.render_mode == 3));
    for t in ["Name", "Value", "Alpha"] {
        assert!(word(t).char_visibility.iter().all(|v| v.render_mode == 0), "{t}");
    }
}

#[test]
fn extract_words_has_no_facts_when_off() {
    let doc = PdfDocument::from_bytes(table_pdf()).unwrap();
    let words = doc.extract_words(0).unwrap();
    assert!(words.iter().any(|w| w.text == "SECRET"));
    assert!(words.iter().all(|w| w.char_visibility.is_empty()));
    let json = serde_json::to_string(&words).unwrap();
    assert!(!json.contains("char_visibility"));
}

#[test]
fn blend_mode_is_recorded_on_paints_and_spans() {
    let content = b"0 0 1 rg 72 500 100 20 re f\n\
                    q /GSM gs 1 1 0 rg 72 700 100 20 re f BT /F1 12 Tf 72 705 Td (UNDERMULTIPLY) Tj ET Q\n\
                    q /GSA gs 72 400 100 20 re f Q\n\
                    q /GSN gs BT /F1 12 Tf 72 300 Td (NORMALGS) Tj ET Q\n\
                    BT /F1 12 Tf 72 200 Td (AFTERQ) Tj ET";
    let res = "/ExtGState << /GSM << /BM /Multiply >> /GSA << /BM [/Screen /Normal] >> \
               /GSN << /BM /Compatible >> >>";
    let doc = open_with_facts(pdf("", res, content, &[]));
    let s = spans(&doc);
    let paints = doc.page_visibility(0).expect("page facts").paints;
    let modes: Vec<BlendMode> = paints.iter().map(|p| p.blend_mode).collect();
    assert_eq!(modes, [BlendMode::Normal, BlendMode::Multiply, BlendMode::Screen]);
    let m = facts_of(&s, "UNDERMULTIPLY");
    assert_eq!(m.blend_mode, BlendMode::Multiply);
    assert!(m.blend_non_normal);
    for t in ["NORMALGS", "AFTERQ"] {
        assert_eq!(facts_of(&s, t).blend_mode, BlendMode::Normal, "{t}");
        assert!(!facts_of(&s, t).blend_non_normal);
    }
}

#[test]
fn word_mixing_visible_and_hidden_runs_keeps_per_char_facts() {
    // Two runs of one word: the facts keep them apart as spans, the word
    // merge joins them, and each char keeps the facts of its own run.
    let content = b"BT /F1 12 Tf 72 700 Td (VIS) Tj 3 Tr (HID) Tj ET";
    let doc = open_with_facts(pdf("", "", content, &[]));
    let words = doc.extract_words(0).unwrap();
    let w = words
        .iter()
        .find(|w| w.text == "VISHID")
        .unwrap_or_else(|| panic!("{:?}", words.iter().map(|w| &w.text).collect::<Vec<_>>()));
    assert_eq!(w.char_visibility.len(), w.chars.len());
    let modes: Vec<u8> = w.char_visibility.iter().map(|v| v.render_mode).collect();
    assert_eq!(modes, [0, 0, 0, 3, 3, 3]);
}
