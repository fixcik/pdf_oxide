//! Opt-in visibility facts for extracted text (hidden-text filtering).
//!
//! Enabled per document with
//! [`PdfDocument::set_visibility_facts`](crate::document::PdfDocument::set_visibility_facts).
//! When on, every extracted [`TextSpan`](crate::layout::TextSpan) carries a
//! [`SpanVisibility`] and each page gets a [`PageVisibility`] (read back with
//! [`PdfDocument::page_visibility`](crate::document::PdfDocument::page_visibility)).
//! The facts are raw observations from the content-stream pass; deciding what
//! counts as "hidden" (contrast, occlusion, OCR layers …) is left to the caller.
//!
//! All geometry is in the same PDF user space as the span bboxes returned by
//! `extract_structured` / `extract_spans_with_reading_order` (not rotated into
//! the `/Rotate` display frame).
//!
//! Known limits: a text clip (`Tr` 4–7) does not narrow the clip for later
//! content; clip paths are tracked as the bbox of their points (curves by their
//! control-point hull); a Form XObject without text is reported as one opaque
//! [`PaintKind::Form`] event instead of being walked; strokes are not
//! recorded; a fill in a named `/Pattern` colour space reports the last
//! device colour instead of `None`.

use crate::geometry::Rect;
use crate::layout::Color;

/// Visibility facts of one text span.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[cfg_attr(feature = "wasm", serde(rename_all = "camelCase"))]
#[non_exhaustive]
pub struct SpanVisibility {
    /// Text render mode (`Tr`, ISO 32000-1 §9.3.6): `3` invisible, `7` clip only.
    pub render_mode: u8,
    /// Non-stroking (fill) alpha, ExtGState `/ca` (§11.6.4.4). `1.0` = opaque.
    pub fill_alpha: f32,
    /// Stroking alpha, ExtGState `/CA`.
    pub stroke_alpha: f32,
    /// An ExtGState soft mask (`/SMask` other than `/None`) is active.
    pub soft_mask: bool,
    /// The ExtGState blend mode (`/BM`) is not `Normal`/`Compatible`.
    pub blend_non_normal: bool,
    /// Fraction (0..=1) of the span bbox inside the current clip region
    /// (`W n` / Form `/BBox`, as a bbox). `1.0` when nothing clips it; `0.0`
    /// for a zero-area clip or a clip elsewhere on the page.
    pub clip_coverage: f32,
    /// Position in the page's paint order, shared with
    /// [`PaintEvent::paint_order`]: a paint with a larger value was drawn on
    /// top of this span.
    pub paint_order: u32,
}

/// What a [`PaintEvent`] painted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[non_exhaustive]
pub enum PaintKind {
    /// A fill of a path consisting of exactly one `re` rectangle.
    Rect,
    /// A fill of any other path (bbox of its points).
    Path,
    /// An image XObject or inline image (bbox of the unit square under the CTM).
    Image,
    /// `sh` shading (bbox = the current clip).
    Shading,
    /// A Form XObject that carries no text and was not walked (its `/BBox`).
    Form,
}

/// One non-text paint operation on a page.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[cfg_attr(feature = "wasm", serde(rename_all = "camelCase"))]
#[non_exhaustive]
pub struct PaintEvent {
    /// Kind of paint.
    pub kind: PaintKind,
    /// Painted area (already intersected with the clip), user space.
    pub bbox: Rect,
    /// Fill colour for `Rect`/`Path` fills in a device/ICC colour space;
    /// `None` for patterns, images, shadings and forms.
    pub color: Option<Color>,
    /// Non-stroking alpha in effect (`/ca`).
    pub alpha: f32,
    /// The paint is masked: image `/SMask`, `/Mask` or `/ImageMask`, or an
    /// ExtGState soft mask is active.
    pub masked: bool,
    /// Position in the page's paint order (see [`SpanVisibility::paint_order`]).
    pub paint_order: u32,
}

/// Page-level visibility facts.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
#[cfg_attr(feature = "wasm", serde(rename_all = "camelCase"))]
#[non_exhaustive]
pub struct PageVisibility {
    /// Fills, images, shadings and text-free forms in paint order.
    pub paints: Vec<PaintEvent>,
    /// Spans dropped because they lie entirely outside CropBox ∩ MediaBox
    /// (by the last structured / reading-order extraction of the page).
    pub offpage_spans: usize,
    /// Non-whitespace characters in those dropped spans.
    pub offpage_chars: usize,
}

/// Axis-aligned box as `[x0, y0, x1, y1]` with `x0 <= x1`, `y0 <= y1`.
type Bounds = [f32; 4];

