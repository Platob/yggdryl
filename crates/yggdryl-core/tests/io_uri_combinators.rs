//! The ergonomic combinators layered over the RFC 3986 split: [`Uri::joinpath`] (lexical
//! path joining), [`Uri::merge_with`] / [`Authority::merge_with`] (component overlay),
//! [`Uri::copy`], and the `with_authority` / `Authority::with_*` builders. Focus is the
//! *seam* cases — trailing/leading slashes, absolute resets, empty inputs, authority
//! interaction — and the invariant that every result still round-trips and keeps value
//! semantics.

use yggdryl_core::io::{Authority, Uri, Url};

// -------------------------------------------------------------------------------------
// joinpath — correct seam handling
// -------------------------------------------------------------------------------------

#[test]
fn joinpath_basic_append() {
    let base = Uri::parse("https://api.example.com/v1").unwrap();
    assert_eq!(
        base.joinpath("users").to_string(),
        "https://api.example.com/v1/users"
    );
    // Chains, each returning a fresh Uri.
    assert_eq!(base.joinpath("users").joinpath("42").path(), "/v1/users/42");
}

#[test]
fn joinpath_never_doubles_the_separator() {
    // Trailing slash on the base + a plain segment collapse to exactly one slash.
    assert_eq!(Uri::from_path("/v1/").joinpath("users").path(), "/v1/users");
    assert_eq!(
        Uri::from_path("/v1///").joinpath("users").path(),
        "/v1/users"
    );
    // A trailing slash on the *segment* is preserved (directory marker).
    assert_eq!(
        Uri::from_path("/v1").joinpath("users/").path(),
        "/v1/users/"
    );
}

#[test]
fn joinpath_multi_segment_in_one_call() {
    // A segment may itself contain slashes — joined in one shot.
    assert_eq!(
        Uri::from_path("/v1").joinpath("users/42/profile").path(),
        "/v1/users/42/profile"
    );
}

#[test]
fn joinpath_absolute_segment_resets_the_path() {
    let base = Uri::parse("https://h/a/b?x=1#f").unwrap();
    // A leading-slash segment replaces the path; query and fragment are kept.
    assert_eq!(base.joinpath("/c").to_string(), "https://h/c?x=1#f");
    assert_eq!(base.joinpath("/c/d").path(), "/c/d");
}

#[test]
fn joinpath_empty_segment_is_a_no_op() {
    let base = Uri::parse("https://h/a").unwrap();
    assert_eq!(base.joinpath(""), base);
    // Even on an authority with an empty path, joining nothing adds no slash.
    assert_eq!(
        Uri::parse("https://h").unwrap().joinpath("").to_string(),
        "https://h"
    );
}

#[test]
fn joinpath_stays_rooted_under_an_authority() {
    // A relative segment onto an empty path must not fuse into the host.
    assert_eq!(Uri::parse("https://h").unwrap().joinpath("p").path(), "/p");
    assert_eq!(
        Uri::parse("https://h").unwrap().joinpath("p").to_string(),
        "https://h/p"
    );
    // The root path joins cleanly too.
    assert_eq!(Uri::parse("https://h/").unwrap().joinpath("p").path(), "/p");
}

#[test]
fn joinpath_relative_without_authority_stays_relative() {
    // No authority: a relative segment onto an empty path yields a relative path.
    assert_eq!(Uri::default().joinpath("a").path(), "a");
    assert_eq!(Uri::default().joinpath("a").joinpath("b").path(), "a/b");
    // A query-only URI has an empty path; joining keeps the query.
    assert_eq!(
        Uri::parse("?q=1").unwrap().joinpath("a").to_string(),
        "a?q=1"
    );
}

#[test]
fn joinpath_encodes_and_normalizes_like_set_path() {
    // The argument is percent-encoded (spaces) and back-slashes are normalized, exactly like
    // `set_path` / `from_path`.
    assert_eq!(Uri::from_path("/v1").joinpath("a b").path(), "/v1/a%20b");
    assert_eq!(Uri::from_path("/v1").joinpath(r"x\y").path(), "/v1/x/y");
}

#[test]
fn joinpath_on_windows_drive_path() {
    let base = Uri::parse(r"C:\Users").unwrap();
    assert_eq!(base.joinpath("docs").path(), "C:/Users/docs");
    assert_eq!(
        base.joinpath(r"docs\notes.txt").path(),
        "C:/Users/docs/notes.txt"
    );
}

#[test]
fn joinpath_result_round_trips_and_has_value_semantics() {
    let joined = Uri::parse("https://h/v1").unwrap().joinpath("users/42");
    assert_eq!(
        Uri::deserialize_bytes(&joined.serialize_bytes()).unwrap(),
        joined
    );
    // Equal to the same URI written out directly.
    assert_eq!(joined, Uri::parse("https://h/v1/users/42").unwrap());
}

