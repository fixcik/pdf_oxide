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
//! control-point hull), with [`ClipKind`] telling whether that bbox is exact;
//! a Form XObject without text is reported as one opaque
//! [`PaintKind::Form`] event instead of being walked; strokes are not
//! recorded. Per-glyph size facts (`SpanVisibility::tiny_glyphs` …) are
//! collected per show-text run and folded when spans merge. Fill colours:
//! Separation `/All` maps to gray `1 - tint` and `/Black` plus DeviceN of
//! process colorants map straight to CMYK; other Separation/DeviceN spaces
//! are evaluated only through Type 2 and Type 4 tint transforms (Type 0/3
//! give `None`); an `Indexed` base must be Device*, Cal* or ICCBased.

use crate::geometry::Rect;
use crate::layout::Color;
use crate::object::Object;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

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
    /// Kind of clip in effect when the span was emitted.
    pub clip_kind: ClipKind,
    /// Resolved fill (non-stroking) colour the span was drawn with. Equals
    /// the span's `color` in device-family spaces (`g`/`rg`/`k`, Device*,
    /// Cal*, Lab, ICCBased); `Separation`, `DeviceN` and `Indexed` spaces
    /// (literal or selected by resource name) are evaluated through their
    /// tint transform / lookup table. `None` = unknown (a Pattern space, an
    /// unresolvable colour-space name, an unsupported tint function or
    /// alternate space) or nothing painted (the `/None` colorant). Span
    /// merging ignores this field, so a merged span keeps its first run's
    /// value.
    pub fill_color: Option<Color>,
    /// ExtGState blend mode (`/BM`, ISO 32000-1 §11.3.5) in effect;
    /// `blend_non_normal == (blend_mode != BlendMode::Normal)`.
    pub blend_mode: BlendMode,
    /// Smallest device-space font size (`Tfs` × vertical scale of
    /// `CTM × Tm`, the span's `font_size`) among the runs merged into this
    /// span; equals `font_size` for an unmerged span.
    pub glyph_size_min: f32,
    /// Largest own advance of a non-whitespace glyph in user space: the
    /// glyph width `w0 × Tfs × Th` under `CTM × Tm`, **without** `Tc`/`Tw`
    /// and TJ adjustments, so letter spacing cannot make a tiny glyph look
    /// wide. `None` when no per-glyph widths were seen (e.g. an
    /// `/ActualText` replacement, or a span with only whitespace).
    pub glyph_advance_max: Option<f32>,
    /// Non-whitespace glyphs that are tiny on both axes: device font size
    /// `<= TINY_GLYPH_PT` and own advance (as in `glyph_advance_max`)
    /// `<= TINY_GLYPH_PT`. Summed over merged runs, so `0 < tiny_glyphs`
    /// on a span whose bbox is not tiny flags a tiny run merged into it or a
    /// letter-spaced tiny run. Approximate where one code maps to several
    /// characters (the code's width is split evenly). A span later split
    /// into pieces (fused-word / camel-case splits) keeps the whole run's
    /// per-glyph values on every piece, so the count is then an upper bound
    /// per piece. Under the non-default `WordBoundaryMode::Primary` TJ path
    /// the glyph advances come from the span's `char_widths` and include
    /// `Tc`.
    pub tiny_glyphs: u32,
}

/// Size threshold (in PDF points, both axes) behind
/// [`SpanVisibility::tiny_glyphs`].
pub const TINY_GLYPH_PT: f32 = 1.0;

/// ExtGState blend mode (`/BM`, ISO 32000-1 §11.3.5, Tables 136–137). For an
/// array value the first entry is taken. `Compatible` maps to `Normal`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[non_exhaustive]
pub enum BlendMode {
    /// `Normal` / `Compatible`, or no `/BM` set.
    #[default]
    Normal,
    /// `/Multiply`.
    Multiply,
    /// `/Screen`.
    Screen,
    /// `/Overlay`.
    Overlay,
    /// `/Darken`.
    Darken,
    /// `/Lighten`.
    Lighten,
    /// `/ColorDodge`.
    ColorDodge,
    /// `/ColorBurn`.
    ColorBurn,
    /// `/HardLight`.
    HardLight,
    /// `/SoftLight`.
    SoftLight,
    /// `/Difference`.
    Difference,
    /// `/Exclusion`.
    Exclusion,
    /// `/Hue`.
    Hue,
    /// `/Saturation`.
    Saturation,
    /// `/Color`.
    Color,
    /// `/Luminosity`.
    Luminosity,
    /// Any other name (or a non-name entry): not `Normal`, effect unknown.
    Other,
}

