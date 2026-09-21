//! `rust/src/txhash/mod.rs`: the width-to-algorithm rule no caller can name.
//!
//! `algorithm_of_width` is the crate-private table that decides which digest a
//! declared width carries, so it is reached through `yggdryl::internals`; the
//! widths and datatypes it answers for are public. What else a caller can
//! observe is beside it here and in `rust/tests/txhash/value.rs`.

#[cfg(feature = "internals")]
mod internal {
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
}

use yggdryl::txhash::{
    DEFAULT_UNIT, TxHash, UNIX_WIDTH, digest, dtype, txh3, txh32, txh64, txh128,
};
use yggdryl::{DigestAlgorithm, TimeUnit};

const INSTANT: i64 = 1_700_000_000_000_000;

/// One instant, one payload, every algorithm.
fn every_algorithm() -> impl Iterator<Item = (DigestAlgorithm, TxHash)> {
    DigestAlgorithm::ALL
        .into_iter()
        .map(|algorithm| (algorithm, digest(b"AAPL", INSTANT, algorithm)))
}

#[test]
fn one_shots_couple_the_instant_with_the_plain_digest() {
    for (algorithm, value) in every_algorithm() {
        assert_eq!(value.unix(), INSTANT, "{algorithm}");
        assert_eq!(value.unit(), DEFAULT_UNIT, "{algorithm}");
        assert_eq!(value.digest(), algorithm.digest(b"AAPL"), "{algorithm}");
        assert_eq!(value.algorithm(), algorithm);
        assert_eq!(value.width(), UNIX_WIDTH + algorithm.width());
        assert_eq!(value.dtype(), dtype(algorithm));
    }
    assert_eq!(
        txh32(b"AAPL", 1),
        digest(b"AAPL", 1, DigestAlgorithm::Xxh32)
    );
    assert_eq!(
        txh64(b"AAPL", 1),
        digest(b"AAPL", 1, DigestAlgorithm::Xxh64)
    );
    assert_eq!(txh3(b"AAPL", 1), digest(b"AAPL", 1, DigestAlgorithm::Xxh3));
    assert_eq!(
        txh128(b"AAPL", 1),
        digest(b"AAPL", 1, DigestAlgorithm::Xxh128)
    );
    assert_eq!(DEFAULT_UNIT, TimeUnit::Microsecond);
}
