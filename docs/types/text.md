# Text & bytes

Three UTF-8 spellings, four binary spellings, the version and URL values, and the regex that turns named captures into a schema.

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
| `url` | `Url` | - |

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

`Version` holds three numeric components in four bytes: `major: u8`, `minor: u8`,
and `patch: u16`. Parsing accepts one to three decimal components and rendering
omits trailing zero components. Equality, hashing and ordering use the numeric
tuple. Python and JavaScript expose the same immutable native value.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar, Version};

    let version = "005.0.00300".parse::<Version>()?;
    assert_eq!(version, Version::new(5, 0, 300));
    assert_eq!(version.to_string(), "5.0.300");
    assert_eq!((version.major(), version.minor(), version.patch()), (5, 0, 300));
    assert_eq!(std::mem::size_of::<Version>(), 4);
    assert!(Version::new(5, 0, 2) < Version::new(5, 0, 10));

    let field = Field::new("version", DataType::Version, false);
    assert_eq!(field.scalar("5.0.300")?, Scalar::from(version));
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Scalar, Version, types
    from yggdryl.text import json

    dtype = DataType("version")
    field = types.version("version", nullable=False)
    version = Version.from_str("005.0.00300")
    value = json.loads('"5.0.300"', field=field, cls=Scalar)
    assert dtype.kind == "text"
    assert value.as_py() == version == Version(5, 0, 300)
    assert (version.major, version.minor, version.patch) == (5, 0, 300)
    assert Version(5, 0, 2) < Version(5, 0, 10)
    assert value.into_arrow_scalar(field).as_py() == "5.0.300"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Version, fields, json } = require('yggdryl')

    const dtype = new DataType('version')
    const field = fields.version('version', { nullable: false })
    const version = Version.fromStr('005.0.00300')
    const value = json.loads('"5.0.300"', {
      field,
      scalar: true,
    })
    assert.equal(dtype.kind, 'text')
    assert.ok(value.asJs().equals(version))
    assert.ok(version.equals(new Version(5, 0, 300)))
    assert.deepEqual([version.major, version.minor, version.patch], [5, 0, 300])
    assert.equal(new Version(5, 0, 2).compare(new Version(5, 0, 10)), -1)
    assert.equal(value.intoArrowScalar(field), '5.0.300')
    ```

| rule | behaviour |
| --- | --- |
| Layout | exactly four bytes: `u8` major, `u8` minor, `u16` patch; omitted parts are zero |
| Kind | `text`; `VersionField`, `types.version`, and `fields.version` declare this datatype |
| Bounds | major and minor `0..=255`; patch `0..=65535` |
| Ordering | numeric tuple: `5 < 5.0.2 < 5.0.10 < 5.1` |
| Text | one to three decimal components; no tag or qualifier; `5.0.0` renders as `5` |
| Storage | `Utf8` holding the canonical spelling, extension name `yggdryl.version` |
| Sorting | Arrow string order stays lexicographic; Rust `Ord`, Python comparisons, and JavaScript `compare` use numeric order |

FIX dictionary intake translates protocol service-pack spellings such as
`5.0SP2` to `5.0.2` before constructing a `Version`. The generic parser accepts
numeric components only.

<div class="ygg-pg" data-playground="versions" markdown="1">
Explore numeric parts, canonical text, hashes and rejected inputs from the
native Version example corpus.
</div>

## Locations

`Url` is the crate's own [`Url`](../holder/index.md) carried as a column: a value read out of a table is a value a handle can be opened from, not prose that happens to look like one. Parsing canonicalizes and validates, so a column holds one spelling per location and nothing that is not a location.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar, Url};

    let location = Url::from_str("HTTPS://example.com/a%2fb")?;
    assert_eq!(location.to_string(), "https://example.com/a%2Fb");
    // A bare platform path is a `file:` URL, which is what a local handle is.
    assert_eq!(Url::from_str("/lake/part.txt")?.to_string(), "file:///lake/part.txt");

    let field = Field::new("location", DataType::Url, false);
    assert_eq!(field.scalar("HTTPS://example.com/a%2fb")?, Scalar::from(location));
    // Relative text names no location, so it is not one.
    assert!(field.scalar("./relative").is_err());
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Scalar, types
    from yggdryl.text import json

    dtype = DataType("url")
    field = types.url("location", nullable=False)
    value = json.loads('"HTTPS://example.com/a"', field=field, cls=Scalar)
    assert dtype.kind == "text"
    assert value.as_py() == "https://example.com/a"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields, json } = require('yggdryl')

    const dtype = new DataType('url')
    const value = json.loads('"HTTPS://example.com/a"', {
      field: fields.url('location', { nullable: false }),
      scalar: true,
    })
    assert.equal(dtype.kind, 'text')
    assert.equal(value.asJs(), 'https://example.com/a')
    ```

