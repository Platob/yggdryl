//! Enum: a value stored as a code that stands for it.
//!
//! One family, one file. Dictionary encoding is its first leaf - a key column
//! over a value column - and the family exists around it because every other
//! way of storing a value as a code for it is a leaf beside that one, not a
//! new variant on [`DataType`] and not a new spelling at every call site.
//!
//! The name is about what the column means, not how it is laid out: the
//! values are drawn from a closed set and the rows carry codes into it. That
//! is a different fact from [`crate::Vocabulary`], which is the closed set a
//! *name* is drawn from and whose datatype is `string`.

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize};

use crate::invalid;
use crate::value::DataTypeValue;
use crate::{DataType, DataTypeId, Result};
use smol_str::format_smolstr;

/// Shared dictionary key and value types.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize)]
pub struct DictionaryType {
    pub(crate) key: DataType,
    pub(crate) value: DataType,
}

impl DictionaryType {
    /// Returns the integer key type without allocating.
    pub const fn key(&self) -> &DataType {
        &self.key
    }

    /// Returns the encoded value type without allocating.
    pub const fn value(&self) -> &DataType {
        &self.value
    }
}

impl<'de> Deserialize<'de> for DictionaryType {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Repr {
            key: DataType,
            value: DataType,
        }
        let repr = Repr::deserialize(deserializer)?;
        validate_dictionary_key(&repr.key).map_err(serde::de::Error::custom)?;
        Ok(Self {
            key: repr.key,
            value: repr.value,
        })
    }
}

/// The enum family's datatype payload.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
#[non_exhaustive]
pub enum EnumType {
    /// A key column over a value column.
    ///
    /// Behind a shared pointer, because the two datatypes are wider than this
    /// enum and a datatype clone must not walk them.
    Dictionary(Arc<DictionaryType>),
}

impl EnumType {
    /// Returns the code datatype the rows carry.
    pub fn key(&self) -> &DataType {
        match self {
            Self::Dictionary(dictionary) => &dictionary.key,
        }
    }

    /// Returns the datatype the codes stand for.
    pub fn value(&self) -> &DataType {
        match self {
            Self::Dictionary(dictionary) => &dictionary.value,
        }
    }

    /// Returns the dictionary parameters when this is that leaf.
    pub fn as_dictionary(&self) -> Option<&DictionaryType> {
        match self {
            Self::Dictionary(dictionary) => Some(dictionary),
        }
    }

    /// Returns whether both payloads are the same leaf over one allocation.
    pub fn shares_storage_with(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Dictionary(left), Self::Dictionary(right)) => Arc::ptr_eq(left, right),
        }
    }
}

impl DataTypeValue for EnumType {
    const FAMILY: &'static str = "enum";

    type Sidecar = crate::DictionaryOptions;

    fn id(&self) -> DataTypeId {
        match self {
            Self::Dictionary(_) => DataTypeId::Dictionary,
        }
    }

    fn validate(&self) -> Result<()> {
        validate_dictionary_key(self.key())
    }

    fn into_dtype(self) -> DataType {
        match self {
            Self::Dictionary(dictionary) => DataType::Dictionary(dictionary),
        }
    }

    fn from_dtype(dtype: &DataType) -> Option<Self> {
        match dtype {
            DataType::Dictionary(dictionary) => Some(Self::Dictionary(Arc::clone(dictionary))),
            _ => None,
        }
    }
}

impl fmt::Display for EnumType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.id().as_str())
    }
}

impl From<EnumType> for DataType {
    fn from(value: EnumType) -> Self {
        DataTypeValue::into_dtype(value)
    }
}

impl DataType {
    /// The typed field's payload over a dictionary datatype, `None` for
    /// every other: a shared-pointer clone.
    #[must_use]
    pub fn enum_type(&self) -> Option<EnumType> {
        match self {
            Self::Dictionary(dictionary) => Some(EnumType::Dictionary(Arc::clone(dictionary))),
            _ => None,
        }
    }

    /// Creates a dictionary and validates its integer key type.
    pub fn dictionary(key: Self, value: Self) -> Result<Self> {
        validate_dictionary_key(&key)?;
        Ok(Self::Dictionary(Arc::new(DictionaryType { key, value })))
    }
}

