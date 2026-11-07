//! A run whose text matrix turns it upright against the page `/Rotate`
//! (`0 1 -1 0 Tm` under `/Rotate 90`, `-1 0 0 -1 Tm` under `/Rotate 180`,
//! `0 -1 1 0 Tm` under `/Rotate 270`) reads
//! as horizontal text on the displayed page. Its span box must cover the ink
//! the renderer draws for it in raw user space (`extract_structured`), and
//! its words in the displayed frame (`extract_words`), instead of a box laid
//! along the raw x axis from the run origin. `extract_spans` keeps its mapped
//! box; its `page_bbox()` is the physical one.

#![cfg(feature = "rendering")]

use pdf_oxide::geometry::Rect;
use pdf_oxide::rendering::{render_page, ImageFormat, RenderOptions};
use pdf_oxide::PdfDocument;

const W: f32 = 300.0;
const H: f32 = 400.0;

fn pdf(rotate: i32, tm: &str) -> Vec<u8> {
    let content = format!("BT /F1 24 Tf {tm} Tm (HELLO WORLD) Tj ET");
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {W} {H}] /Rotate {rotate} \
             /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>"
        ),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offs = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offs.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
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

/// Bounding box of the dark pixels in the displayed frame, as a y-up rect in
/// points (the raster is rendered at 72 dpi, so 1 px = 1 pt).
fn ink(doc: &PdfDocument) -> Rect {
    let mut opts = RenderOptions::default();
    opts.dpi = 72;
    opts.format = ImageFormat::RawRgba8;
    let img = render_page(doc, 0, &opts).expect("render");
    let (w, h) = (img.width as usize, img.height as usize);
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            if img.data[i] < 128 && img.data[i + 1] < 128 && img.data[i + 2] < 128 {
                x0 = x0.min(x);
                x1 = x1.max(x + 1);
                y0 = y0.min(y);
                y1 = y1.max(y + 1);
            }
        }
    }
    assert!(x0 < x1, "nothing rendered");
    Rect::new(x0 as f32, (h - y1) as f32, (x1 - x0) as f32, (y1 - y0) as f32)
}

/// Raw user-space rect -> displayed frame (clockwise `/Rotate`, ISO 32000-1 §8.3.3).
fn to_display(b: Rect, rotate: i32) -> Rect {
    let map = |x: f32, y: f32| match rotate {
        90 => (y, W - x),
        180 => (W - x, H - y),
        270 => (H - y, x),
        _ => (x, y),
    };
    let (ax, ay) = map(b.x, b.y);
    let (bx, by) = map(b.x + b.width, b.y + b.height);
    Rect::new(ax.min(bx), ay.min(by), (ax - bx).abs(), (ay - by).abs())
}

fn assert_covers(bbox: Rect, ink: Rect, what: &str) {
    const SLACK: f32 = 3.0;
    let ok = ink.x >= bbox.x - SLACK
        && ink.y >= bbox.y - SLACK
        && ink.x + ink.width <= bbox.x + bbox.width + SLACK
        && ink.y + ink.height <= bbox.y + bbox.height + SLACK;
    assert!(ok, "{what}: box {bbox:?} does not cover ink {ink:?}");
    // Not a box that merely covers a much larger area: the run is horizontal
    // on the displayed page, so its box is wide and short like the ink.
    assert!(bbox.width > bbox.height, "{what}: box {bbox:?} is not along the line");
    assert!(
        bbox.height <= ink.height * 2.0 + SLACK,
        "{what}: box {bbox:?} too tall for {ink:?}"
    );
}

fn check(rotate: i32, tm: &str) {
    let doc = PdfDocument::from_bytes(pdf(rotate, tm)).unwrap();
    let ink = ink(&doc);
    let raw: Vec<_> = doc
        .extract_structured(0)
        .unwrap()
        .regions
        .into_iter()
        .flat_map(|r| r.spans)
        .collect();
    assert_eq!(raw.len(), 1, "{:?}", raw.iter().map(|s| &s.text).collect::<Vec<_>>());
    assert!(raw[0].turned_bbox);
    assert_eq!(raw[0].page_bbox(), raw[0].bbox);
    assert_covers(to_display(raw[0].bbox, rotate), ink, "extract_structured (raw frame)");
    let shown = doc.extract_spans(0).unwrap();
    assert_eq!(shown.len(), 1);
    assert!(!shown[0].turned_bbox);
    assert_covers(shown[0].page_bbox(), ink, "extract_spans page_bbox (displayed frame)");
    let words = doc.extract_words(0).unwrap();
    let union = words
        .iter()
        .skip(1)
        .fold(words[0].bbox, |a, w| a.union(&w.bbox));
    assert_covers(union, ink, "extract_words (displayed frame)");
    // `extract_page_text` chars of the turned raw span lie inside its box.
    let page = doc.extract_page_text(0).unwrap();
    let sb = page.spans[0].bbox;
    for c in page.chars.iter().filter(|c| !c.char.is_whitespace()) {
        let b = c.bbox;
        let inside = b.x >= sb.x - 0.5
            && b.y >= sb.y - 0.5
            && b.x + b.width <= sb.x + sb.width + 0.5
            && b.y + b.height <= sb.y + sb.height + 0.5;
        assert!(inside, "char {:?} box {b:?} outside span box {sb:?}", c.char);
    }
    let all = page
        .chars
        .iter()
        .skip(1)
        .fold(page.chars[0].bbox, |a, c| a.union(&c.bbox));
    assert_covers(to_display(all, rotate), ink, "extract_page_text chars (raw frame)");
}

#[test]
fn upright_run_on_a_rotate_90_page_covers_its_ink() {
    check(90, "0 1 -1 0 150 60");
}

#[test]
fn upright_run_on_a_rotate_270_page_covers_its_ink() {
    check(270, "0 -1 1 0 150 340");
}

#[test]
fn upright_run_on_a_rotate_180_page_covers_its_ink() {
    check(180, "-1 0 0 -1 250 200");
}

#[test]
fn upright_text_on_a_rotated_page_keeps_its_raw_box() {
    // Control (c17): text upright in raw space on a /Rotate 90 page is not
    // turned and keeps its along-x box.
    let doc = PdfDocument::from_bytes(pdf(90, "1 0 0 1 40 200")).unwrap();
    let s = doc.extract_spans(0).unwrap();
    assert!(!s[0].turned_bbox);
    assert!((s[0].bbox.x - 40.0).abs() < 0.01 && (s[0].bbox.y - 200.0).abs() < 0.01);
    assert!(s[0].bbox.width > s[0].bbox.height);
}
