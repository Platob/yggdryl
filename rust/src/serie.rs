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
//! [`Column`] is the family's one leaf: rows held as values, canonicalized
//! once by the field that types them. A second column layout - buffers held
//! as they arrived, a fixed-length column - is a leaf beside it rather than a
//! new root variant and a new spelling at every call site.
//!
//! Arrow is reached through the crate's single scalar-array boundary, never a
//! second one: [`Serie::into_arrow_array`] and [`Serie::from_arrow_array`] for
//! one column, [`Serie::into_arrow_batch`] and [`Serie::from_arrow_batch`] for
//! the rows of a Struct root, and [`Serie::into_arrow_reader`] and
//! [`Serie::from_arrow_reader`] for the stream of them a record read speaks.
//!
//! A serie is not a second [`ArrowScalar`]. That type holds Arrow *buffers* -
//! a pinned row, a column, a table or a one-shot stream - and every native
//! narrowing accessor answers `None` for it, because the rows it holds are
//! not values until something materializes them. A serie holds the *values*,
//! which is why it reads as the sequence it is: `as_sequence`, `len`, `iter`
//! and indexing all answer its rows. One owns buffers, the other owns rows,
//! and neither is a second value tree.
//!
//! What a column keeps across the value contract is exactly what that
//! contract leaves alone. A serie whose rows a declaring [`Field`] does not
//! have to rewrite comes back the serie it was - the field it carries
//! included, nested in a record row or not. A serie whose rows that field
//! *does* rewrite - a narrower width, a different scale - comes back the run
//! it now holds, because past that point the declaring field is the authority
//! on the column and carrying a second one would be two owners for one fact.
//!
//! ```
//! use yggdryl::{DataType, Field, Scalar, Serie};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let field = Field::new("price", DataType::Int64, false);
//! let serie = Serie::from_rows(field, [Scalar::from(125_i64), Scalar::from(126_i64)])?;
//!
//! assert_eq!(serie.len(), 2);
//! assert_eq!(serie.get(0), Some(&Scalar::from(125_i64)));
//! assert_eq!(serie.dtype()?, DataType::list(Field::new("price", DataType::Int64, false)));
//!
//! // The same column as an Arrow array, and back.
//! let array = serie.into_arrow_array()?;
//! assert_eq!(array.len(), 2);
//! assert_eq!(Serie::from_arrow_array(serie.field().clone(), array.as_ref())?, serie);
//!
//! // And as a value, which is how a column crosses every other boundary.
//! assert_eq!(Scalar::from(serie).len(), 2);
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

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::sequence::Sequence;
use crate::value::{Children, NestedValue, SerieValue, Value};
use crate::{DataType, Field, Result, Scalar};

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

/// A column of any length: the field that types it and the rows it holds.
///
/// Every row was canonicalized once by [`Field::scalar`], so nothing after
/// construction re-checks a row: reading, widening and materializing are the
/// cost of the operation rather than the cost of proving it again. The field
/// and the rows are one shared pointer each, so cloning a column of a million
/// rows copies sixteen bytes.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Column {
    pub(crate) field: Arc<Field>,
    pub(crate) rows: Arc<[Scalar]>,
}

impl Column {
    /// Build a column from the rows a field types.
    ///
    /// Each row goes through [`Field::scalar`], the crate's one value
    /// contract, so a column holds exactly what the field declares - an
    /// integer narrowed to its width, a decimal restated at its scale, a null
    /// only where the column admits one.
    ///
    /// # Errors
    ///
    /// Returns an error naming the field when a row is not one the field's
    /// datatype accepts, or is null under a field that is not nullable.
    pub fn from_rows(field: Field, rows: impl IntoIterator<Item = Scalar>) -> Result<Self> {
        let rows = rows
            .into_iter()
            .map(|row| field.scalar(row))
            .collect::<Result<Arc<[Scalar]>>>()?;
        Ok(Self {
            field: Arc::new(field),
            rows,
        })
    }

