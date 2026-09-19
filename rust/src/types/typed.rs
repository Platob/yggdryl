//! One field paired with the value its own contract answered for it.
//!
//! A [`FieldScalar`] holds a field and that value together, so a reader
//! downstream takes the name, the datatype and the value from one place and
//! re-derives none of them. [`FieldRecord`] is a row of them, and
//! [`UncheckedFieldScalar`] is the same pairing before the proof, for a reader
//! that wants the field's reading of a wire value without committing to it.
//!
//! Narrowing a field to one datatype is not here: a [`Field`] is an enum over
//! its families, so the variant is the proof and no marker is needed to carry
//! it.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};

pub use record::FieldRecord;
use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};
use smol_str::{SmolStr, format_smolstr};

use super::Field;
use crate::{DataType, Error, Result, Scalar};

/// The row view: one Struct [`Field`] and one [`FieldScalar`] per child.
mod record {
    use std::fmt;
    use std::hash::{Hash, Hasher};
    use std::ops::Index;

    use serde::ser::SerializeStruct;
    use serde::{Serialize, Serializer};
    use smol_str::SmolStr;

    use super::FieldScalar;
    use crate::types::FieldKey;
    use crate::{Error, Field, Result, Scalar};

    /// One row under one Struct [`Field`], with every cell proven.
    ///
    /// The row schema is a non-null Struct `Field`, and this is a row read under
    /// it: cell `i` is a [`FieldScalar`] borrowing the `i`th child of that field,
    /// and the row borrows the field itself. It is a view over the one schema,
    /// not a second one - it adds no accessor a `Field` does not already answer,
    /// and a name reaches a cell exactly as [`Field::index_of`] resolves it: by
    /// exact match, so a key the field refuses the row refuses too.
    ///
    /// Building one canonicalizes the row through the field's own row
    /// canonicalization, so an ordered [`Scalar::Sequence`](Scalar) and a named
    /// [`Scalar::Record`](Scalar) are both accepted, and the cells are exactly
    /// what [`Field::canonicalize_value`] answers; a row already canonical costs
    /// the one `Vec` the cells live in. Reading a cell, a name, or iterating
    /// allocates nothing; [`Self::into_scalar`] is the allocating counterpart.
    ///
    /// Equality and hashing read the field's datatype and the cells, never its
    /// name, nullability, or metadata - the rule [`FieldScalar`] states.
    ///
    /// ```
    /// use yggdryl::{DataType, Scalar, FieldRecord};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let row = DataType::from_fields([
    ///     DataType::Int64.required_field("id"),
    ///     DataType::utf8().nullable_field("symbol"),
    /// ])?
    /// .required_field("row");
    ///
    /// let record = FieldRecord::new(&row, Scalar::from_record([
    ///     ("symbol", Scalar::from("AAPL")),
    ///     ("id", Scalar::from(7)),
    /// ])?)?;
    /// assert_eq!(record.len(), 2);
    /// assert_eq!(record.get_by_name("id").and_then(|cell| cell.as_i64()), Some(7));
    /// assert_eq!(record.as_str("symbol"), Some("AAPL"));
    /// // A name resolves exactly, as the field's own lookup does.
    /// assert!(record.get_by_name("SYMBOL").is_none());
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
    pub struct FieldRecord<'a> {
        field: &'a Field,
        values: Vec<FieldScalar<'a>>,
    }

    impl<'a> FieldRecord<'a> {
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
                .map(|(child, value)| FieldScalar::from_checked(child, value.clone()))
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
        ///
        /// A [`FieldKey::Path`] is read as one child's exact name: a cell is a
        /// direct child of the field, so a dotted path, which names a descendant
        /// [`Field::get_field`] would walk to, reaches no cell and answers `None`.
        pub fn get<'key>(&self, key: impl Into<FieldKey<'key>>) -> Option<&FieldScalar<'a>> {
            match key.into() {
                FieldKey::Index(index) => self.get_by_index(index),
                FieldKey::Path(name) => self.get_by_name(name),
            }
        }

        /// Look up a cell by its child's exact name, as [`Field::index_of`]
        /// resolves it.
        pub fn get_by_name(&self, name: &str) -> Option<&FieldScalar<'a>> {
            self.values.get(self.field.index_of(name)?)
        }

        /// Look up a cell by position.
        pub fn get_by_index(&self, index: usize) -> Option<&FieldScalar<'a>> {
            self.values.get(index)
        }

        /// The children's names, in schema order.
        pub fn names(&self) -> impl Iterator<Item = &'a str> + use<'a> {
            self.field.fields().iter().map(Field::name)
        }

        /// Iterate over the cells in schema order.
        pub fn iter(&self) -> std::slice::Iter<'_, FieldScalar<'a>> {
            self.values.iter()
        }

        /// Borrow one cell's text, when the cell is text.
        pub fn as_str<'key>(&self, key: impl Into<FieldKey<'key>>) -> Option<&str> {
            self.get(key)?.as_str()
        }

        /// Consume the row and return its cells' values, in schema order.
        pub fn into_values(self) -> Vec<Scalar> {
            self.values
                .into_iter()
                .map(FieldScalar::into_value)
                .collect()
        }

        /// Consume the row and return the ordered sequence it canonicalizes to.
        ///
        /// One allocation: the sequence's own storage.
        pub fn into_scalar(self) -> Scalar {
            Scalar::from_sequence(self.values.into_iter().map(FieldScalar::into_value))
        }
    }

    impl<'a> FieldRecord<'a> {
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
        /// use yggdryl::{DataType, Scalar, FieldRecord};
        ///
        /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
        /// let row = DataType::from_fields([
        ///     DataType::Int64.required_field("id"),
        ///     DataType::utf8().nullable_field("symbol"),
        /// ])?
        /// .required_field("row");
        /// let rows = Scalar::from_sequence([
        ///     Scalar::from_sequence([Scalar::from(1_i64), Scalar::from("AAPL")]),
        ///     Scalar::from_sequence([Scalar::from(2_i64), Scalar::Null]),
        /// ]);
        /// let batch = batch_from_value(&row, &rows)?;
        ///
        /// let second = FieldRecord::from_arrow_batch(&row, &batch, 1)?;
        /// assert_eq!(second["id"].as_i64(), Some(2));
        /// assert!(second["symbol"].is_null());
        /// assert!(FieldRecord::from_arrow_batch(&row, &batch, 2).is_err());
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
                let value =
                    crate::arrow::value::value_from_array(child.dtype(), column.as_ref(), row)?;
                values.push(FieldScalar::new(child, value)?);
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

    impl<'a> IntoIterator for FieldRecord<'a> {
        type Item = FieldScalar<'a>;
        type IntoIter = std::vec::IntoIter<FieldScalar<'a>>;

        fn into_iter(self) -> Self::IntoIter {
            self.values.into_iter()
        }
    }

    impl<'r, 'a> IntoIterator for &'r FieldRecord<'a> {
        type Item = &'r FieldScalar<'a>;
        type IntoIter = std::slice::Iter<'r, FieldScalar<'a>>;

        fn into_iter(self) -> Self::IntoIter {
            self.values.iter()
        }
    }

    /// Subscripting a row by position reaches that cell.
    ///
    /// # Panics
    ///
    /// Panics when the row has no cell at that position.
    impl<'a> Index<usize> for FieldRecord<'a> {
        type Output = FieldScalar<'a>;

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
    /// [`FieldRecord::get_by_name`] resolves it.
    ///
    /// # Panics
    ///
    /// Panics when no child of the field carries that name.
    impl<'a> Index<&str> for FieldRecord<'a> {
        type Output = FieldScalar<'a>;

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

    impl fmt::Debug for FieldRecord<'_> {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter
                .debug_struct("FieldRecord")
                .field("field", &self.field)
                .field("values", &self.values)
                .finish()
        }
    }

    /// The row as `{name=value, ...}`, each value written as its cell writes it.
    impl fmt::Display for FieldRecord<'_> {
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

    impl PartialEq for FieldRecord<'_> {
        fn eq(&self, other: &Self) -> bool {
            self.field.dtype() == other.field.dtype() && self.values == other.values
        }
    }

    impl Eq for FieldRecord<'_> {}

    impl Hash for FieldRecord<'_> {
        fn hash<H: Hasher>(&self, state: &mut H) {
            self.field.dtype().hash(state);
            self.values.hash(state);
        }
    }

    impl Serialize for FieldRecord<'_> {
        fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let mut structure = serializer.serialize_struct("FieldRecord", 2)?;
            structure.serialize_field("field", self.field)?;
            structure.serialize_field("values", &self.values)?;
            structure.end()
        }
    }

    impl From<FieldRecord<'_>> for Scalar {
        fn from(record: FieldRecord<'_>) -> Self {
            record.into_scalar()
        }
    }
}
/// Prebuilt shared fields, one per leaf datatype.
///
/// A [`FieldScalar`] borrows its field, and a value that names its own datatype
/// has no field to borrow - so the crate keeps one nullable `value` field per
/// leaf datatype for the life of the program, and [`FieldScalar`] borrows
/// that. A parameter-free leaf is built once into a table indexed by
/// [`DataTypeId`]; a leaf whose identity carries a parameter - a decimal's
/// scale, a timestamp's unit and zone, a fixed width - is interned on first
/// use, because its parameters are bounded and the table is, too. Nested
/// datatypes and geospatial parameters are unbounded, so nothing is kept for
/// them and a caller pairs those under a field of its own.
///
/// [`FieldScalar`]: super::FieldScalar
mod shared {
    use std::collections::HashMap;
    use std::sync::{LazyLock, PoisonError, RwLock};

