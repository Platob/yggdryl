# Strings & bytes

One string family in five layouts, four binary spellings, the version and URL values, and the regex that turns named captures into a schema.

## Contract

A string is a layout, the [charset](../charset/index.md) its bytes are written
in, and how long its values may be. Each layout has two spellings: the `string`
name is the general one, and the `utf8` name is the same layout when its
charset is UTF-8, which is the default. Case, `_`, `-` and spaces are ignored,
so `large_utf8`, `largeutf8` and `LargeString` are one datatype.

| layout | UTF-8 spelling | general spelling | with a charset | Arrow storage |
| --- | --- | --- | --- | --- |
| 32-bit offsets | `utf8` | `string` | `string(windows-1252)` | `Utf8` / `Binary` |
| fixed width | `fixed_utf8(n)` | `fixed_string(n)` | `fixed_string(windows-1252,8)` | `FixedSizeBinary(n)` |
| view | `utf8_view` | `string_view` | `string_view(windows-1252)` | `Utf8View` / `BinaryView` |
| 64-bit offsets | `large_utf8` | `large_string` | `large_string(windows-1252)` | `LargeUtf8` / `LargeBinary` |
| view, large | `large_utf8_view` | `large_string_view` | `large_string_view(windows-1252)` | `Utf8View` / `BinaryView` |

Unbounded UTF-8 in the first, third and fourth rows is `Utf8`, `Utf8View` and
`LargeUtf8` - the datatypes Arrow itself names - so `string` *is* `utf8` rather
than a second spelling of it. Unbounded US-ASCII is [`ascii`](ascii.md), and
`fixed_string(us-ascii,n)` is `ascii(n)`: that repertoire has a family of its
own, and one fact has one owner.

| spelling | datatype | also parsed as |
| --- | --- | --- |
| `utf8` | `Utf8` | `string`, `str`, `text`, `varchar`, `nvarchar`, `char`, `character varying` |
| `utf8(n)` | a string of at most `n` bytes | `varchar(n)`, `string(n)` |
| `fixed_utf8(n)` | a string of exactly `n` bytes | `char(n)`, `fixed_string(n)` |
| `large_utf8` | `LargeUtf8` | `large_string` |
| `utf8_view` | `Utf8View` | `string_view` |
| `large_utf8_view` | a large-declared view | `large_string_view` |
| `binary` | `Binary` | `bytes`, `varbinary`, `blob`, `bytea` |
| `large_binary` | `LargeBinary` | - |
| `binary_view` | `BinaryView` | - |
| `fixed_size_binary(n)` | `FixedSizeBinary(n)` | `fixed_binary(n)` |
| `version` | `Version` | - |
| `url` | `Url` | - |

## Charsets and bounds

A bound counts **stored bytes**, not scalars: that is what the buffer holds and
what Arrow's offsets measure. One number carries both readings - the exact
width on a fixed layout, the maximum on every other - because a string is one
shape or the other.

```rust
use yggdryl::types::StringLayout;
use yggdryl::{Charset, DataType};

// A charset or a bound is what makes a string its own datatype.
let latin = DataType::from_str("string(windows-1252,32)")?;
let parameters = latin.string_parameters().expect("a string datatype");
assert_eq!(parameters.layout(), StringLayout::String);
assert_eq!(parameters.charset(), Charset::Cp1252);
assert_eq!(parameters.max(), Some(32));

// Plain UTF-8 is the datatype it already was.
assert_eq!(DataType::from_str("string")?, DataType::Utf8);
// And every string answers one question about its charset.
assert_eq!(DataType::Utf8.charset(), Some(Charset::Utf8));
assert_eq!(DataType::Ascii.charset(), Some(Charset::Ascii));

// The bound is bytes: five scalars are five windows-1252 bytes and seven UTF-8 ones.
let bounded = DataType::from_str("string(windows-1252,5)")?;
assert_eq!(bounded.scalar("Grüße")?.as_str(), Some("Grüße"));
assert!(DataType::from_str("utf8(5)")?.scalar("Grüße").is_err());
```

A value holds UTF-8 whatever charset it arrived in - the bytes are decoded once,
at the seam - and remembers the charset it is *written* in, so it goes back out
the way it came. Bytes arriving at a column that names a charset are
transcribed rather than refused: an unassigned byte reads as its ISO 8859-1
scalar, which is what the WHATWG Encoding Standard's own index maps it to.

