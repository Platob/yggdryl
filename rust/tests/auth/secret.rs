//! `rust/src/auth/secret.rs`: text that never renders.

use yggdryl::internals::auth_secret::Secret;

#[test]
fn a_secret_renders_as_redacted_and_exposes_itself_by_name_only() {
    let secret = Secret::new("wJalrXUtnFEMI/K7MDENG");
    assert_eq!(format!("{secret:?}"), "<redacted>");
    assert_eq!(secret.expose(), "wJalrXUtnFEMI/K7MDENG");

    // A holder that derives Debug inherits the redaction.
    #[derive(Debug)]
    struct Holder {
        key: Secret,
        token: Option<Secret>,
    }
    let held = Holder {
        key: Secret::from("k"),
        token: Some(Secret::from("t".to_owned())),
    };
    let rendered = format!("{held:?}");
    assert_eq!(held.key.expose(), "k");
    assert_eq!(held.token.as_ref().map(Secret::expose), Some("t"));
    assert!(rendered.contains("key: <redacted>"), "{rendered}");
    assert!(rendered.contains("token: Some(<redacted>)"), "{rendered}");
    assert!(!rendered.contains("\"k\""), "{rendered}");
}

#[test]
fn equality_and_hashing_read_the_text() {
    use std::collections::HashSet;

    let one = Secret::new("same");
    let two = Secret::from("same");
    assert_eq!(one, two);
    assert_ne!(one, Secret::new("other"));
    let set: HashSet<Secret> = [one.clone(), two, Secret::new("other")]
        .into_iter()
        .collect();
    assert_eq!(set.len(), 2);
}

/// A file written whole or not at all, and only its owner's to read.
#[cfg(feature = "aws")]
#[test]
fn a_private_write_replaces_the_file_whole_and_leaves_no_sibling() {
    use yggdryl::internals::auth_secret::write_private;

    let directory =
        std::env::temp_dir().join(format!("yggdryl-auth-private-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    let path = directory.join("nested").join("token.json");
    write_private(&path, b"{\"first\":true}").expect("a first write");
    write_private(&path, b"{\"second\":true}").expect("a replacing write");
    assert_eq!(
        std::fs::read(&path).expect("the file"),
        b"{\"second\":true}"
    );
    let names: Vec<_> = std::fs::read_dir(path.parent().expect("a parent"))
        .expect("the directory")
        .map(|entry| entry.expect("an entry").file_name())
        .collect();
    assert_eq!(names, ["token.json"], "no sibling is left behind");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "only its owner reads it");
    }
    let _ = std::fs::remove_dir_all(&directory);
}
