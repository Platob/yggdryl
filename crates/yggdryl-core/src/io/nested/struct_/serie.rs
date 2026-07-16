//! [`StructSerie`] — a nullable **struct column**: a set of equal-length, heterogeneous child
//! columns (each an erased [`AnySerie`](crate::io::AnySerie), held as `Box<dyn AnySerie>`) addressed
//! by an ordered schema, plus an optional top-level validity mask. It builds entirely on the root
//! `Any*` primitives — it is itself an [`AnySerie`] (so it nests) — and bridges to Arrow's
//! `StructArray` / `RecordBatch`: a struct column *is* a batch of named columns.

use core::any::Any;

use super::scalar::StructScalar;
use super::{StructField, StructType};
use crate::io::any_serie::apply_field_header;
use crate::io::bitmap::Bitmap;
use crate::io::field_carrier::{any_serie_field_forwarding, field_accessors};
use crate::io::fixed::Field;
use crate::io::nested::{empty_any_column, read_any_column};
use crate::io::{
    AnyField, AnyScalar, AnySerie, Bytes, DataTypeId, Headers, IOCursor, IoError, SerieType,
};

/// A **nullable struct column** — one child [`AnySerie`](crate::io::AnySerie) per field (all of the
/// same length), an ordered schema of [`AnyField`]s, and an optional top-level validity mask.
///
/// ```
/// use yggdryl_core::io::fixed::Serie;
/// use yggdryl_core::io::var::Utf8Serie;
/// use yggdryl_core::io::{boxed, AnySerie};
/// use yggdryl_core::io::nested::StructSerie;
///
/// let ids = boxed(Serie::from_values(&[1i64, 2, 3]));
/// let names = boxed(Utf8Serie::from_strs(&[Some("a"), None, Some("c")]));
/// let table = StructSerie::from_named(vec![("id", ids), ("name", names)]).unwrap();
/// assert_eq!(table.len(), 3);
/// assert_eq!(table.num_columns(), 2);
/// // Downcast a child back to its concrete Serie, keyed on the field's type.
/// let ids: &Serie<i64> = table.column(0).unwrap().as_serie::<i64>().unwrap();
/// assert_eq!(ids.get(0), Some(1));
/// ```
#[derive(Debug, Clone)]
pub struct StructSerie {
    columns: Vec<Box<dyn AnySerie>>,
    validity: Option<Bitmap>,
    len: usize,
    /// The struct column's **own-header** field (`Struct` type_id) — its name, declared nullability,
    /// and metadata. Excluded from value identity and never written to the standalone frame; the
    /// child schema (each child column's derived field) is the single source of truth for the
    /// children. The struct's own name/metadata surface only through [`field`](StructSerie::field).
    field: Field,
}

/// Value identity is the **derived child schema** (each child column's `field_self`, which carries
/// its NAME — a struct is unreconstructable without child names) + the **child data** (`eq_any`) +
/// the top-level validity, all pairwise. The struct's OWN name / nullability / metadata are schema
/// intent, excluded. Kept in lock-step with the byte codec (the frame writes the derived child
/// schema, never the own header).
impl PartialEq for StructSerie {
    fn eq(&self, other: &Self) -> bool {
        if self.len != other.len
            || self.validity != other.validity
            || self.columns.len() != other.columns.len()
        {
            return false;
        }
        self.columns
            .iter()
            .zip(&other.columns)
            .all(|(a, b)| a.field_self() == b.field_self() && a.eq_any(b.as_ref()))
    }
}

impl Eq for StructSerie {}

impl StructSerie {
    /// A struct column from **self-describing** child columns (each an erased
    /// [`AnySerie`](crate::io::AnySerie), typically named with [`named`](crate::io::AnySerie::named)) —
    /// the schema is each column's own derived [`field_self`](crate::io::AnySerie::field_self) (its
    /// inferred type + header name + metadata). The name/metadata live only in the child's header and
    /// never reach the data frame. Errors [`Unsupported`](IoError::Unsupported) if the columns are not
    /// all the same length.
    ///
    /// ```
    /// use yggdryl_core::io::fixed::Serie;
    /// use yggdryl_core::io::var::Utf8Serie;
    /// use yggdryl_core::io::AnySerie;
    /// use yggdryl_core::io::nested::StructSerie;
    ///
    /// let table = StructSerie::from_series(vec![
    ///     Serie::from_values(&[1i64, 2, 3]).named("id"),
    ///     Utf8Serie::from_strs(&[Some("a"), None, Some("c")]).named("name"),
    /// ])
    /// .unwrap();
    /// assert_eq!(table.num_columns(), 2);
    /// assert_eq!(table.field(1).unwrap().name(), "name");
    /// ```
    pub fn from_series(columns: Vec<Box<dyn AnySerie>>) -> Result<Self, IoError> {
        let len = columns.first().map_or(0, |column| column.len());
        for column in &columns {
            if column.len() != len {
                return Err(mismatch(column.name(), column.len(), len));
            }
        }
        Ok(Self {
            columns,
            validity: None,
            len,
            field: Field::of("", DataTypeId::Struct, 0, false),
        })
    }

