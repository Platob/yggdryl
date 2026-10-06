//! Pull-driven conversions and one bounded chunker for every serie kind.

use std::sync::Arc;

use crate::arrow::Result;
use crate::{
    ChunkedSerie, Field, KeySerie, KeySeries, Serie, SpillOptions, StreamChunkedSerie,
    StreamKeySerie, StreamSerie, WindowSerie,
};

impl Serie {
    /// Read global record rows lazily, with no chunk join.
    ///
    /// # Errors
    /// Refuses a run or an absent held record row before pulling.
    pub fn into_stream(self) -> crate::Result<StreamSerie> {
        if let Some(media) = self.media_state() {
            return media.clone().into_rows()?.into_stream();
        }
        match self {
            Self::Stream(stream) => crate::SharedStream::into_stream(stream)?.into_stream(),
            Self::Key(key) => Arc::unwrap_or_clone(key).into_stream(),
            Self::Keys(keys) => Arc::unwrap_or_clone(keys).into_stream(),
            Self::StreamKey(stream) => crate::SharedStream::into_stream(stream)?.into_stream(),
            rows => StreamChunkedSerie::from_serie(rows)?.into_stream(),
        }
    }

    /// Rechunk lazily by rows, bytes, or whichever bound is reached first.
    /// Both absent uses 64 MiB capped at the process spill bound. Bounds
    /// admit at least one row; an arriving chunk that alone reaches a bound
    /// passes through unchanged. Key items are always chunked separately.
    ///
    /// # Errors
    /// Refuses a run, absent held record rows, an invalid spill default or
    /// an unrepresentable Arrow field before pulling. Source errors follow
    /// their completed prefix, then fuse the stream.
    pub fn into_chunked_stream(
        self,
        row_size: Option<usize>,
        byte_size: Option<u64>,
    ) -> Result<StreamChunkedSerie> {
        if let Some(media) = self.media_state() {
            return media
                .clone()
                .into_rows()?
                .into_chunked_stream(row_size, byte_size);
        }
        match self {
            Self::Key(key) => Arc::unwrap_or_clone(key).into_chunked_stream(row_size, byte_size),
            Self::Keys(keys) => Arc::unwrap_or_clone(keys).into_chunked_stream(row_size, byte_size),
            Self::StreamKey(stream) => {
                crate::SharedStream::into_stream(stream)?.into_chunked_stream(row_size, byte_size)
            }
            Self::Stream(stream) => {
                crate::SharedStream::into_stream(stream)?.into_chunked_stream(row_size, byte_size)
            }
            rows => StreamChunkedSerie::from_serie(rows)?.into_chunked_stream(row_size, byte_size),
        }
    }
}

impl StreamChunkedSerie {
    /// Read rows from each landed chunk, retaining only that chunk.
    ///
    /// # Errors
    /// The stream's Arrow field refusal; source failures arrive as row items.
    pub fn into_stream(self) -> crate::Result<StreamSerie> {
        let field = self.field().clone();
        let rows = self.into_chunks().flat_map(|piece| {
            let (piece, error) = match piece {
                Ok(piece) => (Some(piece), None),
                Err(error) => (None, Some(Err(crate::Error::from(error)))),
            };
            let length = piece.as_ref().map_or(0, Serie::len);
            error.into_iter().chain(
                (0..length).map(move |row| piece.as_ref().expect("a landed chunk").scalar(row)),
            )
        });
        Ok(StreamSerie::from_proven_rows(field, rows))
    }

    /// Apply the bounds [`Serie::into_chunked_stream`] states.
    ///
    /// Pieces are sliced without copying; a pending output joins only its
    /// pieces. The transient peak is those pieces plus the joined output.
    /// No source chunk is pulled past a completed bound.
    ///
    /// # Errors
    /// An invalid spill default or an unrepresentable Arrow field, then
    /// source and layout failures while pulling.
    pub fn into_chunked_stream(
        self,
        row_size: Option<usize>,
        byte_size: Option<u64>,
    ) -> Result<Self> {
        let bounds = Bounds::new(row_size, byte_size)?;
        let root = Arc::new(self.field().clone());
        let chunks = Rechunk {
            source: self.into_chunks(),
            root: Arc::clone(&root),
            bounds,
            tail: None,
            pending_error: None,
            ended: false,
        };
        Self::from_landed_iter(root, chunks)
    }
}

