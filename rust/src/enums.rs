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
// The enum leaves: a closed set of members, each stored as its code at the
// leaf's own width - `uint8`, or `uint16` where the codes pass 255.
// ------------------------------------------------------------------------

/// Declares one leaf of the Enum family from its member table: the variant,
/// the code, the stored name and what the member means, each once.
///
/// What every leaf answers is written here and nowhere else - `ALL`, `code`,
/// `as_str`, `description`, `from_code`, `from_name`, `read`, `read_code`,
/// `Display` and `Serialize` as the name, `Deserialize` from a code or a
/// spelling, [`Value`](crate::Value), [`EnumValue`](crate::EnumValue), and
/// the conversions to a [`Scalar`](crate::Scalar) and to its code. The leaf
/// states its `repr` - `u8`, or `u16` where its codes pass 255, which is
/// also the width a column stores ([`EnumRepr`]) - its `kind` (the refusal
/// word and the datatype's name)
/// and its extension name, and writes `from_spelling` itself, because the
/// spellings a leaf accepts are its own. A registered leaf states its
/// `market` static and the numbers beside it too, and gains its `ID`,
/// `NAME` and `EXTENSION_NAME`, the [`MarketDescriptor`](crate::MarketDescriptor)
/// it is claimed under - its members and its `read` - and
/// [`MarketValue`](crate::MarketValue), and its own `dtype()` and
/// `field(name)` - the datatype every column of the kind declares and a
/// nullable field of it; its scalar is
/// [`Scalar::Market`](crate::Scalar::Market), never a variant of its own.
///
/// Exported for the crates this core is split into, which declare their
/// kinds with it: every path it expands to is public, the core's own
/// crate-private doors reached through [`crate::implementer`].
#[macro_export]
#[doc(hidden)]
macro_rules! enum_leaf {
    // A leaf of the core's own: the datatype has a variant of its own.
    (
        $(#[$outer:meta])*
        $vis:vis enum $name:ident: $repr:ty, kind = $kind:literal, extension = $extension:path, aliases = $aliases:path {
            $($(#[$meta:meta])* $variant:ident = $code:literal as $spelling:literal: $doc:literal,)+
        }
    ) => {
        $crate::implementer::enum_leaf!(@members
            $(#[$outer])*
            $vis enum $name: $repr, kind = $kind, extension = $extension, aliases = $aliases {
                $($(#[$meta])* $variant = $code as $spelling: $doc,)+
            }
        );

        impl $name {
            /// The canonical name: the datatype's spelling, the serde tag
            /// and the parser's word.
            pub const NAME: &'static str = $kind;
            /// The Arrow extension name a column of this enum rides under.
            pub const EXTENSION_NAME: &'static str = $extension;
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
            type Repr = $repr;

            fn code(self) -> $repr {
                <$name>::code(self)
            }

            fn as_str(self) -> &'static str {
                <$name>::as_str(self)
            }

            fn description(self) -> &'static str {
                <$name>::description(self)
            }

            fn from_code(code: $repr) -> Option<Self> {
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

        impl From<$name> for $repr {
            fn from(value: $name) -> Self {
                value.code()
            }
        }

        impl TryFrom<$repr> for $name {
            type Error = $crate::Error;

            fn try_from(code: $repr) -> $crate::Result<Self> {
                Self::read_code(i64::from(code))
            }
        }
    };
    // A registered kind: the leaf states its byte and the three numbers
    // its values and its datatype order and hash by, and every door
    // reaches it through the register.
    (
        $(#[$outer:meta])*
        $vis:vis enum $name:ident: $repr:ty, kind = $kind:literal, extension = $extension:path, aliases = $aliases:path,
        market = $kind_static:ident [$byte:literal, $value_rank:literal, $dtype_rank:literal, $shape:literal] {
            $($(#[$meta:meta])* $variant:ident = $code:literal as $spelling:literal: $doc:literal,)+
        }
    ) => {
        $crate::implementer::enum_leaf!(@members
            $(#[$outer])*
            $vis enum $name: $repr, kind = $kind, extension = $extension, aliases = $aliases {
                $($(#[$meta])* $variant = $code as $spelling: $doc,)+
            }
        );

        impl $name {
            /// The identifier this enum's datatype carries.
            pub const ID: $crate::DataTypeId = $crate::DataTypeId::market($byte);
            /// The canonical name: the datatype's spelling, the serde tag
            /// and the parser's word.
            pub const NAME: &'static str = $kind;
            /// The Arrow extension name a column of this enum rides under.
            pub const EXTENSION_NAME: &'static str = $extension;

            /// The datatype of this kind: `DataType::Market` over its
            /// descriptor, what every column of it declares.
            #[must_use]
            pub const fn dtype() -> $crate::DataType {
                $kind_static.dtype()
            }

            /// A nullable field of this kind, named `name`.
            #[must_use]
            pub fn field(name: impl Into<::smol_str::SmolStr>) -> $crate::Field {
                $kind_static.field(name, true)
            }
        }

        #[doc = concat!("The registered kind `", $kind, "`: what [`", stringify!($name), "`] states about itself, once.")]
        pub static $kind_static: $crate::MarketDescriptor = $crate::MarketDescriptor {
            id: <$name>::ID,
            name: <$name>::NAME,
            extension_name: <$name>::EXTENSION_NAME,
            storage: <$repr as $crate::EnumRepr>::MARKET_STORAGE,
            members: &[$($crate::MarketMember { code: $code as u16, name: $spelling, description: $doc },)+],
            value_rank: $value_rank,
            dtype_rank: $dtype_rank,
            shape: $shape,
            read: |text| <$name>::read(text).map(|member| member.code() as u16),
        };

        // The members are listed in code order, which the register's
        // lookup by code relies on.
        const _: () = {
            let members = $kind_static.members;
            let mut index = 1;
            while index < members.len() {
                assert!(members[index - 1].code < members[index].code, "enum members are listed in code order");
                index += 1;
            }
        };

        impl $crate::Value for $name {
            fn dtype(&self) -> $crate::Result<$crate::DataType> {
                Ok($kind_static.dtype())
            }

            fn into_scalar(self) -> $crate::Scalar {
                $crate::implementer::adopt_market_code(&$kind_static, self.code() as u16)
            }

            fn from_scalar(value: &$crate::Scalar) -> Option<&Self> {
                match value {
                    // `ALL` is in code order, so the member's own place is
                    // one search, never a scan.
                    $crate::Scalar::Market(held) if held.id() == Self::ID => Self::ALL
                        .binary_search_by_key(&held.code(), |member| u16::from(member.code()))
                        .ok()
                        .map(|index| &Self::ALL[index]),
                    _ => None,
                }
            }
        }

        impl $crate::MarketValue for $name {
            const KIND: &'static $crate::MarketDescriptor = &$kind_static;

            fn into_scalar(self) -> $crate::Scalar {
                $crate::implementer::adopt_market_code(&$kind_static, self.code() as u16)
            }

            fn from_scalar(value: &$crate::Scalar) -> Option<Self> {
                <Self as $crate::Value>::from_scalar(value).copied()
            }
        }

        impl $crate::EnumValue for $name {
            const ALL: &'static [Self] = <$name>::ALL;
            type Repr = $repr;

            fn code(self) -> $repr {
                <$name>::code(self)
            }

            fn as_str(self) -> &'static str {
                <$name>::as_str(self)
            }

            fn description(self) -> &'static str {
                <$name>::description(self)
            }

            fn from_code(code: $repr) -> Option<Self> {
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
                $crate::MarketValue::into_scalar(value)
            }
        }

        impl From<$name> for $repr {
            fn from(value: $name) -> Self {
                value.code()
            }
        }

        impl TryFrom<$repr> for $name {
            type Error = $crate::Error;

            fn try_from(code: $repr) -> $crate::Result<Self> {
                Self::read_code(i64::from(code))
            }
        }
    };
    // The member table and what every leaf answers over it, written once
    // for both kinds of leaf.
    (@members
        $(#[$outer:meta])*
        $vis:vis enum $name:ident: $repr:ty, kind = $kind:literal, extension = $extension:path, aliases = $aliases:path {
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

            /// The code a column stores for this member, at the leaf's width.
            #[must_use]
            pub const fn code(self) -> $repr {
                self as $repr
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
            pub const fn from_code(code: $repr) -> Option<Self> {
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

            /// The member the words of `spelling` name, once no exact
            /// vocabulary names it: `Part-Filled`, `partial fill`,
            /// `ORDER FILLED` and `partfilled` read as the member whose own
            /// names make the same words, each read as the word it
            /// abbreviates or inflects and the words that say nothing
            /// dropped. A set of words two members make names neither, and
            /// a word no name uses names nothing. A spelling read is kept,
            /// so a column repeating it reads it once.
            fn from_pattern(spelling: &str) -> Option<Self> {
                static PATTERNS: ::std::sync::LazyLock<$crate::implementer::Patterns<$name>> =
                    ::std::sync::LazyLock::new(|| {
                        $crate::implementer::Patterns::new(
                            $name::ALL.iter().map(|member| (member.as_str(), *member)),
                            $aliases(),
                        )
                    });
                PATTERNS.read(spelling)
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
                <$repr>::try_from(code)
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

    };
}

/// The width an enum leaf's codes are held and stored at: `u8` for a leaf
/// whose codes fit a byte, `u16` for one whose codes pass 255. The one owner
/// of the Arrow storage an enum column has, so a writer, a reader and a cast
/// read the width off the leaf rather than assuming one.
pub trait EnumRepr:
    arrow_buffer::ArrowNativeType + Copy + Default + Into<u16> + Into<i64> + TryFrom<i64> + 'static
{
    /// The Arrow primitive type a column of this width stores its codes as.
    type Primitive: arrow_array::ArrowPrimitiveType<Native = Self>;
    /// The Arrow type a column of this width stores its codes as.
    const ARROW: arrow_schema::DataType;
    /// The bytes one stored code takes.
    const BYTES: usize;
    /// The storage a registered enum of this width declares.
    const MARKET_STORAGE: crate::MarketStorage;
}

impl EnumRepr for u8 {
    type Primitive = arrow_array::types::UInt8Type;
    const ARROW: arrow_schema::DataType = arrow_schema::DataType::UInt8;
    const BYTES: usize = 1;
    const MARKET_STORAGE: crate::MarketStorage = crate::MarketStorage::Code8;
}

impl EnumRepr for u16 {
    type Primitive = arrow_array::types::UInt16Type;
    const ARROW: arrow_schema::DataType = arrow_schema::DataType::UInt16;
    const BYTES: usize = 2;
    const MARKET_STORAGE: crate::MarketStorage = crate::MarketStorage::Code16;
}

impl DataType {
    /// Whether this is one of the enum leaves: a closed set of members stored
    /// as the code of each, at the leaf's width.
    ///
    /// ```
    /// use yggdryl::{DataType, Side};
    ///
    /// assert!(DataType::State.is_enum());
    /// assert!(Side::dtype().is_enum());
    /// assert!(!DataType::Int32.is_enum());
    /// ```
    #[must_use]
    pub const fn is_enum(&self) -> bool {
        crate::DataTypeKind::Enum.contains(self.id())
    }
}

/// The member of the enum leaf `dtype` one stored code names, as the scalar
/// it is: the one door an integer takes into any enum leaf once its
/// datatype is in hand.
///
/// A registered kind rides in the datatype, so a code reads no register:
/// the datatype is a validated field's or the register's own answer, and
/// the kind's member table alone is asked. A caller's value arriving
/// through a datatype door goes through [`crate::MarketDescriptor::member`],
/// which holds the kind's claim.
///
/// # Errors
///
/// The leaf's own refusal naming the code, or one naming `dtype` where it is
/// not an enum leaf.
pub(crate) fn read_enum_code(dtype: &DataType, code: i64) -> Result<crate::Scalar> {
    match dtype {
        DataType::State => crate::State::read_code(code).map(crate::Value::into_scalar),
        DataType::Market(kind) => kind.kind().adopt_member(code),
        other => Err(enum_refusal(other.id())),
    }
}

/// The member of the enum leaf `dtype` one spelling names, as the scalar it
/// is: the one door text takes into any enum leaf, each leaf reading the
/// spellings it accepts - a registered kind through its own reader, held to
/// its member table.
///
/// # Errors
///
/// The leaf's own refusal naming the spelling, or one naming `dtype` where
/// it is not an enum leaf.
pub(crate) fn read_enum_spelling(dtype: &DataType, spelling: &str) -> Result<crate::Scalar> {
    match dtype {
        DataType::State => crate::State::read(spelling).map(crate::Value::into_scalar),
        DataType::Market(kind) => kind.kind().scalar(spelling),
        other => Err(enum_refusal(other.id())),
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

    use arrow_array::{Array, ArrayRef, Int64Array, PrimitiveArray, StringArray};
    use arrow_buffer::{BooleanBuffer, BooleanBufferBuilder, NullBuffer};
    use arrow_schema::DataType as ArrowDataType;
    use smol_str::format_smolstr;

    use crate::arrow::Result;
    use crate::budget::MaterializationBudget;
    use crate::cast::columns::is_exposed;
    use crate::cast::{arrow_cast_exposed, downcast, named_cell};
    use crate::{DataType, EnumRepr, EnumValue, Field, MarketDescriptor, MarketStorage};

    /// Validates every exposed, non-null value entering the core's enum leaf
    /// `E` and stores the code of its member.
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
        ingest_codes::<E::Repr>(
            array,
            safe,
            field,
            exposure,
            budget,
            |code| E::read_code(code).map(E::code),
            |spelling| E::read(spelling).map(E::code),
        )
    }

    /// Validates every exposed, non-null value entering the registered enum
    /// `kind` and stores the code of its member, at the width its storage
    /// states.
    ///
    /// The caller resolves `kind` once per array off the target field, and
    /// the storage picks the width once here; every row then reads the
    /// descriptor alone - an integer as the member code it holds
    /// ([`MarketDescriptor::adopt_member`]), any other cell rendered as Utf8
    /// and read through the kind's spelling door
    /// ([`MarketDescriptor::scalar`]), which holds what the kind's own
    /// reader answers to its member table - so every code stored is a
    /// member's, which is what the plan certifies. The failing-cell and null
    /// policies are [`ingest_enum_array`]'s.
    pub(crate) fn ingest_market_enum_array(
        array: &ArrayRef,
        kind: &'static MarketDescriptor,
        safe: bool,
        field: &Field,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
    ) -> Result<ArrayRef> {
        match kind.storage {
            MarketStorage::Code8 => ingest_codes::<u8>(
                array,
                safe,
                field,
                exposure,
                budget,
                |code| member_code(kind, code),
                |spelling| spelled_code(kind, spelling),
            ),
            MarketStorage::Code16 => ingest_codes::<u16>(
                array,
                safe,
                field,
                exposure,
                budget,
                |code| member_code(kind, code),
                |spelling| spelled_code(kind, spelling),
            ),
        }
    }

    /// The code `kind` stores for the member `code` names, at the width
    /// `R`, refused naming the code where no member holds it.
    fn member_code<R: EnumRepr>(kind: &'static MarketDescriptor, code: i64) -> crate::Result<R> {
        stored_code(kind, &kind.adopt_member(code)?)
    }

    /// The code `kind` stores for the member `spelling` names, read by the
    /// kind's own reader and held to its member table, at the width `R`;
    /// the field the cast lands under proved the kind claimed.
    fn spelled_code<R: EnumRepr>(
        kind: &'static MarketDescriptor,
        spelling: &str,
    ) -> crate::Result<R> {
        stored_code(kind, &kind.adopt_spelling(spelling)?)
    }

    /// The code one member of `kind` stores, at the width `R` its storage
    /// states - which the kind's claim proved every member's code fits.
    fn stored_code<R: EnumRepr>(
        kind: &'static MarketDescriptor,
        member: &crate::Scalar,
    ) -> crate::Result<R> {
        member
            .enum_code()
            .and_then(|code| R::try_from(i64::from(code)).ok())
            .ok_or_else(|| crate::Error::InvalidDataType {
                kind: kind.name,
                reason: format_smolstr!(
                    "expected a member of {} stored as {:?}, got {member:?}",
                    kind.name,
                    kind.storage
                ),
            })
    }

    /// Reads every exposed cell of `array` as a member's code at the width
    /// `R`: an integer source through `read_code`, anything else rendered as
    /// Utf8 through Arrow's kernel and read through `read`.
    fn ingest_codes<R: EnumRepr>(
        array: &ArrayRef,
        safe: bool,
        field: &Field,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
        read_code: impl Fn(i64) -> crate::Result<R>,
        read: impl Fn(&str) -> crate::Result<R>,
    ) -> Result<ArrayRef> {
        if array.data_type().is_integer() {
            // Every width, signed or unsigned, is read as the code it holds.
            // A value past `i64` - a `u64` above its range - is no code any
            // member takes, so it lands as the leaf's own refusal rather
            // than the kernel's overflow.
            let codes = arrow_cast_exposed(
                array,
                &ArrowDataType::Int64,
                true,
                exposure,
                &Field::new(field.name(), DataType::Int64, true),
                budget,
            )?;
            let codes = downcast::<Int64Array>(codes.as_ref())?;
            return code_storage::<R>(field, codes.len(), safe, exposure, budget, |index| {
                if codes.is_valid(index) {
                    Some(read_code(codes.value(index)))
                } else {
                    array.is_valid(index).then(|| read_code(i64::MAX))
                }
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
        code_storage::<R>(field, cells.len(), safe, exposure, budget, |index| {
            cells.is_valid(index).then(|| read(cells.value(index)))
        })
    }

    /// Builds the codes of an enum column, at the width `R`, from one
    /// reading per row.
    fn code_storage<R: EnumRepr>(
        field: &Field,
        rows: usize,
        safe: bool,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
        cell: impl Fn(usize) -> Option<crate::Result<R>>,
    ) -> Result<ArrayRef> {
        budget.add_array(field.dtype(), rows)?;
        let mut codes = vec![R::default(); rows];
        let mut validity = BooleanBufferBuilder::new(rows);
        for (index, code) in codes.iter_mut().enumerate() {
            let stored = match is_exposed(exposure, index).then(|| cell(index)).flatten() {
                Some(Ok(read)) => Some(read),
                Some(Err(_)) if safe => None,
                Some(read @ Err(_)) => Some(named_cell(field, index, read)?),
                None => None,
            };
            if let Some(stored) = stored {
                *code = stored;
            }
            validity.append(stored.is_some());
        }
        Ok(Arc::new(PrimitiveArray::<R::Primitive>::new(
            codes.into(),
            Some(NullBuffer::new(validity.finish())),
        )))
    }
}

pub(crate) use casts::{ingest_enum_array, ingest_market_enum_array};

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

// ------------------------------------------------------------------------
// Spelling patterns: a member read off the words a spelling is made of.
// ------------------------------------------------------------------------

pub(crate) mod patterns {
    //! What every enum leaf reads a spelling by once its exact vocabularies -
    //! the stored name, a wire code, a standard's name folded - name nothing:
    //! the words the spelling is made of. `Part-Filled`, `partial fill`,
    //! `filled partially`, `ORDER FILLED` and `partfilled` are one set of
    //! words - `{fill, partial}` once each word is read as the one it
    //! abbreviates or inflects and the words that say nothing (`order`,
    //! `status`, `fully`) are dropped - and that set is the one a member's
    //! own names make. One reading or none: a set two members make is
    //! ambiguous and reads as neither, and a word the vocabulary does not
    //! know makes the whole spelling unread rather than half read.
    //!
    //! A spelling read this way is kept, so a column repeating it pays the
    //! words once: at most [`CACHE_CAPACITY`] spellings per leaf, because a
    //! column's distinct spellings are few and text from outside must not
    //! grow the process without bound. A spelling that reads as nothing is
    //! never kept.

    use std::collections::{HashMap, HashSet};
    use std::sync::RwLock;

    use smol_str::SmolStr;

    /// How many spellings one leaf keeps the reading of.
    pub(crate) const CACHE_CAPACITY: usize = 1_024;

    /// Each word a spelling may use and the one word it stands for: an
    /// abbreviation, an inflection or a synonym read as its stem, so two
    /// spellings of one thing make one set of words.
    const WORDS: &[(&str, &str)] = &[
        ("ack", "ack"),
        ("acked", "ack"),
        ("acknowledge", "ack"),
        ("acknowledged", "ack"),
        ("acknowledgement", "ack"),
        ("accept", "accept"),
        ("accepted", "accept"),
        ("acpt", "accept"),
        ("amend", "amend"),
        ("amended", "amend"),
        ("amendment", "amend"),
        ("bought", "buy"),
        ("buy", "buy"),
        ("buys", "buy"),
        ("calc", "calculate"),
        ("calculate", "calculate"),
        ("calculated", "calculate"),
        ("canc", "cancel"),
        ("cancel", "cancel"),
        ("canceled", "cancel"),
        ("cancellation", "cancel"),
        ("cancelled", "cancel"),
        ("cncl", "cancel"),
        ("cxl", "cancel"),
        ("cxld", "cancel"),
        ("complete", "complete"),
        ("completed", "complete"),
        ("corr", "correct"),
        ("correct", "correct"),
        ("corrected", "correct"),
        ("correction", "correct"),
        ("exec", "execute"),
        ("execute", "execute"),
        ("executed", "execute"),
        ("execution", "execute"),
        ("exempt", "exempt"),
        ("exp", "expire"),
        ("expire", "expire"),
        ("expired", "expire"),
        ("expiry", "expire"),
        ("fail", "fail"),
        ("failed", "fail"),
        ("failure", "fail"),
        ("fill", "fill"),
        ("filled", "fill"),
        ("fills", "fill"),
        ("fld", "fill"),
        ("new", "new"),
        ("part", "partial"),
        ("partial", "partial"),
        ("partially", "partial"),
        ("prtl", "partial"),
        ("pend", "pending"),
        ("pending", "pending"),
        ("pnd", "pending"),
        ("rej", "reject"),
        ("reject", "reject"),
        ("rejected", "reject"),
        ("rejection", "reject"),
        ("rjct", "reject"),
        ("repl", "replace"),
        ("replace", "replace"),
        ("replaced", "replace"),
        ("replacement", "replace"),
        ("sell", "sell"),
        ("sells", "sell"),
        ("sold", "sell"),
        ("short", "short"),
        ("stop", "stop"),
        ("stopped", "stop"),
        ("susp", "suspend"),
        ("suspend", "suspend"),
        ("suspended", "suspend"),
        ("trade", "trade"),
        ("traded", "trade"),
        ("trades", "trade"),
        ("trd", "trade"),
        ("trig", "trigger"),
        ("trigger", "trigger"),
        ("triggered", "trigger"),
    ];

    /// The words that say nothing about which member a spelling names: an
    /// order filled is filled, a fully filled order is filled.
    const NOISE: &[&str] = &[
        "a",
        "an",
        "been",
        "by",
        "completely",
        "for",
        "fully",
        "has",
        "is",
        "now",
        "of",
        "ord",
        "order",
        "orders",
        "state",
        "status",
        "the",
        "was",
    ];

    /// The members of one leaf by the set of words each of its names makes,
    /// and the spellings already read.
    pub struct Patterns<E: 'static> {
        /// Each set of words, sorted and joined by a space, and the one
        /// member it names - `None` where two members make it.
        members: HashMap<Box<str>, Option<E>>,
        /// The words a run of letters may be cut into: every word of
        /// [`WORDS`] and [`NOISE`], and every word of a stored name.
        vocabulary: HashSet<Box<str>>,
        /// Spellings read, and the member each read as.
        cache: RwLock<HashMap<SmolStr, E>>,
    }

    impl<E: Copy + Eq> Patterns<E> {
        /// Builds the patterns of one leaf from its members' stored names
        /// and the other names each member goes by. The words of a name
        /// written with its word breaks - `PARTIALLY_FILLED`,
        /// `GoodTillCancel` - are the vocabulary; a name folded into one run
        /// of lower-case letters - `partfill` - is cut into that vocabulary.
        pub fn new(
            stored: impl IntoIterator<Item = (&'static str, E)>,
            aliases: impl IntoIterator<Item = (&'static str, E)>,
        ) -> Self {
            let stored: Vec<(&'static str, E)> = stored.into_iter().chain(aliases).collect();
            let mut vocabulary: HashSet<Box<str>> = WORDS
                .iter()
                .map(|(word, _)| *word)
                .chain(NOISE.iter().copied())
                .filter(|word| word.len() > 1)
                .map(Box::from)
                .collect();
            let broken = |name: &str| {
                name.bytes()
                    .any(|byte| byte.is_ascii_uppercase() || !byte.is_ascii_alphanumeric())
            };
            for (name, _) in stored.iter().filter(|(name, _)| broken(name)) {
                for word in words(name) {
                    if word.len() > 1 {
                        vocabulary.insert(word.into_boxed_str());
                    }
                }
            }
            let mut patterns = Self {
                members: HashMap::new(),
                vocabulary,
                cache: RwLock::new(HashMap::new()),
            };
            for (name, member) in stored {
                let Some(key) = patterns.key(name, true) else {
                    continue;
                };
                patterns
                    .members
                    .entry(key.into_boxed_str())
                    .and_modify(|held| {
                        if *held != Some(member) {
                            *held = None;
                        }
                    })
                    .or_insert(Some(member));
            }
            patterns
        }

        /// The member `spelling`'s words name, or `None` where they name
        /// none or two.
        pub fn read(&self, spelling: &str) -> Option<E> {
            let spelling = spelling.trim();
            if let Some(held) = self
                .cache
                .read()
                .ok()
                .and_then(|cache| cache.get(spelling).copied())
            {
                return Some(held);
            }
            let key = self.key(spelling, false)?;
            let member = (*self.members.get(key.as_str())?)?;
            if let Ok(mut cache) = self.cache.write()
                && cache.len() < CACHE_CAPACITY
            {
                cache.insert(SmolStr::new(spelling), member);
            }
            Some(member)
        }

        /// The set of words `spelling` makes, sorted and joined by a space:
        /// each word read as the one it stands for, the noise dropped. A
        /// run of letters no known word spells is cut into known words; one
        /// that cannot be cut is kept as it is where `trusted` - a member's
        /// own name - and makes the spelling unread otherwise.
        fn key(&self, spelling: &str, trusted: bool) -> Option<String> {
            let mut held: Vec<&str> = Vec::new();
            let spelled = words(spelling);
            for word in &spelled {
                if self.vocabulary.contains(word.as_str()) || stem(word).is_some() {
                    push_word(&mut held, word);
                    continue;
                }
                match self.segment(word) {
                    Some(parts) => {
                        for part in parts {
                            push_word(&mut held, part);
                        }
                    }
                    // A member's own name is its own word where no known
                    // words spell it: `GTHX` is a time in force's name.
                    None if trusted => held.push(word),
                    None => return None,
                }
            }
            held.sort_unstable();
            held.dedup();
            (!held.is_empty()).then(|| held.join(" "))
        }

        /// `run` cut into the fewest words of the vocabulary, or `None`
        /// where no cut spells it.
        fn segment<'run>(&self, run: &'run str) -> Option<Vec<&'run str>> {
            let length = run.len();
            // best[i]: the fewest words spelling run[..i], and where the
            // last of them starts.
            let mut best: Vec<Option<(usize, usize)>> = vec![None; length + 1];
            best[0] = Some((0, 0));
            for end in 1..=length {
                for start in 0..end {
                    let Some((count, _)) = best[start] else {
                        continue;
                    };
                    let Some(word) = run.get(start..end) else {
                        continue;
                    };
                    if word.len() > 1
                        && self.vocabulary.contains(word)
                        && best[end].is_none_or(|(held, _)| count + 1 < held)
                    {
                        best[end] = Some((count + 1, start));
                    }
                }
            }
            best[length]?;
            let mut parts = Vec::new();
            let mut end = length;
            while end > 0 {
                let (_, start) = best[end]?;
                parts.push(run.get(start..end)?);
                end = start;
            }
            parts.reverse();
            Some(parts)
        }
    }

    /// Pushes the word `word` stands for, unless it says nothing.
    fn push_word<'word>(held: &mut Vec<&'word str>, word: &'word str) {
        if NOISE.contains(&word) {
            return;
        }
        held.push(stem(word).unwrap_or(word));
    }

    /// The word `word` stands for in [`WORDS`].
    fn stem(word: &str) -> Option<&'static str> {
        WORDS
            .iter()
            .find(|(spelled, _)| *spelled == word)
            .map(|(_, stem)| *stem)
    }

    /// The words of `spelling`, lower-cased: split at every character that
    /// is not a letter or a digit, and where a lower-case letter meets an
    /// upper-case one - `PartiallyFilled` is two words, `PARTIALLY_FILLED`
    /// two, `partfilled` one run a later cut reads.
    fn words(spelling: &str) -> Vec<String> {
        let mut words = Vec::new();
        let mut current = String::new();
        let mut previous_lower = false;
        for character in spelling.chars() {
            if !character.is_ascii_alphanumeric() {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
                previous_lower = false;
                continue;
            }
            if character.is_ascii_uppercase() && previous_lower && !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            previous_lower = character.is_ascii_lowercase() || character.is_ascii_digit();
            current.push(character.to_ascii_lowercase());
        }
        if !current.is_empty() {
            words.push(current);
        }
        words
    }
}
