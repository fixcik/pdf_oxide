//! Optional-content (layer) filtering of the text extractor.
//!
//! Groups are decided by reference through [`LayerFilter`]; see
//! [`crate::optional_content`] for why `/Name` is not an identity.

use std::collections::{HashMap, HashSet};

use super::TextExtractor;
use crate::object::{Object, ObjectRef};
use crate::optional_content::LayerFilter;

/// Streams above this size take the text-only parser's prescan route when
/// no emission filter is set (`parse_and_execute_text_only`).
const PRESCAN_THRESHOLD: usize = 256 * 1024;

impl<'doc> TextExtractor<'doc> {
    /// Set layer names (Optional Content Groups) to exclude from extraction.
    ///
    /// Content within BDC/EMC scopes tagged "OC" whose OCG /Name matches one of
    /// the provided names will be suppressed during text extraction. Names are
    /// not identities; [`Self::set_layer_filter`] decides by reference.
    pub fn set_excluded_layers(&mut self, layers: HashSet<String>) {
        self.excluded_layers = LayerFilter::from_names(layers);
    }

    /// Suppress the content of the optional content groups `filter` turns off
    /// (`BDC /OC` scopes, and XObjects whose `/OC` is hidden).
    pub fn set_layer_filter(&mut self, filter: LayerFilter) {
        self.excluded_layers = filter;
    }

    /// Resolve BDC properties (an inline dictionary or a name in the
    /// `/Properties` resource), with the reference of the named entry.
    pub(super) fn resolve_bdc_properties(
        &self,
        properties: &Object,
    ) -> Option<(Option<ObjectRef>, HashMap<String, Object>)> {
        crate::optional_content::resolve_bdc_properties_with_ref(
            properties,
            self.resources.as_ref(),
            self.document,
        )
    }

    /// Whether the `/OC` scope `dict` (reached through `oc_ref`) is excluded.
    pub(super) fn check_ocg_excluded(
        &self,
        oc_ref: Option<ObjectRef>,
        dict: &HashMap<String, Object>,
    ) -> bool {
        match self.document {
            Some(doc) => {
                crate::optional_content::check_oc_excluded(oc_ref, dict, doc, &self.excluded_layers)
            },
            None => false,
        }
    }

    /// The XObject dict carries `/OC` naming an excluded layer (§8.11.3.3:
    /// such an XObject is not drawn).
    pub(super) fn xobject_oc_hidden(&self, dict: &HashMap<String, Object>) -> bool {
        self.oc_entry_hidden(dict.get("OC"))
    }

    /// An `/OC` entry (of an XObject) that the filter hides.
    pub(super) fn oc_entry_hidden(&self, oc: Option<&Object>) -> bool {
        match (oc, self.document) {
            (Some(oc), Some(doc)) => {
                crate::optional_content::oc_entry_is_excluded(oc, doc, &self.excluded_layers)
            },
            _ => false,
        }
    }

    /// A `Do` inside an excluded layer draws nothing (§8.11.3.2), so it must
    /// neither be expanded nor be recorded as processed: the dedup guard of
    /// (form, CTM) would otherwise skip a later, visible draw of the same
    /// form at the same CTM.
    pub(super) fn do_is_in_excluded_layer(&self) -> bool {
        self.inside_excluded_layer
    }

    /// Whether `content_stream` must take the full parser although no
    /// emission filter is set: when the document's default configuration
    /// hides layers, a filtered and an unfiltered extraction of the same page
    /// must see the same text state, and the >256 KB prescan route loses the
    /// text state set outside `BT` (`TL`, `Tc`, `Tz`, …). Documents without
    /// default-off layers keep the prescan.
    pub(super) fn full_parse_for_layers(&mut self, content_stream: &[u8]) -> bool {
        if content_stream.len() <= PRESCAN_THRESHOLD {
            return false;
        }
        let Some(doc) = self.document else {
            return false;
        };
        *self
            .doc_hides_layers
            .get_or_insert_with(|| !LayerFilter::document_default(doc).is_empty())
    }
}
