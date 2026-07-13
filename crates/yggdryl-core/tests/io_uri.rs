//! Edge-case tests for the RFC 3986 [`Uri`] / [`Url`] / [`Authority`] base types: the
//! component split, POSIX path normalization, the path accessors, the byte codec, value
//! semantics, and Uri↔Url interchange.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use yggdryl_core::io::{Authority, Uri, UriError, Url};

fn hash_of<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

// -------------------------------------------------------------------------------------
// Full URL — every accessor
// -------------------------------------------------------------------------------------

#[test]
fn full_url_every_accessor() {
    let uri = Uri::parse("scheme://user:pass@host:1234/a/b.txt?q=1#frag").unwrap();
    assert_eq!(uri.scheme(), Some("scheme"));
    assert_eq!(uri.user(), Some("user"));
    assert_eq!(uri.password(), Some("pass"));
    assert_eq!(uri.host(), Some("host"));
    assert_eq!(uri.port(), Some(1234));
    assert_eq!(uri.path(), "/a/b.txt");
    assert_eq!(uri.query(), Some("q=1"));
    assert_eq!(uri.fragment(), Some("frag"));
    assert_eq!(uri.name(), Some("b.txt"));
    assert_eq!(uri.stem(), Some("b"));
    assert_eq!(uri.extension(), Some("txt"));
    assert_eq!(uri.extensions(), vec!["txt"]);

    let authority = uri.authority().unwrap();
    assert_eq!(authority.host(), "host");
    assert_eq!(authority.to_string(), "user:pass@host:1234");
}

// -------------------------------------------------------------------------------------
// Scheme-less path
// -------------------------------------------------------------------------------------

#[test]
fn scheme_less_posix_path() {
    let uri = Uri::parse("/a/b/c").unwrap();
    assert_eq!(uri.scheme(), None);
    assert_eq!(uri.authority(), None);
    assert_eq!(uri.path(), "/a/b/c");
    assert_eq!(uri.name(), Some("c"));
}

// -------------------------------------------------------------------------------------
// Windows drive / UNC path normalization -> POSIX slashes
// -------------------------------------------------------------------------------------

#[test]
fn windows_drive_path_normalizes_to_posix() {
    let uri = Uri::parse(r"C:\Users\x\a.tar.gz").unwrap();
    assert_eq!(uri.scheme(), None);
    assert_eq!(uri.path(), "C:/Users/x/a.tar.gz");
    assert_eq!(uri.name(), Some("a.tar.gz"));
    assert_eq!(uri.stem(), Some("a.tar"));
    assert_eq!(uri.extension(), Some("gz"));
    assert_eq!(uri.extensions(), vec!["tar", "gz"]);
}

#[test]
fn drive_path_with_forward_slash_is_still_a_path() {
    // A single-letter "scheme" before a slash is a drive letter, not a URI scheme.
    let uri = Uri::parse("C:/Users/x").unwrap();
    assert_eq!(uri.scheme(), None);
    assert_eq!(uri.path(), "C:/Users/x");
}

#[test]
fn single_letter_scheme_needs_no_trailing_slash() {
    // DESIGN tradeoff: `x:/…` is a drive letter, but `x:foo` (no slash) is scheme `x`.
    let uri = Uri::parse("x:foo").unwrap();
    assert_eq!(uri.scheme(), Some("x"));
    assert_eq!(uri.path(), "foo");
}

#[test]
fn unc_path_normalizes() {
    let uri = Uri::parse(r"\\server\share\f").unwrap();
    assert_eq!(uri.scheme(), None);
    assert_eq!(uri.path(), "//server/share/f");
    assert_eq!(uri.name(), Some("f"));
}

#[test]
fn relative_backslash_path_normalizes() {
    let uri = Uri::parse(r"a\b\c").unwrap();
    assert_eq!(uri.path(), "a/b/c");
}

// -------------------------------------------------------------------------------------
// Directory-like and dotfile names
// -------------------------------------------------------------------------------------

#[test]
fn directory_path_has_no_name() {
    let uri = Uri::parse("/a/b/").unwrap();
    assert_eq!(uri.name(), None);
    assert_eq!(uri.stem(), None);
    assert_eq!(uri.extension(), None);
    assert!(uri.extensions().is_empty());
}

#[test]
fn dotfile_is_hidden_not_extension() {
    // DESIGN: a leading dot marks a hidden file; its dot is not an extension separator.
    let uri = Uri::from_path("/home/user/.bashrc");
    assert_eq!(uri.name(), Some(".bashrc"));
    assert_eq!(uri.stem(), Some(".bashrc"));
    assert_eq!(uri.extension(), None);
    assert!(uri.extensions().is_empty());
}

