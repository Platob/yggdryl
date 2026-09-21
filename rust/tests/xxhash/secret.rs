//! `rust/src/xxhash/secret.rs`: the custom secret the XXH3 pair accepts, and
//! the length it refuses.

mod xxhash {

    use yggdryl::Error;
    use yggdryl::xxhash::{
        SECRET_MINIMUM_LENGTH, Xxh3, Xxh128, xxh3, xxh3_with_secret, xxh3_with_seed_and_secret,
        xxh128, xxh128_with_secret, xxh128_with_seed_and_secret,
    };

    /// A deterministic corpus byte at `index`, mixing so no branch sees a constant.
    fn corpus(length: usize) -> Vec<u8> {
        (0..length).map(|index| (index * 31 + 7) as u8).collect()
    }

    /// A valid custom secret of `length` bytes.
    fn secret(length: usize) -> Vec<u8> {
        (0..length).map(|index| (index * 17 + 3) as u8).collect()
    }

    #[test]
    fn a_custom_secret_changes_the_answer_past_the_cutoff() {
        let custom = secret(SECRET_MINIMUM_LENGTH);
        let payload = corpus(4096);
        assert_ne!(xxh3_with_secret(&payload, &custom).unwrap(), xxh3(&payload));
        assert_ne!(
            xxh128_with_secret(&payload, &custom).unwrap(),
            xxh128(&payload)
        );

        // XXH3 consults a custom secret only past 240 bytes. Below that the
        // algorithm uses its derived secret and the seed, which is the protocol's
        // own rule for the seed-and-secret family and what keeps a one-shot and a
        // streaming state answering one value for the same bytes. Pinned here so
        // the boundary is a stated contract rather than a surprise.
        for length in [0_usize, 1, 64, 240] {
            let short = corpus(length);
            assert_eq!(
                xxh3_with_secret(&short, &custom).unwrap(),
                xxh3(&short),
                "length {length}"
            );
            assert_eq!(
                xxh128_with_secret(&short, &custom).unwrap(),
                xxh128(&short),
                "length {length}"
            );
        }
        for length in [241_usize, 1024] {
            let long = corpus(length);
            assert_ne!(
                xxh3_with_secret(&long, &custom).unwrap(),
                xxh3(&long),
                "length {length}"
            );
        }
    }

    #[test]
    fn a_secret_one_byte_short_is_rejected_by_length() {
        let short = secret(SECRET_MINIMUM_LENGTH - 1);
        let cases: [Error; 6] = [
            xxh3_with_secret(b"", &short).unwrap_err(),
            xxh3_with_seed_and_secret(b"", 1, &short).unwrap_err(),
            xxh128_with_secret(b"", &short).unwrap_err(),
            xxh128_with_seed_and_secret(b"", 1, &short).unwrap_err(),
            Xxh3::from_secret(&short).unwrap_err(),
            Xxh128::from_seed_and_secret(1, &short).unwrap_err(),
        ];
        for error in cases {
            assert!(
                matches!(
                    error,
                    Error::InvalidSecret {
                        required: SECRET_MINIMUM_LENGTH,
                        actual: 135,
                        ..
                    }
                ),
                "{error}"
            );
            assert!(
                error.to_string().contains("at least 136 bytes, got 135"),
                "{error}"
            );
        }
        // A short secret is rejected whatever the payload length, even though the
        // reference only consults a secret past its 240-byte cutoff.
        assert!(xxh3_with_secret(&corpus(4096), &short).is_err());
        assert!(Xxh3::from_secret(&secret(SECRET_MINIMUM_LENGTH)).is_ok());
    }
}
