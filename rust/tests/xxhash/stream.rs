//! `rust/src/xxhash/stream.rs`: digests over a moving stream of bytes, and the
//! handles they are taken through without holding a payload whole.

mod xxhash {

    mod handles {
        use std::io::{Read as _, Write as _};

        use yggdryl::IOBase;
        use yggdryl::holder::Buffer;
        use yggdryl::xxhash::{reader, writer, xxh3};
        use yggdryl::{DigestAlgorithm, Error};

        /// A temporary root, named so parallel tests never share one.
        fn root(label: &str) -> std::path::PathBuf {
            let path = yggdryl::local::Folder::temporary()
                .unwrap()
                .path()
                .unwrap()
                .join(format!(
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
            agrees("local file", &yggdryl::local::File::new(&path).unwrap());
        }

        #[test]
        fn an_arrow_filesystem_handle_streams_the_same_digest_as_its_bytes() {
            use std::sync::Arc;

            use yggdryl::fs::{FileSystem, Folder, MemoryFileSystem};

            let filesystem = Arc::new(MemoryFileSystem::new());
            filesystem.create_dir("lake", false).unwrap();
            let lake = Folder::from_path(filesystem, "lake", None).unwrap();
            let mut leaf = lake.child_by_path("trades.csv").unwrap();
            leaf.write_all_bytes(&payload()).unwrap();
            leaf.close().unwrap();
            agrees("fs file", &lake.child_by_path("trades.csv").unwrap());
        }

        #[test]
        fn a_cache_streams_the_same_digest_and_stays_unpolluted() {
            use yggdryl::holder::buffered::{Buffered, BufferedOptions};

            let bytes = payload();
            let mut inner = Buffer::new();
            inner.write_all_bytes(&bytes).unwrap();

            // A fresh cache, asked only for a digest: the read goes through
            // `pstream_bytes`, which the cache delegates straight to the handle it
            // wraps, so hashing a large object leaves no page behind.
            let handle = Buffered::new(inner, BufferedOptions::default());
            assert_eq!(handle.cached_pages(), 0);
            assert_eq!(
                handle.read_digest(DigestAlgorithm::Xxh3).unwrap(),
                DigestAlgorithm::Xxh3.digest(&bytes)
            );
            assert_eq!(handle.cached_pages(), 0);

            // Reading bytes through the cache does populate it, and the digest
            // still agrees afterwards.
            agrees("buffered", &handle);
            assert!(handle.cached_pages() > 0);
        }

        #[test]
        fn a_coding_wrapper_digests_the_decoded_payload_and_its_handle_the_compressed_form() {
            use yggdryl::gzip::Gzip;

            let plain = payload();
            let mut handle = Gzip::new(Buffer::new());
            handle.write_all_bytes(&plain).unwrap();
            handle.flush().unwrap();

            // The wrapper answers for the bytes it presents.
            assert_eq!(
                handle.read_digest(DigestAlgorithm::Xxh3).unwrap().as_u64(),
                Some(xxh3(&plain))
            );
            // The handle underneath answers for the bytes it holds.
            let compressed = handle.handle().read_all_bytes().unwrap();
            assert_eq!(
                handle
                    .handle()
                    .read_digest(DigestAlgorithm::Xxh3)
                    .unwrap()
                    .as_u64(),
                Some(xxh3(&compressed))
            );
            assert_ne!(xxh3(&plain), xxh3(&compressed));
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
                    yggdryl::local::File::new(&missing)
                        .unwrap()
                        .read_digest(algorithm)
                        .unwrap(),
                    empty
                );
            }
        }

        #[test]
        fn a_container_is_refused_by_kind() {
            let folder = yggdryl::local::Folder::new(root("container")).unwrap();
            let error = folder.read_digest(DigestAlgorithm::Xxh3).unwrap_err();
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
                    .read_range_digest(0, 8, DigestAlgorithm::Xxh3)
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
            use yggdryl::Result;

            /// A handle whose reads fail past the first window.
            struct Failing {
                handle: Buffer,
            }

            impl yggdryl::IOMedia for Failing {
                yggdryl::delegate_iomedia!(handle);
            }

            impl IOBase for Failing {
                // Everything but `pread`, so the stream reaches the failure below
                // through the default `pstream_bytes` rather than around it.
                yggdryl::delegate_iobase!(handle: pwrite, size, capacity, reserve, truncate, url,
                    media_type, set_media_type, flush, open, opened, close, parent, child_by_path,
                    ls, kind, clear, remove, is_atomic, is_tabular, is_io);

                fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
                    if offset > 0 {
                        return Err(yggdryl::Error::Io(std::io::Error::other("backend gone")));
                    }
                    self.handle.pread(offset, buffer)
                }
            }

            let mut inner = Buffer::new();
            inner.write_all_bytes(&payload()).unwrap();
            let handle = Failing { handle: inner };
            let error = handle.read_digest(DigestAlgorithm::Xxh3).unwrap_err();
            assert!(matches!(error, Error::Io(_)), "{error}");
        }

        #[test]
        fn a_pass_through_reader_hashes_what_it_moves() {
            let bytes = payload();
            let mut source = reader(bytes.as_slice(), DigestAlgorithm::Xxh128);
            assert_eq!(source.algorithm(), DigestAlgorithm::Xxh128);
            // The digest is answerable at any point, not only at the end.
            assert_eq!(source.as_digest(), DigestAlgorithm::Xxh128.digest(b""));

            let mut moved = Vec::new();
            source.read_to_end(&mut moved).unwrap();
            assert_eq!(moved, bytes);
            assert_eq!(source.as_digest(), DigestAlgorithm::Xxh128.digest(&bytes));
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
            let mut target = writer(Vec::new(), DigestAlgorithm::Xxh3);
            target.write_all(&bytes).unwrap();
            let digest = target.as_digest();
            handle.write_all_bytes(&target.into_inner()).unwrap();
            assert_eq!(handle.read_digest(DigestAlgorithm::Xxh3).unwrap(), digest);
        }
    }
}
