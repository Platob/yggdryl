//! The typed row view: one Struct [`Field`] and a typed value per child.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Index;

use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};
use smol_str::SmolStr;

use super::TypedScalar;
use crate::types::FieldKey;
use crate::{Error, Field, Result, Scalar};

/// One row under one Struct [`Field`], with every cell proven.
///
/// The row schema is a non-null Struct `Field`, and this is a row read under
/// it: cell `i` is a [`TypedScalar`] borrowing the `i`th child of that field,
/// and the row borrows the field itself. It is a view over the one schema,
/// not a second one - it adds no accessor a `Field` does not already answer,
/// and it resolves a name exactly as the field does, an exact match first and
/// an ASCII case-insensitive one after.
///
/// Building one canonicalizes the row through the field's own row
/// canonicalization, so an ordered [`Scalar::Sequence`](Scalar) and a named
/// [`Scalar::Record`](Scalar) are both accepted, and the cells are exactly
/// what [`Field::canonicalize_value`] answers; a row already canonical costs
/// the one `Vec` the cells live in. Reading a cell, a name, or iterating
/// allocates nothing; [`Self::into_scalar`] is the allocating counterpart.
///
/// Equality and hashing read the field's datatype and the cells, never its
/// name, nullability, or metadata - the rule [`TypedScalar`] states.
///
/// ```
/// use yggdryl::{DataType, Scalar, TypedRecord};
///
/// # fn main() -> yggdryl::Result<()> {
/// let row = DataType::from_fields([
///     DataType::Int64.required_field("id"),
///     DataType::Utf8.nullable_field("symbol"),
/// ])?
/// .required_field("row");
///
/// let record = TypedRecord::new(&row, Scalar::from_record([
///     ("symbol", Scalar::from("AAPL")),
///     ("id", Scalar::from(7)),
/// ])?)?;
/// assert_eq!(record.len(), 2);
/// assert_eq!(record.get_by_name("id").and_then(|cell| cell.as_i64()), Some(7));
/// assert_eq!(record.as_str("symbol"), Some("AAPL"));
/// assert_eq!(record["SYMBOL"].name(), "symbol");
/// assert_eq!(record.names().collect::<Vec<_>>(), ["id", "symbol"]);
/// // The row is the ordered sequence the schema declares.
/// assert_eq!(
///     record.into_scalar(),
///     Scalar::from_sequence([Scalar::from(7_i64), Scalar::from("AAPL")])
/// );
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct TypedRecord<'a> {
    field: &'a Field,
    values: Vec<TypedScalar<'a>>,
}

impl<'a> TypedRecord<'a> {
    /// Read one row under a non-null Struct field.
    ///
    /// # Errors
    ///
    /// Returns an error when the field is nullable or not a Struct, when the
    /// row is neither an ordered sequence of the field's arity nor a record
    /// naming exactly its children, or when a cell is not a value its child
    /// accepts.
    pub fn new(field: &'a Field, row: impl Into<Scalar>) -> Result<Self> {
        field.require_struct_root()?;
        let row = field.canonicalize_row_value(row.into())?;
        let Some(cells) = row.as_sequence() else {
            return Err(Error::InvalidRecord {
                path: SmolStr::new(field.name()),
                reason: SmolStr::new_static("expected an ordered sequence of column values"),
            });
        };
        // Every cell was proven by the one walk above; a shared value clones
        // a reference, so the cells cost only the `Vec` they live in.
        let values = field
            .fields()
            .iter()
            .zip(cells)
            .map(|(child, value)| TypedScalar::from_checked(child, value.clone()))
            .collect();
        Ok(Self { field, values })
    }

