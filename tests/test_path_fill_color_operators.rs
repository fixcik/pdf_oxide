//! `sc`/`scn` (and `SC`/`SCN`) must set the fill (stroke) colour of extracted
//! paths (ISO 32000-1:2008 §8.6.8).
//!
//! Word-style exports paint white cell backgrounds as `/CS0 cs 1 1 1 scn`
//! with CS0 an ICCBased space. Ignoring the operator left the fill at its
//! default, so a white background looked black. A Pattern or tint-based
//! space has no sound RGB value here and must come out as "no colour"
//! rather than a plausible-but-wrong one.

use pdf_oxide::document::PdfDocument;
use pdf_oxide::elements::PathContent;
use pdf_oxide::layout::Color;

/// One filled (or stroked) 10x10 rectangle per case, at x = 10 * index.
fn fixture_pdf() -> Vec<u8> {
    let content = b"\
/CS0 cs 1 1 1 scn 0 0 10 10 re f\n\
/CS1 cs 0.5 sc 20 0 10 10 re f\n\
/CS2 cs 1 0 0 0 scn 40 0 10 10 re f\n\
/Pattern cs /P0 scn 60 0 10 10 re f\n\
/CS3 cs 1 scn 80 0 10 10 re f\n\
/DeviceRGB cs 0 1 0 sc 100 0 10 10 re f\n\
/CS0 CS 1 0 0 SCN 120 0 10 10 re S\n\
0 0 1 rg q /CS0 cs 1 1 1 scn Q 140 0 10 10 re f\n\
/Pattern cs /P0 scn 0 1 0 rg 160 0 10 10 re f\n\
BT /F1 9 Tf 0 0 Td /CS0 cs 1 1 1 scn (x) Tj ET 180 0 10 10 re f\n\
BT /DeviceRGB cs 1 0 0 sc (x) Tj ET 200 0 10 10 re f\n";
    let objs: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
          /Resources << /ColorSpace << /CS0 [/ICCBased 5 0 R] /CS1 [/ICCBased 6 0 R] \
          /CS2 [/ICCBased 7 0 R] /CS3 [/Separation /Spot /DeviceGray \
          << /FunctionType 2 /Domain [0 1] /N 1 >>] >> \
          /Pattern << /P0 8 0 R >> >> >>"
            .to_vec(),
        [
            format!("<< /Length {} >>\nstream\n", content.len()).into_bytes(),
            content.to_vec(),
            b"\nendstream".to_vec(),
        ]
        .concat(),
        b"<< /N 3 /Length 0 >>\nstream\n\nendstream".to_vec(),
        b"<< /N 1 /Length 0 >>\nstream\n\nendstream".to_vec(),
        b"<< /N 4 /Length 0 >>\nstream\n\nendstream".to_vec(),
        b"<< /PatternType 2 >>".to_vec(),
    ];
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![0usize];
    for (i, body) in objs.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        pdf.extend_from_slice(body);
        pdf.extend_from_slice(b"\nendobj\n");
    }
    let xref_pos = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f\r\n", offsets.len()).as_bytes());
    for &off in &offsets[1..] {
        pdf.extend_from_slice(format!("{:010} 00000 n\r\n", off).as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
            offsets.len(),
            xref_pos
        )
        .as_bytes(),
    );
    pdf
}

fn path_at(paths: &[PathContent], x: f32) -> &PathContent {
    paths
        .iter()
        .find(|p| (p.bbox.x - x).abs() < 0.5)
        .unwrap_or_else(|| panic!("no path at x={x}"))
}

fn rgb(c: Option<Color>) -> Option<(f32, f32, f32)> {
    c.map(|c| (c.r, c.g, c.b))
}

#[test]
fn scn_sc_set_path_fill_colour_by_space_kind() {
    let doc = PdfDocument::from_bytes(fixture_pdf()).expect("parse fixture");
    let paths = doc.extract_paths(0).expect("extract paths");
    assert_eq!(paths.len(), 11);

    // ICCBased N=3 + scn -> white.
    assert_eq!(rgb(path_at(&paths, 0.0).fill_color), Some((1.0, 1.0, 1.0)));
    // ICCBased N=1 + sc -> gray.
    assert_eq!(rgb(path_at(&paths, 20.0).fill_color), Some((0.5, 0.5, 0.5)));
    // ICCBased N=4 + scn -> CMYK conversion (same as `k`).
    let (r, g, b) = pdf_oxide::color::cmyk_to_rgb(1.0, 0.0, 0.0, 0.0);
    assert_eq!(rgb(path_at(&paths, 40.0).fill_color), Some((r, g, b)));
    // Pattern space -> no colour.
    assert_eq!(path_at(&paths, 60.0).fill_color, None);
    // Separation tint is not a gray level -> no colour.
    assert_eq!(path_at(&paths, 80.0).fill_color, None);
    // Device space named directly, no resource entry needed.
    assert_eq!(rgb(path_at(&paths, 100.0).fill_color), Some((0.0, 1.0, 0.0)));
    // Stroke: CS + SCN.
    assert_eq!(rgb(path_at(&paths, 120.0).stroke_color), Some((1.0, 0.0, 0.0)));
    // q ... Q restores the colour set before q.
    assert_eq!(rgb(path_at(&paths, 140.0).fill_color), Some((0.0, 0.0, 1.0)));
    // A later device colour replaces an unknown (pattern) one.
    assert_eq!(rgb(path_at(&paths, 160.0).fill_color), Some((0.0, 1.0, 0.0)));

    // A colour set inside a BT..ET block outlives the block.
    assert_eq!(rgb(path_at(&paths, 180.0).fill_color), Some((1.0, 1.0, 1.0)));
    assert_eq!(rgb(path_at(&paths, 200.0).fill_color), Some((1.0, 0.0, 0.0)));
}
