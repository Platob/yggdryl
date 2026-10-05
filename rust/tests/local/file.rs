//! `rust/src/local/file.rs`: the memory-mapped local file - what it creates,
//! what it resizes, what it publishes, and what a reader mapping it keeps.

mod local {
    mod mapped {
        use std::path::{Path, PathBuf};

        use yggdryl::IOBase;
        use yggdryl::local::{LocalFile, LocalFolder};

        fn path(label: &str) -> std::path::PathBuf {
            let mut path = LocalFolder::temporary().unwrap().path().unwrap();
            path.push(format!("yggdryl-mmap-{label}-{}.bin", std::process::id()));
            // Teardown through the abstraction: absence is a no-op success.
            LocalFile::new(&path)
                .expect("a local leaf")
                .remove(false)
                .expect("a removable leaf");
            path
        }

        #[test]
        fn a_mapped_file_round_trips_and_resizes() {
            let path = path("roundtrip");
            {
                let mut mapped = LocalFile::create(&path).unwrap();
                assert!(mapped.is_empty());

                mapped.pwrite(0, b"trade").unwrap();
                assert_eq!(mapped.size(), 5);

                // Growing past the mapping remaps rather than failing.
                let large = vec![7_u8; 256 * 1024];
                mapped.append_bytes(&large).unwrap();
                assert_eq!(mapped.size(), 5 + large.len() as u64);
                assert!(mapped.capacity() >= mapped.size());
                mapped.flush().unwrap();
            }

            // The on-disk length is the logical size, not the mapping capacity.
            assert_eq!(std::fs::metadata(&path).unwrap().len(), 5 + 256 * 1024);

            // Reopening sees the same bytes.
            let reopened = LocalFile::new(&path).unwrap();
            assert_eq!(reopened.size(), 5 + 256 * 1024);
            assert_eq!(reopened.read_range_bytes(0, 5).unwrap(), b"trade");
            assert_eq!(reopened.url().unwrap().extension(), Some("bin"));

            // Teardown through the abstraction: absence is a no-op success.
            LocalFile::new(&path)
                .expect("a local leaf")
                .remove(false)
                .expect("a removable leaf");
        }

        #[test]
        fn an_append_is_published_when_it_returns() {
            let path = path("append");
            let mut mapped = LocalFile::new(&path).unwrap();
            assert_eq!(mapped.append_bytes(b"abc").unwrap(), 0);
            assert_eq!(mapped.append_bytes(b"def").unwrap(), 3);

            // With the handle still open and nothing flushed, the file holds
            // exactly what was appended - never the mapping's growth slack.
            assert_eq!(std::fs::metadata(&path).unwrap().len(), 6);
            assert_eq!(
                LocalFile::new(&path).unwrap().read_all_bytes().unwrap(),
                b"abcdef"
            );
            mapped.remove(false).unwrap();
        }

        #[test]
        fn a_handle_for_a_missing_file_touches_nothing() {
            let path = path("lazy");
            assert!(!path.exists());

            let handle = LocalFile::new(&path).unwrap();
            // Constructing the handle must not create the file.
            assert!(!path.exists());
            assert!(!handle.exists());

            // A read of a missing file is empty, not an error.
            assert_eq!(handle.size(), 0);
            assert!(handle.is_empty());
            let mut probe = [0_u8; 8];
            assert_eq!(handle.pread(0, &mut probe).unwrap(), 0);
            assert!(handle.read_all_bytes().unwrap().is_empty());
            // Reading still did not create it.
            assert!(!path.exists());

            // The media type comes from the name, which exists even when the file
            // does not.
            assert_eq!(handle.url().unwrap().extension(), Some("bin"));
        }

        #[test]
        fn the_first_write_creates_the_file_and_its_parent() {
            let mut path = LocalFolder::temporary().unwrap().path().unwrap();
            path.push(format!("yggdryl-mmap-create-{}", std::process::id()));
            LocalFolder::new(&path)
                .expect("a local container")
                .remove(true)
                .expect("a removable tree");
            path.push("nested");
            path.push("trades.bin");

            let mut handle = LocalFile::new(&path).unwrap();
            assert!(!path.exists());

            handle.pwrite(0, b"created").unwrap();
            handle.flush().unwrap();

            assert!(path.exists());
            assert_eq!(handle.read_all_bytes().unwrap(), b"created");

            let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
        }

