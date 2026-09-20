//! Serie: one column, on the fourth side of the value model.
//!
//! One type, one file. [`DataType`] says what shape a column has, [`Field`]
//! says whose it is and whether a row may be absent, and [`Scalar`] is one
//! row of it. A [`Serie`] is the rows themselves under the field that types
//! them, so a column crosses a boundary as one value rather than as a schema
//! and a loose run beside it.
//!
//! | side | root | trait | widen | narrow |
//! | --- | --- | --- | --- | --- |
//! | datatype | [`DataType`] | [`DataTypeValue`] | `into_dtype` | `from_dtype` |
//! | field | [`Field`] | [`FieldValue`] | `into_field` | `from_field` |
//! | value | [`Scalar`] | [`Value`] | `into_scalar` | `from_scalar` |
//! | column | [`Serie`] | [`SerieValue`] | `into_serie` | `from_serie` |
//!
//! A column of one field is a list of that field, which is a datatype the
//! sequence family already owns, so a serie is registered there rather than
//! as a root of its own: [`Sequence::Serie`] is the leaf that carries it and
//! `Scalar::Sequence(Sequence::Serie(..))` is how a column is a value. That
//! is also why nothing here adds a [`DataTypeId`], a [`DataType`] variant or
//! a [`Field`] variant - a serie is not a second shape, it is the rows of a
//! shape that already exists.
//!
//! # The rows are Arrow buffers
//!
//! A column holds its rows the way Arrow lays them out, not as one boxed
//! value per row: a run of [`ArrayRef`] chunks in row order, each the exact
//! physical layout the field projects to. That is what a read, a write and a
//! cast already speak, so a column crosses into Arrow and back without
//! touching a row, and reading row `i` is an offset into a buffer rather than
//! a walk.
//!
//! | ask | cost |
//! | --- | --- |
//! | [`len`](Serie::len), [`is_null`](Serie::is_null) | constant |
//! | [`get`](Serie::get), the typed readers | one binary search over the chunks, then one buffer read |
//! | [`push`](Serie::push), [`extend`](Serie::extend) | amortized constant, into the unfrozen tail |
//! | [`append_arrow_array`](Serie::append_arrow_array), [`append`](Serie::append) | one chunk, no rows copied |
//! | [`slice`](Serie::slice) | one slice per touched chunk, buffers shared |
//! | [`child`](Serie::child), [`without_child`](Serie::without_child) | one per chunk, and never per row |
//! | [`into_arrow_reader`](Serie::into_arrow_reader) | one batch per chunk, nothing concatenated |
//! | [`rows`](Serie::rows), [`as_sequence`](Scalar::as_sequence) | one decode of every row, cached |
//!
//! A column grows in two halves: rows pushed as values land in a tail, and
//! the tail freezes into one chunk when buffers are asked for. Appending a
//! batch never touches the rows it appends, and appending a value never
//! builds an array per row.
//!
//! The chunks are what a column that is not fully materialized would vary:
//! every reader here reaches its buffers through one place, so a run that
//! names where its rows are rather than holding them is a second [`Chunk`]
//! shape and nothing else.
//!
//! # What a column proves, and when
//!
//! The two doors carry different proofs, and both say so:
//!
//! - [`from_rows`](Serie::from_rows) takes values and proves every one of
//!   them through [`Field::scalar`], the crate's one value contract, so the
//!   rows are the field's own before they are laid out.
//! - [`from_arrow_array`](Serie::from_arrow_array) and its siblings take
//!   buffers and prove the layout and the nullability, which is what buffers
//!   can be asked in constant time. A value those buffers hold that the
//!   field's own contract would refuse - text no [`Currency`](crate::Currency)
//!   accepts, bytes that are not well-known binary - is refused where it is
//!   read, by [`rows`](Serie::rows) or [`get`](Serie::get), and never
//!   silently.
//!
//! That is the same bargain [`ArrowScalar`] makes, and it is why
//! [`Sequence::as_slice`] answers `None` for a column still in its buffers:
//! there is no `&[Scalar]` to lend until something decodes one, and decoding
//! can refuse. [`Sequence::rows`] is the door that decodes and reports.
//!
//! A serie is not a second [`ArrowScalar`]. That type holds one payload in
//! any of four shapes - a pinned row, a column, a table, a one-shot stream -
//! and answers `None` to every native accessor. A serie is one shape, a
//! column, and it reads as the sequence it is.
//!
//! ```
//! use std::sync::Arc;
//!
//! use arrow_array::{ArrayRef, Int64Array};
//! use yggdryl::{DataType, Field, Scalar, Serie};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let field = Field::new("price", DataType::Int64, false);
//! let array: ArrayRef = Arc::new(Int64Array::from(vec![125, 126, 127]));
//!
//! // The buffers cross in as they are, and back out as they were.
//! let mut serie = Serie::from_arrow_array(field, array)?;
//! assert_eq!(serie.len(), 3);
//! assert_eq!(serie.i64_at(1), Some(126));
//! assert_eq!(serie.get(1)?, Scalar::from(126_i64));
//!
//! // And it grows without rebuilding what it already holds.
//! serie.push(Scalar::from(128_i64))?;
//! assert_eq!(serie.len(), 4);
//! assert_eq!(serie.i64_at(3), Some(128));
//!
//! // A column is a value wherever a sequence is one.
//! assert_eq!(Scalar::from(serie).len(), 4);
//! # Ok(())
//! # }
//! ```
//!
//! [`ArrowScalar`]: crate::ArrowScalar
//! [`DataTypeValue`]: crate::DataTypeValue
//! [`DataTypeId`]: crate::DataTypeId
//! [`FieldValue`]: crate::FieldValue
//! [`SerieValue`]: crate::SerieValue
//! [`Value`]: crate::Value
//! [`ArrayRef`]: arrow_array::ArrayRef
//! [`Chunk`]: self

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

use arrow_array::{Array, ArrayRef};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::sequence::Sequence;
use crate::value::{Children, NestedValue, SerieValue, Value};
use crate::{DataType, Field, Result, Scalar, TimeUnit, i256};

/// The rows a column lends when it has none of its own.
const NO_ROWS: &[Scalar] = &[];

// ------------------------------------------------------------------------
// Datatype side: none of its own. A column of one field is a list of that
// field, and `sequence.rs` already owns every list datatype.
// ------------------------------------------------------------------------

// ------------------------------------------------------------------------
// Field side: none of its own. A serie carries the field it is typed by
// rather than being a shape a schema declares.
// ------------------------------------------------------------------------

// ------------------------------------------------------------------------
// Column side: the family and its one leaf.
// ------------------------------------------------------------------------

/// One run of a column's rows, and the row its first row is.
///
/// A run that named where its rows are rather than holding them - a page not
/// read yet, a file not opened - is a second shape here, and nothing else in
/// the file would move: every read reaches a run's buffers through
/// [`Chunk::array`] alone.
#[derive(Clone, Debug)]
enum Chunk {
    /// Rows held as Arrow buffers, in the field's exact physical layout.
    Held {
        /// The row this run starts at, counted from the column's first.
        first: usize,
        /// The buffers.
        array: ArrayRef,
    },
}

impl Chunk {
    /// The row this run starts at.
    const fn first(&self) -> usize {
        match self {
            Self::Held { first, .. } => *first,
        }
    }

    /// The buffers this run holds.
    const fn array(&self) -> &ArrayRef {
        match self {
            Self::Held { array, .. } => array,
        }
    }

    /// How many rows this run holds.
    fn len(&self) -> usize {
        self.array().len()
    }
}

/// The shared body of a column, behind the one pointer a clone copies.
struct ColumnInner {
    /// The field every row is typed by.
    field: Field,
    /// The frozen runs, in row order, each starting where the last ended.
    chunks: Vec<Chunk>,
    /// Rows appended since the last freeze, each already proven by the field.
    tail: Vec<Scalar>,
    /// How many rows the chunks hold, so a length is not a fold.
    frozen: usize,
    /// Every row as a value, decoded once if something asks for one.
    ///
    /// A cache, so it never reaches equality, ordering, hashing, serde or
    /// display; a mutation that changes the rows clears it.
    rows: OnceLock<Arc<[Scalar]>>,
}

