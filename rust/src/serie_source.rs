//! [`SerieSource`]: the one intake every verb that reads rows from a caller
//! takes - a held column, a held chunked column, or a stream - so a record
//! write, a join side and every door after them spell one input type.

use crate::{ChunkedSerie, Result, Serie, SerieReader};

/// Rows in any of the three shapes the crate holds them: one held column, a
/// chunked one, or a stream.
///
/// `From` is implemented for each shape, so a caller writes
/// `handle.append_serie(serie.into(), None)` and names no variant. A held
/// shape is read as the batches it already is - a record column one batch,
/// each chunk one batch - and a stream as itself; nothing is copied or
/// re-landed by the crossing ([`Self::into_reader`]).
#[derive(Debug)]
pub enum SerieSource {
    /// One held column.
    Serie(Serie),
    /// Held chunks under one field.
    Chunked(ChunkedSerie),
    /// A stream, read one batch at a time.
    Reader(SerieReader),
}

impl SerieSource {
    /// The record root the rows are read under: a record column's own, any
    /// other column the one child of a `row` record ([`SerieReader::root_of`]),
    /// a stream's own root.
    ///
    /// # Errors
    ///
    /// Returns an error for a run, which names no column.
    pub fn root(&self) -> Result<crate::Field> {
        match self {
            Self::Serie(serie) => SerieReader::root_of(serie.require_field()?),
            Self::Chunked(chunked) => SerieReader::root_of(chunked.field()),
            Self::Reader(reader) => Ok(reader.field().clone()),
        }
    }

    /// Whether the rows are held rather than streamed.
    #[must_use]
    pub const fn is_held(&self) -> bool {
        !matches!(self, Self::Reader(_))
    }

    /// The bytes a held source occupies; `None` for a stream.
    #[must_use]
    pub fn memory_size(&self) -> Option<usize> {
        match self {
            Self::Serie(serie) => Some(serie.memory_size()),
            Self::Chunked(chunked) => Some(chunked.memory_size()),
            Self::Reader(_) => None,
        }
    }

    /// The order the root declares the rows keep, proven: a held column's
    /// or chunked column's own, a stream's root's - its batches are verified
    /// in that order as they are pulled - and `None` where none is declared.
    pub(crate) fn declared_order(&self) -> Result<Option<Vec<crate::expression::Ordering>>> {
        match self {
            Self::Serie(serie) => serie.declared_order(),
            Self::Chunked(chunked) => chunked.declared_order(),
            Self::Reader(reader) => {
                let root = reader.field();
                if root.dtype().as_fields().is_none() || !root.as_sort().declares_order() {
                    return Ok(None);
                }
                root.as_sort().by()
            }
        }
    }

    /// The rows as the stream of their record batches: a held column one
    /// record batch ([`SerieReader::from_serie`]), chunks one each
    /// ([`SerieReader::from_chunked`]), a stream itself. Nothing is cast,
    /// copied or read.
    ///
    /// # Errors
    ///
    /// Returns an error for a run, which names no layout, and for a record
    /// column holding an absent row, which a table cannot state.
    pub fn into_reader(self) -> Result<SerieReader> {
        Ok(match self {
            Self::Serie(serie) => SerieReader::from_serie(serie)?,
            Self::Chunked(chunked) => SerieReader::from_chunked(chunked)?,
            Self::Reader(reader) => reader,
        })
    }
}

impl From<Serie> for SerieSource {
    fn from(serie: Serie) -> Self {
        Self::Serie(serie)
    }
}

impl From<ChunkedSerie> for SerieSource {
    fn from(chunked: ChunkedSerie) -> Self {
        Self::Chunked(chunked)
    }
}

impl From<SerieReader> for SerieSource {
    fn from(reader: SerieReader) -> Self {
        Self::Reader(reader)
    }
}