#[test]
fn url_joinpath_keeps_the_scheme() {
    let url = Url::parse("https://api.example.com/v1").unwrap();
    let joined = url.joinpath("users/42");
    assert_eq!(joined.scheme(), "https");
    assert_eq!(joined.to_string(), "https://api.example.com/v1/users/42");
}

// -------------------------------------------------------------------------------------
// merge_with — component overlay
// -------------------------------------------------------------------------------------

#[test]
fn merge_with_overlays_only_present_components() {
    let base = Uri::parse("https://prod.example.com/v1?trace=1").unwrap();

    // A patch carrying only an authority swaps the host, keeping scheme/path/query.
    let host_patch = Uri::parse("//staging.example.com").unwrap();
    assert_eq!(
        base.merge_with(&host_patch).to_string(),
        "https://staging.example.com/v1?trace=1"
    );

    // A patch carrying only a path overrides the path, keeping the rest.
    let path_patch = Uri::from_path("/v2");
    assert_eq!(
        base.merge_with(&path_patch).to_string(),
        "https://prod.example.com/v2?trace=1"
    );
}

#[test]
fn merge_with_default_is_an_identity_copy() {
    let base = Uri::parse("https://h/a?q#f").unwrap();
    assert_eq!(base.merge_with(&Uri::default()), base);
}

#[test]
fn merge_with_other_wins_on_every_set_field() {
    let base = Uri::parse("http://a/x?u=1#top").unwrap();
    let other = Uri::parse("https://b/y?v=2#bottom").unwrap();
    assert_eq!(base.merge_with(&other), other); // fully-populated patch replaces everything
}

#[test]
fn merge_with_keeps_base_query_when_patch_has_none() {
    let base = Uri::parse("https://h/a?keep=1").unwrap();
    let patch = Uri::from_path("/b"); // no query
    let merged = base.merge_with(&patch);
    assert_eq!(merged.query(), Some("keep=1"));
    assert_eq!(merged.path(), "/b");
}

#[test]
fn authority_merge_with_is_component_level() {
    let base = Authority::new(Some("svc"), Some("secret"), "db", Some(5432));
    // Only the host set on the patch -> credentials and port survive.
    assert_eq!(
        base.merge_with(&Authority::from_host("replica"))
            .to_string(),
        "svc:secret@replica:5432"
    );
    // Only a port patch -> host and credentials survive.
    let port_patch = Authority::default().with_port(Some(6000));
    assert_eq!(base.merge_with(&port_patch).port(), Some(6000));
    assert_eq!(base.merge_with(&port_patch).host(), "db");
}

#[test]
fn url_merge_with_stays_absolute() {
    let base = Url::parse("https://prod/v1").unwrap();
    let merged = base.merge_with(&Url::parse("https://staging/v2").unwrap());
    assert_eq!(merged.scheme(), "https");
    assert_eq!(merged.to_string(), "https://staging/v2");
}

// -------------------------------------------------------------------------------------
// copy — an explicit, independent clone
// -------------------------------------------------------------------------------------

#[test]
fn copy_is_an_independent_equal_value() {
    let base = Uri::parse("https://h/a?q#f").unwrap();
    let mut dup = base.copy();
    assert_eq!(dup, base);
    // Mutating the copy leaves the original untouched.
    dup.set_path("/b");
    assert_eq!(base.path(), "/a");
    assert_eq!(dup.path(), "/b");

    assert_eq!(
        Url::parse("sc://h").unwrap().copy(),
        Url::parse("sc://h").unwrap()
    );
    assert_eq!(Authority::from_host("h").copy(), Authority::from_host("h"));
}

// -------------------------------------------------------------------------------------
// with_authority / set_authority — attach a whole Authority
// -------------------------------------------------------------------------------------

#[test]
fn with_authority_attaches_a_built_authority() {
    let authority = Authority::default()
        .with_host("db.internal")
        .with_user(Some("svc"))
        .with_port(Some(5432));
    let uri = Uri::default()
        .with_scheme("postgres")
        .with_authority(Some(authority))
        .with_path("/app");
    assert_eq!(uri.to_string(), "postgres://svc@db.internal:5432/app");
    assert_eq!(uri.host(), Some("db.internal"));
    // Round-trips.
    assert_eq!(Uri::deserialize_bytes(&uri.serialize_bytes()).unwrap(), uri);
}

#[test]
fn with_authority_none_drops_the_authority() {
    let uri = Uri::parse("https://user@h:8080/p")
        .unwrap()
        .with_authority(None);
    assert_eq!(uri.authority(), None);
    assert_eq!(uri.to_string(), "https:/p");
}

#[test]
fn authority_with_builders_chain_and_clear() {
    let a = Authority::from_host("h")
        .with_user(Some("u"))
        .with_password(Some("p"))
        .with_port(Some(80));
    assert_eq!(a.to_string(), "u:p@h:80");
    // Clearing via `None`.
    assert_eq!(a.clone().with_password(None).to_string(), "u@h:80");
    assert_eq!(a.with_user(None).with_password(None).to_string(), "h:80");
}