    /// Return the field every row of this column is typed by.
    pub fn field(&self) -> &Field {
        &self.field
    }

    /// Return the shared field without cloning the field itself.
    pub const fn field_ref(&self) -> &Arc<Field> {
        &self.field
    }

    /// Borrow the rows without allocating.
    pub fn as_slice(&self) -> &[Scalar] {
        self.rows.as_ref()
    }

    /// Consume this column and return its shared rows.
    pub fn into_inner(self) -> Arc<[Scalar]> {
        self.rows
    }

    /// Return the datatype this column materializes into.
    ///
    /// # Errors
    ///
    /// Infallible today; the signature is [`Value::dtype`]'s, which a leaf
    /// whose parameters cannot describe a column answers a refusal through.
    pub fn dtype(&self) -> Result<DataType> {
        Ok(DataType::list(self.field.as_ref().clone()))
    }

    /// Return the number of rows.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Return whether this column holds no rows.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Borrow one row, or `None` past the end.
    pub fn get(&self, index: usize) -> Option<&Scalar> {
        self.rows.get(index)
    }

    /// Return this column under another field, rechecking every row.
    ///
    /// # Errors
    ///
    /// [`Self::from_rows`] carries the rule.
    pub fn with_field(&self, field: Field) -> Result<Self> {
        Self::from_rows(field, self.rows.iter().cloned())
    }
}

impl fmt::Display for Column {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}{:?}", self.field.name(), self.rows.as_ref())
    }
}

impl NestedValue for Column {
    fn len(&self) -> usize {
        self.rows.len()
    }

    fn children(&self) -> Children<'_> {
        Children::Sequence(self.rows.iter())
    }
}

impl Value for Column {
    fn dtype(&self) -> Result<DataType> {
        Ok(DataType::list(self.field.as_ref().clone()))
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
        &self.field
    }

    fn as_slice(&self) -> &[Scalar] {
        self.rows.as_ref()
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

/// The column family's value payload.
///
/// One leaf today - rows held as values - because that is the one layout the
/// crate stores a column in. A second layout is a leaf beside it, not a new
/// [`Scalar`] variant and not a new spelling at every call site.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum Serie {
    /// A column of any length, its rows held as values.
    Column(Column),
}

impl Serie {
    /// Build a column from the rows a field types.
    ///
    /// # Errors
    ///
    /// [`Column::from_rows`] carries the rule.
    pub fn from_rows(field: Field, rows: impl IntoIterator<Item = Scalar>) -> Result<Self> {
        Column::from_rows(field, rows).map(Self::Column)
    }

    /// Return the field every row of this column is typed by.
    pub fn field(&self) -> &Field {
        match self {
            Self::Column(value) => &value.field,
        }
    }

    /// Borrow the rows of whichever leaf this is, without allocating.
    pub fn as_slice(&self) -> &[Scalar] {
        match self {
            Self::Column(value) => value.rows.as_ref(),
        }
    }

    /// Return the column value when this is that leaf.
    pub const fn as_column(&self) -> Option<&Column> {
        match self {
            Self::Column(value) => Some(value),
        }
    }

    /// Return the datatype this column materializes into.
    ///
    /// A serie already carries the field its rows are typed by, so this is a
    /// read rather than an agreement computed back off the rows: an empty
    /// serie names its datatype where an empty sequence cannot.
    ///
    /// # Errors
    ///
    /// [`Column::dtype`] carries the rule.
    pub fn dtype(&self) -> Result<DataType> {
        match self {
            Self::Column(value) => value.dtype(),
        }
    }

    /// Return the number of rows.
    pub fn len(&self) -> usize {
        self.as_slice().len()
    }

    /// Return whether this column holds no rows.
    pub fn is_empty(&self) -> bool {
        self.as_slice().is_empty()
    }

    /// Borrow one row, or `None` past the end.
    pub fn get(&self, index: usize) -> Option<&Scalar> {
        self.as_slice().get(index)
    }