impl BlendMode {
    /// Blend mode of a `/BM` value (a name or an array of names).
    pub(crate) fn from_object(bm: &Object) -> Self {
        let first = match bm {
            Object::Array(a) => a.first().and_then(|o| o.as_name()),
            other => other.as_name(),
        };
        match first {
            None => BlendMode::Normal,
            Some("Normal") | Some("Compatible") => BlendMode::Normal,
            Some("Multiply") => BlendMode::Multiply,
            Some("Screen") => BlendMode::Screen,
            Some("Overlay") => BlendMode::Overlay,
            Some("Darken") => BlendMode::Darken,
            Some("Lighten") => BlendMode::Lighten,
            Some("ColorDodge") => BlendMode::ColorDodge,
            Some("ColorBurn") => BlendMode::ColorBurn,
            Some("HardLight") => BlendMode::HardLight,
            Some("SoftLight") => BlendMode::SoftLight,
            Some("Difference") => BlendMode::Difference,
            Some("Exclusion") => BlendMode::Exclusion,
            Some("Hue") => BlendMode::Hue,
            Some("Saturation") => BlendMode::Saturation,
            Some("Color") => BlendMode::Color,
            Some("Luminosity") => BlendMode::Luminosity,
            Some(_) => BlendMode::Other,
        }
    }
}

/// Per-glyph size statistics of one text run (collected only with facts on).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct GlyphStats {
    /// Largest own advance of a non-whitespace glyph (user space).
    pub(crate) advance_max: Option<f32>,
    /// Glyphs tiny on both axes (see [`SpanVisibility::tiny_glyphs`]).
    pub(crate) tiny: u32,
}

impl GlyphStats {
    /// Record one non-whitespace glyph of device font size `size` and own
    /// advance `advance` (user space).
    pub(crate) fn note(&mut self, size: f32, advance: f32) {
        let advance = if advance.is_finite() {
            advance.abs()
        } else {
            return;
        };
        self.advance_max = Some(self.advance_max.map_or(advance, |m| m.max(advance)));
        if size.abs() <= TINY_GLYPH_PT && advance <= TINY_GLYPH_PT {
            self.tiny = self.tiny.saturating_add(1);
        }
    }
}

/// Fold the per-glyph facts of `other` into `into` when two spans merge
/// (the other facts keep `into`'s first-run values).
pub(crate) fn merge_glyph_facts(into: &mut Option<SpanVisibility>, other: &Option<SpanVisibility>) {
    if let (Some(a), Some(b)) = (into.as_mut(), other) {
        a.glyph_size_min = a.glyph_size_min.min(b.glyph_size_min);
        a.glyph_advance_max = match (a.glyph_advance_max, b.glyph_advance_max) {
            (Some(x), Some(y)) => Some(x.max(y)),
            (x, y) => x.or(y),
        };
        a.tiny_glyphs = a.tiny_glyphs.saturating_add(b.tiny_glyphs);
    }
}

/// Shape class of the clip in effect (the clip bbox is always tracked; this
/// says how well the bbox describes the clip region).
///
/// The effective kind is the maximum (`EvenOdd` > `Path` > `Rect` > `None`)
/// over all clips intersected so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, serde::Serialize)]
#[non_exhaustive]
pub enum ClipKind {
    /// No path or `/BBox` clip in effect (text clips, `Tr` 4–7, are not tracked).
    #[default]
    None,
    /// Every clip in effect is a single axis-aligned rectangle (one `re` as the
    /// whole clip path under an axis-aligned CTM, a Form `/BBox` under an
    /// axis-aligned CTM, or an empty `W n`), so the clip bbox is exact. A
    /// single-rectangle `W*` is also `Rect` (same region as `W`).
    Rect,
    /// No `EvenOdd` clip, but some clip in effect is not an exact axis-aligned
    /// rectangle: several subpaths (e.g. `re re W n`), `m`/`l`/`c` polygons or
    /// curves, or a rectangle under a rotated/skewed CTM (also with `W*`); the
    /// bbox over-approximates it.
    Path,
    /// Some clip in effect was set with `W*` and is not a single rectangle
    /// (e.g. `outer re inner re W* n`, a ring); the bbox over-approximates it.
    EvenOdd,
}

