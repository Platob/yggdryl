//! `rust/src/xxhash/scalar.rs` over the enum leaves `yggdryl-market`
//! claims: each kind's value feeds bytes of its own - its claimed
//! identifier byte first - which no other value's feed equals, and the feed
//! hashes alike however a sink batches it and whatever algorithm digests it.
//! The feed of every core value is `rust/tests/xxhash/scalar.rs`'s.

use yggdryl::xxhash::{Xxh3, xxh3};
use yggdryl::{DigestAlgorithm, Scalar, State};
use yggdryl_market::{MarketDataKind, Side, TimeInForce};

/// A sink that keeps the feed so two values can be compared byte for byte.
#[derive(Default)]
struct Collected(Vec<u8>);

impl std::hash::Hasher for Collected {
    fn finish(&self) -> u64 {
        xxh3(&self.0)
    }

    fn write(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
    }
}

fn feed(value: &Scalar) -> Vec<u8> {
    let mut sink = Collected::default();
    value.write_bytes(&mut sink);
    sink.0
}

/// The market kinds' values beside the core's own enum leaf and the code
/// and text a kind's member is stored and spelled as.
fn corpus() -> Vec<Scalar> {
    vec![
        Scalar::State(State::New),
        Scalar::from(Side::new("BUY").unwrap()),
        Scalar::from(MarketDataKind::Order),
        Scalar::from(TimeInForce::GoodTillCancel),
        Scalar::from(1_u8),
        Scalar::from("BUYS"),
    ]
}

#[test]
fn market_values_that_differ_feed_different_bytes() {
    crate::install::installed();
    let values = corpus();
    for (index, left) in values.iter().enumerate() {
        for right in &values[index + 1..] {
            assert_ne!(left, right);
            assert_ne!(feed(left), feed(right), "{left:?} vs {right:?}");
        }
    }
}

#[test]
fn the_feed_of_a_market_value_does_not_depend_on_the_sink_or_the_algorithm() {
    crate::install::installed();
    for value in corpus() {
        let bytes = feed(&value);
        let mut state = Xxh3::new();
        state.write_scalar(&value);
        assert_eq!(state.as_u64(), xxh3(&bytes), "{value:?}");
        for split in [1_usize, 3, 7, 64] {
            let mut chunked = Xxh3::new();
            for chunk in bytes.chunks(split) {
                chunked.write_bytes(chunk);
            }
            assert_eq!(chunked.as_u64(), state.as_u64(), "{value:?} split {split}");
        }
        for algorithm in DigestAlgorithm::ALL {
            assert_eq!(
                value.digest(algorithm),
                algorithm.digest(&bytes),
                "{value:?} under {algorithm}"
            );
        }
    }
}