    /// Return this column under another field, rechecking every row.
    ///
    /// # Errors
    ///
    /// [`Column::from_rows`] carries the rule.
    pub fn with_field(&self, field: Field) -> Result<Self> {
        match self {
            Self::Column(value) => value.with_field(field).map(Self::Column),
        }
    }

    /// Return the rows as the schema-free sequence they are.
    ///
    /// The field is what a serie has that a sequence has not, so this is the
    /// one direction that drops something and it is spelled rather than
    /// implied.
    pub fn into_sequence(self) -> Scalar {
        match self {
            Self::Column(value) => Scalar::Sequence(Sequence::new(value.rows)),
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

impl NestedValue for Serie {
    fn len(&self) -> usize {
        self.as_slice().len()
    }

    fn children(&self) -> Children<'_> {
        Children::Sequence(self.as_slice().iter())
    }
}

impl Value for Serie {
    fn dtype(&self) -> Result<DataType> {
        match self {
            Self::Column(value) => Value::dtype(value),
        }
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
        match self {
            Self::Column(value) => &value.field,
        }
    }

    fn as_slice(&self) -> &[Scalar] {
        match self {
            Self::Column(value) => value.rows.as_ref(),
        }
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
    /// field's own contract here rather than being taken on the document's
    /// word.
    ///
    /// # Errors
    ///
    /// [`Column::from_rows`] carries the rule.
    fn into_column(self) -> Result<Column> {
        Column::from_rows(self.field, self.rows)
    }
}

impl From<&Column> for SerieWire {
    fn from(value: &Column) -> Self {
        Self {
            field: value.field.as_ref().clone(),
            rows: value.rows.to_vec(),
        }
    }
}

impl Serialize for Column {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        SerieWire::from(self).serialize(serializer)
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
// Arrow projection: one column, the rows of a root, and the stream of them.
// Every crossing routes through the crate's single scalar-array boundary.
// ------------------------------------------------------------------------

mod arrow {
    use arrow_array::{Array, ArrayRef, RecordBatch, RecordBatchReader};

    use super::{Column, Serie};
    use crate::arrow::rows::batch_from_values;
    use crate::arrow::value::array_from_values;
    use crate::arrow::{
        BatchReader, Result, array_to_value, arrow_schema_from_field, batch_reader, batch_to_value,
        field_from_arrow_schema,
    };
    use crate::media::DEFAULT_ROOT_NAME;
    use crate::{Field, Scalar};

    /// The column `field` types, out of the rows one Arrow read answered.
    ///
    /// An array is data from outside, so the rows it decodes meet the field's
    /// own contract here rather than being taken on the array's word: the
    /// datatype-directed decode proves the layout, and [`Field::scalar`]
    /// proves what the layout does not - a null under a column that admits
    /// none. Past this point the column's rows are proven and nothing reads
    /// them again.
    fn column_of(field: Field, rows: &Scalar) -> Result<Column> {
        let rows = rows.as_sequence().unwrap_or_default().to_vec();
        Ok(Column::from_rows(field, rows)?)
    }

    impl Serie {
        /// Materialize this column as one Arrow array.
        ///
        /// The rows cross at the crate's one scalar-array boundary, so the
        /// field decides nullability, dictionary identity and extension
        /// identity exactly as it does for a stored column. They cross it as
        /// they are: this field canonicalized them once already, and the one
        /// value contract is never re-run over what it answered.
        ///
        /// # Errors
        ///
        /// Returns an error when the field has no Arrow projection, or when a
        /// row cannot be materialized into it.
        pub fn into_arrow_array(&self) -> Result<ArrayRef> {
            array_from_values(self.field(), &self.borrowed_rows())
        }

        /// Read one Arrow array as the column of `field`.
        ///
        /// # Errors
        ///
        /// Returns an error when the array does not hold `field`'s datatype,
        /// or when a row it holds is not one the field accepts.
        pub fn from_arrow_array(field: Field, array: &dyn Array) -> Result<Self> {
            let rows = array_to_value(&field, array)?;
            Ok(Self::Column(column_of(field, &rows)?))
        }

        /// Materialize the rows of this column as one Arrow table.
        ///
        /// The field is the record root here, so it must be a bounded,
        /// non-null Struct field and every row a record under it.
        ///
        /// # Errors
        ///
        /// Returns an error unless the field is a record root, or when a row
        /// cannot be materialized into it.
        pub fn into_arrow_batch(&self) -> Result<RecordBatch> {
            // The root is proven here, before the columnar build, so a field
            // that is not a record root is refused by name rather than
            // reaching an internal downcast.
            let schema = arrow_schema_from_field(self.field())?;
            batch_from_values(self.field(), schema, self.as_slice())
        }

        /// Read one Arrow table as the column of its rows.
        ///
        /// The root is named [`DEFAULT_ROOT_NAME`], because Arrow names
        /// columns and never the record.
        ///
        /// # Errors
        ///
        /// Returns an error when the batch schema does not project to a
        /// Yggdryl Struct root, or when a row it holds is not one that root
        /// accepts.
        pub fn from_arrow_batch(batch: &RecordBatch) -> Result<Self> {
            let root = field_from_arrow_schema(DEFAULT_ROOT_NAME, batch.schema().as_ref())?;
            let rows = batch_to_value(batch)?;
            Ok(Self::Column(column_of(root, &rows)?))
        }

        /// Stream the rows of this column as one Arrow batch reader.
        ///
        /// One batch is yielded, because a held column is one table; the
        /// reader states its schema before it is pulled, which is what a
        /// record write already speaks.
        ///
        /// # Errors
        ///
        /// [`Self::into_arrow_batch`] carries the rule.
        pub fn into_arrow_reader(&self) -> Result<BatchReader> {
            let schema = arrow_schema_from_field(self.field())?;
            Ok(batch_reader(schema, [self.into_arrow_batch()?]))
        }

        /// Drain one Arrow batch stream into the column of its rows.
        ///
        /// A stream is one-shot and a column is held, so this materializes:
        /// every batch the reader yields is read into rows under the root the
        /// reader declares. [`crate::IOMedia`] is where rows that are not
        /// meant to be held stay a stream.
        ///
        /// # Errors
        ///
        /// Returns an error when the reported schema does not project to a
        /// Yggdryl Struct root, when the reader fails, or when a row it
        /// yields is not one that root accepts.
        pub fn from_arrow_reader(reader: BatchReader) -> Result<Self> {
            let root = field_from_arrow_schema(DEFAULT_ROOT_NAME, reader.schema().as_ref())?;
            let mut rows: Vec<Scalar> = Vec::new();
            for batch in reader {
                let read = batch_to_value(&batch?)?;
                rows.extend(read.as_sequence().unwrap_or_default().iter().cloned());
            }
            Ok(Self::Column(Column::from_rows(root, rows)?))
        }

        /// The rows as the boundary borrows them, one pointer each.
        fn borrowed_rows(&self) -> Vec<&Scalar> {
            self.as_slice().iter().collect()
        }
    }

    impl Column {
        /// Materialize this column as one Arrow array.
        ///
        /// # Errors
        ///
        /// [`Serie::into_arrow_array`] carries the rule.
        pub fn into_arrow_array(&self) -> Result<ArrayRef> {
            array_from_values(&self.field, &self.rows.iter().collect::<Vec<_>>())
        }

        /// Read one Arrow array as the column of `field`.
        ///
        /// # Errors
        ///
        /// [`Serie::from_arrow_array`] carries the rule.
        pub fn from_arrow_array(field: Field, array: &dyn Array) -> Result<Self> {
            let rows = array_to_value(&field, array)?;
            column_of(field, &rows)
        }
    }
}
