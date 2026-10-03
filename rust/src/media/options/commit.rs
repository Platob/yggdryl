//! Bounded publication cadences over a streaming record reader.

use std::num::NonZeroUsize;

use arrow_array::RecordBatch;

use crate::arrow::{BatchReader, memory_size};
use crate::spill::{SpillOptions, spill_batch};

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
/// between awaits use this object. It owns only the current cadence, and
/// holds it under the process spill bound
/// ([`SpillOptions::from_env`]): once the resident bytes of the held
/// batches pass the bound, the heaviest resident batch is written to one
/// spill file and held over its mapping ([`spill_batch`]), heaviest first,
/// until what is resident is under the bound again. A batch is transport
/// here - it is moved to the publication, never read - so its buffers are
/// spilled as they are and no `Serie` claim is made. The cadence itself
/// keeps counting the batches' logical bytes, spilled or not, so what one
/// publication holds does not change with where it is held.
pub(crate) struct CommitBuffer {
    pub(super) schema: arrow_schema::SchemaRef,
    cadence: Cadence,
    batches: Vec<RecordBatch>,
    /// What the held batches occupy, as [`memory_size`] counts them.
    bytes: u64,
    /// The bytes of the held batches that are still on the heap: the
    /// [`memory_size`] of every batch not spilled, what the spill bound
    /// reads.
    resident: u64,
    /// Whether each held batch lies in a spill file.
    spilled: Vec<bool>,
}

impl CommitBuffer {
    pub(crate) fn new(schema: arrow_schema::SchemaRef, cadence: Cadence) -> Self {
        Self {
            schema,
            cadence,
            batches: Vec::new(),
            bytes: 0,
            resident: 0,
            spilled: Vec::new(),
        }
    }

    /// Add one batch and return the cadence it completes, if it does.
    ///
    /// An empty batch is skipped and counts for nothing. A batch is never
    /// cut, so the reader returned holds exactly the batches pushed since
    /// the last one. A batch kept past the cadence is held under the
    /// process spill bound.
    ///
    /// # Errors
    ///
    /// Returns [`spill_batch`]'s refusal of the spill folder; the batch is
    /// held resident and the cadence counts it.
    pub(crate) fn push(&mut self, batch: RecordBatch) -> crate::Result<Option<BatchReader>> {
        if batch.num_rows() == 0 {
            return Ok(None);
        }
        let size = u64::try_from(memory_size(&batch)).unwrap_or(u64::MAX);
        self.bytes = self.bytes.saturating_add(size);
        self.resident = self.resident.saturating_add(size);
        self.batches.push(batch);
        self.spilled.push(false);
        if self.cadence.is_full(self.batches.len(), self.bytes) {
            return Ok(Some(self.take()));
        }
        self.settle()?;
        Ok(None)
    }

    /// Spill the heaviest resident batches until the resident bytes are
    /// under the process bound.
    fn settle(&mut self) -> crate::Result<()> {
        let options = SpillOptions::from_env()?;
        if options.is_never() {
            return Ok(());
        }
        let bound = options.byte_size();
        while self.resident > bound {
            let heaviest = self
                .batches
                .iter()
                .zip(&self.spilled)
                .enumerate()
                .filter(|(_, (_, spilled))| !**spilled)
                .max_by_key(|(_, (batch, _))| memory_size(batch))
                .map(|(index, _)| index);
            let Some(index) = heaviest else {
                return Ok(());
            };
            let size = u64::try_from(memory_size(&self.batches[index])).unwrap_or(u64::MAX);
            self.spilled[index] = true;
            self.resident = self.resident.saturating_sub(size);
            if let Some((rebuilt, _)) = spill_batch(&self.batches[index], options.folder())? {
                self.batches[index] = rebuilt;
            }
        }
        Ok(())
    }

    /// The bytes of the held batches still on the heap, and how many of
    /// the held batches lie in a spill file.
    #[cfg(feature = "internals")]
    pub(crate) fn residency(&self) -> (u64, usize) {
        (
            self.resident,
            self.spilled.iter().filter(|spilled| **spilled).count(),
        )
    }

    /// Take the successful final remainder, if this window holds one.
    pub(crate) fn finish(&mut self) -> Option<BatchReader> {
        (!self.batches.is_empty()).then(|| self.take())
    }

    /// Discard an incomplete cadence after a source or conversion failure.
    pub(crate) fn clear(&mut self) {
        self.bytes = 0;
        self.resident = 0;
        self.batches.clear();
        self.spilled.clear();
    }

    fn take(&mut self) -> BatchReader {
        self.bytes = 0;
        self.resident = 0;
        self.spilled.clear();
        crate::arrow::batch_reader(
            std::sync::Arc::clone(&self.schema),
            std::mem::take(&mut self.batches),
        )
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/spill_doors.rs` pins and a caller cannot reach: the
    //! residency of a publication window held past the process spill bound.

    use arrow_array::RecordBatch;

    /// Push `batches` into one window publishing every `cadence` batches and
    /// answer the bytes of the held batches still on the heap and how many
    /// of them lie in a spill file; a cadence the pushes complete is taken
    /// and counts for nothing.
    ///
    /// # Errors
    ///
    /// The window's own refusal of the spill folder.
    pub fn residency_after(
        schema: arrow_schema::SchemaRef,
        batches: Vec<RecordBatch>,
        cadence: usize,
    ) -> crate::Result<(u64, usize)> {
        let cadence = std::num::NonZeroUsize::new(cadence).expect("a non-zero cadence");
        let mut buffer = super::CommitBuffer::new(schema, super::Cadence::Batches(cadence));
        for batch in batches {
            let _ = buffer.push(batch)?;
        }
        Ok(buffer.residency())
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
                    match buffer.push(batch) {
                        Ok(Some(reader)) => return Some(Ok(reader)),
                        Ok(None) => {}
                        Err(error) => {
                            self.done = true;
                            buffer.clear();
                            return Some(Err(error));
                        }
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
