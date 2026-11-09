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

/// `pages` pages with a running header, a "Page N" footer (the number
/// varies) and a body line.
fn running_heads_pdf(pages: usize) -> Vec<u8> {
    let first_page = 4;
    let kids: Vec<String> = (0..pages)
        .map(|i| format!("{} 0 R", first_page + 2 * i))
        .collect();
    let mut objs = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!("<< /Type /Pages /Kids [{}] /Count {pages} >>", kids.join(" ")),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_string(),
    ];
    for i in 0..pages {
        objs.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {} 0 R \
             /Resources << /Font << /F1 3 0 R >> >> >>",
            first_page + 2 * i + 1
        ));
        objs.push(stream(
            "",
            &format!(
                "BT /F1 9 Tf 72 760 Td (Annual report 2026) Tj ET \
                 /Span << /MCID 0 /ActualText (Body {i}) >> BDC \
                 BT /F1 12 Tf 72 400 Td (Body text of page {i}, unique words {i}) Tj ET EMC \
                 BT /F1 9 Tf 290 30 Td (Page {}) Tj ET",
                i + 1
            ),
        ));
    }
    build(&objs)
}

/// The signature map in a comparable form.
fn signatures_of(doc: &PdfDocument) -> Vec<(String, usize, Vec<(usize, [u32; 4])>)> {
    let map = doc.ensure_running_artifact_signatures().unwrap();
    let mut out: Vec<_> = map
        .iter()
        .map(|(sig, s)| {
            let occ = s
                .occurrences
                .iter()
                .map(|(pi, r)| {
                    let b = [r.x, r.y, r.width, r.height].map(f32::to_bits);
                    (*pi, b)
                })
                .collect();
            (sig.clone(), s.first_seen_page, occ)
        })
        .collect();
    out.sort();
    out
}

#[cfg(feature = "parallel")]
#[test]
fn scan_on_current_pool_equals_the_sequential_scan() {
    let bytes = running_heads_pdf(41);
    let pool = |n| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .unwrap()
    };
    // Lazy scan on a worker: sequential.
    let seq = PdfDocument::from_bytes(bytes.clone()).unwrap();
    let want = pool(1).install(|| signatures_of(&seq));
    assert!(
        want.iter().any(|(sig, ..)| sig.contains('#')),
        "the page-number footer is a signature: {want:?}"
    );
    for threads in [1, 2, 4, 64] {
        let par = PdfDocument::from_bytes(bytes.clone()).unwrap();
        pool(4).install(|| {
            par.scan_running_artifacts_on_current_pool(threads).unwrap();
            // Computed once: the lazy path now returns the stored map.
            assert_eq!(signatures_of(&par), want, "threads {threads}");
        });
        let a: Vec<_> = seq.extract_spans(7).unwrap();
        let b: Vec<_> = par.extract_spans(7).unwrap();
        let marks = |v: &[TextSpan]| -> Vec<(String, bool)> {
            v.iter()
                .map(|s| (s.text.clone(), s.artifact_type.is_some()))
                .collect()
        };
        assert_eq!(marks(&b), marks(&a));
        assert!(marks(&a).iter().any(|(_, m)| *m), "{:?}", marks(&a));
    }
}

/// Everything an `extract_spans_raw` call left behind for `page`: its spans
/// (all fields), the page's ActualText MCIDs and unfiltered paints.
fn raw_extraction_of(doc: &PdfDocument, page: usize) -> (String, String, String) {
    let spans = format!("{:?}", doc.extract_spans_raw(page).unwrap());
    let mut mcids: Vec<u32> = doc
        .mc_actualtext_mcids
        .lock_or_recover()
        .get(&page)
        .map(|s| s.iter().copied().collect())
        .unwrap_or_default();
    mcids.sort_unstable();
    let paints = format!("{:?}", doc.raw_paints.lock_or_recover().get(&(page, false)));
    (spans, format!("{mcids:?}"), paints)
}

#[test]
fn seeded_raw_spans_equal_an_extraction() {
    for facts in [false, true] {
        let bytes = running_heads_pdf(6);
        let fresh = PdfDocument::from_bytes(bytes.clone()).unwrap();
        fresh.set_visibility_facts(facts);
        let scanned = PdfDocument::from_bytes(bytes).unwrap();
        scanned.set_visibility_facts(facts);
        scanned.ensure_running_artifact_signatures().unwrap();
        assert_eq!(scanned.raw_spans_seeds.lock_or_recover().len(), 6);
        for page in 0..6 {
            if page % 2 == 1 {
                // A filtered extraction first (the hidden-layer pass does
                // that) replaces the page's MCID set; the raw one replaces
                // it again, seeded or not.
                for doc in [&fresh, &scanned] {
                    let inks: HashSet<String> = ["Spot".to_string()].into();
                    doc.extract_spans_raw_filtered(page, Default::default(), inks)
                        .unwrap();
                }
            }
            assert_ne!(raw_extraction_of(&fresh, page).1, "[]", "ActualText MCIDs");
            assert_eq!(
                raw_extraction_of(&scanned, page),
                raw_extraction_of(&fresh, page),
                "page {page}, facts {facts}"
            );
        }
        assert_eq!(scanned.raw_spans_seeds.lock_or_recover().len(), 0, "taken once");
    }
}

#[test]
fn no_seed_where_the_scan_bypassed_the_form_span_cache() {
    let doc = PdfDocument::from_bytes(header_form_pdf(4)).unwrap();
    doc.ensure_running_artifact_signatures().unwrap();
    assert_eq!(doc.raw_spans_seeds.lock_or_recover().len(), 0);
    // With visibility facts the cache is off for every extraction: seeded.
    let facts = PdfDocument::from_bytes(header_form_pdf(4)).unwrap();
    facts.set_visibility_facts(true);
    facts.ensure_running_artifact_signatures().unwrap();
    assert_eq!(facts.raw_spans_seeds.lock_or_recover().len(), 4);
    facts.forget_page(1);
    assert_eq!(facts.raw_spans_seeds.lock_or_recover().len(), 3);
    facts.set_visibility_facts(true);
    assert_eq!(facts.raw_spans_seeds.lock_or_recover().len(), 0);
}

#[test]
fn only_a_document_that_fits_the_page_caches_is_seeded() {
    let fits = PdfDocument::from_bytes(running_heads_pdf(page_cache_entries())).unwrap();
    fits.ensure_running_artifact_signatures().unwrap();
    assert_eq!(fits.raw_spans_seeds.lock_or_recover().len(), page_cache_entries());
    let longer = PdfDocument::from_bytes(running_heads_pdf(page_cache_entries() + 1)).unwrap();
    longer.ensure_running_artifact_signatures().unwrap();
    assert_eq!(longer.raw_spans_seeds.lock_or_recover().len(), 0);
}
