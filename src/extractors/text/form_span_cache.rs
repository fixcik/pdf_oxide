//! The document-wide span cache of self-contained Form XObjects
//! (`PdfDocument::xobject_spans_cache`, keyed by form and caller CTM).
//!
//! A cached entry carries the context of the page that filled it (its span
//! `sequence` numbers above all), so which page fills an entry first decides
//! what every later page replays. Extractions whose page order is not the
//! caller's — the parallel running header/footer scan — therefore neither
//! read nor fill it ([`TextExtractor::bypass_form_span_cache`]); otherwise
//! the output would depend on thread timing.

use super::TextExtractor;
use crate::layout::TextSpan;
use crate::object::ObjectRef;

/// Cache key: the form and its caller's CTM in millipoints.
pub(super) type FormSpanKey = (ObjectRef, [i64; 6]);

impl<'doc> TextExtractor<'doc> {
    /// Neither read nor fill the document's Form XObject span cache.
    pub(crate) fn bypass_form_span_cache(&mut self) {
        self.form_span_cache_off = true;
    }

    /// Whether this extraction may use the form span cache. Cached spans
    /// carry the facts of the invocation that filled the cache (alpha, clip,
    /// OC are inherited from the caller at `Do`), so visibility facts bypass
    /// the cache like the emission filters do.
    pub(super) fn form_span_cache_on(&self) -> bool {
        self.extract_spans
            && !self.form_span_cache_off
            && self.excluded_layers.is_empty()
            && self.excluded_inks.is_empty()
            && self.vis.is_none()
    }

    /// Replay the cached spans of `key`; `false` when there is no entry.
    pub(super) fn replay_cached_form_spans(&mut self, key: &FormSpanKey) -> bool {
        let Some(doc) = self.document else {
            return false;
        };
        let cached = doc.xobject_spans_cache.lock().unwrap().get(key).cloned();
        let Some(cached) = cached else {
            return false;
        };
        if let Some(spans) = cached {
            self.spans.extend(spans.iter().cloned());
        }
        true
    }

    /// Store the spans emitted since `spans_before` under `key`.
    pub(super) fn store_form_spans(&self, key: FormSpanKey, spans_before: usize) {
        let Some(doc) = self.document else {
            return;
        };
        let new_spans: Option<Vec<TextSpan>> =
            (self.spans.len() > spans_before).then(|| self.spans[spans_before..].to_vec());
        doc.xobject_spans_cache
            .lock()
            .unwrap()
            .insert(key, new_spans);
    }
}
