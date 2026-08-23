//! Canonical views over map-shaped pairs stored in wire order.

/// Borrow map-shaped pairs in deterministic key order without changing storage.
///
/// The sort is stable so invalid duplicate keys retain their observable
/// first-to-last order while independent keys remain order-insensitive.
pub(crate) fn sorted_pairs<K: Ord, V>(pairs: &[(K, V)]) -> Vec<&(K, V)> {
    let mut sorted: Vec<_> = pairs.iter().collect();
    sorted.sort_by(|left, right| left.0.cmp(&right.0));
    sorted
}

/// Borrow set-shaped values in deterministic complete-value order.
#[cfg(feature = "iceberg")]
pub(crate) fn sorted_values<T: Ord>(values: &[T]) -> Vec<&T> {
    let mut sorted: Vec<_> = values.iter().collect();
    sorted.sort_unstable();
    sorted
}
