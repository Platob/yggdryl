//! `rust/src/fs/system.rs`: the complete synchronous filesystem contract, run
//! against each reference implementation.

mod fs {

    use std::io::SeekFrom;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use yggdryl::fs::*;
    use yggdryl::{Error, IOKind, Result};

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

    fn read(filesystem: &dyn FileSystem, path: &str) -> Result<Vec<u8>> {
        let mut reader = filesystem.open_input_stream(path)?;
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 3];
        loop {
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..count]);
        }
        reader.close()?;
        Ok(bytes)
    }

    fn reference_conformance(filesystem: Arc<dyn FileSystem>, root: &str) {
        filesystem.create_dir(root, true).unwrap();
        filesystem
            .create_dir(&format!("{root}/sub"), false)
            .unwrap();
        write(filesystem.as_ref(), &format!("{root}/b.bin"), b"b").unwrap();
        write(filesystem.as_ref(), &format!("{root}/a.bin"), b"012345").unwrap();
        write(filesystem.as_ref(), &format!("{root}/sub/c.bin"), b"c").unwrap();

        let info = filesystem.file_info(&format!("{root}/a.bin")).unwrap();
        assert_eq!(info.kind, IOKind::File);
        assert_eq!(info.size, Some(6));
        assert!(info.mtime_ns.is_some());
        assert_eq!(
            filesystem.file_info(&format!("{root}/missing")).unwrap(),
            FileInfo::not_found(format!("{root}/missing"))
        );

        let mut sequential = filesystem
            .open_input_stream(&format!("{root}/a.bin"))
            .unwrap();
        let mut first = [0_u8; 2];
        assert_eq!(sequential.read(&mut first).unwrap(), 2);
        assert_eq!(&first, b"01");
        assert_eq!(sequential.tell(), 2);
        sequential.close().unwrap();
        sequential.close().unwrap();
        assert!(sequential.closed());

        let mut random = filesystem
            .open_input_file(&format!("{root}/a.bin"))
            .unwrap();
        let mut middle = [0_u8; 2];
        assert_eq!(random.read_at(2, &mut middle).unwrap(), 2);
        assert_eq!(&middle, b"23");
        assert_eq!(random.tell(), 0);
        assert_eq!(random.seek(SeekFrom::End(-2)).unwrap(), 4);
        assert_eq!(random.read(&mut middle).unwrap(), 2);
        assert_eq!(&middle, b"45");
        random.close().unwrap();

        let mut append = filesystem
            .open_append_stream(&format!("{root}/a.bin"), None)
            .unwrap();
        assert_eq!(append.tell(), 6);
        assert_eq!(append.write(b"67").unwrap(), 2);
        append.flush().unwrap();
        append.close().unwrap();
        append.close().unwrap();
        assert_eq!(
            read(filesystem.as_ref(), &format!("{root}/a.bin")).unwrap(),
            b"01234567"
        );

        let mut created_by_append = filesystem
            .open_append_stream(&format!("{root}/new.bin"), None)
            .unwrap();
        assert_eq!(created_by_append.tell(), 0);
        assert_eq!(created_by_append.write(b"new").unwrap(), 3);
        created_by_append.close().unwrap();
        assert_eq!(
            read(filesystem.as_ref(), &format!("{root}/new.bin")).unwrap(),
            b"new"
        );

        let immediate = filesystem
            .list(&FileSelector::new(root, false, false))
            .collect::<Result<Vec<_>>>()
            .unwrap();
        let paths: Vec<_> = immediate.iter().map(|info| info.path.as_str()).collect();
        let listed_root = root.replace('\\', "/");
        assert_eq!(
            paths,
            [
                format!("{listed_root}/a.bin"),
                format!("{listed_root}/b.bin"),
                format!("{listed_root}/new.bin"),
                format!("{listed_root}/sub")
            ]
        );
        let recursive = filesystem
            .list(&FileSelector::new(root, true, false))
            .collect::<Result<Vec<_>>>()
            .unwrap();
        assert!(recursive.windows(2).all(|pair| pair[0].path < pair[1].path));

        assert!(
            filesystem
                .list(&FileSelector::new(format!("{root}/absent"), false, true))
                .next()
                .is_none()
        );
        let mut missing =
            filesystem.list(&FileSelector::new(format!("{root}/absent"), false, false));
        assert!(missing.next().unwrap().unwrap_err().is_absent());
        assert!(missing.next().is_none());

        let error = filesystem.delete_file(&format!("{root}/sub")).unwrap_err();
        assert!(
            matches!(error, Error::Io(error) if error.kind() == std::io::ErrorKind::IsADirectory)
        );
        let error = filesystem.delete_dir(root).unwrap_err();
        assert!(
            matches!(error, Error::Io(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty)
        );

        filesystem
            .delete_dir_contents(&format!("{root}/sub"), false)
            .unwrap();
        assert_eq!(
            filesystem.file_info(&format!("{root}/sub")).unwrap().kind,
            IOKind::Directory
        );
        filesystem.delete_dir(&format!("{root}/sub")).unwrap();
        assert_eq!(
            filesystem.file_info(&format!("{root}/sub")).unwrap().kind,
            IOKind::Unknown
        );
        filesystem.delete_file(&format!("{root}/a.bin")).unwrap();
        filesystem.delete_file(&format!("{root}/b.bin")).unwrap();
        filesystem.delete_file(&format!("{root}/new.bin")).unwrap();
        filesystem
            .create_dir(&format!("{root}/recursive"), false)
            .unwrap();
        write(
            filesystem.as_ref(),
            &format!("{root}/recursive/child.bin"),
            b"child",
        )
        .unwrap();
        filesystem
            .delete_dir_recursive(&format!("{root}/recursive"))
            .unwrap();
        assert_eq!(
            filesystem
                .file_info(&format!("{root}/recursive"))
                .unwrap()
                .kind,
            IOKind::Unknown
        );
        filesystem.delete_dir(root).unwrap();
    }

    struct TestDirectory(std::path::PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "yggdryl-fs-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn memory_implements_the_complete_reference_contract() {
        reference_conformance(Arc::new(MemoryFileSystem::new()), "suite");
    }

    #[test]
    fn local_implements_the_complete_reference_contract() {
        let temporary = TestDirectory::new();
        let root = temporary.0.join("suite").to_string_lossy().into_owned();
        reference_conformance(Arc::new(LocalFileSystem::new()), &root);
    }

    #[test]
    fn recursive_reference_listings_are_globally_sorted() {
        for filesystem in [
            Arc::new(MemoryFileSystem::new()) as Arc<dyn FileSystem>,
            Arc::new(LocalFileSystem::new()) as Arc<dyn FileSystem>,
        ] {
            let temporary = TestDirectory::new();
            let root = if filesystem.type_name() == "file" {
                temporary.0.join("order").to_string_lossy().into_owned()
            } else {
                "order".to_owned()
            };
            filesystem.create_dir(&format!("{root}/a"), true).unwrap();
            write(filesystem.as_ref(), &format!("{root}/a/z"), b"z").unwrap();
            write(filesystem.as_ref(), &format!("{root}/a-thing"), b"-").unwrap();
            write(filesystem.as_ref(), &format!("{root}/a.child"), b".").unwrap();
            let paths = filesystem
                .list(&FileSelector::new(&root, true, false))
                .map(|entry| entry.unwrap().path)
                .collect::<Vec<_>>();
            assert!(paths.windows(2).all(|pair| pair[0] < pair[1]), "{paths:?}");
        }
    }
}