    use crate::types::DecimalType;
    use crate::types::{BytesType, StringType};
    use crate::{DataType, DataTypeId, Field, Scalar};

    /// The name every shared field carries - the name an inferred scalar field
    /// carries too, so a value typed either way is the same column.
    const SHARED_NAME: &str = "value";

    /// How many parameterized leaf datatypes the interned table holds.
    ///
    /// Past this many distinct datatypes the table answers `None` rather than
    /// growing, because an interned field is never freed: the bound is what makes
    /// leaking one per datatype a fixed cost rather than a leak.
    const INTERN_LIMIT: usize = 1 << 12;

    /// One slot per discriminant byte an identifier can carry: the highest one
    /// stated, plus one. A retired number (58, once `msgdirection`) is an empty slot,
    /// because a discriminant is a wire contract and never moves to close a gap.
    ///
    /// Read off the last identifier declared rather than named here, because a
    /// name here is a second place to remember: ids are appended, so naming one
    /// leaves the table a discriminant short the moment another lands after it,
    /// and the build below indexes by `as_u8` with no bound to catch it.
    const PREBUILT_SLOTS: usize = DataTypeId::ALL[DataTypeId::ALL.len() - 1].as_u8() as usize + 1;

    /// One nullable field per parameter-free leaf datatype, by [`DataTypeId::as_u8`].
    ///
    /// The parser owns which name spells which datatype, so each slot parses the
    /// identifier's canonical name rather than restating that table here; a slot
    /// whose name parses to another identifier - the 128-bit integer widths,
    /// which no datatype answers - stays empty, as does a retired number's.
    static PREBUILT: LazyLock<[Option<Field>; PREBUILT_SLOTS]> = LazyLock::new(|| {
        let mut table: [Option<Field>; PREBUILT_SLOTS] = std::array::from_fn(|_| None);
        for id in DataTypeId::ALL {
            if id.is_parameterized() {
                continue;
            }
            table[usize::from(id.as_u8())] = DataType::from_str(id.as_str())
                .ok()
                .filter(|dtype| dtype.id() == id)
                .map(|dtype| Field::new(SHARED_NAME, dtype, true));
        }
        table
    });

