//! What a datatype, a field and a value each owe the root that holds them,
//! and what a family owes its leaves.
//!
//! [`DataType`], [`Field`] and [`Scalar`] are redirectors: each holds one
//! variant per family, and the family answers which leaf it is. The traits
//! here are the same verbs on the three sides, so a family reads the same
//! whichever side is being asked:
//!
//! | side | trait | widen | narrow |
//! | --- | --- | --- | --- |
//! | datatype | [`DataTypeValue`] | `into_dtype` | `from_dtype` |
//! | field | [`FieldValue`] | `into_field` | `from_field` |
//! | value | [`Value`] | `into_scalar` | `from_scalar` |
//! | family value | [`FamilyValue`] | `into_scalar` | `from_scalar` |
//!
//! The roots implement their own trait too - [`DataType`] is a
//! [`DataTypeValue`] and [`Field`] is a `FieldValue<DataType>` - so code that
//! is generic over a family works unchanged on the root that redirects to it.
//!
//! On the value side a family with several leaves is an enum over them -
//! [`Integer`], [`Floating`], [`Decimal`], [`Temporal`], [`Code`],
//! [`Geospatial`] and [`Nested`] - each a [`FamilyValue`]: it stands for any
//! one leaf, answers that leaf's datatype, widens to the scalar the leaf
//! widens to and narrows a scalar whose variant is one of its leaves. A kind
//! with one leaf value - a boolean, a string, a byte value, a UUID, and the
//! self-families a version, a URL, a time zone, a MIME type and a media type
//! are - has no enum: the leaf is the family. What the leaves of one family
//! share beyond that is the family's own trait - [`IntegerValue`],
//! [`FloatingValue`], [`DecimalValue`], [`TemporalValue`],
//! [`GeospatialValue`], [`CodeValue`] and [`NestedValue`] - declared here
//! and implemented beside each leaf.
//!
//! `canonical` is the schema-directed validation and canonicalization of row
//! values: a struct [`Field`] is the schema of the rows it describes, so
//! validating a row is validating one [`crate::sequence::Sequence`] against
//! that field's children, and canonicalization is the same walk with
//! rewriting - integers, floats and nested containers narrowed into the exact
//! representation the schema declares, and the input answered untouched when
//! nothing needed changing.
//!
//! [`Field`]: crate::Field
//! [`Scalar`]: crate::Scalar
//! [`Integer`]: crate::Integer
//! [`Floating`]: crate::Floating
//! [`Decimal`]: crate::Decimal
//! [`Temporal`]: crate::Temporal
//! [`Code`]: crate::Code
//! [`Geospatial`]: crate::Geospatial

mod canonical;

pub(crate) use canonical::*;

use std::fmt;
use std::hash::Hash;
use std::sync::Arc;

use smol_str::SmolStr;

use crate::{
    DataType, DataTypeId, DataTypeKind, Field, Metadata, Result, Scalar, TimeUnit, Timezone, i256,
};
use crate::{Mapping, Record, Sequence};

/// One concrete scalar representation.
///
/// Implementors are the final representation a [`Scalar`] variant holds.
/// Narrowing an existing scalar only projects a reference; validation remains
/// owned by [`DataType::scalar`](crate::DataType::scalar) and
/// [`Field::scalar`](crate::Field::scalar).
pub trait Value:
    Sized + Clone + fmt::Debug + fmt::Display + Eq + Ord + Hash + Send + Sync + 'static
{
    /// Return the datatype this value materializes into.
    ///
    /// Values whose physical parameters cannot be represented by a valid
    /// [`DataType`] return a typed error instead of guessing or panicking.
    fn dtype(&self) -> Result<DataType>;
    /// Widen this leaf to the dynamic scalar root.
    fn into_scalar(self) -> Scalar;
    /// Narrow a dynamic scalar to this leaf without re-validating it.
    fn from_scalar(value: &Scalar) -> Option<&Self>;
}