impl Clone for ColumnInner {
    /// The cache is not cloned: it is a reading of the rows, and the clone
    /// will decode its own if it is asked to.
    fn clone(&self) -> Self {
        Self {
            field: self.field.clone(),
            chunks: self.chunks.clone(),
            tail: self.tail.clone(),
            frozen: self.frozen,
            rows: OnceLock::new(),
        }
    }
}

/// A column of any length: the field that types it and the rows it holds.
///
/// The rows are Arrow buffers, in chunks, plus whatever has been pushed since
/// the last freeze. The whole body is behind one shared pointer, so cloning a
/// column of a million rows copies eight bytes, and a mutation copies the body
/// only when it is shared.
#[derive(Clone)]
pub struct Column(Arc<ColumnInner>);

impl Column {
    /// The empty column of `field`.
    pub fn empty(field: Field) -> Self {
        Self(Arc::new(ColumnInner {
            field,
            chunks: Vec::new(),
            tail: Vec::new(),
            frozen: 0,
            rows: OnceLock::new(),
        }))
    }

    /// Build a column from the rows a field types.
    ///
    /// Each row goes through [`Field::scalar`], the crate's one value
    /// contract, so a column holds exactly what the field declares - an
    /// integer narrowed to its width, a decimal restated at its scale, a null
    /// only where the column admits one. The rows stay values until buffers
    /// are asked for.
    ///
    /// # Errors
    ///
    /// Returns an error naming the field when a row is not one the field's
    /// datatype accepts, or is null under a field that is not nullable.
    pub fn from_rows(field: Field, rows: impl IntoIterator<Item = Scalar>) -> Result<Self> {
        let tail = rows
            .into_iter()
            .map(|row| field.scalar(row))
            .collect::<Result<Vec<Scalar>>>()?;
        Ok(Self(Arc::new(ColumnInner {
            field,
            chunks: Vec::new(),
            tail,
            frozen: 0,
            rows: OnceLock::new(),
        })))
    }

    /// Pair a field with runs of buffers that already hold its layout.
    ///
    /// Crate-private because the pairing is the caller's claim: every public
    /// door proves the layout and the nullability first.
    fn from_chunks(field: Field, arrays: Vec<ArrayRef>) -> Self {
        let mut first = 0;
        let chunks = arrays
            .into_iter()
            .filter(|array| !array.is_empty())
            .map(|array| {
                let held = Chunk::Held { first, array };
                first += held.len();
                held
            })
            .collect();
        Self(Arc::new(ColumnInner {
            field,
            chunks,
            tail: Vec::new(),
            frozen: first,
            rows: OnceLock::new(),
        }))
    }

    /// Return the field every row of this column is typed by.
    pub fn field(&self) -> &Field {
        &self.0.field
    }

    /// Return the number of rows.
    pub fn len(&self) -> usize {
        self.0.frozen + self.0.tail.len()
    }

    /// Return whether this column holds no rows.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Return the datatype this column materializes into.
    ///
    /// The field is carried, so this is a read rather than an agreement
    /// computed back out of the rows: an empty column names its datatype
    /// where an empty sequence cannot.
    ///
    /// # Errors
    ///
    /// Infallible today; the signature is [`Value::dtype`]'s, which a leaf
    /// whose parameters cannot describe a column answers a refusal through.
    pub fn dtype(&self) -> Result<DataType> {
        Ok(DataType::list(self.0.field.clone()))
    }

    // --------------------------------------------------------------------
    // Reading: where a row is, and what it holds.
    // --------------------------------------------------------------------

    /// The run holding row `index`, and the row's offset inside it.
    ///
    /// One binary search over the chunks; a row in the unfrozen tail answers
    /// `None` and is read from the tail instead.
    fn locate(&self, index: usize) -> Option<(&Chunk, usize)> {
        if index >= self.0.frozen {
            return None;
        }
        let found = self
            .0
            .chunks
            .binary_search_by(|chunk| {
                if chunk.first() > index {
                    Ordering::Greater
                } else if chunk.first() + chunk.len() <= index {
                    Ordering::Less
                } else {
                    Ordering::Equal
                }
            })
            .ok()?;
        let chunk = self.0.chunks.get(found)?;
        Some((chunk, index - chunk.first()))
    }

    /// Borrow the value at `index` when it is still an unfrozen row.
    fn tail_at(&self, index: usize) -> Option<&Scalar> {
        self.0.tail.get(index.checked_sub(self.0.frozen)?)
    }

    /// Return whether row `index` holds no value.
    ///
    /// A row past the end holds no value either, which is what a reader
    /// walking a column against a longer one needs.
    pub fn is_null(&self, index: usize) -> bool {
        if let Some(value) = self.tail_at(index) {
            return value.is_null();
        }
        self.locate(index)
            .is_none_or(|(chunk, offset)| chunk.array().is_null(offset))
    }

    /// Return row `index`, or [`Scalar::Null`] past the end.
    ///
    /// # Errors
    ///
    /// Returns an error when the buffers hold a value the field's own
    /// contract refuses - text no code accepts, bytes that are not
    /// well-known binary - which is the proof a buffer import defers.
    pub fn get(&self, index: usize) -> Result<Scalar> {
        if let Some(value) = self.tail_at(index) {
            return Ok(value.clone());
        }
        let Some((chunk, offset)) = self.locate(index) else {
            return Ok(Scalar::Null);
        };
        Ok(crate::arrow::value::value_from_array(
            self.0.field.dtype(),
            chunk.array().as_ref(),
            offset,
        )?)
    }

    /// Borrow the rows when they are already values, without decoding.
    ///
    /// A column that has never been frozen lends its rows; one holding
    /// buffers answers `None`, because there is no `&[Scalar]` in it to lend.
    /// [`Self::rows`] is the door that decodes.
    pub fn as_slice(&self) -> Option<&[Scalar]> {
        match () {
            () if self.0.chunks.is_empty() => Some(&self.0.tail),
            () => self.0.rows.get().map(AsRef::as_ref),
        }
    }

    /// Return every row as a value, decoding the buffers once.
    ///
    /// The decode is cached, so a second reader pays nothing; a mutation that
    /// changes the rows clears it.
    ///
    /// # Errors
    ///
    /// [`Self::get`] carries the rule, and names the row it refused.
    pub fn rows(&self) -> Result<&[Scalar]> {
        if let Some(rows) = self.as_slice() {
            return Ok(rows);
        }
        let decoded = (0..self.len())
            .map(|index| self.get(index))
            .collect::<Result<Arc<[Scalar]>>>()?;
        Ok(self.0.rows.get_or_init(|| decoded))
    }

    // --------------------------------------------------------------------
    // Typed reading: one buffer offset, no value built. Each reader spans
    // the widths its family spans, the way `Scalar::as_i128` and its
    // siblings do, so shared logic never branches on the leaf.
    // --------------------------------------------------------------------

    /// The buffers holding row `index`, and its offset in them.
    fn array_at(&self, index: usize) -> Option<(&dyn Array, usize)> {
        let (chunk, offset) = self.locate(index)?;
        Some((chunk.array().as_ref(), offset))
    }

    /// Read row `index` of any signed or unsigned integer column.
    pub fn i128_at(&self, index: usize) -> Option<i128> {
        if let Some(value) = self.tail_at(index) {
            return value.as_i128();
        }
        let (array, offset) = self.array_at(index)?;
        typed::integer_at(array, offset)
    }

    /// Read row `index` of an integer column that fits 64 bits.
    pub fn i64_at(&self, index: usize) -> Option<i64> {
        i64::try_from(self.i128_at(index)?).ok()
    }

    /// Read row `index` of any floating column, widened to binary64.
    pub fn f64_at(&self, index: usize) -> Option<f64> {
        if let Some(value) = self.tail_at(index) {
            return value.as_f64();
        }
        let (array, offset) = self.array_at(index)?;
        typed::floating_at(array, offset)
    }

