//! Bounded publication windows over a streaming record reader.

/// The one streaming splitter for row-count publication boundaries.
///
/// A bounded instance owns at most one complete cadence plus the unconsumed
/// remainder of the current input batch. Batch slices are Arrow views over the
/// same buffers. It never pulls a following batch after the current cadence is
/// full, which is what makes a successful prefix observable before a later
/// source failure. An unbounded instance yields the original reader once.
pub(crate) struct CommitReaders {
    pub(super) schema: arrow_schema::SchemaRef,
    pub(super) batches: Option<crate::arrow::BatchReader>,
    pub(super) commit_row_size: Option<usize>,
    pub(super) buffer: Option<CommitBuffer>,
    pub(super) done: bool,
}

/// One owned, bounded publication window.
///
/// Both the ordinary pull reader and a runtime binding that pushes batches
/// between awaits use this object. It owns only the current cadence; slices are
/// Arrow views over their source buffers.
pub(crate) struct CommitBuffer {
    pub(super) schema: arrow_schema::SchemaRef,
    row_size: usize,
    rows: usize,
    batches: Vec<arrow_array::RecordBatch>,
    /// The one input batch whose leading rows completed the last cadence.
    /// Its remaining rows are not sliced until the caller asks for the next
    /// cadence, after it has had a chance to publish the one just returned.
    current: Option<(arrow_array::RecordBatch, usize)>,
}

impl CommitBuffer {
    pub(crate) fn new(schema: arrow_schema::SchemaRef, row_size: usize) -> Self {
        debug_assert!(row_size > 0);
        Self {
            schema,
            row_size,
            rows: 0,
            batches: Vec::new(),
            current: None,
        }
    }

    /// Add one batch and return at most the first cadence it completes.
    ///
    /// A remainder stays in `current`; callers must publish the returned
    /// reader before asking [`next_ready`](Self::next_ready) to advance it.
    pub(crate) fn push(
        &mut self,
        batch: arrow_array::RecordBatch,
    ) -> Option<crate::arrow::BatchReader> {
        debug_assert!(self.current.is_none());
        self.current = Some((batch, 0));
        self.next_ready()
    }

    /// Advance only the retained input batch, yielding at most one cadence.
    pub(crate) fn next_ready(&mut self) -> Option<crate::arrow::BatchReader> {
        let (batch, offset) = self.current.take()?;
        let available = batch.num_rows().saturating_sub(offset);
        if available == 0 {
            return None;
        }
        let take = (self.row_size - self.rows).min(available);
        if offset == 0 && take == batch.num_rows() {
            self.batches.push(batch);
        } else {
            self.batches.push(batch.slice(offset, take));
            if take < available {
                self.current = Some((batch, offset + take));
            }
        }
        self.rows += take;
        if self.rows != self.row_size {
            return None;
        }
        self.rows = 0;
        Some(crate::arrow::batch_reader(
            std::sync::Arc::clone(&self.schema),
            std::mem::take(&mut self.batches),
        ))
    }

    /// Take the successful final remainder, if this window holds one.
    pub(crate) fn finish(&mut self) -> Option<crate::arrow::BatchReader> {
        if self.rows == 0 {
            return None;
        }
        self.rows = 0;
        Some(crate::arrow::batch_reader(
            std::sync::Arc::clone(&self.schema),
            std::mem::take(&mut self.batches),
        ))
    }

    /// Discard an incomplete cadence after a source or conversion failure.
    pub(crate) fn clear(&mut self) {
        self.rows = 0;
        self.batches.clear();
        self.current = None;
    }
}

impl Iterator for CommitReaders {
    type Item = crate::Result<crate::arrow::BatchReader>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let Some(commit_row_size) = self.commit_row_size else {
            self.done = true;
            return self.batches.take().map(Ok);
        };
        loop {
            if let Some(reader) = self.buffer.as_mut().and_then(CommitBuffer::next_ready) {
                return Some(Ok(reader));
            }
            let batches = self
                .batches
                .as_mut()
                .expect("a live commit splitter owns its source");
            match batches.next() {
                Some(Ok(batch)) => {
                    if batch.num_rows() == 0 {
                        continue;
                    }
                    let buffer = self.buffer.get_or_insert_with(|| {
                        CommitBuffer::new(std::sync::Arc::clone(&self.schema), commit_row_size)
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