/// What a family's value enum owes: it stands for any one leaf of the family,
/// so it answers the leaf's datatype, widens to the scalar the leaf widens to,
/// and narrows a scalar whose variant is one of its leaves - by value, because
/// the scalar holds the leaf and not the family, and every leaf is `Copy` or one
/// shared pointer.
pub trait FamilyValue:
    Sized + Clone + fmt::Debug + fmt::Display + Eq + Ord + Hash + Send + Sync + 'static
{
    /// The kind every leaf of this family shares.
    const KIND: DataTypeKind;

    /// Return the datatype the held leaf materializes into.
    ///
    /// # Errors
    ///
    /// Returns the leaf's own refusal when its physical parameters cannot be
    /// represented by a valid [`DataType`].
    fn dtype(&self) -> Result<DataType>;
    /// Widen the held leaf to the dynamic scalar root.
    fn into_scalar(self) -> Scalar;
    /// Narrow a dynamic scalar to this family when its variant is one of the
    /// family's leaves.
    fn from_scalar(value: &Scalar) -> Option<Self>;
}

/// Emit one family's value enum over its leaves.
///
/// Every variant is named for the leaf it wraps, which is also the [`Scalar`]
/// variant that leaf widens to, so the enum, the scalar and the leaf share
/// one spelling: `Integer::Int32(Int32)` is `Scalar::Int32(Int32)`.
macro_rules! family_value {
    (
        $(#[$meta:meta])*
        $family:ident, $kind:ident, [$($leaf:ident),+ $(,)?]
    ) => {
        $(#[$meta])*
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub enum $family {
            $(
                #[doc = concat!("One `", stringify!($leaf), "`.")]
                $leaf($leaf),
            )+
        }

        impl ::std::fmt::Display for $family {
            fn fmt(&self, formatter: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                match self {
                    $(Self::$leaf(value) => ::std::fmt::Display::fmt(value, formatter),)+
                }
            }
        }

        impl $crate::FamilyValue for $family {
            const KIND: $crate::DataTypeKind = $crate::DataTypeKind::$kind;

            fn dtype(&self) -> $crate::Result<$crate::DataType> {
                match self {
                    $(Self::$leaf(value) => $crate::Value::dtype(value),)+
                }
            }

            fn into_scalar(self) -> $crate::Scalar {
                match self {
                    $(Self::$leaf(value) => $crate::Scalar::$leaf(value),)+
                }
            }

            fn from_scalar(value: &$crate::Scalar) -> Option<Self> {
                match value {
                    $($crate::Scalar::$leaf(value) => Some(Self::$leaf(value.clone())),)+
                    _ => None,
                }
            }
        }

        $(
            impl From<$leaf> for $family {
                fn from(value: $leaf) -> Self {
                    Self::$leaf(value)
                }
            }
        )+

        impl From<$family> for $crate::Scalar {
            fn from(value: $family) -> Self {
                $crate::FamilyValue::into_scalar(value)
            }
        }
    };
}

pub(crate) use family_value;

/// Operations shared by every signed and unsigned integer representation.
pub trait IntegerValue: Value {
    /// Whether this representation is signed.
    const SIGNED: bool;
    /// The physical width in bits.
    const BIT_WIDTH: u8;

    /// Return this integer as a signed 128-bit value when it fits.
    fn as_i128(&self) -> Option<i128>;
    /// Return this integer as an unsigned 128-bit value when it is non-negative.
    fn as_u128(&self) -> Option<u128>;
    /// Build this width from a signed 128-bit value.
    fn from_i128(value: i128) -> Result<Self>;
}

/// Operations shared by every IEEE floating-point representation.
pub trait FloatingValue: Value {
    /// The physical width in bits.
    const BIT_WIDTH: u8;

    /// Return this value widened to binary64.
    fn as_f64(&self) -> f64;
}

/// Operations shared by every exact-decimal representation.
pub trait DecimalValue: Value {
    /// Return the coefficient widened to 256 bits.
    fn coefficient(&self) -> i256;
    /// Return the decimal scale.
    fn scale(&self) -> i8;
    /// Return this value represented at `scale` without losing precision.
    fn rescale(self, scale: i8) -> Result<Self>;
}

/// Operations shared by every temporal representation.
pub trait TemporalValue: Value {
    /// The family's name: `date`, `time`, `datetime`, `duration` or
    /// `interval`, as a datatype spells it.
    const FAMILY: &'static str;
    /// The physical count width in bits.
    const BIT_WIDTH: u8;

    /// Return the stored count widened to 64 bits.
    fn count(&self) -> i64;
    /// Return the count's unit.
    fn unit(&self) -> TimeUnit;
    /// Return the explicit timezone marker.
    fn timezone(&self) -> Timezone;
    /// Convert this value to another valid unit.
    fn with_unit(self, unit: TimeUnit) -> Result<Self>;
    /// Restate this value with another valid timezone marker.
    fn with_timezone(self, timezone: Timezone) -> Result<Self>;
}

/// Borrowing access shared by geometry and geography values.
pub trait GeospatialValue: Value {
    /// Borrow the validated Well-Known Binary payload.
    fn as_bytes(&self) -> &[u8];
    /// Borrow the shared storage behind the payload.
    ///
    /// The payload is already validated WKB, so reinterpreting a geometry as
    /// a geography clones this handle rather than copying and re-reading it.
    fn storage(&self) -> &Arc<[u8]>;
}

/// Borrowing access shared by every code representation.
pub trait CodeValue: Value {
    /// The fixed storage width, in bytes.
    const WIDTH: usize;

    /// Borrow the validated code.
    fn as_str(&self) -> &str;
    /// Borrow the shared storage behind the validated code.
    ///
    /// The stored text is already trimmed and checked, so a rewrite that
    /// keeps it clones this handle rather than re-validating and copying.
    fn storage(&self) -> &SmolStr;

    /// The better statement of this code and another of the same kind: this
    /// one, unless it states less than `other` does.
    ///
    /// What "less" means is each code's own, and the codes that can state
    /// nothing say so: a [`Cfi`](crate::Cfi) fills every `X` position from the other
    /// where the two describe one instrument; a
    /// [`State`](crate::State) that reached none, `00UNKNOWN`, takes the other, and
    /// otherwise the further along stands; a [`Side`](crate::Side) `UNKNOWN`, a
    /// [`Currency`](crate::Currency) `XXX` and a [`Mic`](crate::Mic) `XXXX` take the other. Every other
    /// code is an identifier with nothing partial about it, so this one
    /// stands as it is. This is what a graph element folds two statements
    /// of one fact with.
    #[must_use]
    fn merge_with(self, other: &Self) -> Self {
        let _ = other;
        self
    }
}

/// What every nested value answers: its direct children, counted and walked.
pub trait NestedValue: Value {
    /// Return the number of direct children.
    fn len(&self) -> usize;
    /// Return whether this value has no direct children.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Iterate over direct sequence values, mapping keys, or record values.
    fn children(&self) -> Children<'_>;
}

family_value!(
    /// The nested family as one value: a sequence, a mapping or a record.
    ///
    /// ```
    /// use yggdryl::{DataType, FamilyValue, Nested, Scalar};
    ///
    /// let value = Scalar::from_sequence([Scalar::from(1_i64), Scalar::from(2_i64)]);
    /// let held = Nested::from_scalar(&value).expect("a sequence");
    /// assert!(matches!(held, Nested::Sequence(_)));
    /// assert_eq!(held.dtype().unwrap(), DataType::list(DataType::Int64.required_field("item")));
    /// assert_eq!(held.into_scalar(), value);
    /// assert_eq!(Nested::from_scalar(&Scalar::from(1_i64)), None);
    /// ```
    Nested, Nested, [Sequence, Mapping, Record]
);

/// The per-column facts a field carries that only one datatype has.
///
/// Almost every datatype answers `()`: a field's name, nullability, metadata
/// and Arrow projection are common to all of them and nothing else rides
/// along. Dictionary encoding is the exception - Arrow's IPC dictionary
/// identifier and its ordering flag describe that column and no other - so
/// the fact lives with that leaf instead of costing every field sixteen bytes
/// to say it has none.
pub trait FieldSidecar:
    Clone + fmt::Debug + Default + Eq + Ord + Hash + Send + Sync + Sized + 'static
{
    /// Return Arrow's IPC dictionary identifier, if this datatype has one.
    fn dictionary_id(&self) -> Option<i64> {
        None
    }

    /// Return Arrow's dictionary ordering flag, if this datatype has one.
    fn dictionary_is_ordered(&self) -> Option<bool> {
        None
    }

    /// Set both dictionary options, reporting whether anything changed.
    ///
    /// # Errors
    ///
    /// Returns an error for a datatype that carries no dictionary options,
    /// which is every datatype but the dictionary-encoded one.
    fn set_dictionary_options(&mut self, id: i64, is_ordered: bool) -> Result<bool> {
        let _ = (id, is_ordered);
        Err(crate::Error::InvalidDataType {
            kind: "Field",
            reason: "dictionary options require a dictionary datatype".into(),
        })
    }
}

/// A datatype whose fields carry nothing of their own.
impl FieldSidecar for () {}

/// Arrow's IPC dictionary identifier and ordering flag.
///
/// These describe one column's encoding, not its datatype: two dictionary
/// fields with different identifiers have the same datatype, which is why
/// this rides on the field rather than on [`crate::EnumType`].
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DictionaryOptions {
    pub(crate) id: i64,
    pub(crate) is_ordered: bool,
}

impl FieldSidecar for DictionaryOptions {
    fn dictionary_id(&self) -> Option<i64> {
        Some(self.id)
    }

    fn dictionary_is_ordered(&self) -> Option<bool> {
        Some(self.is_ordered)
    }

    fn set_dictionary_options(&mut self, id: i64, is_ordered: bool) -> Result<bool> {
        let changed = self.id != id || self.is_ordered != is_ordered;
        self.id = id;
        self.is_ordered = is_ordered;
        Ok(changed)
    }
}

/// One datatype: a family's payload, or the root that redirects to it.
///
/// The implementor is what a [`DataType`] variant holds - an enum over the
/// family's leaves when it has several, one leaf's parameters when it has one,
/// and a parameter-free marker for the variants that carry nothing. Either way
/// [`Self::id`] names the exact leaf, which is what a caller branching on the
/// variant actually wants.
pub trait DataTypeValue:
    Clone + fmt::Debug + fmt::Display + Eq + Ord + Hash + Send + Sync + Sized + 'static
{
    /// The family's parameter-free name, as a binding and a refusal spell it.
    const FAMILY: &'static str;

    /// The per-column facts a field of this datatype carries of its own.
    ///
    /// `()` for every datatype but the dictionary-encoded one.
    type Sidecar: FieldSidecar;

    /// Return the exact identifier of the leaf this payload holds.
    fn id(&self) -> DataTypeId;

    /// Return the category every leaf in this family belongs to.
    fn kind(&self) -> DataTypeKind;

    /// Reject a payload whose parameters cannot describe a column.
    ///
    /// Public variants stay constructible, so a caller can build a payload
    /// that is temporarily invalid; this is where that is caught, before the
    /// value crosses an interoperability boundary.
    fn validate(&self) -> Result<()>;

    /// Widen this payload to the datatype root.
    fn into_dtype(self) -> DataType;

    /// Narrow the datatype root to this payload.
    ///
    /// Owned rather than borrowed: a payload is not always literally what the
    /// variant holds - a parameter-free one is nothing at all, and a wrapper
    /// stands beside the variant's own parameters - so there is not always a
    /// reference to lend. Every payload is either `Copy` or one shared
    /// pointer, so producing one is cheap.
    fn from_dtype(dtype: &DataType) -> Option<Self>;

    // ---------------------------------------------------------------------
    // Casting. A datatype is always the *target*: a value or an array is
    // reconciled to it, never the other way around. Every family answers the
    // four doors through the root's one recursive walk, so a leaf never has a
    // cast rule the root does not have.
    // ---------------------------------------------------------------------

    /// Rewrite one value into this datatype, exactly.
    ///
    /// The crate's one value contract: the value is checked against the
    /// datatype and restated in the representation it declares - an integer
    /// narrowed to its width, a decimal restated at its scale, a temporal at
    /// its unit. A value that already matches comes back untouched.
    ///
    /// Nullability belongs to the field holding the column, so a null is the
    /// null of this datatype here; [`FieldValue::cast_scalar`] is where a
    /// column refuses one.
    ///
    /// # Errors
    ///
    /// Returns an error when the value cannot be stated in this datatype.
    fn cast_scalar(&self, value: &crate::Scalar) -> Result<crate::Scalar> {
        self.clone().into_dtype().cast_scalar(value)
    }

    /// Cast an Arrow array to this datatype's exact physical array.
    ///
    /// [`ArrowCastOptions::is_safe`](crate::ArrowCastOptions::is_safe) decides
    /// whether a conversion failure becomes null or an error. There is no
    /// field here, so nothing decides nullability: an incoming null stays one.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsupported cast or a value this datatype
    /// cannot hold.
    fn cast_arrow_array(
        &self,
        array: arrow_array::ArrayRef,
        options: crate::ArrowCastOptions,
    ) -> crate::arrow::Result<arrow_array::ArrayRef> {
        crate::cast::cast_dtype_arrow_array(&self.clone().into_dtype(), array, options)
    }

    /// Cast a one-row Arrow array to this datatype, as a scalar.
    ///
    /// # Errors
    ///
    /// Returns an error when the array does not hold exactly one row, or any
    /// error [`Self::cast_arrow_array`] returns.
    fn cast_arrow_scalar(
        &self,
        array: arrow_array::ArrayRef,
        options: crate::ArrowCastOptions,
    ) -> crate::arrow::Result<arrow_array::Scalar<arrow_array::ArrayRef>> {
        crate::cast::one_row(&array)?;
        Ok(arrow_array::Scalar::new(
            self.cast_arrow_array(array, options)?,
        ))
    }

    /// Reconcile an Arrow record batch to this Struct datatype.
    ///
    /// # Errors
    ///
    /// Returns an error unless this is a Struct datatype, or when a child cast
    /// or a missing-column default cannot be materialized.
    fn cast_arrow_batch(
        &self,
        batch: arrow_array::RecordBatch,
        options: crate::ArrowCastOptions,
    ) -> crate::arrow::Result<arrow_array::RecordBatch> {
        let root = Field::new("record", self.clone().into_dtype(), false);
        crate::cast::cast_field_arrow_batch(&root, batch, options)
    }

    /// Wrap a reader so every batch it yields is reconciled to this datatype.
    ///
    /// # Errors
    ///
    /// [`Self::cast_arrow_batch`] carries the rule, raised once from the
    /// reader's schema rather than once per batch.
    fn cast_arrow_reader(
        &self,
        reader: crate::arrow::BatchReader,
        options: crate::ArrowCastOptions,
    ) -> crate::arrow::Result<crate::arrow::BatchReader> {
        let root = Field::new("record", self.clone().into_dtype(), false);
        crate::cast::cast_field_arrow_reader(&root, reader, options)
    }
}

/// One field: a family's field, or the root that redirects to it.
///
/// A field is its datatype plus the per-column facts a datatype does not
/// carry - the name, the nullability, the metadata. `D` is the datatype the
/// implementor holds, so a leaf field answers with its own leaf datatype and
/// the root answers with [`DataType`]; nothing has to widen to ask.
///
/// [`Self::dtype`] always answers [`DataType`], whichever leaf the
/// implementor is, so a caller reading a datatype off a field never has to
/// know which one it holds; [`Self::typed_dtype`] is the dedicated accessor
/// that answers in the leaf's own type. Both return owned values, because a
/// leaf stores the leaf's parameters and has no whole [`DataType`] to lend
/// out; every payload is one shared pointer or nothing at all.
pub trait FieldValue<D: DataTypeValue>: Clone + fmt::Debug + fmt::Display + Sized {
    /// Return the physical field name without allocating.
    fn name(&self) -> &str;

    /// Return this field's datatype.
    fn dtype(&self) -> DataType;

    /// Return this field's datatype in its own type.
    fn typed_dtype(&self) -> D;

    /// Return whether this field admits nulls.
    fn is_nullable(&self) -> bool;

    /// Return the field's metadata without allocating.
    fn metadata(&self) -> &Metadata;

    /// Reject a field whose datatype and options cannot describe a column.
    fn validate(&self) -> Result<()>;

    /// Widen this field to the field root.
    fn into_field(self) -> Field;

    /// Narrow the field root to this family without cloning.
    fn from_field(field: &Field) -> Option<&Self>;

    // ---------------------------------------------------------------------
    // Casting. The field is the target, and it states one thing its datatype
    // does not: whether a null may stand. That is the whole difference
    // between these four doors and [`DataTypeValue`]'s.
    // ---------------------------------------------------------------------

    /// Rewrite one value into this field's column, exactly.
    ///
    /// [`DataTypeValue::cast_scalar`] plus the column's own rule: a null is
    /// refused by name unless this field admits one.
    ///
    /// # Errors
    ///
    /// Returns an error when the value cannot be stated in the field's
    /// datatype, or when it is null and the field is not nullable.
    fn cast_scalar(&self, value: &crate::Scalar) -> Result<crate::Scalar> {
        self.clone().into_field().scalar(value.clone())
    }

    /// Cast an Arrow array to this field's exact physical array.
    ///
    /// The field is the target: `array` is reconciled to its datatype *and*
    /// its nullability.
    /// [`ArrowCastOptions::nullability`](crate::ArrowCastOptions::nullability)
    /// decides what happens to a null a non-nullable field cannot hold - the
    /// canonical default, or a refusal naming the path.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsupported cast, a value that cannot satisfy
    /// the field, or a default that cannot be materialized.
    fn cast_arrow_array(
        &self,
        array: arrow_array::ArrayRef,
        options: crate::ArrowCastOptions,
    ) -> crate::arrow::Result<arrow_array::ArrayRef> {
        crate::cast::cast_field_arrow_array(&self.clone().into_field(), array, options)
    }

    /// Cast a one-row Arrow array to this field, as a scalar.
    ///
    /// # Errors
    ///
    /// Returns an error when the array does not hold exactly one row, or any
    /// error [`Self::cast_arrow_array`] returns.
    fn cast_arrow_scalar(
        &self,
        array: arrow_array::ArrayRef,
        options: crate::ArrowCastOptions,
    ) -> crate::arrow::Result<arrow_array::Scalar<arrow_array::ArrayRef>> {
        crate::cast::one_row(&array)?;
        Ok(arrow_array::Scalar::new(
            self.cast_arrow_array(array, options)?,
        ))
    }

    /// Reconcile an Arrow record batch to this Struct root.
    ///
    /// Children are selected in target order by ASCII-case-insensitive name.
    /// Extra source columns are dropped and missing nullable columns are
    /// null-filled. An already exact batch comes back unchanged - the same
    /// batch object, not a rebuilt one.
    ///
    /// # Errors
    ///
    /// Returns an error unless this is a bounded, non-nullable Struct root, or
    /// when a child cast or a missing-column default cannot be materialized.
    fn cast_arrow_batch(
        &self,
        batch: arrow_array::RecordBatch,
        options: crate::ArrowCastOptions,
    ) -> crate::arrow::Result<arrow_array::RecordBatch> {
        crate::cast::cast_field_arrow_batch(&self.clone().into_field(), batch, options)
    }

    /// Wrap a reader so every batch it yields is reconciled to this root.
    ///
    /// The plan is compiled once from the reader's schema, so the returned
    /// reader answers the cast schema before the first batch is pulled: a lake
    /// being read into a lake being written is never held in memory to be
    /// reconciled.
    ///
    /// # Errors
    ///
    /// [`Self::cast_arrow_batch`] carries the rule, raised once from the
    /// reader's schema rather than once per batch.
    fn cast_arrow_reader(
        &self,
        reader: crate::arrow::BatchReader,
        options: crate::ArrowCastOptions,
    ) -> crate::arrow::Result<crate::arrow::BatchReader> {
        crate::cast::cast_field_arrow_reader(&self.clone().into_field(), reader, options)
    }
}

/// The root is a datatype like any other family payload.
impl DataTypeValue for DataType {
    const FAMILY: &'static str = "datatype";

    // The root can be any datatype, so it carries the widest sidecar.
    type Sidecar = DictionaryOptions;

    fn id(&self) -> DataTypeId {
        Self::id(self)
    }

    fn kind(&self) -> DataTypeKind {
        Self::kind(self)
    }

    fn validate(&self) -> Result<()> {
        Self::validate(self)
    }

    fn into_dtype(self) -> DataType {
        self
    }

    fn from_dtype(dtype: &DataType) -> Option<Self> {
        Some(dtype.clone())
    }
}

/// A borrowed iterator over sequence values, mapping keys or record values.
pub enum Children<'a> {
    /// Sequence values.
    Sequence(std::slice::Iter<'a, Scalar>),
    /// Mapping keys.
    Mapping(std::slice::Iter<'a, (Scalar, Scalar)>),
    /// Record field values in sorted name order.
    Record(std::collections::btree_map::Values<'a, SmolStr, Scalar>),
}

impl<'a> Iterator for Children<'a> {
    type Item = &'a Scalar;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Sequence(values) => values.next(),
            Self::Mapping(entries) => entries.next().map(|(key, _)| key),
            Self::Record(entries) => entries.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let length = self.len();
        (length, Some(length))
    }
}

impl DoubleEndedIterator for Children<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        match self {
            Self::Sequence(values) => values.next_back(),
            Self::Mapping(entries) => entries.next_back().map(|(key, _)| key),
            Self::Record(entries) => entries.next_back(),
        }
    }
}

impl ExactSizeIterator for Children<'_> {
    fn len(&self) -> usize {
        match self {
            Self::Sequence(values) => values.len(),
            Self::Mapping(entries) => entries.len(),
            Self::Record(entries) => entries.len(),
        }
    }
}

