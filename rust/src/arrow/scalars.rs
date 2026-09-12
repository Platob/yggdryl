//! The generic Arrow scalar family.
//!
//! Arrow spells one payload four ways - a pinned one-row array, a column of
//! any length, a table of rows, and a one-shot stream of tables - and a caller
//! holding one of them rarely knows which. [`ArrowValue`] regroups the four
//! behind a single value carrying the exact [`Field`] that types it, so
//! reading, writing, casting, and crossing into [`Scalar`] are asked once
//! rather than once per shape.
//!
//! The family owns no conversion of its own. Every crossing routes through the
//! single scalar/array boundary - [`scalar_array`](super::scalar_array),
//! [`scalar_value`](super::scalar_value), [`array_from_value`](super::array_from_value),
//! [`array_to_value`](super::array_to_value), [`batch_from_value`](super::batch_from_value),
//! and [`batch_to_value`](super::batch_to_value) - and every stream through
//! [`BatchReader`], which is what a record read or write already speaks.
//!
//! ```
//! use std::sync::Arc;
//!
//! use arrow_array::{ArrayRef, Int64Array};
//! use yggdryl::{ArrowShape, ArrowValue, DataType, Field};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let field = Field::new("price", DataType::Int64, false);
//! let column: ArrayRef = Arc::new(Int64Array::from(vec![125, 126, 127]));
//! let value = ArrowValue::from_array(field, column)?;
//!
//! assert_eq!(value.shape(), ArrowShape::Array);
//! assert_eq!(value.row_size(), Some(3));
//!
//! // Whatever the shape, one funnel answers the record surface.
//! let rows: usize = value.into_reader()?.map(|batch| batch.unwrap().num_rows()).sum();
//! assert_eq!(rows, 3);
//! # Ok(())
//! # }
//! ```

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use arrow_array::{
    Array, ArrayRef, RecordBatch, RecordBatchOptions, RecordBatchReader, StructArray,
};
use arrow_schema::SchemaRef;
use smol_str::SmolStr;

use super::{
    BatchReader, Error, Result, array_from_value, array_to_value, arrow_schema_from_field,
    batch_from_value, batch_reader, batch_to_value, field_from_arrow_schema, scalar_array,
    scalar_value,
};
use crate::media::DEFAULT_ROOT_NAME;
use crate::{ArrowCast, ArrowCastOptions, DataType, Field, Scalar};

/// Which of Arrow's four payload shapes an [`ArrowValue`] holds.
///
/// The order is the widening order: a scalar is one row of an array, an array
/// is one column of a batch, and a batch is one element of a stream.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ArrowShape {
    /// One pinned row: an array of length one.
    Scalar,
    /// One column of any length.
    Array,
    /// One held table of rows.
    Batch,
    /// A one-shot stream of tables over one schema.
    Stream,
}

impl ArrowShape {
    /// Every shape in widening order.
    pub const ALL: [Self; 4] = [Self::Scalar, Self::Array, Self::Batch, Self::Stream];

    /// The canonical lowercase name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Scalar => "scalar",
            Self::Array => "array",
            Self::Batch => "batch",
            Self::Stream => "stream",
        }
    }

    /// Report whether rows of this shape are laid out under a Struct root.
    pub const fn is_tabular(self) -> bool {
        matches!(self, Self::Batch | Self::Stream)
    }

    /// Report whether reading this shape consumes it.
    ///
    /// A stream is one-shot, so its row count is unknown until it is drained
    /// and every accessor that would count rows answers `None`.
    pub const fn is_streamed(self) -> bool {
        matches!(self, Self::Stream)
    }

    /// Parse a canonical shape name.
    ///
    /// # Errors
    ///
    /// Returns an error naming the four spellings when `value` is not one.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> crate::Result<Self> {
        <Self as FromStr>::from_str(value)
    }
}

impl fmt::Display for ArrowShape {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ArrowShape {
    type Err = crate::Error;

