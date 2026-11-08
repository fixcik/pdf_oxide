//! `render_page` applies the document's default optional-content state by
//! group reference: a visible group that shares the `/Name` of a hidden one
//! is painted, an unnamed hidden group is not.
#![cfg(feature = "rendering")]

use pdf_oxide::document::PdfDocument;
use pdf_oxide::rendering::{ImageFormat, RenderOptions};

/// Red square in `/a` (group 6) at x 50, green square in `/b` (group 7) at
/// x 300; both 100 pt, top edge 100 pt below the page top.
fn pdf(oc_props: &str, g6: &str, g7: &str) -> Vec<u8> {
    let content = b"/OC /a BDC 1 0 0 rg 50 592 100 100 re f EMC \
                    /OC /b BDC 0 1 0 rg 300 592 100 100 re f EMC";
    let objs: Vec<Vec<u8>> = vec![
        format!("<< /Type /Catalog /Pages 2 0 R /OCProperties {oc_props} >>").into_bytes(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
           /Resources << /Properties << /a 5 0 R /b 6 0 R >> >> >>"
            .to_vec(),
        [
            format!("<< /Length {} >>\nstream\n", content.len()).as_bytes(),
            content,
            b"\nendstream",
        ]
        .concat(),
        g6.as_bytes().to_vec(),
        g7.as_bytes().to_vec(),
    ];
    let mut buf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objs.iter().enumerate() {
        offsets.push(buf.len());
        buf.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        buf.extend_from_slice(body);
        buf.extend_from_slice(b"\nendobj\n");
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

/// Whether the 100x100 square whose left edge is at `x` is painted (its
/// centre pixel is not white).
fn painted(bytes: Vec<u8>, x: u32) -> bool {
    let doc = PdfDocument::from_bytes(bytes).unwrap();
    let mut options = RenderOptions::with_dpi(72);
    options.format = ImageFormat::RawRgba8;
    let img = pdf_oxide::rendering::render_page(&doc, 0, &options).unwrap();
    let idx = ((150 * img.width + x + 50) * 4) as usize;
    let px = &img.data[idx..idx + 4];
    px[3] > 0 && !(px[0] > 240 && px[1] > 240 && px[2] > 240)
}

#[test]
fn a_visible_group_sharing_the_name_of_a_hidden_one_is_painted() {
    let make = || {
        pdf(
            "<< /OCGs [5 0 R 6 0 R] /D << /OFF [5 0 R] >> >>",
            "<< /Type /OCG /Name (L) >>",
            "<< /Type /OCG /Name (L) >>",
        )
    };
    assert!(!painted(make(), 50), "group 5 is off");
    assert!(painted(make(), 300), "group 6 is on, whatever its name");
}

#[test]
fn an_unnamed_group_that_is_off_is_not_painted() {
    let make = || {
        pdf(
            "<< /OCGs [5 0 R 6 0 R] /D << /BaseState /OFF /ON [6 0 R] >> >>",
            "<< /Type /OCG >>",
            "<< /Type /OCG /Name (On) >>",
        )
    };
    assert!(!painted(make(), 50));
    assert!(painted(make(), 300));
}
