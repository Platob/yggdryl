//! `rust/src/fs/folder.rs`: a directory over one bound location - its listings,
//! its globs, and what deleting its contents takes.

mod fs {

    use std::sync::Arc;
    use yggdryl::fs::*;
    use yggdryl::{Error, IOBase, Result};

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

    #[test]
    fn bound_glob_preserves_repeated_separator_paths() {
        let memory = MemoryFileSystem::new();
        memory.create_dir("root/foo/", true).unwrap();
        write(&memory, "root/foo//bar.txt", b"value").unwrap();
        let filesystem: Arc<dyn FileSystem> = Arc::new(memory);
        let root = Folder::from_path(filesystem, "root", None).unwrap();

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
        let non_root = Folder::new(
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
        let root = Folder::new(BoundLocation::new(bound_filesystem, "", None::<String>).unwrap());
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
