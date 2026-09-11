//! A constant an expression holds, under the datatype it belongs to.

use std::cmp::Ordering;
use std::fmt;

use serde::de::Error as _;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{DataType, Result, Scalar};

/// A constant and the datatype it belongs to.
///
/// An expression is a plan over a schema, so a constant in it carries the
/// datatype it will be compared in rather than a bare Rust primitive:
/// `decimal '1.50'` stays an exact decimal at scale two all the way to the
/// comparison. The value is what [`DataType::scalar`] answers - checked and
/// rewritten into the datatype's own representation - so a literal that
/// exists is a literal that holds. A null is the null of its datatype, which
/// every datatype that can spell one accepts.
///
/// Literals order first by datatype and then by value, matching their exact
/// equality and hashing identity.
///
/// ```
/// use yggdryl::expression::Literal;
/// use yggdryl::{DataType, Scalar};
///
/// # fn main() -> yggdryl::Result<()> {
/// let narrowed = Literal::new(DataType::Int32, 7_i64)?;
/// assert_eq!(narrowed.value(), &Scalar::from(7_i32));
/// assert_eq!(narrowed.to_string(), "int32 '7'");
///
/// let inferred = Literal::infer(Scalar::from("AAPL"))?;
/// assert_eq!(inferred.dtype(), &DataType::Utf8);
/// assert_eq!(inferred.to_string(), "'AAPL'");
///
/// assert!(Literal::new(DataType::Int8, 1_000_i64).is_err());
/// assert!(Literal::null().is_null());
/// # Ok(())
/// # }
/// ```
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Literal {
    dtype: DataType,
    value: Scalar,
}

impl Literal {
    /// Hold a constant under an exact datatype.
    ///
    /// # Errors
    ///
    /// Returns an error when the value is not one the datatype accepts.
    pub fn new(dtype: DataType, value: impl Into<Scalar>) -> Result<Self> {
        let value = dtype.scalar(value)?;
        Ok(Self { dtype, value })
    }

    /// Hold a constant under the datatype it already names.
    ///
    /// # Errors
    ///
    /// Returns an error when the value names no single datatype, which is
    /// what [`Scalar::dtype`] reports.
    pub fn infer(value: Scalar) -> Result<Self> {
        let dtype = value.dtype()?;
        Self::new(dtype, value)
    }

    /// The null literal: a null under [`DataType::Null`].
    #[must_use]
    pub const fn null() -> Self {
        Self {
            dtype: DataType::Null,
            value: Scalar::Null,
        }
    }

    /// The datatype the constant belongs to.
    pub const fn dtype(&self) -> &DataType {
        &self.dtype
    }

    /// The constant itself.
    pub const fn value(&self) -> &Scalar {
        &self.value
    }

    /// Return whether the constant is null.
    pub const fn is_null(&self) -> bool {
        self.value.is_null()
    }

    /// Consume this literal and return both halves.
    pub fn into_parts(self) -> (DataType, Scalar) {
        (self.dtype, self.value)
    }
}

impl fmt::Debug for Literal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Literal")
            .field("dtype", &self.dtype)
            .field("value", &self.value)
            .finish()
    }
}

/// The spelling the grammar reads back as this literal.
impl fmt::Display for Literal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        super::display::write_literal(formatter, self)
    }
}

impl PartialOrd for Literal {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Literal {
    fn cmp(&self, other: &Self) -> Ordering {
        self.dtype
            .cmp(&other.dtype)
            .then_with(|| self.value.cmp(&other.value))
    }
}

impl Serialize for Literal {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut structure = serializer.serialize_struct("Literal", 2)?;
        structure.serialize_field("dtype", &self.dtype)?;
        structure.serialize_field("value", &self.value)?;
        structure.end()
    }
}

impl<'de> Deserialize<'de> for Literal {
    /// Read a literal back through the constructor that validates one.
    ///
    /// Deriving this would accept a datatype and a value that never agreed,
    /// which is exactly the state [`Literal::new`] exists to refuse.
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // This mirror must stay field-for-field identical to `Literal`.
        #[derive(Deserialize)]
        struct StructuralLiteral {
            dtype: DataType,
            value: Scalar,
        }

        let structural = StructuralLiteral::deserialize(deserializer)?;
        Self::new(structural.dtype, structural.value).map_err(D::Error::custom)
    }
}
