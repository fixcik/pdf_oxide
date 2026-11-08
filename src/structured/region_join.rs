//! Whether a span joining a region continues the previous span's word.
//!
//! Region text joins its spans with a space. Producers that place every
//! glyph on its own (Chrome/Skia letter positioning: `<gid> Tj` per glyph,
//! kerning folded into the `Td` advance) draw a kerned pair with the second
//! glyph starting before the first one's advance ends — «г»+«л» at 11 pt
//! overlaps by 0.56 pt, `T`+`o` by ~1 pt. The span merger accepts overlaps
//! only down to −0.5 pt (`SpanMergingConfig::severe_overlap_threshold_pt`,
//! the same in every preset), so such a pair reaches the region as two spans
//! and the space split the word («округ лены», «T opics»). The spans stay
//! as they are (column detection and table logic read their boxes); only
//! the region text drops the space.

use crate::layout::TextSpan;

/// The span merger's absolute overlap floor (every `SpanMergingConfig`
/// preset); smaller overlaps were already merged there.
const MERGER_OVERLAP_FLOOR_PT: f32 = -0.5;
/// Kerning reaches about this fraction of the font size; the same bound
/// `extract_words` uses to glue characters into a word.
const KERNING_OVERLAP_EM: f32 = 0.15;

/// `next` continues the word of `prev` (the region's last span): same font (name, size, weight, slant),
/// same marked-content id, same upright baseline, a letter on both sides,
/// and `next` starts before `prev`'s advance ends by more than the merger's
/// floor but by no more than kerning does — at most 0.15 em of the smaller
/// size and at most half the narrower span (a run drawn over most of its
/// neighbour — stacked or offset duplicates, overprints — is not a kerned
/// pair).
pub(super) fn kerned(prev: Option<&TextSpan>, next: &TextSpan) -> bool {
    let Some(prev) = prev else {
        return false;
    };
    let letter = |c: Option<char>| c.is_some_and(char::is_alphabetic);
    let upright = |s: &TextSpan| s.rotation_degrees == 0.0 && !s.turned_bbox;
    if !prev.font_size.is_finite()
        || prev.font_name != next.font_name
        || (prev.font_size - next.font_size).abs() >= 0.01
        || prev.font_weight != next.font_weight
        || prev.is_italic != next.is_italic
        || prev.mcid != next.mcid
        || !upright(prev)
        || !upright(next)
        || (prev.bbox.y - next.bbox.y).abs() >= 1.0
        || !letter(prev.text.chars().next_back())
        || !letter(next.text.chars().next())
    {
        return false;
    }
    let gap = next.bbox.x - (prev.bbox.x + prev.bbox.width);
    let size = prev.font_size.min(next.font_size);
    let half_narrower = 0.5 * prev.bbox.width.min(next.bbox.width);
    let floor = MERGER_OVERLAP_FLOOR_PT.min(-(KERNING_OVERLAP_EM * size).min(half_narrower));
    (floor..MERGER_OVERLAP_FLOOR_PT).contains(&gap)
}
