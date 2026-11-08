//! Shared Optional Content (OCG / OCMD) helpers.
//!
//! Both the text-extraction path (`extractors::text`) and the rendering
//! pipeline (`rendering::page_renderer`) need to decide whether a BDC scope
//! tagged `/OC` belongs to an excluded layer. This module owns the logic so
//! the two callers cannot drift apart (a previous duplication caused a real
//! correctness bug where the renderer failed to decode UTF-16LE / PDFDocEncoding
//! layer names that the extractor handled correctly).
//!
//! NOTE — the two callers apply different *default* policies, by design.
//! `render_page` honours the PDF's own default configuration `/OCProperties/D`
//! (so an OCG marked off in `/D/OFF`, or off under `/D/BaseState /OFF`, is hidden
//! even when the caller passes no layer filter). `extract_text` instead treats
//! all content as visible unless the caller explicitly names layers to exclude,
//! and skips OCG evaluation entirely when that set is empty. Both are spec-
//! defensible — §8.11.3 NOTE 4 leaves it to the consumer whether optional content
//! is processed for a non-interactive (e.g. text-extraction) purpose — but it
//! means the two surfaces can legitimately disagree on visibility for the same
//! PDF. Callers wanting render/extract parity should pass matching `excluded_layers`.
//!
//! References:
//!  - ISO 32000-1:2008 §8.11.2 — Optional Content
//!  - ISO 32000-1:2008 §8.11.2.2 — Optional Content Membership Dictionaries
//!  - ISO 32000-1:2008 §7.9.2 — Text string encoding (UTF-16BE/LE/PDFDocEncoding)

use std::collections::{HashMap, HashSet};

use crate::document::PdfDocument;
use crate::object::Object;

mod layers;
pub use layers::{resolve_bdc_properties_with_ref, LayerFilter, OcGroupState};

/// Compute the document's default-off OCG names per ISO 32000-1 §8.11.4.
///
/// Reads `/OCProperties/D` (the default configuration) and returns the set of
/// OCG `/Name` strings that are off in the default state. Logic:
///
///  - `/D/BaseState` = `/ON` (or absent): OCGs default to on. `/D/OFF` lists
///     names that are explicitly turned off.
///  - `/D/BaseState` = `/OFF`: OCGs default to off. All OCGs in `/OCProperties
///    /OCGs` that are NOT in `/D/ON` are off.
///  - `/D/BaseState` = `/Unchanged`: treat as `/ON` (libraries have no prior
///    viewer state to preserve).
///  - `/OCProperties` or `/D` absent: returns empty set (all OCGs on).
///
/// Names are not identities: groups may share a `/Name` or lack one, so a
/// name set built from them hides a visible group named like a hidden one
/// and misses an unnamed one. Decide visibility with
/// [`LayerFilter::document_default`], which keys groups by reference.
pub fn compute_default_off_ocgs(doc: &PdfDocument) -> HashSet<String> {
    let mut off = HashSet::new();
    let catalog = match doc.catalog() {
        Ok(c) => c,
        Err(_) => return off,
    };
    let catalog_dict = match catalog.as_dict() {
        Some(d) => d,
        None => return off,
    };
    let oc_props = match resolve_indirect(catalog_dict.get("OCProperties"), doc) {
        Some(o) => o,
        None => return off,
    };
    let oc_dict = match oc_props.as_dict() {
        Some(d) => d,
        None => return off,
    };
    let d_obj = match resolve_indirect(oc_dict.get("D"), doc) {
        Some(o) => o,
        None => return off,
    };
    let d_dict = match d_obj.as_dict() {
        Some(d) => d,
        None => return off,
    };

    let base_state = d_dict
        .get("BaseState")
        .and_then(|o| o.as_name())
        .unwrap_or("ON");
    let on_set = collect_ocg_name_set(d_dict.get("ON"), doc);
    let off_set = collect_ocg_name_set(d_dict.get("OFF"), doc);

    if base_state == "OFF" {
        // All OCGs default off; explicit /ON entries are on.
        // Walk /OCProperties/OCGs and mark anything not in on_set as off.
        let all_ocgs = collect_ocg_name_set(oc_dict.get("OCGs"), doc);
        for name in all_ocgs {
            if !on_set.contains(&name) {
                off.insert(name);
            }
        }
    } else {
        // BaseState is /ON or /Unchanged: only explicit /D/OFF entries are off.
        off.extend(off_set);
    }

    off
}