    /// Read row `index` of any boolean column.
    pub fn bool_at(&self, index: usize) -> Option<bool> {
        if let Some(value) = self.tail_at(index) {
            return value.as_bool();
        }
        let (array, offset) = self.array_at(index)?;
        typed::boolean_at(array, offset)
    }

    /// Borrow row `index` of any text column, without copying it.
    pub fn str_at(&self, index: usize) -> Option<&str> {
        if let Some(value) = self.tail_at(index) {
            return value.as_str();
        }
        let (array, offset) = self.array_at(index)?;
        typed::text_at(array, offset)
    }

    /// Borrow row `index` of any byte column, without copying it.
    pub fn bytes_at(&self, index: usize) -> Option<&[u8]> {
        if let Some(value) = self.tail_at(index) {
            return value.as_bytes();
        }
        let (array, offset) = self.array_at(index)?;
        typed::bytes_at(array, offset)
    }

    /// Read row `index` of any exact-decimal column as coefficient and scale.
    pub fn decimal_at(&self, index: usize) -> Option<(i256, i8)> {
        if let Some(value) = self.tail_at(index) {
            return value.as_decimal();
        }
        let (array, offset) = self.array_at(index)?;
        typed::decimal_at(array, offset)
    }

    /// Read row `index` of any temporal column as its stored count and unit.
    pub fn temporal_at(&self, index: usize) -> Option<(i64, TimeUnit)> {
        if let Some(value) = self.tail_at(index) {
            return Some((value.temporal_count()?, value.temporal_unit()?));
        }
        let (array, offset) = self.array_at(index)?;
        typed::temporal_at(array, offset)
    }

    // --------------------------------------------------------------------
    // The buffers themselves.
    // --------------------------------------------------------------------

    /// Return how many frozen runs this column holds.
    ///
    /// Rows pushed since the last freeze are not one of them; they are the
    /// tail, and [`Self::freeze`] is what makes them a run.
    pub fn chunk_count(&self) -> usize {
        self.0.chunks.len()
    }

    /// Borrow one frozen run's buffers.
    pub fn chunk(&self, index: usize) -> Option<&ArrayRef> {
        self.0.chunks.get(index).map(Chunk::array)
    }

    /// Borrow every frozen run's buffers, in row order.
    pub fn chunks(&self) -> impl ExactSizeIterator<Item = &ArrayRef> {
        self.0.chunks.iter().map(Chunk::array)
    }

    /// Return how many rows are held as buffers rather than as values.
    pub fn frozen_len(&self) -> usize {
        self.0.frozen
    }
}

/// Typed reads straight off one Arrow array, each spanning the widths its
/// family spans.
///
/// Inline because these read `arrow_schema::DataType` while the rest of the
/// file reads [`crate::DataType`], and one scope cannot hold both names.
mod typed {
    use arrow_array::{Array, BooleanArray, cast::AsArray, types};
    use arrow_schema::DataType as ArrowDataType;

    use crate::{TimeUnit, i256};

    /// Row `index` of any integer array, widened to 128 bits.
    pub(super) fn integer_at(array: &dyn Array, index: usize) -> Option<i128> {
        if array.is_null(index) {
            return None;
        }
        Some(match array.data_type() {
            ArrowDataType::Int8 => i128::from(array.as_primitive::<types::Int8Type>().value(index)),
            ArrowDataType::Int16 => {
                i128::from(array.as_primitive::<types::Int16Type>().value(index))
            }
            ArrowDataType::Int32 => {
                i128::from(array.as_primitive::<types::Int32Type>().value(index))
            }
            ArrowDataType::Int64 => {
                i128::from(array.as_primitive::<types::Int64Type>().value(index))
            }
            ArrowDataType::UInt8 => {
                i128::from(array.as_primitive::<types::UInt8Type>().value(index))
            }
            ArrowDataType::UInt16 => {
                i128::from(array.as_primitive::<types::UInt16Type>().value(index))
            }
            ArrowDataType::UInt32 => {
                i128::from(array.as_primitive::<types::UInt32Type>().value(index))
            }
            ArrowDataType::UInt64 => {
                i128::from(array.as_primitive::<types::UInt64Type>().value(index))
            }
            _ => return None,
        })
    }

    /// Row `index` of any floating array, widened to binary64.
    pub(super) fn floating_at(array: &dyn Array, index: usize) -> Option<f64> {
        if array.is_null(index) {
            return None;
        }
        Some(match array.data_type() {
            ArrowDataType::Float16 => {
                f64::from(array.as_primitive::<types::Float16Type>().value(index))
            }
            ArrowDataType::Float32 => {
                f64::from(array.as_primitive::<types::Float32Type>().value(index))
            }
            ArrowDataType::Float64 => array.as_primitive::<types::Float64Type>().value(index),
            _ => return None,
        })
    }

    /// Row `index` of a boolean array.
    pub(super) fn boolean_at(array: &dyn Array, index: usize) -> Option<bool> {
        if array.is_null(index) {
            return None;
        }
        array
            .as_any()
            .downcast_ref::<BooleanArray>()
            .map(|values| values.value(index))
    }

    /// Row `index` of any text array, borrowed rather than copied.
    pub(super) fn text_at(array: &dyn Array, index: usize) -> Option<&str> {
        if array.is_null(index) {
            return None;
        }
        match array.data_type() {
            ArrowDataType::Utf8 => Some(array.as_string::<i32>().value(index)),
            ArrowDataType::LargeUtf8 => Some(array.as_string::<i64>().value(index)),
            ArrowDataType::Utf8View => Some(array.as_string_view().value(index)),
            _ => None,
        }
    }

    /// Row `index` of any byte array, borrowed rather than copied.
    pub(super) fn bytes_at(array: &dyn Array, index: usize) -> Option<&[u8]> {
        if array.is_null(index) {
            return None;
        }
        match array.data_type() {
            ArrowDataType::Binary => Some(array.as_binary::<i32>().value(index)),
            ArrowDataType::LargeBinary => Some(array.as_binary::<i64>().value(index)),
            ArrowDataType::BinaryView => Some(array.as_binary_view().value(index)),
            ArrowDataType::FixedSizeBinary(_) => Some(array.as_fixed_size_binary().value(index)),
            _ => None,
        }
    }

    /// Row `index` of any exact-decimal array, as coefficient and scale.
    pub(super) fn decimal_at(array: &dyn Array, index: usize) -> Option<(i256, i8)> {
        if array.is_null(index) {
            return None;
        }
        Some(match *array.data_type() {
            ArrowDataType::Decimal32(_, scale) => (
                i256::from_i128(i128::from(
                    array.as_primitive::<types::Decimal32Type>().value(index),
                )),
                scale,
            ),
            ArrowDataType::Decimal64(_, scale) => (
                i256::from_i128(i128::from(
                    array.as_primitive::<types::Decimal64Type>().value(index),
                )),
                scale,
            ),
            ArrowDataType::Decimal128(_, scale) => (
                i256::from_i128(array.as_primitive::<types::Decimal128Type>().value(index)),
                scale,
            ),
            ArrowDataType::Decimal256(_, scale) => (
                i256::from_le_bytes(
                    array
                        .as_primitive::<types::Decimal256Type>()
                        .value(index)
                        .to_le_bytes(),
                ),
                scale,
            ),
            _ => return None,
        })
    }