    /// One nullable field per plain unbounded UTF-8 leaf.
    ///
    /// Every string identifier is parameterized, so none has a slot in
    /// [`PREBUILT`]; these four are what a bare string value names, and a value
    /// typed by inference borrows one of them rather than interning anything.
    static PLAIN_UTF8: LazyLock<[(StringType, Field); 4]> = LazyLock::new(|| {
        [
            StringType::Utf8String,
            StringType::LargeUtf8String,
            StringType::Utf8StringView,
            StringType::LargeUtf8StringView,
        ]
        .map(|parameters| {
            (
                parameters,
                Field::new(SHARED_NAME, DataType::String(parameters), true),
            )
        })
    });

    /// One nullable field per plain unbounded byte layout, for the same reason.
    static PLAIN_BYTES: LazyLock<[(BytesType, Field); 3]> = LazyLock::new(|| {
        [
            BytesType::Binary,
            BytesType::LargeBinary,
            BytesType::BinaryView,
        ]
        .map(|parameters| {
            (
                parameters,
                Field::new(SHARED_NAME, DataType::Bytes(parameters), true),
            )
        })
    });

    /// The interned fields of parameterized leaf datatypes, bounded by
    /// [`INTERN_LIMIT`].
    static INTERNED: LazyLock<RwLock<HashMap<DataType, &'static Field>>> =
        LazyLock::new(|| RwLock::new(HashMap::new()));