/// Axis-aligned CTM: no rotation or skew other than 90-degree turns.
pub(crate) fn axis_aligned(m: &crate::content::Matrix) -> bool {
    (m.b == 0.0 && m.c == 0.0) || (m.a == 0.0 && m.d == 0.0)
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
    /// Fill colour for `Rect`/`Path` fills, resolved like
    /// [`SpanVisibility::fill_color`]; `None` when it is unknown (Pattern
    /// spaces, literal or named, included) and for images, shadings and forms.
    pub color: Option<Color>,
    /// Non-stroking alpha in effect (`/ca`).
    pub alpha: f32,
    /// The paint is masked: image `/SMask`, `/Mask` or `/ImageMask`, or an
    /// ExtGState soft mask is active.
    pub masked: bool,
    /// Position in the page's paint order (see [`SpanVisibility::paint_order`]).
    pub paint_order: u32,
    /// Kind of clip in effect when the paint was recorded (for `re W f` the
    /// clip before the new one).
    pub clip_kind: ClipKind,
    /// ExtGState blend mode (`/BM`) the paint was drawn with. Under a
    /// non-`Normal` mode (e.g. a `Multiply` highlight) the paint's colour
    /// does not simply cover what is below it.
    pub blend_mode: BlendMode,
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
#[derive(Debug, Clone)]
struct VisState {
    clip: Option<Bounds>,
    clip_kind: ClipKind,
    fill_alpha: f32,
    stroke_alpha: f32,
    soft_mask: bool,
    blend_mode: BlendMode,
    fill: Fill,
}

/// Fill colour state of the facts.
#[derive(Debug, Clone, Default)]
pub(crate) enum Fill {
    /// A device-family space: the extractor's own fill colour applies.
    #[default]
    Device,
    /// A Separation / DeviceN / Indexed space (`space`), or an unknown one
    /// (`space: None`: Pattern, unresolvable name, unsupported definition).
    Space {
        space: Option<Arc<TintSpace>>,
        color: Option<Color>,
    },
}

impl Fill {
    fn color(&self, device: Option<Color>) -> Option<Color> {
        match self {
            Fill::Device => device,
            Fill::Space { color, .. } => *color,
        }
    }
}

/// Process colour space a tint or index maps into.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Process {
    Gray,
    Rgb,
    Cmyk,
}

impl Process {
    fn n(self) -> usize {
        match self {
            Process::Gray => 1,
            Process::Rgb => 3,
            Process::Cmyk => 4,
        }
    }

    fn to_color(self, v: &[f32]) -> Option<Color> {
        let v = v.get(..self.n())?;
        if !v.iter().all(|x| x.is_finite()) {
            return None;
        }
        let c = |x: f32| x.clamp(0.0, 1.0);
        let (r, g, b) = match self {
            Process::Gray => (c(v[0]), c(v[0]), c(v[0])),
            Process::Rgb => (c(v[0]), c(v[1]), c(v[2])),
            Process::Cmyk => crate::color::cmyk_to_rgb(v[0], v[1], v[2], v[3]),
        };
        Some(Color::new(r, g, b))
    }
}

/// A tint transform (ISO 32000-1 §7.10) the facts can evaluate.
#[derive(Debug)]
pub(crate) enum TintFn {
    /// Exponential interpolation, single input.
    Type2 {
        c0: Vec<f32>,
        c1: Vec<f32>,
        n: f32,
        domain: [f32; 2],
    },
    /// PostScript calculator, compiled once.
    Type4 {
        program: crate::functions::Program,
        domain: Vec<[f64; 2]>,
        range: Vec<[f64; 2]>,
    },
}

impl TintFn {
    fn eval(&self, inputs: &[f32]) -> Option<Vec<f32>> {
        match self {
            TintFn::Type2 { c0, c1, n, domain } => {
                let t = inputs.first()?.max(domain[0]).min(domain[1]);
                let tn = t.powf(*n);
                Some(c0.iter().zip(c1).map(|(a, b)| a + tn * (b - a)).collect())
            },
            TintFn::Type4 {
                program,
                domain,
                range,
            } => {
                let inputs: Vec<f64> = inputs.iter().map(|&v| v as f64).collect();
                let out = program.evaluate_clamped(&inputs, domain, range).ok()?;
                Some(out.into_iter().map(|v| v as f32).collect())
            },
        }
    }
}

/// A resolved Separation / DeviceN / Indexed colour space.
#[derive(Debug)]
pub(crate) enum TintSpace {
    /// Separation `/All`: gray `1 - tint`.
    Gray,
    /// Separation `/None`, or DeviceN of `/None` colorants only.
    NoInk,
    /// Separation `/Black` or DeviceN of process colorants: component ->
    /// CMYK slot (`None` ignored).
    ProcessN(Vec<Option<usize>>),
    /// Tint transform into an alternate space.
    Function {
        n_in: usize,
        func: TintFn,
        alt: Process,
    },
    /// `[/Indexed base hival lookup]`.
    Indexed {
        base: Process,
        hival: usize,
        lookup: Vec<u8>,
    },
}

