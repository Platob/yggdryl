//! `rust/src/fs/memory.rs`: the in-memory reference filesystem.

mod fs {

    use yggdryl::IOKind;
    use yggdryl::fs::*;

    #[test]
    fn memory_recursive_directory_creation_preserves_leading_and_repeated_slashes() {
        let filesystem = MemoryFileSystem::new();
        filesystem.create_dir("/a//b", true).unwrap();
        for path in ["/a", "/a/", "/a//b"] {
            assert_eq!(
                filesystem.file_info(path).unwrap().kind,
                IOKind::Directory,
                "{path}"
            );
        }
        assert_eq!(filesystem.file_info("a/b").unwrap().kind, IOKind::Unknown);
    }

    #[test]
    fn reference_filesystems_do_not_invent_output_parents() {
        let memory = MemoryFileSystem::new();
        assert!(
            memory
                .open_output_stream("missing/file.bin", None)
                .err()
                .unwrap()
                .is_absent()
        );
        assert_eq!(memory.file_info("missing").unwrap().kind, IOKind::Unknown);
    }

    #[test]
    fn memory_writer_failures_remain_visible_through_close() {
        let filesystem = MemoryFileSystem::new();
        let mut writer = filesystem.open_output_stream("lost.bin", None).unwrap();
        filesystem.delete_file("lost.bin").unwrap();
        assert!(writer.write(b"x").unwrap_err().is_absent());
        assert!(writer.close().unwrap_err().is_absent());
        assert!(writer.closed());
        assert!(writer.close().unwrap_err().is_absent());

        let mut successful = filesystem.open_output_stream("closed.bin", None).unwrap();
        successful.close().unwrap();
        successful.close().unwrap();
        assert!(successful.closed());
    }
}