impl std::iter::FusedIterator for Children<'_> {}

/// Emit a datatype payload that stands beside one variant's parameters.
///
/// The variant already holds what describes the column; this is the type a
/// field of that variant carries, so reading one out of a datatype and putting
/// it back are the two halves written here once.
macro_rules! payload_datatype {
    (
        $(#[$meta:meta])*
        $name:ident, $variant:ident, $kind:ident,
        fields { $($field:ident : $ty:ty),+ $(,)? },
        read $read:pat => $build:expr,
        write $write:expr $(,)?
    ) => {
        $(#[$meta])*
        #[doc = concat!(
            "The datatype of a [`DataType::",
            stringify!($variant),
            "`](crate::DataType::",
            stringify!($variant),
            ") field."
        )]
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name {
            $(pub(crate) $field: $ty,)+
        }

        impl $name {
            /// Builds this payload from its parts.
            pub const fn new($($field: $ty),+) -> Self {
                Self { $($field),+ }
            }

            $(
                #[doc = concat!("Returns this datatype's `", stringify!($field), "`.")]
                pub const fn $field(&self) -> &$ty {
                    &self.$field
                }
            )+
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.clone().into_dtype(), formatter)
            }
        }

        impl DataTypeValue for $name {
            const FAMILY: &'static str = DataTypeId::$variant.as_str();

            type Sidecar = ();

            fn id(&self) -> DataTypeId {
                DataTypeId::$variant
            }

            fn kind(&self) -> DataTypeKind {
                DataTypeKind::$kind
            }

            fn validate(&self) -> Result<()> {
                self.clone().into_dtype().validate()
            }

            fn into_dtype(self) -> DataType {
                let Self { $($field),+ } = self;
                $write
            }

            fn from_dtype(dtype: &DataType) -> Option<Self> {
                match dtype {
                    $read => Some($build),
                    _ => None,
                }
            }
        }
    };
}

