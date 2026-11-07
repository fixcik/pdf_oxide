//! Per-glyph size facts and the ExtGState blend mode of the visibility facts
//! (re-exported from [`super`]).

use super::SpanVisibility;
use crate::object::Object;

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
