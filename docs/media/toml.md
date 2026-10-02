# TOML

A TOML document as one [`Scalar`](../types/scalar.md): its root is always a record, and TOML's own dates and times arrive typed; the bindings answer native objects unless asked for the scalar.

## Overview

| | |
| --- | --- |
| Declared by | `application/toml`, `.toml` |
| Build | default |
| Rust | `yggdryl::toml`: `from_utf8`, `from_bytes`, `from_reader` and their `_with_field` and `_with_limits` forms; `into_utf8`, `into_bytes`, `into_writer`; `from_toml_scalar`, `from_toml_scalar_with_field` and `into_toml_scalar` at the crate root; placeholders through `yggdryl::text::from_utf8_with` and a `Loading` |
| Python | `yggdryl.toml`: `loads`, `dumps`, `dump` |
| JavaScript | `toml`: `loads`, `load`, `dumps`, `dump`, and `loadStream`/`dumpStream` - the one document over a Node stream |
| Handle | a `.toml` handle reads through `read_scalar` or `read_arrow` and writes whole through `write_scalar` or `write_arrow` ([Structured values](../holder/index.md#structured-values)) |
| Limits | one document, so no multi-document doors (`*_all`); a record at the root; integers within `i64`; no null |

## Read

A document parses into the record its root table is, every key a column: TOML spells its own dates, times and date-times, so they read typed with no field, and a declared `field` types the rest exactly as it types [JSON](json.md#read). Depth, input bytes and nodes are bounded, and a parse error names the byte it stopped at. `{{ }}` placeholders resolve after parsing, only when asked, by the rule [YAML](yaml.md#read) states.

=== "Rust"

    ```rust
    use yggdryl::text::{self, Format, Loading, Placeholders};
    use yggdryl::{from_toml_scalar, toml, Scalar};

    let source = "title = \"yggdryl\"\nsince = 2024-01-02\n\n[owner]\nname = \"Ada\"\n";
    let value = toml::from_utf8(source)?;
    assert_eq!(value.get_key_str("title").and_then(Scalar::as_str), Some("yggdryl"));
    // A table is a nested record, and a TOML date needs no field to be one.
    let owner = value.get_key_str("owner").expect("a table");
    assert_eq!(owner.get_key_str("name").and_then(Scalar::as_str), Some("Ada"));
    assert_eq!(value.get_key_str("since"), Some(&Scalar::date32(19_724)));

    // The crate-root inferring entry point answers the same record, from text or bytes.
    assert_eq!(from_toml_scalar(source.as_bytes())?, value);

    // Placeholders resolve after parsing, and only when asked.
    let loading = Loading::new().with_placeholders(Placeholders::new().with_variable("PORT", Scalar::from(8080)));
    let resolved = text::from_utf8_with("port = \"{{ PORT }}\"\n", Format::Toml, &loading)?;
    assert_eq!(resolved.get_key_str("port"), Some(&Scalar::from(8080)));
    ```

=== "Python"

    ```python
    import datetime

    from yggdryl import Scalar, toml

    source = 'title = "yggdryl"\nsince = 2024-01-02\n\n[owner]\nname = "Ada"\n'

    # A table is a nested record, and a TOML date needs no field to be one.
    value = toml.loads(source)
    assert value == {"owner": {"name": "Ada"}, "since": datetime.date(2024, 1, 2), "title": "yggdryl"}
    assert toml.loads(source, cls=Scalar).kind == "struct"

    # Placeholders resolve after parsing, and only when asked.
    assert toml.loads('port = "{{ PORT }}"\n', placeholders={"PORT": 8080}) == {"port": 8080}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, toml } = require('yggdryl')

    const source = 'title = "yggdryl"\nsince = 2024-01-02\n\n[owner]\nname = "Ada"\n'

    // A table is a nested record, and a TOML date needs no field to be one; a
    // date has no JavaScript spelling, so it stays the native value.
    const value = toml.loads(source)
    assert.equal(value.title, 'yggdryl')
    assert.deepEqual(value.owner, { name: 'Ada' })
    assert.ok(value.since instanceof Scalar)
    assert.equal(value.since.kind, 'date32')

    // Placeholders resolve after parsing, and only when asked.
    assert.deepEqual(toml.loads('port = "{{ PORT }}"\n', { placeholders: { PORT: 8080 } }), { port: 8080 })
    ```

## Write

A record writes one quoted key per line, sorted - a Python `dict` crosses as a map and keeps its own order - and a nested record as an inline table, so the same value always writes the same bytes. What TOML cannot spell is refused by name rather than dropped: a root that is not a record, a null, an integer beyond `i64`.

=== "Rust"

    ```rust
    use yggdryl::{into_toml_scalar, toml, Scalar};

    let value = toml::from_utf8("title = \"yggdryl\"\ncount = 3\n\n[owner]\nname = \"Ada\"\n")?;

    // One quoted key per line, sorted, a nested record inline.
    let encoded = toml::into_utf8(&value)?;
    assert_eq!(encoded, "\"count\" = 3\n\"owner\" = {\"name\" = \"Ada\"}\n\"title\" = \"yggdryl\"\n");
    assert_eq!(into_toml_scalar(&value)?, encoded);
    assert_eq!(toml::from_utf8(&encoded)?, value);

    // A root that is not a record has no TOML spelling.
    let refused = toml::into_utf8(&Scalar::from_sequence([Scalar::from(1), Scalar::from(2)])).unwrap_err();
    assert!(refused.to_string().contains("root must be a record"), "{refused}");
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import toml

    value = toml.loads('title = "yggdryl"\ncount = 3\n\n[owner]\nname = "Ada"\n')

    # One quoted key per line, sorted, a nested record inline.
    encoded = toml.dumps(value)
    assert encoded == b'"count" = 3\n"owner" = {"name" = "Ada"}\n"title" = "yggdryl"\n'
    assert toml.loads(encoded) == value

    # What TOML cannot spell is refused by name.
    with pytest.raises(ValueError, match="root must be a record"):
        toml.dumps([1, 2])
    with pytest.raises(ValueError, match="cannot represent null"):
        toml.dumps({"note": None})
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { toml } = require('yggdryl')

    const value = toml.loads('title = "yggdryl"\ncount = 3\n\n[owner]\nname = "Ada"\n')

    // One quoted key per line, sorted, a nested record inline.
    const encoded = toml.dumps(value)
    assert.ok(Buffer.isBuffer(encoded))
    assert.equal(encoded.toString(), '"count" = 3\n"owner" = {"name" = "Ada"}\n"title" = "yggdryl"\n')
    assert.deepEqual(toml.loads(encoded), value)

    // What TOML cannot spell is refused by name.
    assert.throws(() => toml.dumps([1, 2]), /root must be a record/)
    assert.throws(() => toml.dumps({ note: null }), /cannot represent null/)
    ```

## Performance

### Rust codec

One release run of the `text` Criterion target's `codec/toml` group on one Linux x86_64 container - Intel Xeon @ 2.80 GHz, 4 cores, 15 GiB; rustc 1.97.0, release profile (thin LTO, one codegen unit) - medians of 100 samples, on 2026-10-02. The record is `{symbol: "MSFT", quantity: 120, price: 413.75, tags: ["closing", "auction"]}`; the typed one a `decimal256(76, 4)`, a `datetime64(s, UTC)` and three bytes, read back under its field. The [JSON](json.md#rust-codec), [YAML](yaml.md#rust-codec) and [XML](xml.md#rust-codec) pages carry the same rows from the same run.

| door | document | bytes | median | throughput |
| --- | --- | ---: | ---: | ---: |
| `into_bytes` | record | 84 | 573.4 ns | 139.7 MiB/s |
| `into_utf8` | record | 84 | 582.6 ns | 137.5 MiB/s |
| `into_toml_scalar` | record | 84 | 570.0 ns | 140.5 MiB/s |
| `into_writer` | record | 84 | 377.6 ns | 212.1 MiB/s |
| `from_bytes` | record | 84 | 2.682 us | 29.9 MiB/s |
| `from_toml_scalar` | record | 84 | 2.615 us | 30.6 MiB/s |
| `from_utf8` | record | 84 | 2.493 us | 32.1 MiB/s |
| `from_reader` | record | 84 | 2.630 us | 30.5 MiB/s |
| `text::from_utf8_inferred`, the format read off the content | record | 84 | 2.915 us | 27.5 MiB/s |
| `into_bytes` | typed | 69 | 973.0 ns | 67.6 MiB/s |
| `from_bytes_with_field` | typed | 69 | 3.549 us | 18.5 MiB/s |
| `from_toml_scalar_with_field` | typed | 69 | 3.430 us | 19.2 MiB/s |
| `from_bytes` | 49 levels deep | 108 | 15.54 us | 6.63 MiB/s |
| `from_bytes` | 1,024 keys | 16,212 | 543.0 us | 28.5 MiB/s |

```bash
cargo bench -p yggdryl --bench text -- codec/toml/
```

### Bindings

One Windows x86_64 release run, one fixture per runtime; compare routes within a runtime, never Python against Node.

| operation | runtime | TOML |
| --- | --- | ---: |
| field class encode | CPython | 140 us |
| field class decode | CPython | 363 us |
| bytes decode | CPython | 26.5 us |
| reader redirect | CPython | 27.9 us |
| writer redirect | CPython | 145 us |
| natural document decode | Node | 14.3 ms |
| natural document emit | Node | 15.5 ms |

```bash
python/.venv/bin/python python/benchmarks/text.py --iterations 10000
npm run --prefix node bench:text
```
