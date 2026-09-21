//! `rust/src/xxhash/state.rs`: the four resumable streaming states - what a
//! chunking never changes, and what a clear returns to.

mod xxhash {
    use std::hash::{BuildHasher as _, Hasher as _};
    use yggdryl::xxhash::{
        SECRET_MINIMUM_LENGTH, Xxh3, Xxh32, Xxh64, Xxh128, xxh3, xxh3_with_seed,
        xxh3_with_seed_and_secret, xxh32, xxh32_with_seed, xxh64, xxh64_with_seed, xxh128,
        xxh128_with_seed_and_secret,
    };
    use yggdryl::{DigestAlgorithm, Error};

    /// A deterministic corpus byte at `index`, mixing so no branch sees a constant.
    fn corpus(length: usize) -> Vec<u8> {
        (0..length).map(|index| (index * 31 + 7) as u8).collect()
    }

    /// A valid custom secret of `length` bytes.
    fn secret(length: usize) -> Vec<u8> {
        (0..length).map(|index| (index * 17 + 3) as u8).collect()
    }

    #[test]
    fn chunking_never_changes_a_digest() {
        let payload = corpus(5000);
        for algorithm in DigestAlgorithm::ALL {
            let whole = algorithm.digest(&payload);
            for split in [1, 7, 64, 240, 1024, payload.len()] {
                let mut digester = algorithm.digester();
                for chunk in payload.chunks(split) {
                    digester.write_bytes(chunk);
                }
                assert_eq!(digester.as_digest(), whole, "{algorithm} split {split}");
            }
        }
    }

    #[test]
    fn random_splits_never_change_a_digest() {
        // A deterministic pseudo-random split schedule: the property is that the
        // boundaries do not matter, so the generator only has to be varied.
        let payload = corpus(3000);
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        for algorithm in DigestAlgorithm::ALL {
            let whole = algorithm.digest(&payload);
            for _ in 0..32 {
                let mut digester = algorithm.digester();
                let mut offset = 0;
                while offset < payload.len() {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    let take = (seed as usize % 257).max(1).min(payload.len() - offset);
                    digester.write_bytes(&payload[offset..offset + take]);
                    offset += take;
                }
                assert_eq!(digester.as_digest(), whole, "{algorithm}");
            }
        }
    }

    #[test]
    fn an_empty_chunk_contributes_nothing_wherever_it_sits() {
        for algorithm in DigestAlgorithm::ALL {
            let mut digester = algorithm.digester();
            digester.write_bytes(b"");
            digester.write_bytes(b"fill 100");
            digester.write_bytes(b"");
            assert_eq!(digester.as_digest(), algorithm.digest(b"fill 100"));
        }
    }

    #[test]
    fn a_state_answers_repeatedly_and_keeps_accepting_bytes() {
        let mut state = Xxh3::new();
        state.write_bytes(b"AAPL");
        let first = state.as_u64();
        assert_eq!(first, state.as_u64());
        assert_eq!(first, xxh3(b"AAPL"));
        state.write_bytes(b",187.23");
        assert_eq!(state.as_u64(), xxh3(b"AAPL,187.23"));
    }

    #[test]
    fn clear_returns_to_the_constructed_seed_and_secret() {
        let custom = secret(SECRET_MINIMUM_LENGTH);

        let mut state = Xxh32::with_seed(11);
        state.write_bytes(b"AAPL");
        state.clear();
        assert_eq!(state.as_u32(), xxh32_with_seed(b"", 11));
        assert_eq!(state.seed(), 11);

        let mut state = Xxh64::with_seed(11);
        state.write_bytes(b"AAPL");
        state.clear();
        assert_eq!(state.as_u64(), xxh64_with_seed(b"", 11));

        let mut state = Xxh3::with_seed(11);
        state.write_bytes(b"AAPL");
        state.clear();
        assert_eq!(state.as_u64(), xxh3_with_seed(b"", 11));

        let mut state = Xxh3::from_seed_and_secret(11, &custom).unwrap();
        state.write_bytes(b"AAPL");
        state.clear();
        assert_eq!(
            state.as_u64(),
            xxh3_with_seed_and_secret(b"", 11, &custom).unwrap()
        );
        assert_eq!(state.secret(), Some(custom.as_slice()));

        let mut state = Xxh128::from_seed_and_secret(11, &custom).unwrap();
        state.write_bytes(b"AAPL");
        state.clear();
        assert_eq!(
            state.as_u128(),
            xxh128_with_seed_and_secret(b"", 11, &custom).unwrap()
        );

        let mut state = Xxh128::new();
        state.write_bytes(b"AAPL");
        state.clear();
        assert_eq!(state.as_u128(), xxh128(b""));
    }

