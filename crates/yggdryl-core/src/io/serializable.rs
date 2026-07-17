//! [`Serializable`] — the root byte-codec trait every value type implements when possible.

/// A value that round-trips through a **byte array**: [`serialize_bytes`](Serializable::serialize_bytes)
/// renders the canonical byte form (one pre-sized allocation), and
/// [`deserialize_bytes`](Serializable::deserialize_bytes) is its **exact inverse**. Together with
/// `PartialEq`/`Eq` and `Hash` this is the project's value-type contract: equal iff canonical
/// bytes equal, equal values hash equal, and every value crosses a wire (and the language
/// bindings — pickle in Python, `serializeBytes`/`deserializeBytes` in Node) identically.
///
/// Implementors keep their inherent `serialize_bytes` / `deserialize_bytes` methods (callable
/// without importing the trait); the trait impl delegates, existing so generic code can bound on
/// `T: Serializable`.
///
/// ```
/// use yggdryl_core::io::Serializable;
/// use yggdryl_core::uri::Uri;
///
/// fn roundtrip<T: Serializable>(value: &T) -> Result<T, T::Error> {
///     T::deserialize_bytes(&value.serialize_bytes())
/// }
///
/// let uri = Uri::parse_str("sc://h/p?q=1").unwrap();
/// assert_eq!(roundtrip(&uri).unwrap(), uri);
/// ```
pub trait Serializable: Sized {
    /// The guided error a decode can fail with (`UriError`, `IoError`, …).
    type Error;

    /// The value's canonical byte form — one pre-sized allocation.
    fn serialize_bytes(&self) -> Vec<u8>;

    /// Reconstructs a value from bytes produced by
    /// [`serialize_bytes`](Serializable::serialize_bytes) — the exact inverse.
    fn deserialize_bytes(bytes: &[u8]) -> Result<Self, Self::Error>;
}