    fn from_str(value: &str) -> crate::Result<Self> {
        match value {
            "scalar" => Ok(Self::Scalar),
            "array" => Ok(Self::Array),
            "batch" => Ok(Self::Batch),
            "stream" => Ok(Self::Stream),
            other => Err(crate::Error::InvalidRecord {
                path: SmolStr::new_static("$.shape"),
                reason: crate::text::expected_got("scalar, array, batch, or stream", other),
            }),
        }
    }
}

/// The payload behind an [`ArrowValue`], kept private so the paired Field
/// cannot be separated from the buffers it types.
enum Payload {
    /// Exactly one row, pinned.
    Scalar(ArrayRef),
    /// A column of any length.
    Array(ArrayRef),
    /// Rows held under a Struct root.
    Batch(RecordBatch),
    /// Rows streamed under a Struct root.
    Stream(BatchReader),
}

/// One Arrow-backed value paired with the exact [`Field`] that types it.
///
/// The Field is the authority the rest of the project already uses: it decides
/// nullability, dictionary options, extension identity, and the names a row
/// carries. For a scalar or an array it describes one element; for a batch or
/// a stream it is the non-null Struct root the rows live under.
///
/// Construction validates the pairing once. Nothing after it re-checks the
/// physical layout, so narrowing, casting, and streaming are the cost of the
/// operation rather than the cost of proving it again.
///
/// ```
/// use yggdryl::{ArrowShape, ArrowValue, DataType, Field, Scalar};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let field = Field::new("symbol", DataType::utf8(), false);
/// let value = ArrowValue::from_value(&field, &Scalar::from("AAPL"))?;
///
/// assert_eq!(value.shape(), ArrowShape::Scalar);
/// assert_eq!(value.into_scalar()?.as_str(), Some("AAPL"));
/// # Ok(())
/// # }
/// ```
pub struct ArrowValue {
    field: Field,
    payload: Payload,
}

impl fmt::Debug for ArrowValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // A stream has no observable contents until it is drained, so the
        // shape and its declared field are all a debug rendering may claim.
        formatter
            .debug_struct("ArrowValue")
            .field("shape", &self.shape())
            .field("field", &self.field)
            .field("row_size", &self.row_size())
            .finish()
    }
}

impl ArrowValue {
    /// Pair one pinned Arrow row with the Field that types it.
    ///
    /// This is Arrow's own scalar: a length-one array whose single row is the
    /// value. [`crate::Field`] remains the authority on what that row may
    /// hold, so a foreign array is refused here rather than at the first read.
    ///
    /// # Errors
    ///
    /// Returns an error unless `array` holds exactly one row of `field`'s
    /// exact physical datatype.
    pub fn from_scalar_array(field: Field, array: ArrayRef) -> Result<Self> {
        if array.len() != 1 {
            return Err(Error::IncompatibleSchema(format!(
                "an Arrow scalar holds exactly one row, got {}",
                array.len()
            )));
        }
        require_layout(&field, array.as_ref())?;
        Ok(Self {
            field,
            payload: Payload::Scalar(array),
        })
    }

    /// Pair one Arrow column with the Field that types its items.
    ///
    /// # Errors
    ///
    /// Returns an error unless `array` holds `field`'s exact physical
    /// datatype.
    pub fn from_array(field: Field, array: ArrayRef) -> Result<Self> {
        require_layout(&field, array.as_ref())?;
        Ok(Self {
            field,
            payload: Payload::Array(array),
        })
    }

    /// Take one held Arrow table, reading its root Field from its own schema.
    ///
    /// The root is named [`DEFAULT_ROOT_NAME`](crate::media::DEFAULT_ROOT_NAME),
    /// because Arrow names columns and never the record.
    ///
    /// # Errors
    ///
    /// Returns an error when the batch schema does not project to a Yggdryl
    /// Struct root.
    pub fn from_batch(batch: RecordBatch) -> Result<Self> {
        let field = field_from_arrow_schema(DEFAULT_ROOT_NAME, batch.schema().as_ref())?;
        Ok(Self {
            field,
            payload: Payload::Batch(batch),
        })
    }

