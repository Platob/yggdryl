//! URI and URL value types built on the core foundations.
//!
//! - [`Uri`] is the generic [RFC 3986](https://www.rfc-editor.org/rfc/rfc3986)
//!   shape: `scheme:[//authority]path[?query][#fragment]`.
//! - [`Url`] is the common subset that always has an authority, decomposed into
//!   `username`, `password`, `host` and `port`.

use std::borrow::Cow;

use crate::{encode_component, percent_decode, percent_encode, Params};

mod uri;
#[allow(clippy::module_inception)]
mod url;

pub use uri::{Uri, UriError};
pub use url::{Url, UrlError};

// Per-component sets of delimiter bytes that are left as-is when encoding a
// component for output (on top of the always-safe unreserved set).
pub(crate) const KEEP_AUTHORITY: &[u8] = b":@[]";
pub(crate) const KEEP_PATH: &[u8] = b"/:@";
pub(crate) const KEEP_QUERY: &[u8] = b"/:@?&=";
pub(crate) const KEEP_FRAGMENT: &[u8] = b"/:@?";

/// Renders a component either percent-encoded (`encode`) or percent-decoded
/// (best effort), used by `to_str(encode)`. Borrows `input` when nothing changes.
pub(crate) fn render_component<'a>(input: &'a str, keep: &[u8], encode: bool) -> Cow<'a, str> {
    if encode {
        encode_component(input, keep)
    } else {
        percent_decode(input).unwrap_or(Cow::Borrowed(input))
    }
}

/// Splits a `key=value&key=value2` query into a multimap. Repeated keys
/// accumulate their values; when `decode`, each key/value is percent-decoded
/// (parts that fail to decode are kept verbatim).
pub(crate) fn query_to_params(query: &str, decode: bool) -> Params {
    let unescape = |s: &str| -> String {
        if decode {
            percent_decode(s)
                .map(Cow::into_owned)
                .unwrap_or_else(|_| s.to_string())
        } else {
            s.to_string()
        }
    };
    let mut params = Params::new();
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        params
            .entry(unescape(key))
            .or_default()
            .push(unescape(value));
    }
    params
}

/// Scans a query for a single (decoded) `key`, returning its decoded values, or
/// `None` if absent. Avoids building the whole [`Params`] map for a one-key
/// lookup ([`Uri::get_param`] / [`Uri::has_param`]).
pub(crate) fn query_param(query: &str, key: &str) -> Option<Vec<String>> {
    // Compare the raw key without allocating unless it carries an escape.
    let key_matches = |raw: &str| {
        if raw.contains('%') {
            percent_decode(raw).is_ok_and(|decoded| decoded == key)
        } else {
            raw == key
        }
    };
    let mut values: Option<Vec<String>> = None;
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if key_matches(k) {
            let value = percent_decode(v)
                .map(Cow::into_owned)
                .unwrap_or_else(|_| v.to_string());
            values.get_or_insert_with(Vec::new).push(value);
        }
    }
    values
}

/// Builds a `key=value&…` query from a [`Params`] multimap. When `encode`, each
/// key/value is percent-encoded. Keys with several values are emitted once per
/// value; pairs come out in the map's (sorted) order for a deterministic result.
pub(crate) fn params_to_query(params: &Params, encode: bool) -> String {
    let escape = |s: &str| {
        if encode {
            percent_encode(s)
        } else {
            s.to_string()
        }
    };
    let mut pairs = Vec::new();
    for (key, values) in params {
        let key = escape(key);
        if values.is_empty() {
            pairs.push(key);
        } else {
            for value in values {
                pairs.push(format!("{key}={}", escape(value)));
            }
        }
    }
    pairs.join("&")
}

/// Builds the query field from `params`: `None` for an empty map (clearing the
/// query), otherwise the rendered string. Shared by `with_params` and the query
/// mutators so the empty-check lives in one place.
pub(crate) fn build_query(params: &Params, encode: bool) -> Option<String> {
    (!params.is_empty()).then(|| params_to_query(params, encode))
}