    /// Row `index` of any temporal array, as its stored count and unit.
    pub(super) fn temporal_at(array: &dyn Array, index: usize) -> Option<(i64, TimeUnit)> {
        if array.is_null(index) {
            return None;
        }
        Some(match array.data_type() {
            ArrowDataType::Date32 => (
                i64::from(array.as_primitive::<types::Date32Type>().value(index)),
                TimeUnit::Day,
            ),
            ArrowDataType::Date64 => (
                array.as_primitive::<types::Date64Type>().value(index),
                TimeUnit::Millisecond,
            ),
            ArrowDataType::Time32(unit) => (
                i64::from(match unit {
                    arrow_schema::TimeUnit::Second => {
                        array.as_primitive::<types::Time32SecondType>().value(index)
                    }
                    _ => array
                        .as_primitive::<types::Time32MillisecondType>()
                        .value(index),
                }),
                TimeUnit::from_arrow_time(*unit),
            ),
            ArrowDataType::Time64(unit) => (
                match unit {
                    arrow_schema::TimeUnit::Microsecond => array
                        .as_primitive::<types::Time64MicrosecondType>()
                        .value(index),
                    _ => array
                        .as_primitive::<types::Time64NanosecondType>()
                        .value(index),
                },
                TimeUnit::from_arrow_time(*unit),
            ),
            ArrowDataType::Timestamp(unit, _) => (
                match unit {
                    arrow_schema::TimeUnit::Second => array
                        .as_primitive::<types::TimestampSecondType>()
                        .value(index),
                    arrow_schema::TimeUnit::Millisecond => array
                        .as_primitive::<types::TimestampMillisecondType>()
                        .value(index),
                    arrow_schema::TimeUnit::Microsecond => array
                        .as_primitive::<types::TimestampMicrosecondType>()
                        .value(index),
                    arrow_schema::TimeUnit::Nanosecond => array
                        .as_primitive::<types::TimestampNanosecondType>()
                        .value(index),
                },
                TimeUnit::from_arrow_time(*unit),
            ),
            ArrowDataType::Duration(unit) => (
                match unit {
                    arrow_schema::TimeUnit::Second => array
                        .as_primitive::<types::DurationSecondType>()
                        .value(index),
                    arrow_schema::TimeUnit::Millisecond => array
                        .as_primitive::<types::DurationMillisecondType>()
                        .value(index),
                    arrow_schema::TimeUnit::Microsecond => array
                        .as_primitive::<types::DurationMicrosecondType>()
                        .value(index),
                    arrow_schema::TimeUnit::Nanosecond => array
                        .as_primitive::<types::DurationNanosecondType>()
                        .value(index),
                },
                TimeUnit::from_arrow_time(*unit),
            ),
            _ => return None,
        })
    }
}

/// The column family's value payload.
///
/// One leaf today - the rows of one field - because that is the one thing a
/// column is. A second layout is a leaf beside it, not a new [`Scalar`]
/// variant and not a new spelling at every call site.
#[derive(Clone)]
#[non_exhaustive]
pub enum Serie {
    /// A column of any length: the field that types it and the rows it holds.
    Column(Column),
}

impl Serie {
    /// The empty column of `field`.
    pub fn empty(field: Field) -> Self {
        Self::Column(Column::empty(field))
    }

    /// Build a column from the rows a field types.
    ///
    /// # Errors
    ///
    /// [`Column::from_rows`] carries the rule.
    pub fn from_rows(field: Field, rows: impl IntoIterator<Item = Scalar>) -> Result<Self> {
        Column::from_rows(field, rows).map(Self::Column)
    }

    /// Return the column value when this is that leaf.
    pub const fn as_column(&self) -> Option<&Column> {
        match self {
            Self::Column(value) => Some(value),
        }
    }

    /// Return the field every row of this column is typed by.
    pub fn field(&self) -> &Field {
        match self {
            Self::Column(value) => value.field(),
        }
    }

    /// Return the number of rows.
    pub fn len(&self) -> usize {
        match self {
            Self::Column(value) => value.len(),
        }
    }

    /// Return whether this column holds no rows.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Return the datatype this column materializes into.
    ///
    /// # Errors
    ///
    /// [`Column::dtype`] carries the rule.
    pub fn dtype(&self) -> Result<DataType> {
        match self {
            Self::Column(value) => value.dtype(),
        }
    }

    /// Return whether row `index` holds no value.
    pub fn is_null(&self, index: usize) -> bool {
        match self {
            Self::Column(value) => value.is_null(index),
        }
    }

    /// Return row `index`, or [`Scalar::Null`] past the end.
    ///
    /// # Errors
    ///
    /// [`Column::get`] carries the rule.
    pub fn get(&self, index: usize) -> Result<Scalar> {
        match self {
            Self::Column(value) => value.get(index),
        }
    }

    /// Borrow the rows when they are already values, without decoding.
    pub fn as_slice(&self) -> Option<&[Scalar]> {
        match self {
            Self::Column(value) => value.as_slice(),
        }
    }

    /// Return every row as a value, decoding the buffers once.
    ///
    /// # Errors
    ///
    /// [`Column::rows`] carries the rule.
    pub fn rows(&self) -> Result<&[Scalar]> {
        match self {
            Self::Column(value) => value.rows(),
        }
    }

    /// Return the rows as the schema-free sequence they are.
    ///
    /// The field is what a serie has that a sequence has not, so this is the
    /// one direction that drops something and it is spelled rather than
    /// implied.
    ///
    /// # Errors
    ///
    /// [`Column::rows`] carries the rule.
    pub fn into_sequence(self) -> Result<Scalar> {
        Ok(Scalar::from_sequence(self.rows()?.iter().cloned()))
    }

    /// Return this column under another field, rechecking every row.
    ///
    /// # Errors
    ///
    /// [`Column::rows`] and [`Column::from_rows`] carry the rules.
    pub fn with_field(&self, field: Field) -> Result<Self> {
        Self::from_rows(field, self.rows()?.iter().cloned())
    }
}

// ------------------------------------------------------------------------
// The typed readers, forwarded from the root to whichever leaf holds them.
// ------------------------------------------------------------------------

/// Forward one of the leaf's readers from the root that redirects to it.
macro_rules! forward_reader {
    ($(#[$meta:meta])* $name:ident, $answer:ty) => {
        $(#[$meta])*
        pub fn $name(&self, index: usize) -> Option<$answer> {
            match self {
                Self::Column(value) => value.$name(index),
            }
        }
    };
}

impl Serie {
    forward_reader!(
        /// Read row `index` of any signed or unsigned integer column.
        i128_at,
        i128
    );
    forward_reader!(
        /// Read row `index` of an integer column that fits 64 bits.
        i64_at,
        i64
    );
    forward_reader!(
        /// Read row `index` of any floating column, widened to binary64.
        f64_at,
        f64
    );
    forward_reader!(
        /// Read row `index` of any boolean column.
        bool_at,
        bool
    );
    /// Borrow row `index` of any text column, without copying it.
    pub fn str_at(&self, index: usize) -> Option<&str> {
        match self {
            Self::Column(value) => value.str_at(index),
        }
    }

    /// Borrow row `index` of any byte column, without copying it.
    pub fn bytes_at(&self, index: usize) -> Option<&[u8]> {
        match self {
            Self::Column(value) => value.bytes_at(index),
        }
    }

    forward_reader!(
        /// Read row `index` of any exact-decimal column as coefficient and
        /// scale.
        decimal_at,
        (i256, i8)
    );
    forward_reader!(
        /// Read row `index` of any temporal column as its stored count and
        /// unit.
        temporal_at,
        (i64, TimeUnit)
    );

    /// Return how many frozen runs this column holds.
    pub fn chunk_count(&self) -> usize {
        match self {
            Self::Column(value) => value.chunk_count(),
        }
    }

    /// Borrow one frozen run's buffers.
    pub fn chunk(&self, index: usize) -> Option<&ArrayRef> {
        match self {
            Self::Column(value) => value.chunk(index),
        }
    }

    /// Borrow every frozen run's buffers, in row order.
    pub fn chunks(&self) -> impl ExactSizeIterator<Item = &ArrayRef> {
        match self {
            Self::Column(value) => value.chunks(),
        }
    }

    /// Return how many rows are held as buffers rather than as values.
    pub fn frozen_len(&self) -> usize {
        match self {
            Self::Column(value) => value.frozen_len(),
        }
    }
}

// ------------------------------------------------------------------------
// Equality, order and hash: the field and the rows, never the buffers'
// chunking and never the decode cache, so one column is one value however
// it was assembled.
// ------------------------------------------------------------------------

impl Column {
    /// Order two columns by their field, then their length, then their rows.
    ///
    /// Rows that cannot be decoded order after rows that can, and two columns
    /// whose rows both refuse order equal: a refusal is not a value, and an
    /// order has to be total.
    fn compare(&self, other: &Self) -> Ordering {
        self.0
            .field
            .cmp(&other.0.field)
            .then_with(|| self.len().cmp(&other.len()))
            .then_with(|| match (self.rows(), other.rows()) {
                (Ok(left), Ok(right)) => left.cmp(right),
                (Ok(_), Err(_)) => Ordering::Less,
                (Err(_), Ok(_)) => Ordering::Greater,
                (Err(_), Err(_)) => Ordering::Equal,
            })
    }
}

impl PartialEq for Column {
    fn eq(&self, other: &Self) -> bool {
        self.compare(other) == Ordering::Equal
    }
}

impl Eq for Column {}

impl PartialOrd for Column {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Column {
    fn cmp(&self, other: &Self) -> Ordering {
        self.compare(other)
    }
}

impl Hash for Column {
    /// The field and the length: everything equal columns share without
    /// decoding a buffer, so the hash agrees with [`PartialEq`] and costs
    /// nothing on a column of a million rows.
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.field.hash(state);
        self.len().hash(state);
    }
}

impl fmt::Debug for Column {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Column")
            .field("field", &self.0.field)
            .field("len", &self.len())
            .field("chunks", &self.0.chunks.len())
            .field("unfrozen", &self.0.tail.len())
            .finish()
    }
}

impl fmt::Display for Column {
    /// The field's name and the rows, or the field's name and the refusal
    /// where the buffers hold a value the field does not accept.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.rows() {
            Ok(rows) => write!(formatter, "{}{rows:?}", self.0.field.name()),
            Err(error) => write!(formatter, "{}<{error}>", self.0.field.name()),
        }
    }
}

