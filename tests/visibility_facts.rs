//! Opt-in visibility facts (`PdfDocument::set_visibility_facts`): render mode,
//! ExtGState alpha, clip coverage, paint events, `/OC` on Form XObjects and the
//! off-page counter. Each PDF is a minimal synthetic one built in code.

use pdf_oxide::document::PdfDocument;
use pdf_oxide::extractors::{ClipKind, PaintKind, SpanVisibility};
use pdf_oxide::layout::TextSpan;
use pdf_oxide::optional_content::compute_default_off_ocgs;
use std::collections::HashSet;

/// One-page PDF: 1 catalog (+`catalog_extra`), 2 pages, 3 page (Font F1 plus
/// `resources_extra`), 4 content, 5 Helvetica, 6.. `extra` objects (each a
/// full object body; a `stream` body is given as `(dict, data)`).
fn pdf(
    catalog_extra: &str,
    resources_extra: &str,
    content: &[u8],
    extra: &[(String, Vec<u8>)],
) -> Vec<u8> {
    let mut objs: Vec<(String, Option<Vec<u8>>)> = vec![
        (format!("<< /Type /Catalog /Pages 2 0 R {catalog_extra} >>"), None),
        ("<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(), None),
        (
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
                 /Resources << /Font << /F1 5 0 R >> {resources_extra} >> >>"
            ),
            None,
        ),
        ("<< >>".into(), Some(content.to_vec())),
        (
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
                .into(),
            None,
        ),
    ];
    for (dict, data) in extra {
        objs.push((dict.clone(), (!data.is_empty()).then(|| data.clone())));
    }
    let mut out = b"%PDF-1.5\n".to_vec();
    let mut offs = Vec::new();
    for (i, (dict, data)) in objs.iter().enumerate() {
        offs.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        match data {
            Some(d) => {
                let dict = dict.trim_end_matches(">>").to_string();
                out.extend_from_slice(
                    format!("{dict} /Length {} >>\nstream\n", d.len()).as_bytes(),
                );
                out.extend_from_slice(d);
                out.extend_from_slice(b"\nendstream");
            },
            None => out.extend_from_slice(dict.as_bytes()),
        }
        out.extend_from_slice(b"\nendobj\n");
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

fn spans(doc: &PdfDocument) -> Vec<TextSpan> {
    let page = doc.extract_structured(0).expect("extract_structured");
    page.regions.into_iter().flat_map(|r| r.spans).collect()
}

fn facts_of(spans: &[TextSpan], needle: &str) -> SpanVisibility {
    let s = spans
        .iter()
        .find(|s| s.text.contains(needle))
        .unwrap_or_else(|| {
            panic!(
                "no span with {needle:?} in {:?}",
                spans.iter().map(|s| &s.text).collect::<Vec<_>>()
            )
        });
    s.visibility.expect("facts are on")
}

fn open_with_facts(bytes: Vec<u8>) -> PdfDocument {
    let doc = PdfDocument::from_bytes(bytes).unwrap();
    doc.set_visibility_facts(true);
    doc
}

#[test]
fn facts_are_off_by_default() {
    let bytes = pdf("", "", b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET", &[]);
    let doc = PdfDocument::from_bytes(bytes).unwrap();
    let s = spans(&doc);
    assert!(!s.is_empty());
    assert!(s.iter().all(|s| s.visibility.is_none()));
    assert!(doc.page_visibility(0).is_none());
}

#[test]
fn render_mode_invisible_and_clip_only() {
    let content = b"BT /F1 12 Tf 72 700 Td (VISIBLE) Tj ET\n\
                    BT 3 Tr /F1 12 Tf 72 650 Td (HIDDENTHREE) Tj ET\n\
                    BT 7 Tr /F1 12 Tf 72 600 Td (HIDDENSEVEN) Tj ET\n\
                    BT 0 Tr /F1 12 Tf 72 550 Td (SAMEBT) Tj 3 Tr (SWITCHED) Tj ET";
    let doc = open_with_facts(pdf("", "", content, &[]));
    let s = spans(&doc);
    assert_eq!(facts_of(&s, "VISIBLE").render_mode, 0);
    assert_eq!(facts_of(&s, "HIDDENTHREE").render_mode, 3);
    assert_eq!(facts_of(&s, "HIDDENSEVEN").render_mode, 7);
    // A Tr change inside one text object starts a new span.
    assert_eq!(facts_of(&s, "SAMEBT").render_mode, 0);
    assert_eq!(facts_of(&s, "SWITCHED").render_mode, 3);
    assert!(!s
        .iter()
        .any(|s| s.text.contains("SAMEBT") && s.text.contains("SWITCHED")));
}

#[test]
fn ext_gstate_alpha_is_applied_and_scoped_by_q() {
    let content = b"q /GS0 gs BT /F1 12 Tf 72 700 Td (ALPHAZERO) Tj ET Q\n\
                    q /GS1 gs BT /F1 12 Tf 72 650 Td (ALPHATINY) Tj ET Q\n\
                    q /GS2 gs BT /F1 12 Tf 72 600 Td (ALPHAHALF) Tj ET Q\n\
                    BT /F1 12 Tf 72 550 Td (OPAQUE) Tj ET";
    let res = "/ExtGState << /GS0 << /ca 0 /CA 0 >> /GS1 << /ca 0.02 >> \
               /GS2 << /ca 0.5 /SMask /None >> >>";
    let doc = open_with_facts(pdf("", res, content, &[]));
    let s = spans(&doc);
    assert_eq!(facts_of(&s, "ALPHAZERO").fill_alpha, 0.0);
    assert_eq!(facts_of(&s, "ALPHAZERO").stroke_alpha, 0.0);
    assert!((facts_of(&s, "ALPHATINY").fill_alpha - 0.02).abs() < 1e-6);
    assert!((facts_of(&s, "ALPHAHALF").fill_alpha - 0.5).abs() < 1e-6);
    assert!(!facts_of(&s, "ALPHAHALF").soft_mask);
    assert_eq!(facts_of(&s, "OPAQUE").fill_alpha, 1.0);
}

#[test]
fn clip_coverage_zero_elsewhere_and_inside() {
    let content = b"q 72 700 0 0 re W n BT /F1 12 Tf 72 700 Td (ZEROCLIP) Tj ET Q\n\
                    q 400 50 50 50 re W n BT /F1 12 Tf 72 650 Td (ELSEWHERE) Tj ET Q\n\
                    q 50 580 400 60 re W n BT /F1 12 Tf 72 600 Td (INSIDECLIP) Tj ET Q\n\
                    BT /F1 12 Tf 72 550 Td (NOCLIP) Tj ET";
    let doc = open_with_facts(pdf("", "", content, &[]));
    let s = spans(&doc);
    assert_eq!(facts_of(&s, "ZEROCLIP").clip_coverage, 0.0);
    assert_eq!(facts_of(&s, "ELSEWHERE").clip_coverage, 0.0);
    assert_eq!(facts_of(&s, "INSIDECLIP").clip_coverage, 1.0);
    assert_eq!(facts_of(&s, "NOCLIP").clip_coverage, 1.0);
}

#[test]
fn fills_and_images_share_the_span_paint_order() {
    let content = b"BT /F1 12 Tf 72 300 Td (UNDERRECT) Tj ET\n\
                    1 g 60 290 300 30 re f\n\
                    q 300 0 0 40 60 400 cm /Im0 Do Q\n\
                    BT /F1 12 Tf 72 410 Td (OVERIMAGE) Tj ET";
    let res = "/XObject << /Im0 6 0 R >>";
    let image = (
        "<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray \
         /BitsPerComponent 8 /SMask 7 0 R >>"
            .to_string(),
        vec![0xff],
    );
    let smask = (
        "<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray \
         /BitsPerComponent 8 >>"
            .to_string(),
        vec![0x80],
    );
    let doc = open_with_facts(pdf("", res, content, &[image, smask]));
    let s = spans(&doc);
    let pv = doc.page_visibility(0).expect("page facts");
    let rect = pv
        .paints
        .iter()
        .find(|p| p.kind == PaintKind::Rect)
        .expect("rect fill");
    assert_eq!(rect.color.map(|c| (c.r, c.g, c.b)), Some((1.0, 1.0, 1.0)));
    assert!((rect.bbox.x - 60.0).abs() < 0.01 && (rect.bbox.width - 300.0).abs() < 0.01);
    assert!(rect.paint_order > facts_of(&s, "UNDERRECT").paint_order);
    let img = pv
        .paints
        .iter()
        .find(|p| p.kind == PaintKind::Image)
        .expect("image");
    assert!(img.masked);
    assert!((img.bbox.y - 400.0).abs() < 0.01 && (img.bbox.height - 40.0).abs() < 0.01);
    assert!(img.paint_order < facts_of(&s, "OVERIMAGE").paint_order);
}

#[test]
fn optional_content_on_form_xobject_is_honoured_by_the_layer_filter() {
    let content = b"BT /F1 12 Tf 72 700 Td (PAGEBODY) Tj ET\n/X0 Do";
    let catalog = "/OCProperties << /OCGs [7 0 R] /D << /OFF [7 0 R] >> >>";
    let res = "/XObject << /X0 6 0 R >>";
    let form = (
        "<< /Type /XObject /Subtype /Form /BBox [0 0 612 792] /OC 7 0 R \
         /Resources << /Font << /F1 5 0 R >> >> >>"
            .to_string(),
        b"BT /F1 12 Tf 72 500 Td (INHIDDENFORM) Tj ET".to_vec(),
    );
    let ocg = ("<< /Type /OCG /Name (HiddenLayer) >>".to_string(), Vec::new());
    let bytes = pdf(catalog, res, content, &[form, ocg]);
    let doc = PdfDocument::from_bytes(bytes).unwrap();
    let all: String = spans(&doc).iter().map(|s| s.text.clone()).collect();
    assert!(all.contains("INHIDDENFORM"), "unfiltered extraction keeps it: {all}");
    let off = compute_default_off_ocgs(&doc);
    assert!(!off.is_empty());
    let filtered = doc
        .extract_structured_filtered(0, off, HashSet::new())
        .unwrap();
    let text: String = filtered.regions.iter().map(|r| r.text.clone()).collect();
    assert!(text.contains("PAGEBODY"), "{text}");
    assert!(!text.contains("INHIDDENFORM"), "{text}");
}

#[test]
fn offpage_drops_are_counted() {
    let content = b"BT /F1 12 Tf 72 700 Td (ONPAGE) Tj ET\n\
                    BT /F1 12 Tf 72 5000 Td (OFFPAGE) Tj ET";
    let doc = open_with_facts(pdf("", "", content, &[]));
    let s = spans(&doc);
    assert!(s.iter().any(|s| s.text.contains("ONPAGE")));
    assert!(!s.iter().any(|s| s.text.contains("OFFPAGE")));
    let pv = doc.page_visibility(0).expect("page facts");
    assert_eq!(pv.offpage_spans, 1);
    assert_eq!(pv.offpage_chars, "OFFPAGE".len());
}

#[test]
fn facts_do_not_change_extracted_text() {
    let content = b"BT /F1 12 Tf 72 700 Td (The quick brown fox) Tj ET\n\
                    q 1 0 0 1 0 0 cm BT /F1 12 Tf 72 680 Td [(jumps) -250 (over)] TJ ET Q";
    let bytes = pdf("", "", content, &[]);
    let plain = PdfDocument::from_bytes(bytes.clone()).unwrap();
    let with = open_with_facts(bytes);
    let text = |d: &PdfDocument| -> Vec<String> {
        d.extract_structured(0)
            .unwrap()
            .regions
            .into_iter()
            .map(|r| r.text)
            .collect()
    };
    assert_eq!(text(&plain), text(&with));
}

#[test]
fn page_facts_follow_the_layer_filter_of_the_structured_call() {
    let content = b"BT /F1 12 Tf 72 700 Td (BODY) Tj ET\nq 100 0 0 100 50 50 cm /Im0 Do Q";
    let catalog = "/OCProperties << /OCGs [7 0 R] /D << /OFF [7 0 R] >> >>";
    let res = "/XObject << /Im0 6 0 R >>";
    let image = (
        "<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray \
         /BitsPerComponent 8 /OC 7 0 R >>"
            .to_string(),
        vec![0x00],
    );
    let ocg = ("<< /Type /OCG /Name (HiddenLayer) >>".to_string(), Vec::new());
    let doc = open_with_facts(pdf(catalog, res, content, &[image, ocg]));
    let images = |d: &PdfDocument| {
        d.page_visibility(0)
            .expect("page facts")
            .paints
            .iter()
            .filter(|p| p.kind == PaintKind::Image)
            .count()
    };
    doc.extract_structured(0).unwrap();
    assert_eq!(images(&doc), 1, "unfiltered: the OC image is a paint");
    let off = compute_default_off_ocgs(&doc);
    doc.extract_structured_filtered(0, off, HashSet::new())
        .unwrap();
    assert_eq!(images(&doc), 0, "filtered: a hidden-layer image is not drawn");
    // Another API extracting the page unfiltered must not replace the facts.
    let _ = doc.classify_page(0);
    let _ = doc.extract_spans(0);
    assert_eq!(images(&doc), 0);
}

#[test]
fn text_after_the_operator_cap_survives_with_facts_on() {
    // A vector map larger than the 1,000,000-operator cap, then text. The full
    // parser that visibility facts route through used to stop at the cap and
    // drop the text silently (PMC4875632 page 2).
    let mut content = b"BT /F1 12 Tf 72 720 Td (BeforeTheMap) Tj ET\n".to_vec();
    for _ in 0..600_000 {
        content.extend_from_slice(b"0 0 1 1 re f\n");
    }
    content.extend_from_slice(b"BT /F1 12 Tf 72 700 Td (AfterTheMap) Tj ET");
    let bytes = pdf("", "", &content, &[]);
    let text = |d: &PdfDocument| -> String {
        d.extract_structured(0)
            .unwrap()
            .regions
            .into_iter()
            .map(|r| r.text)
            .collect::<Vec<_>>()
            .join("\n")
    };
    let plain = text(&PdfDocument::from_bytes(bytes.clone()).unwrap());
    assert!(plain.contains("AfterTheMap"), "{plain}");
    let doc = open_with_facts(bytes);
    let with = text(&doc);
    assert!(with.contains("BeforeTheMap") && with.contains("AfterTheMap"), "{with}");
    assert!(facts_of(&spans(&doc), "AfterTheMap").clip_coverage > 0.99);
}

fn clip_kind_of(content: &[u8]) -> ClipKind {
    let doc = open_with_facts(pdf("", "", content, &[]));
    facts_of(&spans(&doc), "PROBE").clip_kind
}

#[test]
fn clip_kind_per_clip_shape() {
    let probe = " BT /F1 12 Tf 72 600 Td (PROBE) Tj ET";
    let k = |clip: &str| clip_kind_of(format!("q {clip}{probe} Q").as_bytes());
    assert_eq!(clip_kind_of(probe.as_bytes()), ClipKind::None);
    assert_eq!(k("50 580 400 60 re W n"), ClipKind::Rect);
    assert_eq!(k("50 580 400 60 re W* n"), ClipKind::Rect);
    assert_eq!(k("W n"), ClipKind::Rect);
    assert_eq!(k("50 580 400 60 re 60 590 10 10 re W n"), ClipKind::Path);
    assert_eq!(k("50 580 m 450 580 l 450 640 l h W n"), ClipKind::Path);
    assert_eq!(k("0.9 0.1 -0.1 0.9 0 0 cm 50 580 400 60 re W n"), ClipKind::Path);
    assert_eq!(k("50 580 400 60 re 60 590 380 40 re W* n"), ClipKind::EvenOdd);
    assert_eq!(k("0.9 0.1 -0.1 0.9 0 0 cm 50 580 400 60 re W* n"), ClipKind::Path);
    // 90-degree turn stays axis-aligned.
    assert_eq!(k("0 1 -1 0 612 0 cm 50 580 400 60 re W n"), ClipKind::Rect);
}

#[test]
fn clip_kind_nested_q_restores_and_form_bbox_is_rect() {
    let content = b"q 0 0 612 792 re W n q 50 580 400 60 re 60 590 380 40 re W* n \
                    BT /F1 12 Tf 72 600 Td (RING) Tj ET Q \
                    BT /F1 12 Tf 72 560 Td (AFTER) Tj ET Q /X0 Do";
    let form = (
        "<< /Type /XObject /Subtype /Form /BBox [0 0 300 300] >>".to_string(),
        b"BT /F1 12 Tf 72 100 Td (INFORM) Tj ET".to_vec(),
    );
    let doc = open_with_facts(pdf("", "/XObject << /X0 6 0 R >>", content, &[form]));
    let s = spans(&doc);
    assert_eq!(facts_of(&s, "RING").clip_kind, ClipKind::EvenOdd);
    assert_eq!(facts_of(&s, "AFTER").clip_kind, ClipKind::Rect);
    assert_eq!(facts_of(&s, "INFORM").clip_kind, ClipKind::Rect);
}

#[test]
fn ring_clip_fill_is_tagged_and_keeps_bbox_semantics() {
    let content = b"q 50 580 400 60 re 60 590 380 40 re W* n \
                    1 g 0 0 612 792 re f \
                    BT /F1 12 Tf 72 600 Td (RINGTEXT) Tj ET Q \
                    1 g 10 10 20 20 re f";
    let doc = open_with_facts(pdf("", "", content, &[]));
    let s = spans(&doc);
    let pv = doc.page_visibility(0).expect("page facts");
    let ring = &pv.paints[0];
    assert_eq!(ring.kind, PaintKind::Rect);
    assert_eq!(ring.clip_kind, ClipKind::EvenOdd);
    // bbox is still the outer rectangle (unchanged semantics).
    assert!((ring.bbox.x - 50.0).abs() < 0.01 && (ring.bbox.width - 400.0).abs() < 0.01);
    let t = facts_of(&s, "RINGTEXT");
    assert_eq!(t.clip_kind, ClipKind::EvenOdd);
    assert_eq!(t.clip_coverage, 1.0);
    assert_eq!(pv.paints[1].clip_kind, ClipKind::None);
}

#[test]
fn fill_and_clip_in_one_path_uses_the_previous_clip() {
    let content = b"1 g 10 10 20 20 re W f BT /F1 12 Tf 72 700 Td (X) Tj ET";
    let doc = open_with_facts(pdf("", "", content, &[]));
    let s = spans(&doc);
    assert_eq!(facts_of(&s, "X").clip_kind, ClipKind::Rect);
    let pv = doc.page_visibility(0).expect("page facts");
    assert_eq!(pv.paints[0].clip_kind, ClipKind::None);
}
