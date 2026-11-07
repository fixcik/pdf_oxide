//! Visibility-facts hooks of the text extractor: per-glyph size statistics
//! of a show-text run and span emission with facts. Facts off: one
//! `vis.is_none()` test, then the unchanged paths.

use super::{TextExtractor, TjBuffer};
use crate::error::Result;
use crate::extractors::visibility::GlyphStats;
use crate::layout::TextSpan;

impl TjBuffer {
    /// Record the glyphs appended since `(byte_from, width_from)` into
    /// `glyphs` (visibility facts only). `char_space` is the `Tc` share of
    /// each width (text space), removed so the own glyph advance remains.
    pub(super) fn note_glyphs(&mut self, byte_from: usize, width_from: usize, char_space: f32) {
        let (Some(text), Some(widths)) =
            (self.unicode.get(byte_from..), self.char_widths.get(width_from..))
        else {
            return;
        };
        for (c, w) in text.chars().zip(widths) {
            if !c.is_whitespace() {
                self.glyphs
                    .note(self.effective_font_size, (w - char_space) * self.user_h_scale);
            }
        }
    }
}

impl<'doc> TextExtractor<'doc> {
    /// Append an emitted span, stamping its visibility facts when enabled.
    pub(super) fn push_span(&mut self, span: TextSpan) {
        self.push_span_with_glyphs(span, None);
    }

    /// [`Self::push_span`] with the run's per-glyph statistics; `None` takes
    /// them from the span itself (its `char_widths`, `Tc` included).
    pub(super) fn push_span_with_glyphs(&mut self, mut span: TextSpan, glyphs: Option<GlyphStats>) {
        if let Some(vis) = self.vis.as_mut() {
            let render_mode = self.state_stack.current().render_mode;
            let glyphs = glyphs.unwrap_or_else(|| {
                let mut g = GlyphStats::default();
                for (c, w) in span.text.chars().zip(&span.char_widths) {
                    if !c.is_whitespace() {
                        g.note(span.font_size, *w);
                    }
                }
                g
            });
            span.visibility =
                Some(vis.span_facts(render_mode, &span.bbox, span.color, span.font_size, glyphs));
        }
        self.spans.push(span);
    }

    /// `Tc` share of one glyph width in text space (what the append paths
    /// add per glyph): `Tc × Th` horizontally, `Tc` in vertical mode.
    pub(super) fn char_space_share(&self) -> f32 {
        let state = self.state_stack.current();
        if state.text_wmode == 0 {
            state.char_space * state.horizontal_scaling / 100.0
        } else {
            state.char_space
        }
    }

    /// [`Self::append_and_advance_impl`], recording per-glyph size facts
    /// when visibility facts are on.
    #[inline]
    pub(super) fn append_and_advance(&mut self, text: &[u8]) -> Result<()> {
        if self.vis.is_none() {
            return self.append_and_advance_impl(text);
        }
        let cs = self.char_space_share();
        let mark = self
            .tj_span_buffer
            .as_ref()
            .map(|b| (b.unicode.len(), b.char_widths.len()));
        self.append_and_advance_impl(text)?;
        if let (Some(b), Some((u, w))) = (self.tj_span_buffer.as_mut(), mark) {
            b.note_glyphs(u, w, cs);
        }
        Ok(())
    }

    /// [`Self::append_advance_buffer_impl`], recording per-glyph size facts
    /// when visibility facts are on.
    #[inline]
    pub(super) fn append_advance_buffer(
        &mut self,
        buffer: &mut TjBuffer,
        text: &[u8],
    ) -> Result<()> {
        if self.vis.is_none() {
            return self.append_advance_buffer_impl(buffer, text);
        }
        let cs = self.char_space_share();
        let (u, w) = (buffer.unicode.len(), buffer.char_widths.len());
        self.append_advance_buffer_impl(buffer, text)?;
        buffer.note_glyphs(u, w, cs);
        Ok(())
    }
}