fn resolve_indirect(obj: Option<&Object>, doc: &PdfDocument) -> Option<Object> {
    let obj = obj?;
    match obj.as_reference() {
        Some(r) => doc.load_object(r).ok(),
        None => Some(obj.clone()),
    }
}

fn collect_ocg_name_set(arr_obj: Option<&Object>, doc: &PdfDocument) -> HashSet<String> {
    let mut names = HashSet::new();
    let resolved = match resolve_indirect(arr_obj, doc) {
        Some(o) => o,
        None => return names,
    };
    let arr = match resolved.as_array() {
        Some(a) => a,
        None => return names,
    };
    for item in arr {
        let resolved = match item.as_reference() {
            Some(r) => match doc.load_object(r) {
                Ok(o) => o,
                Err(_) => continue,
            },
            None => item.clone(),
        };
        if let Some(d) = resolved.as_dict() {
            if let Some(Object::Name(n)) = d.get("Name") {
                names.insert(n.clone());
            } else if let Some(name_obj) = d.get("Name") {
                if let Some(b) = name_obj.as_string() {
                    names.insert(decode_pdf_text_string(b));
                }
            }
        }
    }
    names
}

/// OCMD visibility policy (`/P` entry). Per ISO 32000-1 §8.11.2.2 Table 102.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OcmdPolicy {
    /// Visible if **any** referenced OCG is on. (Default per spec.)
    AnyOn,
    /// Visible only if **all** referenced OCGs are on.
    AllOn,
    /// Visible if **any** referenced OCG is off.
    AnyOff,
    /// Visible only if **all** referenced OCGs are off.
    AllOff,
}

impl OcmdPolicy {
    fn from_name(s: &str) -> Self {
        match s {
            "AllOn" => OcmdPolicy::AllOn,
            "AnyOff" => OcmdPolicy::AnyOff,
            "AllOff" => OcmdPolicy::AllOff,
            // "AnyOn" or anything unknown -> spec default
            _ => OcmdPolicy::AnyOn,
        }
    }
}

/// Decode a PDF text string per ISO 32000-1 §7.9.2.
///
/// Handles:
///  - UTF-16BE with `FE FF` BOM
///  - UTF-16LE with `FF FE` BOM
///  - UTF-8 (lenient — non-spec PDFs sometimes embed raw UTF-8)
///  - PDFDocEncoding fallback (the spec default for non-BOM strings)
pub fn decode_pdf_text_string(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        let utf16_pairs: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
            .collect();
        String::from_utf16(&utf16_pairs)
            .unwrap_or_else(|_| String::from_utf8_lossy(bytes).to_string())
    } else if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        let utf16_pairs: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        String::from_utf16(&utf16_pairs)
            .unwrap_or_else(|_| String::from_utf8_lossy(bytes).to_string())
    } else {
        String::from_utf8(bytes.to_vec()).unwrap_or_else(|_| {
            bytes
                .iter()
                .filter_map(|&b| crate::fonts::font_dict::pdfdoc_encoding_lookup(b))
                .collect()
        })
    }
}

/// Check if a `Name` value (which can be either a `/Name` token or a PDF string)
/// matches any entry in `excluded`.
pub fn ocg_name_is_excluded(name_obj: &Object, excluded: &HashSet<String>) -> bool {
    if let Some(name_str) = name_obj.as_name() {
        return excluded.contains(name_str);
    }
    if let Some(name_bytes) = name_obj.as_string() {
        let name_str = decode_pdf_text_string(name_bytes);
        return excluded.contains(&name_str);
    }
    false
}

/// Resolve a BDC `properties` operand into a property dictionary.
///
/// `properties` is either an inline dictionary (e.g. `BDC /OC << /Name /Foo >>`)
/// or a name (e.g. `BDC /OC /MC0`) which references an entry in the page's
/// `/Resources /Properties` dictionary. Indirect references resolve at every
/// level. With `doc` `None` only the inline dictionary resolves.
/// [`resolve_bdc_properties_with_ref`] also returns the entry's reference.
pub fn resolve_bdc_properties(
    properties: &Object,
    resources: Option<&Object>,
    doc: Option<&PdfDocument>,
) -> Option<HashMap<String, Object>> {
    resolve_bdc_properties_with_ref(properties, resources, doc).map(|(_, d)| d)
}

/// Whether a resolved `/OC` dictionary is a membership dictionary (OCMD)
/// rather than a group. `/Type` is optional on both; an untyped dictionary
/// carrying `/OCGs` or `/VE` can only be an OCMD (§8.11.2.2).
fn is_ocmd(d: &HashMap<String, Object>) -> bool {
    match d.get("Type").and_then(|t| t.as_name()) {
        Some(t) => t == "OCMD",
        None => d.contains_key("OCGs") || d.contains_key("VE"),
    }
}

