//! Boxes of text runs turned upright against the page `/Rotate`.
//!
//! A run whose text matrix turns it by the page rotation (content angle ≡
//! `/Rotate`, e.g. `0 -s s 0 Tm` under `/Rotate 270`) reads as ordinary
//! horizontal text on the displayed page. The extractor boxes every run along
//! the raw x axis from its origin; for these runs that box lies across the
//! line they cover. The helpers here give them their physical box.

use super::PdfDocument;
use crate::geometry::Rect;
use crate::layout::{TextChar, TextSpan};

/// The quadrant (90 / 180 / 270) a non-mirrored run is turned by against the
/// page rotation `rot`, when its content angle equals `rot`.
fn turned_quadrant(s: &TextSpan, rot: i32) -> Option<i32> {
    let rot = rot.rem_euclid(360);
    (matches!(rot, 90 | 180 | 270)
        && !s.mirrored
        && !s.turned_bbox
        && (s.rotation_degrees.rem_euclid(360.0) - rot as f32).abs() < 0.5)
        .then_some(rot)
}

/// `(dx, dy)` rotated counter-clockwise by the quadrant `q`.
fn turn(q: i32, dx: f32, dy: f32) -> (f32, f32) {
    match q {
        90 => (-dy, dx),
        180 => (-dx, -dy),
        _ => (dy, -dx),
    }
}

/// Axis-aligned hull of `b` rotated by the quadrant `q` about `(px, py)`.
fn turn_rect(b: Rect, q: i32, px: f32, py: f32) -> Rect {
    let (ax, ay) = turn(q, b.x - px, b.y - py);
    let (bx, by) = turn(q, b.x + b.width - px, b.y + b.height - py);
    Rect::new(px + ax.min(bx), py + ay.min(by), (ax - bx).abs(), (ay - by).abs())
}

impl PdfDocument {
    /// Box runs turned upright against the page `/Rotate` (raw user space,
    /// `page_rotation_applied == 0`) by their physical hull
    /// ([`TextSpan::page_bbox`]) instead of the along-x extents, and mark them
    /// `turned_bbox`. All other runs are left alone.
    pub(super) fn turn_upright_run_boxes(spans: &mut [TextSpan], rot: i32) {
        for s in spans.iter_mut() {
            if s.rotation_degrees != 0.0
                && s.page_rotation_applied == 0
                && turned_quadrant(s, rot).is_some()
            {
                s.bbox = s.page_bbox();
                s.turned_bbox = true;
            }
        }
    }

    /// A displayed-frame span (already mapped through the page `/Rotate`) of
    /// a run turned upright against that rotation, re-boxed by its physical
    /// hull so its glyphs lay out along x like any horizontal run (words and
    /// text lines); `None` for every other span.
    pub(super) fn upright_in_displayed_frame(s: &TextSpan) -> Option<TextSpan> {
        if s.page_rotation_applied == 0 || s.rotation_degrees == 0.0 {
            return None;
        }
        turned_quadrant(s, s.page_rotation_applied)?;
        let mut t = s.clone();
        t.bbox = s.page_bbox();
        t.char_x_offsets.clear();
        t.turned_bbox = true;
        Some(t)
    }

    /// Chars of a raw-frame `turned_bbox` span, placed where its glyphs are:
    /// laid out along the run from its origin as before the box was turned,
    /// then each char box (and origin) turned about the run origin, so every
    /// char lies inside the span's box. Plain `to_chars` for other spans.
    pub(super) fn chars_of_reading_order_span(s: &TextSpan) -> Vec<TextChar> {
        let q = s.rotation_degrees.rem_euclid(360.0).round() as i32;
        if !s.turned_bbox || s.page_rotation_applied != 0 || !matches!(q, 90 | 180 | 270) {
            return s.to_chars();
        }
        // Undo the turn: run origin and along-x extents (see `page_bbox`).
        let b = s.bbox;
        let (px, py, w, h) = match q {
            90 => (b.x + b.width, b.y, b.height, b.width),
            180 => (b.x + b.width, b.y + b.height, b.width, b.height),
            _ => (b.x, b.y + b.height, b.height, b.width),
        };
        let mut flat = s.clone();
        flat.bbox = Rect::new(px, py, w, h);
        flat.turned_bbox = false;
        let mut chars = flat.to_chars();
        for c in &mut chars {
            c.bbox = turn_rect(c.bbox, q, px, py);
            let (ox, oy) = turn(q, c.origin_x - px, c.origin_y - py);
            c.origin_x = px + ox;
            c.origin_y = py + oy;
        }
        chars
    }
}