impl PartialEq for Serie {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Column(left), Self::Column(right)) => left == right,
        }
    }
}

impl Eq for Serie {}

impl PartialOrd for Serie {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Serie {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Column(left), Self::Column(right)) => left.cmp(right),
        }
    }
}

impl Hash for Serie {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::Column(value) => value.hash(state),
        }
    }
}

impl fmt::Debug for Serie {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Column(value) => fmt::Debug::fmt(value, formatter),
        }
    }
}

impl fmt::Display for Serie {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Column(value) => fmt::Display::fmt(value, formatter),
        }
    }
}

// ------------------------------------------------------------------------
// The value contracts: a column is a value, its rows are its children.
// ------------------------------------------------------------------------

impl NestedValue for Column {
    fn len(&self) -> usize {
        Self::len(self)
    }

    fn children(&self) -> Children<'_> {
        // A column whose buffers hold a value its field refuses has no
        // children to lend; `rows` is where that refusal is reported.
        Children::Sequence(self.rows().unwrap_or(NO_ROWS).iter())
    }
}

impl Value for Column {
    fn dtype(&self) -> Result<DataType> {
        Self::dtype(self)
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Sequence(Sequence::Serie(Serie::Column(self)))
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Sequence(Sequence::Serie(Serie::Column(value))) => Some(value),
            _ => None,
        }
    }
}

impl SerieValue for Column {
    fn field(&self) -> &Field {
        Self::field(self)
    }

    fn as_slice(&self) -> Option<&[Scalar]> {
        Self::as_slice(self)
    }

    fn rows(&self) -> Result<&[Scalar]> {
        Self::rows(self)
    }

    fn get(&self, index: usize) -> Result<Scalar> {
        Self::get(self, index)
    }

    fn is_null(&self, index: usize) -> bool {
        Self::is_null(self, index)
    }

    fn into_serie(self) -> Serie {
        Serie::Column(self)
    }

    fn from_serie(value: &Serie) -> Option<&Self> {
        match value {
            Serie::Column(value) => Some(value),
        }
    }
}

impl NestedValue for Serie {
    fn len(&self) -> usize {
        Self::len(self)
    }

    fn children(&self) -> Children<'_> {
        Children::Sequence(self.rows().unwrap_or(NO_ROWS).iter())
    }
}

impl Value for Serie {
    fn dtype(&self) -> Result<DataType> {
        Self::dtype(self)
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Sequence(Sequence::Serie(self))
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Sequence(Sequence::Serie(value)) => Some(value),
            _ => None,
        }
    }
}

impl SerieValue for Serie {
    fn field(&self) -> &Field {
        Self::field(self)
    }

    fn as_slice(&self) -> Option<&[Scalar]> {
        Self::as_slice(self)
    }

    fn rows(&self) -> Result<&[Scalar]> {
        Self::rows(self)
    }

    fn get(&self, index: usize) -> Result<Scalar> {
        Self::get(self, index)
    }

    fn is_null(&self, index: usize) -> bool {
        Self::is_null(self, index)
    }

    fn into_serie(self) -> Self {
        self
    }

    fn from_serie(value: &Self) -> Option<&Self> {
        Some(value)
    }
}

impl From<Column> for Serie {
    fn from(value: Column) -> Self {
        Self::Column(value)
    }
}

impl From<Column> for Scalar {
    fn from(value: Column) -> Self {
        Value::into_scalar(value)
    }
}

impl From<Serie> for Scalar {
    fn from(value: Serie) -> Self {
        Value::into_scalar(value)
    }
}

// ------------------------------------------------------------------------
// The serde shadow: a column is its field and its rows, which is the one
// thing the sequence wire cannot write for it.
// ------------------------------------------------------------------------

/// The private serde shadow of a [`Column`], which is a [`Serie`]'s too.
#[derive(Deserialize, Serialize)]
struct SerieWire {
    field: Field,
    rows: Vec<Scalar>,
}

impl SerieWire {
    /// Read the column this document describes, rechecking every row.
    ///
    /// A document is data from outside, so the rows it carries meet the
    /// field's own contract here rather than being taken on its word.
    ///
    /// # Errors
    ///
    /// [`Column::from_rows`] carries the rule.
    fn into_column(self) -> Result<Column> {
        Column::from_rows(self.field, self.rows)
    }
}

impl TryFrom<&Column> for SerieWire {
    type Error = crate::Error;

    fn try_from(value: &Column) -> Result<Self> {
        Ok(Self {
            field: value.field().clone(),
            rows: value.rows()?.to_vec(),
        })
    }
}

impl Serialize for Column {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::Error as _;

        SerieWire::try_from(self)
            .map_err(S::Error::custom)?
            .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Column {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        use serde::de::Error as _;

        SerieWire::deserialize(deserializer)?
            .into_column()
            .map_err(D::Error::custom)
    }
}

impl Serialize for Serie {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        match self {
            Self::Column(value) => value.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for Serie {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        Column::deserialize(deserializer).map(Self::Column)
    }
}

// ------------------------------------------------------------------------
// Growing: rows appended as values land in the tail, runs appended as
// buffers become chunks. Neither rebuilds what the column already holds.
// ------------------------------------------------------------------------

impl Column {
    /// The body, copied only where this column's is shared.
    ///
    /// The decoded reading is dropped here rather than at each caller: every
    /// mutation reaches the rows through this one door, so a stale reading
    /// cannot outlive the rows it read.
    fn inner_mut(&mut self) -> &mut ColumnInner {
        let inner = Arc::make_mut(&mut self.0);
        inner.rows = OnceLock::new();
        inner
    }

    /// Append one row, proven by this column's field.
    ///
    /// The row lands in the unfrozen tail, so a push costs one value and
    /// never one array.
    ///
    /// # Errors
    ///
    /// [`Field::scalar`] carries the rule.
    pub fn push(&mut self, value: impl Into<Scalar>) -> Result<()> {
        let row = self.0.field.scalar(value.into())?;
        self.inner_mut().tail.push(row);
        Ok(())
    }

    /// Append every row of `rows`, each proven by this column's field.
    ///
    /// Nothing is appended when one of them is refused: the rows are proven
    /// before any of them lands, so a refusal leaves the column as it was.
    ///
    /// # Errors
    ///
    /// [`Field::scalar`] carries the rule.
    pub fn extend(&mut self, rows: impl IntoIterator<Item = Scalar>) -> Result<()> {
        let proven = rows
            .into_iter()
            .map(|row| self.0.field.scalar(row))
            .collect::<Result<Vec<Scalar>>>()?;
        self.inner_mut().tail.extend(proven);
        Ok(())
    }

