//! `rust/src/iofolder.rs`: the container role - whether a container is
//! there, a glob location answering by what its pattern selects.

use yggdryl::local::LocalFolder;
use yggdryl::{IOBase, IOFolder, Url};

/// A fresh folder under the temporary root holding two logs and a note.
fn tree(label: &str) -> std::path::PathBuf {
    let mut root = LocalFolder::temporary().unwrap().path().unwrap();
    root.push(format!("yggdryl-iofolder-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    for name in ["a.log", "b.log", "notes.txt"] {
        std::fs::write(root.join(name), b"x\n").unwrap();
    }
    root
}

/// The folder `pattern` spells under `root`, a glob kept as written.
fn under(root: &std::path::Path, pattern: &str) -> LocalFolder {
    LocalFolder::from_url(Url::from_path(root).unwrap().joinpath(pattern).unwrap()).unwrap()
}

#[test]
fn a_folder_over_a_pattern_exists_while_it_selects_an_entry() {
    let root = tree("pattern");

    let logs = under(&root, "*.log");
    assert!(logs.is_container());
    assert!(logs.folder_exists());
    assert!(logs.exists(), "the inherent answer is the role's");
    // No directory is literally named `*.log`, so the backend's own question
    // answers `false`: the pattern is what the provided method reads.
    assert!(!logs.has_folder());

    let csv = under(&root, "*.csv");
    assert!(csv.is_container());
    assert!(!csv.folder_exists());
    assert!(!csv.exists());

    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_plain_folder_exists_as_its_backend_says() {
    let root = tree("plain");
    let folder = LocalFolder::new(root.join("made")).unwrap();

    assert!(!folder.has_folder());
    assert_eq!(folder.has_folder(), folder.folder_exists());
    folder.create().unwrap();
    assert!(folder.has_folder());
    assert_eq!(folder.has_folder(), folder.folder_exists());
    assert!(folder.exists());

    std::fs::remove_dir_all(&root).unwrap();
}
