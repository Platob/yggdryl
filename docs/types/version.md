# Version

Three numeric components in four bytes: the canonical, numerically ordered version a column declares.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `DataType::Version` and the value `Version`: `major: u8`, `minor: u8`, `patch: u16` |
| Validates | At the value door: the major and the minor are decimal numbers under 256, and text naming no major is refused at the first bad byte |
| Lazy | Nothing - four bytes, parsed once and held as numbers |
| Cached | The Arrow projection of a [`Field`](field.md); the value itself needs no heap at all |
| Refuses | Empty text, a major or minor that is not a decimal number under 256, and a fractional or out-of-range constructor argument in either binding |
| Kind | `DataTypeKind::Text`, id `0x63` - in the text family's range, after the eighteen [string](text/string.md) leaves, because `as_u8` is a wire contract laid out by family |
| Bindings | Python and JavaScript expose the same immutable native value, `Version`, and the same three accessors |

## DataType

One parameterless variant, so the enum is the constructor and `version` is the
one spelling. It is text without being a [string](text/string.md): the value
parses, canonicalizes and orders itself, which is exactly what a `utf8` column
of version-looking text cannot do.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeId, DataTypeKind};

    assert_eq!(DataType::from_str("VERSION")?, DataType::Version);
    assert_eq!(DataType::Version.to_string(), "version");
    assert_eq!(DataType::Version.id(), DataTypeId::Version);
    assert_eq!(DataType::Version.kind(), DataTypeKind::Text);
    assert_eq!(DataTypeId::Version.as_u8(), 0x63);
    assert!(!DataTypeId::Version.is_parameterized());
    assert_eq!(DataTypeId::Version.fixed_byte_width(), None);

    assert_eq!(DataType::Version.into_json()?, r#"{"type":"version"}"#);
    assert_eq!(DataType::from_json(r#"{"type":"version"}"#)?, DataType::Version);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    dtype = DataType("VERSION")
    assert dtype == DataType("version")
    assert str(dtype) == "version"
    assert dtype.id == "version"
    assert dtype.kind == "text"
    assert dtype.fixed_byte_width is None
    assert dtype.into_dict() == {"type": "version"}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const dtype = DataType.from('VERSION')
    assert.ok(dtype.equals(DataType.from('version')))
    assert.equal(dtype.toString(), 'version')
    assert.equal(dtype.id, 'version')
    assert.equal(dtype.kind, 'text')
    assert.equal(dtype.fixedByteWidth, null)
    assert.deepEqual(dtype.toJSON(), { type: 'version' })
    ```

## Field

`VersionField` is the typed marker, and there is nothing to pass:
`unit(name, nullable)` is the whole constructor. `types.version` and
`fields.version` declare the same column, nullable unless the call says
otherwise.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar, Version, VersionField};

    let release = VersionField::unit("release", false);
    assert_eq!(release.name(), "release");
    assert_eq!(release.dtype(), &DataType::Version);
    assert!(!release.is_nullable());
    assert_eq!(release.to_field(), Field::new("release", DataType::Version, false));

    // The field is where a caller's text becomes a stored value.
    let field = Field::new("release", DataType::Version, false);
    assert_eq!(field.scalar("5.0.300")?, Scalar::from(Version::new(5, 0, 300)));
    // A required column refuses absence; a nullable one reads it as null.
    assert!(field.scalar(Scalar::Null).is_err());
    assert_eq!(
        Field::new("release", DataType::Version, true).scalar(Scalar::Null)?,
        Scalar::Null,
    );
    ```

=== "Python"

    ```python
    import pytest

    import yggdryl

    from yggdryl import Field, Version

    release = yggdryl.version("release", nullable=False)
    assert isinstance(release, Field)
    assert release.name == "release"
    assert str(release.dtype) == "version"
    assert release.nullable is False
    assert release.scalar("5.0.300").as_py() == Version(5, 0, 300)

    # A required column refuses absence; a nullable one reads it as null.
    assert yggdryl.version("release").scalar("").is_null()
    with pytest.raises(ValueError, match="non-nullable field received null"):
        release.scalar("")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, Scalar, Version, fields } = require('yggdryl')

    const release = fields.version('release', { nullable: false })
    assert.ok(release instanceof Field)
    assert.equal(release.name, 'release')
    assert.equal(release.dtype.toString(), 'version')
    assert.equal(release.nullable, false)
    assert.ok(Scalar.from('5.0.300', { field: release }).asJs().equals(new Version(5, 0, 300)))

    // A required column refuses absence; a nullable one reads it as null.
    assert.equal(Scalar.from('', { field: fields.version('release') }).kind, 'null')
    assert.throws(() => Scalar.from('', { field: release }), /non-nullable field received null/)
    ```

## Scalar

`Scalar::Version(Version)` holds the numeric tuple, not the text it was read
from. Parsing accepts one to three decimal components and rendering omits
trailing zero ones, so `5`, `5.0` and `5.0.0` are one value with one spelling.
Equality, hashing and ordering read the tuple.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Scalar, Version};

    let version = "005.0.00300".parse::<Version>()?;
    assert_eq!(version, Version::new(5, 0, 300));
    assert_eq!(version.to_string(), "5.0.300");
    assert_eq!((version.major(), version.minor(), version.patch()), (5, 0, 300));
    assert_eq!(std::mem::size_of::<Version>(), 4);
    assert_eq!("4.4.0".parse::<Version>()?.to_string(), "4.4");
    assert_eq!("5".parse::<Version>()?, "5.0".parse::<Version>()?);
    assert_eq!(Version::MAX, Version::new(255, 255, 65_535));

    // The door reads the text once and stores the numbers.
    assert_eq!(DataType::Version.scalar("005.000.001")?, Scalar::from(Version::new(5, 0, 1)));
    assert_eq!(DataType::Version.default_value()?, Scalar::Version(Version::MIN));
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Version

    version = Version.from_str("005.0.00300")
    assert version == Version(5, 0, 300)
    assert str(version) == "5.0.300"
    assert (version.major, version.minor, version.patch) == (5, 0, 300)
    assert str(Version.from_str("4.4.0")) == "4.4"
    assert Version.from_str("5") == Version.from_str("5.0")

    value = DataType("version").scalar("005.000.001")
    assert value.as_py() == Version(5, 0, 1)
    assert value.kind == "version"
    assert value.family == "text"
    assert DataType("version").default_scalar().as_py() == Version(0, 0, 0)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Version } = require('yggdryl')

    const version = Version.fromStr('005.0.00300')
    assert.ok(version.equals(new Version(5, 0, 300)))
    assert.equal(version.toString(), '5.0.300')
    assert.deepEqual([version.major, version.minor, version.patch], [5, 0, 300])
    assert.equal(Version.fromStr('4.4.0').toString(), '4.4')
    assert.ok(Version.fromStr('5').equals(new Version(5)))

    const value = DataType.from('version').scalar('005.000.001')
    assert.ok(value.asJs().equals(new Version(5, 0, 1)))
    assert.equal(value.kind, 'version')
    assert.equal(value.family, 'text')
    ```

## Arrow storage

`Utf8` holding the canonical spelling, under the extension name
`yggdryl.version`, which is what keeps a version column a version column across
a round trip. A cast into the column canonicalizes every cell on the way in, so
`005.000.001` is stored as `5.0.1`; a numeric source is refused by name.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, StringArray};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::FieldValue as _;
    use yggdryl::{ArrowCastOptions, DataType, Field};

    let field = Field::new("release", DataType::Version, false);
    let arrow = field.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.version");
    assert_eq!(Field::from_arrow_field(&arrow)?, field);

    // The cast canonicalizes every cell on the way in.
    let text: ArrayRef = Arc::new(StringArray::from(vec!["005.000.001"]));
    let stored = field.cast_arrow_array(text, ArrowCastOptions::new().with_safe(false))?;
    let cells = stored.as_any().downcast_ref::<StringArray>().unwrap();
    assert_eq!(cells.value(0), "5.0.1");
    ```

=== "Python"

    ```python
    import pyarrow as pa

    import yggdryl

    from yggdryl import Field

    release = yggdryl.version("release", nullable=False)
    arrow = release.into_arrow()
    assert arrow.type == pa.string()
    assert arrow.metadata[b"ARROW:extension:name"] == b"yggdryl.version"
    assert Field.from_arrow(arrow) == release

    # The cast canonicalizes every cell on the way in.
    assert release.cast_arrow_array(pa.array(["005.000.001"])).to_pylist() == ["5.0.1"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { fields } = require('yggdryl')

    const release = fields.version('release', { nullable: false })
    const text = arrow.vectorFromArray(['005.000.001'], new arrow.Utf8())

    // The cast canonicalizes every cell on the way in.
    assert.deepEqual(Array.from(release.castArrowArray(text, { safe: false })), ['5.0.1'])
    // A scalar crosses as the canonical spelling its column stores.
    assert.equal(fields.version('release').dtype.scalar('5.0.300').intoArrowScalar(release), '5.0.300')
    ```

## Numeric order, not lexicographic

`5.0.2` is before `5.0.10` because the comparison reads three numbers, where
the stored text does not: Arrow's string order over the same column stays
lexicographic. Rust `Ord`, Python's comparison operators and JavaScript's
`compare` all read the tuple.

=== "Rust"

    ```rust
    use yggdryl::Version;

    assert!(Version::new(5, 0, 2) < Version::new(5, 0, 10));
    assert!(Version::new(5, 0, 300) < Version::new(5, 1, 0));
    // The stored text does not order that way, which is why the value does.
    assert!("5.0.10" < "5.0.2");
    ```

=== "Python"

    ```python
    from yggdryl import Version

    assert Version(5, 0, 2) < Version(5, 0, 10)
    assert Version(5, 0, 300) < Version(5, 1, 0)
    assert sorted([Version(5, 0, 10), Version(5, 0, 2)]) == [Version(5, 0, 2), Version(5, 0, 10)]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Version } = require('yggdryl')

    assert.equal(new Version(5, 0, 2).compare(new Version(5, 0, 10)), -1)
    assert.equal(new Version(5, 0, 300).compare(new Version(5, 1, 0)), -1)
    ```

## The major and minor are strict, the patch is best effort

A tail stating a number is that number, whether it states it as `.250` or as a
case-insensitive FIX service pack `sp250`, so `5.0SP2` is `5.0.2` with no
separately stored qualifier. A tail stating no number - a qualifier, a fourth
component, an extension pack - folds into the patch's sixteen bits through the
crate's stable XXH3 rather than refusing the version. A folded patch is an
identity rather than a quantity: the same tail always reads as the same
version, but it orders arbitrarily against a stated patch, two unlike tails can
fold together, and the canonical text states the fold rather than the tail.

=== "Rust"

    ```rust
    use yggdryl::Version;

    assert_eq!("5.0SP2".parse::<Version>()?, Version::new(5, 0, 2));
    assert_eq!("5.0sp250".parse::<Version>()?, Version::new(5, 0, 250));

    // A tail stating no number folds, so anything naming a major parses.
    let qualified = "1.0-rc1".parse::<Version>()?;
    assert_eq!((qualified.major(), qualified.minor()), (1, 0));
    assert_eq!(qualified, "1.0-rc1".parse::<Version>()?);
    assert_ne!(qualified, "1.0-rc2".parse::<Version>()?);

    // The major and the minor are not best effort.
    assert!("".parse::<Version>().is_err());
    assert!("256.0".parse::<Version>().is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import Version

    assert Version.from_str("5.0SP2") == Version(5, 0, 2)
    assert Version.from_str("5.0sp250") == Version(5, 0, 250)

    # A tail stating no number folds, so anything naming a major parses.
    qualified = Version.from_str("1.0-rc1")
    assert (qualified.major, qualified.minor) == (1, 0)
    assert qualified == Version.from_str("1.0-rc1")
    assert qualified != Version.from_str("1.0-rc2")

    # The major and the minor are not best effort.
    for refused in ("", "256.0", "v1"):
        with pytest.raises(ValueError, match="version"):
            Version.from_str(refused)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Version } = require('yggdryl')

    assert.ok(Version.fromStr('5.0SP2').equals(new Version(5, 0, 2)))
    assert.ok(Version.fromStr('5.0sp250').equals(new Version(5, 0, 250)))

    // A tail stating no number folds, so anything naming a major parses.
    const qualified = Version.fromStr('1.0-rc1')
    assert.deepEqual([qualified.major, qualified.minor], [1, 0])
    assert.ok(qualified.equals(Version.fromStr('1.0-rc1')))
    assert.ok(!qualified.equals(Version.fromStr('1.0-rc2')))

    // The major and the minor are not best effort.
    for (const refused of ['', '256.0', 'v1']) {
      assert.throws(() => Version.fromStr(refused), /version/i)
    }
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
| Default | `0`, the numeric minimum, which is what a required column fills with |
| Merging | only with itself: merging into text would drop the canonicalization |
| Sorting | Arrow string order stays lexicographic; Rust `Ord`, Python comparisons, and JavaScript `compare` use numeric order |

<div class="ygg-pg" data-playground="versions" markdown="1">
Explore numeric parts, canonical text, hashes and rejected inputs from the
native Version example corpus.
</div>

## Edges

- `005.0.000` -> the canonical `5`; a version whose major or minor exceeds `255`, or whose major is not a decimal number -> refused at the first bad byte.
- A fourth component, an empty component, a qualifier, or a patch above `65535` -> folded into the patch, never refused.
- Empty text is no `Version`, and the datatype door reads an empty cell entering a non-text column as absence: a nullable column holds null, a required one refuses it.
- Fractional or out-of-range constructor arguments in Python or JavaScript -> refused without narrowing.
- The canonical default is `0` (`Version::MIN`), which is what a [strict-nullability](cast.md) cast writes where a required column holds a null.
- A version merges only with itself; merged with `utf8` -> refused naming both, because the canonicalization is what the column is for.
- An Arrow column under `yggdryl.version` over a storage that is not `Utf8` -> a foreign field wearing our name, imported as its storage.
- A numeric Arrow source cast into a version column -> refused naming the datatype; text is the only source a version reads.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- version::ordered
    cargo test --features "iceberg internals parquet" --manifest-path rust/Cargo.toml -p yggdryl --test root -- version::internal
    cargo bench --manifest-path rust/Cargo.toml --bench types -- version
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_version.py
    python/.venv/bin/python python/benchmarks/types/version.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/version.test.js
    npm run --prefix node bench:types
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
