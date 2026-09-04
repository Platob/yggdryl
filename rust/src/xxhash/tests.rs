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
fn a_custom_secret_changes_the_answer() {
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