    /// A struct column from named child columns — a thin wrapper over
    /// [`from_series`](StructSerie::from_series): each `(name, column)` names the (self-describing)
    /// child column via [`set_name`](crate::io::AnySerie::set_name) before storing, so the schema is
    /// the children's own derived fields. Errors [`Unsupported`](IoError::Unsupported) if the columns
    /// are not all the same length.
    pub fn from_named(columns: Vec<(&str, Box<dyn AnySerie>)>) -> Result<Self, IoError> {
        Self::from_series(
            columns
                .into_iter()
                .map(|(name, mut column)| {
                    column.set_name(name);
                    column
                })
                .collect(),
        )
    }

    /// A struct column from an explicit schema + one child column per field, with an optional per-row
    /// **present** mask (`present[i] == false` marks row `i` a null struct). Errors if the counts or
    /// lengths disagree.
    pub fn from_columns(
        fields: Vec<AnyField>,
        mut columns: Vec<Box<dyn AnySerie>>,
        present: Option<&[bool]>,
    ) -> Result<Self, IoError> {
        if fields.len() != columns.len() {
            return Err(IoError::Unsupported {
                what: format!(
                    "struct has {} fields but {} child columns; they must match",
                    fields.len(),
                    columns.len()
                ),
            });
        }
        let len = columns.first().map_or(0, |column| column.len());
        for (field, column) in fields.iter().zip(&columns) {
            if column.len() != len {
                return Err(mismatch(field.name(), column.len(), len));
            }
        }
        // The explicit schema names the (self-describing) child columns: stamp each column's header
        // from its field so the derived child schema round-trips exactly.
        for (field, column) in fields.iter().zip(&mut columns) {
            apply_field_header(column, field);
        }
        let validity = present.and_then(|flags| {
            let mut bitmap = Bitmap::all_present(len);
            for (index, &is_present) in flags.iter().take(len).enumerate() {
                if !is_present {
                    bitmap.set(index, false);
                }
            }
            (bitmap.null_count() > 0).then_some(bitmap)
        });
        Ok(Self {
            columns,
            validity,
            len,
            field: Field::of("", DataTypeId::Struct, 0, false),
        })
    }

    /// An empty (zero-row) struct column of the given schema.
    pub fn empty(schema: &StructField) -> Self {
        let columns = schema
            .fields()
            .iter()
            .map(|field| {
                let mut column = empty_any_column(field);
                apply_field_header(&mut column, field);
                column
            })
            .collect();
        Self {
            columns,
            validity: None,
            len: 0,
            field: Field::of("", DataTypeId::Struct, 0, false),
        }
    }

    // DESIGN: no `from_scalars(&[StructScalar])`. Unlike a leaf column — whose `from_scalars` is a
    // thin map over each scalar's value into the family's `from_options` — a struct column is built
    // from *child columns* (`from_columns` / `from_named`), or reconstructed whole via
    // `deserialize_bytes` / `from_arrow_array` / `from_record_batch`, not transposed from row scalars.
    // A row-scalar factory would be a ROW→COLUMN transpose: rebuild each child column from the k-th
    // `AnyScalar` of every row. That needs an "erased column from `AnyScalar` cells" primitive, which
    // does not exist — the only ways to make a `Box<dyn AnySerie>` are boxing a concrete `Serie`, the
    // byte-frame [`read_any_leaf`](crate::io::read_any_leaf), or the Arrow importer. Building one
    // would duplicate the whole per-family `DataTypeId` dispatch (decode each cell's canonical bytes
    // back to a typed value across every leaf type, plus recurse for nested children) — substantial
    // new machinery, not a thin delegation, so it is intentionally omitted here.

    /// The number of rows.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the column has no rows.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The number of null struct rows.
    pub fn null_count(&self) -> usize {
        self.validity.as_ref().map_or(0, Bitmap::null_count)
    }