fn bounds_to_rect(b: Bounds) -> Rect {
    Rect::new(b[0], b[1], (b[2] - b[0]).max(0.0), (b[3] - b[1]).max(0.0))
}

fn intersect(a: Bounds, b: Bounds) -> Bounds {
    let x0 = a[0].max(b[0]);
    let y0 = a[1].max(b[1]);
    // An empty intersection collapses to a zero-area box at the overlap start.
    [x0, y0, a[2].min(b[2]).max(x0), a[3].min(b[3]).max(y0)]
}

/// Inherited part of the graphics state the facts need (`q`/`Q` scoped).
#[derive(Debug, Clone, Copy)]
struct VisState {
    clip: Option<Bounds>,
    fill_alpha: f32,
    stroke_alpha: f32,
    soft_mask: bool,
    blend_non_normal: bool,
}

impl Default for VisState {
    fn default() -> Self {
        Self {
            clip: None,
            fill_alpha: 1.0,
            stroke_alpha: 1.0,
            soft_mask: false,
            blend_non_normal: false,
        }
    }
}

/// Per-extraction tracker owned by the text extractor while facts are on.
#[derive(Debug, Default)]
pub(crate) struct VisibilityTracker {
    stack: Vec<VisState>,
    current: VisState,
    path: Option<Bounds>,
    path_segments: u32,
    path_is_single_rect: bool,
    pending_clip: bool,
    order: u32,
    pub(crate) paints: Vec<PaintEvent>,
}

impl VisibilityTracker {
    pub(crate) fn save(&mut self) {
        self.stack.push(self.current);
    }

    pub(crate) fn restore(&mut self) {
        if let Some(s) = self.stack.pop() {
            self.current = s;
        }
    }

    pub(crate) fn set_alpha(&mut self, fill: Option<f32>, stroke: Option<f32>) {
        if let Some(a) = fill.filter(|a| a.is_finite()) {
            self.current.fill_alpha = a.clamp(0.0, 1.0);
        }
        if let Some(a) = stroke.filter(|a| a.is_finite()) {
            self.current.stroke_alpha = a.clamp(0.0, 1.0);
        }
    }

    pub(crate) fn set_soft_mask(&mut self, on: bool) {
        self.current.soft_mask = on;
    }

    pub(crate) fn set_blend_non_normal(&mut self, on: bool) {
        self.current.blend_non_normal = on;
    }

    /// Intersect the current clip with `rect`'s points mapped through `ctm`.
    pub(crate) fn clip_to(
        &mut self,
        ctm: &crate::content::Matrix,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
    ) {
        let b = Self::map_box(ctm, x0, y0, x1, y1);
        self.current.clip = Some(match self.current.clip {
            Some(c) => intersect(c, b),
            None => b,
        });
    }

    fn map_box(ctm: &crate::content::Matrix, x0: f32, y0: f32, x1: f32, y1: f32) -> Bounds {
        let pts = [
            ctm.transform_point(x0, y0),
            ctm.transform_point(x1, y0),
            ctm.transform_point(x1, y1),
            ctm.transform_point(x0, y1),
        ];
        let mut b = [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ];
        for p in pts {
            b[0] = b[0].min(p.x);
            b[1] = b[1].min(p.y);
            b[2] = b[2].max(p.x);
            b[3] = b[3].max(p.y);
        }
        b
    }

    /// Add a path point (user space after `ctm`) to the current path bbox.
    pub(crate) fn path_point(&mut self, ctm: &crate::content::Matrix, x: f32, y: f32) {
        let p = ctm.transform_point(x, y);
        if !(p.x.is_finite() && p.y.is_finite()) {
            return;
        }
        let b = self.path.get_or_insert([p.x, p.y, p.x, p.y]);
        b[0] = b[0].min(p.x);
        b[1] = b[1].min(p.y);
        b[2] = b[2].max(p.x);
        b[3] = b[3].max(p.y);
    }

    /// Count a path segment; `rect` marks an `re` operator.
    pub(crate) fn path_segment(&mut self, rect: bool) {
        self.path_segments += 1;
        self.path_is_single_rect = rect && self.path_segments == 1;
    }

    pub(crate) fn mark_clip(&mut self) {
        self.pending_clip = true;
    }

