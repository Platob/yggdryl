//! `StreamSerie`: a lazy stream of rows, each a [`Scalar`], under one record
//! field - the row-at-a-time face of a serie, where a
//! [`StreamChunkedSerie`](crate::StreamChunkedSerie) is the chunk-at-a-time
//! one.
//!
//! Rows are ordered sequences of column values in the field's order, the one
//! row shape this crate has; a caller wanting names reads them through
//! [`FieldRecord`](crate::FieldRecord) over [`StreamSerie::field`]. Nothing
//! is held beyond the row being produced.

use crate::{Error, Field, Result, Scalar};

/// A lazy stream of rows under one record field.
pub struct StreamSerie {
    field: Field,
    rows: Box<dyn Iterator<Item = Result<Scalar>> + Send>,
    pending_error: Option<Error>,
    ended: bool,
    proven: bool,
    checked_order: bool,
    order: Option<RowOrder>,
}

struct RowOrder {
    key: crate::expression::BoundSelector,
    options: Vec<crate::SortOptions>,
    previous: Option<Scalar>,
}
impl RowOrder {
    fn check(&mut self, row: &Scalar, root: &Field) -> Result<()> {
        let key = self.key.apply_scalar(row)?;
        if let Some(previous) = &self.previous {
            let before = previous.sequence_rows().expect("order keys are records");
            let after = key.sequence_rows().expect("order keys are records");
            for ((before, after), options) in before.iter().zip(after.iter()).zip(&self.options) {
                match crate::serie::compare_values(before, after, *options) {
                    std::cmp::Ordering::Less => break,
                    std::cmp::Ordering::Equal => {}
                    std::cmp::Ordering::Greater => {
                        return Err(Error::InvalidRecord {
                            path: root.name().into(),
                            reason: "a row is out of the order its root declares".into(),
                        });
                    }
                }
            }
        }
        self.previous = Some(key);
        Ok(())
    }
}

impl StreamSerie {
    /// The rows `rows` yields, each an ordered sequence under `field`, taken
    /// as they come: nothing is pulled before the first row is asked for.
    pub fn from_rows<I>(field: Field, rows: I) -> Self
    where
        I: IntoIterator<Item = Result<Scalar>>,
        I::IntoIter: Send + 'static,
    {
        Self {
            field,
            rows: Box::new(rows.into_iter()),
            pending_error: None,
            ended: false,
            proven: false,
            checked_order: false,
            order: None,
        }
    }

    pub(crate) fn prepare_order(&mut self) -> Result<()> {
        if self.proven || self.checked_order {
            return Ok(());
        }
        self.checked_order = true;
        if let Some(by) = self.field.as_sort().by()?
            && !by.is_empty()
        {
            let projections = by.iter().enumerate().map(|(index, ordering)| {
                crate::expression::Projection::aliased(
                    ordering.term().clone(),
                    format!("_order_{index}"),
                )
            });
            self.order = Some(RowOrder {
                key: crate::Selector::new(projections).bind(&self.field)?,
                options: by.iter().map(|ordering| ordering.options()).collect(),
                previous: None,
            });
        }
        Ok(())
    }

    /// The struct root every row is an ordered sequence under.
    #[must_use]
    pub const fn field(&self) -> &Field {
        &self.field
    }

    pub(crate) fn from_proven_rows<I>(field: Field, rows: I) -> Self
    where
        I: IntoIterator<Item = Result<Scalar>>,
        I::IntoIter: Send + 'static,
    {
        Self {
            proven: true,
            ..Self::from_rows(field, rows)
        }
    }

    pub(crate) fn require_record_field(&self) -> Result<()> {
        if self.field.dtype().as_fields().is_none() || self.field.is_nullable() {
            return Err(Error::InvalidRecord {
                path: smol_str::SmolStr::new(self.field.name()),
                reason: smol_str::SmolStr::new_static("StreamSerie needs a required record field"),
            });
        }
        Ok(())
    }

