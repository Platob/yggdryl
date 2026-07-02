//! The [`Nested`] category trait: a type composed of child fields.

use super::RawDataType;

/// A nested type composed of one or more child fields — e.g. `struct`, `list`, `map`.
///
/// [`child_count`](Nested::child_count) reports how many children the type has. Typed
/// child accessors — which must span children of differing data types — land with the
/// concrete nested types as the layer grows.
///
/// ```
/// use yggdryl_data::{Nested, RawDataType};
///
/// // A struct with two children.
/// #[derive(Debug)]
/// struct Pair;
///
/// impl RawDataType for Pair {
///     fn name(&self) -> &str { "struct" }
///     fn arrow_format(&self) -> String { "+s".to_string() }
///     fn byte_width(&self) -> Option<usize> { None } // nested types have no fixed width
/// }
///
/// impl Nested for Pair {
///     fn child_count(&self) -> usize {
///         2
///     }
/// }
///
/// assert_eq!(Pair.child_count(), 2);
/// assert_eq!(Pair.byte_width(), None);
/// ```
pub trait Nested: RawDataType {
    /// The number of child fields this type contains.
    fn child_count(&self) -> usize;
}