    impl DataType {
        /// The shared nullable `value` field of this datatype, when it has one.
        ///
        /// Every parameter-free leaf answers a field built once for the program,
        /// and so does a plain unbounded UTF-8 string or a plain unbounded byte
        /// column in any layout; every other parameterized leaf - a string
        /// declaring a charset, a bound or a fixed width, bounded or fixed bytes,
        /// a decimal, a timestamp, a time, a duration, an interval - answers one
        /// interned on its first ask, so a second ask for the same datatype is a
        /// lookup that allocates nothing. A nested datatype, and a geometry or
        /// geography whose coordinate reference is unbounded text, answers
        /// `None`; so does a parameterized leaf once 4096 distinct ones are held,
        /// and one whose parameters are invalid.
        ///
        /// ```
        /// use yggdryl::{DataType, Field};
        ///
        /// # fn main() -> yggdryl::Result<()> {
        /// let shared = DataType::Int64.shared_field().unwrap();
        /// assert_eq!(shared.name(), "value");
        /// assert_eq!(shared.dtype(), &DataType::Int64);
        /// assert!(shared.is_nullable());
        /// // The same field every time, for a bare leaf and a parameterized one.
        /// assert!(std::ptr::eq(shared, DataType::Int64.shared_field().unwrap()));
        /// let price = DataType::decimal128(10, 2)?;
        /// assert!(std::ptr::eq(
        ///     price.shared_field().unwrap(),
        ///     price.shared_field().unwrap()
        /// ));
        /// // A nested datatype has no shared field: pair it under your own.
        /// let list = DataType::list(Field::new("item", DataType::Int64, true));
        /// assert!(list.shared_field().is_none());
        /// # Ok(())
        /// # }
        /// ```
        pub fn shared_field(&self) -> Option<&'static Field> {
            let id = self.id();
            if !id.is_parameterized() {
                return PREBUILT[usize::from(id.as_u8())].as_ref();
            }
            match self {
                Self::String(parameters) => PLAIN_UTF8
                    .iter()
                    .find(|(plain, _)| plain == parameters)
                    .map(|(_, field)| field)
                    .or_else(|| interned(self)),
                Self::Bytes(parameters) => PLAIN_BYTES
                    .iter()
                    .find(|(plain, _)| plain == parameters)
                    .map(|(_, field)| field)
                    .or_else(|| interned(self)),
                Self::DateTime64 { .. }
                | Self::Time32(_)
                | Self::Time64(_)
                | Self::Duration32(_)
                | Self::Duration64(_)
                | Self::Interval(_)
                | Self::Decimal(DecimalType::Decimal32 { .. })
                | Self::Decimal(DecimalType::Decimal64 { .. })
                | Self::Decimal(DecimalType::Decimal128 { .. })
                | Self::Decimal(DecimalType::Decimal256 { .. }) => interned(self),
                _ => None,
            }
        }
    }

    /// The interned field of one parameterized leaf datatype.
    fn interned(dtype: &DataType) -> Option<&'static Field> {
        if let Some(field) = INTERNED
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(dtype)
        {
            return Some(field);
        }
        // A datatype with an invalid parameter never earns a permanent field.
        dtype.validate().ok()?;
        let mut table = INTERNED.write().unwrap_or_else(PoisonError::into_inner);
        if let Some(field) = table.get(dtype) {
            return Some(field);
        }
        if table.len() >= INTERN_LIMIT {
            return None;
        }
        let field: &'static Field =
            Box::leak(Box::new(Field::new(SHARED_NAME, dtype.clone(), true)));
        table.insert(dtype.clone(), field);
        Some(field)
    }

    impl Scalar {
        /// The shared field of the datatype this value names, when it has one.
        ///
        /// [`Self::dtype`] reads what the value is, and
        /// [`DataType::shared_field`] answers the field the crate keeps for it.
        ///
        /// ```
        /// use yggdryl::{DataType, Scalar};
        ///
        /// let shared = Scalar::from("AAPL").shared_field().unwrap();
        /// assert_eq!(shared.dtype(), &DataType::utf8());
        /// assert!(Scalar::from_sequence([Scalar::from(1)]).shared_field().is_none());
        /// ```
        pub fn shared_field(&self) -> Option<&'static Field> {
            self.dtype().ok()?.shared_field()
        }
    }
}

/// One [`Field`] and the value it holds, proven together.
///
/// The value is exactly what [`Field::scalar`] answers for it: checked
/// against the datatype, rewritten into the representation the datatype
/// declares, and refused as null where the field is not nullable. The pairing
/// borrows the field rather than copying its datatype, so a reader downstream
/// takes the datatype, the name, and the value from one place - and a
/// borrowed view over a value already canonical allocates nothing, which is
/// what lets a row be typed cell by cell without a schema copy per cell.
///
/// Equality, ordering and hashing read the datatype and the value, never the
/// field's name, nullability, or metadata: two pairings of one value under
/// two columns of the same datatype are the same typed value, exactly as a
/// bare [`Scalar`] is one value whichever column stored it. The same rule
/// holds for [`FieldRecord`].
///
/// ```
/// use yggdryl::{DataType, Field, Scalar, FieldScalar};
///
/// # fn main() -> yggdryl::Result<()> {
/// let price = Field::new("price", DataType::Int32, false);
/// let typed = FieldScalar::new(&price, 7_i64)?;
/// assert_eq!(typed.name(), "price");
/// assert_eq!(typed.dtype(), &DataType::Int32);
/// // The value was narrowed to the width the field declares.
/// assert_eq!(typed.value(), &Scalar::from(7_i32));
/// assert_eq!(typed.as_i64(), Some(7));
/// // Nullability is the field's rule, so a required column refuses a null.
/// assert!(FieldScalar::new(&price, Scalar::Null).is_err());
/// assert!(FieldScalar::new(&price, "seven").is_err());
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct FieldScalar<'a> {
    field: &'a Field,
    value: Scalar,
}

