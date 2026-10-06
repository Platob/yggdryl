//! `rust/src/iopath.rs`: the unresolved-location role - whether anything is
//! there, a glob location answering by what its pattern selects.

use yggdryl::holder::Holder;
use yggdryl::local::{LocalFolder, LocalPath};
use yggdryl::{IOBase, Url};

/// A fresh tree under the temporary root: two logs at the top, one a level
/// deeper, a note, and a private log no listing names.
fn tree(label: &str) -> std::path::PathBuf {
    let mut root = LocalFolder::temporary().unwrap().path().unwrap();
    root.push(format!("yggdryl-iopath-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("sub")).unwrap();
    for (name, bytes) in [
        ("a.log", &b"a\n"[..]),
        ("b.log", b"b\n"),
        ("notes.txt", b"n\n"),
        ("sub/c.log", b"c\n"),
        (".hidden.log", b"h\n"),
    ] {
        std::fs::write(root.join(name), bytes).unwrap();
    }
    root
}

/// The location `pattern` spells under `root`, a glob kept as written.
fn under(root: &std::path::Path, pattern: &str) -> LocalPath {
    LocalPath::from_url(Url::from_path(root).unwrap().joinpath(pattern).unwrap()).unwrap()
}

#[test]
fn a_pattern_exists_while_it_selects_an_entry() {
    let root = tree("pattern");

    // A glob is a container by its spelling, and there while it selects one.
    let logs = under(&root, "*.log");
    assert!(logs.is_container());
    assert!(logs.exists());
    // The door both bindings call answers the same.
    let url = Url::from_path(&root).unwrap().joinpath("*.log").unwrap();
    let held = Holder::from_url(&url, std::iter::empty::<(&str, &str)>()).unwrap();
    assert!(held.exists());

    // A pattern selecting nothing is still a container, and is not there.
    let csv = under(&root, "*.csv");
    assert!(csv.is_container());
    assert!(!csv.exists());
    assert_eq!(csv.ls(false, false).count(), 0, "what `exists` reads");

    // `**` spans the levels; a fixed prefix that is absent selects nothing,
    // and asking creates nothing.
    assert!(under(&root, "**/*.log").exists());
    assert!(!under(&root, "absent/**/*.log").exists());
    assert!(!root.join("absent").exists());

    // A private name is selected by no listing, as its stream reads nothing.
    assert!(!under(&root, ".hidden*").exists());

    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_spelled_container_exists_only_when_it_is_there() {
    let root = tree("spelled");
    let absent = root.join("lake");

    // The trailing slash settles the role and never the presence.
    let spelled = LocalPath::new(format!("{}/", absent.display())).unwrap();
    assert!(spelled.is_container());
    assert!(!spelled.exists());

    std::fs::create_dir(&absent).unwrap();
    assert!(spelled.is_container());
    assert!(spelled.exists());

    std::fs::remove_dir_all(&root).unwrap();
}