```rust
use yggdryl::{DataType, Scalar};

let dtype = DataType::from_str("string(windows-1252)")?;
let value = dtype.scalar(Scalar::from(vec![0x47_u8, 0x72, 0xFC, 0xDF, 0x65]))?;
assert_eq!(value.as_str(), Some("Grüße"));

// `0x81` is unassigned in windows-1252, and still reads.
let recovered = dtype.scalar(Scalar::from(vec![0x6F_u8, 0x6B, 0x81]))?;
assert_eq!(recovered.as_str(), Some("ok\u{0081}"));
```

Arrow is told the truth about the bytes: UTF-8 rides its string layouts, every
other charset rides the matching binary layout, and what Arrow cannot say - the
charset, the bound, and which of the two view layouts this is, since Arrow has
one - rides the `yggdryl.string` extension document on the field.

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

The major and minor are strict; the patch is best effort. A tail stating a
number is that number, whether it states it as `.250` or as a case-insensitive
FIX service pack `sp250`. A tail stating no number - a qualifier, a fourth
component, an extension pack - folds into the patch's sixteen bits through the
crate's stable XXH3 rather than refusing the version. A folded patch is an
identity rather than a quantity: the same tail always reads as the same
version, but it orders arbitrarily against a stated patch, two unlike tails can
fold together, and the canonical text states the fold rather than the tail.

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
| Text | one to three decimal components; `5.0.0` renders as `5` |
| Patch tail | `.250` and `sp250` state 250; any other tail folds to `1..=65535` via XXH3 |
| Storage | `Utf8` holding the canonical spelling, extension name `yggdryl.version` |
| Sorting | Arrow string order stays lexicographic; Rust `Ord`, Python comparisons, and JavaScript `compare` use numeric order |

The parser reads a compact FIX service pack itself, so `5.0SP2` is `5.0.2` and
`5.0sp250` is `5.0.250`, case-insensitively and with no separately stored
qualifier.

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
- `varchar(255)` -> `utf8(255)`; a string length is a bound this crate stores.
- `binary(16)` -> the length parses and is dropped; a binary layout holds no bound.
- `char(8)` -> `fixed_utf8(8)`, blank-padded as SQL means it; a bare `char` declares no width and so is `utf8`.
- `fixed_string` with no width -> refused; the width is what makes a string fixed.
- `utf8(windows-1252)` -> refused; the `utf8` spellings declare their charset in the name.
- `string_view(us-ascii)` -> refused by name; US-ASCII text is [`ascii`](ascii.md)'s.
- Text a declared charset has no bytes for -> held as a value, refused when the column is written, naming the scalar. The value door counts rather than judges, because [`transcribe`](../charset/index.md) recovers damage precisely by answering scalars the charset does not assign.
- Case, `_`, `-` and spaces are ignored in a spelling, so `LargeUtf8`, `large_utf8` and `large_string` are one [datatype](datatype.md).
- Bytes merged with text -> bytes win, text wins next, per the merge order in [Field](field.md).
- `005.0.000` -> the canonical `5`; trailing zero components are omitted.
- A version whose major or minor exceeds `255`, or whose major is not a decimal number -> refused at the first bad byte.
- Empty text, a leading sign, leading whitespace, or a letter where the major belongs -> refused.
- A fourth component, an empty component, a qualifier, or a patch above `65535` -> folded into the patch, not refused.
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

### Allocations per row

A string value is a `SmolStr`, which holds its first twenty-three bytes inline
and shares an `Arc<str>` above them. That threshold is the whole allocation
story: below it a cell is free to build and free to clone, above it it is one
shared handle and one copy. These counts are measured with the counting
allocator over a thousand-row column and asserted in
`rust/tests/allocations.rs`, not timed - a count is the same on every machine,
and a timing is not.

| path | cell ≤ 23 bytes | cell > 23 bytes |
| --- | ---: | ---: |
| `utf8` build | one buffer per column | one buffer per column |
| `utf8` read | 0 | 1 |
| `string(windows-1252)` build | one buffer per column | one buffer per column |
| `string(windows-1252)` read, all-ASCII cell | 0 | 1 |
| `string(windows-1252)` read, transcoded cell | 0 | 2 |

A column's build cost is its buffers and not its rows: the payload is measured
with [`Charset::encoded_len`](../charset/index.md) before a byte of it is
built, so the count is equal at sixteen rows and at sixteen thousand. The
`read` row above 23 bytes is one `Arc<str>` per cell out of a buffer Arrow
already shares; removing it needs a storage handle that does not fit
[`Scalar`](scalar.md)'s pinned forty-eight bytes, so it is recorded rather
than spent.

A transcoded cell over 23 bytes costs two because the text is built once and
copied once into the shared handle, and `String` and `Arc<str>` have different
layouts, so no conversion between them is free.

### Timings

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