impl ChunkedSerie {
    /// Read this column's rows as a lazy record stream.
    ///
    /// # Errors
    /// [`Serie::into_stream`]'s refusals.
    pub fn into_stream(self) -> crate::Result<StreamSerie> {
        Serie::from(self).into_stream()
    }
    /// Stream this column under the common row and byte bounds.
    ///
    /// # Errors
    /// [`Serie::into_chunked_stream`]'s refusals.
    pub fn into_chunked_stream(
        self,
        row_size: Option<usize>,
        byte_size: Option<u64>,
    ) -> Result<StreamChunkedSerie> {
        Serie::from(self).into_chunked_stream(row_size, byte_size)
    }
}

impl StreamSerie {
    /// This row stream itself, after checking its record field.
    ///
    /// # Errors
    /// Refuses a field that is not a required record.
    pub fn into_stream(mut self) -> crate::Result<Self> {
        self.require_record_field()?;
        self.prepare_order()?;
        Ok(self)
    }
    /// Lay out rows under the common bounds. With both absent, use the
    /// default byte bound; a row ceiling applies only when stated.
    ///
    /// # Errors
    /// Field and spill-default refusals before pulling, then row failures.
    pub fn into_chunked_stream(
        self,
        row_size: Option<usize>,
        byte_size: Option<u64>,
    ) -> Result<StreamChunkedSerie> {
        self.require_record_field()?;
        let bounds = Bounds::new(row_size, byte_size)?;
        let rows = bounds.rows.unwrap_or(usize::MAX);
        let field = self.field().clone();
        let batches = self.bounded_reader(Some(rows), bounds.bytes)?;
        StreamChunkedSerie::from_arrow_reader(Some(&field), batches, crate::ArrowCastOptions::new())
    }
}

impl KeySerie {
    /// Read this item's global rows lazily.
    ///
    /// # Errors
    /// [`Serie::into_stream`]'s refusals.
    pub fn into_stream(self) -> crate::Result<StreamSerie> {
        let field = StreamChunkedSerie::root_of(self.field())?;
        let (key, rows) = self.into_parts();
        let rows = rows.into_stream()?;
        let cells = key
            .sequence_rows()
            .expect("keys are record runs")
            .into_owned();
        Ok(StreamSerie::from_proven_rows(
            field,
            rows.map(move |row| {
                let row = row?;
                let values = row.sequence_rows().expect("canonical record rows");
                Ok(crate::Scalar::from_sequence(
                    cells.iter().chain(values.iter()).cloned(),
                ))
            }),
        ))
    }
    /// Bound global rows, including their constant key columns. Literals
    /// share one allocation per longest chunk and grow geometrically for
    /// a lazy payload.
    ///
    /// # Errors
    /// [`Serie::into_chunked_stream`]'s refusals.
    pub fn into_chunked_stream(
        self,
        row_size: Option<usize>,
        byte_size: Option<u64>,
    ) -> Result<StreamChunkedSerie> {
        self.bounded_record_stream(row_size, byte_size)
    }
}

impl KeySeries {
    /// Read global rows in item order.
    ///
    /// # Errors
    /// [`Serie::into_stream`]'s refusals.
    pub fn into_stream(self) -> crate::Result<StreamSerie> {
        self.require_table_rows()?;
        StreamKeySerie::new(Arc::clone(&self.layout), self.items.into_iter().map(Ok)).into_stream()
    }
    /// Bound each item separately, never joining across items.
    ///
    /// # Errors
    /// [`Serie::into_chunked_stream`]'s refusals.
    pub fn into_chunked_stream(
        self,
        row_size: Option<usize>,
        byte_size: Option<u64>,
    ) -> Result<StreamChunkedSerie> {
        self.require_table_rows()?;
        StreamKeySerie::new(Arc::clone(&self.layout), self.items.into_iter().map(Ok))
            .into_chunked_stream(row_size, byte_size)
    }
}

impl StreamKeySerie {
    /// Read global rows in item order, with at most the current item open.
    ///
    /// # Errors
    /// [`Serie::into_stream`]'s refusals.
    pub fn into_stream(mut self) -> crate::Result<StreamSerie> {
        let field = StreamChunkedSerie::root_of(self.field())?;
        let mut current: Option<StreamSerie> = None;
        Ok(StreamSerie::from_proven_rows(
            field,
            std::iter::from_fn(move || {
                loop {
                    if let Some(rows) = &mut current
                        && let Some(row) = rows.next()
                    {
                        return Some(row);
                    }
                    current = None;
                    match self.next()? {
                        Ok(item) => match item.into_stream() {
                            Ok(rows) => current = Some(rows),
                            Err(error) => return Some(Err(error)),
                        },
                        Err(error) => return Some(Err(crate::Error::from(error))),
                    }
                }
            }),
        ))
    }
    /// Bound each lazy item separately, never joining across keys.
    ///
    /// # Errors
    /// [`Serie::into_chunked_stream`]'s refusals.
    pub fn into_chunked_stream(
        self,
        row_size: Option<usize>,
        byte_size: Option<u64>,
    ) -> Result<StreamChunkedSerie> {
        Bounds::new(row_size, byte_size)?;
        self.record_stream_with_bounds(Some((row_size, byte_size)))
    }
}