#[test]
fn multi_dot_file_extensions() {
    let uri = Uri::from_path("/x/a.b.c.d");
    assert_eq!(uri.name(), Some("a.b.c.d"));
    assert_eq!(uri.stem(), Some("a.b.c"));
    assert_eq!(uri.extension(), Some("d"));
    assert_eq!(uri.extensions(), vec!["b", "c", "d"]);
}

// -------------------------------------------------------------------------------------
// Query-only / fragment-only URIs
// -------------------------------------------------------------------------------------

#[test]
fn query_only_and_fragment_only() {
    let q = Uri::parse("?q=1").unwrap();
    assert_eq!(q.path(), "");
    assert_eq!(q.query(), Some("q=1"));
    assert_eq!(q.fragment(), None);
    assert_eq!(q.name(), None);

    let f = Uri::parse("#frag").unwrap();
    assert_eq!(f.path(), "");
    assert_eq!(f.fragment(), Some("frag"));
    assert_eq!(f.query(), None);
}

// -------------------------------------------------------------------------------------
// Ports, schemes, IPv6 — guided errors
// -------------------------------------------------------------------------------------

#[test]
fn out_of_range_port_is_guided_error() {
    let err = Uri::parse("http://host:99999/").unwrap_err();
    assert!(matches!(err, UriError::InvalidPort { .. }));
    assert!(err.to_string().contains("99999"));
    assert!(err.to_string().contains("0..=65535"));
}

#[test]
fn non_numeric_port_is_guided_error() {
    assert!(matches!(
        Uri::parse("http://host:abc/"),
        Err(UriError::InvalidPort { .. })
    ));
}

#[test]
fn empty_scheme_is_guided_error() {
    assert_eq!(Uri::parse("://host"), Err(UriError::EmptyScheme));
}

#[test]
fn invalid_scheme_is_guided_error() {
    assert!(matches!(
        Uri::parse("ht tp://host"),
        Err(UriError::InvalidScheme { .. })
    ));
}

#[test]
fn ipv6_host_is_bracketed_with_port() {
    let uri = Uri::parse("http://[::1]:8080/p").unwrap();
    assert_eq!(uri.host(), Some("[::1]"));
    assert_eq!(uri.port(), Some(8080));
    assert_eq!(uri.path(), "/p");
}

#[test]
fn ipv6_host_without_port() {
    let uri = Uri::parse("http://[2001:db8::1]/p").unwrap();
    assert_eq!(uri.host(), Some("[2001:db8::1]"));
    assert_eq!(uri.port(), None);
}

// -------------------------------------------------------------------------------------
// Byte round-trip (serialize/deserialize inverse)
// -------------------------------------------------------------------------------------

#[test]
fn byte_round_trip() {
    for s in [
        "scheme://user:pass@host:1234/a/b.txt?q=1#frag",
        "/a/b/c",
        "mailto:person@example.com",
        "file:///etc/hosts",
        "?q=1",
        "#frag",
        "http://[::1]:8080/p",
    ] {
        let uri = Uri::parse(s).unwrap();
        let bytes = uri.serialize_bytes();
        assert_eq!(bytes, s.as_bytes());
        assert_eq!(Uri::deserialize_bytes(&bytes).unwrap(), uri);
    }
}

#[test]
fn deserialize_non_utf8_is_guided_error() {
    assert!(matches!(
        Uri::deserialize_bytes(&[0xff, 0xfe]),
        Err(UriError::NonUtf8 { len: 2 })
    ));
}

// -------------------------------------------------------------------------------------
// Value semantics — equal iff canonical-equal, hash agrees
// -------------------------------------------------------------------------------------

#[test]
fn value_semantics_equal_iff_canonical_equal() {
    let a = Uri::parse("sc://h/p?q#f").unwrap();
    let b = Uri::deserialize_bytes(&a.serialize_bytes()).unwrap();
    let c = Uri::parse("sc://h/other").unwrap();

    assert_eq!(a, b);
    assert_eq!(hash_of(&a), hash_of(&b));
    assert_ne!(a, c);

    // Built with mutators, equal to the parsed form.
    let built = Uri::default()
        .with_scheme("sc")
        .with_host("h")
        .with_path("/p")
        .with_query("q")
        .with_fragment("f");
    assert_eq!(built, a);
    assert_eq!(hash_of(&built), hash_of(&a));
}

