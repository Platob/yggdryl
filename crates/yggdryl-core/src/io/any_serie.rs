//! [`AnySerie`] — the **erased, recursive column**: any yggdryl column behind a `Box<dyn AnySerie>`,
//! so a struct column can hold heterogeneous children. It is a *contract*, not a parallel type —
//! every concrete `Serie` (`Serie<T>`, `DecimalSerie<B>`, `ByteSerie<E>`, `FixedSizeSerie<K>`,
//! `NullSerie`, and — in [`nested`](crate::io::nested) — `StructSerie`) implements it, and every
//! method delegates to that `Serie`'s own implementation. It lives at the `io` root because it spans
//! every family.
//!
//! Downcasting is safe: an `AnySerie` reports its [`field`](AnySerie::field) (and hence its exact
//! type), and `dyn AnySerie` offers [`downcast_ref`](AnySerie::downcast_ref) /
//! [`as_serie`](AnySerie::as_serie) to recover the concrete `Serie` — `None` if the assumed type is
//! wrong, so a caller keyed on the linked field never mis-reads.
//!
//! Arrow **recomposition is zero-copy** wherever the wrapped `Serie` is: the fixed-primitive columns
//! (native *and* wide) build their Arrow array from the Serie's shared `Arc` buffer + the id's Arrow
//! type, uniformly and with no per-type code; decimals share their `Arc` too.

use core::any::Any;
use core::fmt::Debug;

use super::fixed::{
    f16, Dec128, Dec256, Dec32, Dec64, DecimalBacking, DecimalField, DecimalSerie, Field,
    FixedBinarySerie, FixedElement, FixedSizeSerie, FixedUtf8Serie, NativeType, NullSerie, Serie,
    I256, I96, U256, U96,
};
use super::var::{BinarySerie, ByteSerie, Utf8Serie, VarElement};
use super::{AnyField, AnyScalar, Bytes, DataTypeId, FieldType, IoError};

/// The width of one variable-length offset (`i32`).
const OFFSET_WIDTH: usize = core::mem::size_of::<i32>();

/// A **column of any type**, type-erased — the recursive carrier a struct column's heterogeneous
/// children live in (`Box<dyn AnySerie>`). Implemented by every concrete `Serie`; each method
/// delegates. Build one by boxing a `Serie` (`Box::new(serie) as Box<dyn AnySerie>`) or with the
/// erased reader / Arrow importer in [`nested`](crate::io::nested).
///
/// `Send + Sync` because every concrete column is (its buffers are `Arc`-shared or owned `Vec`s,
/// like Arrow's own `Send + Sync` `ArrayRef`), so an erased column — and a `StructSerie` of them —
/// crosses threads and satisfies the language bindings' thread-safety bound.
pub trait AnySerie: Debug + Send + Sync {
    /// The number of elements.
    fn len(&self) -> usize;

    /// The number of null elements.
    fn null_count(&self) -> usize;

    /// Whether the column is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether the column carries any nulls.
    fn has_nulls(&self) -> bool {
        self.null_count() > 0
    }

    /// The column's element [`DataTypeId`].
    fn type_id(&self) -> DataTypeId;

    /// The [`AnyField`] naming a column of this type `name` (nullability inferred from the nulls).
    fn field(&self, name: &str) -> AnyField;

    /// The value at `index` as an erased [`AnyScalar`] — null if the element is null or out of range.
    fn value(&self, index: usize) -> AnyScalar;

    /// Writes this column to `sink` — delegates to the wrapped `Serie`'s own byte codec.
    fn write_to(&self, sink: &mut Bytes) -> Result<(), IoError>;

    /// This column as an Arrow [`ArrayRef`](arrow_array::ArrayRef) (feature `arrow`) — delegates to
    /// the wrapped `Serie`'s own (zero-copy where it is) converter.
    #[cfg(feature = "arrow")]
    fn to_arrow_array(&self) -> arrow_array::ArrayRef;

