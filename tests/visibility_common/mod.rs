//! Shared helpers of the `visibility_facts*` integration tests: a minimal
//! one-page PDF builder and span lookups.
#![allow(dead_code)]

use pdf_oxide::document::PdfDocument;
use pdf_oxide::extractors::SpanVisibility;
use pdf_oxide::layout::TextSpan;

/// One-page PDF: 1 catalog (+`catalog_extra`), 2 pages, 3 page (Font F1 plus
/// `resources_extra`), 4 content, 5 Helvetica, 6.. `extra` objects (each a
/// full object body; a `stream` body is given as `(dict, data)`).
pub fn pdf(
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

pub fn spans(doc: &PdfDocument) -> Vec<TextSpan> {
    let page = doc.extract_structured(0).expect("extract_structured");
    page.regions.into_iter().flat_map(|r| r.spans).collect()
}

pub fn facts_of(spans: &[TextSpan], needle: &str) -> SpanVisibility {
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

pub fn open_with_facts(bytes: Vec<u8>) -> PdfDocument {
    let doc = PdfDocument::from_bytes(bytes).unwrap();
    doc.set_visibility_facts(true);
    doc
}

pub fn span_of<'a>(spans: &'a [TextSpan], needle: &str) -> &'a TextSpan {
    spans
        .iter()
        .find(|s| s.text.contains(needle))
        .unwrap_or_else(|| panic!("no span with {needle:?}"))
}
