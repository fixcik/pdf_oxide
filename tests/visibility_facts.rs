//! Opt-in visibility facts (`PdfDocument::set_visibility_facts`): render mode,
//! ExtGState alpha, clip coverage, paint events, `/OC` on Form XObjects and the
//! off-page counter. Each PDF is a minimal synthetic one built in code.

use pdf_oxide::document::PdfDocument;
use pdf_oxide::extractors::{PaintKind, SpanVisibility};
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

fn span_of<'a>(spans: &'a [TextSpan], needle: &str) -> &'a TextSpan {
    spans
        .iter()
        .find(|s| s.text.contains(needle))
        .unwrap_or_else(|| panic!("no span with {needle:?}"))
}

fn is_rgb(c: Option<pdf_oxide::layout::Color>, rgb: (f32, f32, f32)) -> bool {
    c.is_some_and(|c| {
        (c.r - rgb.0).abs() < 1e-3 && (c.g - rgb.1).abs() < 1e-3 && (c.b - rgb.2).abs() < 1e-3
    })
}

/// Colour spaces selected by resource name (`/Cs8 cs 1 scn`), as real files do.
const NAMED_SPACES: &str = "/ColorSpace << \
    /Cs8 [/Separation /Black /DeviceCMYK 6 0 R] \
    /Cs9 [/Separation /PANTONE#20Red /DeviceRGB 7 0 R] \
    /Cs10 [/Separation /Spot /DeviceGray 8 0 R] \
    /Cs11 [/DeviceN [/Cyan /Magenta /Yellow /Black] /DeviceCMYK 6 0 R] \
    /Cs12 [/Pattern] \
    /Cs13 [/Indexed /DeviceRGB 1 <FF000000FF00>] >>";

fn named_space_objects() -> Vec<(String, Vec<u8>)> {
    vec![
        (
            "<< /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 0 0 1] /N 1 >>".into(),
            Vec::new(),
        ),
        (
            "<< /FunctionType 2 /Domain [0 1] /C0 [1 1 1] /C1 [1 0 0] /N 1 >>".into(),
            Vec::new(),
        ),
        (
            "<< /FunctionType 4 /Domain [0 1] /Range [0 1] >>".into(),
            b"{ 1 exch sub }".to_vec(),
        ),
    ]
}

#[test]
fn fill_color_resolves_named_separation_and_devicen_spaces() {
    let content = b"BT /F1 12 Tf /Cs8 cs 1 scn 72 700 Td (SEPBLACK) Tj ET\n\
                    BT /F1 12 Tf /Cs9 cs 0.5 scn 72 670 Td (SPOTHALF) Tj ET\n\
                    BT /F1 12 Tf /Cs10 cs 0.25 sc 72 640 Td (TYPEFOUR) Tj ET\n\
                    BT /F1 12 Tf /Cs11 cs 0 0 0 1 scn 72 610 Td (DEVICEN) Tj ET\n\
                    BT /F1 12 Tf 0 0 0 1 k 72 580 Td (PROCESSK) Tj ET\n\
                    BT /F1 12 Tf /Cs12 cs /P0 scn 72 550 Td (PATTERNED) Tj ET\n\
                    BT /F1 12 Tf /Nope cs 1 scn 72 520 Td (UNRESOLVED) Tj ET\n\
                    BT /F1 12 Tf 0.5 g 72 490 Td (HALFGRAY) Tj ET\n\
                    BT /F1 12 Tf /Cs9 cs 72 460 Td (INITIALTINT) Tj ET\n\
                    BT /F1 12 Tf /Cs13 cs 1 sc 72 430 Td (INDEXED) Tj ET";
    let bytes = pdf("", NAMED_SPACES, content, &named_space_objects());
    let plain = spans(&PdfDocument::from_bytes(bytes.clone()).unwrap());
    let s = spans(&open_with_facts(bytes));
    // The span colour itself is unchanged (the count-based guess: white).
    assert!(is_rgb(Some(span_of(&s, "SEPBLACK").color), (1.0, 1.0, 1.0)));
    assert_eq!(span_of(&plain, "SEPBLACK").color, span_of(&s, "SEPBLACK").color);
    assert!(is_rgb(facts_of(&s, "SEPBLACK").fill_color, (0.0, 0.0, 0.0)));
    assert!(is_rgb(facts_of(&s, "SPOTHALF").fill_color, (1.0, 0.5, 0.5)));
    assert!(is_rgb(facts_of(&s, "TYPEFOUR").fill_color, (0.75, 0.75, 0.75)));
    let k = span_of(&s, "PROCESSK");
    assert_eq!(facts_of(&s, "PROCESSK").fill_color, Some(k.color));
    assert_eq!(facts_of(&s, "DEVICEN").fill_color, Some(k.color));
    assert_eq!(facts_of(&s, "PATTERNED").fill_color, None);
    assert_eq!(facts_of(&s, "UNRESOLVED").fill_color, None);
    assert!(is_rgb(facts_of(&s, "HALFGRAY").fill_color, (0.5, 0.5, 0.5)));
    // §8.6.8: a Separation space starts at tint 1.0.
    assert!(is_rgb(facts_of(&s, "INITIALTINT").fill_color, (1.0, 0.0, 0.0)));
    assert!(is_rgb(facts_of(&s, "INDEXED").fill_color, (0.0, 1.0, 0.0)));
}

