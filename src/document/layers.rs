//! Layer-filtered extraction entry points of `PdfDocument`.
//!
//! The `HashSet<String>` methods exclude layers by `/Name` (the long-standing
//! API); the [`LayerFilter`] methods decide groups by reference, which is what
//! a document's own default configuration requires (see
//! [`crate::optional_content`]).

use std::collections::HashSet;

use super::{PdfDocument, ReadingOrder};
use crate::error::Result;
use crate::layout::TextSpan;
use crate::optional_content::LayerFilter;
use crate::structured::{ColumnMode, StructuredPage};

impl PdfDocument {
    /// Extract positioned spans in a reading order, excluding optional-content
    /// layers and/or Separation/DeviceN inks.
    ///
    /// This is [`extract_spans_with_reading_order`](Self::extract_spans_with_reading_order)
    /// and [`extract_text_filtered`](Self::extract_text_filtered) combined: the
    /// former cannot filter, and the latter returns assembled text rather than
    /// positioned spans. A consumer that lays spans out itself - an HTML/XML
    /// emitter placing each span at its own rectangle - needs both at once.
    ///
    /// The motivating case is render/extract parity. `render_page` honours the
    /// document's default configuration `/OCProperties/D`, but span extraction
    /// treats everything as visible unless the caller names layers (see the
    /// `optional_content` module note). Excluding the default-off layers makes
    /// extraction agree with what the page actually displays - without it, a
    /// default-OFF layer holding a copy of the page contributes a SECOND copy
    /// of every word. Layers given here are matched by `/Name`; to exclude the
    /// default-off layers use [`Self::extract_spans_with_layer_filter`] with
    /// [`LayerFilter::document_default`], which matches groups by reference.
    ///
    /// Empty sets are exactly equivalent to the unfiltered call, so this is a
    /// superset of the existing API and costs nothing when no filtering is asked
    /// for.
    ///
    /// # Arguments
    ///
    /// * `page_index` - Zero-based page index
    /// * `reading_order` - The reading order strategy to apply
    /// * `excluded_layers` - OCG layer names to suppress (empty = no filtering)
    /// * `excluded_inks` - Separation/DeviceN ink names to suppress (empty = none)
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use pdf_oxide::document::{PdfDocument, ReadingOrder};
    /// # use pdf_oxide::optional_content::LayerFilter;
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let doc = PdfDocument::open("layered.pdf")?;
    /// // Agree with what the page displays: drop the default-off layers.
    /// let hidden = LayerFilter::document_default(&doc);
    /// let spans = doc.extract_spans_with_layer_filter(0, ReadingOrder::ColumnAware, &hidden)?;
    /// for span in spans {
    ///     println!("{}", span.text);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn extract_spans_filtered_with_reading_order(
        &self,
        page_index: usize,
        reading_order: ReadingOrder,
        excluded_layers: HashSet<String>,
        excluded_inks: HashSet<String>,
    ) -> Result<Vec<TextSpan>> {
        let layers = LayerFilter::from_names(excluded_layers);
        self.spans_in_reading_order(page_index, reading_order, layers, excluded_inks)
    }

    /// Positioned spans in `reading_order` without the content of the
    /// optional content groups `layers` turns off (`BDC /OC` scopes and
    /// XObjects whose `/OC` is hidden). An empty filter is exactly
    /// [`Self::extract_spans_with_reading_order`].
    pub fn extract_spans_with_layer_filter(
        &self,
        page_index: usize,
        reading_order: ReadingOrder,
        layers: &LayerFilter,
    ) -> Result<Vec<TextSpan>> {
        self.spans_in_reading_order(page_index, reading_order, layers.clone(), HashSet::new())
    }

    /// [`extract_structured`](Self::extract_structured) with optional-content
    /// layers and/or Separation/DeviceN inks excluded — the structured
    /// counterpart of
    /// [`extract_spans_filtered_with_reading_order`](Self::extract_spans_filtered_with_reading_order).
    /// Layers are matched by `/Name`; to drop what the page does not display
    /// by default use [`Self::extract_structured_with_layer_filter`]. Empty
    /// sets are exactly [`extract_structured`](Self::extract_structured).
    pub fn extract_structured_filtered(
        &self,
        page_index: usize,
        excluded_layers: HashSet<String>,
        excluded_inks: HashSet<String>,
    ) -> Result<StructuredPage> {
        let layers = LayerFilter::from_names(excluded_layers);
        let spans = self.spans_in_reading_order(
            page_index,
            ReadingOrder::default(),
            layers,
            excluded_inks,
        )?;
        self.structured_from_spans(page_index, spans, ColumnMode::Auto)
    }

    /// [`extract_structured_with_column_mode`](Self::extract_structured_with_column_mode)
    /// without the content of the optional content groups `layers` turns off.
    /// Pass [`LayerFilter::document_default`] to drop what the page does not
    /// display by default. An empty filter is exactly
    /// `extract_structured_with_column_mode`.
    ///
    /// When the document's default configuration hides layers, every text
    /// extraction of it (filtered or not, `extract_words` included) parses
    /// large content streams in full, so a filtered and an unfiltered page
    /// differ only by the suppressed content: a span of one that is missing
    /// from the other belongs to a hidden layer.
    pub fn extract_structured_with_layer_filter(
        &self,
        page_index: usize,
        column_mode: ColumnMode,
        layers: &LayerFilter,
    ) -> Result<StructuredPage> {
        let order = ReadingOrder::default();
        let spans =
            self.spans_in_reading_order(page_index, order, layers.clone(), HashSet::new())?;
        self.structured_from_spans(page_index, spans, column_mode)
    }

    pub(super) fn structured_from_spans(
        &self,
        page_index: usize,
        spans: Vec<TextSpan>,
        column_mode: ColumnMode,
    ) -> Result<StructuredPage> {
        let media_box = self.get_page_media_box(page_index)?;
        let struct_info = self.structured_mcid_info(page_index);
        Ok(crate::structured::build_structured_page_full(
            page_index,
            media_box.2,
            media_box.3,
            spans,
            column_mode,
            &struct_info,
        ))
    }
}
