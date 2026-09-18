//! Row application: an expression over native records, bound lazily.
//!
//! The vectorized tier is where volume goes; this is the tier for rows a
//! caller already holds as [`Scalar`] values - one ordered sequence or one
//! named record per row - and wants back the same way, with no batch in
//! between. The plan is settled once, against the schema the caller declares
//! or, when none is declared, against the schema the first record implies;
//! every row after that runs the settled plan.
//!
//! A named record is a schema in itself: its names and its values' datatypes
//! are exactly a struct. An ordered sequence is not - its cells have no
//! names - so a stream of sequences needs the schema declared, the same way
//! every record write in this crate does.

use crate::{Error, Field, Result, Scalar};

use super::filter::Filter;
use super::selector::Selector;
use super::{Expression, unwritable_rows};

/// The rows one application yields, under the field it published them as.
///
/// Rows are ordered sequences of column values in the field's order, the
/// one row shape this crate has; a caller wanting names reads them through
/// [`FieldRecord`](crate::FieldRecord) over [`Self::field`]. Nothing is held
/// beyond the row being produced.
pub struct Records {
    field: Field,
    rows: Box<dyn Iterator<Item = Result<Scalar>> + Send>,
}

impl Records {
    /// The struct root every row is an ordered sequence under.
    #[must_use]
    pub const fn field(&self) -> &Field {
        &self.field
    }

    /// Everything left, collected.
    ///
    /// # Errors
    ///
    /// Returns the first error a row produced.
    pub fn collect_rows(self) -> Result<Vec<Scalar>> {
        self.rows.collect()
    }

    /// The rows as a stream of Arrow batches, widened lazily.
    ///
    /// # Errors
    ///
    /// Returns an error when the field cannot be expressed as an Arrow schema.
    pub fn into_arrow_reader(self) -> Result<crate::arrow::BatchReader> {
        crate::arrow::rows::result_reader(&self.field, self.rows, None, None, None, None)
            .map_err(Error::from)
    }

    /// Read a stream of Arrow batches back as rows.
    ///
    /// # Errors
    ///
    /// Returns an error when the reader's schema is not one this crate can
    /// type, or a batch cannot be read.
    pub fn from_arrow_reader(reader: crate::arrow::BatchReader) -> Result<Self> {
        let field = crate::arrow::field_from_arrow_schema(
            crate::media::DEFAULT_ROOT_NAME,
            &reader.schema(),
        )?;
        let rows = crate::ArrowScalar::from_reader_as(field.clone(), reader)?
            .into_scalar()?
            .as_sequence()
            .map(<[Scalar]>::to_vec)
            .unwrap_or_default();
        Ok(Self {
            field,
            rows: Box::new(rows.into_iter().map(Ok)),
        })
    }
}

impl Iterator for Records {
    type Item = Result<Scalar>;

    fn next(&mut self) -> Option<Self::Item> {
        self.rows.next()
    }
}

impl std::fmt::Debug for Records {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Records")
            .field("field", &self.field)
            .finish_non_exhaustive()
    }
}

/// The rows to run over, and the schema they run under.
type Rows = Box<dyn Iterator<Item = Result<Scalar>> + Send>;

/// Settle the schema and hand the rows over, the first one included.
///
/// A declared schema is taken as is. Without one, the first record is read,
/// its own struct datatype is the schema, and the record is put back in front
/// of the rest; a stream with no record and no schema has nothing to bind
/// against and is refused.
fn schema_of<I, R>(schema: Option<&Field>, records: I) -> Result<(Field, Rows)>
where
    I: IntoIterator<Item = R>,
    I::IntoIter: Send + 'static,
    R: TryInto<Scalar>,
    R::Error: Into<Error>,
{
    let mut rows = records
        .into_iter()
        .map(|record| record.try_into().map_err(Into::into));
    if let Some(schema) = schema {
        schema.require_struct()?;
        return Ok((schema.clone(), Box::new(rows)));
    }
    let Some(first) = rows.next() else {
        return Err(unwritable_rows(
            "expected a declared schema, or at least one record to infer one from; got neither",
        ));
    };
    let first = first?;
    let dtype = first.dtype()?;
    if first.as_record().is_none() || !matches!(dtype, crate::DataType::Structure(_)) {
        return Err(unwritable_rows(format_args!(
            "expected a named record to infer a schema from, got {}; declare the schema, or pass \
             records built by Scalar::from_record",
            first.kind()
        )));
    }
    let schema = dtype.required_field(crate::media::DEFAULT_ROOT_NAME);
    Ok((schema, Box::new(std::iter::once(Ok(first)).chain(rows))))
}

