//! `rust/src/local/mod.rs`: the well-known roots a local tree is reached by.
//!
//! `LocalFolder::home_from` takes the two environment variables directly, which is
//! the only way to name a home without touching the developer's real one, and
//! a caller has no name for it, so it is reached through `yggdryl::internals`.
//! Everything a caller can observe lives beside it, one file per role.
//!
//! The well-known roots: handles over what the platform and the environment
//! report, none of which creates anything.

#[cfg(feature = "internals")]
mod internal {
    use std::ffi::OsString;

    use yggdryl::IOBase;
    use yggdryl::internals::local::home_from;
    use yggdryl::local::LocalFolder;

    fn set(text: &str) -> Option<OsString> {
        Some(OsString::from(text))
    }

    /// Two distinct candidate homes under the temporary root, so a wrong pick
    /// is visible and neither is the developer's real home.
    fn candidates() -> (std::path::PathBuf, std::path::PathBuf) {
        let root = LocalFolder::temporary().unwrap().path().unwrap();
        (root.join("yggdryl-home"), root.join("yggdryl-profile"))
    }

    #[test]
    fn the_temporary_root_is_a_local_container() {
        let temporary = LocalFolder::temporary().unwrap();
        assert!(temporary.is_container());
        assert!(temporary.url().is_local());
        // The platform's own directory is there; nothing here made it.
        assert!(temporary.exists());
    }

    #[test]
    fn home_prefers_home_over_userprofile() {
        let (home, profile) = candidates();
        let resolved =
            home_from(set(home.to_str().unwrap()), set(profile.to_str().unwrap())).unwrap();
        assert_eq!(resolved.url(), LocalFolder::new(&home).unwrap().url());
    }

    #[test]
    fn home_alone_resolves() {
        let (home, _) = candidates();
        let resolved = home_from(set(home.to_str().unwrap()), None).unwrap();
        assert_eq!(resolved.url(), LocalFolder::new(&home).unwrap().url());
    }

    #[test]
    fn userprofile_alone_resolves() {
        let (_, profile) = candidates();
        let resolved = home_from(None, set(profile.to_str().unwrap())).unwrap();
        assert_eq!(resolved.url(), LocalFolder::new(&profile).unwrap().url());
    }

    #[test]
    fn an_empty_value_counts_as_unset() {
        let (_, profile) = candidates();
        let resolved = home_from(set(""), set(profile.to_str().unwrap())).unwrap();
        assert_eq!(resolved.url(), LocalFolder::new(&profile).unwrap().url());
        assert!(home_from(set(""), set("")).unwrap_err().is_absent());
    }

    #[test]
    fn neither_variable_is_a_typed_absence_naming_both() {
        let error = home_from(None, None).unwrap_err();
        assert!(error.is_absent());
        let message = error.to_string();
        assert!(message.contains("HOME"), "{message}");
        assert!(message.contains("USERPROFILE"), "{message}");
    }

    #[test]
    fn a_resolved_home_creates_nothing() {
        let (home, _) = candidates();
        let _ = std::fs::remove_dir_all(&home);
        let resolved = home_from(set(home.to_str().unwrap()), None).unwrap();
        assert!(!resolved.exists());
        assert!(!home.exists());
    }
}

/// The three roles are what a backend implements; `local` is the reference.
mod roles {
    use yggdryl::local::{LocalFile, LocalFolder, LocalPath};
    use yggdryl::{IOBase, IOFile, IOFolder, IOPath};
    use yggdryl::{IOKind, MimeType};

    fn root(label: &str) -> std::path::PathBuf {
        let mut path = LocalFolder::temporary().unwrap().path().unwrap();
        path.push(format!("yggdryl-roles-{label}-{}", std::process::id()));
        LocalFolder::new(&path)
            .expect("a local container")
            .remove(true)
            .expect("a removable tree");
        path
    }

    #[test]
    fn the_folder_role_supplies_the_byte_half_of_the_contract() {
        let path = root("folder");
        let mut folder = LocalFolder::new(&path).unwrap();

        // A container holds no bytes, refuses byte writes, and is created by
        // truncating it to zero - all of that comes from the role.
        assert_eq!(folder.size(), 0);
        assert!(folder.read_all_bytes().unwrap().is_empty());
        let message = folder.pwrite(0, b"x").unwrap_err().to_string();
        assert!(message.contains("got the directory"), "{message}");
        assert!(folder.truncate(4).is_err());

        assert!(!folder.folder_exists());
        folder.truncate(0).unwrap();
        assert!(folder.folder_exists());
        assert_eq!(folder.kind(), IOKind::Directory);
        assert_eq!(folder.media_type().base(), &MimeType::DIRECTORY);

        LocalFolder::new(&path)
            .expect("a local container")
            .remove(true)
            .expect("a removable tree");
    }

    #[test]
    fn the_file_role_supplies_the_container_half_of_the_contract() {
        let path = root("file");
        std::fs::create_dir_all(&path).unwrap();
        let leaf = LocalFile::new(path.join("trades.bin")).unwrap();

        // A leaf contains nothing and resolves no children.
        assert!(leaf.file_ls().count() == 0);
        let message = leaf.file_child_by_path("child").unwrap_err().to_string();
        assert!(message.contains("got the file"), "{message}");

        // Its kind follows from whether it exists yet.
        assert_eq!(leaf.file_kind(), IOKind::Unknown);

        LocalFolder::new(&path)
            .expect("a local container")
            .remove(true)
            .expect("a removable tree");
    }

    #[test]
    fn the_path_role_answers_by_looking() {
        let path = root("path");
        std::fs::create_dir_all(path.join("nested")).unwrap();
        std::fs::write(path.join("a.bin"), b"a").unwrap();

        let folder = LocalPath::new(&path).unwrap();
        assert!(folder.is_folder());
        assert!(!folder.is_file());
        assert_eq!(folder.path_kind(), IOKind::Directory);
        assert_eq!(folder.path_media_type().base(), &MimeType::DIRECTORY);

        let leaf = LocalPath::new(path.join("a.bin")).unwrap();
        assert!(leaf.is_file());
        assert_eq!(leaf.path_kind(), IOKind::File);

        let absent = LocalPath::new(path.join("absent.arrows")).unwrap();
        assert!(!absent.path_exists());
        assert_eq!(absent.path_kind(), IOKind::Unknown);
        // An undecided location still reports what its name says it holds.
        assert_eq!(absent.path_media_type().base(), &MimeType::ARROW_STREAM);

        LocalFolder::new(&path)
            .expect("a local container")
            .remove(true)
            .expect("a removable tree");
    }
}