impl TintSpace {
    fn eval(&self, comps: &[f32]) -> Option<Color> {
        match self {
            TintSpace::Gray => match comps {
                [t] => Process::Gray.to_color(&[1.0 - t]),
                _ => None,
            },
            TintSpace::NoInk => None,
            TintSpace::ProcessN(slots) => {
                if comps.len() != slots.len() {
                    return None;
                }
                let mut cmyk = [0.0; 4];
                for (v, slot) in comps.iter().zip(slots) {
                    if let Some(s) = slot {
                        cmyk[*s] = *v;
                    }
                }
                Process::Cmyk.to_color(&cmyk)
            },
            TintSpace::Function { n_in, func, alt } => {
                if comps.len() != *n_in {
                    return None;
                }
                let out = func.eval(comps)?;
                if out.len() != alt.n() {
                    return None;
                }
                alt.to_color(&out)
            },
            TintSpace::Indexed {
                base,
                hival,
                lookup,
            } => {
                let i = *comps.first()?;
                if !i.is_finite() {
                    return None;
                }
                let i = (i.round().max(0.0) as usize).min(*hival);
                let n = base.n();
                let bytes = lookup.get(i * n..(i + 1) * n)?;
                let v: Vec<f32> = bytes.iter().map(|&b| b as f32 / 255.0).collect();
                base.to_color(&v)
            },
        }
    }

    /// Colour right after `cs` (§8.6.8): tint 1.0 per component, index 0.
    fn initial(&self) -> Option<Color> {
        match self {
            TintSpace::Indexed { .. } => self.eval(&[0.0]),
            TintSpace::ProcessN(slots) => self.eval(&vec![1.0; slots.len()]),
            TintSpace::Function { n_in, .. } => self.eval(&vec![1.0; *n_in]),
            TintSpace::Gray | TintSpace::NoInk => self.eval(&[1.0]),
        }
    }
}

/// How a colour space is treated by the facts.
#[derive(Debug, Clone)]
pub(crate) enum SpaceFact {
    Device,
    Unknown,
    Tint(Arc<TintSpace>),
}

type Doc<'a> = Option<&'a crate::document::PdfDocument>;

fn deref<'a>(o: &'a Object, doc: Doc) -> Option<Cow<'a, Object>> {
    match o.as_reference() {
        Some(r) => doc?
            .load_object(r)
            .ok()
            .filter(|o| o.as_reference().is_none())
            .map(Cow::Owned),
        None => Some(Cow::Borrowed(o)),
    }
}

fn num(o: &Object) -> Option<f32> {
    match o {
        Object::Integer(i) => Some(*i as f32),
        Object::Real(r) => Some(*r as f32),
        _ => None,
    }
    .filter(|v| v.is_finite())
}

fn nums(o: Option<&Object>, doc: Doc) -> Option<Vec<f32>> {
    let o = deref(o?, doc)?;
    o.as_array()?.iter().map(num).collect()
}

fn stream_bytes(o: &Object, doc: Doc) -> Option<Vec<u8>> {
    match o.as_reference() {
        Some(r) => {
            let d = doc?;
            let s = d.load_object(r).ok()?;
            matches!(s, Object::Stream { .. })
                .then(|| d.decode_stream_with_encryption(&s, r).ok())
                .flatten()
        },
        None => matches!(o, Object::Stream { .. })
            .then(|| o.decode_stream_data().ok())
            .flatten(),
    }
}

/// A process space (Device*, Cal*, ICCBased by `/N`) used as an alternate
/// or `Indexed` base.
fn process(o: &Object, doc: Doc) -> Option<Process> {
    let o = deref(o, doc)?;
    let (family, arg) = match &*o {
        Object::Name(n) => (n.as_str(), None),
        Object::Array(a) => (a.first()?.as_name()?, a.get(1)),
        _ => return None,
    };
    match family {
        "DeviceGray" | "CalGray" => Some(Process::Gray),
        "DeviceRGB" | "CalRGB" => Some(Process::Rgb),
        "DeviceCMYK" => Some(Process::Cmyk),
        "ICCBased" => {
            let s = deref(arg?, doc)?;
            match deref(s.as_dict()?.get("N")?, doc)?.as_integer()? {
                1 => Some(Process::Gray),
                3 => Some(Process::Rgb),
                4 => Some(Process::Cmyk),
                _ => None,
            }
        },
        _ => None,
    }
}

