use std::hash::{BuildHasher as _, Hasher as _};

use super::{
    SECRET_MINIMUM_LENGTH, Xxh3_64, Xxh3_128, Xxh32, Xxh64, xxh3_64, xxh3_64_with_secret,
    xxh3_64_with_seed, xxh3_64_with_seed_and_secret, xxh3_128, xxh3_128_with_secret,
    xxh3_128_with_seed, xxh3_128_with_seed_and_secret, xxh32, xxh32_with_seed, xxh64,
    xxh64_with_seed,
};
use crate::{DigestAlgorithm, Error};

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
    assert_eq!(xxh3_64(b""), 0x2d06_8005_38d3_94c2);
    assert_eq!(xxh3_128(b""), 0x99aa_06d3_0147_98d8_6001_c324_468d_497f);

    assert_eq!(xxh32(b"abc"), 0x32d1_53ff);
    assert_eq!(xxh64(b"abc"), 0x44bc_2cf5_ad77_0999);
    assert_eq!(xxh3_64(b"abc"), 0x78af_5f94_892f_3950);
    assert_eq!(xxh3_128(b"abc"), 0x06b0_5ab6_733a_6185_78af_5f94_892f_3950);
}

#[test]
fn xxh3_128_carries_xxh3_64_in_its_low_half_where_the_branches_agree() {
    // The reference shares its mixing between the two widths on exactly two
    // branches: 1-to-3 bytes, and the long path past the 240-byte cutoff.
    // Everywhere else - the empty input and the 4-to-240 branches - XXH3-128
    // folds a second accumulator that moves the low half, so the two widths
    // are different values and neither can be derived from the other.
    for length in [1, 2, 3, 241, 512, 4096] {
        let payload = corpus(length);
        assert_eq!(
            super::low_64(xxh3_128(&payload)),
            xxh3_64(&payload),
            "length {length}"
        );
    }
    assert_eq!(super::low_64(xxh3_128(b"abc")), xxh3_64(b"abc"));

    for length in [0, 4, 8, 16, 128, 240] {
        let payload = corpus(length);
        assert_ne!(
            super::low_64(xxh3_128(&payload)),
            xxh3_64(&payload),
            "length {length}"
        );
    }
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

        let mut state = Xxh3_64::with_seed(42);
        state.write_bytes(&payload);
        assert_eq!(state.as_u64(), xxh3_64_with_seed(&payload, 42));

        let mut state = Xxh3_128::with_seed(42);
        state.write_bytes(&payload);
        assert_eq!(state.as_u128(), xxh3_128_with_seed(&payload, 42));
    }
}

#[test]
fn every_size_branch_agrees_under_a_custom_secret() {
    let custom = secret(SECRET_MINIMUM_LENGTH + 56);
    for length in BRANCH_LENGTHS {
        let payload = corpus(length);

        let mut state = Xxh3_64::from_secret(&custom).unwrap();
        state.write_bytes(&payload);
        assert_eq!(
            state.as_u64(),
            xxh3_64_with_secret(&payload, &custom).unwrap(),
            "xxh3-64 at length {length}"
        );

        let mut state = Xxh3_128::from_seed_and_secret(9, &custom).unwrap();
        state.write_bytes(&payload);
        assert_eq!(
            state.as_u128(),
            xxh3_128_with_seed_and_secret(&payload, 9, &custom).unwrap(),
            "xxh3-128 at length {length}"
        );
    }
}

#[test]
fn a_custom_secret_changes_the_answer_past_the_cutoff() {
    let custom = secret(SECRET_MINIMUM_LENGTH);
    let payload = corpus(4096);
    assert_ne!(
        xxh3_64_with_secret(&payload, &custom).unwrap(),
        xxh3_64(&payload)
    );
    assert_ne!(
        xxh3_128_with_secret(&payload, &custom).unwrap(),
        xxh3_128(&payload)
    );

    // XXH3 consults a custom secret only past 240 bytes. Below that the
    // algorithm uses its derived secret and the seed, which is the protocol's
    // own rule for the seed-and-secret family and what keeps a one-shot and a
    // streaming state answering one value for the same bytes. Pinned here so
    // the boundary is a stated contract rather than a surprise.
    for length in [0_usize, 1, 64, 240] {
        let short = corpus(length);
        assert_eq!(
            xxh3_64_with_secret(&short, &custom).unwrap(),
            xxh3_64(&short),
            "length {length}"
        );
        assert_eq!(
            xxh3_128_with_secret(&short, &custom).unwrap(),
            xxh3_128(&short),
            "length {length}"
        );
    }
    for length in [241_usize, 1024] {
        let long = corpus(length);
        assert_ne!(
            xxh3_64_with_secret(&long, &custom).unwrap(),
            xxh3_64(&long),
            "length {length}"
        );
    }
}

