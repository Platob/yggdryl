//! [`SerieType`] — the root contract for a nullable column of values, shared by every family.

/// The **generic column** root trait — a sequence of nullable `Elem` values. The fixed
/// [`Serie`](crate::io::fixed::Serie) implements it; the variable
/// [`ByteSerie`](crate::io::var::ByteSerie) (strings, binary) does too.
pub trait SerieType {
    /// The element type.
    type Elem;

    /// The number of elements.
    fn len(&self) -> usize;

    /// The number of null elements.
    fn null_count(&self) -> usize;

    /// The element at `index`, or `None` if null or out of range.
    fn get(&self, index: usize) -> Option<Self::Elem>;

    /// Whether the column is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether the column carries any nulls.
    fn has_nulls(&self) -> bool {
        self.null_count() > 0
    }
}