    /// The Struct field this row is read under.
    pub const fn field(&self) -> &'a Field {
        self.field
    }

    /// The number of cells, which is the field's number of children.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Return whether the row has no cells.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Look up a cell by position or by name.
    pub fn get<'key>(&self, key: impl Into<FieldKey<'key>>) -> Option<&TypedScalar<'a>> {
        match key.into() {
            FieldKey::Index(index) => self.get_by_index(index),
            FieldKey::Path(name) => self.get_by_name(name),
        }
    }

    /// Look up a cell by name: an exact match first, then an ASCII
    /// case-insensitive one.
    pub fn get_by_name(&self, name: &str) -> Option<&TypedScalar<'a>> {
        let fields = self.field.fields();
        let position = fields
            .iter()
            .position(|child| child.name() == name)
            .or_else(|| {
                fields
                    .iter()
                    .position(|child| child.name().eq_ignore_ascii_case(name))
            })?;
        self.values.get(position)
    }

    /// Look up a cell by position.
    pub fn get_by_index(&self, index: usize) -> Option<&TypedScalar<'a>> {
        self.values.get(index)
    }

    /// The children's names, in schema order.
    pub fn names(&self) -> impl Iterator<Item = &'a str> + use<'a> {
        self.field.fields().iter().map(Field::name)
    }

    /// Iterate over the cells in schema order.
    pub fn iter(&self) -> std::slice::Iter<'_, TypedScalar<'a>> {
        self.values.iter()
    }

    /// Borrow one cell's text, when the cell is text.
    pub fn as_str<'key>(&self, key: impl Into<FieldKey<'key>>) -> Option<&str> {
        self.get(key)?.as_str()
    }

    /// Return a deterministic hash of the row.
    ///
    /// The row hashes as the ordered sequence it canonicalizes to, so it
    /// answers what [`Self::into_scalar`] followed by [`Scalar::stable_hash`]
    /// answers, without building the sequence.
    pub fn stable_hash(&self) -> u64 {
        let mut state = crate::xxhash::Xxh3::new();
        crate::xxhash::write_row_bytes(&mut state, self.values.iter().map(TypedScalar::value));
        state.as_u64()
    }

    /// Consume the row and return its cells' values, in schema order.
    pub fn into_values(self) -> Vec<Scalar> {
        self.values
            .into_iter()
            .map(TypedScalar::into_value)
            .collect()
    }

    /// Consume the row and return the ordered sequence it canonicalizes to.
    ///
    /// One allocation: the sequence's own storage.
    pub fn into_scalar(self) -> Scalar {
        Scalar::from_sequence(self.values.into_iter().map(TypedScalar::into_value))
    }
}

#[cfg(feature = "arrow")]
impl<'a> TypedRecord<'a> {
    /// Read one row of a batch under the field the batch was written under.
    ///
    /// The caller guarantees the batch is under `field`: the columns are
    /// read positionally as the field's children, in the datatype each child
    /// declares, and a batch of another schema is a schema error only where
    /// a column's physical layout disagrees with the child reading it. The
    /// row bound and the column count are checked here.
    ///
    /// ```
    /// use yggdryl::arrow::batch_from_value;
    /// use yggdryl::{DataType, Scalar, TypedRecord};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let row = DataType::from_fields([
    ///     DataType::Int64.required_field("id"),
    ///     DataType::Utf8.nullable_field("symbol"),
    /// ])?
    /// .required_field("row");
    /// let rows = Scalar::from_sequence([
    ///     Scalar::from_sequence([Scalar::from(1_i64), Scalar::from("AAPL")]),
    ///     Scalar::from_sequence([Scalar::from(2_i64), Scalar::Null]),
    /// ]);
    /// let batch = batch_from_value(&row, &rows)?;
    ///
    /// let second = TypedRecord::from_arrow_batch(&row, &batch, 1)?;
    /// assert_eq!(second["id"].as_i64(), Some(2));
    /// assert!(second["symbol"].is_null());
    /// assert!(TypedRecord::from_arrow_batch(&row, &batch, 2).is_err());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the field is not a non-null Struct, when `row`
    /// is past the batch's rows, when the batch has another number of
    /// columns than the field has children, or when a column cannot be read
    /// as the child's datatype.
    pub fn from_arrow_batch(
        field: &'a Field,
        batch: &arrow_array::RecordBatch,
        row: usize,
    ) -> crate::arrow::Result<Self> {
        field.require_struct_root()?;
        if row >= batch.num_rows() {
            return Err(crate::arrow::Error::IncompatibleSchema(format!(
                "row {row} is past the batch's {} rows",
                batch.num_rows()
            )));
        }
        if batch.num_columns() != field.field_len() {
            return Err(crate::arrow::Error::IncompatibleSchema(format!(
                "expected {} columns under field {:?}, got {}",
                field.field_len(),
                field.name(),
                batch.num_columns()
            )));
        }
        let mut values = Vec::with_capacity(field.field_len());
        for (child, column) in field.fields().iter().zip(batch.columns()) {
            let value = crate::arrow::value::value_from_array(child.dtype(), column.as_ref(), row)?;
            values.push(TypedScalar::new(child, value)?);
        }
        Ok(Self { field, values })
    }

