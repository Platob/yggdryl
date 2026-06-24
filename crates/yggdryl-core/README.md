# yggdryl-core

Dependency-free foundations shared by the [**yggdryl**](https://github.com/Platob/yggdryl)
crates. The `Uri` / `Url` types live in
[`yggdryl-url`](https://crates.io/crates/yggdryl-url), which builds on this crate.

It provides:

- the generic [`FromInput`] parsing trait (with `Input`, `Mapping`, `Params`) —
  every parse takes a `safe` flag (`true` validates fully, `false` is faster and
  lenient);
- URL-safe percent-encoding (`percent_encode` / `percent_decode`) plus the
  lower-level component helpers `yggdryl-url` builds on;
- the `Version` (`major.minor.patch`) value type, numerically ordered.

```rust
use yggdryl_core::{percent_encode, percent_decode, FromInput, Version};

assert_eq!(percent_encode("a b"), "a%20b");
assert_eq!(percent_decode("a%20b").unwrap(), "a b");

assert!(Version::from_str("1.4.2", true).unwrap() < Version::from_str("1.10.0", true).unwrap());
```