impl WindowSerie<'_> {
    /// Read this view's rows lazily.
    ///
    /// # Errors
    /// [`Serie::into_stream`]'s refusals.
    pub fn into_stream(self) -> crate::Result<StreamSerie> {
        self.into_serie().into_stream()
    }
    /// Stream this view under the common bounds.
    ///
    /// # Errors
    /// [`Serie::into_chunked_stream`]'s refusals.
    pub fn into_chunked_stream(
        self,
        row_size: Option<usize>,
        byte_size: Option<u64>,
    ) -> Result<StreamChunkedSerie> {
        self.into_serie().into_chunked_stream(row_size, byte_size)
    }
}

#[derive(Clone, Copy)]
struct Bounds {
    rows: Option<usize>,
    bytes: Option<u64>,
}
impl Bounds {
    fn new(rows: Option<usize>, bytes: Option<u64>) -> crate::Result<Self> {
        let bytes = if rows.is_none() && bytes.is_none() {
            Some(crate::media::DEFAULT_COMMIT_BYTE_SIZE.min(SpillOptions::from_env()?.byte_size()))
        } else {
            bytes
        };
        Ok(Self {
            rows: rows.map(|rows| rows.max(1)),
            bytes,
        })
    }
    fn reached(self, rows: usize, bytes: usize) -> bool {
        self.rows.is_some_and(|bound| rows >= bound)
            || self.bytes.is_some_and(|bound| bytes as u64 >= bound)
    }
}

struct Rechunk {
    source: super::IntoStreamChunks,
    root: Arc<Field>,
    bounds: Bounds,
    tail: Option<Serie>,
    pending_error: Option<crate::arrow::Error>,
    ended: bool,
}

impl Iterator for Rechunk {
    type Item = Result<Serie>;
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(error) = self.pending_error.take() {
            return Some(Err(error));
        }
        if self.ended {
            return None;
        }
        let mut pieces = Vec::new();
        let mut rows = 0;
        let mut bytes: usize = 0;
        loop {
            let next = self.tail.take().map(Ok).or_else(|| self.source.next());
            let piece = match next {
                Some(Ok(piece)) => piece,
                Some(Err(error)) => {
                    self.ended = true;
                    if pieces.is_empty() {
                        return Some(Err(error));
                    }
                    self.pending_error = Some(error);
                    break;
                }
                None => {
                    self.ended = true;
                    if pieces.is_empty() {
                        return None;
                    }
                    break;
                }
            };
            if piece.is_empty() {
                continue;
            }
            let size = piece.memory_size();
            if pieces.is_empty() && self.bounds.reached(piece.len(), size) {
                return Some(Ok(piece));
            }
            let mut count = self.bounds.rows.map_or(piece.len(), |bound| {
                bound.saturating_sub(rows).min(piece.len())
            });
            if let Some(bound) = self.bounds.bytes
                && bytes.saturating_add(size) as u64 > bound
            {
                let allowance = bound.saturating_sub(bytes as u64);
                let mut low = 0;
                let mut high = count;
                while low < high {
                    let middle = low + (high - low).div_ceil(2);
                    match piece.slice(0, middle) {
                        Ok(head) if head.memory_size() as u64 <= allowance => low = middle,
                        Ok(_) => high = middle - 1,
                        Err(error) => {
                            self.ended = true;
                            return Some(Err(error.into()));
                        }
                    }
                }
                count = low.max(1);
            }
            let (head, tail) = if count < piece.len() {
                match (
                    piece.slice(0, count),
                    piece.slice(count, piece.len() - count),
                ) {
                    (Ok(head), Ok(tail)) => (head, Some(tail)),
                    (Err(error), _) | (_, Err(error)) => {
                        self.ended = true;
                        return Some(Err(error.into()));
                    }
                }
            } else {
                (piece, None)
            };
            rows += head.len();
            bytes += head.memory_size();
            pieces.push(head);
            self.tail = tail;
            if self.tail.is_some() || self.bounds.reached(rows, bytes) {
                break;
            }
        }
        Some(ChunkedSerie::from_landed(Arc::clone(&self.root), pieces).joined())
    }
}
impl std::iter::FusedIterator for Rechunk {}
