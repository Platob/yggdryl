//! `rust/src/fs/folder.rs`: a directory over one bound location - its listings,
//! its globs, the stream of the files beneath it, and what deleting its
//! contents takes.

mod fs {

    use std::sync::Arc;
    use yggdryl::fs::*;
    use yggdryl::{ByteStream, Codec, Error, IOBase, Result};

    fn write(filesystem: &dyn FileSystem, path: &str, bytes: &[u8]) -> Result<()> {
        let mut writer = filesystem.open_output_stream(path, None)?;
        let mut offset = 0;
        while offset < bytes.len() {
            let written = writer.write(&bytes[offset..])?;
            if written == 0 {
                return Err(Error::Io(std::io::Error::from(
                    std::io::ErrorKind::WriteZero,
                )));
            }
            offset += written;
        }
        writer.close()
    }

    /// A folder at `logs` over a fresh memory filesystem holding `files`, each
    /// a path under the filesystem root and its bytes.
    fn logs(files: &[(&str, &[u8])]) -> FsFolder {
        let memory = MemoryFileSystem::new();
        memory.create_dir("logs", true).unwrap();
        for (path, bytes) in files {
            if let Some((parent, _)) = path.rsplit_once('/') {
                memory.create_dir(parent, true).unwrap();
            }
            write(&memory, path, bytes).unwrap();
        }
        FsFolder::from_path(Arc::new(memory), "logs", None).unwrap()
    }

    /// Every chunk a stream yields, failing on the first error.
    fn chunks(stream: ByteStream<'_>) -> Vec<Vec<u8>> {
        stream.collect::<Result<Vec<_>>>().unwrap()
    }

    /// Three files at two depths, one of them without a final newline, one
    /// empty, and one hidden: what a stream of the folder must pass over and
    /// what it must run together.
    const TREE: &[(&str, &[u8])] = &[
        ("logs/a.log", b"a1\na2\n"),
        ("logs/b.log", b"b1\nb2"),
        ("logs/empty.log", b""),
        ("logs/.hidden", b"h\n"),
        ("logs/sub/c.log", b"c1\n"),
        ("logs/sub/.private/d.log", b"d1\n"),
    ];

    /// The files of [`TREE`] a stream reads, in listing order, end to end.
    const STREAMED: &[u8] = b"a1\na2\nb1\nb2c1\n";

    #[test]
    fn every_listed_and_globbed_location_names_the_host_its_folder_does() {
        let folder = logs(TREE);
        let host = folder.url().hostname().map(str::to_owned);
        assert_eq!(host.as_deref(), Some("localhost"));
        let listed = folder.ls(true, true).collect::<Result<Vec<_>>>().unwrap();
        let globbed = folder
            .glob("**/*.log", false)
            .unwrap()
            .collect::<Result<Vec<_>>>()
            .unwrap();
        assert!(!listed.is_empty() && !globbed.is_empty());
        for entry in listed.iter().chain(&globbed) {
            let url = entry.url().expect("a bound entry has a URL");
            assert_eq!(url.hostname(), host.as_deref(), "{url}");
            assert!(
                url.to_string().starts_with("memory://localhost/logs/"),
                "{url}"
            );
        }
    }

    #[test]
    fn a_folder_refuses_a_stream_that_could_never_yield_a_byte() {
        let folder = logs(TREE);
        for refused in [
            folder.pstream_bytes(0, 0).unwrap_err(),
            ByteStream::from_container(&folder, 0, 0).unwrap_err(),
        ] {
            assert!(
                matches!(&refused, Error::Io(error) if error.kind() == std::io::ErrorKind::InvalidInput),
                "{refused:?}"
            );
        }
    }

    #[test]
    fn a_folder_streams_every_file_beneath_it_end_to_end() {
        let folder = logs(TREE);

        // Depth first, sorted, nothing between two files: `b.log` has no final
        // newline and runs straight into `sub/c.log`. The empty file adds
        // nothing, and neither the hidden file nor anything under a hidden
        // directory is read.
        let streamed = chunks(folder.pstream_bytes(0, 4).unwrap());
        assert_eq!(streamed.concat(), STREAMED);
        // Every chunk is full across the file boundaries but the last, and
        // none is empty.
        let sizes: Vec<usize> = streamed.iter().map(Vec::len).collect();
        assert_eq!(sizes, [4, 4, 4, 2]);

        // A whole read is that stream drained, and a ranged one is the same
        // stream from the offset.
        assert_eq!(folder.read_all_bytes().unwrap(), STREAMED);
        assert_eq!(folder.read_range_bytes(7, 5).unwrap(), &STREAMED[7..12]);
        assert_eq!(folder.read_range_bytes(12, 64).unwrap(), &STREAMED[12..]);
        assert!(folder.read_range_bytes(64, 8).unwrap().is_empty());

        // The folder itself still holds no positional bytes and is no value.
        let mut window = [0_u8; 8];
        assert_eq!(folder.pread(0, &mut window).unwrap(), 0);
        assert_eq!(folder.size(), 0);
        assert!(!folder.is_atomic());
    }

    #[test]
    fn a_folder_stream_starts_at_a_content_position_across_its_files() {
        let folder = logs(TREE);
        for position in [0, 3, 6, 10, 11, 13, 14] {
            let streamed = chunks(folder.pstream_bytes(position, 3).unwrap()).concat();
            assert_eq!(streamed, &STREAMED[position as usize..], "from {position}");
        }
        // Past the last file there is nothing left, and nothing is yielded.
        assert!(folder.pstream_bytes(64, 3).unwrap().next().is_none());
    }