    /// Whether any struct row is null.
    pub fn has_nulls(&self) -> bool {
        self.null_count() > 0
    }

    /// The number of child columns (fields).
    pub fn num_columns(&self) -> usize {
        self.columns.len()
    }

    field_accessors!();

    /// The child field descriptors, in order — **derived on demand** from each child column's own
    /// header (its [`field_self`](crate::io::AnySerie::field_self)); the columns are the single source
    /// of truth, so there is no cached schema. Allocates the returned `Vec` (and each child field).
    pub fn fields(&self) -> Vec<AnyField> {
        self.columns
            .iter()
            .map(|column| column.field_self())
            .collect()
    }

    /// The child field at `index`, **derived** from the child column's own header, or `None` if out
    /// of range. Owned (there is no cached field to borrow).
    pub fn field(&self, index: usize) -> Option<AnyField> {
        self.columns.get(index).map(|column| column.field_self())
    }

    /// The child column at `index` (as an erased [`AnySerie`](crate::io::AnySerie), downcast with
    /// `.as_serie::<T>()`), or `None` if out of range. The `'static` object bound lets the borrow
    /// call the downcast helpers (which are defined on `dyn AnySerie + 'static`).
    pub fn column(&self, index: usize) -> Option<&(dyn AnySerie + 'static)> {
        self.columns.get(index).map(AsRef::as_ref)
    }

    /// The child column named `name` (first match), or `None`.
    pub fn column_named(&self, name: &str) -> Option<&(dyn AnySerie + 'static)> {
        let index = self.columns.iter().position(|c| c.name() == name)?;
        self.column(index)
    }

    /// The typed [`StructType`] descriptor (its child fields).
    pub fn data_type(&self) -> StructType {
        StructType::new(self.fields())
    }

    /// A [`StructField`] naming this struct column, its nullability inferred from whether it holds
    /// any null rows.
    pub fn to_field(&self, name: &str) -> StructField {
        StructField::new(name, self.fields(), self.has_nulls())
    }

    /// The row at `index` as an erased [`AnyScalar::Struct`] — [`AnyScalar::Null`] if the row is null
    /// or out of range.
    pub fn row(&self, index: usize) -> AnyScalar {
        if index >= self.len || self.validity.as_ref().is_some_and(|v| !v.get(index)) {
            return AnyScalar::Null;
        }
        AnyScalar::Struct(self.cell_values(index))
    }

    /// The row at `index` as a [`StructScalar`] — its `is_null` flag reflects the top-level validity,
    /// but its per-field values are always populated. Out of range yields a null scalar.
    pub fn row_scalar(&self, index: usize) -> StructScalar {
        if index >= self.len {
            return StructScalar::null(self.fields(), Vec::new());
        }
        let values = self.cell_values(index);
        if self.validity.as_ref().is_some_and(|v| !v.get(index)) {
            StructScalar::null(self.fields(), values)
        } else {
            StructScalar::new(self.fields(), values)
        }
    }

    /// The per-field erased values at `index`.
    fn cell_values(&self, index: usize) -> Vec<AnyScalar> {
        self.columns
            .iter()
            .map(|column| column.value(index))
            .collect()
    }

    /// A **new** struct column holding rows `[offset, offset + len)` — the range is clamped to the
    /// column (an out-of-range or overlong request yields the in-bounds sub-window, never a panic).
    /// Each child column and the top-level validity are sliced to the same window; the schema is
    /// preserved. The result is a fresh column (the children copy their windows); the original is
    /// untouched.
    ///
    /// ```
    /// use yggdryl_core::io::fixed::Serie;
    /// use yggdryl_core::io::AnySerie;
    /// use yggdryl_core::io::nested::StructSerie;
    ///
    /// let table = StructSerie::from_series(vec![
    ///     Serie::from_values(&[1i32, 2, 3, 4]).named("n"),
    /// ])
    /// .unwrap();
    /// let middle = table.slice(1, 2);
    /// assert_eq!(middle.len(), 2);
    /// assert_eq!(middle.column(0).unwrap().value(0), table.column(0).unwrap().value(1));
    /// ```
    pub fn slice(&self, offset: usize, len: usize) -> Self {
        let start = offset.min(self.len);
        let count = len.min(self.len - start);
        // A freshly-sliced child column carries an empty header, so restore each child's own field
        // (its name / nullable / metadata) onto the slice — a struct is unreconstructable without its
        // child names, and the derived schema must survive a slice.
        let columns = self
            .columns
            .iter()
            .map(|column| {
                let mut sliced = column.slice(start, count);
                apply_field_header(&mut sliced, &column.field_self());
                sliced
            })
            .collect();
        let validity = self.validity.as_ref().map(|mask| {
            let mut sliced = Bitmap::all_present(count);
            for index in 0..count {
                if !mask.get(start + index) {
                    sliced.set(index, false);
                }
            }
            sliced
        });
        Self {
            columns,
            validity: normalize(validity),
            len: count,
            field: self.field.clone(),
        }
    }