    /// Take one held Arrow table under a root Field the caller already has.
    ///
    /// The pairing is exact: this is the declaration of what the batch already
    /// is, never a request to convert it. Use [`cast`](Self::cast) to reshape
    /// rows that do not yet match.
    ///
    /// # Errors
    ///
    /// Returns an error unless `root` is a bounded, non-nullable Struct root
    /// projecting to exactly the batch's own schema.
    pub fn from_batch_as(root: Field, batch: RecordBatch) -> Result<Self> {
        let expected = arrow_schema_from_field(&root)?;
        require_schema(&expected, batch.schema_ref())?;
        Ok(Self {
            field: root,
            payload: Payload::Batch(batch),
        })
    }

    /// Take one batch stream, reading its root Field from its reported schema.
    ///
    /// Nothing is pulled: a reader states its schema before its first batch,
    /// which is what lets the root be known without deciding to read.
    ///
    /// # Errors
    ///
    /// Returns an error when the reported schema does not project to a
    /// Yggdryl Struct root.
    pub fn from_reader(reader: BatchReader) -> Result<Self> {
        let field = field_from_arrow_schema(DEFAULT_ROOT_NAME, reader.schema().as_ref())?;
        Ok(Self {
            field,
            payload: Payload::Stream(reader),
        })
    }

    /// Take one batch stream under a root Field the caller already has.
    ///
    /// # Errors
    ///
    /// Returns an error unless `root` is a bounded, non-nullable Struct root
    /// projecting to exactly the reader's reported schema.
    pub fn from_reader_as(root: Field, reader: BatchReader) -> Result<Self> {
        let expected = arrow_schema_from_field(&root)?;
        require_schema(&expected, &reader.schema())?;
        Ok(Self {
            field: root,
            payload: Payload::Stream(reader),
        })
    }

    /// Materialize one native value as a pinned Arrow row.
    ///
    /// The value crosses through [`scalar_array`](super::scalar_array), so it
    /// is validated and canonicalized by `field` exactly as a stored column
    /// value is.
    ///
    /// # Errors
    ///
    /// Returns an error when the value violates `field` or its physical Arrow
    /// layout cannot represent it.
    pub fn from_value(field: &Field, value: &Scalar) -> Result<Self> {
        let array = scalar_array(field, value)?;
        Ok(Self {
            field: field.clone(),
            payload: Payload::Scalar(array),
        })
    }

    /// Materialize a native sequence as one Arrow column.
    ///
    /// # Errors
    ///
    /// Returns an error when `values` is not a sequence or an element violates
    /// `field`.
    pub fn from_values(field: &Field, values: &Scalar) -> Result<Self> {
        let array = array_from_value(field, values)?;
        Ok(Self {
            field: field.clone(),
            payload: Payload::Array(array),
        })
    }

    /// Materialize a native sequence of rows as one Arrow table.
    ///
    /// # Errors
    ///
    /// Returns an error when `root` is not a record root, `rows` is not a
    /// sequence, or a row violates the schema.
    pub fn from_rows(root: &Field, rows: &Scalar) -> Result<Self> {
        let batch = batch_from_value(root, rows)?;
        Ok(Self {
            field: root.clone(),
            payload: Payload::Batch(batch),
        })
    }
}

impl ArrowValue {
    /// The exact Field this value is typed by.
    ///
    /// For a scalar or an array it describes one element; for a batch or a
    /// stream it is the non-null Struct root the rows live under.
    pub const fn field(&self) -> &Field {
        &self.field
    }

    /// The datatype the Field declares.
    pub fn dtype(&self) -> &DataType {
        self.field.dtype()
    }

    /// Which of Arrow's four payload shapes this value holds.
    pub const fn shape(&self) -> ArrowShape {
        match self.payload {
            Payload::Scalar(_) => ArrowShape::Scalar,
            Payload::Array(_) => ArrowShape::Array,
            Payload::Batch(_) => ArrowShape::Batch,
            Payload::Stream(_) => ArrowShape::Stream,
        }
    }

    /// Report whether this value is one pinned row.
    pub const fn is_scalar(&self) -> bool {
        matches!(self.payload, Payload::Scalar(_))
    }

    /// Report whether this value is one column.
    pub const fn is_array(&self) -> bool {
        matches!(self.payload, Payload::Array(_))
    }