pub(crate) fn validate_dictionary_key(key: &DataType) -> Result<()> {
    if is_valid_dictionary_key(key) {
        Ok(())
    } else {
        Err(invalid(
            "Dictionary",
            format_smolstr!(
                "expected an integer key datatype (int8, int16, int32, int64, uint8, uint16, uint32, or uint64), got {key}"
            ),
        ))
    }
}

fn is_valid_dictionary_key(key: &DataType) -> bool {
    key.is_integer()
}

// ------------------------------------------------------------------------
// The enum leaves: a closed set of members, each stored as its `int32` code.
// ------------------------------------------------------------------------

/// Declares one leaf of the Enum family from its member table: the variant,
/// the code, the stored name and what the member means, each once.
///
/// What every leaf answers is written here and nowhere else - `ALL`, `code`,
/// `as_str`, `description`, `from_code`, `from_name`, `read`, `read_code`,
/// `Display` and `Serialize` as the name, `Deserialize` from a code or a
/// spelling, [`Value`](crate::Value), [`EnumValue`](crate::EnumValue), and
/// the conversions to a [`Scalar`](crate::Scalar) and an `i32`. The leaf
/// states its `repr`, its `kind` (the refusal word and the datatype's name)
/// and its extension name, and writes `from_spelling` itself, because the
/// spellings a leaf accepts are its own.
macro_rules! enum_leaf {
    (
        $(#[$outer:meta])*
        $vis:vis enum $name:ident: $repr:ty, kind = $kind:literal, extension = $extension:path {
            $($(#[$meta:meta])* $variant:ident = $code:literal as $spelling:literal: $doc:literal,)+
        }
    ) => {
        $(#[$outer])*
        #[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
        #[repr($repr)]
        $vis enum $name {
            $(#[doc = $doc] $(#[$meta])* $variant = $code,)+
        }

        impl $name {
            /// Every member, in code order.
            pub const ALL: &'static [Self] = &[$(Self::$variant,)+];

            /// The `int32` a column stores for this member.
            #[must_use]
            pub const fn code(self) -> i32 {
                self as i32
            }

            /// The stored name.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $spelling,)+
                }
            }

            /// What this member means, in a sentence.
            #[must_use]
            pub const fn description(self) -> &'static str {
                match self {
                    $(Self::$variant => $doc,)+
                }
            }

            /// The member one stored code names, or `None` where none does.
            #[must_use]
            pub const fn from_code(code: i32) -> Option<Self> {
                match code {
                    $($code => Some(Self::$variant),)+
                    _ => None,
                }
            }

            /// The member one stored name names, exactly as [`Self::as_str`]
            /// spells it, or `None` where none does.
            #[must_use]
            pub fn from_name(name: &str) -> Option<Self> {
                match name {
                    $($spelling => Some(Self::$variant),)+
                    _ => None,
                }
            }

            /// The member one spelling names, refused where none does.
            ///
            /// [`Self::from_spelling`] as the value contract reads it: a column
            /// of this leaf holds members only, so text that names none leaves
            /// the column null rather than storing a value nothing can read.
            ///
            /// # Errors
            ///
            /// Returns an error naming the spelling.
            pub fn read(spelling: &str) -> $crate::Result<Self> {
                Self::from_spelling(spelling).ok_or_else(|| $crate::Error::InvalidDataType {
                    kind: $kind,
                    reason: ::smol_str::format_smolstr!(
                        concat!("expected a ", $kind, " code, name or stored name, got {:?}"),
                        spelling
                    ),
                })
            }

            /// The member one stored code names, refused where none does.
            ///
            /// # Errors
            ///
            /// Returns an error naming the code.
            pub fn read_code(code: i64) -> $crate::Result<Self> {
                i32::try_from(code)
                    .ok()
                    .and_then(Self::from_code)
                    .ok_or_else(|| $crate::Error::InvalidDataType {
                        kind: $kind,
                        reason: ::smol_str::format_smolstr!(
                            concat!("expected the code of a ", $kind, ", got {}"),
                            code
                        ),
                    })
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl ::serde::Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> ::std::result::Result<S::Ok, S::Error>
            where
                S: ::serde::Serializer,
            {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> ::std::result::Result<Self, D::Error>
            where
                D: ::serde::Deserializer<'de>,
            {
                // A stored code or any spelling the leaf reads; owned where the
                // deserializer cannot lend, as a parsed JSON value cannot.
                #[derive(::serde::Deserialize)]
                #[serde(untagged)]
                enum Held {
                    Code(i64),
                    Spelling(String),
                }
                match Held::deserialize(deserializer)? {
                    Held::Code(code) => Self::read_code(code),
                    Held::Spelling(spelling) => Self::read(&spelling),
                }
                .map_err(<D::Error as ::serde::de::Error>::custom)
            }
        }

        impl $crate::Value for $name {
            fn dtype(&self) -> $crate::Result<$crate::DataType> {
                Ok($crate::DataType::$name)
            }

            fn into_scalar(self) -> $crate::Scalar {
                $crate::Scalar::$name(self)
            }

            fn from_scalar(value: &$crate::Scalar) -> Option<&Self> {
                match value {
                    $crate::Scalar::$name(value) => Some(value),
                    _ => None,
                }
            }
        }

        impl $crate::EnumValue for $name {
            const ALL: &'static [Self] = <$name>::ALL;
            const KIND: &'static str = $kind;
            const EXTENSION_NAME: &'static str = $extension;

            fn code(self) -> i32 {
                <$name>::code(self)
            }

            fn as_str(self) -> &'static str {
                <$name>::as_str(self)
            }

            fn description(self) -> &'static str {
                <$name>::description(self)
            }

            fn from_code(code: i32) -> Option<Self> {
                <$name>::from_code(code)
            }

            fn read(spelling: &str) -> $crate::Result<Self> {
                <$name>::read(spelling)
            }

            fn read_code(code: i64) -> $crate::Result<Self> {
                <$name>::read_code(code)
            }
        }

        impl From<$name> for $crate::Scalar {
            fn from(value: $name) -> Self {
                Self::$name(value)
            }
        }

        impl From<$name> for i32 {
            fn from(value: $name) -> Self {
                value.code()
            }
        }

        impl TryFrom<i32> for $name {
            type Error = $crate::Error;

            fn try_from(code: i32) -> $crate::Result<Self> {
                Self::read_code(i64::from(code))
            }
        }
    };
}

pub(crate) use enum_leaf;

impl DataType {
    /// Whether this is one of the enum leaves: a closed set of members stored
    /// as the `int32` code of each.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert!(DataType::State.is_enum());
    /// assert!(!DataType::Int32.is_enum());
    /// ```
    #[must_use]
    pub const fn is_enum(&self) -> bool {
        crate::DataTypeKind::Enum.contains(self.id())
    }
}

/// The member of the enum leaf `id` one stored code names, as the scalar it
/// is: the one door an integer takes into any enum leaf.
///
/// # Errors
///
/// The leaf's own refusal naming the code, or one naming `id` where it is not
/// an enum leaf.
pub(crate) fn read_enum_code(id: DataTypeId, code: i64) -> Result<crate::Scalar> {
    match id {
        DataTypeId::State => crate::State::read_code(code).map(crate::Value::into_scalar),
        DataTypeId::MarketDataKind => {
            crate::MarketDataKind::read_code(code).map(crate::Value::into_scalar)
        }
        DataTypeId::Side => crate::Side::read_code(code).map(crate::Value::into_scalar),
        _ => Err(enum_refusal(id)),
    }
}

/// The member of the enum leaf `id` one spelling names, as the scalar it is:
/// the one door text takes into any enum leaf, each leaf reading the
/// spellings it accepts.
///
/// # Errors
///
/// The leaf's own refusal naming the spelling, or one naming `id` where it
/// is not an enum leaf.
pub(crate) fn read_enum_spelling(id: DataTypeId, spelling: &str) -> Result<crate::Scalar> {
    match id {
        DataTypeId::State => crate::State::read(spelling).map(crate::Value::into_scalar),
        DataTypeId::MarketDataKind => {
            crate::MarketDataKind::read(spelling).map(crate::Value::into_scalar)
        }
        DataTypeId::Side => crate::Side::read(spelling).map(crate::Value::into_scalar),
        _ => Err(enum_refusal(id)),
    }
}

/// The Arrow extension name one enum leaf's `Int32` codes ride under.
pub(crate) const fn enum_extension_name(id: DataTypeId) -> Option<&'static str> {
    match id {
        DataTypeId::State => Some(crate::STATE_EXTENSION_NAME),
        DataTypeId::MarketDataKind => Some(crate::MARKETDATAKIND_EXTENSION_NAME),
        DataTypeId::Side => Some(crate::SIDE_EXTENSION_NAME),
        _ => None,
    }
}

/// The enum leaf one Arrow extension name imports as.
///
/// The name alone, because every enum leaf's storage is Arrow's `Int32` and
/// the caller has already checked it: a `yggdryl.state` over anything else
/// stays the storage it is rather than silently becoming a state.
pub(crate) fn enum_for_extension(name: &str) -> Option<DataType> {
    match name {
        crate::STATE_EXTENSION_NAME => Some(DataType::State),
        crate::MARKETDATAKIND_EXTENSION_NAME => Some(DataType::MarketDataKind),
        crate::SIDE_EXTENSION_NAME => Some(DataType::Side),
        _ => None,
    }
}

/// The refusal a datatype that is not an enum leaf answers with.
pub(crate) fn enum_refusal(id: DataTypeId) -> crate::Error {
    crate::Error::InvalidDataType {
        kind: "enum",
        reason: crate::text::expected_got(
            format_args!("one of the enum leaves"),
            format_args!("{id}"),
        ),
    }
}

/// Arrow casts every enum leaf shares.
pub(crate) mod casts {
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, Int32Array, Int64Array, StringArray};
    use arrow_buffer::{BooleanBuffer, BooleanBufferBuilder, NullBuffer};
    use arrow_schema::DataType as ArrowDataType;

    use crate::arrow::Result;
    use crate::budget::MaterializationBudget;
    use crate::cast::columns::is_exposed;
    use crate::cast::{arrow_cast_exposed, downcast, named_cell};
    use crate::{DataType, EnumValue, Field};

    /// Validates every exposed, non-null value entering an enum leaf and
    /// stores the code of its member.
    ///
    /// An integer source is read as the codes it holds; anything else renders
    /// as Utf8 through Arrow's kernel and is read as the spelling it is. A
    /// failing cell is null under `safe` and an error naming the row
    /// otherwise; the target's own null policy runs after the reading.
    pub(crate) fn ingest_enum_array<E: EnumValue>(
        array: &ArrayRef,
        safe: bool,
        field: &Field,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
    ) -> Result<ArrayRef> {
        if array.data_type().is_integer() {
            let codes = arrow_cast_exposed(
                array,
                &ArrowDataType::Int64,
                safe,
                exposure,
                &Field::new(field.name(), DataType::Int64, true),
                budget,
            )?;
            let codes = downcast::<Int64Array>(codes.as_ref())?;
            return enum_storage::<E>(field, codes.len(), safe, exposure, budget, |index| {
                codes
                    .is_valid(index)
                    .then(|| E::read_code(codes.value(index)))
            });
        }
        let text = if array.data_type() == &ArrowDataType::Utf8 {
            Arc::clone(array)
        } else {
            arrow_cast_exposed(
                array,
                &ArrowDataType::Utf8,
                safe,
                exposure,
                &Field::new(field.name(), DataType::utf8(), true),
                budget,
            )?
        };
        let cells = downcast::<StringArray>(text.as_ref())?;
        enum_storage::<E>(field, cells.len(), safe, exposure, budget, |index| {
            cells.is_valid(index).then(|| E::read(cells.value(index)))
        })
    }

    /// Builds the `int32` codes of an enum column from one reading per row.
    fn enum_storage<E: EnumValue>(
        field: &Field,
        rows: usize,
        safe: bool,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
        cell: impl Fn(usize) -> Option<crate::Result<E>>,
    ) -> Result<ArrayRef> {
        budget.add_array(field.dtype(), rows)?;
        let mut codes = vec![0_i32; rows];
        let mut validity = BooleanBufferBuilder::new(rows);
        for (index, code) in codes.iter_mut().enumerate() {
            let member = match is_exposed(exposure, index).then(|| cell(index)).flatten() {
                Some(Ok(member)) => Some(member),
                Some(Err(_)) if safe => None,
                Some(read @ Err(_)) => Some(named_cell(field, index, read)?),
                None => None,
            };
            if let Some(member) = member {
                *code = member.code();
            }
            validity.append(member.is_some());
        }
        Ok(Arc::new(Int32Array::new(
            codes.into(),
            Some(NullBuffer::new(validity.finish())),
        )))
    }
}