    // ---- serialization: the schema, then each child via its own `Serie` codec ----------

    /// This struct column's canonical bytes — a self-contained `[schema][len][validity?][children]`
    /// frame. The exact inverse of [`deserialize_bytes`](StructSerie::deserialize_bytes).
    pub fn serialize_bytes(&self) -> Vec<u8> {
        let mut sink = Bytes::new();
        self.write_frame(&mut sink)
            .expect("writing to an in-memory buffer is infallible");
        sink.as_slice().to_vec()
    }

    /// Reconstructs a struct column from [`serialize_bytes`](StructSerie::serialize_bytes) bytes.
    pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, IoError> {
        Self::read_frame(&mut Bytes::from_slice(bytes))
    }

    /// Writes the self-contained frame to a byte sink (shared by `serialize_bytes` and the
    /// [`AnySerie`](crate::io::AnySerie) impl, so a struct child serializes recursively).
    fn write_frame(&self, sink: &mut Bytes) -> Result<(), IoError> {
        // Encode the schema (a struct field over the **derived** child fields). Its name / metadata
        // are deliberately empty and its nullability is `has_nulls()` (not the own-header flag), so a
        // struct equal-in-data but differing in its own name/metadata still serializes byte-identical.
        let fields = self.fields();
        let mut schema = Vec::new();
        AnyField::encode_struct("", self.has_nulls(), &Headers::new(), &fields, &mut schema);
        sink.write_all(&(schema.len() as u64).to_le_bytes())?;
        sink.write_all(&schema)?;
        sink.write_all(&(self.len as u64).to_le_bytes())?;
        write_validity(sink, self.validity.as_ref())?;
        for column in &self.columns {
            column.write_to(sink)?;
        }
        Ok(())
    }

    /// Reads a frame written by [`write_frame`](StructSerie::write_frame). Crate-visible so the
    /// shared recursive [`read_any_column`](crate::io::nested::read_any_column) dispatch can read a
    /// struct child.
    pub(crate) fn read_frame(source: &mut Bytes) -> Result<Self, IoError> {
        let schema_len = read_u64(source)? as usize;
        let schema_bytes = source.read_exact_vec(schema_len)?;
        let schema = AnyField::deserialize_bytes(&schema_bytes)?;
        let fields = match schema {
            AnyField::Struct { children, .. } => children,
            AnyField::Leaf(_) | AnyField::List { .. } | AnyField::Map { .. } => {
                return Err(IoError::Unsupported {
                    what: "serialized struct schema did not decode to a struct".to_string(),
                })
            }
        };
        let len = read_u64(source)? as usize;
        let validity = read_validity(source, len)?;
        let mut columns = Vec::with_capacity(fields.len());
        for field in &fields {
            let mut column = read_any_column(field, source)?;
            if column.len() != len {
                return Err(mismatch(field.name(), column.len(), len));
            }
            // Restore each child column's header from the schema field, so the derived child schema
            // round-trips exactly (a struct is unreconstructable without its child names).
            apply_field_header(&mut column, field);
            columns.push(column);
        }
        Ok(Self {
            columns,
            validity: normalize(validity),
            len,
            field: Field::of("", DataTypeId::Struct, 0, false),
        })
    }
}

impl SerieType for StructSerie {
    type Elem = AnyScalar;

    fn len(&self) -> usize {
        self.len
    }

    fn null_count(&self) -> usize {
        self.null_count()
    }

    fn get(&self, index: usize) -> Option<AnyScalar> {
        match self.row(index) {
            AnyScalar::Null => None,
            value => Some(value),
        }
    }
}

impl AnySerie for StructSerie {
    fn len(&self) -> usize {
        self.len
    }

    fn null_count(&self) -> usize {
        StructSerie::null_count(self)
    }

    fn type_id(&self) -> DataTypeId {
        DataTypeId::Struct
    }

    any_serie_field_forwarding!();