/// The off-state of every group an OCMD `/OCGs` entry references. `/OCGs`
/// may be a single group or an array; entries that do not resolve to a
/// dictionary (null, deleted objects) are ignored (§8.11.2.2 Table 99).
fn ocmd_member_states<S: OcGroupState + ?Sized>(
    ocgs_obj: &Object,
    doc: &PdfDocument,
    excluded: &S,
) -> Vec<bool> {
    let members: Vec<&Object> = match ocgs_obj.as_array() {
        Some(arr) => arr.iter().collect(),
        None => vec![ocgs_obj],
    };
    members
        .into_iter()
        .filter_map(|o| layers::resolve_with_ref(o, Some(doc)))
        .map(|(r, d)| excluded.group_is_off(r, &d))
        .collect()
}

/// Decide whether a resolved BDC properties dict represents an excluded
/// optional-content scope (OCG or OCMD). The dictionary's identity is not
/// known here; [`check_oc_excluded`] takes it.
pub fn check_ocg_excluded<S: OcGroupState + ?Sized>(
    props_dict: &HashMap<String, Object>,
    doc: &PdfDocument,
    excluded: &S,
) -> bool {
    check_oc_excluded(None, props_dict, doc, excluded)
}

/// Decide whether an `/OC` scope — the group or OCMD `dict`, reached through
/// `oc_ref` when it is indirect — is hidden under `excluded`.
///
///  - **OCMD**: if `/VE` is present, evaluate it (`/And`, `/Or`, `/Not`);
///    otherwise apply the `/P` policy (`AnyOn` default) over `/OCGs`.
///  - **OCG** (anything else, `/Name` or not): `excluded.group_is_off`.
pub fn check_oc_excluded<S: OcGroupState + ?Sized>(
    oc_ref: Option<crate::object::ObjectRef>,
    dict: &HashMap<String, Object>,
    doc: &PdfDocument,
    excluded: &S,
) -> bool {
    if !is_ocmd(dict) {
        return excluded.group_is_off(oc_ref, dict);
    }
    // /VE takes precedence over /P per ISO 32000-1 §8.11.2.4.
    if let Some(ve) = dict.get("VE") {
        return evaluate_visibility_expression(ve, doc, excluded) != Ve::Visible;
    }
    let policy = dict
        .get("P")
        .and_then(|o| o.as_name())
        .map(OcmdPolicy::from_name)
        .unwrap_or(OcmdPolicy::AnyOn);
    match dict.get("OCGs") {
        Some(o) => ocmd_is_hidden(&ocmd_member_states(o, doc, excluded), policy),
        None => false,
    }
}

/// Result of a `/VE` visibility expression. `Malformed` (nesting deeper
/// than [`VE_MAX_DEPTH`], e.g. a reference cycle, or more than
/// [`VE_MAX_NODES`] nodes, e.g. a DAG of shared references) propagates through every
/// operator — `/Not` included — and hides the scope: a structure no viewer
/// can be relied on to evaluate must not reveal what it guards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ve {
    Visible,
    Hidden,
    Malformed,
}

/// Nesting bound of a `/VE` expression. ISO 32000-1 sets none; poppler stops
/// at 50.
const VE_MAX_DEPTH: u8 = 64;

/// Node budget of one `/VE` evaluation. Shared references make a DAG that a
/// tree walk visits exponentially often; past the budget the expression is
/// `Malformed` (hidden), so a hostile file cannot stall the extractor.
const VE_MAX_NODES: u32 = 4096;

