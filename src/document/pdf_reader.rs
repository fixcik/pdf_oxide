//! The document's shared byte reader.
//!
//! One `PdfReader` sits behind `PdfDocument::reader` and is shared by every
//! thread working on the document. Its position is therefore only meaningful
//! inside a single lock guard: a reader must seek (or check its position with
//! [`Seek::stream_position`]) after taking the guard, never rely on where an
//! earlier guard left it.

use std::io::{BufRead, BufReader, Cursor, Read, Seek, SeekFrom};

/// In-memory reader used by `open()` and `from_bytes()`. Wrapping in an enum
/// is kept (rather than using `BufReader<Cursor<Vec<u8>>>` directly) so a
/// future file-backed variant can be re-introduced without touching call
/// sites.
pub(super) enum PdfReader {
    Memory(BufReader<Cursor<Vec<u8>>>),
}

impl Read for PdfReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            PdfReader::Memory(r) => r.read(buf),
        }
    }
}

impl Seek for PdfReader {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        match self {
            PdfReader::Memory(r) => r.seek(pos),
        }
    }

    /// Delegates to `BufReader::stream_position`, which keeps the buffer
    /// (the default would go through `seek(Current(0))` and drop it).
    fn stream_position(&mut self) -> std::io::Result<u64> {
        match self {
            PdfReader::Memory(r) => r.stream_position(),
        }
    }
}

impl BufRead for PdfReader {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        match self {
            PdfReader::Memory(r) => r.fill_buf(),
        }
    }

    fn consume(&mut self, amt: usize) {
        match self {
            PdfReader::Memory(r) => r.consume(amt),
        }
    }
}

/// Moves `reader` to `pos` unless it is already there (a seek would drop the
/// read buffer even when the position does not change).
pub(super) fn seek_to(reader: &mut PdfReader, pos: u64) -> std::io::Result<()> {
    if reader.stream_position()? != pos {
        reader.seek(SeekFrom::Start(pos))?;
    }
    Ok(())
}