impl<'a> FieldScalar<'a> {
    /// Pair a field with a value it accepts.
    ///
    /// This is [`Field::scalar`], and nothing else: the pairing exists exactly
    /// when the field's own contract answered a value.
    ///
    /// # Errors
    ///
    /// Returns an error naming the field when the value is not one its
    /// datatype accepts, or is null under a field that is not nullable.
    pub fn new(field: &'a Field, value: impl Into<Scalar>) -> Result<Self> {
        Ok(Self {
            field,
            value: field.scalar(value)?,
        })
    }

    /// Read natural text under a field, then pair the value it spells.
    ///
    /// This is the one door from text to a typed value: the bytes a document
    /// spells base64 are substituted, and then the reading is
    /// [`Field::scalar`], which accepts every spelling a document leaves
    /// behind - a number, a date, a code - and answers the exact value.
    ///
    /// ```
    /// use yggdryl::{DataType, Field, Scalar, FieldScalar};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let size = Field::new("size", DataType::Int64, false);
    /// assert_eq!(FieldScalar::parse_str(&size, "42")?.value(), &Scalar::from(42_i64));
    /// assert!(FieldScalar::parse_str(&size, "forty-two").is_err());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming the field when the text spells no value the
    /// field accepts.
    pub fn parse_str(field: &'a Field, text: &str) -> Result<Self> {
        let value = crate::text::prepare_text(Scalar::from(text), field)?;
        Ok(Self::from_checked(field, value))
    }

