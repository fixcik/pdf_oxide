//! Region text of `extract_structured`: same-font letter runs whose kerning
//! makes the next glyph start slightly before the previous advance ends
//! (Chrome/Skia letter positioning) are one word; runs that overlap more than
//! kerning does, or that are not on one upright line, keep their space. The
//! spans themselves are not merged (layout reads their boxes).
//!
//! Font: 11 pt, every glyph 600/1000 wide (6.6 pt) except `.`, `i`, `j`
//! (100/1000, 1.1 pt).

use pdf_oxide::document::PdfDocument;

const SIZE: f32 = 11.0;
const ADV: f32 = 6.6;

fn pdf(content: &str) -> Vec<u8> {
    let widths: Vec<&str> = (32..=126)
        .map(|c| {
            if matches!(c, 46 | 105 | 106) {
                "100"
            } else {
                "600"
            }
        })
        .collect();
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R >> >> >>"
            .to_string(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        format!(
            "<< /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding \
             /FirstChar 32 /LastChar 126 /Widths [{}] >>",
            widths.join(" ")
        ),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offs = Vec::new();
    for (i, body) in objs.iter().enumerate() {
        offs.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offs {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objs.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// Two runs on the baseline y = 700: `a` at x = 50, `b` where `a`'s advance
/// ends plus `gap` (negative = overlap). Each run in its own BT/ET, so they
/// reach the span merger as two spans (consecutive `Tj`s of one text object
/// are joined into one run before it, whatever the `Tm` between them).
fn two_runs(a: &str, b: &str, gap: f32) -> String {
    let x2 = 50.0 + ADV * a.len() as f32 + gap;
    format!("BT /F1 {SIZE} Tf 1 0 0 1 50 700 Tm ({a}) Tj ET BT /F1 {SIZE} Tf 1 0 0 1 {x2} 700 Tm ({b}) Tj ET")
}

fn span_texts(content: &str) -> Vec<String> {
    let doc = PdfDocument::from_bytes(pdf(content)).unwrap();
    doc.extract_spans(0)
        .unwrap()
        .into_iter()
        .map(|s| s.text)
        .filter(|t| !t.trim().is_empty())
        .collect()
}

fn region_text(content: &str) -> String {
    let doc = PdfDocument::from_bytes(pdf(content)).unwrap();
    let page = doc.extract_structured(0).unwrap();
    page.regions
        .iter()
        .map(|r| r.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn texts(content: &str) -> (String, Vec<String>) {
    (region_text(content), span_texts(content))
}

#[test]
fn kerned_pair_overlapping_more_than_half_a_point_is_one_word() {
    // «округ»+«лены» in the Chrome PDFs: -0.56 pt at 11 pt (-0.051 em).
    let (region, spans) = texts(&two_runs("roun", "ded", -0.56));
    assert_eq!(region.trim(), "rounded");
    assert_eq!(spans, vec!["roun", "ded"], "spans stay as they were");
}

#[test]
fn kerned_pair_up_to_the_kerning_bound_is_one_word() {
    // `T`+`o`-style kerning, -1.2 pt at 11 pt (-0.11 em).
    let (region, spans) = texts(&two_runs("T", "opics", -1.2));
    assert_eq!(region.trim(), "Topics");
    assert_eq!(spans.len(), 2);
}

#[test]
fn overlap_beyond_the_kerning_bound_keeps_the_space() {
    // -2.5 pt at 11 pt = -0.23 em: more than kerning does.
    assert_eq!(region_text(&two_runs("Total", "Amount", -2.5)).trim(), "Total Amount");
}

#[test]
fn overprinted_text_keeps_the_space() {
    // A second run drawn over the middle of the first (stamp, overlay).
    let c =
        "BT /F1 11 Tf 1 0 0 1 50 700 Tm (Hello) Tj ET BT /F1 11 Tf 1 0 0 1 60 700 Tm (World) Tj ET";
    assert!(!region_text(c).contains("HelloWorld"));
}

#[test]
fn narrow_run_offset_duplicate_is_not_a_kerned_pair() {
    // Two 1.1 pt wide letters 0.3 pt apart (an offset duplicate): the 0.8 pt
    // overlap is within 0.15 em but more than half the narrower run.
    let c = "BT /F1 11 Tf 1 0 0 1 50 700 Tm (i) Tj ET BT /F1 11 Tf 1 0 0 1 50.3 700 Tm (j) Tj ET";
    assert!(!region_text(c).contains("ij"), "{:?}", region_text(c));
}

#[test]
fn overlap_at_a_non_letter_boundary_keeps_the_space() {
    // A number and its unit, or a run ending in a space glyph.
    assert!(!region_text(&two_runs("50", "ml", -0.8)).contains("50ml"));
    assert!(region_text(&two_runs("at ", "home", -0.8)).contains("at home"));
}

#[test]
fn table_cells_and_columns_keep_their_space() {
    assert!(region_text(&two_runs("Cell", "Next", 20.0)).contains("Cell"));
    assert!(!region_text(&two_runs("Cell", "Next", 20.0)).contains("CellNext"));
    assert!(!region_text(&two_runs("left", "right", 40.0)).contains("leftright"));
}

#[test]
fn kerned_overlap_on_another_line_keeps_the_space() {
    let x2 = 50.0 + ADV * 4.0 - 0.56;
    let c = format!(
        "BT /F1 11 Tf 1 0 0 1 50 700 Tm (roun) Tj ET BT /F1 11 Tf 1 0 0 1 {x2} 698 Tm (ded) Tj ET"
    );
    assert!(!region_text(&c).contains("rounded"));
}

#[test]
fn rotated_runs_with_a_kerned_overlap_keep_the_space() {
    // 90° runs advance along y, so the |dy| < 1 guard rejects them (the
    // `upright` check is a second line of defence for this case).
    let y2 = 300.0 + ADV * 4.0 - 0.56;
    let c = format!(
        "BT /F1 11 Tf 0 1 -1 0 300 300 Tm (roun) Tj ET BT /F1 11 Tf 0 1 -1 0 300 {y2} Tm (ded) Tj ET"
    );
    assert!(!region_text(&c).contains("rounded"));
}

#[test]
fn different_font_size_keeps_the_space() {
    // -1.2 pt: outside the merger's small-caps glue (|gap| < 1 pt).
    let x2 = 50.0 + ADV * 4.0 - 1.2;
    let c = format!(
        "BT /F1 11 Tf 1 0 0 1 50 700 Tm (roun) Tj ET BT /F1 10 Tf 1 0 0 1 {x2} 700 Tm (ded) Tj ET"
    );
    assert!(!region_text(&c).contains("rounded"));
}
