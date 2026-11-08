//! Shared document state touched by several threads at once.

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