    /// Pair a value with the shared field of the datatype it already names.
    ///
    /// [`Scalar::dtype`] reads what the value is, and
    /// [`DataType::shared_field`] answers the one nullable `value` field the
    /// crate prebuilt for that datatype, so a leaf value becomes a typed value
    /// without building a field - the pairing borrows a field that lives for
    /// the whole program. A datatype with no shared field, such as a struct
    /// or a list, is paired through [`Self::new`] under a field of the
    /// caller's own.
    ///
    /// ```
    /// use yggdryl::{DataType, Scalar, FieldScalar};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let typed = FieldScalar::infer(Scalar::from(7_i64))?;
    /// assert_eq!(typed.dtype(), &DataType::Int64);
    /// assert_eq!(typed.name(), "value");
    /// assert!(typed.field().is_nullable());
    ///
    /// let row = Scalar::from_sequence([Scalar::from(1_i64)]);
    /// let refused = FieldScalar::infer(row).unwrap_err().to_string();
    /// assert!(refused.contains("list"), "{refused}");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the value names no single datatype, which is
    /// what [`Scalar::dtype`] reports, or names one the crate prebuilds no
    /// shared field for, in which case the error names that datatype.
    pub fn infer(value: Scalar) -> Result<FieldScalar<'static>> {
        let dtype = value.dtype()?;
        let Some(field) = dtype.shared_field() else {
            return Err(Error::InvalidDataType {
                kind: "FieldScalar",
                reason: format_smolstr!(
                    "no shared field is prebuilt for datatype {dtype}; pair the value under a \
                     field of your own through FieldScalar::new"
                ),
            });
        };
        FieldScalar::new(field, value)
    }

    /// Pair a field with a value its contract already answered.
    ///
    /// Row canonicalization proves every cell of a row in one walk, and this
    /// is how those cells become pairings without a second walk each.
    pub(crate) const fn from_checked(field: &'a Field, value: Scalar) -> Self {
        Self { field, value }
    }

    /// The field this value belongs to.
    pub const fn field(&self) -> &'a Field {
        self.field
    }

    /// The datatype this value belongs to.
    pub fn dtype(&self) -> &'a DataType {
        self.field.dtype()
    }

    /// The name of the field this value belongs to.
    pub fn name(&self) -> &'a str {
        self.field.name()
    }

    /// The value itself.
    pub const fn value(&self) -> &Scalar {
        &self.value
    }

    /// Return whether the value is null.
    ///
    /// A null here is what a nullable column stores for an absent value; a
    /// field that is not nullable never pairs with one.
    pub const fn is_null(&self) -> bool {
        self.value.is_null()
    }

    /// Return a string slice when the value is text, ASCII, or an enum member.
    pub fn as_str(&self) -> Option<&str> {
        self.value.as_str()
    }

    /// Return the payload when the value is bytes.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        self.value.as_bytes()
    }

    /// Return a boolean when the value is one.
    pub const fn as_bool(&self) -> Option<bool> {
        self.value.as_bool()
    }

    /// Read the value as an `i64`, when it is an integer that fits.
    pub const fn as_i64(&self) -> Option<i64> {
        self.value.as_i64()
    }

    /// Read the value as a `u64`, when it is an integer that fits.
    pub const fn as_u64(&self) -> Option<u64> {
        self.value.as_u64()
    }

    /// Read the value as an `i128`, when it is a signed integer that fits.
    pub const fn as_i128(&self) -> Option<i128> {
        self.value.as_i128()
    }

    /// Read the value as an `f64`, when it is a float of any width.
    pub fn as_f64(&self) -> Option<f64> {
        self.value.as_f64()
    }

    /// Return the unscaled coefficient and scale of a decimal of any width.
    pub fn as_decimal(&self) -> Option<(crate::i256, i8)> {
        self.value.as_decimal()
    }

    /// Return sequence children when the value is a sequence.
    pub fn as_sequence(&self) -> Option<&[Scalar]> {
        self.value.as_sequence()
    }

    /// Return record fields in name order when the value is a record.
    pub fn as_record(&self) -> Option<&std::collections::BTreeMap<SmolStr, Scalar>> {
        self.value.as_record()
    }

    /// Look up a sequence index.
    pub fn get(&self, index: usize) -> Option<&Scalar> {
        self.value.get(index)
    }

    /// Look up a record field or a text mapping key.
    pub fn get_key_str(&self, key: &str) -> Option<&Scalar> {
        self.value.get_key_str(key)
    }

    /// Return a deterministic hash of the value.
    ///
    /// The field is proof, not content: the hash is the value's own canonical
    /// feed, so a typed value and the bare [`Scalar`] inside it hash alike,
    /// and so does the same value under another field of the same datatype.
    pub fn stable_hash(&self) -> u64 {
        self.value.stable_hash()
    }

    /// Consume this pairing and return the value alone.
    pub fn into_value(self) -> Scalar {
        self.value
    }

    /// Consume this pairing and return both halves.
    pub fn into_parts(self) -> (&'a Field, Scalar) {
        (self.field, self.value)
    }

    /// Consume this pairing and return the value's canonical text.
    ///
    /// The spelling is the one a text column stores for the value - the
    /// canonical form each family prints, a temporal spelled the classic way,
    /// bytes as their characters, a geometry as WKT - and it is what
    /// [`Display`](fmt::Display) writes. A value that spells no text of its
    /// own, such as a row or an interval, writes its family's own form.
    pub fn into_str(self) -> SmolStr {
        match crate::types::string::str_from_value(&self.value) {
            Some(Ok(text)) => text.into_inner(),
            _ => format_smolstr!("{self}"),
        }
    }
}

impl<'a> FieldScalar<'a> {
    /// Decode row 0 of a one-row Arrow array under the field.
    ///
    /// [`crate::arrow::scalar_value`] reads the array - exactly one row, the
    /// field's exact physical layout - and the reading is then paired the
    /// way every value is, through [`Field::scalar`]: an Arrow reading spells
    /// a value physically, a float16 as its narrow float, and the pairing
    /// holds the canonical one.
    ///
    /// ```
    /// use yggdryl::{DataType, Field, Scalar, FieldScalar};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let field = Field::new("size", DataType::Int64, false);
    /// let array = FieldScalar::new(&field, 7_i64)?.into_arrow_array()?;
    /// let typed = FieldScalar::from_arrow_array(&field, array.as_ref())?;
    /// assert_eq!(typed.value(), &Scalar::from(7));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the array does not hold exactly one row of the
    /// field's exact physical layout, or when the decoded value is not one
    /// the field accepts.
    pub fn from_arrow_array(
        field: &'a Field,
        array: &dyn arrow_array::Array,
    ) -> crate::arrow::Result<Self> {
        let value = crate::arrow::scalar_value(field, array)?;
        Self::new(field, value).map_err(crate::arrow::Error::from)
    }

    /// Materialize this pairing as an exact one-row Arrow array.
    ///
    /// The field decides nullability, dictionary options, and extension
    /// identity, exactly as [`crate::arrow::scalar_array`] reads them; the
    /// pairing is that function's validated half, so the array is built
    /// without a second walk over the value.
    ///
    /// ```
    /// use yggdryl::{DataType, Field, FieldScalar};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let field = Field::new("size", DataType::Int64, true);
    /// let array = FieldScalar::new(&field, 7_i64)?.into_arrow_array()?;
    /// assert_eq!(array.len(), 1);
    /// assert_eq!(array.data_type(), &arrow_schema::DataType::Int64);
    /// // A null is what the nullable column stores, so it projects too.
    /// assert!(FieldScalar::new(&field, yggdryl::Scalar::Null)?.into_arrow_array()?.is_null(0));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the physical Arrow layout cannot represent the
    /// value.
    pub fn into_arrow_array(self) -> crate::arrow::Result<arrow_array::ArrayRef> {
        crate::arrow::value::array_from_values(self.field, &[&self.value])
    }
}