        #[test]
        fn a_write_into_a_missing_ancestry_creates_it_from_the_write_itself() {
            // No `mkdir` step runs first: the open fails with the typed absence,
            // the ancestry is repaired once, and the same open is retried once.
            let mut root = LocalFolder::temporary().unwrap().path().unwrap();
            root.push(format!("yggdryl-ancestry-{}", std::process::id()));
            yggdryl::local::LocalFolder::new(&root)
                .expect("a local folder")
                .remove(true)
                .expect("a removable folder");

            let deep = root.join("a").join("b").join("c").join("trades.bin");
            let mut leaf = LocalFile::new(&deep).expect("a local leaf");
            leaf.write_all_bytes(b"rows").expect("a created ancestry");

            assert_eq!(
                LocalFile::new(&deep)
                    .expect("a local leaf")
                    .read_all_bytes()
                    .expect("the written bytes"),
                b"rows"
            );

            yggdryl::local::LocalFolder::new(&root)
                .expect("a local folder")
                .remove(true)
                .expect("a removable folder");
        }

        #[test]
        fn a_read_of_a_missing_file_is_empty_rather_than_an_absence() {
            // The open *is* the existence question; nothing probes before it, and
            // a read that finds nothing is emptiness rather than a failure.
            let path = path("absent-read");
            let leaf = LocalFile::new(&path).expect("a local leaf");
            assert_eq!(leaf.size(), 0);
            assert!(leaf.read_all_bytes().expect("an empty read").is_empty());
        }

        #[test]
        fn a_complete_write_publishes_its_length_to_another_handle() {
            let path = path("published");

            let mut writer = LocalFile::new(&path).unwrap();
            writer.write_all_bytes(b"one\ntwo\n").unwrap();

            // The geometric growth is this handle's working state, not the value:
            // a second handle - or another process - must see the logical length,
            // or it would read the mapping's zero padding as content.
            assert_eq!(std::fs::metadata(&path).unwrap().len(), 8);
            let second = LocalFile::new(&path).unwrap();
            assert_eq!(second.size(), 8);
            assert_eq!(second.read_all_bytes().unwrap(), b"one\ntwo\n");

            drop(writer);
            // Teardown through the abstraction: absence is a no-op success.
            LocalFile::new(&path)
                .expect("a local leaf")
                .remove(false)
                .expect("a removable leaf");
        }

        #[cfg(unix)]
        #[test]
        fn closing_a_read_only_mapping_does_not_restore_replaced_bytes() {
            let path = path("read-only-close");
            std::fs::write(&path, b"before").unwrap();

            let mut reader = LocalFile::new(&path).unwrap();
            reader.open().unwrap();
            assert_eq!(reader.read_all_bytes().unwrap(), b"before");

            std::fs::write(&path, b"after!").unwrap();
            reader.close().unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), b"after!");

