//! Raw spans of the first pages, kept from the running header/footer scan.
//!
//! The scan extracts every page with the Form XObject span cache bypassed
//! (`extract_spans_for_scan`), and the caller's own pass then extracts the
//! same pages again through `extract_spans_raw`. When the whole document
//! fits the page caches ([`page_cache_entries`] pages) the scan keeps each
//! page's extraction as a seed that the page's next `extract_spans_raw` miss
//! takes instead of extracting again. Longer documents keep nothing: their
//! seeds would sit next to the caller's pages in flight and raise its memory
//! peak, for a small share of the pages.
//!
//! A seed is kept only when it is what that miss would compute:
//! - same extraction: both are `extract_spans_impl` with the default config,
//!   no layer or ink filter, under the same visibility-facts setting (the
//!   seeds are dropped with the span caches when it changes), and the only
//!   difference, the form span cache bypass, made no difference: no Form
//!   XObject reached a span-cache lookup that only the bypass kept closed
//!   (`TextExtractor::form_span_cache_skipped`), so the miss would neither
//!   have replayed nor filled that cache;
//! - same writes into the document: an extraction that gets past the
//!   no-text early returns replaces the page's ActualText MCID set and, with
//!   visibility facts on, stores the page's unfiltered paints. The seed
//!   carries copies and the miss that takes it writes them again, as an
//!   extraction would at that moment.
//!
//! Assumed, as everywhere a page is extracted more than once: the other
//! document caches an extraction goes through (fonts, XObject streams and
//! subtypes, objects) return the same thing whichever page filled them, so
//! extracting a page earlier does not change its spans.
//!
//! A seed lives until its page's next raw extraction, `forget_page`, a
//! span-cache reset (`set_visibility_facts`, erase regions) or
//! `authenticate`: at most [`page_cache_entries`] pages, only for a document
//! that short.

use super::*;

/// What one `extract_spans_traced` call wrote into the document besides its
/// spans (`trace` on).
#[derive(Default)]
pub(super) struct RawSpansTrace {
    /// Got past the early returns, so the MCID set (and the paints) below
    /// were written.
    pub(super) wrote: bool,
    pub(super) mcids: HashSet<u32>,
    /// The unfiltered paints, with visibility facts on (shared with
    /// `raw_paints`).
    pub(super) paints: Option<Arc<Vec<crate::extractors::visibility::PaintEvent>>>,
    /// A Form XObject span-cache lookup was skipped only for the bypass.
    pub(super) form_span_cache_skipped: bool,
}

/// A page's raw spans from the scan, with the writes to replay.
pub(super) struct ScanSeed {
    spans: Vec<crate::layout::TextSpan>,
    trace: RawSpansTrace,
}

impl PdfDocument {
    /// Raw spans for the running header/footer scan. The scan walks pages in
    /// its own order (in parallel when the caller is not a rayon worker or
    /// asks for it, `scan_running_artifacts_on_current_pool`), so it keeps
    /// out of the Form XObject span cache, whose entries carry the page that
    /// filled them: the caller's own extraction order alone fills it. Calls
    /// [`Self::seed_raw_spans`] with the extraction of a page that may be
    /// seeded (`seed`: the document fits the page caches; see the module
    /// docs).
    pub(super) fn extract_spans_for_scan(
        &self,
        page_index: usize,
        seed: bool,
        summarize: impl FnOnce(&[crate::layout::TextSpan]) -> Option<ScanPage>,
    ) -> Option<ScanPage> {
        let config = crate::extractors::TextExtractionConfig::default();
        let (spans, trace) = self
            .extract_spans_traced(
                page_index,
                config,
                Default::default(),
                HashSet::new(),
                true,
                seed,
            )
            .ok()?;
        let page = summarize(&spans);
        // Not seeded: a page whose raw spans are cached already (the page
        // that started a lazy scan) would never take its seed, and a page of
        // a document not readable yet (no password) is extracted empty.
        if seed
            && !trace.form_span_cache_skipped
            && !self.is_encrypted_unreadable()
            && !self
                .page_raw_spans_cache
                .lock_or_recover()
                .contains_key(&page_index)
        {
            self.seed_raw_spans(page_index, ScanSeed { spans, trace });
        }
        page
    }

    fn seed_raw_spans(&self, page_index: usize, seed: ScanSeed) {
        self.raw_spans_seeds
            .lock_or_recover()
            .insert(page_index, seed);
    }

    /// The seeded raw spans of `page_index`, if any, with the seed's writes
    /// replayed as the extraction they stand for would make them now.
    pub(super) fn take_seeded_raw_spans(
        &self,
        page_index: usize,
    ) -> Option<Vec<crate::layout::TextSpan>> {
        let ScanSeed { spans, trace } =
            self.raw_spans_seeds.lock_or_recover().remove(&page_index)?;
        if trace.wrote {
            if let Some(paints) = trace.paints {
                self.raw_paints
                    .lock_or_recover()
                    .insert((page_index, false), paints);
            }
            let mut guard = self.mc_actualtext_mcids.lock_or_recover();
            if trace.mcids.is_empty() {
                guard.remove(&page_index);
            } else {
                guard.insert(page_index, trace.mcids);
            }
        }
        Some(spans)
    }

    /// Drops every seed (with the span caches) or the seed of one page.
    pub(super) fn drop_raw_spans_seeds(&self, page_index: Option<usize>) {
        let mut seeds = self.raw_spans_seeds.lock_or_recover();
        match page_index {
            Some(p) => {
                seeds.remove(&p);
            },
            None => seeds.clear(),
        }
    }
}