    /// End the current path. `fill` = the painting operator fills (records a
    /// paint unless `suppressed`); a pending `W`/`W*` is applied after.
    pub(crate) fn end_path(&mut self, fill: bool, suppressed: bool, color: Option<Color>) {
        if let Some(b) = self.path {
            if fill && !suppressed {
                let painted = match self.current.clip {
                    Some(c) => intersect(b, c),
                    None => b,
                };
                let kind = if self.path_is_single_rect && self.path_segments == 1 {
                    PaintKind::Rect
                } else {
                    PaintKind::Path
                };
                self.push_paint(kind, painted, color, false);
            }
            if self.pending_clip {
                self.current.clip = Some(match self.current.clip {
                    Some(c) => intersect(c, b),
                    None => b,
                });
            }
        } else if self.pending_clip {
            // `W n` with no path: an empty clip (nothing is visible after it).
            let c = self.current.clip.unwrap_or([0.0; 4]);
            self.current.clip = Some([c[0], c[1], c[0], c[1]]);
        }
        self.path = None;
        self.path_segments = 0;
        self.path_is_single_rect = false;
        self.pending_clip = false;
    }

    /// Record an image (unit square under `ctm`).
    pub(crate) fn image(&mut self, ctm: &crate::content::Matrix, masked: bool) {
        let b = Self::map_box(ctm, 0.0, 0.0, 1.0, 1.0);
        let b = match self.current.clip {
            Some(c) => intersect(b, c),
            None => b,
        };
        let masked = masked || self.current.soft_mask;
        self.push_paint(PaintKind::Image, b, None, masked);
    }

    /// Record a text-free Form XObject by its `/BBox` under `ctm`.
    pub(crate) fn form(&mut self, ctm: &crate::content::Matrix, bbox: [f32; 4]) {
        let b = Self::map_box(ctm, bbox[0], bbox[1], bbox[2], bbox[3]);
        let b = match self.current.clip {
            Some(c) => intersect(b, c),
            None => b,
        };
        let masked = self.current.soft_mask;
        self.push_paint(PaintKind::Form, b, None, masked);
    }

    /// Record `sh` (fills the current clip; unbounded without one).
    pub(crate) fn shading(&mut self) {
        const FAR: f32 = 1.0e6;
        let b = self.current.clip.unwrap_or([-FAR, -FAR, FAR, FAR]);
        let masked = self.current.soft_mask;
        self.push_paint(PaintKind::Shading, b, None, masked);
    }

    fn push_paint(&mut self, kind: PaintKind, b: Bounds, color: Option<Color>, masked: bool) {
        if !b.iter().all(|v| v.is_finite()) {
            return;
        }
        let paint_order = self.next_order();
        self.paints.push(PaintEvent {
            kind,
            bbox: bounds_to_rect(b),
            color,
            alpha: self.current.fill_alpha,
            masked,
            paint_order,
        });
    }

    fn next_order(&mut self) -> u32 {
        let o = self.order;
        self.order = self.order.saturating_add(1);
        o
    }

    /// Facts for a span just emitted with `bbox` under `render_mode`.
    pub(crate) fn span_facts(&mut self, render_mode: u8, bbox: &Rect) -> SpanVisibility {
        let clip_coverage = match self.current.clip {
            None => 1.0,
            Some(c) => coverage(bbox, c),
        };
        SpanVisibility {
            render_mode,
            fill_alpha: self.current.fill_alpha,
            stroke_alpha: self.current.stroke_alpha,
            soft_mask: self.current.soft_mask,
            blend_non_normal: self.current.blend_non_normal,
            clip_coverage,
            paint_order: self.next_order(),
        }
    }
}

/// Dict facts of an image / form XObject needed for its paint event.
#[derive(Debug)]
pub(crate) struct XObjectPaintMeta {
    /// `/OC` entry (unresolved).
    pub(crate) oc: Option<crate::object::Object>,
    kind: XObjectPaintKind,
}

#[derive(Debug)]
enum XObjectPaintKind {
    Image {
        masked: bool,
    },
    Form {
        bbox: [f32; 4],
        matrix: crate::content::Matrix,
    },
    Other,
}

impl XObjectPaintMeta {
    pub(crate) fn from_dict(d: &std::collections::HashMap<String, crate::object::Object>) -> Self {
        use crate::object::Object;
        let nums = |key: &str, n: usize| -> Option<Vec<f32>> {
            match d.get(key) {
                Some(Object::Array(a)) if a.len() >= n => Some(
                    a.iter()
                        .take(n)
                        .map(|o| match o {
                            Object::Real(v) => *v as f32,
                            Object::Integer(v) => *v as f32,
                            _ => 0.0,
                        })
                        .collect(),
                ),
                _ => None,
            }
        };
        let kind = match d.get("Subtype").and_then(|s| s.as_name()) {
            Some("Image") => XObjectPaintKind::Image {
                masked: d.get("SMask").is_some_and(|m| !m.is_null())
                    || d.get("Mask").is_some_and(|m| !m.is_null())
                    || d.get("ImageMask").and_then(|o| o.as_bool()) == Some(true),
            },
            Some("Form") => match nums("BBox", 4) {
                Some(b) => XObjectPaintKind::Form {
                    bbox: [
                        b[0].min(b[2]),
                        b[1].min(b[3]),
                        b[0].max(b[2]),
                        b[1].max(b[3]),
                    ],
                    matrix: nums("Matrix", 6)
                        .map(|m| crate::content::Matrix {
                            a: m[0],
                            b: m[1],
                            c: m[2],
                            d: m[3],
                            e: m[4],
                            f: m[5],
                        })
                        .unwrap_or_else(crate::content::Matrix::identity),
                },
                None => XObjectPaintKind::Other,
            },
            _ => XObjectPaintKind::Other,
        };
        Self {
            oc: d.get("OC").cloned(),
            kind,
        }
    }
}