    fn field(&self, name: &str) -> AnyField {
        AnyField::struct_(name, self.fields(), self.nullable() || self.has_nulls())
            .with_metadata_overlay(self.metadata())
    }

    fn value(&self, index: usize) -> AnyScalar {
        self.row(index)
    }

    fn slice(&self, offset: usize, len: usize) -> Box<dyn AnySerie> {
        Box::new(StructSerie::slice(self, offset, len))
    }

    fn write_to(&self, sink: &mut Bytes) -> Result<(), IoError> {
        self.write_frame(sink)
    }

    #[cfg(feature = "arrow")]
    fn to_arrow_array(&self) -> Result<arrow_array::ArrayRef, IoError> {
        Ok(std::sync::Arc::new(StructSerie::to_arrow_array(self)?))
    }

    fn clone_box(&self) -> Box<dyn AnySerie> {
        Box::new(self.clone())
    }

    fn eq_any(&self, other: &dyn AnySerie) -> bool {
        other
            .as_any()
            .downcast_ref::<Self>()
            .is_some_and(|other| self == other)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A guided length-mismatch error.
fn mismatch(name: &str, got: usize, expected: usize) -> IoError {
    IoError::Unsupported {
        what: format!(
            "struct child column {name:?} has length {got} but the struct length is {expected}; \
             every child column must be the same length"
        ),
    }
}

/// Writes the struct's top-level validity `[has_validity: u8][validity bytes?]`.
fn write_validity<W: IOCursor>(sink: &mut W, validity: Option<&Bitmap>) -> Result<(), IoError> {
    let present = validity.is_some_and(|bitmap| bitmap.null_count() > 0);
    sink.write_all(&[u8::from(present)])?;
    if present {
        sink.write_all(validity.unwrap().as_bytes())?;
    }
    Ok(())
}

/// Reads the struct's top-level validity for `len` rows (the mask read is length-bounded).
fn read_validity<R: IOCursor>(source: &mut R, len: usize) -> Result<Option<Bitmap>, IoError> {
    let mut flag = [0u8; 1];
    source.read_exact(&mut flag)?;
    if flag[0] == 0 {
        return Ok(None);
    }
    let bits = source.read_exact_vec(len.div_ceil(8))?;
    Ok(Some(Bitmap::from_bytes(&bits, len)))
}

/// Drops an all-present mask to `None` so equality/serialization stay canonical.
fn normalize(validity: Option<Bitmap>) -> Option<Bitmap> {
    validity.filter(|bitmap| bitmap.null_count() > 0)
}

/// Reads a little-endian `u64`.
fn read_u64<R: IOCursor>(source: &mut R) -> Result<u64, IoError> {
    let mut bytes = [0u8; 8];
    source.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

// -------------------------------------------------------------------------------------
// Arrow interop (feature `arrow`): struct column <-> StructArray, and <-> RecordBatch.
// -------------------------------------------------------------------------------------

#[cfg(feature = "arrow")]
impl StructSerie {
    /// This struct column as an Arrow [`StructArray`](arrow_array::StructArray) — **recursive**, each
    /// child mapped by its [`AnySerie::to_arrow_array`](crate::io::AnySerie), top-level validity as a
    /// `NullBuffer`. Fallible because a temporal child at a resolution Arrow cannot express
    /// (`Minute`…`Year`) has no Arrow array.
    pub fn to_arrow_array(&self) -> Result<arrow_array::StructArray, IoError> {
        let arrow_fields: Vec<arrow_schema::Field> =
            self.fields().iter().map(AnyField::to_arrow).collect();
        let nulls = self.validity.as_ref().map(|bitmap| {
            let buffer = arrow_buffer::Buffer::from(bitmap.as_bytes());
            arrow_buffer::NullBuffer::new(arrow_buffer::BooleanBuffer::new(buffer, 0, self.len))
        });
        if arrow_fields.is_empty() {
            return Ok(arrow_array::StructArray::new_empty_fields(self.len, nulls));
        }
        let child_arrays: Vec<arrow_array::ArrayRef> = self
            .columns
            .iter()
            .map(|column| column.to_arrow_array())
            .collect::<Result<Vec<_>, _>>()?;
        Ok(arrow_array::StructArray::new(
            arrow_schema::Fields::from(arrow_fields),
            child_arrays,
            nulls,
        ))
    }

    /// Builds a struct column from an Arrow [`StructArray`](arrow_array::StructArray) and its
    /// [`Field`](arrow_schema::Field) (of `Struct` type), recovering each child recursively.
    pub fn from_arrow_array(
        array: &arrow_array::StructArray,
        field: &arrow_schema::Field,
    ) -> Result<Self, IoError> {
        use arrow_array::Array;
        let arrow_schema::DataType::Struct(child_fields) = field.data_type() else {
            return Err(IoError::Unsupported {
                what: format!(
                    "expected an Arrow Struct field, got {:?}",
                    field.data_type()
                ),
            });
        };
        let mut columns = Vec::with_capacity(child_fields.len());
        for (arrow_field, child) in child_fields.iter().zip(array.columns()) {
            let field =
                AnyField::from_arrow(arrow_field).ok_or_else(|| not_modeled(arrow_field))?;
            let mut column = crate::io::nested::from_arrow_any_column(child.as_ref(), arrow_field)?;
            apply_field_header(&mut column, &field);
            columns.push(column);
        }
        Ok(Self {
            columns,
            validity: struct_validity_from_arrow(array),
            len: array.len(),
            field: Field::of("", DataTypeId::Struct, 0, false),
        })
    }

    /// This struct column as an Arrow [`RecordBatch`](arrow_array::RecordBatch) — each field becomes
    /// a batch column. A `RecordBatch` has no top-level validity, so a struct with **null rows**
    /// cannot be a batch: errors [`Unsupported`](IoError::Unsupported) (use
    /// [`to_arrow_array`](StructSerie::to_arrow_array) for a nullable `StructArray`).
    pub fn to_record_batch(&self) -> Result<arrow_array::RecordBatch, IoError> {
        use std::sync::Arc;
        if self.has_nulls() {
            return Err(IoError::Unsupported {
                what: "a struct column with null rows has no RecordBatch form (a batch has no \
                       top-level validity); use to_arrow_array for a nullable StructArray"
                    .to_string(),
            });
        }
        let arrow_fields: Vec<arrow_schema::Field> =
            self.fields().iter().map(AnyField::to_arrow).collect();
        let schema = Arc::new(arrow_schema::Schema::new(arrow_fields));
        let columns: Vec<arrow_array::ArrayRef> = self
            .columns
            .iter()
            .map(|column| column.to_arrow_array())
            .collect::<Result<Vec<_>, _>>()?;
        if columns.is_empty() {
            let options = arrow_array::RecordBatchOptions::new().with_row_count(Some(self.len));
            return arrow_array::RecordBatch::try_new_with_options(schema, columns, &options)
                .map_err(record_batch_err);
        }
        arrow_array::RecordBatch::try_new(schema, columns).map_err(record_batch_err)
    }

    /// Builds a struct column from an Arrow [`RecordBatch`](arrow_array::RecordBatch) — its columns
    /// become the struct's fields (no top-level nulls).
    pub fn from_record_batch(batch: &arrow_array::RecordBatch) -> Result<Self, IoError> {
        let schema = batch.schema();
        let mut columns = Vec::with_capacity(schema.fields().len());
        for (arrow_field, array) in schema.fields().iter().zip(batch.columns()) {
            let field =
                AnyField::from_arrow(arrow_field).ok_or_else(|| not_modeled(arrow_field))?;
            let mut column = crate::io::nested::from_arrow_any_column(array.as_ref(), arrow_field)?;
            apply_field_header(&mut column, &field);
            columns.push(column);
        }
        Ok(Self {
            columns,
            validity: None,
            len: batch.num_rows(),
            field: Field::of("", DataTypeId::Struct, 0, false),
        })
    }
}

/// The struct's top-level validity from a `StructArray`, offset-aware, canonicalized.
#[cfg(feature = "arrow")]
fn struct_validity_from_arrow(array: &arrow_array::StructArray) -> Option<Bitmap> {
    use arrow_array::Array;
    if array.null_count() == 0 {
        return None;
    }
    let mut bitmap = Bitmap::all_present(array.len());
    for index in 0..array.len() {
        if array.is_null(index) {
            bitmap.set(index, false);
        }
    }
    Some(bitmap)
}

#[cfg(feature = "arrow")]
fn not_modeled(field: &arrow_schema::Field) -> IoError {
    IoError::Unsupported {
        what: format!(
            "Arrow field {:?} of type {:?} is not a yggdryl-modeled column type",
            field.name(),
            field.data_type()
        ),
    }
}

#[cfg(feature = "arrow")]
fn record_batch_err(error: arrow_schema::ArrowError) -> IoError {
    IoError::Unsupported {
        what: format!("could not build a RecordBatch from the struct column: {error}"),
    }
}
