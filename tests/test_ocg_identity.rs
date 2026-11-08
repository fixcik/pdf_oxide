//! Optional content groups are decided by object identity, per draw.
//!
//! A group is an indirect object (ISO 32000-1:2008 §8.11.2.1); its `/Name`
//! is user-interface text that two groups may share and one may lack. These
//! tests pin the default configuration (`/OCProperties /D`, §8.11.4.3) as
//! [`LayerFilter::document_default`] reads it, against constructs that a
//! name-keyed filter got wrong:
//!
//! - two groups sharing a `/Name`, one on and one off (both base states);
//! - a group without `/Name`;
//! - a group used by content but missing from `/OCGs` (shown, as by viewers);
//! - an inline (direct) group dictionary, which has no identity;
//! - an OCMD whose `/VE` nests past the old 16-level bound, or never ends;
//! - a Form XObject drawn inside a hidden layer and again visibly;
//! - a >256 KB content stream whose text state is set outside `BT`.

use std::collections::HashSet;

use pdf_oxide::document::PdfDocument;
use pdf_oxide::optional_content::LayerFilter;
use pdf_oxide::structured::ColumnMode;

/// A one-page PDF: object 1 catalog (`catalog_extra` appended), 2 pages,
/// 3 page (`/F1` Helvetica plus `res_extra` in its resources), 4 the content
/// stream, 5 the font, and `objs` as objects 6, 7, … in order.
fn pdf(catalog_extra: &str, res_extra: &str, content: &[u8], objs: &[&str]) -> Vec<u8> {
    let mut buf: Vec<u8> = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    let mut obj = |buf: &mut Vec<u8>, body: &[u8]| {
        offsets.push(buf.len());
        buf.extend_from_slice(format!("{} 0 obj\n", offsets.len()).as_bytes());
        buf.extend_from_slice(body);
        buf.extend_from_slice(b"\nendobj\n");
    };
    obj(
        &mut buf,
        format!("<< /Type /Catalog /Pages 2 0 R {catalog_extra} >>").as_bytes(),
    );
    obj(&mut buf, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    let page = format!(
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R >> {res_extra} >> >>"
    );
    obj(&mut buf, page.as_bytes());
    let mut stream = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
    stream.extend_from_slice(content);
    stream.extend_from_slice(b"\nendstream");
    obj(&mut buf, &stream);
    obj(&mut buf, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    for o in objs {
        obj(&mut buf, o.as_bytes());
    }
    let xref = buf.len();
    let n = offsets.len() + 1;
    buf.extend_from_slice(format!("xref\n0 {n}\n0000000000 65535 f \n").as_bytes());
    for off in &offsets {
        buf.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    buf.extend_from_slice(
        format!("trailer\n<< /Size {n} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    buf
}

fn open(bytes: Vec<u8>) -> PdfDocument {
    PdfDocument::from_bytes(bytes).expect("open synthetic PDF")
}

/// Text of the page as the document's default configuration displays it.
fn shown(doc: &PdfDocument) -> String {
    let filter = LayerFilter::document_default(doc);
    assert!(!filter.is_empty(), "the fixture must hide a layer by default");
    let page = doc
        .extract_structured_with_layer_filter(0, ColumnMode::Auto, &filter)
        .expect("filtered extraction");
    page.regions
        .iter()
        .map(|r| r.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Two marked-content runs: `PAYLOAD` in `/a`, `HONEST` in `/b`.
const TWO_RUNS: &[u8] = b"/OC /a BDC BT /F1 12 Tf 72 700 Td (PAYLOAD) Tj ET EMC \
    /OC /b BDC BT /F1 12 Tf 72 650 Td (HONEST) Tj ET EMC";
const AB: &str = "/Properties << /a 6 0 R /b 7 0 R >>";

#[test]
fn a_visible_group_sharing_the_name_of_a_hidden_one_stays_visible() {
    let doc = open(pdf(
        "/OCProperties << /OCGs [6 0 R 7 0 R] /D << /OFF [6 0 R] >> >>",
        AB,
        TWO_RUNS,
        &["<< /Type /OCG /Name (L) >>", "<< /Type /OCG /Name (L) >>"],
    ));
    let text = shown(&doc);
    assert!(text.contains("HONEST"), "{text}");
    assert!(!text.contains("PAYLOAD"), "{text}");
}

#[test]
fn base_state_off_hides_a_group_named_like_an_on_group() {
    let doc = open(pdf(
        "/OCProperties << /OCGs [6 0 R 7 0 R] /D << /BaseState /OFF /ON [7 0 R] >> >>",
        AB,
        TWO_RUNS,
        &["<< /Type /OCG /Name (L) >>", "<< /Type /OCG /Name (L) >>"],
    ));
    let text = shown(&doc);
    assert!(!text.contains("PAYLOAD"), "{text}");
    assert!(text.contains("HONEST"), "{text}");
}

#[test]
fn a_group_without_a_name_is_hidden_when_off() {
    let doc = open(pdf(
        "/OCProperties << /OCGs [6 0 R 7 0 R] /D << /OFF [6 0 R] >> >>",
        AB,
        TWO_RUNS,
        &["<< /Type /OCG >>", "<< /Type /OCG /Name (Other) >>"],
    ));
    let text = shown(&doc);
    assert!(!text.contains("PAYLOAD"), "{text}");
    assert!(text.contains("HONEST"), "{text}");
}

#[test]
fn base_state_off_keeps_a_group_missing_from_ocgs_as_viewers_do() {
    // Object 6 is used by content but not listed in /OCGs; 8 is listed and
    // off. poppler and pdf.js display such a group: it is not hidden text.
    let doc = open(pdf(
        "/OCProperties << /OCGs [7 0 R 8 0 R] /D << /BaseState /OFF /ON [7 0 R] >> >>",
        AB,
        TWO_RUNS,
        &[
            "<< /Type /OCG /Name (A) >>",
            "<< /Type /OCG /Name (B) >>",
            "<< /Type /OCG /Name (C) >>",
        ],
    ));
    let text = shown(&doc);
    assert!(text.contains("PAYLOAD"), "{text}");
    assert!(text.contains("HONEST"), "{text}");
}

#[test]
fn base_state_off_with_every_group_on_hides_nothing() {
    let doc = open(pdf(
        "/OCProperties << /OCGs [6 0 R 7 0 R] /D << /BaseState /OFF /ON [6 0 R 7 0 R] >> >>",
        AB,
        TWO_RUNS,
        &["<< /Type /OCG /Name (A) >>", "<< /Type /OCG /Name (B) >>"],
    ));
    let text = shown(&doc);
    assert!(text.contains("PAYLOAD") && text.contains("HONEST"), "{text}");
}

#[test]
fn base_state_off_applies_off_after_on_as_viewers_do() {
    // Group 6 is in /ON and /OFF: poppler and pdf.js apply /OFF last.
    let doc = open(pdf(
        "/OCProperties << /OCGs [6 0 R 7 0 R] \
         /D << /BaseState /OFF /ON [6 0 R 7 0 R] /OFF [6 0 R] >> >>",
        AB,
        TWO_RUNS,
        &["<< /Type /OCG /Name (A) >>", "<< /Type /OCG /Name (B) >>"],
    ));
    let text = shown(&doc);
    assert!(!text.contains("PAYLOAD"), "{text}");
    assert!(text.contains("HONEST"), "{text}");
}

/// No group is off; `/a` is the OCMD `ocmd` over the on group 7.
fn ocmd_over_on_groups(ocmd: &str) -> PdfDocument {
    open(pdf(
        "/OCProperties << /OCGs [7 0 R] /D << /Order [7 0 R] >> >>",
        "/Properties << /a 6 0 R /b 7 0 R >>",
        TWO_RUNS,
        &[ocmd, "<< /Type /OCG /Name (On) >>"],
    ))
}

#[test]
fn an_ocmd_over_groups_that_are_on_hides_its_content_without_any_off_group() {
    for ocmd in [
        "<< /Type /OCMD /OCGs [7 0 R] /VE [/Not 7 0 R] >>",
        "<< /Type /OCMD /OCGs [7 0 R] /P /AnyOff >>",
    ] {
        let text = shown(&ocmd_over_on_groups(ocmd));
        assert!(!text.contains("PAYLOAD"), "{ocmd}: {text}");
        assert!(text.contains("HONEST"), "{ocmd}: {text}");
    }
}

#[test]
fn an_inline_group_dict_is_not_the_hidden_group_of_the_same_name() {
    let content = b"/OC /a BDC BT /F1 12 Tf 72 700 Td (PAYLOAD) Tj ET EMC \
        /OC << /Type /OCG /Name (L) >> BDC BT /F1 12 Tf 72 650 Td (HONEST) Tj ET EMC";
    let doc = open(pdf(
        "/OCProperties << /OCGs [6 0 R] /D << /OFF [6 0 R] >> >>",
        "/Properties << /a 6 0 R >>",
        content,
        &["<< /Type /OCG /Name (L) >>"],
    ));
    let text = shown(&doc);
    assert!(!text.contains("PAYLOAD"), "{text}");
    assert!(text.contains("HONEST"), "{text}");
}

#[test]
fn ocmd_members_are_matched_by_reference() {
    // /b is an OCMD AllOn over group 7, which is on but named like off 6.
    let doc = open(pdf(
        "/OCProperties << /OCGs [6 0 R 7 0 R] /D << /OFF [6 0 R] >> >>",
        "/Properties << /a 6 0 R /b 8 0 R >>",
        TWO_RUNS,
        &[
            "<< /Type /OCG /Name (L) >>",
            "<< /Type /OCG /Name (L) >>",
            "<< /Type /OCMD /OCGs [7 0 R] /P /AllOn >>",
        ],
    ));
    let text = shown(&doc);
    assert!(text.contains("HONEST"), "{text}");
    assert!(!text.contains("PAYLOAD"), "{text}");
}

#[test]
fn layer_names_given_by_the_caller_still_match_by_name() {
    let doc = open(pdf(
        "/OCProperties << /OCGs [6 0 R 7 0 R] >>",
        AB,
        TWO_RUNS,
        &["<< /Type /OCG /Name (L) >>", "<< /Type /OCG /Name (L) >>"],
    ));
    let names: HashSet<String> = ["L".to_string()].into();
    let page = doc
        .extract_structured_filtered(0, names, HashSet::new())
        .unwrap();
    let text: String = page.regions.iter().map(|r| r.text.as_str()).collect();
    assert!(!text.contains("PAYLOAD") && !text.contains("HONEST"), "{text}");
}

/// `[/op [/op … leaf]]` nested `depth` times.
fn nested(op: &str, depth: usize, leaf: &str) -> String {
    let mut ve = leaf.to_string();
    for _ in 0..depth {
        ve = format!("[/{op} {ve}]");
    }
    ve
}

/// `/b` is an OCMD with visibility expression `ve`; group 6 is off, 7 on.
fn ve_doc(ve: &str, extra: &[&str]) -> PdfDocument {
    let ocmd = format!("<< /Type /OCMD /OCGs [6 0 R] /VE {ve} >>");
    let mut objs = vec![
        "<< /Type /OCG /Name (Off) >>",
        "<< /Type /OCG /Name (On) >>",
    ];
    objs.push(&ocmd);
    objs.extend_from_slice(extra);
    open(pdf(
        "/OCProperties << /OCGs [6 0 R 7 0 R] /D << /OFF [6 0 R] >> >>",
        "/Properties << /a 6 0 R /b 8 0 R >>",
        TWO_RUNS,
        &objs,
    ))
}

#[test]
fn a_visibility_expression_nested_past_sixteen_levels_is_evaluated() {
    // 20 levels of /And over the off group: hidden (it was "visible" past 16).
    let text = shown(&ve_doc(&nested("And", 20, "6 0 R"), &[]));
    assert!(!text.contains("HONEST"), "{text}");
    // /Not of the same: visible, so the depth is evaluated, not just refused.
    let ve = format!("[/Not {}]", nested("And", 20, "6 0 R"));
    let text = shown(&ve_doc(&ve, &[]));
    assert!(text.contains("HONEST"), "{text}");
}

#[test]
fn an_unbounded_visibility_expression_hides_its_content_even_under_not() {
    // 70 levels: past the bound, so malformed — not "visible" (fail-open)…
    let text = shown(&ve_doc(&nested("Or", 70, "6 0 R"), &[]));
    assert!(!text.contains("HONEST"), "{text}");
    // …and /Not must not turn a malformed operand into "visible" either.
    let ve = format!("[/Not {}]", nested("Or", 70, "7 0 R"));
    let text = shown(&ve_doc(&ve, &[]));
    assert!(!text.contains("HONEST"), "{text}");
    // A reference cycle: object 9 is `[/Not 9 0 R]`.
    let text = shown(&ve_doc("9 0 R", &["[/Not 9 0 R]"]));
    assert!(!text.contains("HONEST"), "{text}");
    assert!(!text.contains("PAYLOAD"), "{text}");
}

/// Group 6 off; form 7 draws `ACME`; `content` invokes it.
fn form_doc(content: &[u8]) -> PdfDocument {
    let body = "BT /F1 12 Tf 72 600 Td (ACME) Tj ET";
    let form = format!(
        "<< /Type /XObject /Subtype /Form /BBox [0 0 612 792] \
         /Resources << /Font << /F1 5 0 R >> >> /Length {} >>\nstream\n{body}\nendstream",
        body.len()
    );
    open(pdf(
        "/OCProperties << /OCGs [6 0 R] /D << /OFF [6 0 R] >> >>",
        "/Properties << /a 6 0 R >> /XObject << /X1 7 0 R >>",
        content,
        &["<< /Type /OCG /Name (L) >>", &form],
    ))
}

fn count_acme(doc: &PdfDocument) -> usize {
    shown(doc).matches("ACME").count()
}

#[test]
fn a_form_drawn_hidden_then_visible_at_the_same_ctm_is_shown() {
    let doc = form_doc(b"/OC /a BDC /X1 Do EMC /X1 Do");
    assert_eq!(count_acme(&doc), 1);
}

#[test]
fn a_form_drawn_visible_then_hidden_is_shown_once() {
    let doc = form_doc(b"/X1 Do /OC /a BDC /X1 Do EMC");
    assert_eq!(count_acme(&doc), 1);
}

/// `16 TL` and `2 Tc` set outside `BT`, a hidden run, then two lines written
/// with `T*`, padded past the 256 KB prescan threshold.
fn large_stream_doc() -> PdfDocument {
    let mut content = b"16 TL 2 Tc\n/OC /a BDC BT /F1 12 Tf 72 720 Td (SECRET) Tj ET EMC\n\
        BT /F1 12 Tf 72 700 Td (Apples) Tj T* (Pears) Tj ET\n"
        .to_vec();
    while content.len() <= 300 * 1024 {
        content.extend_from_slice(b"q 1 0 0 1 0 0 cm Q\n");
    }
    open(pdf(
        "/OCProperties << /OCGs [6 0 R] /D << /OFF [6 0 R] >> >>",
        "/Properties << /a 6 0 R >>",
        &content,
        &["<< /Type /OCG /Name (L) >>"],
    ))
}

#[test]
fn filtered_and_unfiltered_extraction_agree_on_a_large_stream() {
    use pdf_oxide::document::ReadingOrder;
    let doc = large_stream_doc();
    let filter = LayerFilter::document_default(&doc);
    let key = |s: &pdf_oxide::layout::TextSpan| {
        (s.text.clone(), s.bbox.x.to_bits(), s.bbox.y.to_bits(), s.bbox.width.to_bits())
    };
    let full: Vec<_> = doc.extract_spans(0).unwrap().iter().map(key).collect();
    let shown: Vec<_> = doc
        .extract_spans_with_layer_filter(0, ReadingOrder::default(), &filter)
        .unwrap()
        .iter()
        .map(key)
        .collect();
    assert!(full.iter().any(|k| k.0.contains("SECRET")));
    let visible: Vec<_> = full.iter().filter(|k| !k.0.contains("SECRET")).collect();
    assert!(!shown.iter().any(|k| k.0.contains("SECRET")), "{shown:?}");
    for k in &shown {
        assert!(visible.contains(&k), "{k:?} missing from the unfiltered page {visible:?}");
    }
    // `T*` used the leading set outside BT: Pears is 16 pt below Apples.
    let y = |t: &str| {
        let s = doc.extract_spans(0).unwrap();
        s.iter().find(|s| s.text.contains(t)).unwrap().bbox.y
    };
    assert!((y("Apples") - y("Pears") - 16.0).abs() < 0.5);
    // Words come from the same parser.
    let words = doc.extract_words(0).unwrap();
    let pears = words
        .iter()
        .find(|w| w.text == "Pears")
        .expect("Pears word");
    assert!((pears.bbox.y - y("Pears")).abs() < 0.5);
}

#[test]
fn a_hidden_run_inside_a_visible_text_object_is_cut_from_the_filtered_page() {
    use pdf_oxide::document::ReadingOrder;
    let doc = open(pdf(
        "/OCProperties << /OCGs [6 0 R] /D << /OFF [6 0 R] >> >>",
        "/Properties << /a 6 0 R >>",
        b"BT /F1 12 Tf 72 700 Td (Total ) Tj /OC /a BDC (PWNED) Tj EMC ET",
        &["<< /Type /OCG /Name (L) >>"],
    ));
    let filter = LayerFilter::document_default(&doc);
    let shown: Vec<String> = doc
        .extract_spans_with_layer_filter(0, ReadingOrder::default(), &filter)
        .unwrap()
        .into_iter()
        .map(|s| s.text)
        .collect();
    // The unfiltered page assembles one span "Total PWNED" here: a span
    // missing from the filtered page is not necessarily all hidden text.
    let full: Vec<String> = doc
        .extract_spans(0)
        .unwrap()
        .into_iter()
        .map(|s| s.text)
        .collect();
    assert!(full.iter().any(|t| t.contains("PWNED")), "{full:?}");
    assert!(shown.iter().all(|t| !t.contains("PWNED")), "{shown:?}");
    assert!(shown.iter().any(|t| t.contains("Total")), "{shown:?}");
}

#[test]
fn a_malformed_operand_decides_an_or_with_a_visible_sibling() {
    // [/Or <70 levels> on-group]: the well-formed sibling is visible, but a
    // malformed operand anywhere hides the scope.
    let ve = format!("[/Or {} 7 0 R]", nested("Or", 70, "7 0 R"));
    let text = shown(&ve_doc(&ve, &[]));
    assert!(!text.contains("HONEST"), "{text}");
}

#[test]
fn a_shared_reference_dag_in_a_visibility_expression_is_bounded() {
    // Object 9 + k is `[/And 10+k 0 R 10+k 0 R]` for k < 64, ending at the on
    // group 7: a tree walk would visit 2^64 nodes.
    let levels: Vec<String> = (0..64)
        .map(|k| {
            let next = if k == 63 {
                "7 0 R".to_string()
            } else {
                format!("{} 0 R", 10 + k)
            };
            format!("[/And {next} {next}]")
        })
        .collect();
    let refs: Vec<&str> = levels.iter().map(String::as_str).collect();
    let start = std::time::Instant::now();
    let text = shown(&ve_doc("9 0 R", &refs));
    let took = start.elapsed();
    assert!(took < std::time::Duration::from_secs(5), "took {took:?}");
    // Past the node budget the expression is malformed: hidden.
    assert!(!text.contains("HONEST"), "{text}");
    assert!(!text.contains("PAYLOAD"), "{text}");
}