    #[test]
    fn a_coded_file_contributes_its_decoded_content() {
        let coded = Codec::Gzip.dump(b"b1\nb2\n").unwrap();
        let folder = logs(&[
            ("logs/a.log", b"a1\n"),
            ("logs/b.log.gz", &coded),
            ("logs/c.log", b"c1\n"),
        ]);
        let content: &[u8] = b"a1\nb1\nb2\nc1\n";

        assert_eq!(
            chunks(folder.pstream_bytes(0, 5).unwrap()).concat(),
            content
        );
        // A position counts content bytes, so it lands inside the decoded
        // file, and past it once the decoding has counted it whole.
        for position in [4, 9, 11] {
            assert_eq!(
                chunks(folder.pstream_bytes(position, 5).unwrap()).concat(),
                &content[position as usize..],
                "from {position}"
            );
        }
        assert_eq!(folder.read_range_bytes(5, 4).unwrap(), &content[5..9]);
    }

    #[test]
    fn a_failing_file_arrives_after_the_prefix_and_the_stream_is_fused() {
        let folder = logs(&[
            ("logs/a.log", b"a1\n"),
            ("logs/b.log.gz", b"these bytes are not gzip"),
            ("logs/c.log", b"c1\n"),
        ]);

        let mut stream = folder.pstream_bytes(0, 16).unwrap();
        assert_eq!(stream.next().unwrap().unwrap(), b"a1\n");
        let refused = stream.next().unwrap().unwrap_err();
        assert!(matches!(refused, Error::Io(_)), "{refused:?}");
        // Nothing after the failure, not even the file that would follow it.
        assert!(stream.next().is_none());
        assert!(stream.next().is_none());

        assert!(folder.read_all_bytes().is_err());
    }

    #[test]
    fn a_file_its_store_cannot_size_is_read_to_count_rather_than_passed_by() {
        use crate::counting_filesystem::counted_folder;

        let (filesystem, folder) = counted_folder("logs");
        for (name, bytes) in [("a.log", &b"a1\na2\n"[..]), ("b.log", b"b1\n")] {
            folder
                .child_by_path(name)
                .unwrap()
                .write_all_bytes(bytes)
                .unwrap();
        }
        // A size of zero is what a store answers when it cannot say, so
        // passing a file by it would shift every later byte: `a.log` is read
        // to count off the position instead.
        filesystem.set_sizeless(true);
        assert_eq!(folder.size(), 0);
        assert_eq!(
            chunks(folder.pstream_bytes(4, 2).unwrap()).concat(),
            b"2\nb1\n"
        );
        assert_eq!(folder.read_range_bytes(6, 2).unwrap(), b"b1");
    }

    #[test]
    fn a_folder_holding_no_files_streams_nothing() {
        // Only a directory, a hidden file and an empty file beneath it.
        let folder = logs(&[("logs/.hidden", b"h\n"), ("logs/sub/empty.log", b"")]);
        assert!(folder.pstream_bytes(0, 4).unwrap().next().is_none());
        assert!(folder.read_all_bytes().unwrap().is_empty());

        // Nothing at the location at all is the same empty stream, under the
        // laziness contract: absence is emptiness.
        let absent = FsFolder::from_path(Arc::new(MemoryFileSystem::new()), "logs", None).unwrap();
        assert!(absent.pstream_bytes(0, 4).unwrap().next().is_none());
        assert!(absent.read_all_bytes().unwrap().is_empty());
    }

    #[test]
    fn a_folder_stream_outlives_the_handle_that_built_it() {
        let folder = logs(TREE);
        let stream = ByteStream::from_container(&folder, 3, 4).unwrap();
        drop(folder);
        assert_eq!(chunks(stream).concat(), &STREAMED[3..]);
    }

    #[test]
    fn bound_glob_preserves_repeated_separator_paths() {
        let memory = MemoryFileSystem::new();
        memory.create_dir("root/foo/", true).unwrap();
        write(&memory, "root/foo//bar.txt", b"value").unwrap();
        let filesystem: Arc<dyn FileSystem> = Arc::new(memory);
        let root = FsFolder::from_path(filesystem, "root", None).unwrap();

        let paths = root
            .glob("foo//*.txt", true)
            .unwrap()
            .map(|entry| entry.unwrap().bound_location().unwrap().path().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(paths, ["root/foo//bar.txt"]);
    }

    #[test]
    fn root_content_deletion_is_explicit_and_listing_fuses_after_error() {
        let memory = MemoryFileSystem::new();
        write(&memory, "a.bin", b"a").unwrap();
        memory.create_dir("folder", false).unwrap();
        let bound_filesystem: Arc<dyn FileSystem> = Arc::new(memory.clone());
        let non_root = FsFolder::new(
            BoundLocation::new(Arc::clone(&bound_filesystem), "folder", None::<String>).unwrap(),
        );
        assert!(
            non_root
                .delete_root_dir_contents()
                .unwrap_err()
                .is_unsupported()
        );
        let error = memory.delete_dir_contents("", false).unwrap_err();
        assert!(error.is_unsupported());
        let root = FsFolder::new(BoundLocation::new(bound_filesystem, "", None::<String>).unwrap());
        root.delete_root_dir_contents().unwrap();
        assert!(
            memory
                .list(&FileSelector::new("", false, false))
                .next()
                .is_none()
        );

        let failure = Error::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "denied",
        ));
        let mut entries = FileInfos::new(
            vec![
                Ok(FileInfo::file("a", 1, None)),
                Err(failure),
                Ok(FileInfo::file("b", 1, None)),
            ]
            .into_iter(),
        );
        assert_eq!(entries.next().unwrap().unwrap().path, "a");
        assert!(
            matches!(entries.next().unwrap(), Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::PermissionDenied)
        );
        assert!(entries.next().is_none());
    }
}
