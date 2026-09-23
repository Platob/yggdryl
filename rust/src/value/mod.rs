//! What a datatype, a field and a value each owe the root that holds them,
//! and what the leaves of one family share.
//!
//! [`DataType`], [`Field`] and [`Scalar`] are redirectors: each holds one
//! variant per leaf, or, where one payload names the leaf - the eighteen
//! string leaves under `String(StringType)`, the six byte leaves under
//! `Bytes(BytesType)` - one variant for them all, and the traits here are
//! the same verbs on the three sides, so a leaf reads the same whichever
//! side is being asked:
//!
//! | side | trait | widen | narrow |
//! | --- | --- | --- | --- |
//! | datatype | [`DataTypeValue`] | `into_dtype` | `from_dtype` |
//! | field | [`FieldValue`] | `into_field` | `from_field` |
//! | value | [`Value`] | `into_scalar` | `from_scalar` |
//! | column | [`SerieValue`] | `into_serie` | `from_serie` |
//!
//! The roots implement their own trait too - [`DataType`] is a
//! [`DataTypeValue`] and [`Field`] is a `FieldValue<DataType>` - so code that
//! is generic over a payload works unchanged on the root that redirects to
//! it.
//!
//! A family is no type of its own: it is the range of
//! [`DataTypeId`] bytes its [`DataTypeKind`] owns, so "is this an integer"
//! is [`DataTypeKind::contains`] over a value's, a field's or a column's
//! [`id`](crate::Scalar::id), and the value itself is the leaf the
//! [`Scalar`] variant holds. What the leaves of one family share is a leaf
//! contract - [`IntegerValue`], [`FloatingValue`], [`DecimalValue`],
//! [`TemporalValue`], [`GeospatialValue`], [`CodeValue`] and
//! [`NestedValue`] - declared here and implemented beside each leaf.
//!
//! A fourth side stands beside the three: many values of one field, which is
//! a column. The root is [`Serie`] - one column leaf per storage layout,
//! beside the schema-free [`Run`] a row canonicalizes to - and
//! [`SerieValue`] is what each column leaf owes it. A serie is a value as
//! well, because it *is* the value a serie holds, `Scalar::Serie(Serie)`:
//! [`Serie`] implements [`Value`] and [`NestedValue`], and nothing about a
//! column is a second value model. It does not implement [`SerieValue`],
//! whose every method answers from a field, because the run leaf declares
//! none.
//!
//! `canonical` is the schema-directed validation and canonicalization of row
//! values: a struct [`Field`] is the schema of the rows it describes, so
//! validating a row is validating one [`Run`] against that field's
//! children, and canonicalization is the same walk with rewriting -
//! integers, floats and nested containers narrowed into the exact
//! representation the schema declares, and the input answered untouched when
//! nothing needed changing.
//!
//! [`Field`]: crate::Field
//! [`Run`]: crate::Run
//! [`Serie`]: crate::Serie
//! [`Scalar`]: crate::Scalar

mod canonical;

pub(crate) use canonical::*;

use std::borrow::Cow;
use std::fmt;
use std::hash::Hash;
use std::ops::Range;
use std::sync::Arc;

use arrow_array::ArrayRef;
use smol_str::SmolStr;

use crate::{
    DataType, DataTypeId, DataTypeKind, Field, Metadata, Result, Scalar, Serie, TimeUnit, Timezone,
    i256,
};

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

    /// Encode this value using the Apache Parquet Variant version-one mapping.
    ///
    /// The mapping preserves the standard's representations; use the value
    /// stream when widths, charsets or other leaf parameters must survive.
    // Generic encoding borrows the leaf and returns owned encoded bytes,
    // matching Scalar::into_variant without moving values out of row holders.
    #[allow(clippy::wrong_self_convention)]
    fn into_variant(&self) -> Result<crate::Variant> {
        self.clone().into_scalar().into_variant()
    }

    /// Decode a variant and narrow its result to this concrete leaf.
    ///
    /// This has the exact projection semantics of [`Self::from_scalar`].
    /// A different decoded leaf is an error, even when a cast could convert it;
    /// use [`DataType::decode_variant`] to request that cast explicitly.
    fn from_variant(value: &crate::Variant) -> Result<Self> {
        let decoded = Scalar::from_variant(value)?;
        Self::from_scalar(&decoded)
            .cloned()
            .ok_or_else(|| crate::Error::InvalidRecord {
                path: "$".into(),
                reason: smol_str::format_smolstr!(
                    "expected {}, variant decoded as {}",
                    std::any::type_name::<Self>(),
                    decoded.id().as_str()
                ),
            })
    }
}

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
    /// nothing say so: a [`CfiCode`](crate::CfiCode) fills every `X` position from the other
    /// where the two describe one instrument; a
    /// [`State`](crate::State) that reached none, `00UNKNOWN`, takes the other, and
    /// otherwise the further along stands; a [`Side`](crate::Side) `UNKNOWN`, a
    /// [`Currency`](crate::Currency) `XXX` and a [`MicCode`](crate::MicCode) `XXXX` take the other. Every other
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

