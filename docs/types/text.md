# Text & bytes

Three UTF-8 spellings, four binary spellings, the version value, and the regex that turns named captures into a schema.

## Contract

| spelling | datatype | also parsed as |
| --- | --- | --- |
| `utf8` | `Utf8` | `string`, `str`, `text`, `varchar`, `nvarchar`, `char`, `character varying` |
| `large_utf8` | `LargeUtf8` | `large_string` |
| `utf8_view` | `Utf8View` | `string_view` |
| `binary` | `Binary` | `bytes`, `varbinary`, `blob`, `bytea` |
| `large_binary` | `LargeBinary` | - |
| `binary_view` | `BinaryView` | - |
| `fixed_size_binary(n)` | `FixedSizeBinary(n)` | `fixed_binary(n)` |
| `version` | `Version` | - |

## Use

`DataType::from_regex` builds one Struct from a byte regex's named captures, in capture order.

=== "Rust"

    ```rust
    use yggdryl::DataType;

    let dtype = DataType::from_regex(
        r"\[(?<level>[A-Z]+)\] id=(?<id>\d+)",
        true,
    )?;
    assert_eq!(dtype.field("level")?.dtype(), &DataType::Utf8);
    assert_eq!(dtype.field("id")?.dtype(), &DataType::Int64);
    assert!(dtype.field("id")?.is_nullable());
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    dtype = DataType.from_regex(r"\[(?<level>[A-Z]+)\] id=(?<id>\d+)")
    assert dtype["level"].dtype == DataType("utf8")
    assert dtype["id"].dtype == DataType("int64")
    assert dtype["id"].nullable
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const dtype = DataType.fromRegex(
      '\\[(?<level>[A-Z]+)\\] id=(?<id>\\d+)',
    )
    assert.equal(dtype.field('level').dtype.id, 'utf8')
    assert.equal(dtype.field('id').dtype.id, 'int64')
    assert.equal(dtype.field('id').nullable, true)
    ```

## Regex captures

| rule | behaviour |
| --- | --- |
| Nullability | every capture field is nullable |
| Autotyping argument | required in Rust, defaults to `true` in Python and JavaScript |
| Typed captures | boolean, integer, finite float, ISO date, time, datetime |
| Broad captures | a capture such as `\S+` stays `utf8` |
| Rows read | none, so [plain-text records](../media/text.md) publish a schema before opening a source |

## Versions

`Version` is a generic, numerically ordered value, 16 bytes wide. Parsing trims trailing numeric
zeroes and canonicalizes appended, dot-introduced, and hyphen-introduced qualifiers.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar, Version};

    let version = "5.0.SP1".parse::<Version>()?;
    assert_eq!(version.to_string(), "5.0SP1");
    assert_eq!(std::mem::size_of::<Version>(), 16);

    let field = Field::new("version", DataType::Version, false);
    assert_eq!(field.scalar("5.0.SP1")?, Scalar::from(version));
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Scalar, types
    from yggdryl.text import json

    dtype = DataType("version")
    field = types.version("version", nullable=False)
    value = json.loads('"5.0.SP1"', field=field, cls=Scalar)
    assert dtype.kind == "text"
    assert value.as_py() == "5.0SP1"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields, json } = require('yggdryl')

    const dtype = new DataType('version')
    const value = json.loads('"5.0.SP1"', {
      field: fields.version('version', { nullable: false }),
      scalar: true,
    })
    assert.equal(dtype.kind, 'text')
    assert.equal(value.asJs(), '5.0SP1')
    ```

| rule | behaviour |
| --- | --- |
| Layout | required `u8` major, nullable `u8` minor, nullable fixed 14-byte patch tail |
| Kind | `text`; the aliases are `VersionField`, `types.version`, `fields.version` |
| Ordering | hyphens are pre-release, other qualifiers post-release: `5.0-rc1 < 5 < 5.0SP1`, `SP2 < SP10` |
| Storage | `Utf8` holding the canonical spelling, extension name `yggdryl.version` |
| Sorting | Arrow string order stays lexicographic; `Version::cmp` is the ordering contract |

## Edges

- `from_regex(pattern, false)` -> every capture stays `utf8`.
- Invalid regex syntax -> datatype error.
- An expression beyond the shared datatype recursion limit -> datatype error.
- `fixed_size_binary(-1)` -> refused, the width must be non-negative.
- `varchar(255)`, `binary(16)` -> the length parses and is dropped, the datatype stays variable.
- Case, `_`, `-` and spaces are ignored in a spelling, so `LargeUtf8` and `large_utf8` are one [datatype](datatype.md).
- Bytes merged with text -> bytes win, text wins next, per the merge order in [Field](field.md).
- `5.0.SP1` -> the canonical `5.0SP1`; trailing numeric zeroes are trimmed away.
- A version whose first two components exceed `u8`, or a later one `u16` -> refused at the first bad byte.
- A canonical patch tail over 14 ASCII bytes -> refused.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --lib types::regex
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- field::binary
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/media/test_text_lines.py -k regex
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="regex captures" node/tests/types/datatype.test.js
    ```

## Performance

Version parse and compare on an AMD Ryzen 5 150, Rust 1.96.1, release build. Two deliberately light
cases covering the two hot operations this value owns.

| operation | estimate |
| --- | ---: |
| parse | 35.1 ns |
| compare | 49.3 ns |

```bash
cargo bench -p yggdryl --bench types -- version --warm-up-time 0.1 --measurement-time 0.2 --sample-size 10
```