    /// A boxed clone (value semantics for `Box<dyn AnySerie>`).
    fn clone_box(&self) -> Box<dyn AnySerie>;

    /// Content equality against another erased column (equal type *and* value).
    fn eq_any(&self, other: &dyn AnySerie) -> bool;

    /// This column as `&dyn Any`, for the safe downcast helpers.
    fn as_any(&self) -> &dyn Any;

    /// This column's canonical bytes (the wrapped `Serie`'s frame), as an owned `Vec`.
    fn serialize_bytes(&self) -> Vec<u8> {
        let mut sink = Bytes::new();
        self.write_to(&mut sink)
            .expect("writing to an in-memory buffer is infallible");
        sink.as_slice().to_vec()
    }
}

impl dyn AnySerie {
    /// The concrete `Serie` behind this erased column, if it is of type `S` — the safe downcast a
    /// caller reaches for after reading the [`field`](AnySerie::field) to know the type. `None` if
    /// the assumed type is wrong.
    pub fn downcast_ref<S: AnySerie + 'static>(&self) -> Option<&S> {
        self.as_any().downcast_ref::<S>()
    }

    /// Whether this erased column is a concrete `S`.
    pub fn is<S: AnySerie + 'static>(&self) -> bool {
        self.as_any().is::<S>()
    }

    /// This column as a fixed-width primitive [`Serie<T>`](Serie), if it is one — the `as_ref`-style
    /// typed view (`any.as_serie::<i32>()`), keyed on the element type the field reports.
    pub fn as_serie<T: NativeType>(&self) -> Option<&Serie<T>> {
        self.downcast_ref::<Serie<T>>()
    }

    /// This column as a [`DecimalSerie<B>`](DecimalSerie), if it is one.
    pub fn as_decimal<B: DecimalBacking>(&self) -> Option<&DecimalSerie<B>>
    where
        B::Coeff: arrow_buffer::ArrowNativeType,
    {
        self.downcast_ref::<DecimalSerie<B>>()
    }

    /// This column as a variable-length [`ByteSerie<E>`](ByteSerie) (`Utf8Serie` / `BinarySerie`).
    pub fn as_bytes_serie<E: VarElement>(&self) -> Option<&ByteSerie<E>> {
        self.downcast_ref::<ByteSerie<E>>()
    }
}

/// Boxes a concrete `Serie` as an erased [`AnySerie`] column — the ergonomic constructor for a
/// heterogeneous child, e.g. `boxed(Serie::from_values(&[1i32, 2]))`.
pub fn boxed<S: AnySerie + 'static>(serie: S) -> Box<dyn AnySerie> {
    Box::new(serie)
}

impl Clone for Box<dyn AnySerie> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

impl PartialEq for dyn AnySerie {
    fn eq(&self, other: &Self) -> bool {
        self.eq_any(other)
    }
}

impl Eq for dyn AnySerie {}