/// One column: the leaf that holds one storage layout's buffers.
///
/// A column is many values of one field, and it holds them the way Arrow
/// lays them out - a values buffer, offsets where the layout has them, a
/// validity bitmap - never one boxed value per row. The field is the
/// authority the rest of the project already uses: it decides nullability,
/// dictionary options and extension identity, and it says which of the
/// crate's leaves the buffers under it are.
///
/// What every column owes is this trait; what one leaf's buffers *are* is the
/// leaf's own inherent surface - [`Int32Serie::values`](crate::Int32Serie::values)
/// lends `&[i32]`, [`StructSerie::child`](crate::StructSerie::child) lends a
/// child column - because a buffer is the one thing a family cannot share a
/// spelling for.
///
/// Every write is one verb, [`Self::splice`], and the rest are spelled over
/// it. A column holds only rows its field accepts: `splice` proves every row
/// through the field's contract exactly once, then checks what a write could
/// not do, then writes without failing - so a refusal leaves the column
/// exactly as it was, and a stored row never refuses to be read.
///
/// [`Serie`] itself does *not* implement this, because its
/// [`Run`](crate::Serie::Run) leaf is a schema-free run with no field to
/// answer `field` with. The root answers the same verbs inherently, with
/// [`Serie::field`] returning `Option`; this trait is what a column leaf
/// owes.
///
/// ```
/// use yggdryl::{DataType, Field, Int64Serie, Scalar, Serie, SerieValue};
///
/// # fn main() -> yggdryl::Result<()> {
/// let field = Field::new("size", DataType::Int64, true);
/// let serie = Serie::from_scalars(field, [Scalar::from(7_i64), Scalar::Null])?;
///
/// // The column leaf owes this contract.
/// let column: &Int64Serie = serie.as_int64().expect("an int64 column");
/// assert_eq!(SerieValue::field(column).name(), "size");
/// assert_eq!(SerieValue::len(column), 2);
/// assert_eq!(SerieValue::scalar(column, 0)?, Scalar::from(7_i64));
/// assert!(SerieValue::is_null(column, 1)?);
///
/// // A write goes through the field's contract and lands in the buffer.
/// let mut owned = column.clone();
/// owned.push(Scalar::from(9_i8))?;
/// assert_eq!(owned.values(), &[7, 0, 9]);
/// assert!(owned.set(9, Scalar::Null).is_err());
///
/// // The root answers the same verbs, and says a run has no field.
/// assert_eq!(serie.field().map(|held| held.name()), Some("size"));
/// assert_eq!(Serie::new(vec![Scalar::from(7_i64)]).field(), None);
/// # Ok(())
/// # }
/// ```
// `into_arrow_array` borrows: the buffers are already shared, and moving the
// column to share them would make every caller clone it first.
#[allow(clippy::wrong_self_convention)]
pub trait SerieValue:
    Clone + fmt::Debug + fmt::Display + Eq + Ord + Hash + Send + Sync + Sized + 'static
{
    /// Return the field every row of this column is typed by.
    fn field(&self) -> &Field;

    /// Return the shared field, without cloning it.
    fn field_ref(&self) -> &Arc<Field>;

    /// Return the identifier of the datatype every row is typed by: the
    /// field's, because one layout holds several - a UTF-8 column holds
    /// every string leaf laid out as UTF-8, a duration column both widths -
    /// so the leaf alone never says which. A dictionary or run-end column
    /// answers its encoding, not the rows it yields.
    ///
    /// ```
    /// use yggdryl::{DataType, DataTypeId, DataTypeKind, Field, Serie, SerieValue, TimeUnit};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let dtype = DataType::duration32(TimeUnit::Millisecond)?;
    /// let serie = Serie::empty(Field::new("at", dtype, true))?;
    /// let column = serie.as_duration_millisecond().expect("a millisecond duration column");
    /// assert_eq!(column.id(), DataTypeId::Duration32);
    /// assert_eq!(column.kind(), DataTypeKind::Temporal);
    /// # Ok(())
    /// # }
    /// ```
    fn id(&self) -> DataTypeId {
        self.field().id()
    }

    /// Return the family the column's datatype belongs to: the one whose
    /// [range](DataTypeKind::range) [`Self::id`] is in.
    fn kind(&self) -> DataTypeKind {
        self.id().kind()
    }

    /// Return the number of rows.
    ///
    /// Constant: a column reads its length off its buffers.
    fn len(&self) -> usize;

    /// Return whether this column holds no rows.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Return how many rows hold no value.
    ///
    /// Constant: the validity bitmap keeps the count, and an encoding counts
    /// its logical nulls once.
    fn null_count(&self) -> usize;

    /// Return whether row `index` holds no value.
    ///
    /// # Errors
    ///
    /// Returns an error naming the column and both counts when `index` is
    /// past the end.
    fn is_null(&self, index: usize) -> Result<bool>;

    /// Build row `index` as a value.
    ///
    /// One row is read off the buffers here; the rest are not touched. A
    /// stored row is one its field accepts - the door proved it - so the only
    /// refusal is an index past the end.
    ///
    /// # Errors
    ///
    /// Returns an error naming the column and both counts when `index` is
    /// past the end.
    fn scalar(&self, index: usize) -> Result<Scalar>;

    /// Return the window `offset..offset + length`, sharing the buffers.
    ///
    /// Zero copy: an Arrow slice of the values and the validity, and a
    /// nested column slices its children to the reached window.
    ///
    /// # Errors
    ///
    /// Returns an error naming the column and both counts when the window
    /// reaches past the end.
    fn slice(&self, offset: usize, length: usize) -> Result<Self>;

    /// Replace `range` by `rows`: the one mutation every other write is
    /// spelled over.
    ///
    /// Proves every row through the field's contract once, checks what a
    /// write could not do - an offsets total past the offset type, a fixed
    /// width total past `i32` - and then writes without failing, so a
    /// refusal leaves the column exactly as it was. The buffers are written
    /// in place where this column holds them alone and copied once where it
    /// does not.
    ///
    /// # Errors
    ///
    /// Returns an error naming the column when `range` is reversed or reaches
    /// past the end, when a row is not one the field accepts, or when the
    /// layout cannot hold the total the write would reach.
    fn splice(&mut self, range: Range<usize>, rows: Vec<Scalar>) -> Result<()>;

    /// Return this column's rows as the Arrow array they already are.
    ///
    /// The buffers are shared, never copied; a nested column assembles from
    /// its children's arrays, which are aligned by construction.
    fn into_arrow_array(&self) -> ArrayRef;

    /// Widen this column to the dynamic serie root.
    fn into_serie(self) -> Serie;

    /// Narrow a dynamic serie to this leaf without re-validating it.
    fn from_serie(value: &Serie) -> Option<&Self>;

    /// Overwrite row `index` with `value`, through the field's contract.
    ///
    /// # Errors
    ///
    /// Returns an error when `index` is past the end, or [`Self::splice`]'s
    /// refusal.
    fn set(&mut self, index: usize, value: Scalar) -> Result<()> {
        crate::serie::require_row(self.field().name(), index, self.len())?;
        self.splice(index..index + 1, vec![value])
    }

    /// Append one row, through the field's contract.
    ///
    /// Amortized in place once the column owns its buffers.
    ///
    /// # Errors
    ///
    /// [`Self::splice`] carries the rule.
    fn push(&mut self, value: Scalar) -> Result<()> {
        let len = self.len();
        self.splice(len..len, vec![value])
    }

    /// Insert one row before `index`, which may be `len`.
    ///
    /// # Errors
    ///
    /// Returns an error when `index` is past `len`, or [`Self::splice`]'s
    /// refusal.
    fn insert(&mut self, index: usize, value: Scalar) -> Result<()> {
        crate::serie::require_range(self.field().name(), &(index..index), self.len())?;
        self.splice(index..index, vec![value])
    }

    /// Remove row `index` and answer it.
    ///
    /// # Errors
    ///
    /// Returns an error when `index` is past the end.
    fn remove(&mut self, index: usize) -> Result<Scalar> {
        let row = self.scalar(index)?;
        self.splice(index..index + 1, Vec::new())?;
        Ok(row)
    }

    /// Remove the last row and answer it, or `None` when the column is empty.
    ///
    /// # Errors
    ///
    /// [`Self::splice`] carries the rule.
    fn pop(&mut self) -> Result<Option<Scalar>> {
        match self.len().checked_sub(1) {
            Some(last) => self.remove(last).map(Some),
            None => Ok(None),
        }
    }

    /// Drop every row from `len` on; a no-op when the column is no longer.
    ///
    /// # Errors
    ///
    /// [`Self::splice`] carries the rule.
    fn truncate(&mut self, len: usize) -> Result<()> {
        let held = self.len();
        if len >= held {
            return Ok(());
        }
        self.splice(len..held, Vec::new())
    }

    /// Drop every row and keep the field.
    ///
    /// # Errors
    ///
    /// [`Self::splice`] carries the rule.
    fn clear(&mut self) -> Result<()> {
        self.truncate(0)
    }

    /// Append `rows`, each through the field's contract.
    ///
    /// # Errors
    ///
    /// [`Self::splice`] carries the rule.
    fn extend(&mut self, rows: Vec<Scalar>) -> Result<()> {
        let len = self.len();
        self.splice(len..len, rows)
    }

    /// Truncate to `len`, or grow to it with clones of `value`.
    ///
    /// `value` is proved once; the clones re-enter [`Self::splice`] already
    /// canonical, where the field's contract answers them untouched.
    ///
    /// # Errors
    ///
    /// Returns an error when `value` is not one the field accepts, or
    /// [`Self::splice`]'s refusal.
    fn resize(&mut self, len: usize, value: Scalar) -> Result<()> {
        let held = self.len();
        if len <= held {
            return self.truncate(len);
        }
        let canonical = self.field().scalar(value)?;
        self.splice(held..held, vec![canonical; len - held])
    }
}

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

    /// Return the family the leaf this payload holds belongs to: the one
    /// whose [range](DataTypeKind::range) its identifier is in.
    fn kind(&self) -> DataTypeKind {
        self.id().kind()
    }

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

    // ---------------------------------------------------------------------
    // The variant encoding: `value` cast to this datatype and encoded, and
    // encoded bytes read back and cast to it. `crate::valuestream` owns the
    // bytes; a leaf answers through the datatype it widens to.
    // ---------------------------------------------------------------------

    /// `value` cast to this datatype and encoded as [the value
    /// stream](crate::Scalar::encode_value_stream_bytes), one chunk at
    /// a time.
    ///
    /// # Errors
    ///
    /// Returns the cast's refusal where the value is not one this datatype
    /// holds.
    fn encode_value_stream_bytes(&self, value: &crate::Scalar) -> Result<crate::ValueStream> {
        self.clone().into_dtype().encode_value_stream_bytes(value)
    }

    /// `value` cast to this datatype and encoded, whole.
    ///
    /// # Errors
    ///
    /// Returns the cast's refusal where the value is not one this datatype
    /// holds.
    fn encode_value_bytes(&self, value: &crate::Scalar) -> Result<Vec<u8>> {
        self.clone().into_dtype().encode_value_bytes(value)
    }

    /// The value one value stream holds, cast to this datatype.
    ///
    /// # Errors
    ///
    /// Returns the codec's refusal, or the cast's where the bytes hold a
    /// value this datatype does not.
    fn decode_value_bytes(&self, bytes: &[u8]) -> Result<crate::Scalar> {
        self.clone().into_dtype().decode_value_bytes(bytes)
    }

    /// The value a value stream split into chunks holds, cast to this
    /// datatype.
    ///
    /// # Errors
    ///
    /// [`Self::decode_value_bytes`] carries the rule.
    fn decode_value_stream_bytes<I>(&self, chunks: I) -> Result<crate::Scalar>
    where
        I: IntoIterator,
        I::Item: AsRef<[u8]>,
    {
        self.clone().into_dtype().decode_value_stream_bytes(chunks)
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
}

