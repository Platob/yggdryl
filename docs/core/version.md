# Version

A small `major.minor.patch` value type. It parses one, two or three dot-separated
non-negative integers (omitted components default to `0`), exposes the parts,
orders numerically, and renders back to its canonical string — the same surface in
Python, Node and Rust.

## Parse

`from_str` accepts `major[.minor[.patch]]`; every component must be a non-negative
integer and there may be at most three. It raises / returns an error on malformed
input.

=== "Python"

    ```python
    import yggdryl

    v = yggdryl.Version.from_str("1.4.2")
    assert (v.major, v.minor, v.patch) == (1, 4, 2)
    # Omitted components default to 0.
    assert yggdryl.Version.from_str("2") == yggdryl.Version(2, 0, 0)
    ```

=== "Node"

    ```javascript
    const { Version } = require("yggdryl");

    const v = Version.fromStr("1.4.2");
    // [v.major, v.minor, v.patch] === [1, 4, 2]
    // Omitted components default to 0.
    Version.fromStr("2").equals(new Version(2, 0, 0)); // true
    ```

=== "Rust"

    ```rust
    use yggdryl_core::Version;

    let v = Version::from_str("1.4.2")?;
    assert_eq!((v.major(), v.minor(), v.patch()), (1, 4, 2));
    // Omitted components default to 0.
    assert_eq!(Version::from_str("2")?, Version::new(2, 0, 0));
    ```

!!! note
    Parsing always validates: an empty string, more than three components, or a
    non-numeric / negative part is an error — there is no lenient mode. Leading
    zeros (`01.02`) are accepted.

## Construct from parts

Build directly from components; `minor` and `patch` default to `0`.

=== "Python"

    ```python
    import yggdryl

    v = yggdryl.Version(1, 4, 2)
    v = yggdryl.Version(2)          # 2.0.0
    ```

=== "Node"

    ```javascript
    const { Version } = require("yggdryl");

    const v = new Version(1, 4, 2);
    const w = new Version(2, 0, 0);
    ```

=== "Rust"

    ```rust
    use yggdryl_core::Version;

    let v = Version::new(1, 4, 2);
    ```

## Parts

`major` / `minor` / `patch` read the three components.

=== "Python"

    ```python
    import yggdryl

    v = yggdryl.Version.from_str("1.4.2")
    print(v.major, v.minor, v.patch)   # 1 4 2
    ```

=== "Node"

    ```javascript
    const { Version } = require("yggdryl");

    const v = Version.fromStr("1.4.2");
    console.log(v.major, v.minor, v.patch); // 1 4 2
    ```

=== "Rust"

    ```rust
    use yggdryl_core::Version;

    let v = Version::from_str("1.4.2")?;
    println!("{} {} {}", v.major(), v.minor(), v.patch()); // 1 4 2
    ```

## Update

`copy` overrides any component you pass and keeps the rest; `with_major` /
`with_minor` / `with_patch` replace a single field. All return a new value and
never mutate.

=== "Python"

    ```python
    import yggdryl

    v = yggdryl.Version(1, 4, 2)
    assert v.copy(major=2) == yggdryl.Version(2, 4, 2)
    assert v.with_patch(0) == yggdryl.Version(1, 4, 0)
    ```

=== "Node"

    ```javascript
    const { Version } = require("yggdryl");

    const v = new Version(1, 4, 2);
    v.copy(2).equals(new Version(2, 4, 2));     // copy(major, minor, patch)
    v.withPatch(0).equals(new Version(1, 4, 0));
    ```

=== "Rust"

    ```rust
    use yggdryl_core::Version;

    let v = Version::new(1, 4, 2);
    assert_eq!(v.copy(Some(2), None, None), Version::new(2, 4, 2));
    assert_eq!(v.with_patch(0), Version::new(1, 4, 0));
    ```

## Compare

Ordering is numeric and field-major (`major`, then `minor`, then `patch`), so
`1.4.2 < 1.10.0`.

=== "Python"

    ```python
    import yggdryl

    assert yggdryl.Version(1, 4, 2) < yggdryl.Version(1, 10, 0)
    assert yggdryl.Version(2, 0, 0) > yggdryl.Version(1, 99, 99)
    latest = max(yggdryl.Version.from_str(s) for s in ["1.0.5", "1.2.0", "0.9.9"])
    ```

=== "Node"

    ```javascript
    const { Version } = require("yggdryl");

    // compare(other) -> -1 | 0 | 1
    new Version(1, 4, 2).compare(new Version(1, 10, 0)); // -1
    new Version(2, 0, 0).compare(new Version(1, 99, 99)); // 1
    new Version(1, 4, 2).equals(new Version(1, 4, 2));    // true
    ```

=== "Rust"

    ```rust
    use yggdryl_core::Version;

    assert!(Version::new(1, 4, 2) < Version::new(1, 10, 0));
    assert!(Version::new(2, 0, 0) > Version::new(1, 99, 99));
    ```

!!! tip
    Because the type is fully ordered, you can sort a list of versions or take the
    `min`/`max` directly. In Python it supports the standard comparison operators;
    in Node use `compare(other)` (`-1` / `0` / `1`) or `equals(other)`.

## Render

`to_str` (Rust) / `str()` (Python) / `toString()` (Node) renders the canonical
`major.minor.patch` string — the inverse of `from_str`.

=== "Python"

    ```python
    import yggdryl

    assert str(yggdryl.Version.from_str("3")) == "3.0.0"
    assert str(yggdryl.Version(1, 4, 2)) == "1.4.2"
    ```

=== "Node"

    ```javascript
    const { Version } = require("yggdryl");

    Version.fromStr("3").toString();       // "3.0.0"
    new Version(1, 4, 2).toString();       // "1.4.2"
    ```

=== "Rust"

    ```rust
    use yggdryl_core::Version;

    assert_eq!(Version::from_str("3")?.to_string(), "3.0.0");
    assert_eq!(Version::new(1, 4, 2).to_str(true), "1.4.2");
    ```

## Mapping

A `Version` also converts to and from a component map (`major` / `minor` /
`patch`), and serializes as its canonical string (pickle in Python,
`JSON.stringify` in Node, serde in Rust).

=== "Python"

    ```python
    import yggdryl

    v = yggdryl.Version.from_mapping({"major": "1", "minor": "4"})  # 1.4.0
    fields = v.to_mapping()                                          # {"major": "1", ...}
    ```

=== "Node"

    ```javascript
    const { Version } = require("yggdryl");

    const v = Version.fromMapping({ major: "1", minor: "4" }); // 1.4.0
    const fields = v.toMapping();
    JSON.stringify(v);                                          // "\"1.4.0\""
    ```

=== "Rust"

    ```rust
    use std::collections::BTreeMap;
    use yggdryl_core::Version;

    let fields = BTreeMap::from([("major".into(), "1".into()), ("minor".into(), "4".into())]);
    let v = Version::from_mapping(&fields)?;   // 1.4.0
    let back = v.to_mapping();
    ```

## Next

- [Media types](media.md) — single MIME types and layered media stacks
- [URI & URL](url.md) — the URL types built on the same parsing conventions
- Back to [Getting started](../getting-started.md)