pub(crate) use casts::ingest_enum_array;

// ------------------------------------------------------------------------
// Arrow projection: a key width over the values it stands for.
// ------------------------------------------------------------------------

mod arrow {
    use std::sync::Arc;

    use arrow_schema::DataType as ArrowDataType;
    use arrow_schema::ffi::Flags;

    use super::{EnumType, validate_dictionary_key};
    use crate::value::ArrowFfiParts;
    use crate::{DataType, Result};

    impl EnumType {
        /// The Arrow storage this encoding lays out.
        ///
        /// # Errors
        ///
        /// Returns an error when the key is not an integer a dictionary may
        /// use, or either half has no Arrow projection.
        pub(crate) fn arrow_storage(&self) -> Result<ArrowDataType> {
            let Self::Dictionary(dictionary) = self;
            validate_dictionary_key(&dictionary.key)?;
            Ok(ArrowDataType::Dictionary(
                Box::new(dictionary.key.arrow_datatype()?),
                Box::new(dictionary.value.arrow_datatype()?),
            ))
        }

        /// The same projection, consuming a uniquely held encoding.
        ///
        /// # Errors
        ///
        /// [`Self::arrow_storage`] carries the rule.
        pub(crate) fn into_arrow_storage(self) -> Result<ArrowDataType> {
            let Self::Dictionary(dictionary) = self;
            match Arc::try_unwrap(dictionary) {
                Ok(dictionary) => {
                    validate_dictionary_key(&dictionary.key)?;
                    Ok(ArrowDataType::Dictionary(
                        Box::new(dictionary.key.into_arrow_datatype()?),
                        Box::new(dictionary.value.into_arrow_datatype()?),
                    ))
                }
                Err(dictionary) => {
                    validate_dictionary_key(&dictionary.key)?;
                    Ok(ArrowDataType::Dictionary(
                        Box::new(dictionary.key.clone().into_arrow_datatype()?),
                        Box::new(dictionary.value.clone().into_arrow_datatype()?),
                    ))
                }
            }
        }