/// Write a value's canonical text, or its family's own form when it has none.
///
/// `str_from_value` owns the spelling, and it declines exactly four shapes:
/// a null, a nested value, a temporal without a classic spelling, and a bytes
/// or geometry payload it read and refused. Only those reach the arms below,
/// each writing what its family prints - the hex of a payload, an interval's
/// components, a row as its sequence.
fn write_value(formatter: &mut fmt::Formatter<'_>, value: &Scalar) -> fmt::Result {
    match crate::types::string::str_from_value(value) {
        Some(Ok(text)) => formatter.write_str(&text),
        Some(Err(_)) | None => match value {
            Scalar::Null => formatter.write_str("null"),
            Scalar::Bytes(held) => fmt::Display::fmt(held, formatter),
            Scalar::Geometry(held) => fmt::Display::fmt(held, formatter),
            Scalar::Geography(held) => fmt::Display::fmt(held, formatter),
            // A temporal without a classic spelling and a nested value write
            // their own leaf's form.
            other => match other.leaf_display() {
                Some(held) if other.is_temporal() || other.is_container() => {
                    fmt::Display::fmt(held, formatter)
                }
                _ => unreachable!("every other family spells text, answered above"),
            },
        },
    }
}

impl fmt::Debug for FieldScalar<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FieldScalar")
            .field("field", &self.field)
            .field("value", &self.value)
            .finish()
    }
}

impl fmt::Display for FieldScalar<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_value(formatter, &self.value)
    }
}

impl PartialEq for FieldScalar<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.dtype() == other.dtype() && self.value == other.value
    }
}

impl Eq for FieldScalar<'_> {}

impl PartialOrd for FieldScalar<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for FieldScalar<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.dtype()
            .cmp(other.dtype())
            .then_with(|| self.value.cmp(&other.value))
    }
}

impl Hash for FieldScalar<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.dtype().hash(state);
        self.value.hash(state);
    }
}

impl Serialize for FieldScalar<'_> {
    /// Write the field and the value; the proof does not survive a
    /// serialization, which is why the pairing implements no `Deserialize`.
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut structure = serializer.serialize_struct("FieldScalar", 2)?;
        structure.serialize_field("field", self.field)?;
        structure.serialize_field("value", &self.value)?;
        structure.end()
    }
}

impl AsRef<Scalar> for FieldScalar<'_> {
    fn as_ref(&self) -> &Scalar {
        &self.value
    }
}

impl From<FieldScalar<'_>> for Scalar {
    fn from(typed: FieldScalar<'_>) -> Self {
        typed.into_value()
    }
}

/// A [`Field`] and a value it has not yet proven.
///
/// This is the pairing before the check [`FieldScalar`] carries: a wire
/// spelling and the field a reader resolved it to, held together so the
/// reader can ask for the field's reading of the value without committing to
/// it. Every numeric accessor casts on read - text `"42"` under an `Int64`
/// field answers `Some(42)` - by running the datatype's own value contract
/// and reading the asked shape from its answer, so a reading that fails
/// answers `None` rather than an error. [`Self::as_str`], [`Self::as_bytes`]
/// and [`Self::as_bool`] borrow instead: they hand back the held value only
/// when it already has that shape, and never read it - text `"true"` under
/// a Boolean field answers `None` until [`Self::checked`] proves it.
///
/// The state is unproven, so the pairing has no equality, hash, or serde:
/// [`Self::checked`] is the door to a value that does.
///
/// ```
/// use yggdryl::types::UncheckedFieldScalar;
/// use yggdryl::{DataType, Field, Scalar};
///
/// # fn main() -> yggdryl::Result<()> {
/// let size = Field::new("size", DataType::Int64, false);
/// let unchecked = UncheckedFieldScalar::from_str(&size, "42");
/// // The held text is borrowed as it is, and cast when read as a number.
/// assert_eq!(unchecked.as_str(), Some("42"));
/// assert_eq!(unchecked.as_i64(), Some(42));
/// assert_eq!(unchecked.value(), &Scalar::from("42"));
/// // Proving it rewrites the value into what the field stores.
/// let checked = unchecked.checked()?;
/// assert_eq!(checked.value(), &Scalar::from(42_i64));
///
/// let wrong = UncheckedFieldScalar::from_str(&size, "forty-two");
/// assert_eq!(wrong.as_i64(), None);
/// assert!(wrong.checked().is_err());
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct UncheckedFieldScalar<'a> {
    field: &'a Field,
    value: Scalar,
}