fn tint_fn(o: &Object, doc: Doc, n_in: usize, n_out: usize) -> Option<TintFn> {
    let f = deref(o, doc)?;
    let d = f.as_dict()?;
    let pairs = |k: &str| -> Vec<[f64; 2]> {
        nums(d.get(k), doc)
            .unwrap_or_default()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| [p[0] as f64, p[1] as f64])
            .collect()
    };
    match deref(d.get("FunctionType")?, doc)?.as_integer()? {
        2 if n_in == 1 => {
            let c0 = match d.get("C0") {
                Some(o) => nums(Some(o), doc)?,
                None => vec![0.0],
            };
            let c1 = match d.get("C1") {
                Some(o) => nums(Some(o), doc)?,
                None => vec![1.0],
            };
            let n = num(&*deref(d.get("N")?, doc)?)?;
            let domain = pairs("Domain")
                .first()
                .map(|p| [p[0].min(p[1]) as f32, p[0].max(p[1]) as f32])
                .unwrap_or([0.0, 1.0]);
            (c0.len() == n_out && c1.len() == n_out).then_some(TintFn::Type2 { c0, c1, n, domain })
        },
        4 => {
            let program = crate::functions::Program::compile(&stream_bytes(o, doc)?).ok()?;
            Some(TintFn::Type4 {
                program,
                domain: pairs("Domain"),
                range: pairs("Range"),
            })
        },
        _ => None,
    }
}

fn tint_space(family: &str, a: &[Object], doc: Doc) -> Option<TintSpace> {
    match family {
        "Separation" => match deref(a.get(1)?, doc)?.as_name()? {
            "All" => Some(TintSpace::Gray),
            "Black" => Some(TintSpace::ProcessN(vec![Some(3)])),
            "None" => Some(TintSpace::NoInk),
            _ => {
                let alt = process(a.get(2)?, doc)?;
                Some(TintSpace::Function {
                    n_in: 1,
                    func: tint_fn(a.get(3)?, doc, 1, alt.n())?,
                    alt,
                })
            },
        },
        "DeviceN" => {
            let names = deref(a.get(1)?, doc)?;
            let names: Vec<&str> = names
                .as_array()?
                .iter()
                .map(|n| n.as_name())
                .collect::<Option<_>>()?;
            if names.is_empty() || names.len() > 32 {
                return None;
            }
            let slots: Option<Vec<Option<usize>>> = names
                .iter()
                .map(|n| match *n {
                    "Cyan" => Some(Some(0)),
                    "Magenta" => Some(Some(1)),
                    "Yellow" => Some(Some(2)),
                    "Black" => Some(Some(3)),
                    "None" => Some(None),
                    _ => None,
                })
                .collect();
            match slots {
                Some(s) if s.iter().all(|x| x.is_none()) => Some(TintSpace::NoInk),
                Some(s) => Some(TintSpace::ProcessN(s)),
                None => {
                    let alt = process(a.get(2)?, doc)?;
                    Some(TintSpace::Function {
                        n_in: names.len(),
                        func: tint_fn(a.get(3)?, doc, names.len(), alt.n())?,
                        alt,
                    })
                },
            }
        },
        "Indexed" => {
            let base = process(a.get(1)?, doc)?;
            let hival = deref(a.get(2)?, doc)?.as_integer()?;
            if !(0..=255).contains(&hival) {
                return None;
            }
            let lookup = match &*deref(a.get(3)?, doc)? {
                Object::String(s) => s.clone(),
                Object::Stream { .. } => stream_bytes(a.get(3)?, doc)?,
                _ => return None,
            };
            Some(TintSpace::Indexed {
                base,
                hival: hival as usize,
                lookup,
            })
        },
        _ => None,
    }
}

/// Classify a colour space given as a name or an array.
fn classify(o: &Object, doc: Doc) -> SpaceFact {
    let (family, arr) = match o {
        Object::Name(n) => (n.as_str(), std::slice::from_ref(o)),
        Object::Array(a) => match a.first().and_then(|f| f.as_name()) {
            Some(f) => (f, a.as_slice()),
            None => return SpaceFact::Unknown,
        },
        _ => return SpaceFact::Unknown,
    };
    match family {
        "DeviceGray" | "DeviceRGB" | "DeviceCMYK" | "CalGray" | "CalRGB" | "Lab" | "ICCBased" => {
            SpaceFact::Device
        },
        "Separation" | "DeviceN" | "Indexed" => tint_space(family, arr, doc)
            .map(|t| SpaceFact::Tint(Arc::new(t)))
            .unwrap_or(SpaceFact::Unknown),
        _ => SpaceFact::Unknown,
    }
}

