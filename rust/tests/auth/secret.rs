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