    /// A column over `len` rows starting at `offset`, sharing the buffers.
    ///
    /// One slice per run the range touches, and Arrow's own slice shares the
    /// buffer rather than copying it, so a window over a million rows costs
    /// the runs it spans and nothing per row.
    pub fn slice(&self, offset: usize, len: usize) -> Self {
        let end = offset.saturating_add(len).min(self.len());
        let offset = offset.min(end);
        let mut arrays = Vec::new();
        for run in &self.0.chunks {
            let start = run.first().max(offset);
            let stop = (run.first() + run.len()).min(end);
            if start < stop {
                arrays.push(run.array().slice(start - run.first(), stop - start));
            }
        }
        let mut sliced = Self::from_chunks(self.0.field.clone(), arrays);
        let tail_from = offset.saturating_sub(self.0.frozen);
        let tail_to = end.saturating_sub(self.0.frozen);
        if tail_from < tail_to {
            let inner = sliced.inner_mut();
            inner
                .tail
                .extend_from_slice(&self.0.tail[tail_from..tail_to]);
        }
        sliced
    }

    /// Drop every row past `len`.
    pub fn truncate(&mut self, len: usize) {
        if len >= self.len() {
            return;
        }
        let inner = self.inner_mut();
        if len >= inner.frozen {
            inner.tail.truncate(len - inner.frozen);
            return;
        }
        inner.tail.clear();
        inner.chunks.retain(|chunk| chunk.first() < len);
        if let Some(last) = inner.chunks.last_mut() {
            let kept = len - last.first();
            if kept < last.len() {
                *last = Chunk::Held {
                    first: last.first(),
                    array: last.array().slice(0, kept),
                };
            }
        }
        inner.frozen = len;
    }
}

// ------------------------------------------------------------------------
// Arrow: the buffers a column holds, both directions, and the runs it grows
// by. Every crossing routes through the crate's single scalar-array
// boundary, and none of them walks a row it does not have to.
// ------------------------------------------------------------------------

mod arrow {
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, RecordBatch, RecordBatchReader, StructArray};
    use arrow_schema::Fields;

    use super::{Chunk, Column, Serie};
    use crate::arrow::scalars::require_layout;
    use crate::arrow::value::array_from_values;
    use crate::arrow::{
        BatchReader, Error, Result, arrow_schema_from_field, batch_reader, field_from_arrow_schema,
    };
    use crate::media::DEFAULT_ROOT_NAME;
    use crate::{Field, Scalar};

    /// Refuse a run whose layout or nullability is not the field's.
    ///
    /// Both are constant-time reads off the array, which is what a buffer
    /// import can prove without decoding a row; the values themselves are the
    /// field's own contract and are proven where they are read.
    fn require_run(field: &Field, array: &dyn Array) -> Result<()> {
        require_layout(field, array)?;
        if !field.is_nullable() && array.logical_null_count() > 0 {
            return Err(Error::RequiredField {
                path: smol_str::format_smolstr!("$.{}", field.name()),
                nulls: Some(array.logical_null_count()),
            });
        }
        Ok(())
    }

    impl Column {
        /// Take one run of Arrow buffers as the column of `field`.
        ///
        /// Nothing is copied and no row is read: the layout and the
        /// nullability are proven here, and the values are proven where they
        /// are read.
        ///
        /// # Errors
        ///
        /// Returns an error when the array does not hold `field`'s exact
        /// physical layout, or carries a null a required column cannot.
        pub fn from_arrow_array(field: Field, array: ArrayRef) -> Result<Self> {
            Self::from_arrow_chunks(field, vec![array])
        }

        /// Take several runs of Arrow buffers, in row order, as one column.
        ///
        /// # Errors
        ///
        /// [`Self::from_arrow_array`] carries the rule, for each run.
        pub fn from_arrow_chunks(field: Field, arrays: Vec<ArrayRef>) -> Result<Self> {
            for array in &arrays {
                require_run(&field, array.as_ref())?;
            }
            Ok(Self::from_chunks(field, arrays))
        }

        /// Append one run of Arrow buffers, copying no row.
        ///
        /// Rows pushed since the last freeze are frozen first, so the runs
        /// stay in row order.
        ///
        /// # Errors
        ///
        /// [`Self::from_arrow_array`] carries the rule, and freezing the tail
        /// carries [`Self::freeze`]'s.
        pub fn append_arrow_array(&mut self, array: ArrayRef) -> Result<()> {
            require_run(self.field(), array.as_ref())?;
            self.freeze()?;
            let inner = self.inner_mut();
            let first = inner.frozen;
            inner.frozen += array.len();
            inner.chunks.push(Chunk::Held { first, array });
            Ok(())
        }

        /// Append every row of `other`, copying no row of either.
        ///
        /// # Errors
        ///
        /// Returns an error when the two columns are not typed by one field,
        /// or when either tail cannot be frozen.
        pub fn append(&mut self, other: &Self) -> Result<()> {
            if self.field().dtype() != other.field().dtype() {
                return Err(Error::IncompatibleSchema(format!(
                    "a column of {} does not append to a column of {}",
                    other.field().dtype(),
                    self.field().dtype(),
                )));
            }
            let mut appended = other.clone();
            appended.freeze()?;
            for run in appended.chunks() {
                self.append_arrow_array(ArrayRef::clone(run))?;
            }
            Ok(())
        }

        /// Freeze the rows pushed since the last freeze into one run.
        ///
        /// The rows cross at the crate's one scalar-array boundary as they
        /// are: this field proved them when they were pushed, and the one
        /// value contract is never re-run over what it answered.
        ///
        /// # Errors
        ///
        /// Returns an error when the field has no Arrow projection, or when a
        /// row cannot be laid out in it.
        pub fn freeze(&mut self) -> Result<()> {
            if self.0.tail.is_empty() {
                return Ok(());
            }
            let borrowed: Vec<&Scalar> = self.0.tail.iter().collect();
            let array = array_from_values(self.field(), &borrowed)?;
            let inner = self.inner_mut();
            let first = inner.frozen;
            inner.frozen += array.len();
            inner.chunks.push(Chunk::Held { first, array });
            inner.tail.clear();
            Ok(())
        }

        /// Gather every run into one, so the column is one set of buffers.
        ///
        /// # Errors
        ///
        /// [`Self::freeze`] carries the rule, and Arrow's own concatenation
        /// carries the rest.
        pub fn compact(&mut self) -> Result<()> {
            self.freeze()?;
            if self.0.chunks.len() < 2 {
                return Ok(());
            }
            let gathered = self.one_array()?;
            let inner = self.inner_mut();
            inner.frozen = gathered.len();
            inner.chunks = vec![Chunk::Held {
                first: 0,
                array: gathered,
            }];
            Ok(())
        }

        /// Every row of this column as one run of buffers.
        ///
        /// A column already holding one run lends it; one holding several
        /// concatenates them, which is the only time a row is copied.
        ///
        /// # Errors
        ///
        /// [`Self::freeze`] carries the rule.
        pub fn into_arrow_array(&self) -> Result<ArrayRef> {
            if self.0.tail.is_empty() {
                return self.one_array();
            }
            let mut frozen = self.clone();
            frozen.freeze()?;
            frozen.one_array()
        }

        /// Every frozen run of this column, in row order, sharing its buffers.
        ///
        /// # Errors
        ///
        /// [`Self::freeze`] carries the rule.
        pub fn into_arrow_chunks(&self) -> Result<Vec<ArrayRef>> {
            let mut frozen = self.clone();
            frozen.freeze()?;
            Ok(frozen.chunks().map(ArrayRef::clone).collect())
        }

        /// The frozen runs gathered into one, without freezing the tail.
        fn one_array(&self) -> Result<ArrayRef> {
            match self.0.chunks.as_slice() {
                [] => Ok(arrow_array::new_empty_array(
                    self.field().clone().into_arrow_field_ref()?.data_type(),
                )),
                [one] => Ok(ArrayRef::clone(one.array())),
                many => {
                    let borrowed: Vec<&dyn Array> =
                        many.iter().map(|run| run.array().as_ref()).collect();
                    Ok(arrow_select::concat::concat(&borrowed)?)
                }
            }
        }