/// Writes an `eq_any` in terms of `PartialEq` + a same-type downcast — a value type equals another
/// erased column iff it is the same concrete `Serie` type and compares equal.
macro_rules! eq_via_downcast {
    () => {
        fn eq_any(&self, other: &dyn AnySerie) -> bool {
            other
                .as_any()
                .downcast_ref::<Self>()
                .is_some_and(|other| self == other)
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
        fn clone_box(&self) -> Box<dyn AnySerie> {
            // UFCS to clone `Self` (not the `&Self` reference — `self.clone()` would autoref).
            Box::new(<Self as Clone>::clone(self))
        }
    };
}

// -------------------------------------------------------------------------------------
// Fixed-width primitive columns: one blanket impl. Arrow export builds the array from the
// Serie's shared Arc buffer, so it is zero-copy and uniform over native *and* wide integers.
// -------------------------------------------------------------------------------------

impl<T: NativeType> AnySerie for Serie<T> {
    fn len(&self) -> usize {
        Serie::len(self)
    }

    fn null_count(&self) -> usize {
        Serie::null_count(self)
    }

    fn type_id(&self) -> DataTypeId {
        T::TYPE_ID
    }

    fn field(&self, name: &str) -> AnyField {
        AnyField::leaf(Field::of(name, T::TYPE_ID, T::WIDTH, self.has_nulls()))
    }

    fn value(&self, index: usize) -> AnyScalar {
        match self.get(index) {
            Some(value) => {
                let mut scratch = [0u8; 32];
                value.write_le(&mut scratch);
                AnyScalar::leaf(
                    Field::of("", T::TYPE_ID, T::WIDTH, false),
                    scratch[..T::WIDTH].to_vec(),
                )
            }
            None => AnyScalar::Null,
        }
    }

    fn write_to(&self, sink: &mut Bytes) -> Result<(), IoError> {
        Serie::write_to(self, sink)
    }

    #[cfg(feature = "arrow")]
    fn to_arrow_array(&self) -> arrow_array::ArrayRef {
        let data_type = T::TYPE_ID.to_arrow(T::WIDTH);
        let values = self.arrow_value_buffer();
        let nulls = self
            .validity_bitmap()
            .map(|bitmap| arrow_buffer::Buffer::from(bitmap.as_bytes()));
        let data =
            arrow_data::ArrayData::try_new(data_type, self.len(), nulls, 0, vec![values], vec![])
                .expect("a primitive column's Arc buffer is valid for its Arrow type");
        arrow_array::make_array(data)
    }

    eq_via_downcast!();
}

// -------------------------------------------------------------------------------------
// Decimal, variable-length, fixed-size, and null columns: each delegates to its own Serie.
// -------------------------------------------------------------------------------------

impl<B: DecimalBacking> AnySerie for DecimalSerie<B>
where
    B::Coeff: arrow_buffer::ArrowNativeType,
{
    fn len(&self) -> usize {
        DecimalSerie::len(self)
    }

    fn null_count(&self) -> usize {
        DecimalSerie::null_count(self)
    }

    fn type_id(&self) -> DataTypeId {
        B::TYPE_ID
    }

    fn field(&self, name: &str) -> AnyField {
        AnyField::leaf(
            DecimalField::<B>::new(name, self.precision(), self.scale(), self.has_nulls()).erase(),
        )
    }

    fn value(&self, index: usize) -> AnyScalar {
        if index >= self.len() || self.get_coeff(index).is_none() {
            return AnyScalar::Null;
        }
        let field = DecimalField::<B>::new("", self.precision(), self.scale(), false).erase();
        let bytes = self.coeff_bytes()[index * B::WIDTH..(index + 1) * B::WIDTH].to_vec();
        AnyScalar::leaf(field, bytes)
    }

    fn write_to(&self, sink: &mut Bytes) -> Result<(), IoError> {
        DecimalSerie::write_to(self, sink)
    }

    #[cfg(feature = "arrow")]
    fn to_arrow_array(&self) -> arrow_array::ArrayRef {
        std::sync::Arc::new(DecimalSerie::to_arrow_array(self))
    }

    eq_via_downcast!();
}

impl<E: VarElement> AnySerie for ByteSerie<E> {
    fn len(&self) -> usize {
        ByteSerie::len(self)
    }

    fn null_count(&self) -> usize {
        ByteSerie::null_count(self)
    }

    fn type_id(&self) -> DataTypeId {
        E::TYPE_ID
    }

    fn field(&self, name: &str) -> AnyField {
        AnyField::leaf(Field::of(name, E::TYPE_ID, OFFSET_WIDTH, self.has_nulls()))
    }

    fn value(&self, index: usize) -> AnyScalar {
        match self.get_bytes(index) {
            Some(bytes) => AnyScalar::leaf(
                Field::of("", E::TYPE_ID, OFFSET_WIDTH, false),
                bytes.to_vec(),
            ),
            None => AnyScalar::Null,
        }
    }

    fn write_to(&self, sink: &mut Bytes) -> Result<(), IoError> {
        ByteSerie::write_to(self, sink)
    }

    #[cfg(feature = "arrow")]
    fn to_arrow_array(&self) -> arrow_array::ArrayRef {
        std::sync::Arc::new(ByteSerie::to_arrow_array(self))
    }

    eq_via_downcast!();
}

impl<K: FixedElement> AnySerie for FixedSizeSerie<K> {
    fn len(&self) -> usize {
        FixedSizeSerie::len(self)
    }

    fn null_count(&self) -> usize {
        FixedSizeSerie::null_count(self)
    }

    fn type_id(&self) -> DataTypeId {
        K::TYPE_ID
    }

    fn field(&self, name: &str) -> AnyField {
        AnyField::leaf(Field::of(name, K::TYPE_ID, self.width(), self.has_nulls()))
    }

    fn value(&self, index: usize) -> AnyScalar {
        match self.get_bytes(index) {
            Some(bytes) => AnyScalar::leaf(
                Field::of("", K::TYPE_ID, self.width(), false),
                bytes.to_vec(),
            ),
            None => AnyScalar::Null,
        }
    }

    fn write_to(&self, sink: &mut Bytes) -> Result<(), IoError> {
        FixedSizeSerie::write_to(self, sink)
    }

    #[cfg(feature = "arrow")]
    fn to_arrow_array(&self) -> arrow_array::ArrayRef {
        std::sync::Arc::new(FixedSizeSerie::to_arrow_array(self))
    }

    eq_via_downcast!();
}

impl AnySerie for NullSerie {
    fn len(&self) -> usize {
        NullSerie::len(self)
    }

    fn null_count(&self) -> usize {
        NullSerie::null_count(self)
    }

    fn type_id(&self) -> DataTypeId {
        DataTypeId::Null
    }

    fn field(&self, name: &str) -> AnyField {
        AnyField::leaf(Field::of(name, DataTypeId::Null, 0, true))
    }

    fn value(&self, _index: usize) -> AnyScalar {
        AnyScalar::Null
    }

    fn write_to(&self, sink: &mut Bytes) -> Result<(), IoError> {
        NullSerie::write_to(self, sink)
    }

    #[cfg(feature = "arrow")]
    fn to_arrow_array(&self) -> arrow_array::ArrayRef {
        std::sync::Arc::new(NullSerie::to_arrow_array(self))
    }

    eq_via_downcast!();
}

// -------------------------------------------------------------------------------------
// Erased leaf construction — the one dispatch on `type_id` that has to name the concrete `T`
// (nested `StructSerie` extends it in the `nested` module).
// -------------------------------------------------------------------------------------

/// Reads a **leaf** column of the type named by `field` from `source` (the bytes a
/// [`Serie::write_to`](Serie) produced). Errors for a nested field — the [`nested`](crate::io::nested)
/// module's reader handles those recursively.
pub fn read_any_leaf(field: &AnyField, source: &mut Bytes) -> Result<Box<dyn AnySerie>, IoError> {
    let leaf = field.as_leaf().ok_or_else(|| IoError::Unsupported {
        what: "read_any_leaf was given a nested field; use the nested reader".to_string(),
    })?;
    Ok(match FieldType::type_id(leaf) {
        DataTypeId::Null => Box::new(NullSerie::read_from(source)?),
        DataTypeId::U8 => Box::new(Serie::<u8>::read_from(source)?),
        DataTypeId::U16 => Box::new(Serie::<u16>::read_from(source)?),
        DataTypeId::U32 => Box::new(Serie::<u32>::read_from(source)?),
        DataTypeId::U64 => Box::new(Serie::<u64>::read_from(source)?),
        DataTypeId::U96 => Box::new(Serie::<U96>::read_from(source)?),
        DataTypeId::U128 => Box::new(Serie::<u128>::read_from(source)?),
        DataTypeId::U256 => Box::new(Serie::<U256>::read_from(source)?),
        DataTypeId::I8 => Box::new(Serie::<i8>::read_from(source)?),
        DataTypeId::I16 => Box::new(Serie::<i16>::read_from(source)?),
        DataTypeId::I32 => Box::new(Serie::<i32>::read_from(source)?),
        DataTypeId::I64 => Box::new(Serie::<i64>::read_from(source)?),
        DataTypeId::I96 => Box::new(Serie::<I96>::read_from(source)?),
        DataTypeId::I128 => Box::new(Serie::<i128>::read_from(source)?),
        DataTypeId::I256 => Box::new(Serie::<I256>::read_from(source)?),
        DataTypeId::F16 => Box::new(Serie::<f16>::read_from(source)?),
        DataTypeId::F32 => Box::new(Serie::<f32>::read_from(source)?),
        DataTypeId::F64 => Box::new(Serie::<f64>::read_from(source)?),
        DataTypeId::D32 => Box::new(DecimalSerie::<Dec32>::read_from(source)?),
        DataTypeId::D64 => Box::new(DecimalSerie::<Dec64>::read_from(source)?),
        DataTypeId::D128 => Box::new(DecimalSerie::<Dec128>::read_from(source)?),
        DataTypeId::D256 => Box::new(DecimalSerie::<Dec256>::read_from(source)?),
        DataTypeId::Utf8 => Box::new(Utf8Serie::read_from(source)?),
        DataTypeId::Binary => Box::new(BinarySerie::read_from(source)?),
        DataTypeId::FixedBinary => Box::new(FixedBinarySerie::read_from(source)?),
        DataTypeId::FixedUtf8 => Box::new(FixedUtf8Serie::read_from(source)?),
        other => {
            return Err(IoError::Unsupported {
                what: format!("cannot deserialize a leaf column of type {}", other.name()),
            })
        }
    })
}

/// Builds a **leaf** erased column from an Arrow array + its [`Field`](arrow_schema::Field) (feature
/// `arrow`), delegating to the matching `Serie`'s own zero-copy `from_arrow_array`. Errors for a
/// nested or unmodeled field.
#[cfg(feature = "arrow")]
pub fn from_arrow_any_leaf(
    array: &dyn arrow_array::Array,
    field: &arrow_schema::Field,
) -> Result<Box<dyn AnySerie>, IoError> {
    let leaf = Field::from_arrow(field).ok_or_else(|| unsupported(field))?;
    Ok(match FieldType::type_id(&leaf) {
        DataTypeId::Null => Box::new(NullSerie::with_len(array.len())),
        DataTypeId::U8 => Box::new(Serie::<u8>::from_arrow_array(down::<
            arrow_array::UInt8Array,
        >(array, field)?)),
        DataTypeId::U16 => Box::new(Serie::<u16>::from_arrow_array(down::<
            arrow_array::UInt16Array,
        >(array, field)?)),
        DataTypeId::U32 => Box::new(Serie::<u32>::from_arrow_array(down::<
            arrow_array::UInt32Array,
        >(array, field)?)),
        DataTypeId::U64 => Box::new(Serie::<u64>::from_arrow_array(down::<
            arrow_array::UInt64Array,
        >(array, field)?)),
        DataTypeId::I8 => Box::new(Serie::<i8>::from_arrow_array(
            down::<arrow_array::Int8Array>(array, field)?,
        )),
        DataTypeId::I16 => Box::new(Serie::<i16>::from_arrow_array(down::<
            arrow_array::Int16Array,
        >(array, field)?)),
        DataTypeId::I32 => Box::new(Serie::<i32>::from_arrow_array(down::<
            arrow_array::Int32Array,
        >(array, field)?)),
        DataTypeId::I64 => Box::new(Serie::<i64>::from_arrow_array(down::<
            arrow_array::Int64Array,
        >(array, field)?)),
        DataTypeId::F16 => Box::new(Serie::<f16>::from_arrow_array(down::<
            arrow_array::Float16Array,
        >(array, field)?)),
        DataTypeId::F32 => Box::new(Serie::<f32>::from_arrow_array(down::<
            arrow_array::Float32Array,
        >(array, field)?)),
        DataTypeId::F64 => Box::new(Serie::<f64>::from_arrow_array(down::<
            arrow_array::Float64Array,
        >(array, field)?)),
        DataTypeId::U96 => Box::new(wide_from_arrow::<U96>(array)),
        DataTypeId::U128 => Box::new(wide_from_arrow::<u128>(array)),
        DataTypeId::U256 => Box::new(wide_from_arrow::<U256>(array)),
        DataTypeId::I96 => Box::new(wide_from_arrow::<I96>(array)),
        DataTypeId::I128 => Box::new(wide_from_arrow::<i128>(array)),
        DataTypeId::I256 => Box::new(wide_from_arrow::<I256>(array)),
        DataTypeId::D32 => Box::new(DecimalSerie::<Dec32>::from_arrow_array(down::<
            arrow_array::Decimal32Array,
        >(
            array, field
        )?)),
        DataTypeId::D64 => Box::new(DecimalSerie::<Dec64>::from_arrow_array(down::<
            arrow_array::Decimal64Array,
        >(
            array, field
        )?)),
        DataTypeId::D128 => Box::new(DecimalSerie::<Dec128>::from_arrow_array(down::<
            arrow_array::Decimal128Array,
        >(
            array, field
        )?)),
        DataTypeId::D256 => Box::new(DecimalSerie::<Dec256>::from_arrow_array(down::<
            arrow_array::Decimal256Array,
        >(
            array, field
        )?)),
        DataTypeId::Utf8 => Box::new(Utf8Serie::from_arrow_array(down::<
            arrow_array::StringArray,
        >(array, field)?)?),
        DataTypeId::Binary => Box::new(BinarySerie::from_arrow_array(down::<
            arrow_array::BinaryArray,
        >(array, field)?)?),
        DataTypeId::FixedBinary => Box::new(FixedBinarySerie::from_arrow_array(down::<
            arrow_array::FixedSizeBinaryArray,
        >(
            array, field
        )?)?),
        DataTypeId::FixedUtf8 => Box::new(FixedUtf8Serie::from_arrow_array(down::<
            arrow_array::FixedSizeBinaryArray,
        >(
            array, field
        )?)?),
        _ => return Err(unsupported(field)),
    })
}

/// Downcasts a `PrimitiveArray<A>` (or other concrete array) or errors.
#[cfg(feature = "arrow")]
fn down<'a, A: 'static>(
    array: &'a dyn arrow_array::Array,
    field: &arrow_schema::Field,
) -> Result<&'a A, IoError> {
    array
        .as_any()
        .downcast_ref::<A>()
        .ok_or_else(|| unsupported(field))
}

