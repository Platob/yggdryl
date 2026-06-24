# yggdryl (Rust core)

The pure-Rust core of [**yggdryl**](https://github.com/Platob/yggdryl).

It provides small, dependency-free value types — [`Uri`], [`Url`] and
[`Version`] — a generic [`FromInput`] trait, multi-valued query `params`, and
URL-safe percent-encoding helpers. The Python and Node extensions in the wider
project are thin wrappers around this crate, so behaviour is identical in every
language.

```rust
use yggdryl::{FromInput, Params, Uri, Url, Version, percent_encode};

// `from_str(value, safe)` — safe = full validation, false = fast/lenient.
let uri = Uri::from_str("https://example.com/a%20b?p=2#intro", true).unwrap();
assert_eq!(uri.scheme(), "https");

let url = Url::from_str("https://user:pw@example.com:8443/api?a=1&a=2", true).unwrap();
assert_eq!(url.host(), "example.com");
assert_eq!(url.port(), Some(8443));

// Multi-valued query params (key -> list of values), percent-decoded.
assert_eq!(url.params().get("a"), Some(&vec!["1".into(), "2".into()]));

// Functional builders leave the original untouched.
let bumped = url.add_param("page", vec!["2".into()]);

// Construct from parts (no string building) and view a Url as a Uri.
let made = Url::new("https", "example.com").with_port(443).with_path("/x");
let as_uri: Uri = made.to_uri();

// Every Version is a `major.minor.patch`; ordering is numeric.
assert!(Version::from_str("1.4.2", true).unwrap() < Version::from_str("1.10.0", true).unwrap());

assert_eq!(percent_encode("a b"), "a%20b");
```
