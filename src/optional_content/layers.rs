//! Optional-content group identity and the document's default state.
//!
//! An optional content group is an indirect object (ISO 32000-1:2008
//! §8.11.2.1), and every place that refers to one — `/OCProperties /OCGs`,
//! `/D /ON`, `/D /OFF`, an OCMD's `/OCGs` and `/VE`, the `/Properties`
//! entry a `BDC /OC` names, an XObject's or annotation's `/OC` — holds a
//! reference to that object. The `/Name` entry is only text for a user
//! interface: two groups may share it, and a group may lack it. Deciding
//! visibility by `/Name` therefore hides a visible group that shares the
//! name of a hidden one, and shows a hidden group that has no name.
//!
//! [`LayerFilter`] decides by reference. It still carries a set of layer
//! names for callers that ask to exclude layers by name (the long-standing
//! `excluded_layers` APIs); those match by `/Name`, as they always did.

use std::collections::{HashMap, HashSet};

use crate::document::PdfDocument;
use crate::object::{Object, ObjectRef};

/// What decides whether one optional content group is off.
///
/// Implemented by [`LayerFilter`] (by reference, plus caller names) and by
/// `HashSet<String>` (caller layer names only, matched by `/Name`), so the
/// public helpers of [`crate::optional_content`] accept either.
pub trait OcGroupState {
    /// No group is off: nothing needs evaluating.
    fn is_empty(&self) -> bool;
    /// Whether the group `group` (its dictionary, and its reference when it
    /// was reached through one) is off.
    fn group_is_off(&self, group_ref: Option<ObjectRef>, group: &HashMap<String, Object>) -> bool;
}

impl OcGroupState for HashSet<String> {
    fn is_empty(&self) -> bool {
        HashSet::is_empty(self)
    }

    fn group_is_off(&self, _: Option<ObjectRef>, group: &HashMap<String, Object>) -> bool {
        group
            .get("Name")
            .is_some_and(|n| super::ocg_name_is_excluded(n, self))
    }
}

/// The set of optional content groups to treat as off.
///
/// Built from the document's default configuration
/// ([`LayerFilter::document_default`]), from caller layer names
/// ([`LayerFilter::from_names`]), or both ([`LayerFilter::with_names`]).
///
/// The default configuration (§8.11.4.3): under `/BaseState /OFF` the
/// groups of `/OCGs` that `/ON` does not list are off; under `/ON` (or
/// `/Unchanged`) the groups `/OFF` lists. A group `/OCGs` does not list
/// (which §8.11.4.2 does not allow) and a direct (inline) group dictionary,
/// which has no reference, stay on — as poppler and pdf.js display them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LayerFilter {
    names: HashSet<String>,
    off: HashSet<ObjectRef>,
}

impl LayerFilter {
    /// Exclude the layers whose `/Name` is in `names` (the legacy rule).
    pub fn from_names(names: HashSet<String>) -> Self {
        Self {
            names,
            ..Self::default()
        }
    }

    /// Also exclude the layers whose `/Name` is in `names`.
    pub fn with_names(mut self, names: impl IntoIterator<Item = String>) -> Self {
        self.names.extend(names);
        self
    }

    /// The groups that `/OCProperties /D` (the default viewing configuration)
    /// turns off, identified by reference. Empty when the document has no
    /// optional content or its default hides nothing.
    pub fn document_default(doc: &PdfDocument) -> Self {
        let mut filter = Self::default();
        let Some(oc) = doc
            .catalog()
            .ok()
            .and_then(|c| resolved(c.as_dict()?.get("OCProperties"), doc))
        else {
            return filter;
        };
        let Some(oc) = oc.as_dict() else {
            return filter;
        };
        let Some(d) = resolved(oc.get("D"), doc) else {
            return filter;
        };
        let Some(d) = d.as_dict() else {
            return filter;
        };
        filter.off = if d.get("BaseState").and_then(|o| o.as_name()) == Some("OFF") {
            let on = refs(d.get("ON"), doc);
            let mut all = refs(oc.get("OCGs"), doc);
            all.retain(|r| !on.contains(r));
            all
        } else {
            refs(d.get("OFF"), doc)
        };
        filter
    }

    /// Whether the filter turns off no group at all.
    pub fn is_empty(&self) -> bool {
        self.names.is_empty() && self.off.is_empty()
    }

    /// Whether the group behind `group_ref` is off by reference alone (the
    /// default state; caller names are not consulted).
    pub fn ref_is_off(&self, group_ref: Option<ObjectRef>) -> bool {
        group_ref.is_some_and(|r| self.off.contains(&r))
    }
}

impl OcGroupState for LayerFilter {
    fn is_empty(&self) -> bool {
        LayerFilter::is_empty(self)
    }

    fn group_is_off(&self, group_ref: Option<ObjectRef>, group: &HashMap<String, Object>) -> bool {
        self.ref_is_off(group_ref)
            || (!self.names.is_empty() && self.names.group_is_off(group_ref, group))
    }
}

fn resolved(obj: Option<&Object>, doc: &PdfDocument) -> Option<Object> {
    let obj = obj?;
    match obj.as_reference() {
        Some(r) => doc.load_object(r).ok(),
        None => Some(obj.clone()),
    }
}

/// The references in an array of groups (`/ON`, `/OFF`, `/OCGs`); direct
/// entries carry no identity and are skipped.
fn refs(arr: Option<&Object>, doc: &PdfDocument) -> HashSet<ObjectRef> {
    resolved(arr, doc)
        .as_ref()
        .and_then(|a| a.as_array())
        .map(|a| a.iter().filter_map(|o| o.as_reference()).collect())
        .unwrap_or_default()
}

/// Resolve an object that names a group or membership dictionary, keeping
/// the reference it was reached through (its identity).
pub(super) fn resolve_with_ref(
    obj: &Object,
    doc: Option<&PdfDocument>,
) -> Option<(Option<ObjectRef>, HashMap<String, Object>)> {
    match obj.as_reference() {
        Some(r) => {
            let loaded = doc?.load_object(r).ok()?;
            Some((Some(r), loaded.as_dict()?.clone()))
        },
        None => Some((None, obj.as_dict()?.clone())),
    }
}

/// [`super::resolve_bdc_properties`], also returning the reference of the
/// `/Properties` entry the operand names (`None` for an inline dictionary).
pub fn resolve_bdc_properties_with_ref(
    properties: &Object,
    resources: Option<&Object>,
    doc: Option<&PdfDocument>,
) -> Option<(Option<ObjectRef>, HashMap<String, Object>)> {
    if let Some(dict) = properties.as_dict() {
        return Some((None, dict.clone()));
    }
    let prop_name = properties.as_name()?;
    let doc_ref = doc?;
    let res = resolved(resources, doc_ref)?;
    let props = resolved(res.as_dict()?.get("Properties"), doc_ref)?;
    let entry = props.as_dict()?.get(prop_name)?;
    resolve_with_ref(entry, doc)
}
