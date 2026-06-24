# yggdryl-version

A standalone `major.minor.patch` `Version` type for the
[**yggdryl**](https://github.com/Platob/yggdryl) project, built on the
[`yggdryl-core`](https://crates.io/crates/yggdryl-core) parsing traits
(`FromInput` / `ToOutput`).

```rust
use yggdryl_version::{FromInput, ToOutput, Version};

let v = Version::from_str("1.4.2", true).unwrap();
assert_eq!((v.major(), v.minor(), v.patch()), (1, 4, 2));
assert!(Version::from_str("1.4.2", true).unwrap() < Version::from_str("1.10.0", true).unwrap());

// Render back to a component mapping (inverse of `from_mapping`).
assert_eq!(v.to_mapping().get("minor"), Some(&"4".to_string()));
```