    pub(crate) fn bounded_reader(
        mut self,
        rows: Option<usize>,
        bytes: Option<u64>,
    ) -> crate::arrow::Result<crate::arrow::BatchReader> {
        self.prepare_order()?;
        let field = self.field.clone();
        crate::arrow::rows::proven_result_reader(&field, self, rows, bytes)
    }

    /// Everything left, collected.
    ///
    /// # Errors
    ///
    /// Returns the first error a row produced.
    pub fn collect_rows(self) -> Result<Vec<Scalar>> {
        self.collect()
    }

    /// The rows as a stream of Arrow batches, widened lazily.
    ///
    /// # Errors
    ///
    /// Returns an error when the field cannot be expressed as an Arrow schema.
    pub fn into_arrow_reader(self) -> Result<crate::arrow::BatchReader> {
        self.bounded_reader(None, None).map_err(Error::from)
    }

    /// Lay out one bounded prefix; a failure follows the rows before it.
    pub(crate) fn next_chunk(&mut self) -> Option<crate::arrow::Result<crate::Serie>> {
        let mut rows = Vec::new();
        for _ in 0..crate::media::DEFAULT_RECORD_BATCH_ROW_SIZE {
            match self.next() {
                Some(Ok(row)) => rows.push(row),
                Some(Err(error)) if rows.is_empty() => return Some(Err(error.into())),
                Some(Err(error)) => {
                    self.pending_error = Some(error);
                    break;
                }
                None => break,
            }
        }
        if rows.is_empty() {
            return None;
        }
        let chunk = crate::serie::from_canonical_rows(
            std::sync::Arc::new(self.field.clone()),
            &rows.iter().collect::<Vec<_>>(),
        )
        .map_err(Into::into);
        if chunk.is_err() {
            self.ended = true;
        }
        Some(chunk)
    }

    /// Read a stream of Arrow batches back as rows, lazily.
    ///
    /// Each batch lands as one record column when it is pulled, and its rows
    /// are read through the column's leaves one at a time; a batch the root
    /// refuses, or the reader's own failure, is the item where it stands.
    ///
    /// # Errors
    ///
    /// Returns an error when the reader's schema is not one this crate can
    /// type.
    pub fn from_arrow_reader(reader: crate::arrow::BatchReader) -> Result<Self> {
        let batches = crate::StreamChunkedSerie::from_arrow_reader(
            None,
            reader,
            crate::ArrowCastOptions::default(),
        )?;
        let field = batches.field().clone();
        let rows = batches.into_chunks().flat_map(|records| {
            let (records, refused) = match records {
                Ok(records) => (Some(records), None),
                Err(error) => (None, Some(Err(Error::from(error)))),
            };
            let len = records.as_ref().map_or(0, crate::Serie::len);
            refused.into_iter().chain((0..len).map(move |row| {
                records
                    .as_ref()
                    .map_or(Ok(Scalar::Null), |held| held.scalar(row))
            }))
        });
        Ok(Self::from_proven_rows(field, rows))
    }
}

impl Iterator for StreamSerie {
    type Item = Result<Scalar>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(error) = self.pending_error.take() {
            return Some(Err(error));
        }
        if self.ended {
            return None;
        }
        if let Err(error) = self.prepare_order() {
            self.ended = true;
            return Some(Err(error));
        }
        let next = self.rows.next().map(|row| {
            row.and_then(|row| {
                if self.proven {
                    Ok(row)
                } else {
                    let row = self.field.scalar(row)?;
                    if let Some(order) = &mut self.order {
                        order.check(&row, &self.field)?;
                    }
                    Ok(row)
                }
            })
        });
        if !matches!(next, Some(Ok(_))) {
            self.ended = true;
        }
        next
    }
}

impl std::iter::FusedIterator for StreamSerie {}

impl std::fmt::Debug for StreamSerie {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StreamSerie")
            .field("field", &self.field)
            .finish_non_exhaustive()
    }
}