    /// Materialize this row as a one-row Arrow batch under the field.
    ///
    /// # Errors
    ///
    /// Returns an error when the physical Arrow layout cannot represent a
    /// cell.
    pub fn into_arrow_batch(self) -> crate::arrow::Result<arrow_array::RecordBatch> {
        let field = self.field;
        crate::arrow::batch_from_value(field, &Scalar::from_sequence([self.into_scalar()]))
    }
}

impl<'a> IntoIterator for TypedRecord<'a> {
    type Item = TypedScalar<'a>;
    type IntoIter = std::vec::IntoIter<TypedScalar<'a>>;

    fn into_iter(self) -> Self::IntoIter {
        self.values.into_iter()
    }
}

impl<'r, 'a> IntoIterator for &'r TypedRecord<'a> {
    type Item = &'r TypedScalar<'a>;
    type IntoIter = std::slice::Iter<'r, TypedScalar<'a>>;

    fn into_iter(self) -> Self::IntoIter {
        self.values.iter()
    }
}

/// Subscripting a row by position reaches that cell.
///
/// # Panics
///
/// Panics when the row has no cell at that position.
impl<'a> Index<usize> for TypedRecord<'a> {
    type Output = TypedScalar<'a>;

    fn index(&self, index: usize) -> &Self::Output {
        self.get_by_index(index).unwrap_or_else(|| {
            panic!(
                "the row under {:?} has {} cells, so position {index} is out of range",
                self.field.name(),
                self.values.len()
            )
        })
    }
}

/// Subscripting a row by name reaches that cell, resolved as
/// [`TypedRecord::get_by_name`] resolves it.
///
/// # Panics
///
/// Panics when no child of the field carries that name.
impl<'a> Index<&str> for TypedRecord<'a> {
    type Output = TypedScalar<'a>;

    fn index(&self, name: &str) -> &Self::Output {
        self.get_by_name(name).unwrap_or_else(|| {
            panic!(
                "{:?} is not a child of the field {:?}",
                name,
                self.field.name()
            )
        })
    }
}

impl fmt::Debug for TypedRecord<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TypedRecord")
            .field("field", &self.field)
            .field("values", &self.values)
            .finish()
    }
}

/// The row as `{name=value, ...}`, each value written as its cell writes it.
impl fmt::Display for TypedRecord<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("{")?;
        for (index, cell) in self.values.iter().enumerate() {
            if index != 0 {
                formatter.write_str(", ")?;
            }
            formatter.write_str(cell.name())?;
            formatter.write_str("=")?;
            fmt::Display::fmt(cell, formatter)?;
        }
        formatter.write_str("}")
    }
}

impl PartialEq for TypedRecord<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.field.dtype() == other.field.dtype() && self.values == other.values
    }
}

impl Eq for TypedRecord<'_> {}

impl Hash for TypedRecord<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.field.dtype().hash(state);
        self.values.hash(state);
    }
}

impl Serialize for TypedRecord<'_> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut structure = serializer.serialize_struct("TypedRecord", 2)?;
        structure.serialize_field("field", self.field)?;
        structure.serialize_field("values", &self.values)?;
        structure.end()
    }
}

impl From<TypedRecord<'_>> for Scalar {
    fn from(record: TypedRecord<'_>) -> Self {
        record.into_scalar()
    }
}

#[cfg(test)]
mod tests;