impl<'a> UncheckedFieldScalar<'a> {
    /// Hold a value beside a field without checking it.
    pub fn new(field: &'a Field, value: impl Into<Scalar>) -> Self {
        Self {
            field,
            value: value.into(),
        }
    }

    /// Hold a wire spelling beside the field it was resolved to.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(field: &'a Field, text: &str) -> Self {
        Self::new(field, Scalar::from(text))
    }

    /// The field the value is held beside.
    pub const fn field(&self) -> &'a Field {
        self.field
    }

    /// The datatype the value will be read as.
    pub fn dtype(&self) -> &'a DataType {
        self.field.dtype()
    }

    /// The name of the field the value is held beside.
    pub fn name(&self) -> &'a str {
        self.field.name()
    }

    /// The value as it was held, unread.
    pub const fn value(&self) -> &Scalar {
        &self.value
    }

    /// Return whether the held value is null.
    pub const fn is_null(&self) -> bool {
        self.value.is_null()
    }

    /// Prove the value under the field.
    ///
    /// # Errors
    ///
    /// Returns what [`FieldScalar::new`] returns for the held value.
    pub fn checked(self) -> Result<FieldScalar<'a>> {
        FieldScalar::new(self.field, self.value)
    }

    /// The datatype's reading of the held value, when it has one.
    fn read(&self) -> Option<Scalar> {
        self.field.dtype().scalar(self.value.clone()).ok()
    }

    /// Borrow the held value when it already is text.
    pub fn as_str(&self) -> Option<&str> {
        self.value.as_str()
    }

    /// Borrow the held value when it already is bytes.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        self.value.as_bytes()
    }

    /// Borrow the held value when it already is a boolean.
    pub fn as_bool(&self) -> Option<bool> {
        self.value.as_bool()
    }

    /// The field's reading of the value as an `i64`, when it fits.
    pub fn as_i64(&self) -> Option<i64> {
        self.read()?.as_i64()
    }

    /// The field's reading of the value as a `u64`, when it fits.
    pub fn as_u64(&self) -> Option<u64> {
        self.read()?.as_u64()
    }

    /// The field's reading of the value as an `i128`, when it fits.
    pub fn as_i128(&self) -> Option<i128> {
        self.read()?.as_i128()
    }

    /// The field's reading of the value as an `f64`.
    pub fn as_f64(&self) -> Option<f64> {
        self.read()?.as_f64()
    }

    /// The field's reading of the value as a decimal of any width.
    pub fn as_decimal(&self) -> Option<(crate::i256, i8)> {
        self.read()?.as_decimal()
    }

    /// Consume the pairing and return the held value, unread.
    pub fn into_value(self) -> Scalar {
        self.value
    }
}

impl fmt::Debug for UncheckedFieldScalar<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UncheckedFieldScalar")
            .field("field", &self.field)
            .field("value", &self.value)
            .finish()
    }
}

macro_rules! define_field_types {
    // A datatype that carries no parameters is its own payload: one zero-sized
    // value that stands for the variant. It is what a field of that datatype
    // holds, so it implements the datatype contract rather than a marker trait.
    ($(#[$meta:meta])* $marker:ident, $variant:ident $(,)?) => {
        $(#[$meta])*
        #[doc = concat!(
            "The parameter-free datatype of a [`DataType::",
            stringify!($variant),
            "`](crate::DataType::",
            stringify!($variant),
            ") field."
        )]
        #[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $marker;

        impl ::std::fmt::Display for $marker {
            fn fmt(&self, formatter: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                formatter.write_str($crate::DataTypeId::$variant.as_str())
            }
        }

        impl $crate::types::DataTypeValue for $marker {
            const FAMILY: &'static str = $crate::DataTypeId::$variant.as_str();

            type Sidecar = ();

            fn id(&self) -> $crate::DataTypeId {
                $crate::DataTypeId::$variant
            }

            fn kind(&self) -> $crate::DataTypeKind {
                $crate::DataTypeId::$variant.kind()
            }

            fn validate(&self) -> $crate::Result<()> {
                Ok(())
            }

            fn into_dtype(self) -> $crate::DataType {
                $crate::DataType::$variant
            }

            fn from_dtype(dtype: &$crate::DataType) -> Option<Self> {
                matches!(dtype, $crate::DataType::$variant).then_some(Self)
            }
        }
    };
}

pub(crate) use define_field_types;
