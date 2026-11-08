//! Unit tests of `optional_content` (name decoding, OCMD policies).

use super::*;

#[test]
fn decode_utf16be_bom() {
    // FE FF "Layer"
    let bytes = b"\xFE\xFF\x00L\x00a\x00y\x00e\x00r";
    assert_eq!(decode_pdf_text_string(bytes), "Layer");
}

#[test]
fn decode_utf16le_bom() {
    // FF FE "Layer"
    let bytes = b"\xFF\xFEL\x00a\x00y\x00e\x00r\x00";
    assert_eq!(decode_pdf_text_string(bytes), "Layer");
}

#[test]
fn decode_utf8_ascii() {
    assert_eq!(decode_pdf_text_string(b"Hello"), "Hello");
}

#[test]
fn decode_pdfdoc_fallback() {
    // 0x85 in PDFDocEncoding = U+2013 (endash)
    assert_eq!(decode_pdf_text_string(&[0x85]), "\u{2013}");
}

#[test]
fn ocg_name_is_excluded_matches_name_token() {
    let mut excluded = HashSet::new();
    excluded.insert("Watermark".to_string());
    assert!(ocg_name_is_excluded(&Object::Name("Watermark".into()), &excluded));
    assert!(!ocg_name_is_excluded(&Object::Name("Other".into()), &excluded));
}

#[test]
fn ocg_name_is_excluded_matches_utf16le_string() {
    let mut excluded = HashSet::new();
    excluded.insert("Layer".to_string());
    let bytes: Vec<u8> = b"\xFF\xFEL\x00a\x00y\x00e\x00r\x00".to_vec();
    assert!(ocg_name_is_excluded(&Object::String(bytes), &excluded));
}

#[test]
fn policy_default_is_any_on() {
    assert_eq!(OcmdPolicy::from_name("nonsense"), OcmdPolicy::AnyOn);
    assert_eq!(OcmdPolicy::from_name("AnyOn"), OcmdPolicy::AnyOn);
    assert_eq!(OcmdPolicy::from_name("AllOn"), OcmdPolicy::AllOn);
    assert_eq!(OcmdPolicy::from_name("AnyOff"), OcmdPolicy::AnyOff);
    assert_eq!(OcmdPolicy::from_name("AllOff"), OcmdPolicy::AllOff);
}

/// The pre-reference rule: member off-states by `/Name` membership.
fn hidden_by_name(names: &[Object], policy: OcmdPolicy, excluded: &HashSet<String>) -> bool {
    let off: Vec<bool> = names
        .iter()
        .map(|n| ocg_name_is_excluded(n, excluded))
        .collect();
    ocmd_is_hidden(&off, policy)
}

fn names(slice: &[&str]) -> Vec<Object> {
    slice.iter().map(|s| Object::Name((*s).into())).collect()
}

#[test]
fn policy_any_on_hides_when_all_excluded() {
    let mut excluded = HashSet::new();
    excluded.insert("A".to_string());
    excluded.insert("B".to_string());

    // both off -> hidden
    assert!(hidden_by_name(&names(&["A", "B"]), OcmdPolicy::AnyOn, &excluded));
    // one on -> visible
    assert!(!hidden_by_name(&names(&["A", "C"]), OcmdPolicy::AnyOn, &excluded));
}

#[test]
fn policy_all_on_hides_when_any_excluded() {
    let mut excluded = HashSet::new();
    excluded.insert("A".to_string());

    // any off -> hidden
    assert!(hidden_by_name(&names(&["A", "B"]), OcmdPolicy::AllOn, &excluded));
    // all on -> visible
    assert!(!hidden_by_name(&names(&["C", "B"]), OcmdPolicy::AllOn, &excluded));
}

#[test]
fn policy_any_off_hides_when_all_on() {
    let mut excluded = HashSet::new();
    excluded.insert("A".to_string());

    // some off -> visible
    assert!(!hidden_by_name(&names(&["A", "B"]), OcmdPolicy::AnyOff, &excluded));
    // all on -> hidden
    assert!(hidden_by_name(&names(&["B", "C"]), OcmdPolicy::AnyOff, &excluded));
}

#[test]
fn policy_all_off_hides_when_any_on() {
    let mut excluded = HashSet::new();
    excluded.insert("A".to_string());
    excluded.insert("B".to_string());

    // any on -> hidden
    assert!(hidden_by_name(&names(&["A", "C"]), OcmdPolicy::AllOff, &excluded));
    // all off -> visible
    assert!(!hidden_by_name(&names(&["A", "B"]), OcmdPolicy::AllOff, &excluded));
}

#[test]
fn empty_ocgs_is_not_hidden() {
    let excluded = HashSet::new();
    assert!(!hidden_by_name(&[], OcmdPolicy::AnyOn, &excluded));
    assert!(!hidden_by_name(&[], OcmdPolicy::AllOn, &excluded));
    assert!(!hidden_by_name(&[], OcmdPolicy::AnyOff, &excluded));
    assert!(!hidden_by_name(&[], OcmdPolicy::AllOff, &excluded));
}
