//! [`BufferType`] — the root contract for indexed contiguous storage, shared by every family.

/// The **generic contiguous-storage** root trait — a run of `Elem` values addressable by
/// index, over raw bytes. The fixed [`Buffer`](crate::io::fixed::Buffer) implements it; a
/// variable family reuses that same byte storage for its data buffer.
pub trait BufferType {
    /// The element type.
    type Elem;

    /// The number of elements.
    fn count(&self) -> usize;

    /// The raw backing bytes.
    fn as_bytes(&self) -> &[u8];

    /// The element at `index`, or `None` if out of range.
    fn get(&self, index: usize) -> Option<Self::Elem>;

    /// Whether the buffer has no elements.
    fn is_empty(&self) -> bool {
        self.count() == 0
    }
}
