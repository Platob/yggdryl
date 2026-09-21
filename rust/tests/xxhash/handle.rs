//! `rust/src/xxhash/handle.rs`: the transparent handle that digests bytes as
//! they are written.

mod xxhash {

    mod hashed {
        use yggdryl::IOBase;
        use yggdryl::holder::Buffer;
        use yggdryl::xxhash::{Hashed, xxh3, xxh3_with_seed};
        use yggdryl::{DigestAlgorithm, Error};

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

        impl yggdryl::IOMedia for Counted {
            yggdryl::delegate_iomedia!(handle);
        }

        impl IOBase for Counted {
            yggdryl::delegate_iobase!(handle: pwrite, size, capacity, reserve, truncate, url,
                media_type, set_media_type, flush, open, opened, close, parent, child_by_path,
                ls, kind, clear, remove, is_atomic, is_tabular, is_io);

            fn pread(&self, offset: u64, buffer: &mut [u8]) -> yggdryl::Result<usize> {
                self.reads
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                self.handle.pread(offset, buffer)
            }
        }

        #[test]
        fn sequential_writes_answer_without_reading_the_bytes_back() {
            let mut handle = Hashed::new(Counted::new(), DigestAlgorithm::Xxh3);
            handle.write_all_bytes(b"symbol,price\n").unwrap();
            handle.append_bytes(b"AAPL,187.23\n").unwrap();
            handle.append_bytes(b"MSFT,410.10\n").unwrap();
            handle.flush().unwrap();

            let expected = b"symbol,price\nAAPL,187.23\nMSFT,410.10\n";
            assert_eq!(
                handle.read_digest(DigestAlgorithm::Xxh3).unwrap().as_u64(),
                Some(xxh3(expected))
            );
            assert_eq!(handle.handle().reads(), 0, "the running state was used");
            // Asking again is still free.
            assert_eq!(
                handle.read_digest(DigestAlgorithm::Xxh3).unwrap().as_u64(),
                Some(xxh3(expected))
            );
            assert_eq!(handle.handle().reads(), 0);
            assert_eq!(handle.read_all_bytes().unwrap(), expected);
        }

        #[test]
        fn an_out_of_order_write_re_streams_to_the_same_value() {
            let mut handle = Hashed::new(Counted::new(), DigestAlgorithm::Xxh3);
            handle
                .write_all_bytes(b"symbol,price\nAAPL,187.23\n")
                .unwrap();
            assert_eq!(handle.handle().reads(), 0);

            // A write that is neither offset 0 nor the running append point.
            handle.pwrite_all(7, b"PRICE").unwrap();
            let bytes = handle.read_all_bytes().unwrap();
            let before = handle.handle().reads();
            assert_eq!(
                handle.read_digest(DigestAlgorithm::Xxh3).unwrap().as_u64(),
                Some(xxh3(&bytes))
            );
            assert!(
                handle.handle().reads() > before,
                "a stale state must re-stream"
            );
            // The state re-armed, so the next ask is free again.
            let after = handle.handle().reads();
            assert_eq!(
                handle.read_digest(DigestAlgorithm::Xxh3).unwrap().as_u64(),
                Some(xxh3(&bytes))
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
            let mut handle = Hashed::new(Buffer::new(), DigestAlgorithm::Xxh3);
            handle.write_all_bytes(b"AAPL,187.23\n").unwrap();

            handle.clear().unwrap();
            assert_eq!(
                handle.read_digest(DigestAlgorithm::Xxh3).unwrap(),
                DigestAlgorithm::Xxh3.digest(b"")
            );
            handle.write_all_bytes(b"MSFT,410.10\n").unwrap();
            assert_eq!(
                handle.read_digest(DigestAlgorithm::Xxh3).unwrap().as_u64(),
                Some(xxh3(b"MSFT,410.10\n"))
            );

            handle.remove(false).unwrap();
            assert_eq!(
                handle.read_digest(DigestAlgorithm::Xxh3).unwrap(),
                DigestAlgorithm::Xxh3.digest(b"")
            );

            handle.write_all_bytes(b"AAPL,187.23\n").unwrap();
            handle.truncate(4).unwrap();
            let bytes = handle.read_all_bytes().unwrap();
            assert_eq!(bytes, b"AAPL");
            assert_eq!(
                handle.read_digest(DigestAlgorithm::Xxh3).unwrap(),
                DigestAlgorithm::Xxh3.digest(&bytes)
            );
        }

        #[test]
        fn filesystem_stream_writes_are_visible_without_staging() {
            use std::sync::Arc;

            use yggdryl::fs::{FileSystem, Folder, MemoryFileSystem};

            // An Arrow filesystem file forwards a completed positional write
            // through one output stream rather than retaining a second payload.
            let filesystem = Arc::new(MemoryFileSystem::new());
            filesystem.create_dir("lake", false).unwrap();
            let lake = Folder::from_path(filesystem, "lake", None).unwrap();
            let leaf = lake.child_by_path("trades.csv").unwrap();
            let mut handle = Hashed::new(leaf, DigestAlgorithm::Xxh3);

            handle.pwrite_all(0, b"AAPL,187.23\n").unwrap();
            assert_eq!(
                handle.read_digest(DigestAlgorithm::Xxh3).unwrap(),
                DigestAlgorithm::Xxh3.digest(&handle.read_all_bytes().unwrap()),
            );

            // A later wrapper flush is harmless because the filesystem stream was
            // already closed after the complete write.
            handle.flush().unwrap();
            assert_eq!(
                handle.read_digest(DigestAlgorithm::Xxh3).unwrap().as_u64(),
                Some(xxh3(b"AAPL,187.23\n"))
            );
            assert_eq!(handle.read_all_bytes().unwrap(), b"AAPL,187.23\n");
        }

        #[test]
        fn a_seed_travels_with_the_wrapper() {
            let mut handle = Hashed::new(Buffer::new(), DigestAlgorithm::Xxh3).with_seed(42);
            assert_eq!(handle.seed(), 42);
            assert_eq!(handle.algorithm(), DigestAlgorithm::Xxh3);
            handle.write_all_bytes(b"AAPL,187.23\n").unwrap();
            assert_eq!(
                handle.read_digest(DigestAlgorithm::Xxh3).unwrap().as_u64(),
                Some(xxh3_with_seed(b"AAPL,187.23\n", 42))
            );

            // Re-streaming re-arms under the same seed, not an unseeded state.
            handle.pwrite_all(4, b"!").unwrap();
            let bytes = handle.read_all_bytes().unwrap();
            assert_eq!(
                handle.read_digest(DigestAlgorithm::Xxh3).unwrap().as_u64(),
                Some(xxh3_with_seed(&bytes, 42))
            );
        }

        #[test]
        fn another_algorithm_streams_rather_than_reading_the_running_state() {
            let mut handle = Hashed::new(Buffer::new(), DigestAlgorithm::Xxh3);
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
            let mut handle = Hashed::new(Buffer::new(), DigestAlgorithm::Xxh3);
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
            let root = yggdryl::local::Folder::temporary()
                .unwrap()
                .path()
                .unwrap()
                .join(format!(
                    "yggdryl-xxhash-hashed-{}-{:?}",
                    std::process::id(),
                    std::thread::current().id()
                ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            let handle = Hashed::new(
                yggdryl::local::Folder::new(&root).unwrap(),
                DigestAlgorithm::Xxh3,
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
}