        /// Every row of this column as one Arrow table.
        ///
        /// The field is the record root here, so it must be a bounded,
        /// non-null Struct field and every row a record under it.
        ///
        /// # Errors
        ///
        /// Returns an error unless the field is a record root, or when a row
        /// cannot be laid out in it.
        pub fn into_arrow_batch(&self) -> Result<RecordBatch> {
            let schema = arrow_schema_from_field(self.field())?;
            let rows = self.into_arrow_array()?;
            let Some(records) = rows.as_any().downcast_ref::<StructArray>() else {
                return Err(Error::Internal {
                    site: "serie::into_arrow_batch",
                });
            };
            Ok(RecordBatch::try_new_with_options(
                schema,
                records.columns().to_vec(),
                &arrow_array::RecordBatchOptions::new().with_row_count(Some(rows.len())),
            )?)
        }

        /// Read one Arrow table as the column of its rows.
        ///
        /// The root is named [`DEFAULT_ROOT_NAME`], because Arrow names
        /// columns and never the record. Nothing is copied: the batch's own
        /// columns become one run.
        ///
        /// # Errors
        ///
        /// Returns an error when the batch schema does not project to a
        /// Yggdryl Struct root.
        pub fn from_arrow_batch(batch: &RecordBatch) -> Result<Self> {
            let root = field_from_arrow_schema(DEFAULT_ROOT_NAME, batch.schema().as_ref())?;
            Self::from_arrow_array(root, Arc::new(StructArray::from(batch.clone())))
        }

        /// Stream this column's rows, one batch per run.
        ///
        /// Nothing is concatenated and nothing is decoded: a column of a
        /// hundred appended batches streams back as a hundred batches.
        ///
        /// # Errors
        ///
        /// [`Self::into_arrow_batch`] carries the rule.
        pub fn into_arrow_reader(&self) -> Result<BatchReader> {
            let schema = arrow_schema_from_field(self.field())?;
            let mut frozen = self.clone();
            frozen.freeze()?;
            let mut batches = Vec::with_capacity(frozen.chunk_count());
            for run in frozen.chunks() {
                let Some(records) = run.as_any().downcast_ref::<StructArray>() else {
                    return Err(Error::Internal {
                        site: "serie::into_arrow_reader",
                    });
                };
                batches.push(RecordBatch::try_new_with_options(
                    Arc::clone(&schema),
                    records.columns().to_vec(),
                    &arrow_array::RecordBatchOptions::new().with_row_count(Some(run.len())),
                )?);
            }
            Ok(batch_reader(schema, batches))
        }

        /// Take one Arrow batch stream as the column of its rows.
        ///
        /// Each batch becomes one run, so a stream crosses without a row
        /// being decoded and without the batches being gathered.
        ///
        /// # Errors
        ///
        /// Returns an error when the reported schema does not project to a
        /// Yggdryl Struct root, or when the reader fails.
        pub fn from_arrow_reader(reader: BatchReader) -> Result<Self> {
            let root = field_from_arrow_schema(DEFAULT_ROOT_NAME, reader.schema().as_ref())?;
            let mut runs: Vec<ArrayRef> = Vec::new();
            for batch in reader {
                runs.push(Arc::new(StructArray::from(batch?)));
            }
            Self::from_arrow_chunks(root, runs)
        }
    }

    // --------------------------------------------------------------------
    // Nested: the columns a Struct-typed column is made of. Adding or
    // dropping one is one array per run and never one row.
    // --------------------------------------------------------------------

    impl Column {
        /// The Arrow children a Struct-typed column projects to.
        ///
        /// # Errors
        ///
        /// Returns an error unless this column's field is a record.
        fn record_fields(&self) -> Result<Fields> {
            match self.field().clone().into_arrow_field_ref()?.data_type() {
                arrow_schema::DataType::Struct(fields) => Ok(fields.clone()),
                other => Err(Error::InvalidRootField {
                    role: "column children",
                    url: None,
                    name: smol_str::SmolStr::new(self.field().name()),
                    expected: smol_str::SmolStr::new_static("a struct datatype"),
                    actual: smol_str::format_smolstr!("{other}"),
                }),
            }
        }

        /// Every run of this column, read as the record it is.
        ///
        /// # Errors
        ///
        /// [`Self::record_fields`] carries the rule, and freezing the tail
        /// carries [`Self::freeze`]'s.
        fn records(&self) -> Result<Vec<StructArray>> {
            self.record_fields()?;
            let mut frozen = self.clone();
            frozen.freeze()?;
            frozen
                .chunks()
                .map(|run| {
                    run.as_any()
                        .downcast_ref::<StructArray>()
                        .cloned()
                        .ok_or(Error::Internal {
                            site: "serie::records",
                        })
                })
                .collect()
        }

        /// Borrow one child of a Struct-typed column as a column of its own.
        ///
        /// One array per run, shared rather than copied, and never one row.
        ///
        /// # Errors
        ///
        /// Returns an error unless this column is a record with a child of
        /// that name.
        pub fn child(&self, name: &str) -> Result<Self> {
            let index = self.field().index_of(name).ok_or_else(|| {
                Error::IncompatibleSchema(format!("no child named {name:?} in this column"))
            })?;
            self.child_at(index)
        }

        /// Borrow child `index` of a Struct-typed column as a column of its
        /// own.
        ///
        /// # Errors
        ///
        /// [`Self::child`] carries the rule.
        pub fn child_at(&self, index: usize) -> Result<Self> {
            let child = self
                .field()
                .get_field_at(index)
                .ok_or_else(|| {
                    Error::IncompatibleSchema(format!("this column has no child {index}"))
                })?
                .clone();
            let arrays = self
                .records()?
                .iter()
                .map(|run| {
                    run.columns().get(index).map(ArrayRef::clone).ok_or({
                        Error::Internal {
                            site: "serie::child_at",
                        }
                    })
                })
                .collect::<Result<Vec<ArrayRef>>>()?;
            Ok(Self::from_chunks(child, arrays))
        }

        /// Return this column with `child` added, or replacing the child of
        /// its name.
        ///
        /// The children that stay are shared, not rebuilt, and no row is
        /// read. The two columns need not be cut into the same runs, so both
        /// are gathered into one first; that is the only copy, and it is one
        /// per buffer rather than one per row.
        ///
        /// # Errors
        ///
        /// Returns an error unless this column is a record and `child` holds
        /// exactly as many rows as it does.
        pub fn with_child(&self, child: &Self) -> Result<Self> {
            if child.len() != self.len() {
                return Err(Error::IncompatibleSchema(format!(
                    "a child of {} rows does not fit a column of {} rows",
                    child.len(),
                    self.len(),
                )));
            }
            let mut field = self.field().clone();
            field.set_field(child.field().name(), child.field().clone())?;
            let index = field
                .index_of(child.field().name())
                .ok_or(Error::Internal {
                    site: "serie::with_child",
                })?;

            let mut records = self.clone();
            records.compact()?;
            let runs = records.records()?;
            let nulls = runs.first().and_then(|run| run.nulls().cloned());
            let mut columns = runs
                .first()
                .map_or_else(Vec::new, |run| run.columns().to_vec());
            let array = child.into_arrow_array()?;
            if index < columns.len() {
                columns[index] = array;
            } else {
                columns.push(array);
            }
            Self::rebuilt(field, columns, self.len(), nulls)
        }

        /// Return this column with the child of `name` dropped.
        ///
        /// The children that stay are shared, not rebuilt, and no row is
        /// read: each run keeps its own buffers.
        ///
        /// # Errors
        ///
        /// Returns an error unless this column is a record with a child of
        /// that name.
        pub fn without_child(&self, name: &str) -> Result<Self> {
            let mut field = self.field().clone();
            let index = field.index_of(name).ok_or_else(|| {
                Error::IncompatibleSchema(format!("no child named {name:?} in this column"))
            })?;
            field.remove_field_at(index)?;
            let kept = match field.clone().into_arrow_field_ref()?.data_type() {
                arrow_schema::DataType::Struct(fields) => fields.clone(),
                _ => {
                    return Err(Error::Internal {
                        site: "serie::without_child",
                    });
                }
            };
            let mut arrays: Vec<ArrayRef> = Vec::new();
            for run in self.records()? {
                let mut columns = run.columns().to_vec();
                columns.remove(index);
                arrays.push(record_array(
                    &kept,
                    columns,
                    run.len(),
                    run.nulls().cloned(),
                )?);
            }
            Self::from_arrow_chunks(field, arrays)
        }