payload_datatype!(
    UnionType, Union, Nested,
    fields { fields: crate::UnionFields, mode: crate::UnionMode },
    read DataType::Union(fields, mode) => Self::new(fields.clone(), *mode),
    write DataType::Union(fields, mode),
);

payload_datatype!(
    RunEndType, RunEndEncoded, Nested,
    fields { encoding: std::sync::Arc<crate::RunEndEncodedType> },
    read DataType::RunEndEncoded(encoding) => Self::new(std::sync::Arc::clone(encoding)),
    write DataType::RunEndEncoded(encoding),
);

payload_datatype!(
    GeometryType, Geometry, Geospatial,
    fields { parameters: std::sync::Arc<crate::GeospatialParameters> },
    read DataType::Geometry(parameters) => Self::new(std::sync::Arc::clone(parameters)),
    write DataType::Geometry(parameters),
);

payload_datatype!(
    GeographyType, Geography, Geospatial,
    fields { parameters: std::sync::Arc<crate::GeospatialParameters> },
    read DataType::Geography(parameters) => Self::new(std::sync::Arc::clone(parameters)),
    write DataType::Geography(parameters),
);

// ------------------------------------------------------------------------
// The C Data Interface parts a nested family writes.
// ------------------------------------------------------------------------

/// What one nested datatype contributes to its C Data Interface schema.
///
/// Arrow's own `FFI_ArrowSchema` conversion drops the flags a nested datatype
/// owns - sorted map keys above all - when it adds a field's flags on top, so
/// this crate builds the node itself. Each family answers the four parts of
/// its own node here and [`crate::DataType::into_arrow_datatype_ffi`] assembles
/// them, so no family's flags are lost to a shared walker.
pub(crate) struct ArrowFfiParts {
    /// The format string Arrow's C interface names this layout by.
    pub(crate) format: String,
    /// One C schema per child field, in the order the layout declares them.
    pub(crate) children: Vec<arrow_schema::ffi::FFI_ArrowSchema>,
    /// The values schema, for a layout whose values are dictionary-encoded.
    pub(crate) dictionary: Option<arrow_schema::ffi::FFI_ArrowSchema>,
    /// The flags this layout owns, before a field adds its own.
    pub(crate) flags: arrow_schema::ffi::Flags,
}

impl ArrowFfiParts {
    /// The parts of a node that carries children and no flags of its own.
    pub(crate) fn nested(
        format: impl Into<String>,
        children: Vec<arrow_schema::ffi::FFI_ArrowSchema>,
    ) -> Self {
        Self {
            format: format.into(),
            children,
            dictionary: None,
            flags: arrow_schema::ffi::Flags::empty(),
        }
    }
}
