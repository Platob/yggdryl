//! The [`Column`] trait — the shared behaviour of a single column, whatever its
//! backing. One contract covers a **materialized** column (values already in
//! memory) and a **lazy** column (an expression yet to be evaluated), so the rest
//! of the engine can treat them alike.

use std::fmt;

use crate::{DataType, Field};

/// Error returned by [`Column`] operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColumnError {
    /// A slice/index reached past the column's length.
    OutOfBounds(String),
    /// A cast between incompatible types was requested.
    Cast {
        /// The column's current type.
        from: DataType,
        /// The requested target type.
        to: DataType,
    },
    /// The operation is not supported by this column backing.
    Unsupported(String),
}

impl fmt::Display for ColumnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ColumnError::OutOfBounds(detail) => write!(f, "column index out of bounds: {detail}"),
            ColumnError::Cast { from, to } => {
                write!(f, "cannot cast column from {from} to {to}")
            }
            ColumnError::Unsupported(detail) => write!(f, "unsupported column operation: {detail}"),
        }
    }
}

impl std::error::Error for ColumnError {}

/// A single named, typed column.
///
/// Implementors fall into two camps that this trait deliberately unifies:
///
/// - a **materialized** column owns its values ([`is_materialized`](Column::is_materialized)
///   is `true`, [`len`](Column::len) is known);
/// - a **lazy** column describes a computation; its [`len`](Column::len) may be
///   unknown until evaluated (returns `None`).
///
/// The identity of a column — its [`field`](Column::field) (name, [`DataType`] and
/// nullability) — is always known, so [`name`](Column::name) /
/// [`data_type`](Column::data_type) / [`is_nullable`](Column::is_nullable) are
/// total. Transformations consume `self` and return a new column of the same kind,
/// so they compose whether they run now (materialized) or are recorded (lazy).
///
/// ```
/// use yggdryl_saga::{Column, ColumnError, DataType, Field, PrimitiveType};
///
/// // A minimal materialized column for illustration.
/// struct Vec64 { field: Field, values: Vec<i64> }
/// impl Column for Vec64 {
///     fn field(&self) -> &Field { &self.field }
///     fn is_materialized(&self) -> bool { true }
///     fn len(&self) -> Option<usize> { Some(self.values.len()) }
///     fn rename(mut self, name: impl Into<String>) -> Self {
///         self.field = self.field.with_name(name);
///         self
///     }
///     fn cast(self, to: DataType) -> Result<Self, ColumnError> {
///         Err(ColumnError::Cast { from: self.field.data_type().clone(), to })
///     }
///     fn slice(mut self, offset: usize, length: usize) -> Result<Self, ColumnError> {
///         let end = offset.saturating_add(length).min(self.values.len());
///         self.values = self.values[offset.min(end)..end].to_vec();
///         Ok(self)
///     }
///     fn tail(self, n: usize) -> Result<Self, ColumnError> {
///         let len = self.values.len();
///         self.slice(len.saturating_sub(n), n)
///     }
/// }
///
/// let col = Vec64 { field: Field::new("px", PrimitiveType::Int64.into(), false), values: vec![1, 2, 3] };
/// assert_eq!(col.name(), "px");
/// assert_eq!(col.data_type(), &DataType::from(PrimitiveType::Int64));
/// assert_eq!(col.head(2).unwrap().len(), Some(2));
/// ```
pub trait Column: Sized {
    /// The column's header: its name, [`DataType`] and nullability.
    fn field(&self) -> &Field;

    /// The column name.
    fn name(&self) -> &str {
        self.field().name()
    }

    /// The column's logical [`DataType`].
    fn data_type(&self) -> &DataType {
        self.field().data_type()
    }

    /// Whether the column admits nulls.
    fn is_nullable(&self) -> bool {
        self.field().is_nullable()
    }

    /// Whether the values are already in memory (`false` for an unevaluated lazy
    /// column).
    fn is_materialized(&self) -> bool;

    /// The number of values, if known without forcing evaluation (`None` when a
    /// lazy column would have to be computed to answer).
    fn len(&self) -> Option<usize>;

    /// Whether the column is known to be empty (`None` when the length is unknown).
    fn is_empty(&self) -> Option<bool> {
        self.len().map(|n| n == 0)
    }

    /// Returns the column renamed.
    fn rename(self, name: impl Into<String>) -> Self;

    /// Returns the column cast to `data_type`, or [`ColumnError::Cast`] if the
    /// conversion is not possible.
    fn cast(self, data_type: DataType) -> Result<Self, ColumnError>;

    /// Returns `length` values starting at `offset`.
    fn slice(self, offset: usize, length: usize) -> Result<Self, ColumnError>;

    /// Returns the first `n` values.
    fn head(self, n: usize) -> Result<Self, ColumnError> {
        self.slice(0, n)
    }

    /// Returns the last `n` values.
    fn tail(self, n: usize) -> Result<Self, ColumnError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PrimitiveType;

    /// A tiny materialized column used to exercise the trait's provided methods.
    struct TestColumn {
        field: Field,
        len: usize,
    }

    impl Column for TestColumn {
        fn field(&self) -> &Field {
            &self.field
        }
        fn is_materialized(&self) -> bool {
            true
        }
        fn len(&self) -> Option<usize> {
            Some(self.len)
        }
        fn rename(mut self, name: impl Into<String>) -> Self {
            self.field = self.field.with_name(name);
            self
        }
        fn cast(mut self, data_type: DataType) -> Result<Self, ColumnError> {
            self.field = self.field.with_data_type(data_type);
            Ok(self)
        }
        fn slice(mut self, offset: usize, length: usize) -> Result<Self, ColumnError> {
            let end = offset.saturating_add(length).min(self.len);
            self.len = end - offset.min(end);
            Ok(self)
        }
        fn tail(self, n: usize) -> Result<Self, ColumnError> {
            let len = self.len;
            self.slice(len.saturating_sub(n), n)
        }
    }

    fn col() -> TestColumn {
        TestColumn {
            field: Field::new("px", PrimitiveType::Int64.into(), false),
            len: 5,
        }
    }

    #[test]
    fn identity_defaults_read_through_field() {
        let c = col();
        assert_eq!(c.name(), "px");
        assert_eq!(c.data_type(), &DataType::from(PrimitiveType::Int64));
        assert!(!c.is_nullable());
        assert_eq!(c.is_empty(), Some(false));
        assert!(c.is_materialized());
    }

    #[test]
    fn provided_head_uses_slice() {
        assert_eq!(col().head(2).unwrap().len(), Some(2));
        assert_eq!(col().tail(1).unwrap().len(), Some(1));
        // Out-of-range slices clamp.
        assert_eq!(col().slice(10, 5).unwrap().len(), Some(0));
    }

    #[test]
    fn transforms_compose() {
        let c = col()
            .rename("price")
            .cast(PrimitiveType::Float64.into())
            .unwrap();
        assert_eq!(c.name(), "price");
        assert_eq!(c.data_type(), &DataType::from(PrimitiveType::Float64));
    }
}
