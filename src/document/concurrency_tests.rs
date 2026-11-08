//! Shared document state touched by several threads at once: the byte
//! reader and the Form XObject span cache.

use super::*;

/// A PDF of the given object bodies (object `i + 1` = `objs[i]`, catalog
/// first), with a correct xref table.
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

/// `pages` pages, each painting the shared header form (object 4, own
/// /Resources) and a body line.
fn header_form_pdf(pages: usize) -> Vec<u8> {
    let first_page = 5;
    let kids: Vec<String> = (0..pages)
        .map(|i| format!("{} 0 R", first_page + 2 * i))
        .collect();
    let mut objs = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!("<< /Type /Pages /Kids [{}] /Count {pages} >>", kids.join(" ")),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_string(),
        stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >>",
            "BT /F1 9 Tf 72 760 Td (Journal of Things) Tj ET",
        ),
    ];
    for i in 0..pages {
        objs.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {} 0 R \
             /Resources << /Font << /F1 3 0 R >> /XObject << /Hd 4 0 R >> >> >>",
            first_page + 2 * i + 1
        ));
        objs.push(stream(
            "",
            &format!("q /Hd Do Q BT /F1 12 Tf 72 400 Td (Body text of page {i}) Tj ET"),
        ));
    }
    build(&objs)
}

#[test]
fn header_footer_scan_stays_out_of_the_form_span_cache() {
    let doc = PdfDocument::from_bytes(header_form_pdf(4)).unwrap();
    doc.ensure_running_artifact_signatures().unwrap();
    assert_eq!(
        doc.xobject_spans_cache.lock_or_recover().len(),
        0,
        "the scan filled the form span cache: its page order would decide what callers replay"
    );
    // Control: the caller's own extraction does fill it.
    doc.extract_spans_raw_uncached(2).unwrap();
    assert_eq!(doc.xobject_spans_cache.lock_or_recover().len(), 1);
}

#[test]
fn scan_does_not_change_the_spans_a_caller_gets() {
    let fresh = PdfDocument::from_bytes(header_form_pdf(4)).unwrap();
    let want: Vec<(String, usize)> = fresh
        .extract_spans_raw_uncached(3)
        .unwrap()
        .into_iter()
        .map(|s| (s.text, s.sequence))
        .collect();
    let scanned = PdfDocument::from_bytes(header_form_pdf(4)).unwrap();
    scanned.ensure_running_artifact_signatures().unwrap();
    let got: Vec<(String, usize)> = scanned
        .extract_spans_raw_uncached(3)
        .unwrap()
        .into_iter()
        .map(|s| (s.text, s.sequence))
        .collect();
    assert_eq!(got, want);
}

/// Probabilistic (thread timing): `is_form_xobject` peeks the shared reader
/// without `load_lock`, so it can move the reader between the header and the
/// body read of a concurrent `load_object`. The body read must position
/// itself. Without that, this test failed in most runs.
#[test]
fn object_body_read_is_positioned_under_concurrent_peeks() {
    const N: usize = 40;
    let body = |i: usize| -> String {
        // Multi-line dictionaries, so the body read spans several lines.
        format!("<< /Tag {i}\n/Pad (xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx)\n/Kind /Probe{i} >>")
    };
    let mut objs = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [] /Count 0 >>".to_string(),
        stream("/Type /XObject /Subtype /Form /BBox [0 0 1 1]", "0 0 m 1 1 l S"),
    ];
    objs.extend((0..N).map(body));
    let bytes = build(&objs);
    for _ in 0..150 {
        let doc = PdfDocument::from_bytes(bytes.clone()).unwrap();
        let stop = AtomicBool::new(false);
        let tags: Vec<Option<i64>> = std::thread::scope(|scope| {
            scope.spawn(|| {
                while !stop.load(Ordering::Relaxed) {
                    doc.is_form_xobject(ObjectRef::new(3, 0));
                }
            });
            let tags = (0..N)
                .map(|i| {
                    let obj = doc.load_object(ObjectRef::new(4 + i as u32, 0)).ok()?;
                    obj.as_dict()?.get("Tag")?.as_integer()
                })
                .collect();
            stop.store(true, Ordering::Relaxed);
            tags
        });
        let want: Vec<Option<i64>> = (0..N as i64).map(Some).collect();
        assert_eq!(tags, want, "objects read from a moved reader");
    }
}