/// The root is a datatype like any other family payload.
impl DataTypeValue for DataType {
    const FAMILY: &'static str = "datatype";

    // The root can be any datatype, so it carries the widest sidecar.
    type Sidecar = DictionaryOptions;

    fn id(&self) -> DataTypeId {
        Self::id(self)
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

/// An iterator over sequence values, mapping keys or record values.
///
/// Three of the four shapes already hold the values and lend them; a column
/// holds Arrow buffers and builds one row at a time, so the item is a
/// [`Cow`] - borrowed where the value is already there, owned where it had
/// to be built. A walk therefore costs nothing extra on the shapes that were
/// always values, and costs one row at a time on the one that is not.
pub enum Children<'a> {
    /// Sequence values.
    Sequence(std::slice::Iter<'a, Scalar>),
    /// Mapping keys.
    Mapping(std::slice::Iter<'a, (Scalar, Scalar)>),
    /// Struct field values in sorted name order.
    Struct(std::collections::btree_map::Values<'a, SmolStr, Scalar>),
    /// A column's rows, each built as it is reached.
    Column(ColumnRows<'a>),
}

/// The rows of one column, built one at a time as the walk reaches them.
///
/// A column holds only rows its field accepts (the door proved them), so a
/// row is built and never refused; the walk stops at the column's length.
pub struct ColumnRows<'a> {
    column: &'a Serie,
    front: usize,
    back: usize,
}

impl<'a> ColumnRows<'a> {
    /// Walk every row of `column`.
    pub(crate) fn new(column: &'a Serie) -> Self {
        Self {
            column,
            front: 0,
            back: column.len(),
        }
    }

    /// Build row `index`, which the walk keeps below the column's length.
    fn row(&self, index: usize) -> Scalar {
        crate::serie::proven_row(self.column, index)
    }
}

impl Iterator for ColumnRows<'_> {
    type Item = Scalar;

    fn next(&mut self) -> Option<Self::Item> {
        if self.front >= self.back {
            return None;
        }
        let row = self.row(self.front);
        self.front += 1;
        Some(row)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let length = self.back - self.front;
        (length, Some(length))
    }
}

impl DoubleEndedIterator for ColumnRows<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.front >= self.back {
            return None;
        }
        self.back -= 1;
        Some(self.row(self.back))
    }
}

impl ExactSizeIterator for ColumnRows<'_> {
    fn len(&self) -> usize {
        self.back - self.front
    }
}

