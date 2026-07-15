//! [`DataTypeCategory`] — the coarse type-space bucket, derived from a [`DataTypeId`].

/// The **coarse category** of a data type — the broad family bucket a
/// [`DataTypeId`](crate::io::DataTypeId) falls in, obtained via
/// [`DataTypeId::category`](crate::io::DataTypeId::category). It is a *lossy* summary for callers
/// that want to switch on the family; the fine-grained identity and every `is_*` predicate live
/// on [`DataTypeId`](crate::io::DataTypeId) (which is where the type space is actually
/// enumerated). In particular **width is not a category property** — a fixed-size binary and a
/// variable binary share the `Binary` category but differ in width — so the width predicates
/// live only on the id / [`DataType`](crate::io::DataType), never here.
///
/// It lives at the `io` root — above both the fixed and variable families — because it spans
/// them: the numeric primitives report an integer/float category and the byte types report
/// [`Utf8`](DataTypeCategory::Utf8) / [`Binary`](DataTypeCategory::Binary).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DataTypeCategory {
    /// The null type.
    Null,
    /// An unsigned integer (`u8` … `u256`).
    UnsignedInteger,
    /// A signed integer (`i8` … `i256`).
    SignedInteger,
    /// An IEEE-754 float (`f16`, `f32`, `f64`).
    Float,
    /// A scaled decimal (`d32`, `d64`, `d128`, `d256`).
    Decimal,
    /// A temporal value (date, time, timestamp, duration).
    Temporal,
    /// A UTF-8 string (fixed-size or variable-length).
    Utf8,
    /// Opaque binary (fixed-size or variable-length).
    Binary,
    /// A nested / composite type (struct, list, or map).
    Nested,
}

impl DataTypeCategory {
    /// Whether the category is any integer (signed or unsigned).
    pub const fn is_integer(self) -> bool {
        matches!(self, Self::UnsignedInteger | Self::SignedInteger)
    }

    /// Whether the category is a **signed** numeric (a signed integer, a float, or a decimal —
    /// every decimal is signed).
    pub const fn is_signed(self) -> bool {
        matches!(self, Self::SignedInteger | Self::Float | Self::Decimal)
    }

    /// Whether the category is a float.
    pub const fn is_floating(self) -> bool {
        matches!(self, Self::Float)
    }

    /// Whether the category is a scaled decimal.
    pub const fn is_decimal(self) -> bool {
        matches!(self, Self::Decimal)
    }

    /// Whether the category is a temporal value.
    pub const fn is_temporal(self) -> bool {
        matches!(self, Self::Temporal)
    }

    /// Whether the category is any number (integer, float, or decimal).
    pub const fn is_numeric(self) -> bool {
        self.is_integer() || self.is_floating() || self.is_decimal()
    }

    /// Whether the category is a UTF-8 string.
    pub const fn is_utf8(self) -> bool {
        matches!(self, Self::Utf8)
    }

    /// Whether the category is opaque binary.
    pub const fn is_binary(self) -> bool {
        matches!(self, Self::Binary)
    }

    /// Whether the category is a nested / composite type (struct, list, or map).
    pub const fn is_nested(self) -> bool {
        matches!(self, Self::Nested)
    }
}