#[test]
fn a_secret_one_byte_short_is_rejected_by_length() {
    let short = secret(SECRET_MINIMUM_LENGTH - 1);
    let cases: [Error; 6] = [
        xxh3_64_with_secret(b"", &short).unwrap_err(),
        xxh3_64_with_seed_and_secret(b"", 1, &short).unwrap_err(),
        xxh3_128_with_secret(b"", &short).unwrap_err(),
        xxh3_128_with_seed_and_secret(b"", 1, &short).unwrap_err(),
        Xxh3_64::from_secret(&short).unwrap_err(),
        Xxh3_128::from_seed_and_secret(1, &short).unwrap_err(),
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
    assert!(xxh3_64_with_secret(&corpus(4096), &short).is_err());
    assert!(Xxh3_64::from_secret(&secret(SECRET_MINIMUM_LENGTH)).is_ok());
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
    let mut state = Xxh3_64::new();
    state.write_bytes(b"AAPL");
    let first = state.as_u64();
    assert_eq!(first, state.as_u64());
    assert_eq!(first, xxh3_64(b"AAPL"));
    state.write_bytes(b",187.23");
    assert_eq!(state.as_u64(), xxh3_64(b"AAPL,187.23"));
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

    let mut state = Xxh3_64::with_seed(11);
    state.write_bytes(b"AAPL");
    state.clear();
    assert_eq!(state.as_u64(), xxh3_64_with_seed(b"", 11));

    let mut state = Xxh3_64::from_seed_and_secret(11, &custom).unwrap();
    state.write_bytes(b"AAPL");
    state.clear();
    assert_eq!(
        state.as_u64(),
        xxh3_64_with_seed_and_secret(b"", 11, &custom).unwrap()
    );
    assert_eq!(state.secret(), Some(custom.as_slice()));

    let mut state = Xxh3_128::from_seed_and_secret(11, &custom).unwrap();
    state.write_bytes(b"AAPL");
    state.clear();
    assert_eq!(
        state.as_u128(),
        xxh3_128_with_seed_and_secret(b"", 11, &custom).unwrap()
    );

    let mut state = Xxh3_128::new();
    state.write_bytes(b"AAPL");
    state.clear();
    assert_eq!(state.as_u128(), xxh3_128(b""));
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

    let mut state = Xxh3_64::new();
    state.write_reader(&mut payload.as_slice()).unwrap();
    assert_eq!(state.as_u64(), xxh3_64(&payload));

    let mut state = Xxh3_128::new();
    state.write_reader(&mut payload.as_slice()).unwrap();
    assert_eq!(state.as_u128(), xxh3_128(&payload));

    // A reader interleaved with byte writes is still one contiguous payload.
    let mut state = Xxh3_64::new();
    state.write_bytes(b"AAPL");
    state.write_reader(&mut b",187.23".as_slice()).unwrap();
    assert_eq!(state.as_u64(), xxh3_64(b"AAPL,187.23"));
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

    let mut state = Xxh3_64::new();
    let error = state.write_reader(&mut Failing(false)).unwrap_err();
    assert!(matches!(error, Error::Io(_)), "{error}");
    assert_eq!(state.as_u64(), xxh3_64(b"AAPL"));
}

#[test]
fn every_state_answers_its_own_digest_width() {
    let mut narrow = Xxh32::new();
    narrow.write_bytes(b"AAPL");
    assert_eq!(narrow.as_digest().as_u32(), Some(narrow.as_u32()));
    assert_eq!(narrow.as_digest().algorithm(), DigestAlgorithm::Xxh32);

    let mut wide = Xxh3_128::new();
    wide.write_bytes(b"AAPL");
    assert_eq!(wide.as_digest().as_u128(), Some(wide.as_u128()));
    assert_eq!(wide.as_digest().algorithm(), DigestAlgorithm::Xxh3_128);
}

#[test]
fn a_state_is_a_hasher_and_a_build_hasher() {
    let mut state = Xxh3_64::new();
    state.write(b"abc");
    assert_eq!(state.finish(), xxh3_64(b"abc"));

    let mut narrow = Xxh32::new();
    narrow.write(b"abc");
    assert_eq!(narrow.finish(), u64::from(xxh32(b"abc")));

    // `Hasher::finish` on the 128-bit state answers the low half; `as_u128`
    // is the full value.
    let mut wide = Xxh3_128::new();
    wide.write(b"abc");
    assert_eq!(wide.finish(), xxh3_64(b"abc"));
    assert_eq!(wide.as_u128(), xxh3_128(b"abc"));

    // A builder carries the seed and secret into every state it builds, and
    // builds a fresh one every time rather than handing back a fed state.
    let seeded = Xxh64::with_seed(5);
    assert_eq!(seeded.hash_one("AAPL"), seeded.hash_one("AAPL"));
    assert_ne!(seeded.hash_one("AAPL"), Xxh64::new().hash_one("AAPL"));
    let mut built = seeded.build_hasher();
    built.write_bytes(b"abc");
    assert_eq!(built.as_u64(), xxh64_with_seed(b"abc", 5));

    let mut built = Xxh3_64::with_seed(5).build_hasher();
    built.write_bytes(b"abc");
    assert_eq!(built.as_u64(), xxh3_64_with_seed(b"abc", 5));

    let custom = secret(SECRET_MINIMUM_LENGTH);
    let mut built = Xxh3_128::from_seed_and_secret(5, &custom)
        .unwrap()
        .build_hasher();
    built.write_bytes(b"abc");
    assert_eq!(
        built.as_u128(),
        xxh3_128_with_seed_and_secret(b"abc", 5, &custom).unwrap()
    );

    // The states drop into a `HashMap` through their own `BuildHasher`.
    let mut map: std::collections::HashMap<&str, u8, Xxh3_64> =
        std::collections::HashMap::with_hasher(Xxh3_64::new());
    map.insert("AAPL", 1);
    assert_eq!(map.get("AAPL"), Some(&1));
}

#[test]
fn a_default_state_is_an_unseeded_state() {
    assert_eq!(Xxh32::default().as_u32(), xxh32(b""));
    assert_eq!(Xxh64::default().as_u64(), xxh64(b""));
    assert_eq!(Xxh3_64::default().as_u64(), xxh3_64(b""));
    assert_eq!(Xxh3_128::default().as_u128(), xxh3_128(b""));
    assert!(Xxh3_64::default().secret().is_none());
}

#[test]
fn debug_shows_the_seed_and_secret_length_rather_than_the_accumulator() {
    let custom = secret(SECRET_MINIMUM_LENGTH);
    assert_eq!(format!("{:?}", Xxh32::with_seed(3)), "Xxh32 { seed: 3 }");
    assert_eq!(
        format!("{:?}", Xxh3_64::from_seed_and_secret(3, &custom).unwrap()),
        "Xxh3_64 { seed: 3, secret: Some(136) }"
    );
    assert_eq!(
        format!("{:?}", Xxh3_128::new()),
        "Xxh3_128 { seed: 0, secret: None }"
    );
}

#[test]
fn the_module_digest_helper_dispatches_like_the_algorithm() {
    for algorithm in DigestAlgorithm::ALL {
        assert_eq!(super::digest(b"AAPL", algorithm), algorithm.digest(b"AAPL"));
    }
}

mod handles {
    use std::io::{Read as _, Write as _};

    use super::super::{reader, writer, xxh3_64};
    use crate::io::{Buffer, IOBase};
    use crate::{DigestAlgorithm, Error};

    /// A temporary root, named so parallel tests never share one.
    fn root(label: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "yggdryl-xxhash-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    /// Long enough to cross several stream windows, so the digest proves the
    /// chunk joining rather than a single short read.
    fn payload() -> Vec<u8> {
        let row = b"AAPL,187.23,2024-02-01T10:00:00Z\n";
        row.iter().copied().cycle().take(400_000).collect()
    }

    /// Assert a handle's streamed digest equals the one-shot of its bytes.
    fn agrees(label: &str, handle: &impl IOBase) {
        let bytes = handle.read_all_bytes().unwrap();
        for algorithm in DigestAlgorithm::ALL {
            assert_eq!(
                handle.read_digest(algorithm).unwrap(),
                algorithm.digest(&bytes),
                "{label} under {algorithm}"
            );
        }
    }

    #[test]
    fn a_memory_buffer_streams_the_same_digest_as_its_bytes() {
        let mut handle = Buffer::new();
        handle.write_all_bytes(&payload()).unwrap();
        agrees("buffer", &handle);
    }

    #[test]
    fn a_memory_mapped_local_file_streams_the_same_digest_as_its_bytes() {
        let path = root("local").join("trades.csv");
        std::fs::write(&path, payload()).unwrap();
        agrees("local file", &crate::local::File::new(&path).unwrap());
    }

    #[test]
    fn an_arrow_filesystem_handle_streams_the_same_digest_as_its_bytes() {
        use std::sync::Arc;

        use crate::arrowfs::{Folder, MemoryFileSystem};

        let lake = Folder::from_location(Arc::new(MemoryFileSystem::new()), "lake").unwrap();
        let mut leaf = lake.child_by_path("trades.csv").unwrap();
        leaf.write_all_bytes(&payload()).unwrap();
        leaf.close().unwrap();
        agrees("arrowfs file", &lake.child_by_path("trades.csv").unwrap());
    }

    #[test]
    fn a_cache_streams_the_same_digest_and_stays_unpolluted() {
        use crate::buffered::{Buffered, BufferedOptions};

        let bytes = payload();
        let mut inner = Buffer::new();
        inner.write_all_bytes(&bytes).unwrap();

        // A fresh cache, asked only for a digest: the read goes through
        // `pstream_bytes`, which the cache delegates straight to the handle it
        // wraps, so hashing a large object leaves no page behind.
        let handle = Buffered::new(inner, BufferedOptions::default());
        assert_eq!(handle.cached_pages(), 0);
        assert_eq!(
            handle.read_digest(DigestAlgorithm::Xxh3_64).unwrap(),
            DigestAlgorithm::Xxh3_64.digest(&bytes)
        );
        assert_eq!(handle.cached_pages(), 0);

        // Reading bytes through the cache does populate it, and the digest
        // still agrees afterwards.
        agrees("buffered", &handle);
        assert!(handle.cached_pages() > 0);
    }

    #[test]
    fn a_coding_wrapper_digests_the_decoded_payload_and_its_handle_the_compressed_form() {
        use crate::gzip::Gzip;

        let plain = payload();
        let mut handle = Gzip::new(Buffer::new());
        handle.write_all_bytes(&plain).unwrap();
        handle.flush().unwrap();

        // The wrapper answers for the bytes it presents.
        assert_eq!(
            handle
                .read_digest(DigestAlgorithm::Xxh3_64)
                .unwrap()
                .as_u64(),
            Some(xxh3_64(&plain))
        );
        // The handle underneath answers for the bytes it holds.
        let compressed = handle.handle().read_all_bytes().unwrap();
        assert_eq!(
            handle
                .handle()
                .read_digest(DigestAlgorithm::Xxh3_64)
                .unwrap()
                .as_u64(),
            Some(xxh3_64(&compressed))
        );
        assert_ne!(xxh3_64(&plain), xxh3_64(&compressed));
        agrees("gzip view", &handle);
        agrees("gzip handle", handle.handle());
    }

    #[test]
    fn an_empty_or_missing_resource_digests_as_no_bytes() {
        for algorithm in DigestAlgorithm::ALL {
            let empty = algorithm.digest(b"");
            assert_eq!(Buffer::new().read_digest(algorithm).unwrap(), empty);

            let missing = root("missing").join("never-written.csv");
            assert_eq!(
                crate::local::File::new(&missing)
                    .unwrap()
                    .read_digest(algorithm)
                    .unwrap(),
                empty
            );
        }
    }

    #[test]
    fn a_container_is_refused_by_kind() {
        let folder = crate::local::Folder::new(root("container")).unwrap();
        let error = folder.read_digest(DigestAlgorithm::Xxh3_64).unwrap_err();
        assert!(
            matches!(
                error,
                Error::NotAtomic {
                    operation: "digest",
                    kind: "directory",
                    ..
                }
            ),
            "{error}"
        );
        assert!(error.to_string().contains("got a directory"), "{error}");
        assert!(
            folder
                .read_range_digest(0, 8, DigestAlgorithm::Xxh3_64)
                .is_err()
        );
    }

    #[test]
    fn a_range_digest_clamps_exactly_as_a_range_read_does() {
        let bytes = payload();
        let mut handle = Buffer::new();
        handle.write_all_bytes(&bytes).unwrap();

        let cases = [
            (0_u64, 0_usize),
            (0, 1),
            (0, 240),
            (7, 100_000),
            (0, bytes.len()),
            // Past the end on the length, and then on the offset itself.
            (0, bytes.len() * 2),
            (bytes.len() as u64 - 3, 64),
            (bytes.len() as u64, 64),
            (bytes.len() as u64 * 2, 64),
        ];
        for (offset, length) in cases {
            for algorithm in DigestAlgorithm::ALL {
                assert_eq!(
                    handle.read_range_digest(offset, length, algorithm).unwrap(),
                    algorithm.digest(&handle.read_range_bytes(offset, length).unwrap()),
                    "{algorithm} over {offset}..+{length}"
                );
            }
        }
        // The whole-value digest is the range digest of the whole range.
        assert_eq!(
            handle.read_digest(DigestAlgorithm::Xxh64).unwrap(),
            handle
                .read_range_digest(0, bytes.len(), DigestAlgorithm::Xxh64)
                .unwrap()
        );
    }

    #[test]
    fn a_backend_failure_surfaces_instead_of_a_partial_digest() {
        use crate::Result;

        /// A handle whose reads fail past the first window.
        struct Failing {
            handle: Buffer,
        }

        impl crate::io::IOMedia for Failing {
            crate::delegate_iomedia!(handle);
        }

        impl IOBase for Failing {
            // Everything but `pread`, so the stream reaches the failure below
            // through the default `pstream_bytes` rather than around it.
            crate::delegate_iobase!(handle: pwrite, size, capacity, reserve, truncate, url,
                media_type, set_media_type, flush, open, opened, close, parent, child_by_path,
                ls, kind, clear, remove, is_atomic, is_tabular, is_io);

            fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
                if offset > 0 {
                    return Err(crate::Error::Io(std::io::Error::other("backend gone")));
                }
                self.handle.pread(offset, buffer)
            }
        }

        let mut inner = Buffer::new();
        inner.write_all_bytes(&payload()).unwrap();
        let handle = Failing { handle: inner };
        let error = handle.read_digest(DigestAlgorithm::Xxh3_64).unwrap_err();
        assert!(matches!(error, Error::Io(_)), "{error}");
    }

    #[test]
    fn a_pass_through_reader_hashes_what_it_moves() {
        let bytes = payload();
        let mut source = reader(bytes.as_slice(), DigestAlgorithm::Xxh3_128);
        assert_eq!(source.algorithm(), DigestAlgorithm::Xxh3_128);
        // The digest is answerable at any point, not only at the end.
        assert_eq!(source.as_digest(), DigestAlgorithm::Xxh3_128.digest(b""));

        let mut moved = Vec::new();
        source.read_to_end(&mut moved).unwrap();
        assert_eq!(moved, bytes);
        assert_eq!(source.as_digest(), DigestAlgorithm::Xxh3_128.digest(&bytes));
        assert!(source.into_inner().is_empty());
    }

    #[test]
    fn a_tee_writer_hashes_what_it_writes() {
        let bytes = payload();
        let mut target = writer(Vec::new(), DigestAlgorithm::Xxh32);
        assert_eq!(target.algorithm(), DigestAlgorithm::Xxh32);
        for chunk in bytes.chunks(9_973) {
            target.write_all(chunk).unwrap();
        }
        target.flush().unwrap();
        assert_eq!(target.as_digest(), DigestAlgorithm::Xxh32.digest(&bytes));
        assert_eq!(target.into_inner(), bytes);
    }

    #[test]
    fn a_writer_hashes_a_handle_as_it_fills_it() {
        // The case the pair exists for: bytes are being moved anyway, so the
        // digest costs the pass that was already happening.
        let bytes = payload();
        let mut handle = Buffer::new();
        let mut target = writer(Vec::new(), DigestAlgorithm::Xxh3_64);
        target.write_all(&bytes).unwrap();
        let digest = target.as_digest();
        handle.write_all_bytes(&target.into_inner()).unwrap();
        assert_eq!(
            handle.read_digest(DigestAlgorithm::Xxh3_64).unwrap(),
            digest
        );
    }
}

mod hashed {
    use super::super::{Hashed, xxh3_64, xxh3_64_with_seed};
    use crate::io::{Buffer, IOBase};
    use crate::{DigestAlgorithm, Error};

    /// A handle that counts the reads reaching the one it wraps, so "answered
    /// without re-reading" is a number rather than a claim.
    #[derive(Debug)]
    struct Counted {
        handle: Buffer,
        reads: std::sync::atomic::AtomicUsize,
    }

    impl Counted {
        fn new() -> Self {
            Self {
                handle: Buffer::new(),
                reads: std::sync::atomic::AtomicUsize::new(0),
            }
        }

        fn reads(&self) -> usize {
            self.reads.load(std::sync::atomic::Ordering::Relaxed)
        }
    }

    impl crate::io::IOMedia for Counted {
        crate::delegate_iomedia!(handle);
    }

    impl IOBase for Counted {
        crate::delegate_iobase!(handle: pwrite, size, capacity, reserve, truncate, url,
            media_type, set_media_type, flush, open, opened, close, parent, child_by_path,
            ls, kind, clear, remove, is_atomic, is_tabular, is_io);

        fn pread(&self, offset: u64, buffer: &mut [u8]) -> crate::Result<usize> {
            self.reads
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.handle.pread(offset, buffer)
        }
    }

    #[test]
    fn sequential_writes_answer_without_reading_the_bytes_back() {
        let mut handle = Hashed::new(Counted::new(), DigestAlgorithm::Xxh3_64);
        handle.write_all_bytes(b"symbol,price\n").unwrap();
        handle.append_bytes(b"AAPL,187.23\n").unwrap();
        handle.append_bytes(b"MSFT,410.10\n").unwrap();
        handle.flush().unwrap();

        let expected = b"symbol,price\nAAPL,187.23\nMSFT,410.10\n";
        assert_eq!(
            handle
                .read_digest(DigestAlgorithm::Xxh3_64)
                .unwrap()
                .as_u64(),
            Some(xxh3_64(expected))
        );
        assert_eq!(handle.handle().reads(), 0, "the running state was used");
        // Asking again is still free.
        assert_eq!(
            handle
                .read_digest(DigestAlgorithm::Xxh3_64)
                .unwrap()
                .as_u64(),
            Some(xxh3_64(expected))
        );
        assert_eq!(handle.handle().reads(), 0);
        assert_eq!(handle.read_all_bytes().unwrap(), expected);
    }

    #[test]
    fn an_out_of_order_write_re_streams_to_the_same_value() {
        let mut handle = Hashed::new(Counted::new(), DigestAlgorithm::Xxh3_64);
        handle
            .write_all_bytes(b"symbol,price\nAAPL,187.23\n")
            .unwrap();
        assert_eq!(handle.handle().reads(), 0);

        // A write that is neither offset 0 nor the running append point.
        handle.pwrite_all(7, b"PRICE").unwrap();
        let bytes = handle.read_all_bytes().unwrap();
        let before = handle.handle().reads();
        assert_eq!(
            handle
                .read_digest(DigestAlgorithm::Xxh3_64)
                .unwrap()
                .as_u64(),
            Some(xxh3_64(&bytes))
        );
        assert!(
            handle.handle().reads() > before,
            "a stale state must re-stream"
        );
        // The state re-armed, so the next ask is free again.
        let after = handle.handle().reads();
        assert_eq!(
            handle
                .read_digest(DigestAlgorithm::Xxh3_64)
                .unwrap()
                .as_u64(),
            Some(xxh3_64(&bytes))
        );
        assert_eq!(handle.handle().reads(), after);
    }

    #[test]
    fn a_prefix_overwrite_shorter_than_the_value_is_not_the_whole_digest() {
        let mut handle = Hashed::new(Buffer::new(), DigestAlgorithm::Xxh64);
        handle
            .write_all_bytes(b"symbol,price\nAAPL,187.23\n")
            .unwrap();
        // Offset 0, but only a prefix: the running state would cover 4 bytes
        // of a 25-byte value, so it must not be answered from.
        handle.pwrite_all(0, b"SYMB").unwrap();
        let bytes = handle.read_all_bytes().unwrap();
        assert_eq!(
            handle.read_digest(DigestAlgorithm::Xxh64).unwrap(),
            DigestAlgorithm::Xxh64.digest(&bytes)
        );
    }

    #[test]
    fn clear_and_remove_re_arm_the_state() {
        let mut handle = Hashed::new(Buffer::new(), DigestAlgorithm::Xxh3_64);
        handle.write_all_bytes(b"AAPL,187.23\n").unwrap();

        handle.clear().unwrap();
        assert_eq!(
            handle.read_digest(DigestAlgorithm::Xxh3_64).unwrap(),
            DigestAlgorithm::Xxh3_64.digest(b"")
        );
        handle.write_all_bytes(b"MSFT,410.10\n").unwrap();
        assert_eq!(
            handle
                .read_digest(DigestAlgorithm::Xxh3_64)
                .unwrap()
                .as_u64(),
            Some(xxh3_64(b"MSFT,410.10\n"))
        );

        handle.remove(false).unwrap();
        assert_eq!(
            handle.read_digest(DigestAlgorithm::Xxh3_64).unwrap(),
            DigestAlgorithm::Xxh3_64.digest(b"")
        );

        handle.write_all_bytes(b"AAPL,187.23\n").unwrap();
        handle.truncate(4).unwrap();
        let bytes = handle.read_all_bytes().unwrap();
        assert_eq!(bytes, b"AAPL");
        assert_eq!(
            handle.read_digest(DigestAlgorithm::Xxh3_64).unwrap(),
            DigestAlgorithm::Xxh3_64.digest(&bytes)
        );
    }

    #[test]
    fn pending_writes_count_only_after_flush() {
        use std::sync::Arc;

        use crate::arrowfs::{Folder, MemoryFileSystem};

        // An Arrow filesystem file stages writes in memory and publishes the
        // whole value on flush, which is exactly the case the size check is
        // there for.
        let lake = Folder::from_location(Arc::new(MemoryFileSystem::new()), "lake").unwrap();
        let leaf = lake.child_by_path("trades.csv").unwrap();
        let mut handle = Hashed::new(leaf, DigestAlgorithm::Xxh3_64);

        handle.pwrite_all(0, b"AAPL,187.23\n").unwrap();
        // Staged but not published: the digest describes what is stored.
        assert_eq!(
            handle.read_digest(DigestAlgorithm::Xxh3_64).unwrap(),
            DigestAlgorithm::Xxh3_64.digest(&handle.read_all_bytes().unwrap()),
        );

        handle.flush().unwrap();
        assert_eq!(
            handle
                .read_digest(DigestAlgorithm::Xxh3_64)
                .unwrap()
                .as_u64(),
            Some(xxh3_64(b"AAPL,187.23\n"))
        );
        assert_eq!(handle.read_all_bytes().unwrap(), b"AAPL,187.23\n");
    }

    #[test]
    fn a_seed_travels_with_the_wrapper() {
        let mut handle = Hashed::new(Buffer::new(), DigestAlgorithm::Xxh3_64).with_seed(42);
        assert_eq!(handle.seed(), 42);
        assert_eq!(handle.algorithm(), DigestAlgorithm::Xxh3_64);
        handle.write_all_bytes(b"AAPL,187.23\n").unwrap();
        assert_eq!(
            handle
                .read_digest(DigestAlgorithm::Xxh3_64)
                .unwrap()
                .as_u64(),
            Some(xxh3_64_with_seed(b"AAPL,187.23\n", 42))
        );

        // Re-streaming re-arms under the same seed, not an unseeded state.
        handle.pwrite_all(4, b"!").unwrap();
        let bytes = handle.read_all_bytes().unwrap();
        assert_eq!(
            handle
                .read_digest(DigestAlgorithm::Xxh3_64)
                .unwrap()
                .as_u64(),
            Some(xxh3_64_with_seed(&bytes, 42))
        );
    }

    #[test]
    fn another_algorithm_streams_rather_than_reading_the_running_state() {
        let mut handle = Hashed::new(Buffer::new(), DigestAlgorithm::Xxh3_64);
        handle.write_all_bytes(b"AAPL,187.23\n").unwrap();
        for algorithm in DigestAlgorithm::ALL {
            assert_eq!(
                handle.read_digest(algorithm).unwrap(),
                algorithm.digest(b"AAPL,187.23\n"),
                "{algorithm}"
            );
        }
    }

    #[test]
    fn the_wrapper_is_transparent_to_everything_else() {
        let mut handle = Hashed::new(Buffer::new(), DigestAlgorithm::Xxh3_64);
        handle
            .write_all_bytes(b"symbol,price\nAAPL,187.23\n")
            .unwrap();

        assert_eq!(handle.size(), 25);
        assert_eq!(handle.read_range_bytes(0, 6).unwrap(), b"symbol");
        assert_eq!(
            handle
                .read_range_digest(0, 6, DigestAlgorithm::Xxh64)
                .unwrap(),
            DigestAlgorithm::Xxh64.digest(b"symbol")
        );
        assert!(!handle.is_container());
        assert_eq!(
            handle.into_handle().read_all_bytes().unwrap(),
            b"symbol,price\nAAPL,187.23\n"
        );
    }

    #[test]
    fn a_container_is_still_refused_by_kind() {
        let root = std::env::temp_dir().join(format!(
            "yggdryl-xxhash-hashed-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let handle = Hashed::new(
            crate::local::Folder::new(&root).unwrap(),
            DigestAlgorithm::Xxh3_64,
        );
        // The running state starts live and empty and a folder's size is
        // zero, so the check has to come from the handle's kind rather than
        // from the state - under this wrapper's own algorithm as much as under
        // any other.
        for algorithm in DigestAlgorithm::ALL {
            let error = handle.read_digest(algorithm).unwrap_err();
            assert!(
                matches!(error, Error::NotAtomic { .. }),
                "{algorithm}: {error}"
            );
        }
    }
}

mod values {
    use std::hash::Hasher as _;
    use std::sync::Arc;

    use super::super::{Xxh3_64, xxh3_64};
    use crate::{
        Codec, DataTypeId, DigestAlgorithm, EnumScalar, Float16, Float32, Float64, I256, Scalar,
        TimeUnit, Timezone,
    };

    /// A sink that keeps the feed so two values can be compared byte for byte.
    #[derive(Default)]
    struct Collected(Vec<u8>);

    impl std::hash::Hasher for Collected {
        fn finish(&self) -> u64 {
            xxh3_64(&self.0)
        }

        fn write(&mut self, bytes: &[u8]) {
            self.0.extend_from_slice(bytes);
        }
    }

    /// Return one value's canonical feed.
    fn feed(value: &Scalar) -> Vec<u8> {
        let mut sink = Collected::default();
        value.write_bytes(&mut sink);
        sink.0
    }

    /// Every variant, plus the pairs that must agree and the pairs that must not.
    fn corpus() -> Vec<Scalar> {
        vec![
            Scalar::Null,
            Scalar::Bool(false),
            Scalar::Bool(true),
            Scalar::I8(-1),
            Scalar::I8(0),
            Scalar::I8(1),
            Scalar::I16(-300),
            Scalar::I32(0x31),
            Scalar::I64(i64::MIN),
            Scalar::I128(i128::MIN),
            Scalar::U8(0x31),
            Scalar::U16(u16::MAX),
            Scalar::U32(u32::MAX),
            Scalar::U64(u64::MAX),
            Scalar::U128(u128::MAX),
            Scalar::F16(Float16::from_f16(half::f16::from_f32(1.5))),
            Scalar::F32(Float32::from_f32(1.5)),
            Scalar::F64(Float64::from_f64(1.5)),
            Scalar::F64(Float64::from_f64(-0.0)),
            Scalar::F64(Float64::from_f64(0.0)),
            Scalar::F64(Float64::from_f64(f64::NAN)),
            Scalar::D128(100, 2),
            Scalar::D128(-1, 0),
            Scalar::D256(I256::from_i128(1), 0),
            Scalar::from(""),
            Scalar::from("1"),
            Scalar::from("AAPL"),
            Scalar::Enum(EnumScalar::Codec(Codec::Gzip)),
            Scalar::Enum(EnumScalar::Codec(Codec::Zstd)),
            Scalar::Enum(EnumScalar::DataTypeId(DataTypeId::Int128)),
            Scalar::Bytes(Arc::from(b"".as_slice())),
            Scalar::Bytes(Arc::from(b"1".as_slice())),
            Scalar::Bytes(Arc::from(b"AAPL".as_slice())),
            Scalar::Geospatial(Arc::from(b"AAPL".as_slice())),
            Scalar::date32_in(1, TimeUnit::Day, Timezone::NAIVE).unwrap(),
            Scalar::date64_in(86_400_000, TimeUnit::Millisecond, Timezone::NAIVE).unwrap(),
            Scalar::time32(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
            Scalar::time64(1_000_000_000, TimeUnit::Nanosecond, Timezone::NAIVE).unwrap(),
            Scalar::datetime64(0, TimeUnit::Microsecond, Timezone::NAIVE).unwrap(),
            Scalar::datetime64(0, TimeUnit::Microsecond, Timezone::UTC).unwrap(),
            Scalar::duration32_in(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
            Scalar::duration64_in(1_000, TimeUnit::Millisecond, Timezone::NAIVE).unwrap(),
            Scalar::from_sequence([]),
            Scalar::from_sequence([Scalar::from("a"), Scalar::from("b")]),
            Scalar::from_sequence([Scalar::from("ab")]),
            Scalar::from_mapping([(Scalar::from("a"), Scalar::I64(1))]).unwrap(),
            Scalar::from_record([("a", Scalar::I64(1))]).unwrap(),
            Scalar::from_record([("a", Scalar::I64(1)), ("b", Scalar::Null)]).unwrap(),
        ]
    }

    #[test]
    fn equal_values_feed_identical_bytes() {
        // The pairs that make this non-trivial: `Scalar` compares across
        // widths, so a feed keyed on the storage width would break here.
        let equal: [(Scalar, Scalar); 8] = [
            (Scalar::I8(1), Scalar::I64(1)),
            (Scalar::U8(1), Scalar::I128(1)),
            (Scalar::I64(-1), Scalar::I128(-1)),
            (
                Scalar::F32(Float32::from_f32(1.5)),
                Scalar::F64(Float64::from_f64(1.5)),
            ),
            (
                Scalar::F16(Float16::from_f16(half::f16::from_f32(1.5))),
                Scalar::F64(Float64::from_f64(1.5)),
            ),
            (Scalar::D128(100, 2), Scalar::D256(I256::from_i128(1), 0)),
            (
                Scalar::date32_in(1, TimeUnit::Day, Timezone::NAIVE).unwrap(),
                Scalar::date64_in(86_400_000, TimeUnit::Millisecond, Timezone::NAIVE).unwrap(),
            ),
            (
                Scalar::time32(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
                Scalar::time64(1_000_000_000, TimeUnit::Nanosecond, Timezone::NAIVE).unwrap(),
            ),
        ];
        for (left, right) in equal {
            assert_eq!(left, right, "the corpus pair is not equal to begin with");
            assert_eq!(feed(&left), feed(&right), "{left:?} vs {right:?}");
            assert_eq!(
                left.digest(DigestAlgorithm::Xxh3_64),
                right.digest(DigestAlgorithm::Xxh3_64)
            );
        }

        // And over the whole corpus: equality and an identical feed agree in
        // both directions.
        let values = corpus();
        for left in &values {
            for right in &values {
                assert_eq!(
                    left == right,
                    feed(left) == feed(right),
                    "{left:?} vs {right:?}"
                );
            }
        }
    }

    #[test]
    fn values_that_differ_feed_different_bytes() {
        let values = corpus();
        for (index, left) in values.iter().enumerate() {
            for right in &values[index + 1..] {
                if left == right {
                    continue;
                }
                assert_ne!(feed(left), feed(right), "{left:?} vs {right:?}");
            }
        }

        // The specific boundaries a tagless feed would collapse.
        assert_ne!(feed(&Scalar::from("1")), feed(&Scalar::U8(0x31)));
        assert_ne!(
            feed(&Scalar::from("1")),
            feed(&Scalar::Bytes(Arc::from(b"1".as_slice())))
        );
        assert_ne!(
            feed(&Scalar::Bytes(Arc::from(b"AAPL".as_slice()))),
            feed(&Scalar::Geospatial(Arc::from(b"AAPL".as_slice())))
        );
        assert_ne!(
            feed(&Scalar::from_sequence([
                Scalar::from("a"),
                Scalar::from("b")
            ])),
            feed(&Scalar::from_sequence([Scalar::from("ab")]))
        );
        // A null and an empty string are not the same absence.
        assert_ne!(feed(&Scalar::Null), feed(&Scalar::from("")));
        assert_ne!(
            feed(&Scalar::Null),
            feed(&Scalar::Bytes(Arc::from(b"".as_slice())))
        );
    }

    #[test]
    fn the_feed_starts_with_the_pinned_datatype_id_byte() {
        // The wire contract: a variant inserted into `DataTypeId` anywhere but
        // the end moves these numbers and changes every stored digest.
        let cases: [(Scalar, DataTypeId); 16] = [
            (Scalar::Null, DataTypeId::Null),
            (Scalar::Bool(true), DataTypeId::Boolean),
            (Scalar::U8(1), DataTypeId::UInt128),
            (Scalar::I8(-1), DataTypeId::Int128),
            (Scalar::F32(Float32::from_f32(1.5)), DataTypeId::Float64),
            (Scalar::D128(1, 0), DataTypeId::Decimal256),
            (Scalar::from("AAPL"), DataTypeId::Utf8),
            (
                Scalar::Enum(EnumScalar::Codec(Codec::Gzip)),
                DataTypeId::Dictionary,
            ),
            (
                Scalar::Bytes(Arc::from(b"AAPL".as_slice())),
                DataTypeId::Binary,
            ),
            (
                Scalar::Geospatial(Arc::from(b"AAPL".as_slice())),
                DataTypeId::Geometry,
            ),
            (
                Scalar::date32_in(1, TimeUnit::Day, Timezone::NAIVE).unwrap(),
                DataTypeId::Date64,
            ),
            (
                Scalar::time32(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
                DataTypeId::Time64,
            ),
            (
                Scalar::datetime64(0, TimeUnit::Microsecond, Timezone::UTC).unwrap(),
                DataTypeId::Timestamp,
            ),
            (
                Scalar::duration32_in(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
                DataTypeId::Duration64,
            ),
            (Scalar::from_sequence([]), DataTypeId::List),
            (
                Scalar::from_record([] as [(&str, Scalar); 0]).unwrap(),
                DataTypeId::Struct,
            ),
        ];
        for (value, id) in cases {
            assert_eq!(feed(&value)[0], id.as_u8(), "{value:?}");
        }
        assert_eq!(
            feed(&Scalar::from_mapping([]).unwrap())[0],
            DataTypeId::Map.as_u8()
        );

        // The exact bytes of a small value, so the layout itself is pinned and
        // not only its first byte.
        assert_eq!(feed(&Scalar::Null), vec![DataTypeId::Null.as_u8()]);
        assert_eq!(
            feed(&Scalar::Bool(true)),
            vec![DataTypeId::Boolean.as_u8(), 1]
        );
        let mut expected = vec![DataTypeId::Utf8.as_u8()];
        expected.extend_from_slice(&4_u64.to_le_bytes());
        expected.extend_from_slice(b"AAPL");
        assert_eq!(feed(&Scalar::from("AAPL")), expected);
    }

    #[test]
    fn the_feed_does_not_depend_on_how_the_sink_batches_it() {
        for value in corpus() {
            let bytes = feed(&value);
            let mut state = Xxh3_64::new();
            state.write_scalar(&value);
            assert_eq!(state.as_u64(), xxh3_64(&bytes), "{value:?}");

            for split in [1_usize, 3, 7, 64] {
                let mut chunked = Xxh3_64::new();
                for chunk in bytes.chunks(split) {
                    chunked.write_bytes(chunk);
                }
                assert_eq!(chunked.as_u64(), state.as_u64(), "{value:?} split {split}");
            }
        }
    }

    #[test]
    fn every_algorithm_digests_a_value_through_the_same_feed() {
        for value in corpus() {
            let bytes = feed(&value);
            for algorithm in DigestAlgorithm::ALL {
                assert_eq!(
                    value.digest(algorithm),
                    algorithm.digest(&bytes),
                    "{value:?} under {algorithm}"
                );
            }
        }
    }

    #[test]
    fn a_typed_scalar_digests_as_the_value_inside_it() {
        let typed =
            crate::TypedScalar::from_parts(crate::DataType::Utf8, Scalar::from("AAPL")).unwrap();
        for algorithm in DigestAlgorithm::ALL {
            assert_eq!(
                typed.digest(algorithm),
                Scalar::from("AAPL").digest(algorithm)
            );
        }
    }

    #[test]
    fn nesting_past_the_shared_limit_is_bounded_rather_than_a_panic() {
        // Caller input can nest as deeply as whoever built it chose, so the
        // walk is bounded the way every other recursive descent here is.
        let mut deep = Scalar::from("leaf");
        for _ in 0..crate::DataType::PARSE_RECURSION_LIMIT * 4 {
            deep = Scalar::from_sequence([deep]);
        }
        let bytes = feed(&deep);
        assert_eq!(*bytes.last().unwrap(), 0xff, "the subtree was cut");
        assert_eq!(
            deep.digest(DigestAlgorithm::Xxh3_64),
            DigestAlgorithm::Xxh3_64.digest(&bytes)
        );
        // Values differing only below the cut are indistinguishable, exactly
        // as `dtype` refuses to name them.
        let mut other = Scalar::from("other");
        for _ in 0..crate::DataType::PARSE_RECURSION_LIMIT * 4 {
            other = Scalar::from_sequence([other]);
        }
        assert_eq!(feed(&other), bytes);
        assert!(deep.dtype().is_err());
    }

    #[test]
    fn value_bytes_are_the_payload_alone() {
        assert_eq!(&*Scalar::from("AAPL").as_value_bytes().unwrap(), b"AAPL");
        assert_eq!(
            &*Scalar::Bytes(Arc::from(b"\x00\xff".as_slice()))
                .as_value_bytes()
                .unwrap(),
            &[0x00, 0xff]
        );
        assert_eq!(
            &*Scalar::Geospatial(Arc::from(b"wkb".as_slice()))
                .as_value_bytes()
                .unwrap(),
            b"wkb"
        );
        assert_eq!(&*Scalar::Bool(true).as_value_bytes().unwrap(), &[1]);
        assert_eq!(&*Scalar::Bool(false).as_value_bytes().unwrap(), &[0]);
        assert_eq!(&*Scalar::I32(1).as_value_bytes().unwrap(), &[1, 0, 0, 0]);
        assert_eq!(&*Scalar::U8(0x31).as_value_bytes().unwrap(), b"1");
        assert_eq!(
            &*Scalar::F64(Float64::from_f64(1.5))
                .as_value_bytes()
                .unwrap(),
            &1.5_f64.to_bits().to_le_bytes()
        );
        assert_eq!(
            Scalar::D256(I256::from_i128(1), 3)
                .as_value_bytes()
                .unwrap()
                .len(),
            32
        );
        assert_eq!(
            &*Scalar::Enum(EnumScalar::Codec(Codec::Gzip))
                .as_value_bytes()
                .unwrap(),
            b"gzip"
        );
        assert_eq!(
            &*Scalar::date32_in(1, TimeUnit::Day, Timezone::NAIVE)
                .unwrap()
                .as_value_bytes()
                .unwrap(),
            &[1, 0, 0, 0]
        );

        // The four variants with no payload of their own.
        assert!(Scalar::Null.as_value_bytes().is_none());
        assert!(Scalar::from_sequence([]).as_value_bytes().is_none());
        assert!(Scalar::from_mapping([]).unwrap().as_value_bytes().is_none());
        assert!(
            Scalar::from_record([] as [(&str, Scalar); 0])
                .unwrap()
                .as_value_bytes()
                .is_none()
        );

        // The widths a payload view keeps, which the canonical feed collapses.
        assert_eq!(Scalar::I8(1).as_value_bytes().unwrap().len(), 1);
        assert_eq!(Scalar::I64(1).as_value_bytes().unwrap().len(), 8);
        assert_ne!(
            Scalar::I8(1).as_value_bytes().unwrap(),
            Scalar::I64(1).as_value_bytes().unwrap()
        );
        assert_eq!(feed(&Scalar::I8(1)), feed(&Scalar::I64(1)));
    }

    #[test]
    fn a_value_byte_view_compares_and_hashes_as_its_bytes() {
        let text = Scalar::from("1");
        let number = Scalar::U8(0x31);
        let borrowed = text.as_value_bytes().unwrap();
        let inline = number.as_value_bytes().unwrap();
        assert_eq!(borrowed, inline);
        assert_eq!(format!("{borrowed:?}"), format!("{inline:?}"));
        assert_eq!(borrowed.as_ref(), b"1");

        let mut left = Xxh3_64::new();
        std::hash::Hash::hash(&borrowed, &mut left);
        let mut right = Xxh3_64::new();
        std::hash::Hash::hash(&inline, &mut right);
        assert_eq!(left.finish(), right.finish());
    }

    #[test]
    fn a_record_feeds_its_fields_in_sorted_name_order() {
        // The stored map is sorted, so two records built in different orders
        // are one value and feed one way.
        let left = Scalar::from_record([("b", Scalar::I64(2)), ("a", Scalar::I64(1))]).unwrap();
        let right = Scalar::from_record([("a", Scalar::I64(1)), ("b", Scalar::I64(2))]).unwrap();
        assert_eq!(feed(&left), feed(&right));

        // A mapping is insertion-ordered, so its order is part of the value.
        let left = Scalar::from_mapping([
            (Scalar::from("b"), Scalar::I64(2)),
            (Scalar::from("a"), Scalar::I64(1)),
        ])
        .unwrap();
        let right = Scalar::from_mapping([
            (Scalar::from("a"), Scalar::I64(1)),
            (Scalar::from("b"), Scalar::I64(2)),
        ])
        .unwrap();
        assert_ne!(left, right);
        assert_ne!(feed(&left), feed(&right));
    }

    #[test]
    fn a_record_name_cannot_be_confused_with_its_value() {
        // Names are length-prefixed, so a field named "ab" with value "" and a
        // field named "a" with value "b" are different feeds.
        let left = Scalar::from_record([("ab", Scalar::from(""))]).unwrap();
        let right = Scalar::from_record([("a", Scalar::from("b"))]).unwrap();
        assert_ne!(feed(&left), feed(&right));
    }
}