impl std::iter::FusedIterator for ColumnRows<'_> {}

impl<'a> Iterator for Children<'a> {
    type Item = Cow<'a, Scalar>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Sequence(values) => values.next().map(Cow::Borrowed),
            Self::Mapping(entries) => entries.next().map(|(key, _)| Cow::Borrowed(key)),
            Self::Struct(entries) => entries.next().map(Cow::Borrowed),
            Self::Column(rows) => rows.next().map(Cow::Owned),
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
            Self::Sequence(values) => values.next_back().map(Cow::Borrowed),
            Self::Mapping(entries) => entries.next_back().map(|(key, _)| Cow::Borrowed(key)),
            Self::Struct(entries) => entries.next_back().map(Cow::Borrowed),
            Self::Column(rows) => rows.next_back().map(Cow::Owned),
        }
    }
}

impl ExactSizeIterator for Children<'_> {
    fn len(&self) -> usize {
        match self {
            Self::Sequence(values) => values.len(),
            Self::Mapping(entries) => entries.len(),
            Self::Struct(entries) => entries.len(),
            Self::Column(rows) => rows.len(),
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
        $name:ident, $variant:ident,
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
    UnionType, Union,
    fields { fields: crate::UnionFields, mode: crate::UnionMode },
    read DataType::Union(fields, mode) => Self::new(fields.clone(), *mode),
    write DataType::Union(fields, mode),
);

payload_datatype!(
    RunEndType, RunEndEncoded,
    fields { encoding: std::sync::Arc<crate::RunEndEncodedType> },
    read DataType::RunEndEncoded(encoding) => Self::new(std::sync::Arc::clone(encoding)),
    write DataType::RunEndEncoded(encoding),
);

payload_datatype!(
    GeometryType, Geometry,
    fields { parameters: std::sync::Arc<crate::GeospatialParameters> },
    read DataType::Geometry(parameters) => Self::new(std::sync::Arc::clone(parameters)),
    write DataType::Geometry(parameters),
);

payload_datatype!(
    GeographyType, Geography,
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