/// Returns `true` if `scheme` matches `ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )`.
pub(crate) fn is_valid_scheme(scheme: &str) -> bool {
    let mut chars = scheme.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// Renders one path component: percent-decoded unless `encode` keeps it as-is.
fn render_part(part: &str, encode: bool) -> String {
    if encode {
        part.to_string()
    } else {
        percent_decode(part)
            .map(Cow::into_owned)
            .unwrap_or_else(|_| part.to_string())
    }
}

/// Splits a `/`-separated path into its non-empty, rendered segments.
pub(crate) fn path_parts(path: &str, encode: bool) -> Vec<String> {
    path.split('/')
        .filter(|p| !p.is_empty())
        .map(|p| render_part(p, encode))
        .collect()
}

/// The last non-empty path segment (the file name), rendered.
pub(crate) fn path_name(path: &str, encode: bool) -> String {
    path.rsplit('/')
        .find(|p| !p.is_empty())
        .map(|p| render_part(p, encode))
        .unwrap_or_default()
}

/// Splits a file name into `(stem, extensions)` at the first non-leading `.`, so
/// `"a.tar.gz"` → `("a", ["tar", "gz"])` and `".bashrc"` → `(".bashrc", [])`.
pub(crate) fn split_stem_ext(name: &str) -> (&str, Vec<&str>) {
    let dot = if name.len() > 1 {
        name[1..].find('.').map(|i| i + 1)
    } else {
        None
    };
    match dot {
        Some(idx) => (&name[..idx], name[idx + 1..].split('.').collect()),
        None => (name, Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use crate::{percent_decode, percent_encode};
    use crate::{Mapping, MediaType, MimeType, Params};
    use crate::{Uri, UriError, Url, UrlError};

    #[test]
    fn param_read_edge_cases() {
        // Encoded key, multi-value, empty value, and a bare flag (no `=`).
        let u = Uri::from_str("http://h/p?a%20b=1&c=2&c=3&d=&flag").unwrap();
        assert_eq!(u.get_param("a b"), Some(vec!["1".to_string()])); // key decoded
        assert_eq!(
            u.get_param("c"),
            Some(vec!["2".to_string(), "3".to_string()])
        );
        assert_eq!(u.get_param("d"), Some(vec![String::new()])); // empty value
        assert_eq!(u.get_param("flag"), Some(vec![String::new()])); // no `=`
        assert_eq!(u.get_param("missing"), None);
        assert!(u.has_param("a b") && u.has_param("flag") && !u.has_param("missing"));
        // No query at all.
        let bare = Uri::from_str("http://h/p").unwrap();
        assert_eq!(bare.get_param("x"), None);
        assert!(!bare.has_param("x"));
        // get_param agrees with the full params() map.
        assert_eq!(u.get_param("c"), u.params(true).get("c").cloned());
    }

    #[test]
    fn path_and_scheme_edge_cases() {
        // Trailing/leading/double slashes collapse to non-empty segments.
        let u = Uri::from_str("http://h//a//b/").unwrap();
        assert_eq!(u.parts(false), vec!["a", "b"]);
        assert_eq!(u.name(false), "b");
        // A dotfile has no extensions; its stem keeps the leading dot.
        let dot = Uri::from_str("file:/etc/.bashrc").unwrap();
        assert_eq!(dot.stem(false), ".bashrc");
        assert!(dot.extensions(false).is_empty());
        // Multi-`+` scheme splits into base + every extension.
        let s = Uri::from_str("a+b+c://h/p").unwrap();
        assert_eq!(s.scheme_base(), "a");
        assert_eq!(s.scheme_ext(), vec!["b", "c"]);
        // No path -> empty accessors, no panic.
        let empty = Uri::from_str("mailto:x@y").unwrap();
        assert_eq!(empty.name(false), "x@y");
        assert!(empty.parts(false).len() == 1);
    }

    #[test]
    fn uri_edge_cases() {
        // Empty input.
        assert_eq!(Uri::from_str(""), Err(UriError::Empty));
        // A `?`/`#` inside the query/fragment is kept (split is leftmost only).
        let u = Uri::from_str("http://h/p?a=b?c#x#y").unwrap();
        assert_eq!(u.query(), Some("a=b?c"));
        assert_eq!(u.fragment(), Some("x#y"));
        // A `:` after the first `/` is part of the path, not a scheme.
        assert_eq!(Uri::from_str("a/b:c").unwrap().scheme(), "file");
        // Empty query/fragment are preserved as `Some("")`.
        let e = Uri::from_str("http://h/p?#").unwrap();
        assert_eq!(e.query(), Some(""));
        assert_eq!(e.fragment(), Some(""));
        // Scheme-only with empty authority.
        let bare = Uri::from_str("http://").unwrap();
        assert_eq!(bare.authority(), Some(""));
        assert_eq!(bare.path(), "");
        // A trailing `%` is a malformed escape.
        assert!(Uri::from_str("http://h/a%").is_err());
        assert!(Uri::from_str("http://h/a%2").is_err());
    }

    #[test]
    fn url_edge_cases() {
        // Port out of the u16 range is rejected.
        assert!(matches!(
            Url::from_str("http://h:99999"),
            Err(UrlError::InvalidPort(_))
        ));
        // An unclosed IPv6 literal is rejected.
        assert!(Url::from_str("http://[::1/p").is_err());
        // Userinfo with an `@`-free password and an empty path round-trips.
        let u = Url::from_str("http://user:pa:ss@h:1/").unwrap();
        assert_eq!(u.username(), Some("user"));
        assert_eq!(u.password(), Some("pa:ss"));
        assert_eq!(u.port(), Some(1));
        // Empty host is rejected even with userinfo.
        assert_eq!(Url::from_str("http://u@:80"), Err(UrlError::MissingHost));
        // IPv6 with zone-id-like content round-trips through brackets.
        assert_eq!(
            Url::from_str("http://[::1]:8080/").unwrap().to_string(),
            "http://[::1]:8080/"
        );
    }

    #[test]
    fn uri_full() {
        let uri = Uri::from_str("https://example.com/docs?page=2#intro").unwrap();
        assert_eq!(uri.scheme(), "https");
        assert_eq!(uri.authority(), Some("example.com"));
        assert_eq!(uri.path(), "/docs");
        assert_eq!(uri.query(), Some("page=2"));
        assert_eq!(uri.fragment(), Some("intro"));
    }

    #[test]
    fn uri_without_authority() {
        let uri = Uri::from_str("mailto:alice@example.com").unwrap();
        assert_eq!(uri.scheme(), "mailto");
        assert_eq!(uri.authority(), None);
        assert_eq!(uri.path(), "alice@example.com");
    }

    #[test]
    fn uri_errors() {
        assert_eq!(Uri::from_str(""), Err(UriError::Empty));
        // An empty (but present) scheme is still an error.
        assert_eq!(Uri::from_str(":no-scheme"), Err(UriError::MissingScheme));
        assert_eq!(Uri::from_str("1http://x"), Err(UriError::InvalidScheme));
    }

    #[test]
    fn path_accessors() {
        let url = Url::from_str("https://h/a/b/archive.tar.gz").unwrap();
        assert_eq!(url.parts(false), vec!["a", "b", "archive.tar.gz"]);
        assert_eq!(url.name(false), "archive.tar.gz");
        assert_eq!(url.stem(false), "archive");
        assert_eq!(url.extensions(false), vec!["tar", "gz"]);
        // encode flag: decoded by default, kept when true.
        let enc = Uri::from_str("file:/dir/a%20b.txt").unwrap();
        assert_eq!(enc.name(false), "a b.txt");
        assert_eq!(enc.name(true), "a%20b.txt");
        // no extension and dotfiles.
        assert_eq!(
            Uri::from_str("file:/x/README").unwrap().extensions(false),
            Vec::<String>::new()
        );
        assert_eq!(
            Uri::from_str("file:/x/.bashrc").unwrap().stem(false),
            ".bashrc"
        );
    }

    #[test]
    fn media_type_inference() {
        // A single extension yields a one-element stack.
        assert_eq!(
            Url::from_str("https://h/a/data.json")
                .unwrap()
                .media_type()
                .unwrap()
                .types(),
            [MimeType::Json]
        );
        // Compound extensions yield the ordered stack (content first).
        assert_eq!(
            Uri::from_str("file:/dump/archive.tar.gz")
                .unwrap()
                .media_type()
                .unwrap()
                .types(),
            [MimeType::Tar, MimeType::Gzip]
        );
        // No (known) extension yields `None`.
        assert_eq!(Url::from_str("https://h/page").unwrap().media_type(), None);
        // `mime_type()` is the single outermost type; `From<&Uri>` mirrors it.
        let uri = Uri::from_str("file:/dump/archive.tar.gz").unwrap();
        assert_eq!(uri.mime_type(), Some(MimeType::Gzip));
        assert_eq!(
            MediaType::from(&uri).types(),
            [MimeType::Tar, MimeType::Gzip]
        );
        assert_eq!(Url::from_str("https://h/page").unwrap().mime_type(), None);
    }

    #[test]
    fn default_file_scheme_and_windows_paths() {
        // No scheme -> file.
        let u = Uri::from_str("no-scheme/path").unwrap();
        assert_eq!(u.scheme(), "file");
        assert_eq!(u.path(), "no-scheme/path");
        // A `:` after a `/` is part of the path, not a scheme.
        assert_eq!(Uri::from_str("a/b:c").unwrap().scheme(), "file");
        // Backslashes are normalised to `/`.
        let w = Uri::from_str("dir\\sub\\file").unwrap();
        assert_eq!(w.scheme(), "file");
        assert_eq!(w.path(), "dir/sub/file");
        // A drive letter is a Windows path -> file.
        let d = Uri::from_str("C:\\Users\\me").unwrap();
        assert_eq!(d.scheme(), "file");
        assert_eq!(d.path(), "/C:/Users/me");
    }

    #[test]
    fn uri_round_trips() {
        for input in [
            "https://example.com/docs?page=2#intro",
            "mailto:alice@example.com",
            "file:///etc/hosts",
            "urn:isbn:0451450523",
        ] {
            assert_eq!(Uri::from_str(input).unwrap().to_string(), input);
        }
    }

    #[test]
    fn uri_validates_scheme() {
        // An invalid scheme is always rejected.
        assert_eq!(Uri::from_str("1http:x"), Err(UriError::InvalidScheme));
    }

    #[test]
    fn uri_validates_percent_encoding() {
        // A malformed `%XX` escape is rejected; well-formed escapes pass.
        assert!(Uri::from_str("http://h/a%zz").is_err());
        assert!(Uri::from_str("http://h/a%20b").is_ok());
    }

    #[test]
    fn uri_from_mapping() {
        let fields = Mapping::from([
            ("scheme".to_string(), "https".to_string()),
            ("authority".to_string(), "example.com".to_string()),
            ("path".to_string(), "/x".to_string()),
        ]);
        let uri = Uri::from_mapping(&fields).unwrap();
        assert_eq!(uri.to_string(), "https://example.com/x");
    }

    #[test]
    fn url_full() {
        let url = Url::from_str("https://user:pw@example.com:8443/api?v=1#top").unwrap();
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.username(), Some("user"));
        assert_eq!(url.password(), Some("pw"));
        assert_eq!(url.host(), "example.com");
        assert_eq!(url.port(), Some(8443));
        assert_eq!(url.path(), "/api");
        assert_eq!(url.query(), Some("v=1"));
        assert_eq!(url.fragment(), Some("top"));
    }

    #[test]
    fn url_minimal() {
        let url = Url::from_str("http://example.com").unwrap();
        assert_eq!(url.host(), "example.com");
        assert_eq!(url.port(), None);
        assert_eq!(url.username(), None);
        assert_eq!(url.path(), "");
    }

    #[test]
    fn url_username_only() {
        let url = Url::from_str("ftp://anon@files.example.com/pub").unwrap();
        assert_eq!(url.username(), Some("anon"));
        assert_eq!(url.password(), None);
        assert_eq!(url.host(), "files.example.com");
    }

    #[test]
    fn url_ipv6() {
        let url = Url::from_str("http://[::1]:8080/status").unwrap();
        assert_eq!(url.host(), "::1");
        assert_eq!(url.port(), Some(8080));
        assert_eq!(url.to_string(), "http://[::1]:8080/status");
    }

    #[test]
    fn url_errors() {
        assert_eq!(
            Url::from_str("mailto:alice@example.com"),
            Err(UrlError::MissingAuthority)
        );
        assert_eq!(Url::from_str("http://user@:80"), Err(UrlError::MissingHost));
        assert!(matches!(
            Url::from_str("http://example.com:notaport"),
            Err(UrlError::InvalidPort(_))
        ));
        // No scheme defaults to `file`, but a Url still needs an authority.
        assert_eq!(Url::from_str("notauri"), Err(UrlError::MissingAuthority));
    }

    #[test]
    fn url_round_trips() {
        for input in [
            "https://user:pw@example.com:8443/api?v=1#top",
            "http://example.com",
            "ftp://anon@files.example.com/pub",
            "http://[::1]:8080/status",
        ] {
            assert_eq!(Url::from_str(input).unwrap().to_string(), input);
        }
    }

    #[test]
    fn url_authority_is_reconstructed() {
        let url = Url::from_str("https://user:pw@example.com:8443/api").unwrap();
        assert_eq!(url.authority(), "user:pw@example.com:8443");
    }

    #[test]
    fn url_from_mapping() {
        let fields = Mapping::from([
            ("scheme".to_string(), "https".to_string()),
            ("host".to_string(), "example.com".to_string()),
            ("port".to_string(), "8443".to_string()),
            ("path".to_string(), "/api".to_string()),
        ]);
        let url = Url::from_mapping(&fields).unwrap();
        assert_eq!(url.to_string(), "https://example.com:8443/api");

        let missing_host = Mapping::from([("scheme".to_string(), "https".to_string())]);
        assert_eq!(Url::from_mapping(&missing_host), Err(UrlError::MissingHost));
    }

    #[test]
    fn uri_constructors_and_builders() {
        let uri = Uri::new("https", "/docs")
            .with_authority("example.com")
            .with_query("page=2")
            .with_fragment("intro");
        assert_eq!(uri.to_string(), "https://example.com/docs?page=2#intro");

        // with_*/without_* leave the original untouched (functional style).
        let bare = uri.clone().without_query().without_fragment();
        assert_eq!(bare.to_string(), "https://example.com/docs");
        assert_eq!(uri.query(), Some("page=2"));

        let from_parts = Uri::from_parts("ftp".into(), Some("h".into()), "/p".into(), None, None);
        assert_eq!(from_parts.to_string(), "ftp://h/p");
    }

    #[test]
    fn copy_overrides_selected_fields() {
        let uri = Uri::new("https", "/a").with_authority("h");
        // Override one field; the rest come from `self`.
        let moved = uri.copy(None, None, Some("/b".into()), None, None);
        assert_eq!(moved.to_string(), "https://h/b");
        // copy(None, …) is a plain clone.
        assert_eq!(uri.copy(None, None, None, None, None), uri);
    }

    #[test]
    fn url_constructors_and_builders() {
        let url = Url::new("https", "example.com")
            .with_port(8443)
            .with_username("user")
            .with_password("pw")
            .with_path("/api");
        assert_eq!(url.to_string(), "https://user:pw@example.com:8443/api");

        let public = url.clone().without_userinfo().without_port();
        assert_eq!(public.to_string(), "https://example.com/api");
        assert_eq!(url.port(), Some(8443));
    }

    #[test]
    fn params_round_trip_and_multivalue() {
        let url = Url::from_str("https://h/p?a=1&a=2&b=hello%20world").unwrap();
        let params = url.params(true);
        assert_eq!(
            params.get("a"),
            Some(&vec!["1".to_string(), "2".to_string()])
        );
        assert_eq!(params.get("b"), Some(&vec!["hello world".to_string()]));

        // Building a query encodes each part.
        let built = Uri::new("https", "/p").with_params(
            &Params::from([("q".to_string(), vec!["a b".to_string()])]),
            true,
        );
        assert_eq!(built.query(), Some("q=a%20b"));

        // add_param adds a new key or replaces an existing one's values.
        let updated = built.add_param("q", vec!["x".to_string(), "y".to_string()], true);
        assert_eq!(updated.query(), Some("q=x&q=y"));
        let added = updated.add_param("r", vec!["1".to_string()], true);
        assert_eq!(added.query(), Some("q=x&q=y&r=1"));
    }

    #[test]
    fn scheme_extensions() {
        let uri = Uri::from_str("https+zip://h/f").unwrap();
        assert_eq!(uri.scheme(), "https+zip");
        assert_eq!(uri.scheme_base(), "https");
        assert_eq!(uri.scheme_ext(), vec!["zip"]);
        let plain = Uri::from_str("https://h").unwrap();
        assert_eq!(plain.scheme_base(), "https");
        assert!(plain.scheme_ext().is_empty());
        // works on Url too
        let url = Url::from_str("git+ssh://h/r").unwrap();
        assert_eq!(url.scheme_base(), "git");
        assert_eq!(url.scheme_ext(), vec!["ssh"]);
    }

    #[test]
    fn uri_url_conversions() {
        let url = Url::from_str("https://user@h:8443/p?x=1").unwrap();
        let uri = Uri::from_url(&url);
        assert_eq!(uri.authority(), Some("user@h:8443"));
        // round-trip back to Url
        let back = uri.to_url().unwrap();
        assert_eq!(back, url);
        // a Uri without authority cannot become a Url
        assert_eq!(
            Uri::from_str("mailto:a@b").unwrap().to_url(),
            Err(UrlError::MissingAuthority)
        );
        // Url::from_uri mirrors to_url
        assert_eq!(Url::from_uri(&uri).unwrap(), url);
    }

    #[test]
    fn params_crud_single_and_bulk() {
        let base = Url::from_str("https://h/p?a=1&b=2&c=3").unwrap();
        assert_eq!(base.get_param("a"), Some(vec!["1".to_string()]));
        assert_eq!(base.get_param("z"), None);
        assert!(base.has_param("a") && !base.has_param("z"));

        // single update
        assert_eq!(
            base.set_param("a", vec!["9".into()], true).get_param("a"),
            Some(vec!["9".to_string()])
        );

        // bulk update (b replaced, d added, a/c kept)
        let bulk = base.set_params(
            &Params::from([
                ("b".to_string(), vec!["x".to_string()]),
                ("d".to_string(), vec!["y".to_string()]),
            ]),
            true,
        );
        assert_eq!(bulk.get_param("b"), Some(vec!["x".to_string()]));
        assert_eq!(bulk.get_param("d"), Some(vec!["y".to_string()]));
        assert_eq!(bulk.get_param("a"), Some(vec!["1".to_string()]));

        // single + bulk delete, and clear
        assert_eq!(base.remove_param("a", true).get_param("a"), None);
        let trimmed = base.remove_params(&["a".to_string(), "b".to_string()], true);
        assert_eq!(
            trimmed.params(true).keys().cloned().collect::<Vec<_>>(),
            vec!["c".to_string()]
        );
        assert_eq!(base.clear_params().query(), None);
    }

    #[test]
    fn to_str_encode_decode() {
        let url = Url::from_str("https://h/a%20b?q=x%20y#f%20g").unwrap();
        // encode=true ensures a transport-safe string (idempotent, no double-encode).
        assert_eq!(url.to_str(true), "https://h/a%20b?q=x%20y#f%20g");
        // encode=false decodes each component for display.
        assert_eq!(url.to_str(false), "https://h/a b?q=x y#f g");
        // Display uses the encoded form.
        assert_eq!(url.to_string(), "https://h/a%20b?q=x%20y#f%20g");

        // A space in a freshly-built component is encoded by to_str(true).
        // (no authority, so a single slash for the path)
        assert_eq!(Uri::new("https", "/a b").to_str(true), "https:/a%20b");
    }

    #[test]
    fn url_is_a_uri() {
        let url = Url::from_str("https://user@h:8443/p?x=1#f").unwrap();
        let uri: Uri = url.to_uri();
        assert_eq!(uri.scheme(), "https");
        assert_eq!(uri.authority(), Some("user@h:8443"));
        assert_eq!(uri.path(), "/p");
        assert_eq!(uri.to_string(), "https://user@h:8443/p?x=1#f");
        // `From` conversions are available too.
        let via_from = Uri::from(&url);
        assert_eq!(via_from, uri);
    }

    #[test]
    fn percent_encode_decode_round_trip() {
        assert_eq!(percent_encode("a b/c?d"), "a%20b%2Fc%3Fd");
        assert_eq!(percent_decode("a%20b%2Fc%3Fd").unwrap(), "a b/c?d");
        assert_eq!(percent_encode("safe-._~"), "safe-._~");
        assert!(percent_decode("%zz").is_err());
        assert!(percent_decode("%2").is_err());
    }
}
