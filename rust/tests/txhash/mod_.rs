//! `rust/src/txhash/mod.rs`: the width-to-algorithm rule no caller can name.
//!
//! `algorithm_of_width` is the crate-private table that decides which digest a
//! declared width carries, so it is reached through `yggdryl::internals`; the
//! widths and datatypes it answers for are public. Everything else a caller
//! can observe lives in `rust/tests/hashing/txhash.rs`.

use yggdryl::internals::txhash::algorithm_of_width;
use yggdryl::txhash::{dtype, width};
use yggdryl::{DataType, DigestAlgorithm};

#[test]
fn widths_and_datatypes_follow_the_algorithm() {
    assert_eq!(width(DigestAlgorithm::Xxh32), 12);
    assert_eq!(width(DigestAlgorithm::Xxh64), 16);
    assert_eq!(width(DigestAlgorithm::Xxh3), 16);
    assert_eq!(width(DigestAlgorithm::Xxh128), 24);
    assert_eq!(
        dtype(DigestAlgorithm::Xxh32),
        DataType::fixed_binary(12).unwrap()
    );
    assert_eq!(
        dtype(DigestAlgorithm::Xxh128),
        DataType::fixed_binary(24).unwrap()
    );
    assert_eq!(algorithm_of_width(12), Some(DigestAlgorithm::Xxh32));
    // Sixteen bytes answer the project default, not XXH64.
    assert_eq!(algorithm_of_width(16), Some(DigestAlgorithm::Xxh3));
    assert_eq!(algorithm_of_width(24), Some(DigestAlgorithm::Xxh128));
    assert_eq!(algorithm_of_width(8), None);
    assert_eq!(algorithm_of_width(20), None);
}