    /// Report whether this value is one held table.
    pub const fn is_batch(&self) -> bool {
        matches!(self.payload, Payload::Batch(_))
    }

    /// Report whether this value is a one-shot stream.
    pub const fn is_stream(&self) -> bool {
        matches!(self.payload, Payload::Stream(_))
    }

    /// The number of rows, or `None` for a stream that has not been drained.
    ///
    /// A stream states its schema before its first batch but never its length,
    /// so counting it is a decision to read it. [`into_batch`](Self::into_batch)
    /// is that decision spelled out.
    pub fn row_size(&self) -> Option<usize> {
        match &self.payload {
            Payload::Scalar(_) => Some(1),
            Payload::Array(array) => Some(array.len()),
            Payload::Batch(batch) => Some(batch.num_rows()),
            Payload::Stream(_) => None,
        }
    }

    /// The number of columns one row carries.
    ///
    /// This is the width of the root the rows live under, so it agrees with
    /// [`into_batch`](Self::into_batch): a Field that is its own root answers
    /// its direct children, and anything else - a nullable Struct included -
    /// is the one column of a wrapping root.
    pub fn column_size(&self) -> usize {
        match &self.payload {
            Payload::Batch(batch) => batch.num_columns(),
            Payload::Stream(reader) => reader.schema().fields().len(),
            Payload::Scalar(_) | Payload::Array(_) => {
                if is_own_root(&self.field) {
                    self.field.fields().len()
                } else {
                    1
                }
            }
        }
    }

    /// Borrow the column behind a scalar or an array.
    ///
    /// `None` means the payload is tabular; nothing is materialized to answer.
    pub const fn as_array(&self) -> Option<&ArrayRef> {
        match &self.payload {
            Payload::Scalar(array) | Payload::Array(array) => Some(array),
            Payload::Batch(_) | Payload::Stream(_) => None,
        }
    }

    /// Borrow the held table behind a batch.
    ///
    /// `None` means the payload is not a held batch; a stream is never drained
    /// to answer.
    pub const fn as_batch(&self) -> Option<&RecordBatch> {
        match &self.payload {
            Payload::Batch(batch) => Some(batch),
            Payload::Scalar(_) | Payload::Array(_) | Payload::Stream(_) => None,
        }
    }

    /// The non-null Struct root this value's rows live under.
    ///
    /// A tabular payload already has one. A scalar or an array of Structs is
    /// its own root; any other element Field becomes the single column of a
    /// root named [`DEFAULT_ROOT_NAME`](crate::media::DEFAULT_ROOT_NAME),
    /// which is the same wrapping a native array-of-values already takes.
    ///
    /// # Errors
    ///
    /// Returns an error when the element Field cannot form a bounded record
    /// root.
    pub fn root(&self) -> Result<Field> {
        root_of(&self.field)
    }

    /// Project the Arrow schema of this value's rows.
    ///
    /// # Errors
    ///
    /// Returns an error when the root cannot be projected to Arrow.
    pub fn schema(&self) -> Result<SchemaRef> {
        match &self.payload {
            Payload::Batch(batch) => Ok(batch.schema()),
            Payload::Stream(reader) => Ok(reader.schema()),
            Payload::Scalar(_) | Payload::Array(_) => arrow_schema_from_field(&self.root()?),
        }
    }
}

impl ArrowValue {
    /// Narrow to one Arrow column.
    ///
    /// A tabular payload becomes the Struct column its rows already are, which
    /// shares the batch's own child buffers rather than rebuilding them. A
    /// stream is drained first.
    ///
    /// # Errors
    ///
    /// Returns a schema projection failure, or whatever draining a stream
    /// raised.
    pub fn into_array(self) -> Result<ArrayRef> {
        match self.payload {
            Payload::Scalar(array) | Payload::Array(array) => Ok(array),
            Payload::Batch(_) | Payload::Stream(_) => {
                let batch = self.into_batch()?;
                Ok(Arc::new(struct_array_from_batch(&batch)?))
            }
        }
    }