        /// The C Data Interface node this encoding writes.
        ///
        /// An encoded extension declares its identity once, on the node the
        /// field is: Arrow's dictionary values are a bare datatype, so an
        /// importer reads the outer entries and never the ones a values
        /// projection would carry. The outer node writes them instead.
        ///
        /// # Errors
        ///
        /// [`Self::arrow_storage`] carries the rule.
        pub(crate) fn arrow_ffi_parts(&self) -> Result<ArrowFfiParts> {
            let Self::Dictionary(dictionary) = self;
            validate_dictionary_key(&dictionary.key)?;
            let key = dictionary.key.clone().into_arrow_datatype_ffi()?;
            let mut values = dictionary.value.clone().into_arrow_datatype_ffi()?;
            if dictionary.value.arrow_extension().is_some() {
                values = values.with_metadata::<[(&str, &str); 0], _>([])?;
            }
            Ok(ArrowFfiParts {
                format: key.format().to_owned(),
                children: Vec::new(),
                dictionary: Some(values),
                flags: Flags::empty(),
            })
        }

        /// The enum datatype one Arrow dictionary storage imports as.
        ///
        /// # Errors
        ///
        /// Returns an error when either half cannot be imported, or the key is
        /// not an integer a dictionary may use.
        pub(crate) fn from_arrow_storage_at_depth(
            key: &ArrowDataType,
            values: &ArrowDataType,
            depth: usize,
        ) -> Result<DataType> {
            DataType::dictionary(
                DataType::from_arrow_datatype_at_depth(key, depth)?,
                DataType::from_arrow_datatype_at_depth(values, depth)?,
            )
        }

        /// The same import, consuming Arrow's boxed halves.
        ///
        /// # Errors
        ///
        /// [`Self::from_arrow_storage_at_depth`] carries the rule.
        pub(crate) fn from_arrow_storage_owned_at_depth(
            key: ArrowDataType,
            values: ArrowDataType,
            depth: usize,
        ) -> Result<DataType> {
            DataType::dictionary(
                DataType::from_arrow_datatype_owned_at_depth(key, depth)?,
                DataType::from_arrow_datatype_owned_at_depth(values, depth)?,
            )
        }
    }
}