impl Selector {
    /// The rows this selector publishes from native records.
    ///
    /// The plan is bound once - against `schema`, or against the schema the
    /// first record implies - and each row is canonicalized under it, run,
    /// and answered as an ordered sequence under [`Records::field`].
    ///
    /// ```
    /// use yggdryl::expression::Selector;
    /// use yggdryl::Scalar;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let selector: Selector = "symbol, price * 2 as doubled".parse()?;
    /// let rows = [
    ///     Scalar::from_record([("symbol", Scalar::from("AAPL")), ("price", Scalar::from(10_i64))])?,
    ///     Scalar::from_record([("symbol", Scalar::from("MSFT")), ("price", Scalar::from(20_i64))])?,
    /// ];
    /// let records = selector.apply_records(None, rows)?;
    /// assert_eq!(records.field().fields()[1].name(), "doubled");
    /// let held = records.collect_rows()?;
    /// assert_eq!(held[1], Scalar::from_sequence([Scalar::from("MSFT"), Scalar::from(40_i64)]));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when no schema is declared and none can be inferred,
    /// or the selector does not bind against it; a row that fails is the
    /// error item in its place.
    pub fn apply_records<I, R>(&self, schema: Option<&Field>, records: I) -> Result<Records>
    where
        I: IntoIterator<Item = R>,
        I::IntoIter: Send + 'static,
        R: TryInto<Scalar>,
        R::Error: Into<Error>,
    {
        let (schema, rows) = schema_of(schema, records)?;
        let bound = self.bind(&schema)?;
        let field = bound.output().clone();
        let rows = rows.map(move |row| bound.apply_scalar(&schema.canonicalize_row_value(row?)?));
        Ok(Records {
            field,
            rows: Box::new(rows),
        })
    }
}

impl Filter {
    /// The native records this filter answers true for.
    ///
    /// Bound once, as [`Selector::apply_records`] is; a kept row comes back
    /// canonical under [`Records::field`], which is the schema itself.
    ///
    /// # Errors
    ///
    /// Returns an error when no schema is declared and none can be inferred,
    /// or the filter does not bind against it as a predicate; a row that
    /// fails is the error item in its place.
    pub fn apply_records<I, R>(&self, schema: Option<&Field>, records: I) -> Result<Records>
    where
        I: IntoIterator<Item = R>,
        I::IntoIter: Send + 'static,
        R: TryInto<Scalar>,
        R::Error: Into<Error>,
    {
        let (schema, rows) = schema_of(schema, records)?;
        let bound = self.bind(&schema)?;
        let field = schema.clone();
        let rows = rows.filter_map(move |row| {
            let row = match row.and_then(|row| schema.canonicalize_row_value(row)) {
                Ok(row) => row,
                Err(error) => return Some(Err(error)),
            };
            match bound.matches(&row) {
                Ok(true) => Some(Ok(row)),
                Ok(false) => None,
                Err(error) => Some(Err(error)),
            }
        });
        Ok(Records {
            field,
            rows: Box::new(rows),
        })
    }
}

impl Expression {
    /// Apply this expression to native records.
    ///
    /// A `select` or `where` clause runs row by row, bound once. A plan or a
    /// sequence widens the records into the streamed Arrow path and runs
    /// there, so it needs the `arrow` feature; a plan that writes yields no
    /// rows.
    ///
    /// # Errors
    ///
    /// Returns an error when no schema is declared and none can be inferred,
    /// when the expression does not bind against it, or when a statement's
    /// target cannot be written.
    pub fn apply_records<I, R>(&self, schema: Option<&Field>, records: I) -> Result<Records>
    where
        I: IntoIterator<Item = R>,
        I::IntoIter: Send + 'static,
        R: TryInto<Scalar>,
        R::Error: Into<Error>,
    {
        match self {
            Self::Selector(selector) => selector.apply_records(schema, records),
            Self::Filter(filter) => filter.apply_records(schema, records),
            Self::Plan(_) | Self::Sequence(_) => {
                let (schema, rows) = schema_of(schema, records)?;
                let reader = Records {
                    field: schema,
                    rows,
                }
                .into_arrow_reader()?;
                Records::from_arrow_reader(self.apply_arrow_reader(reader)?)
            }
        }
    }
}
