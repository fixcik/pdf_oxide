//! What the font-set caches of `PdfDocument::load_fonts` store.

use crate::fonts::FontInfo;
use crate::object::Object;
use std::collections::HashMap;
use std::sync::Arc;

/// The fonts `font_dict` itself defines, taken from the extractor's font set
/// after loading it, minus the names whose load failed.
///
/// The set-level caches (by `/Font` reference, by name→reference fingerprint
/// and by font names) are shared by every caller of the same dictionary. A
/// Form XObject's `/Font` dictionary is loaded into an extractor that already
/// holds the page's fonts, so the extractor's whole set would carry the fonts
/// of whichever page filled the cache first; and leaving out every name the
/// page already had (the former Layer-4 "delta") drops the form's own fonts
/// whose names shadow page fonts (`/T1_0` of the form vs `/T1_0` of the page),
/// so a later hit kept the page font. Either way the result depended on which
/// page — under the parallel header/footer scan, which thread — came first.
pub(super) fn own_fonts(
    font_set: Vec<(String, Arc<FontInfo>)>,
    font_dict: &HashMap<String, Object>,
    failed: &[&str],
) -> Vec<(String, Arc<FontInfo>)> {
    font_set
        .into_iter()
        .filter(|(name, _)| font_dict.contains_key(name) && !failed.contains(&name.as_str()))
        .collect()
}