impl Default for VisState {
    fn default() -> Self {
        Self {
            clip: None,
            clip_kind: ClipKind::None,
            fill_alpha: 1.0,
            stroke_alpha: 1.0,
            soft_mask: false,
            blend_mode: BlendMode::Normal,
            fill: Fill::Device,
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
    pending_clip_even_odd: bool,
    pending_clip_axis: bool,
    order: u32,
    pub(crate) paints: Vec<PaintEvent>,
    /// `cs` operand -> resolved space, for the current `/Resources` only.
    cs_names: CsNames,
}

/// `cs` operand -> resolved space cache of one `/Resources` scope.
pub(crate) type CsNames = HashMap<String, SpaceFact>;

impl VisibilityTracker {
    pub(crate) fn save(&mut self) {
        self.stack.push(self.current.clone());
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

    /// The extractor's `/Resources` changed: drop the name cache.
    pub(crate) fn resources_changed(&mut self) {
        self.cs_names.clear();
    }

    /// Swap in the name cache of another `/Resources` scope (Form entry /
    /// exit), returning the current one.
    pub(crate) fn swap_cs_names(&mut self, names: CsNames) -> CsNames {
        std::mem::replace(&mut self.cs_names, names)
    }

    /// Fill state after `cs name`; `lookup` resolves `/Resources/ColorSpace/name`.
    pub(crate) fn fill_after_cs(
        &mut self,
        name: &str,
        doc: Doc,
        lookup: impl FnOnce(&str) -> Option<Object>,
    ) -> Fill {
        let fact = match self.cs_names.get(name) {
            Some(f) => f.clone(),
            None => {
                let literal = classify(&Object::Name(name.to_string()), doc);
                let f = match literal {
                    SpaceFact::Unknown if name != "Pattern" => lookup(name)
                        .map(|o| classify(&o, doc))
                        .unwrap_or(SpaceFact::Unknown),
                    f => f,
                };
                if self.cs_names.len() >= 256 {
                    self.cs_names.clear();
                }
                self.cs_names.insert(name.to_string(), f.clone());
                f
            },
        };
        match fact {
            SpaceFact::Device => Fill::Device,
            SpaceFact::Unknown => Fill::Space {
                space: None,
                color: None,
            },
            SpaceFact::Tint(t) => Fill::Space {
                color: t.initial(),
                space: Some(t),
            },
        }
    }

    /// Fill state after `sc`/`scn` with `comps` in the current space.
    pub(crate) fn fill_after_sc(&self, comps: &[f32]) -> Fill {
        match &self.current.fill {
            Fill::Device => Fill::Device,
            Fill::Space { space, .. } => Fill::Space {
                space: space.clone(),
                color: space.as_ref().and_then(|s| s.eval(comps)),
            },
        }
    }

    /// Switching to `new` changes a non-device fill fact (spans drawn
    /// before must be emitted first).
    pub(crate) fn fill_changes(&self, new: &Fill) -> bool {
        match (&self.current.fill, new) {
            (Fill::Device, Fill::Device) => false,
            (Fill::Space { color: a, .. }, Fill::Space { color: b, .. }) => a != b,
            _ => true,
        }
    }

    pub(crate) fn set_fill(&mut self, fill: Fill) {
        self.current.fill = fill;
    }

    pub(crate) fn set_soft_mask(&mut self, on: bool) {
        self.current.soft_mask = on;
    }

    pub(crate) fn set_blend_mode(&mut self, mode: BlendMode) {
        self.current.blend_mode = mode;
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
        let kind = if axis_aligned(ctm) {
            ClipKind::Rect
        } else {
            ClipKind::Path
        };
        self.current.clip_kind = self.current.clip_kind.max(kind);
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

    /// Mark the current path as a clip (`W`, or `W*` when `even_odd`);
    /// `axis_aligned` = the CTM at the clip operator is axis-aligned.
    pub(crate) fn mark_clip(&mut self, even_odd: bool, axis_aligned: bool) {
        self.pending_clip = true;
        self.pending_clip_even_odd = even_odd;
        self.pending_clip_axis = axis_aligned;
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
                let color = self.current.fill.color(color);
                self.push_paint(kind, painted, color, false);
            }
            if self.pending_clip {
                let single = self.path_is_single_rect;
                let kind = if single && self.pending_clip_axis {
                    ClipKind::Rect
                } else if self.pending_clip_even_odd && !single {
                    ClipKind::EvenOdd
                } else {
                    ClipKind::Path
                };
                self.current.clip_kind = self.current.clip_kind.max(kind);
                self.current.clip = Some(match self.current.clip {
                    Some(c) => intersect(c, b),
                    None => b,
                });
            }
        } else if self.pending_clip {
            self.current.clip_kind = self.current.clip_kind.max(ClipKind::Rect);
            // `W n` with no path: an empty clip (nothing is visible after it).
            let c = self.current.clip.unwrap_or([0.0; 4]);
            self.current.clip = Some([c[0], c[1], c[0], c[1]]);
        }
        self.path = None;
        self.path_segments = 0;
        self.path_is_single_rect = false;
        self.pending_clip = false;
        self.pending_clip_even_odd = false;
        self.pending_clip_axis = false;
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
            clip_kind: self.current.clip_kind,
            blend_mode: self.current.blend_mode,
        });
    }