/// Evaluate an OCMD `/VE` visibility expression (ISO 32000-1 §8.11.2.4): an
/// array of an operator (`/And`, `/Or`, `/Not`) and operands, each a group
/// reference or a nested expression.
fn evaluate_visibility_expression<S: OcGroupState + ?Sized>(
    expr: &Object,
    doc: &PdfDocument,
    excluded: &S,
) -> Ve {
    fn eval<S: OcGroupState + ?Sized>(
        expr: &Object,
        doc: &PdfDocument,
        ex: &S,
        depth: u8,
        budget: &mut u32,
    ) -> Ve {
        if depth > VE_MAX_DEPTH || *budget == 0 {
            return Ve::Malformed;
        }
        *budget -= 1;
        let group_ref = expr.as_reference();
        let resolved = match group_ref {
            Some(r) => doc.load_object(r).unwrap_or_else(|_| expr.clone()),
            None => expr.clone(),
        };
        // Leaf: a group dictionary.
        if let Some(d) = resolved.as_dict() {
            return if ex.group_is_off(group_ref, d) {
                Ve::Hidden
            } else {
                Ve::Visible
            };
        }
        let Some(arr) = resolved.as_array() else {
            return Ve::Visible;
        };
        let Some(op) = arr.first().and_then(|o| o.as_name()) else {
            return Ve::Visible;
        };
        match op {
            "Not" => match arr
                .get(1)
                .map_or(Ve::Visible, |o| eval(o, doc, ex, depth + 1, budget))
            {
                Ve::Visible => Ve::Hidden,
                Ve::Hidden => Ve::Visible,
                Ve::Malformed => Ve::Malformed,
            },
            "And" | "Or" => {
                // Evaluate every operand: a malformed one anywhere decides.
                let all: Vec<Ve> = arr[1..]
                    .iter()
                    .map(|o| eval(o, doc, ex, depth + 1, budget))
                    .collect();
                if all.contains(&Ve::Malformed) {
                    Ve::Malformed
                } else if (op == "And" && all.iter().all(|v| *v == Ve::Visible))
                    || (op == "Or" && all.contains(&Ve::Visible))
                {
                    Ve::Visible
                } else {
                    Ve::Hidden
                }
            },
            _ => Ve::Visible,
        }
    }

    let mut budget = VE_MAX_NODES;
    eval(expr, doc, excluded, 0, &mut budget)
}

/// Resolve BDC properties and decide if the resulting scope is excluded.
///
/// Convenience wrapper combining [`resolve_bdc_properties_with_ref`] and
/// [`check_oc_excluded`]. Without a document only an inline group
/// dictionary can be decided.
pub fn resolve_and_check_ocg_excluded<S: OcGroupState + ?Sized>(
    properties: &Object,
    resources: Option<&Object>,
    doc: Option<&PdfDocument>,
    excluded: &S,
) -> bool {
    let Some((oc_ref, dict)) = resolve_bdc_properties_with_ref(properties, resources, doc) else {
        return false;
    };
    match doc {
        Some(d) => check_oc_excluded(oc_ref, &dict, d, excluded),
        None => !is_ocmd(&dict) && excluded.group_is_off(oc_ref, &dict),
    }
}

/// Resolve an annotation `/OC` entry (an OCG or OCMD dict, possibly indirect)
/// and decide whether the annotation belongs to an excluded layer.
///
/// Per ISO 32000-1 §12.5.2, annotation dictionaries can carry an `/OC` entry
/// that references the OCG / OCMD the annotation belongs to. If that scope is
/// excluded, the annotation should not be rendered.
pub fn annotation_is_excluded<S: OcGroupState + ?Sized>(
    oc_obj: &Object,
    doc: &PdfDocument,
    excluded: &S,
) -> bool {
    oc_entry_is_excluded(oc_obj, doc, excluded)
}

/// An `/OC` entry (of an XObject or annotation): hidden under `excluded`.
pub fn oc_entry_is_excluded<S: OcGroupState + ?Sized>(
    oc_obj: &Object,
    doc: &PdfDocument,
    excluded: &S,
) -> bool {
    if excluded.is_empty() {
        return false;
    }
    match layers::resolve_with_ref(oc_obj, Some(doc)) {
        Some((r, d)) => check_oc_excluded(r, &d, doc, excluded),
        None => false,
    }
}

/// Apply an OCMD policy to the off-states of its member groups.
///
/// Returns `true` if the content should be **hidden**. With no members the
/// result is `false` (such an OCMD is always visible).
///
///  - `AnyOn`  — visible iff any member is on → hide iff all are off.
///  - `AllOn`  — visible iff all members are on → hide iff any is off.
///  - `AnyOff` — visible iff any member is off → hide iff all are on.
///  - `AllOff` — visible iff all members are off → hide iff any is on.
fn ocmd_is_hidden(member_off: &[bool], policy: OcmdPolicy) -> bool {
    if member_off.is_empty() {
        return false;
    }
    let any_off = member_off.iter().any(|&o| o);
    let any_on = member_off.iter().any(|&o| !o);
    match policy {
        OcmdPolicy::AnyOn => !any_on,
        OcmdPolicy::AllOn => any_off,
        OcmdPolicy::AnyOff => !any_off,
        OcmdPolicy::AllOff => any_on,
    }
}

#[cfg(test)]
#[path = "optional_content_test.rs"]
mod tests;