#[test]
fn authority_value_semantics() {
    let a = Authority::new(Some("u"), None, "h", Some(80));
    let b = Authority::new(Some("u"), None, "h", Some(80));
    assert_eq!(a, b);
    assert_eq!(hash_of(&a), hash_of(&b));
    assert_ne!(a, Authority::new(Some("u"), None, "h", Some(81)));
}

// -------------------------------------------------------------------------------------
// Mutators re-normalize; setters create an authority
// -------------------------------------------------------------------------------------

#[test]
fn set_path_renormalizes_slashes() {
    let mut uri = Uri::from_path("/tmp");
    uri.set_path(r"C:\a\b");
    assert_eq!(uri.path(), "C:/a/b");
}

#[test]
fn setters_create_authority() {
    let uri = Uri::default()
        .with_host("h")
        .with_port(9000)
        .with_user("me");
    assert_eq!(uri.host(), Some("h"));
    assert_eq!(uri.port(), Some(9000));
    assert_eq!(uri.user(), Some("me"));
    assert_eq!(uri.to_string(), "//me@h:9000");
}

// -------------------------------------------------------------------------------------
// Uri <-> Url interchange
// -------------------------------------------------------------------------------------

#[test]
fn scheme_less_uri_is_not_a_url() {
    let uri = Uri::parse("/relative/path").unwrap();
    assert!(matches!(
        Url::try_from(uri.clone()),
        Err(UriError::MissingScheme { .. })
    ));
    assert!(uri.into_url().is_err());
}

#[test]
fn full_url_round_trips_through_uri() {
    let url = Url::parse("https://user@example.com:8443/a/b.json?x=1#top").unwrap();
    assert_eq!(url.scheme(), "https");
    assert_eq!(url.name(), Some("b.json"));
    assert_eq!(url.extension(), Some("json"));

    // Url -> Uri -> Url round-trip preserves the canonical form.
    let uri: Uri = url.clone().into();
    let back: Url = Url::try_from(uri.clone()).unwrap();
    assert_eq!(back, url);
    assert_eq!(url.as_uri(), &uri);
    assert_eq!(url.clone().into_uri(), uri);

    // Byte codec round-trip at the Url level.
    assert_eq!(Url::deserialize_bytes(&url.serialize_bytes()).unwrap(), url);
}

#[test]
fn url_with_no_authority_is_valid() {
    // DESIGN: a Url requires only a scheme; the authority is optional.
    let url = Url::parse("mailto:person@example.com").unwrap();
    assert_eq!(url.scheme(), "mailto");
    assert_eq!(url.host(), None);
    assert_eq!(url.path(), "person@example.com");
}

// Regression (review): a malformed multi-colon authority must be REJECTED at parse rather
// than parse into a `Uri` whose canonical string does not round-trip.
#[test]
fn malformed_multi_colon_authority_is_rejected() {
    for bad in ["//a::", "//:a:", "//::", "//host:8080:9090"] {
        assert!(
            matches!(Uri::parse(bad), Err(UriError::InvalidPort { .. })),
            "expected InvalidPort for {bad:?}, got {:?}",
            Uri::parse(bad)
        );
    }
    // A single trailing colon (empty port) is still fine and round-trips as no port.
    let u = Uri::parse("//a:").unwrap();
    assert_eq!(u.host(), Some("a"));
    assert_eq!(u.port(), None);
    assert_eq!(Uri::deserialize_bytes(&u.serialize_bytes()).unwrap(), u);
}

// Regression (review): a password set with no user must not be dropped by the canonical
// form — it survives a serialize/deserialize round-trip.
#[test]
fn password_without_user_round_trips() {
    let u = Uri::default().with_password("secret");
    assert_eq!(u.password(), Some("secret"));
    let round = Uri::deserialize_bytes(&u.serialize_bytes()).unwrap();
    assert_eq!(round.password(), Some("secret"));
    assert_eq!(round, u); // equal by canonical string
}

// Regression (review): `extension()` and `extensions()` must agree — the last extension of
// `extensions()` is exactly `extension()`, including the trailing-dot case.
#[test]
fn extension_agrees_with_extensions() {
    for (path, ext) in [
        ("/x/a.tar.gz", Some("gz")),
        ("/x/a.b.", None), // trailing dot -> no extension, and extensions() is empty
        ("/x/.bashrc", None),
        ("/x/a", None),
        ("/x/a.b..c", Some("c")),
    ] {
        let uri = Uri::from_path(path);
        assert_eq!(uri.extension(), ext, "extension() for {path:?}");
        assert_eq!(
            uri.extensions().last().map(String::as_str),
            ext,
            "extensions().last() for {path:?}"
        );
    }
    assert!(Uri::from_path("/x/a.b.").extensions().is_empty());
}