| rule | behaviour |
| --- | --- |
| Kind | `text`; the aliases are `UrlField`, `types.url`, `fields.url` |
| Value | `crate::Url` behind one shared pointer, so a row clone moves a reference count rather than a URI |
| Storage | `Utf8` holding the canonical text, extension name `yggdryl.url` |
| Ordering | the canonical text's, which is Arrow's own string order; there is no numeric component to sort by |
| Default | `file:///`, the shortest URL the validator accepts, because a location has no zero |
| Merging | only with itself: merging into text would drop the validation that makes it a URL |

## Edges

- `from_regex(pattern, false)` -> every capture stays `utf8`.
- Invalid regex syntax -> datatype error.
- An expression beyond the shared datatype recursion limit -> datatype error.
- `fixed_size_binary(-1)` -> refused, the width must be non-negative.
- `varchar(255)`, `binary(16)` -> the length parses and is dropped, the datatype stays variable.
- Case, `_`, `-` and spaces are ignored in a spelling, so `LargeUtf8` and `large_utf8` are one [datatype](datatype.md).
- Bytes merged with text -> bytes win, text wins next, per the merge order in [Field](field.md).
- `005.0.000` -> the canonical `5`; trailing zero components are omitted.
- A version whose major or minor exceeds `255`, or patch exceeds `65535` -> refused at the first bad byte.
- A fourth component, empty component, sign, whitespace, or qualifier -> refused.
- Fractional or out-of-range constructor arguments in Python or JavaScript -> refused without narrowing.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --lib types::regex
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- field::binary
    cargo test -p yggdryl --lib types::tests::version
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/media/test_text_lines.py -k regex
    python/.venv/bin/python -m pytest python/tests/types/test_version.py
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="regex captures" node/tests/types/datatype.test.js
    node --test node/tests/types/version.test.js
    ```

## Performance

AMD Ryzen 5 150, 12 logical CPUs, Windows; Rust 1.96, Python 3.12.13 and Node
24.18, release builds. Rust reports Criterion point estimates; Python reports
the median of five runs of 10,000 iterations; Node reports throughput over
2,000 iterations after warmup. These harnesses measure different boundaries.

| operation | Rust | Python native boundary | JavaScript native boundary |
| --- | ---: | ---: | ---: |
| parse | 28.8 ns | 187.6 ns/op | 320,631 ops/s |
| numeric compare | 3.78 ns | 142.5 ns/op | 1,160,631 ops/s |
| native parts construction | 3.85 ns | 186.0 ns/op | 558,722 ops/s |
| native value into `Scalar` | — | 317.9 ns/op | 38,527 ops/s |
| `Scalar` into host `Version` | — | 82.7 ns/op | 142,733 ops/s |

The Rust maximum-width parse (`255.255.65535`) measured 35.7 ns. The four-byte
value needs no heap allocation for parsing or comparison; host wrappers and
text or Arrow projections have their own allocation costs.

Rust's native-parts case also reads all three accessors. The binding cases
measure constructor calls. The counting-allocator test
`version_parse_compare_and_render_allocate_nothing` checks the core allocation
claim independently of these timings.

```bash
cargo bench -p yggdryl --bench types -- version --warm-up-time 0.1 --measurement-time 0.2 --sample-size 10
python/.venv/bin/python python/benchmarks/types/version.py --iterations 10000
YGGDRYL_BENCH_ITERATIONS=2000 npm run --prefix node bench:types
```

On Windows, use `python/.venv/Scripts/python.exe` and set
`$env:YGGDRYL_BENCH_ITERATIONS = '2000'` before the Node command.
