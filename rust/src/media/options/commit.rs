//! Bounded publication cadences over a streaming record reader.

use std::num::NonZeroUsize;

use arrow_array::RecordBatch;

use crate::arrow::{BatchReader, memory_size};

/// How a streamed write is cut into publications.
///
/// A cadence counts whole batches and never cuts one: a batch is the unit
/// the source yields, and the one shaping pass has already cut it to the
/// `batch_row_size` and `batch_byte_size` a caller asked for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Cadence {
    /// One publication after the source ends.
    Once,
    /// A publication every `n` batches, then the remainder.
    Batches(NonZeroUsize),
    /// A publication each time the held batches reach this many bytes as
    /// [`memory_size`] counts them, then the remainder. A non-zero target
    /// always yields at least one batch, so one enormous batch is still one
    /// cadence.
    Bytes(u64),
}

impl Cadence {
    /// Whether `batches` held batches of `bytes` complete one cadence.
    fn is_full(self, batches: usize, bytes: u64) -> bool {
        match self {
            Self::Once => false,
            Self::Batches(count) => batches >= count.get(),
            Self::Bytes(target) => bytes >= target,
        }
    }
}

/// The one streaming splitter for publication boundaries.
///
/// A bounded instance owns at most one complete cadence of whole batches. It
/// never pulls a following batch after the current cadence is full, which is
/// what makes a successful prefix observable before a later source failure.
/// A once-only instance yields the original reader once.
pub(crate) struct CommitReaders {
    pub(super) schema: arrow_schema::SchemaRef,
    pub(super) batches: Option<BatchReader>,
    pub(super) cadence: Cadence,
    pub(super) buffer: Option<CommitBuffer>,
    pub(super) done: bool,
}

/// One owned, bounded publication window of whole batches.
///
/// Both the ordinary pull reader and a runtime binding that pushes batches
/// between awaits use this object. It owns only the current cadence.
pub(crate) struct CommitBuffer {
    pub(super) schema: arrow_schema::SchemaRef,
    cadence: Cadence,
    batches: Vec<RecordBatch>,
    /// What the held batches occupy, as [`memory_size`] counts them.
    bytes: u64,
}

impl CommitBuffer {
    pub(crate) fn new(schema: arrow_schema::SchemaRef, cadence: Cadence) -> Self {
        Self {
            schema,
            cadence,
            batches: Vec::new(),
            bytes: 0,
        }
    }

    /// Add one batch and return the cadence it completes, if it does.
    ///
    /// An empty batch is skipped and counts for nothing. A batch is never
    /// cut, so the reader returned holds exactly the batches pushed since
    /// the last one.
    pub(crate) fn push(&mut self, batch: RecordBatch) -> Option<BatchReader> {
        if batch.num_rows() == 0 {
            return None;
        }
        if matches!(self.cadence, Cadence::Bytes(_)) {
            self.bytes = self
                .bytes
                .saturating_add(u64::try_from(memory_size(&batch)).unwrap_or(u64::MAX));
        }
        self.batches.push(batch);
        self.cadence
            .is_full(self.batches.len(), self.bytes)
            .then(|| self.take())
    }

    /// Take the successful final remainder, if this window holds one.
    pub(crate) fn finish(&mut self) -> Option<BatchReader> {
        (!self.batches.is_empty()).then(|| self.take())
    }

    /// Discard an incomplete cadence after a source or conversion failure.
    pub(crate) fn clear(&mut self) {
        self.bytes = 0;
        self.batches.clear();
    }

    fn take(&mut self) -> BatchReader {
        self.bytes = 0;
        crate::arrow::batch_reader(
            std::sync::Arc::clone(&self.schema),
            std::mem::take(&mut self.batches),
        )
    }
}

impl Iterator for CommitReaders {
    type Item = crate::Result<BatchReader>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        if self.cadence == Cadence::Once {
            self.done = true;
            return self.batches.take().map(Ok);
        }
        loop {
            let batches = self
                .batches
                .as_mut()
                .expect("a live commit splitter owns its source");
            match batches.next() {
                Some(Ok(batch)) => {
                    let buffer = self.buffer.get_or_insert_with(|| {
                        CommitBuffer::new(std::sync::Arc::clone(&self.schema), self.cadence)
                    });
                    if let Some(reader) = buffer.push(batch) {
                        return Some(Ok(reader));
                    }
                }
                Some(Err(error)) => {
                    // The partial cadence has not been published. Previous
                    // complete cadences remain visible by design.
                    self.done = true;
                    if let Some(buffer) = &mut self.buffer {
                        buffer.clear();
                    }
                    return Some(Err(crate::arrow::from_reader_error(error).into()));
                }
                None => {
                    self.done = true;
                    return self.buffer.as_mut().and_then(CommitBuffer::finish).map(Ok);
                }
            }
        }
    }
}