    fn next_order(&mut self) -> u32 {
        let o = self.order;
        self.order = self.order.saturating_add(1);
        o
    }

    /// Facts for a span just emitted with `bbox` under `render_mode`, device
    /// fill colour `color`, device font size `font_size` and per-glyph
    /// statistics `glyphs`.
    pub(crate) fn span_facts(
        &mut self,
        render_mode: u8,
        bbox: &Rect,
        color: Color,
        font_size: f32,
        glyphs: GlyphStats,
    ) -> SpanVisibility {
        let clip_coverage = match self.current.clip {
            None => 1.0,
            Some(c) => coverage(bbox, c),
        };
        SpanVisibility {
            render_mode,
            fill_alpha: self.current.fill_alpha,
            stroke_alpha: self.current.stroke_alpha,
            soft_mask: self.current.soft_mask,
            blend_non_normal: self.current.blend_mode != BlendMode::Normal,
            clip_coverage,
            paint_order: self.next_order(),
            clip_kind: self.current.clip_kind,
            fill_color: self.current.fill.color(Some(color)),
            blend_mode: self.current.blend_mode,
            glyph_size_min: font_size,
            glyph_advance_max: glyphs.advance_max,
            tiny_glyphs: glyphs.tiny,
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
        t.mark_clip(false, true);
        t.end_path(false, false, None);
        let v = t.span_facts(
            0,
            &Rect::new(0.0, 0.0, 50.0, 12.0),
            Color::black(),
            12.0,
            GlyphStats::default(),
        );
        assert_eq!(v.clip_coverage, 0.0);
    }

    #[test]
    fn clip_restored_by_q_q() {
        let mut t = VisibilityTracker::default();
        t.save();
        t.clip_to(&Matrix::identity(), 0.0, 0.0, 10.0, 10.0);
        let inside = t.span_facts(
            0,
            &Rect::new(0.0, 0.0, 5.0, 5.0),
            Color::black(),
            12.0,
            GlyphStats::default(),
        );
        let half = t.span_facts(
            0,
            &Rect::new(5.0, 0.0, 10.0, 5.0),
            Color::black(),
            12.0,
            GlyphStats::default(),
        );
        t.restore();
        let after = t.span_facts(
            0,
            &Rect::new(500.0, 500.0, 5.0, 5.0),
            Color::black(),
            12.0,
            GlyphStats::default(),
        );
        assert_eq!(inside.clip_coverage, 1.0);
        assert!((half.clip_coverage - 0.5).abs() < 1e-6);
        assert_eq!(after.clip_coverage, 1.0);
        assert!(inside.paint_order < half.paint_order && half.paint_order < after.paint_order);
    }

    fn space(src: &str) -> Option<TintSpace> {
        let (_, o) = crate::parser::parse_object(src.as_bytes()).expect("object");
        let a = o.as_array().expect("array");
        tint_space(a[0].as_name().unwrap(), a, None)
    }

    fn rgb(c: Option<Color>) -> Option<[i32; 3]> {
        c.map(|c| [c.r, c.g, c.b].map(|v| (v * 1000.0).round() as i32))
    }

    #[test]
    fn separation_tints() {
        let cmyk = space(
            "[/Separation /Spot /DeviceCMYK \
             << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 1 0 0] /N 1 >>]",
        )
        .unwrap();
        let (r, g, b) = crate::color::cmyk_to_rgb(0.0, 0.5, 0.0, 0.0);
        assert_eq!(rgb(cmyk.eval(&[0.5])), rgb(Some(Color::new(r, g, b))));
        // N = 2 and Domain [0 0.5]: t = 0.8 clips to 0.5, gray 0.25.
        let pow = space(
            "[/Separation /Spot /DeviceGray \
             << /FunctionType 2 /Domain [0 0.5] /C0 [0] /C1 [1] /N 2 >>]",
        )
        .unwrap();
        assert_eq!(rgb(pow.eval(&[0.8])), Some([250; 3]));
        // C0/C1 not matching the alternate's component count.
        assert!(
            space("[/Separation /Spot /DeviceRGB << /FunctionType 2 /C0 [0] /C1 [1] /N 1 >>]")
                .is_none()
        );
        assert!(
            space("[/Separation /Spot /DeviceGray << /FunctionType 2 /C1 (x) /N 1 >>]").is_none()
        );
        let none = space("[/Separation /None /DeviceGray << /FunctionType 2 /N 1 >>]").unwrap();
        assert_eq!(none.eval(&[1.0]), None);
        let all = space("[/Separation /All /DeviceGray << /FunctionType 2 /N 1 >>]").unwrap();
        assert_eq!(rgb(all.eval(&[0.25])), Some([750; 3]));
        assert_eq!(all.eval(&[0.25, 0.5]), None);
    }

    #[test]
    fn devicen_spot_and_process_through_type4() {
        let (_, o) =
            crate::parser::parse_object(b"[/DeviceN [/Spot /Black] /DeviceGray 0]").unwrap();
        let mut a = o.as_array().unwrap().clone();
        let mut dict = HashMap::new();
        dict.insert("FunctionType".to_string(), Object::Integer(4));
        // gray = 1 - max(spot, black)
        a[3] = Object::Stream {
            dict,
            data: bytes::Bytes::from_static(b"{ 2 copy lt { exch } if pop 1 exch sub }"),
        };
        let t = tint_space("DeviceN", &a, None).unwrap();
        assert_eq!(rgb(t.eval(&[0.2, 0.6])), Some([400; 3]));
        assert_eq!(t.eval(&[0.2]), None, "component count");
        // Output count must match the alternate (gray needs exactly one).
        a[2] = Object::Name("DeviceRGB".into());
        assert_eq!(tint_space("DeviceN", &a, None).unwrap().eval(&[0.2, 0.6]), None);
    }

    #[test]
    fn malformed_indexed_is_unknown() {
        assert!(space("[/Indexed /DeviceRGB 300 <FF0000>]").is_none());
        let short = space("[/Indexed /DeviceRGB 2 <FF0000>]").unwrap();
        assert_eq!(rgb(short.eval(&[0.0])), Some([1000, 0, 0]));
        assert_eq!(short.eval(&[2.0]), None);
        assert_eq!(short.eval(&[f32::NAN]), None);
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

    fn clip_rect(t: &mut VisibilityTracker, even_odd: bool) {
        let m = Matrix::identity();
        t.path_point(&m, 0.0, 0.0);
        t.path_point(&m, 10.0, 10.0);
        t.path_segment(true);
        t.mark_clip(even_odd, true);
        t.end_path(false, false, None);
    }

    #[test]
    fn clip_kind_nested_q_restores_and_takes_max() {
        let mut t = VisibilityTracker::default();
        let bbox = Rect::new(0.0, 0.0, 1.0, 1.0);
        assert_eq!(
            t.span_facts(0, &bbox, Color::black(), 12.0, GlyphStats::default())
                .clip_kind,
            ClipKind::None
        );
        t.save();
        clip_rect(&mut t, true);
        assert_eq!(
            t.span_facts(0, &bbox, Color::black(), 12.0, GlyphStats::default())
                .clip_kind,
            ClipKind::Rect
        );
        t.save();
        let m = Matrix::identity();
        for _ in 0..2 {
            t.path_point(&m, 0.0, 0.0);
            t.path_point(&m, 5.0, 5.0);
            t.path_segment(false);
        }
        t.mark_clip(true, true);
        t.end_path(false, false, None);
        assert_eq!(
            t.span_facts(0, &bbox, Color::black(), 12.0, GlyphStats::default())
                .clip_kind,
            ClipKind::EvenOdd
        );
        t.restore();
        assert_eq!(
            t.span_facts(0, &bbox, Color::black(), 12.0, GlyphStats::default())
                .clip_kind,
            ClipKind::Rect
        );
        t.restore();
        assert_eq!(
            t.span_facts(0, &bbox, Color::black(), 12.0, GlyphStats::default())
                .clip_kind,
            ClipKind::None
        );
    }

    #[test]
    fn clip_kind_empty_clip_is_rect_and_rotated_form_bbox_is_path() {
        let mut t = VisibilityTracker::default();
        t.mark_clip(false, true);
        t.end_path(false, false, None);
        assert_eq!(t.current.clip_kind, ClipKind::Rect);
        let rot = Matrix {
            a: 0.7,
            b: 0.7,
            c: -0.7,
            d: 0.7,
            e: 0.0,
            f: 0.0,
        };
        t.clip_to(&rot, 0.0, 0.0, 1.0, 1.0);
        assert_eq!(t.current.clip_kind, ClipKind::Path);
    }
}