#[test]
fn fill_color_follows_q_and_resolves_path_fills() {
    let content = b"0.5 g\n\
                    q /Cs8 cs 1 scn BT /F1 12 Tf 72 700 Td (INSIDEQ) Tj ET Q\n\
                    BT /F1 12 Tf 72 670 Td (AFTERQ) Tj ET\n\
                    q /Cs9 cs 1 scn 60 290 300 30 re f Q\n\
                    q /Cs12 cs /P0 scn 60 200 300 30 re f Q\n\
                    60 100 300 30 re f";
    let bytes = pdf("", NAMED_SPACES, content, &named_space_objects());
    let doc = open_with_facts(bytes);
    let s = spans(&doc);
    assert!(is_rgb(facts_of(&s, "INSIDEQ").fill_color, (0.0, 0.0, 0.0)));
    assert!(is_rgb(facts_of(&s, "AFTERQ").fill_color, (0.5, 0.5, 0.5)));
    let pv = doc.page_visibility(0).expect("page facts");
    let rects: Vec<_> = pv
        .paints
        .iter()
        .filter(|p| p.kind == PaintKind::Rect)
        .collect();
    assert_eq!(rects.len(), 3);
    assert!(is_rgb(rects[0].color, (1.0, 0.0, 0.0)));
    assert_eq!(rects[1].color, None, "named Pattern space");
    assert!(is_rgb(rects[2].color, (0.5, 0.5, 0.5)));
}

#[test]
fn form_resolves_a_redefined_colour_space_name_against_its_own_resources() {
    let content = b"BT /F1 12 Tf /Cs8 cs 1 scn 72 700 Td (PAGEBEFORE) Tj ET\n\
                    /X0 Do\n\
                    BT /F1 12 Tf /Cs8 cs 1 scn 72 600 Td (PAGEAFTER) Tj ET";
    let res = "/ColorSpace << /Cs8 [/Separation /Black /DeviceCMYK 6 0 R] >> \
               /XObject << /X0 8 0 R >>";
    let mut extra = named_space_objects();
    extra.truncate(2);
    extra.push((
        "<< /Type /XObject /Subtype /Form /BBox [0 0 612 792] \
         /Resources << /Font << /F1 5 0 R >> \
         /ColorSpace << /Cs8 [/Separation /Spot /DeviceRGB 7 0 R] >> >> >>"
            .to_string(),
        b"BT /F1 12 Tf /Cs8 cs 1 scn 72 650 Td (INFORM) Tj ET".to_vec(),
    ));
    let s = spans(&open_with_facts(pdf("", res, content, &extra)));
    assert!(is_rgb(facts_of(&s, "PAGEBEFORE").fill_color, (0.0, 0.0, 0.0)));
    assert!(is_rgb(facts_of(&s, "INFORM").fill_color, (1.0, 0.0, 0.0)));
    assert!(is_rgb(facts_of(&s, "PAGEAFTER").fill_color, (0.0, 0.0, 0.0)));
}