            LocalFile::new(&path).unwrap().remove(false).unwrap();
        }

        #[test]
        fn a_create_repairs_a_missing_parent_and_the_handle_keeps_what_it_created() {
            let mut root = LocalFolder::temporary().unwrap().path().unwrap();
            root.push(format!("yggdryl-mmap-create-bytes-{}", std::process::id()));
            LocalFolder::new(&root).unwrap().remove(true).unwrap();
            let path = root.join("nested").join("value.bin");

            let mut mapped = LocalFile::new(&path).unwrap();
            mapped.create_bytes(b"trade").unwrap();
            // The descriptor the exclusive open answered is the handle's now,
            // and the file is exactly the value: nothing is left to publish.
            assert!(mapped.opened());
            assert_eq!(mapped.size(), 5);
            assert_eq!(std::fs::metadata(&path).unwrap().len(), 5);
            mapped.append_bytes(b"!").unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), b"trade!");

            // The same handle creating again asks the path, and loses.
            let error = mapped.create_bytes(b"other").unwrap_err();
            assert!(error.is_conflict(), "{error}");
            assert_eq!(mapped.read_all_bytes().unwrap(), b"trade!");

            drop(mapped);
            LocalFolder::new(&root).unwrap().remove(true).unwrap();
        }

        /// A fresh folder of the test's own, so what a replace leaves beside
        /// its file is the folder's whole listing.
        fn folder(label: &str) -> PathBuf {
            let mut root = LocalFolder::temporary().unwrap().path().unwrap();
            root.push(format!("yggdryl-mmap-{label}-{}", std::process::id()));
            LocalFolder::new(&root).unwrap().remove(true).unwrap();
            std::fs::create_dir_all(&root).unwrap();
            root
        }

        /// Every name in `root`, private ones included, in order.
        fn entries(root: &Path) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(root)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }

        /// Four pages and a tail, so a mapping of it is touched well past
        /// the end of any shorter value.
        #[cfg(unix)]
        fn pages() -> Vec<u8> {
            (0..=250_u8).cycle().take(4 * 4096 + 17).collect()
        }

        #[cfg(unix)]
        #[test]
        fn a_mapped_reader_keeps_the_value_a_whole_write_replaced() {
            let root = folder("replaced");
            let path = root.join("value.bin");
            let old = pages();
            LocalFile::new(&path)
                .unwrap()
                .write_all_bytes(&old)
                .unwrap();

            let reader = LocalFile::new(&path).unwrap();
            assert_eq!(reader.read_all_bytes().unwrap(), old);

            let mut writer = LocalFile::new(&path).unwrap();
            writer.write_all_bytes(b"short").unwrap();

            // The reader maps the file the rename took the name from, which
            // nothing shortened: every page of it is still there.
            assert_eq!(reader.read_all_bytes().unwrap(), old);
            assert_eq!(
                reader.read_range_bytes(4 * 4096, 17).unwrap(),
                &old[4 * 4096..]
            );
            // A handle opened after the rename reads the new value, and the
            // writer holds the file it wrote, nothing left to publish.
            assert_eq!(
                LocalFile::new(&path).unwrap().read_all_bytes().unwrap(),
                b"short"
            );
            assert!(writer.opened());
            assert_eq!(writer.size(), 5);
            assert_eq!(writer.path(), path);
            assert_eq!(std::fs::metadata(&path).unwrap().len(), 5);
            writer.append_bytes(b"!").unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), b"short!");
            // The sibling was renamed, not copied: nothing is left beside it.
            assert_eq!(entries(&root), ["value.bin"]);

            drop((reader, writer));
            LocalFolder::new(&root).unwrap().remove(true).unwrap();
        }

        #[cfg(unix)]
        #[test]
        fn a_mapped_reader_keeps_the_value_a_clear_replaced() {
            let root = folder("cleared");
            let path = root.join("value.bin");
            let old = pages();
            LocalFile::new(&path)
                .unwrap()
                .write_all_bytes(&old)
                .unwrap();

            let reader = LocalFile::new(&path).unwrap();
            assert_eq!(reader.read_all_bytes().unwrap(), old);

            let mut clearing = LocalFile::new(&path).unwrap();
            clearing.clear().unwrap();

            assert_eq!(reader.read_all_bytes().unwrap(), old);
            // Emptied, not deleted, and empty to a handle opened after.
            assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
            assert!(
                LocalFile::new(&path)
                    .unwrap()
                    .read_all_bytes()
                    .unwrap()
                    .is_empty()
            );
            // Clearing what is not there creates nothing.
            let absent = root.join("absent.bin");
            LocalFile::new(&absent).unwrap().clear().unwrap();
            assert!(!absent.exists());
            assert_eq!(entries(&root), ["value.bin"]);

            drop((reader, clearing));
            LocalFolder::new(&root).unwrap().remove(true).unwrap();
        }

        #[cfg(unix)]
        #[test]
        fn a_located_write_reaches_the_replace_a_mapped_reader_survives() {
            use yggdryl::local::LocalPath;

            let root = folder("located");
            let path = root.join("value.bin");
            let old = pages();
            // The location role, as a URL or a folder's child resolves it: its
            // whole write is the leaf's replace, not a resize in place.
            let mut located = LocalPath::new(&path).unwrap();
            located.write_all_bytes(&old).unwrap();

            let reader = LocalPath::new(&path).unwrap();
            assert_eq!(reader.read_all_bytes().unwrap(), old);

            located.write_all_bytes(b"short").unwrap();

            assert_eq!(reader.read_all_bytes().unwrap(), old);
            assert_eq!(
                LocalPath::new(&path).unwrap().read_all_bytes().unwrap(),
                b"short"
            );
            assert_eq!(located.size(), 5);
            assert_eq!(entries(&root), ["value.bin"]);

            drop((reader, located));
            LocalFolder::new(&root).unwrap().remove(true).unwrap();
        }

        #[cfg(unix)]
        #[test]
        fn a_replaced_file_keeps_its_permission_mode() {
            use std::os::unix::fs::PermissionsExt as _;

            let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o7777;
            let root = folder("mode");
            let path = root.join("secret.json");
            std::fs::write(&path, b"{}").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

            let mut handle = LocalFile::new(&path).unwrap();
            handle.write_all_bytes(b"{\"v\":1}").unwrap();
            assert_eq!(mode(&path), 0o600);
            handle.clear().unwrap();
            assert_eq!(mode(&path), 0o600);

            // Where no file was there, the default mode a plain create takes.
            let replaced = root.join("fresh.json");
            LocalFile::new(&replaced)
                .unwrap()
                .write_all_bytes(b"{}")
                .unwrap();
            let created = root.join("created.json");
            std::fs::write(&created, b"{}").unwrap();
            assert_eq!(mode(&replaced), mode(&created));

            drop(handle);
            LocalFolder::new(&root).unwrap().remove(true).unwrap();
        }

        #[cfg(unix)]
        #[test]
        fn a_handle_writes_into_the_file_another_handle_replaced() {
            let root = folder("followed");
            let path = root.join("value.log");
            let mut first = LocalFile::new(&path).unwrap();
            first.write_all_bytes(b"first\n").unwrap();
            let mut second = LocalFile::new(&path).unwrap();
            second.write_all_bytes(b"second\n").unwrap();

            // The first handle's file was replaced under it: its next write
            // asks the path which file it names and writes there, as an
            // in-place write would have, rather than into the replaced one.
            assert_eq!(first.append_bytes(b"third\n").unwrap(), 7);
            assert_eq!(std::fs::read(&path).unwrap(), b"second\nthird\n");
            first.pwrite(0, b"S").unwrap();
            first.flush().unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), b"Second\nthird\n");

            // A truncation follows the same way.
            second.write_all_bytes(b"fourth\n").unwrap();
            first.truncate(3).unwrap();
            first.flush().unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), b"fou");

            drop((first, second));
            LocalFolder::new(&root).unwrap().remove(true).unwrap();
        }

        #[cfg(unix)]
        #[test]
        fn a_replace_that_fails_leaves_the_handle_as_it_was() {
            let root = folder("restored");
            let path = root.join("value.bin");
            let mut handle = LocalFile::new(&path).unwrap();
            handle.pwrite(0, b"staged").unwrap();

            // A directory takes the file's place, so the rename is refused:
            // the handle still holds what it staged, its length and its bytes.
            std::fs::remove_file(&path).unwrap();
            std::fs::create_dir(&path).unwrap();
            assert!(handle.write_all_bytes(b"other").is_err());
            assert_eq!(handle.size(), 6);
            assert_eq!(handle.read_all_bytes().unwrap(), b"staged");
            assert_eq!(entries(&root), ["value.bin"]);

            drop(handle);
            LocalFolder::new(&root).unwrap().remove(true).unwrap();
        }

        #[cfg(unix)]
        #[test]
        fn a_whole_write_through_a_symbolic_link_writes_its_target() {
            let root = folder("linked");
            let target = root.join("target.bin");
            let link = root.join("link.bin");
            std::fs::write(&target, b"old").unwrap();
            std::os::unix::fs::symlink("target.bin", &link).unwrap();
            let is_link = |path: &Path| {
                std::fs::symlink_metadata(path)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            };

            let mut handle = LocalFile::new(&link).unwrap();
            handle.write_all_bytes(b"new").unwrap();
            assert!(is_link(&link));
            assert_eq!(std::fs::read(&target).unwrap(), b"new");
            handle.clear().unwrap();
            assert!(is_link(&link));
            assert!(std::fs::read(&target).unwrap().is_empty());

            // A link to nothing yet: the write creates what it names.
            let dangling = root.join("dangling.bin");
            std::os::unix::fs::symlink("named.bin", &dangling).unwrap();
            LocalFile::new(&dangling)
                .unwrap()
                .write_all_bytes(b"named")
                .unwrap();
            assert!(is_link(&dangling));
            assert_eq!(std::fs::read(root.join("named.bin")).unwrap(), b"named");
            assert_eq!(
                entries(&root),
                ["dangling.bin", "link.bin", "named.bin", "target.bin"]
            );

            drop(handle);
            LocalFolder::new(&root).unwrap().remove(true).unwrap();
        }

        #[test]
        fn a_create_is_whole_or_absent_to_a_reader_and_leaves_no_sibling() {
            use std::sync::Arc;
            use std::sync::atomic::{AtomicBool, Ordering};

            let root = folder("created");
            let path = root.join("v1.metadata.json");
            let payload: Arc<Vec<u8>> = Arc::new((0..=250_u8).cycle().take(8 << 20).collect());
            let done = Arc::new(AtomicBool::new(false));
            let reader = {
                let (path, payload, done) = (path.clone(), Arc::clone(&payload), Arc::clone(&done));
                std::thread::spawn(move || {
                    let mut seen = 0_usize;
                    while !done.load(Ordering::Acquire) {
                        match std::fs::read(&path) {
                            Ok(bytes) => {
                                assert!(
                                    bytes == *payload,
                                    "a read of {} bytes is no create whole",
                                    bytes.len()
                                );
                                seen += 1;
                            }
                            Err(error) => {
                                assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
                            }
                        }
                    }
                    seen
                })
            };
            for _ in 0..8 {
                LocalFile::new(&path).unwrap().remove(false).unwrap();
                LocalFile::new(&path)
                    .unwrap()
                    .create_bytes(&payload)
                    .unwrap();
            }
            done.store(true, Ordering::Release);
            reader.join().unwrap();

            // The second creator loses, and the first bytes stand.
            let error = LocalFile::new(&path)
                .unwrap()
                .create_bytes(b"{}")
                .unwrap_err();
            assert!(error.is_conflict(), "{error}");
            assert!(std::fs::read(&path).unwrap() == *payload);
            assert_eq!(entries(&root), ["v1.metadata.json"]);
            LocalFolder::new(&root).unwrap().remove(true).unwrap();
        }

        /// Eight writers, each a handle of its own on one path, replace the
        /// value fifty times each - `write` given the handle, its payload and
        /// the round - while two readers, each a handle of its own, read it
        /// whole twice over one mapping and then close it to map afresh.
        /// Every read answers one payload whole or the empty value; the
        /// process faulting on a shortened mapping would end the test run.
        #[cfg(unix)]
        fn hammer(label: &str, write: fn(&mut LocalFile, &[u8], usize)) {
            use std::sync::atomic::{AtomicBool, Ordering};
            use std::sync::{Arc, Barrier};

            const WRITERS: usize = 8;
            const ROUNDS: usize = 50;
            let root = folder(label);
            let path = root.join("value.bin");
            // Each over two pages and of its own length, so a reader that
            // mapped a longer one touches pages a shorter one would drop.
            let payloads: Arc<Vec<Vec<u8>>> = Arc::new(
                (0..WRITERS)
                    .zip(b'a'..)
                    .map(|(writer, fill)| vec![fill; (writer + 1) * 9_000 + writer])
                    .collect(),
            );
            let start = Arc::new(Barrier::new(WRITERS + 2));
            let done = Arc::new(AtomicBool::new(false));

            let readers: Vec<_> = (0..2)
                .map(|_| {
                    let (path, payloads) = (path.clone(), Arc::clone(&payloads));
                    let (start, done) = (Arc::clone(&start), Arc::clone(&done));
                    std::thread::spawn(move || {
                        let mut reader = LocalFile::new(&path).unwrap();
                        start.wait();
                        let mut reads = 0_usize;
                        while !done.load(Ordering::Acquire) {
                            let value = reader.read_all_bytes().unwrap();
                            assert!(
                                value.is_empty() || payloads.contains(&value),
                                "a read of {} bytes is no payload whole",
                                value.len()
                            );
                            // One mapping answers one value, however the
                            // path moved meanwhile; a read that found no file
                            // yet holds none.
                            if reader.opened() {
                                assert!(reader.read_all_bytes().unwrap() == value);
                            }
                            reader.close().unwrap();
                            reads += 1;
                        }
                        reads
                    })
                })
                .collect();
            let writers: Vec<_> = (0..WRITERS)
                .map(|writer| {
                    let (path, payloads) = (path.clone(), Arc::clone(&payloads));
                    let start = Arc::clone(&start);
                    std::thread::spawn(move || {
                        let mut handle = LocalFile::new(&path).unwrap();
                        start.wait();
                        for round in 0..ROUNDS {
                            write(&mut handle, &payloads[writer], round);
                        }
                    })
                })
                .collect();
            for writer in writers {
                writer.join().unwrap();
            }
            done.store(true, Ordering::Release);
            let reads: usize = readers
                .into_iter()
                .map(|reader| reader.join().unwrap())
                .sum();
            assert!(reads > 0);

            let last = LocalFile::new(&path).unwrap().read_all_bytes().unwrap();
            assert!(last.is_empty() || payloads.contains(&last));
            // Every sibling was renamed over the path: none is left beside it.
            assert_eq!(entries(&root), ["value.bin"]);
            LocalFolder::new(&root).unwrap().remove(true).unwrap();
        }

        #[cfg(unix)]
        #[test]
        fn racing_whole_writes_never_fault_a_mapped_reader() {
            hammer("racing-writes", |handle, payload, _| {
                handle.write_all_bytes(payload).unwrap();
            });
        }

        #[cfg(unix)]
        #[test]
        fn racing_clears_never_fault_a_mapped_reader() {
            hammer("racing-clears", |handle, payload, round| {
                if round % 2 == 0 {
                    handle.write_all_bytes(payload).unwrap();
                } else {
                    handle.clear().unwrap();
                }
            });
        }

        #[test]
        fn a_replace_that_fails_leaves_no_sibling_behind() {
            let root = folder("refused");
            // A directory where the file goes: the sibling is created and
            // written, and the rename over the directory is refused.
            let path = root.join("value.bin");
            std::fs::create_dir(&path).unwrap();

            let mut handle = LocalFile::new(&path).unwrap();
            assert!(handle.write_all_bytes(b"rows").is_err());
            assert!(handle.clear().is_err());
            assert!(path.is_dir());
            assert_eq!(entries(&root), ["value.bin"]);

            drop(handle);
            LocalFolder::new(&root).unwrap().remove(true).unwrap();
        }

        #[test]
        fn a_mapped_file_zero_fills_a_write_gap() {
            let path = path("gap");
            let mut mapped = LocalFile::create(&path).unwrap();
            mapped.pwrite(0, b"ab").unwrap();
            mapped.pwrite(5, b"z").unwrap();

            assert_eq!(mapped.size(), 6);
            assert_eq!(mapped.read_all_bytes().unwrap(), b"ab\0\0\0z");

            drop(mapped);
            // Teardown through the abstraction: absence is a no-op success.
            LocalFile::new(&path)
                .expect("a local leaf")
                .remove(false)
                .expect("a removable leaf");
        }
    }
}