/// Rebuilds a wide (non-Arrow-native) `Serie<T>` from an imported Arrow array's flat value bytes,
/// reading its **logical** window (offset-aware) and zeroing bytes under null slots.
#[cfg(feature = "arrow")]
fn wide_from_arrow<T: NativeType>(array: &dyn arrow_array::Array) -> Serie<T> {
    let width = T::WIDTH;
    let len = array.len();
    let data = array.to_data();
    let src = data.buffers()[0].as_slice();
    let base = data.offset() * width;
    let mut values = vec![0u8; len * width];
    let mut validity = None;
    for index in 0..len {
        if array.is_null(index) {
            validity
                .get_or_insert_with(|| crate::io::bitmap::Bitmap::all_present(len))
                .set(index, false);
        } else {
            let start = base + index * width;
            values[index * width..(index + 1) * width].copy_from_slice(&src[start..start + width]);
        }
    }
    Serie::from_byte_slice(values, validity, len)
}

/// The guided "Arrow type not modeled" error for a field the crate cannot import.
#[cfg(feature = "arrow")]
fn unsupported(field: &arrow_schema::Field) -> IoError {
    IoError::Unsupported {
        what: format!(
            "Arrow field {:?} of type {:?} is not a yggdryl-modeled column type",
            field.name(),
            field.data_type()
        ),
    }
}