    /// Narrow to one held Arrow table.
    ///
    /// This is where a stream stops being a stream: every batch is pulled and
    /// concatenated, so memory is the whole result. Prefer
    /// [`into_reader`](Self::into_reader) wherever the consumer can take rows
    /// as they arrive.
    ///
    /// # Errors
    ///
    /// Returns a schema projection failure, a concatenation failure, or
    /// whatever the stream raised.
    pub fn into_batch(self) -> Result<RecordBatch> {
        match self.payload {
            Payload::Batch(batch) => Ok(batch),
            Payload::Scalar(array) | Payload::Array(array) => batch_from_array(&self.field, array),
            Payload::Stream(reader) => {
                let schema = reader.schema();
                let mut batches = Vec::new();
                for batch in reader {
                    batches.push(batch.map_err(super::from_reader_error)?);
                }
                Ok(arrow_select::concat::concat_batches(&schema, &batches)?)
            }
        }
    }

    /// Widen to the one shape every record read and write speaks.
    ///
    /// This is the funnel: a scalar, a column, and a table each become a
    /// reader over one batch, and a stream is already one. Nothing is
    /// collected and nothing is copied - a held batch is handed to the reader
    /// as it stands.
    ///
    /// # Errors
    ///
    /// Returns a schema projection failure, or a failure building the single
    /// batch a non-tabular payload becomes.
    pub fn into_reader(self) -> Result<BatchReader> {
        match self.payload {
            Payload::Stream(reader) => Ok(reader),
            Payload::Batch(batch) => Ok(batch_reader(batch.schema(), [batch])),
            Payload::Scalar(array) | Payload::Array(array) => {
                let batch = batch_from_array(&self.field, array)?;
                Ok(batch_reader(batch.schema(), [batch]))
            }
        }
    }

    /// Cross into the native value model.
    ///
    /// A scalar answers one [`Scalar`]; every other shape answers a sequence -
    /// of items for a column, of ordered row sequences for a table or a
    /// stream - which is the shape the rest of the value model, and every
    /// structured text format, already speaks.
    ///
    /// # Errors
    ///
    /// Returns an error when a value cannot be represented, or whatever the
    /// stream raised.
    pub fn into_scalar(self) -> Result<Scalar> {
        match self.payload {
            Payload::Scalar(array) => scalar_value(&self.field, array.as_ref()),
            Payload::Array(array) => array_to_value(&self.field, array.as_ref()),
            Payload::Batch(batch) => batch_to_value(&batch),
            Payload::Stream(reader) => {
                let mut rows = Vec::new();
                for batch in reader {
                    let batch = batch.map_err(super::from_reader_error)?;
                    let values = batch_to_value(&batch)?;
                    let Some(values) = values.as_sequence() else {
                        return Err(Error::internal("arrow::ArrowValue::into_scalar"));
                    };
                    rows.extend(values.iter().cloned());
                }
                Ok(Scalar::from_sequence(rows))
            }
        }
    }

    /// Consume this value into Arrow's own scalar marker.
    ///
    /// [`arrow_array::Scalar`] is the `Datum` Arrow's kernels take for one
    /// value, so this is how a Yggdryl-typed row is handed to them.
    ///
    /// # Errors
    ///
    /// Returns an error unless this value holds exactly one row.
    pub fn into_arrow_scalar(self) -> Result<arrow_array::Scalar<ArrayRef>> {
        if self.row_size() != Some(1) {
            return Err(Error::IncompatibleSchema(format!(
                "an Arrow scalar holds exactly one row, got a {} of {}",
                self.shape(),
                self.row_size()
                    .map_or_else(|| "unknown length".to_owned(), |rows| rows.to_string())
            )));
        }
        Ok(arrow_array::Scalar::new(self.into_array()?))
    }