    #[test]
    fn a_state_reads_a_reader_in_bounded_chunks() {
        let payload = corpus(200_000);

        let mut state = Xxh32::new();
        assert_eq!(
            state.write_reader(&mut payload.as_slice()).unwrap(),
            payload.len() as u64
        );
        assert_eq!(state.as_u32(), xxh32(&payload));

        let mut state = Xxh64::new();
        state.write_reader(&mut payload.as_slice()).unwrap();
        assert_eq!(state.as_u64(), xxh64(&payload));

        let mut state = Xxh3::new();
        state.write_reader(&mut payload.as_slice()).unwrap();
        assert_eq!(state.as_u64(), xxh3(&payload));

        let mut state = Xxh128::new();
        state.write_reader(&mut payload.as_slice()).unwrap();
        assert_eq!(state.as_u128(), xxh128(&payload));

        // A reader interleaved with byte writes is still one contiguous payload.
        let mut state = Xxh3::new();
        state.write_bytes(b"AAPL");
        state.write_reader(&mut b",187.23".as_slice()).unwrap();
        assert_eq!(state.as_u64(), xxh3(b"AAPL,187.23"));
    }

    #[test]
    fn a_reader_failure_surfaces_and_leaves_the_fed_prefix() {
        struct Failing(bool);
        impl std::io::Read for Failing {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                if self.0 {
                    return Err(std::io::Error::other("backend gone"));
                }
                self.0 = true;
                buffer[..4].copy_from_slice(b"AAPL");
                Ok(4)
            }
        }

        let mut state = Xxh3::new();
        let error = state.write_reader(&mut Failing(false)).unwrap_err();
        assert!(matches!(error, Error::Io(_)), "{error}");
        assert_eq!(state.as_u64(), xxh3(b"AAPL"));
    }

    #[test]
    fn every_state_answers_its_own_digest_width() {
        let mut narrow = Xxh32::new();
        narrow.write_bytes(b"AAPL");
        assert_eq!(narrow.as_digest().as_u32(), Some(narrow.as_u32()));
        assert_eq!(narrow.as_digest().algorithm(), DigestAlgorithm::Xxh32);

        let mut wide = Xxh128::new();
        wide.write_bytes(b"AAPL");
        assert_eq!(wide.as_digest().as_u128(), Some(wide.as_u128()));
        assert_eq!(wide.as_digest().algorithm(), DigestAlgorithm::Xxh128);
    }

    #[test]
    fn a_state_is_a_hasher_and_a_build_hasher() {
        let mut state = Xxh3::new();
        state.write(b"abc");
        assert_eq!(state.finish(), xxh3(b"abc"));

        let mut narrow = Xxh32::new();
        narrow.write(b"abc");
        assert_eq!(narrow.finish(), u64::from(xxh32(b"abc")));

        // `Hasher::finish` on the 128-bit state answers the low half; `as_u128`
        // is the full value.
        let mut wide = Xxh128::new();
        wide.write(b"abc");
        assert_eq!(wide.finish(), xxh3(b"abc"));
        assert_eq!(wide.as_u128(), xxh128(b"abc"));

        // A builder carries the seed and secret into every state it builds, and
        // builds a fresh one every time rather than handing back a fed state.
        let seeded = Xxh64::with_seed(5);
        assert_eq!(seeded.hash_one("AAPL"), seeded.hash_one("AAPL"));
        assert_ne!(seeded.hash_one("AAPL"), Xxh64::new().hash_one("AAPL"));
        let mut built = seeded.build_hasher();
        built.write_bytes(b"abc");
        assert_eq!(built.as_u64(), xxh64_with_seed(b"abc", 5));

        let mut built = Xxh3::with_seed(5).build_hasher();
        built.write_bytes(b"abc");
        assert_eq!(built.as_u64(), xxh3_with_seed(b"abc", 5));

        let custom = secret(SECRET_MINIMUM_LENGTH);
        let mut built = Xxh128::from_seed_and_secret(5, &custom)
            .unwrap()
            .build_hasher();
        built.write_bytes(b"abc");
        assert_eq!(
            built.as_u128(),
            xxh128_with_seed_and_secret(b"abc", 5, &custom).unwrap()
        );

        // The states drop into a `HashMap` through their own `BuildHasher`.
        let mut map: std::collections::HashMap<&str, u8, Xxh3> =
            std::collections::HashMap::with_hasher(Xxh3::new());
        map.insert("AAPL", 1);
        assert_eq!(map.get("AAPL"), Some(&1));
    }

    #[test]
    fn a_default_state_is_an_unseeded_state() {
        assert_eq!(Xxh32::default().as_u32(), xxh32(b""));
        assert_eq!(Xxh64::default().as_u64(), xxh64(b""));
        assert_eq!(Xxh3::default().as_u64(), xxh3(b""));
        assert_eq!(Xxh128::default().as_u128(), xxh128(b""));
        assert!(Xxh3::default().secret().is_none());
    }

    #[test]
    fn debug_shows_the_seed_and_secret_length_rather_than_the_accumulator() {
        let custom = secret(SECRET_MINIMUM_LENGTH);
        assert_eq!(format!("{:?}", Xxh32::with_seed(3)), "Xxh32 { seed: 3 }");
        assert_eq!(
            format!("{:?}", Xxh3::from_seed_and_secret(3, &custom).unwrap()),
            "Xxh3 { seed: 3, secret: Some(136) }"
        );
        assert_eq!(
            format!("{:?}", Xxh128::new()),
            "Xxh128 { seed: 0, secret: None }"
        );
    }
}
