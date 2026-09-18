//! What a datatype, a field and a value each owe the root that holds them.
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
//! | value | [`crate::Value`] | `into_scalar` | `from_scalar` |
//!
//! The roots implement their own trait too - [`DataType`] is a
//! [`DataTypeValue`] and [`Field`] is a `FieldValue<DataType>` - so code that
//! is generic over a family works unchanged on the root that redirects to it.
//!
//! [`Field`]: crate::Field
//! [`Scalar`]: crate::Scalar

use std::fmt;
use std::hash::Hash;

use smol_str::SmolStr;

use crate::{DataType, DataTypeId, DataTypeKind, Field, Metadata, Result, Scalar};

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
/// this rides on the field rather than on [`crate::types::EnumType`].
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

pub trait NestedValue: crate::Value {
    /// Return the number of direct children.
    fn len(&self) -> usize;
    /// Return whether this value has no direct children.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Iterate over direct sequence values, mapping keys, or record values.
    fn children(&self) -> Children<'_>;
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
    DateTime64Type, DateTime64, Temporal,
    fields { unit: crate::TimeUnit, timezone: crate::Timezone },
    read DataType::DateTime64 { unit, timezone } => Self::new(*unit, timezone.clone()),
    write DataType::DateTime64 { unit, timezone },
);

payload_datatype!(
    Time32Type, Time32, Temporal,
    fields { unit: crate::TimeUnit },
    read DataType::Time32(unit) => Self::new(*unit),
    write DataType::Time32(unit),
);

payload_datatype!(
    Time64Type, Time64, Temporal,
    fields { unit: crate::TimeUnit },
    read DataType::Time64(unit) => Self::new(*unit),
    write DataType::Time64(unit),
);

payload_datatype!(
    Duration32Type, Duration32, Temporal,
    fields { unit: crate::TimeUnit },
    read DataType::Duration32(unit) => Self::new(*unit),
    write DataType::Duration32(unit),
);

payload_datatype!(
    Duration64Type, Duration64, Temporal,
    fields { unit: crate::TimeUnit },
    read DataType::Duration64(unit) => Self::new(*unit),
    write DataType::Duration64(unit),
);

payload_datatype!(
    IntervalType, Interval, Temporal,
    fields { unit: crate::TimeUnit },
    read DataType::Interval(unit) => Self::new(*unit),
    write DataType::Interval(unit),
);

payload_datatype!(
    UnionType, Union, Nested,
    fields { fields: crate::UnionFields, mode: crate::UnionMode },
    read DataType::Union(fields, mode) => Self::new(fields.clone(), *mode),
    write DataType::Union(fields, mode),
);

payload_datatype!(
    Decimal32Type, Decimal32, Decimal,
    fields { precision: u8, scale: i8 },
    read DataType::Decimal32 { precision, scale } => Self::new(*precision, *scale),
    write DataType::Decimal32 { precision, scale },
);

payload_datatype!(
    Decimal64Type, Decimal64, Decimal,
    fields { precision: u8, scale: i8 },
    read DataType::Decimal64 { precision, scale } => Self::new(*precision, *scale),
    write DataType::Decimal64 { precision, scale },
);

payload_datatype!(
    Decimal128Type, Decimal128, Decimal,
    fields { precision: u8, scale: i8 },
    read DataType::Decimal128 { precision, scale } => Self::new(*precision, *scale),
    write DataType::Decimal128 { precision, scale },
);

payload_datatype!(
    Decimal256Type, Decimal256, Decimal,
    fields { precision: u8, scale: i8 },
    read DataType::Decimal256 { precision, scale } => Self::new(*precision, *scale),
    write DataType::Decimal256 { precision, scale },
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
