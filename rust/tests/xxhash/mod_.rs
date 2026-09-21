//! `rust/src/xxhash/mod.rs`: the digest halves no caller can name.
//!
//! `low_64` is the crate-private narrowing every 64-bit digest takes from its
//! 128-bit sibling, so it is reached through `yggdryl::internals`; the two
//! widths it relates are public. Everything else a caller can observe lives
//! beside it here.

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::internals::xxhash::low_64;
    use yggdryl::xxhash::{xxh3, xxh128};

    /// A deterministic payload of `length` bytes.
    fn corpus(length: usize) -> Vec<u8> {
        (0..length).map(|index| (index * 31 + 7) as u8).collect()
    }

    #[test]
    fn xxh128_carries_xxh3_in_its_low_half_where_the_branches_agree() {
        // The reference shares its mixing between the two widths on exactly two
        // branches: 1-to-3 bytes, and the long path past the 240-byte cutoff.
        // Everywhere else - the empty input and the 4-to-240 branches - XXH3-128
        // folds a second accumulator that moves the low half, so the two widths
        // are different values and neither can be derived from the other.
        for length in [1, 2, 3, 241, 512, 4096] {
            let payload = corpus(length);
            assert_eq!(low_64(xxh128(&payload)), xxh3(&payload), "length {length}");
        }
        assert_eq!(low_64(xxh128(b"abc")), xxh3(b"abc"));

        for length in [0, 4, 8, 16, 128, 240] {
            let payload = corpus(length);
            assert_ne!(low_64(xxh128(&payload)), xxh3(&payload), "length {length}");
        }
    }
}

mod algorithms {

    use yggdryl::DigestAlgorithm;
    use yggdryl::xxhash::{
        SECRET_MINIMUM_LENGTH, Xxh3, Xxh32, Xxh64, Xxh128, xxh3, xxh3_with_secret, xxh3_with_seed,
        xxh32, xxh32_with_seed, xxh64, xxh64_with_seed, xxh128, xxh128_with_seed,
        xxh128_with_seed_and_secret,
    };

    /// One payload per XXH3 size branch, plus the boundaries between them.
    const BRANCH_LENGTHS: [usize; 14] = [0, 1, 3, 4, 8, 9, 16, 17, 64, 128, 129, 240, 241, 4096];

    /// A deterministic corpus byte at `index`, mixing so no branch sees a constant.
    fn corpus(length: usize) -> Vec<u8> {
        (0..length).map(|index| (index * 31 + 7) as u8).collect()
    }

    /// A valid custom secret of `length` bytes.
    fn secret(length: usize) -> Vec<u8> {
        (0..length).map(|index| (index * 17 + 3) as u8).collect()
    }

    #[test]
    fn published_vectors_pin_every_algorithm() {
        assert_eq!(xxh32(b""), 0x02cc_5d05);
        assert_eq!(xxh64(b""), 0xef46_db37_51d8_e999);
        assert_eq!(xxh3(b""), 0x2d06_8005_38d3_94c2);
        assert_eq!(xxh128(b""), 0x99aa_06d3_0147_98d8_6001_c324_468d_497f);

        assert_eq!(xxh32(b"abc"), 0x32d1_53ff);
        assert_eq!(xxh64(b"abc"), 0x44bc_2cf5_ad77_0999);
        assert_eq!(xxh3(b"abc"), 0x78af_5f94_892f_3950);
        assert_eq!(xxh128(b"abc"), 0x06b0_5ab6_733a_6185_78af_5f94_892f_3950);
    }

    #[test]
    fn every_size_branch_agrees_between_one_shot_and_streaming() {
        for length in BRANCH_LENGTHS {
            let payload = corpus(length);
            for algorithm in DigestAlgorithm::ALL {
                let mut digester = algorithm.digester();
                digester.write_bytes(&payload);
                assert_eq!(
                    digester.as_digest(),
                    algorithm.digest(&payload),
                    "{algorithm} at length {length}"
                );
            }
        }
    }

    #[test]
    fn every_size_branch_agrees_under_a_seed() {
        for length in BRANCH_LENGTHS {
            let payload = corpus(length);

            let mut state = Xxh32::with_seed(0x9e37_79b1);
            state.write_bytes(&payload);
            assert_eq!(state.as_u32(), xxh32_with_seed(&payload, 0x9e37_79b1));

            let mut state = Xxh64::with_seed(0x9e37_79b1_85eb_ca87);
            state.write_bytes(&payload);
            assert_eq!(
                state.as_u64(),
                xxh64_with_seed(&payload, 0x9e37_79b1_85eb_ca87)
            );

            let mut state = Xxh3::with_seed(42);
            state.write_bytes(&payload);
            assert_eq!(state.as_u64(), xxh3_with_seed(&payload, 42));

            let mut state = Xxh128::with_seed(42);
            state.write_bytes(&payload);
            assert_eq!(state.as_u128(), xxh128_with_seed(&payload, 42));
        }
    }

    #[test]
    fn every_size_branch_agrees_under_a_custom_secret() {
        let custom = secret(SECRET_MINIMUM_LENGTH + 56);
        for length in BRANCH_LENGTHS {
            let payload = corpus(length);

            let mut state = Xxh3::from_secret(&custom).unwrap();
            state.write_bytes(&payload);
            assert_eq!(
                state.as_u64(),
                xxh3_with_secret(&payload, &custom).unwrap(),
                "xxh3-64 at length {length}"
            );

            let mut state = Xxh128::from_seed_and_secret(9, &custom).unwrap();
            state.write_bytes(&payload);
            assert_eq!(
                state.as_u128(),
                xxh128_with_seed_and_secret(&payload, 9, &custom).unwrap(),
                "xxh3-128 at length {length}"
            );
        }
    }

    #[test]
    fn the_module_digest_helper_dispatches_like_the_algorithm() {
        for algorithm in DigestAlgorithm::ALL {
            assert_eq!(
                yggdryl::xxhash::digest(b"AAPL", algorithm),
                algorithm.digest(b"AAPL")
            );
        }
    }
}