        /// One column over `field`, out of one run's child arrays.
        fn rebuilt(
            field: Field,
            columns: Vec<ArrayRef>,
            rows: usize,
            nulls: Option<arrow_buffer::NullBuffer>,
        ) -> Result<Self> {
            let fields = match field.clone().into_arrow_field_ref()?.data_type() {
                arrow_schema::DataType::Struct(fields) => fields.clone(),
                _ => {
                    return Err(Error::Internal {
                        site: "serie::rebuilt",
                    });
                }
            };
            let array = record_array(&fields, columns, rows, nulls)?;
            Self::from_arrow_array(field, array)
        }
    }

    /// One record run out of its child arrays, keeping a record with no
    /// children countable.
    fn record_array(
        fields: &Fields,
        columns: Vec<ArrayRef>,
        rows: usize,
        nulls: Option<arrow_buffer::NullBuffer>,
    ) -> Result<ArrayRef> {
        if columns.is_empty() {
            return Ok(Arc::new(StructArray::new_empty_fields(rows, nulls)));
        }
        Ok(Arc::new(StructArray::try_new(
            fields.clone(),
            columns,
            nulls,
        )?))
    }

    impl Serie {
        /// Take one run of Arrow buffers as the column of `field`.
        ///
        /// # Errors
        ///
        /// [`Column::from_arrow_array`] carries the rule.
        pub fn from_arrow_array(field: Field, array: ArrayRef) -> Result<Self> {
            Column::from_arrow_array(field, array).map(Self::Column)
        }

        /// Take several runs of Arrow buffers, in row order, as one column.
        ///
        /// # Errors
        ///
        /// [`Column::from_arrow_chunks`] carries the rule.
        pub fn from_arrow_chunks(field: Field, arrays: Vec<ArrayRef>) -> Result<Self> {
            Column::from_arrow_chunks(field, arrays).map(Self::Column)
        }

        /// Read one Arrow table as the column of its rows.
        ///
        /// # Errors
        ///
        /// [`Column::from_arrow_batch`] carries the rule.
        pub fn from_arrow_batch(batch: &RecordBatch) -> Result<Self> {
            Column::from_arrow_batch(batch).map(Self::Column)
        }

        /// Take one Arrow batch stream as the column of its rows.
        ///
        /// # Errors
        ///
        /// [`Column::from_arrow_reader`] carries the rule.
        pub fn from_arrow_reader(reader: BatchReader) -> Result<Self> {
            Column::from_arrow_reader(reader).map(Self::Column)
        }
    }
}

// ------------------------------------------------------------------------
// The root forwards every door to whichever leaf holds the rows.
// ------------------------------------------------------------------------

impl Serie {
    /// Append one row, proven by this column's field.
    ///
    /// # Errors
    ///
    /// [`Column::push`] carries the rule.
    pub fn push(&mut self, value: impl Into<Scalar>) -> Result<()> {
        match self {
            Self::Column(held) => held.push(value),
        }
    }

    /// Append every row of `rows`, each proven by this column's field.
    ///
    /// # Errors
    ///
    /// [`Column::extend`] carries the rule.
    pub fn extend(&mut self, rows: impl IntoIterator<Item = Scalar>) -> Result<()> {
        match self {
            Self::Column(held) => held.extend(rows),
        }
    }

    /// A column over `len` rows starting at `offset`, sharing the buffers.
    pub fn slice(&self, offset: usize, len: usize) -> Self {
        match self {
            Self::Column(held) => Self::Column(held.slice(offset, len)),
        }
    }

    /// Drop every row past `len`.
    pub fn truncate(&mut self, len: usize) {
        match self {
            Self::Column(held) => held.truncate(len),
        }
    }

    /// Freeze the rows pushed since the last freeze into one run.
    ///
    /// # Errors
    ///
    /// [`Column::freeze`] carries the rule.
    pub fn freeze(&mut self) -> crate::arrow::Result<()> {
        match self {
            Self::Column(held) => held.freeze(),
        }
    }

    /// Gather every run into one, so the column is one set of buffers.
    ///
    /// # Errors
    ///
    /// [`Column::compact`] carries the rule.
    pub fn compact(&mut self) -> crate::arrow::Result<()> {
        match self {
            Self::Column(held) => held.compact(),
        }
    }

    /// Append one run of Arrow buffers, copying no row.
    ///
    /// # Errors
    ///
    /// [`Column::append_arrow_array`] carries the rule.
    pub fn append_arrow_array(&mut self, array: ArrayRef) -> crate::arrow::Result<()> {
        match self {
            Self::Column(held) => held.append_arrow_array(array),
        }
    }

    /// Append every row of `other`, copying no row of either.
    ///
    /// # Errors
    ///
    /// [`Column::append`] carries the rule.
    pub fn append(&mut self, other: &Self) -> crate::arrow::Result<()> {
        match (self, other) {
            (Self::Column(held), Self::Column(other)) => held.append(other),
        }
    }

    /// Every row of this column as one run of buffers.
    ///
    /// # Errors
    ///
    /// [`Column::into_arrow_array`] carries the rule.
    pub fn into_arrow_array(&self) -> crate::arrow::Result<ArrayRef> {
        match self {
            Self::Column(held) => held.into_arrow_array(),
        }
    }

    /// Every frozen run of this column, in row order, sharing its buffers.
    ///
    /// # Errors
    ///
    /// [`Column::into_arrow_chunks`] carries the rule.
    pub fn into_arrow_chunks(&self) -> crate::arrow::Result<Vec<ArrayRef>> {
        match self {
            Self::Column(held) => held.into_arrow_chunks(),
        }
    }

    /// Every row of this column as one Arrow table.
    ///
    /// # Errors
    ///
    /// [`Column::into_arrow_batch`] carries the rule.
    pub fn into_arrow_batch(&self) -> crate::arrow::Result<arrow_array::RecordBatch> {
        match self {
            Self::Column(held) => held.into_arrow_batch(),
        }
    }

    /// Stream this column's rows, one batch per run.
    ///
    /// # Errors
    ///
    /// [`Column::into_arrow_reader`] carries the rule.
    pub fn into_arrow_reader(&self) -> crate::arrow::Result<crate::arrow::BatchReader> {
        match self {
            Self::Column(held) => held.into_arrow_reader(),
        }
    }

    /// Borrow one child of a Struct-typed column as a column of its own.
    ///
    /// # Errors
    ///
    /// [`Column::child`] carries the rule.
    pub fn child(&self, name: &str) -> crate::arrow::Result<Self> {
        match self {
            Self::Column(held) => held.child(name).map(Self::Column),
        }
    }

    /// Borrow child `index` of a Struct-typed column as a column of its own.
    ///
    /// # Errors
    ///
    /// [`Column::child_at`] carries the rule.
    pub fn child_at(&self, index: usize) -> crate::arrow::Result<Self> {
        match self {
            Self::Column(held) => held.child_at(index).map(Self::Column),
        }
    }

    /// Return this column with `child` added, or replacing the child of its
    /// name.
    ///
    /// # Errors
    ///
    /// [`Column::with_child`] carries the rule.
    pub fn with_child(&self, child: &Self) -> crate::arrow::Result<Self> {
        match (self, child) {
            (Self::Column(held), Self::Column(child)) => held.with_child(child).map(Self::Column),
        }
    }

    /// Return this column with the child of `name` dropped.
    ///
    /// # Errors
    ///
    /// [`Column::without_child`] carries the rule.
    pub fn without_child(&self, name: &str) -> crate::arrow::Result<Self> {
        match self {
            Self::Column(held) => held.without_child(name).map(Self::Column),
        }
    }
}
