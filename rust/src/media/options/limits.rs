//! Row and Arrow-byte limits over one streaming reader.

/// A reader bounding the rows and Arrow bytes that flow through it.
///
/// One batch is held at a time - pulled, cut if a bound lands inside it,
/// handed on - and the moment the bounds are met the source is never pulled
/// again, because a limit exists precisely so the rest of a file is not
/// decoded.
pub(super) struct Limited {
    pub(super) inner: crate::arrow::BatchReader,
    pub(super) schema: arrow_schema::SchemaRef,
    pub(super) state: WriteLimitState,
}

/// Stateful write limits shared by a pull reader and an incremental runtime
/// bridge.
///
/// The JavaScript binding has to leave the Rust stack while it awaits the next
/// asynchronous record chunk. Keeping these counters outside `Limited` lets
/// that bridge resume the exact same logical limit: byte accounting and the
/// one global "at least one row" decision never restart at a chunk boundary.
#[derive(Clone, Debug)]
pub(crate) struct WriteLimitState {
    /// Rows the row bound still admits; `None` when no row bound is set.
    remaining_rows: Option<u64>,
    /// Bytes the byte bound still admits; `None` when no byte bound is set.
    remaining_bytes: Option<u64>,
    /// Whether any row went out yet, for the at-least-one-row rule.
    yielded: bool,
    /// Set the moment the bounds are met, so no caller touches its source
    /// again.
    satisfied: bool,
}

impl WriteLimitState {
    pub(crate) const fn new(remaining_rows: Option<u64>, remaining_bytes: Option<u64>) -> Self {
        Self {
            remaining_rows,
            remaining_bytes,
            yielded: false,
            // `Some(0)` on either bound admits nothing: a reader still reports
            // its shaped schema, and no input batch is pulled.
            satisfied: matches!(remaining_rows, Some(0)) || matches!(remaining_bytes, Some(0)),
        }
    }

    pub(crate) const fn satisfied(&self) -> bool {
        self.satisfied
    }

    /// Admit the leading rows one logical write limit still allows.
    ///
    /// `None` means the limit is complete. A zero-row batch passes through as
    /// `Some`, because it carries schema continuity without spending either
    /// budget.
    pub(crate) fn apply(
        &mut self,
        batch: arrow_array::RecordBatch,
    ) -> Option<arrow_array::RecordBatch> {
        if self.satisfied {
            return None;
        }
        let rows = batch.num_rows() as u64;
        if rows == 0 {
            return Some(batch);
        }
        let mut take = rows;
        if let Some(remaining) = self.remaining_rows {
            take = take.min(remaining);
        }
        let mut size = 0_u64;
        if let Some(remaining) = self.remaining_bytes {
            size = u64::try_from(batch.get_array_memory_size()).unwrap_or(u64::MAX);
            if size > remaining {
                let fits = u64::try_from(
                    u128::from(remaining) * u128::from(rows) / u128::from(size.max(1)),
                )
                .unwrap_or(u64::MAX);
                let fits = if fits == 0 && !self.yielded { 1 } else { fits };
                take = take.min(fits);
            }
        }
        if take == 0 {
            self.satisfied = true;
            return None;
        }
        if take < rows {
            self.satisfied = true;
            self.yielded = true;
            let take = usize::try_from(take).unwrap_or(usize::MAX);
            return Some(batch.slice(0, take));
        }
        if let Some(remaining) = &mut self.remaining_rows {
            *remaining -= rows;
            if *remaining == 0 {
                self.satisfied = true;
            }
        }
        if let Some(remaining) = &mut self.remaining_bytes {
            *remaining = remaining.saturating_sub(size);
            if *remaining == 0 {
                self.satisfied = true;
            }
        }
        self.yielded = true;
        Some(batch)
    }
}

impl Iterator for Limited {
    type Item = std::result::Result<arrow_array::RecordBatch, arrow_schema::ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.state.satisfied() {
            return None;
        }
        let batch = match self.inner.next()? {
            Ok(batch) => batch,
            Err(error) => {
                // Fused after the first error, per the listing contract.
                self.state.satisfied = true;
                return Some(Err(error));
            }
        };
        self.state.apply(batch).map(Ok)
    }
}

impl arrow_array::RecordBatchReader for Limited {
    fn schema(&self) -> arrow_schema::SchemaRef {
        std::sync::Arc::clone(&self.schema)
    }
}