    /// Reshape this value onto another Field, keeping its shape.
    ///
    /// The cast is the project's one recursive implementation, so it
    /// reconciles Struct names, fills valid missing children, and returns the
    /// caller's own arrays when the pair is already exact. A stream is cast
    /// one batch at a time under a single compiled plan.
    ///
    /// # Errors
    ///
    /// Returns whatever the cast returns, including an unsupported
    /// conversion, an ambiguous Struct match, or a target the result cannot
    /// satisfy.
    pub fn cast(self, field: &Field, options: ArrowCastOptions) -> Result<Self> {
        match self.payload {
            Payload::Scalar(array) => {
                Self::from_scalar_array(field.clone(), field.cast_arrow_array(array, options)?)
            }
            Payload::Array(array) => {
                Self::from_array(field.clone(), field.cast_arrow_array(array, options)?)
            }
            Payload::Batch(batch) => Ok(Self {
                field: field.clone(),
                payload: Payload::Batch(field.cast_arrow_batch(batch, options)?),
            }),
            Payload::Stream(reader) => Ok(Self {
                field: field.clone(),
                payload: Payload::Stream(super::cast_reader(reader, field, options)?),
            }),
        }
    }
}

/// Report whether a Field is already the root its own rows live under.
///
/// A non-null Struct is: its children are the columns. Everything else - a
/// nullable Struct included, because a record root may not be absent - becomes
/// the single column of a wrapping root.
fn is_own_root(field: &Field) -> bool {
    field.is_struct() && !field.is_nullable()
}

/// The non-null Struct root a Field's values form rows of.
fn root_of(field: &Field) -> Result<Field> {
    if is_own_root(field) {
        return Ok(field.clone());
    }
    Ok(DataType::from_fields([field.clone()])?.required_field(DEFAULT_ROOT_NAME))
}

/// Refuse an array whose physical layout is not the one the Field declares.
fn require_layout(field: &Field, array: &dyn Array) -> Result<()> {
    // A caller-built DataType can be arbitrarily deep, so bound the shape
    // before Arrow's recursive projection walks it.
    field.dtype().validate_bounded()?;
    let expected = field.clone().into_arrow_ref()?.data_type().clone();
    if array.data_type() == &expected {
        return Ok(());
    }
    Err(Error::IncompatibleSchema(format!(
        "Arrow datatype {:?} differs from the {:?} field's {expected:?}",
        array.data_type(),
        field.name()
    )))
}

/// Refuse rows whose schema is not exactly the declared root's projection.
fn require_schema(expected: &SchemaRef, actual: &SchemaRef) -> Result<()> {
    if expected == actual {
        return Ok(());
    }
    Err(Error::SchemaMismatch {
        index: None,
        path: SmolStr::new_static("$"),
        diff: format!(
            "\u{2260} rows do not match the declared root\n  \u{2212} {}\n  + {}",
            crate::text::elide_display(expected),
            crate::text::elide_display(actual)
        ),
    })
}

/// Lay one column out as the rows of a batch.
///
/// A non-null Struct column already is those rows and hands over its own child
/// buffers; anything else becomes the single column of a wrapping root.
fn batch_from_array(field: &Field, array: ArrayRef) -> Result<RecordBatch> {
    let rows = array.len();
    let schema = arrow_schema_from_field(&root_of(field)?)?;
    let columns = if is_own_root(field) {
        let structs = array
            .as_any()
            .downcast_ref::<StructArray>()
            .ok_or_else(|| Error::internal("arrow::ArrowValue::batch_from_array"))?;
        if structs.null_count() != 0 {
            return Err(Error::IncompatibleSchema(format!(
                "a batch has no row validity, so a Struct column with {} null rows is not rows",
                structs.null_count()
            )));
        }
        structs.columns().to_vec()
    } else {
        vec![array]
    };
    // The row count travels explicitly: a root with no columns, or an empty
    // column, would otherwise lose the length it is declaring.
    Ok(RecordBatch::try_new_with_options(
        schema,
        columns,
        &RecordBatchOptions::new().with_row_count(Some(rows)),
    )?)
}

/// Read a batch's rows back as the one Struct column they are.
fn struct_array_from_batch(batch: &RecordBatch) -> Result<StructArray> {
    let fields = batch.schema_ref().fields().clone();
    if fields.is_empty() {
        return Ok(StructArray::new_empty_fields(batch.num_rows(), None));
    }
    Ok(StructArray::try_new(
        fields,
        batch.columns().to_vec(),
        None,
    )?)
}

#[cfg(test)]
mod tests;
