//! The root resolution an integration test cannot reach.
//!
//! `Folder::home_from` takes the two environment variables directly, which is
//! the only way to name a home without touching the developer's real one.
//! Everything a caller can observe lives in `tests/holder/local.rs`.
//!
//! The well-known roots: handles over what the platform and the environment
//! report, none of which creates anything.

use std::ffi::OsString;

use crate::IOBase;
use crate::holder::local::Folder;

fn set(text: &str) -> Option<OsString> {
    Some(OsString::from(text))
}

/// Two distinct candidate homes under the temporary root, so a wrong pick
/// is visible and neither is the developer's real home.
fn candidates() -> (std::path::PathBuf, std::path::PathBuf) {
    let root = Folder::temporary().unwrap().path().unwrap();
    (root.join("yggdryl-home"), root.join("yggdryl-profile"))
}

#[test]
fn the_temporary_root_is_a_local_container() {
    let temporary = Folder::temporary().unwrap();
    assert!(temporary.is_container());
    assert!(temporary.url().is_local());
    // The platform's own directory is there; nothing here made it.
    assert!(temporary.exists());
}

#[test]
fn home_prefers_home_over_userprofile() {
    let (home, profile) = candidates();
    let resolved =
        Folder::home_from(set(home.to_str().unwrap()), set(profile.to_str().unwrap())).unwrap();
    assert_eq!(resolved.url(), Folder::new(&home).unwrap().url());
}

#[test]
fn home_alone_resolves() {
    let (home, _) = candidates();
    let resolved = Folder::home_from(set(home.to_str().unwrap()), None).unwrap();
    assert_eq!(resolved.url(), Folder::new(&home).unwrap().url());
}

#[test]
fn userprofile_alone_resolves() {
    let (_, profile) = candidates();
    let resolved = Folder::home_from(None, set(profile.to_str().unwrap())).unwrap();
    assert_eq!(resolved.url(), Folder::new(&profile).unwrap().url());
}

#[test]
fn an_empty_value_counts_as_unset() {
    let (_, profile) = candidates();
    let resolved = Folder::home_from(set(""), set(profile.to_str().unwrap())).unwrap();
    assert_eq!(resolved.url(), Folder::new(&profile).unwrap().url());
    assert!(Folder::home_from(set(""), set("")).unwrap_err().is_absent());
}

#[test]
fn neither_variable_is_a_typed_absence_naming_both() {
    let error = Folder::home_from(None, None).unwrap_err();
    assert!(error.is_absent());
    let message = error.to_string();
    assert!(message.contains("HOME"), "{message}");
    assert!(message.contains("USERPROFILE"), "{message}");
}

#[test]
fn a_resolved_home_creates_nothing() {
    let (home, _) = candidates();
    let _ = std::fs::remove_dir_all(&home);
    let resolved = Folder::home_from(set(home.to_str().unwrap()), None).unwrap();
    assert!(!resolved.exists());
    assert!(!home.exists());
}