impl VisibilityTracker {
    /// Record an image / text-free form painted under `ctm`.
    pub(crate) fn xobject(&mut self, ctm: &crate::content::Matrix, meta: &XObjectPaintMeta) {
        match meta.kind {
            XObjectPaintKind::Image { masked } => self.image(ctm, masked),
            XObjectPaintKind::Form { bbox, matrix } => self.form(&matrix.multiply(ctm), bbox),
            XObjectPaintKind::Other => {},
        }
    }
}

/// Fraction of `bbox` inside `clip` (1-D when the bbox is degenerate).
fn coverage(bbox: &Rect, clip: Bounds) -> f32 {
    if clip[2] <= clip[0] || clip[3] <= clip[1] {
        return 0.0;
    }
    let (x0, x1) = (bbox.x.min(bbox.x + bbox.width), bbox.x.max(bbox.x + bbox.width));
    let (y0, y1) = (bbox.y.min(bbox.y + bbox.height), bbox.y.max(bbox.y + bbox.height));
    let along = |a0: f32, a1: f32, c0: f32, c1: f32| -> f32 {
        if a1 > a0 {
            ((a1.min(c1) - a0.max(c0)).max(0.0)) / (a1 - a0)
        } else if a0 >= c0 && a0 <= c1 {
            1.0
        } else {
            0.0
        }
    };
    let f = along(x0, x1, clip[0], clip[2]) * along(y0, y1, clip[1], clip[3]);
    if f.is_finite() {
        f.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

/// Spans whose facts differ in a way a hidden-text filter would treat
/// differently must not be merged or de-duplicated into one another.
pub(crate) fn facts_differ(a: &Option<SpanVisibility>, b: &Option<SpanVisibility>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => {
            let invisible = |v: &SpanVisibility| v.render_mode == 3 || v.render_mode == 7;
            invisible(a) != invisible(b)
                || (a.fill_alpha - b.fill_alpha).abs() > 0.01
                || a.soft_mask != b.soft_mask
                // Fully clipped out vs (at least partly) shown; a partly
                // clipped run (text overflowing a table cell) still merges.
                || (a.clip_coverage < 0.01) != (b.clip_coverage < 0.01)
        },
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::Matrix;

    #[test]
    fn zero_area_clip_has_zero_coverage() {
        let mut t = VisibilityTracker::default();
        t.path_point(&Matrix::identity(), 10.0, 10.0);
        t.path_segment(true);
        t.mark_clip();
        t.end_path(false, false, None);
        let v = t.span_facts(0, &Rect::new(0.0, 0.0, 50.0, 12.0));
        assert_eq!(v.clip_coverage, 0.0);
    }

    #[test]
    fn clip_restored_by_q_q() {
        let mut t = VisibilityTracker::default();
        t.save();
        t.clip_to(&Matrix::identity(), 0.0, 0.0, 10.0, 10.0);
        let inside = t.span_facts(0, &Rect::new(0.0, 0.0, 5.0, 5.0));
        let half = t.span_facts(0, &Rect::new(5.0, 0.0, 10.0, 5.0));
        t.restore();
        let after = t.span_facts(0, &Rect::new(500.0, 500.0, 5.0, 5.0));
        assert_eq!(inside.clip_coverage, 1.0);
        assert!((half.clip_coverage - 0.5).abs() < 1e-6);
        assert_eq!(after.clip_coverage, 1.0);
        assert!(inside.paint_order < half.paint_order && half.paint_order < after.paint_order);
    }

    #[test]
    fn single_re_fill_is_a_rect_paint() {
        let mut t = VisibilityTracker::default();
        t.path_point(&Matrix::identity(), 0.0, 0.0);
        t.path_point(&Matrix::identity(), 100.0, 20.0);
        t.path_segment(true);
        t.end_path(true, false, Some(Color::white()));
        assert_eq!(t.paints.len(), 1);
        assert_eq!(t.paints[0].kind, PaintKind::Rect);
        assert_eq!(t.paints[0].bbox, Rect::new(0.0, 0.0, 100.0, 20.0));
    }
}
