# yggdryl-url

URI and URL value types for [**yggdryl**](https://github.com/Platob/yggdryl),
built on the [`yggdryl-core`](https://crates.io/crates/yggdryl-core) foundations.
It re-exports everything from `yggdryl-core`, so a dependent only needs this
crate.

- `Uri` — the generic [RFC 3986](https://www.rfc-editor.org/rfc/rfc3986) shape:
  `scheme:[//authority]path[?query][#fragment]`.
- `Url` — the common subset that always has an authority, decomposed into
  `username`, `password`, `host` and `port`.

```rust
use yggdryl_url::{FromInput, Uri, Url};

// `from_str(value, safe)` — safe = full validation, false = fast/lenient.
let uri = Uri::from_str("https://example.com/a%20b?p=2#intro", true).unwrap();
assert_eq!(uri.scheme(), "https");

let url = Url::from_str("https://user:pw@example.com:8443/api?a=1&a=2", true).unwrap();
assert_eq!(url.host(), "example.com");
assert_eq!(url.port(), Some(8443));

// Multi-valued query params (key -> list of values); `decode` percent-decodes.
assert_eq!(url.params(true).get("a"), Some(&vec!["1".into(), "2".into()]));

// Functional builders; `encode` percent-encodes, `to_str(false)` decodes.
let bumped = url.add_param("page", vec!["2".into()], true);
assert_eq!(bumped.to_str(false), "https://user:pw@example.com:8443/api?a=1&a=2&page=2");

// `+` schemes and Uri <-> Url conversions.
assert_eq!(Uri::from_str("git+ssh://h/r", true).unwrap().scheme_base(), "git");
let as_uri: Uri = Url::new("https", "example.com").with_port(443).to_uri();
```
