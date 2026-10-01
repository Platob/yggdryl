# Version

A sixteen-bit major and minor and an optional text patch: the canonical, naturally ordered version a column declares.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `DataType::Version` and the value `Version`: `major: u16`, `minor: u16`, `patch: Option<SmolStr>` - `int`, `int` and `str` or `None` in Python, `number`, `number` and `string` or `null` in JavaScript |
| Validates | At the value door: the text opens with a decimal major under 65536, a `.` followed by digits is a minor under 65536, and anything else is refused at the first bad byte; the patch is whatever the tail states |
| Lazy | Nothing - parsed once and held as two numbers and the patch text |
| Cached | The Arrow projection of a [`Field`](field.md); a patch of up to 23 bytes is held inline, and a longer one is one shared allocation its clones share |
| Refuses | Empty text, text that does not open with a decimal major under 65536, a minor whose digits pass 65535, a fractional or out-of-range constructor argument in either binding, and a negative number patch |
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
`unit(name, nullable)` is the whole constructor. `yggdryl.version` and
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
    assert_eq!(field.scalar("5.0.300")?, Scalar::from(Version::new(5, 0, Some("300"))));
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

`Scalar::Version(Version)` holds the major and the minor as numbers and the
patch as text, not the text the version was read from. A patch stating a number
is held as its digits without leading zeros, and zero states no patch, so `5`,
`5.0` and `5.0.0` are one value with one spelling and `5.0.007` is `5.0.7`. A
constructor takes the patch as text or, in Python and JavaScript, as a
non-negative whole number, and holds it as the parser would. Equality and
hashing read the three parts; the order is [natural](#natural-order-not-lexicographic).

=== "Rust"

    ```rust
    use yggdryl::{DataType, Scalar, Version};

    let version = "005.0.00300".parse::<Version>()?;
    assert_eq!(version, Version::new(5, 0, Some("300")));
    assert_eq!(version.to_string(), "5.0.300");
    assert_eq!((version.major(), version.minor(), version.patch()), (5, 0, Some("300")));
    assert_eq!("4.4.0".parse::<Version>()?.to_string(), "4.4");
    assert_eq!("5".parse::<Version>()?, "5.0.0".parse::<Version>()?);
    assert_eq!("5.0.007".parse::<Version>()?.to_string(), "5.0.7");
    // A constructed patch is held as a parsed one: digits without their leading
    // zeros, and zero as no patch at all.
    assert_eq!(Version::new(5, 0, Some("0300")), version);
    assert_eq!(Version::new(5, 0, Some("0")).patch(), None);
    assert_eq!(Version::new(u16::MAX, u16::MAX, None).to_string(), "65535.65535");

    // The door reads the text once and stores the parts.
    assert_eq!(DataType::Version.scalar("005.000.001")?, Scalar::from(Version::new(5, 0, Some("1"))));
    assert_eq!(DataType::Version.default_value()?, Scalar::Version(Version::MIN));
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType, Version

    version = Version.from_str("005.0.00300")
    assert version == Version(5, 0, 300) == Version(5, 0, "300")
    assert str(version) == "5.0.300"
    assert (version.major, version.minor, version.patch) == (5, 0, "300")
    assert repr(version) == "Version(5, 0, '300')"
    assert str(Version.from_str("4.4.0")) == "4.4"
    assert Version.from_str("5") == Version.from_str("5.0.0") == Version(5)
    assert str(Version.from_str("5.0.007")) == "5.0.7"
    assert Version(5, 0, 0).patch is None
    assert Version(65535, 65535).major == 65535

    # Each part is checked as it is given, never narrowed.
    for parts in ((65536,), (1, -1), (1, 2, -1)):
        with pytest.raises(OverflowError):
            Version(*parts)
    with pytest.raises(TypeError, match="patch"):
        Version(1, 2, 2.5)

    value = DataType("version").scalar("005.000.001")
    assert value.as_py() == Version(5, 0, 1)
    assert value.kind == "version"
    assert value.family == "text"
    assert DataType("version").default_scalar().as_py() == Version(0)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Version } = require('yggdryl')

    const version = Version.fromStr('005.0.00300')
    assert.ok(version.equals(new Version(5, 0, 300)))
    assert.ok(version.equals(new Version(5, 0, '300')))
    assert.equal(version.toString(), '5.0.300')
    assert.deepEqual([version.major, version.minor, version.patch], [5, 0, '300'])
    assert.equal(Version.fromStr('4.4.0').toString(), '4.4')
    assert.ok(Version.fromStr('5.0.0').equals(new Version(5)))
    assert.equal(Version.fromStr('5.0.007').toString(), '5.0.7')
    assert.equal(new Version(5, 0, 0).patch, null)
    assert.equal(new Version(65535, 65535).major, 65535)

    // Each part is checked as it is given, never narrowed.
    assert.throws(() => new Version(65536), /major must be in 0\.\.65535/)
    assert.throws(() => new Version(1, 0.5), /minor/)
    assert.throws(() => new Version(1, 2, -1), /patch/)

    const value = DataType.from('version').scalar('005.000.001')
    assert.ok(value.asJs().equals(new Version(5, 0, 1)))
    assert.equal(value.kind, 'version')
    assert.equal(value.family, 'text')
    ```

## Arrow storage

`Utf8` holding the canonical spelling, under the extension name
`yggdryl.version`, which is what keeps a version column a version column across
a round trip. A cast into the column canonicalizes every cell on the way in, so
`005.000.001` is stored as `5.0.1`, `5.0SP2` as `5.0.2` and `1.0rc1` as
`1.0.rc1`; a numeric source is refused by name.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, StringArray};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{ArrowCastOptions, DataType, Field, Serie};

    let field = Field::new("release", DataType::Version, false);
    let arrow = field.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.version");
    assert_eq!(Field::from_arrow_field(&arrow)?, field);

    // The cast canonicalizes every cell on the way in.
    let text: ArrayRef = Arc::new(StringArray::from(vec!["005.000.001", "5.0SP2", "1.0rc1"]));
    let strict = ArrowCastOptions::new().with_safe(false);
    let stored = Serie::from_arrow_array(Some(&field), text, strict)?;
    let cells = stored.as_utf8().expect("the text column");
    assert_eq!(cells.value(0), Some("5.0.1"));
    assert_eq!(cells.value(1), Some("5.0.2"));
    assert_eq!(cells.value(2), Some("1.0.rc1"));
    ```

=== "Python"

    ```python
    import pyarrow as pa

    import yggdryl

    from yggdryl import Field, Serie

    release = yggdryl.version("release", nullable=False)
    arrow = release.into_arrow()
    assert arrow.type.storage_type == pa.string()
    assert arrow.type.extension_name == "yggdryl.version"
    assert Field.from_arrow(arrow) == release

    # The cast canonicalizes every cell on the way in.
    stored = Serie.from_arrow_array(pa.array(["005.000.001", "5.0SP2", "1.0rc1"]), release)
    assert stored.into_arrow_array().storage.to_pylist() == ["5.0.1", "5.0.2", "1.0.rc1"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Serie, fields } = require('yggdryl')

    const release = fields.version('release', { nullable: false })
    const text = arrow.vectorFromArray(['005.000.001', '5.0SP2', '1.0rc1'], new arrow.Utf8())

    // The cast canonicalizes every cell on the way in.
    const stored = Serie.fromArrowArray(text, release, { safe: false })
    assert.deepEqual(Array.from(stored.intoArrowArray()), ['5.0.1', '5.0.2', '1.0.rc1'])
    // A scalar crosses as the canonical spelling its column stores.
    assert.equal(Serie.fromScalars(release, ['5.0.300']).intoArrowScalar(), '5.0.300')
    ```

## Natural order, not lexicographic

The major, then the minor, then the patch: no patch orders before any patch,
and two patches compare naturally - a run of digits as the number it spells,
leading zeros aside, every other byte as itself, and a patch that ends first
before one that goes on. So `5.0.2 < 5.0.2.1 < 5.0.10` and
`1.0 < 1.0-rc1 < 1.0-rc2 < 1.0-rc10`. Patches that compare equal that way,
`rc01` and `rc1`, fall back to their bytes, which keeps the order total and in
agreement with equality and hashing. The stored text does not order like this:
Arrow's string order over the same column stays lexicographic. Rust `Ord`,
Python's comparison operators and JavaScript's `compare` all read the value.

=== "Rust"

    ```rust
    use yggdryl::Version;

    assert!(Version::new(5, 0, Some("2")) < Version::new(5, 0, Some("10")));
    assert!(Version::new(5, 0, Some("300")) < Version::new(5, 1, None));
    // No patch first, then patches in natural order.
    let ordered = ["1.0", "1.0-rc1", "1.0-rc2", "1.0-rc10", "1.1"]
        .into_iter()
        .map(str::parse::<Version>)
        .collect::<Result<Vec<_>, _>>()?;
    assert!(ordered.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(Version::new(5, 0, Some("2")) < Version::new(5, 0, Some("2.1")));
    assert!(Version::new(5, 0, Some("2.1")) < Version::new(5, 0, Some("10")));
    // `rc01` and `rc1` spell one number, so their bytes break the tie.
    assert!(Version::new(1, 0, Some("rc01")) < Version::new(1, 0, Some("rc1")));
    // The stored text does not order that way, which is why the value does.
    assert!("5.0.10" < "5.0.2");
    ```

=== "Python"

    ```python
    from yggdryl import Version

    assert Version(5, 0, 2) < Version(5, 0, 10)
    assert Version(5, 0, 300) < Version(5, 1)
    # No patch first, then patches in natural order.
    ordered = [Version.from_str(text) for text in ("1.0", "1.0-rc1", "1.0-rc2", "1.0-rc10", "1.1")]
    assert sorted(reversed(ordered)) == ordered
    assert Version(5, 0, 2) < Version(5, 0, "2.1") < Version(5, 0, 10)
    # rc01 and rc1 spell one number, so their bytes break the tie.
    assert Version(1, 0, "rc01") < Version(1, 0, "rc1")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Version } = require('yggdryl')

    assert.equal(new Version(5, 0, 2).compare(new Version(5, 0, 10)), -1)
    assert.equal(new Version(5, 0, 300).compare(new Version(5, 1)), -1)
    // No patch first, then patches in natural order.
    const ordered = ['1.0', '1.0-rc1', '1.0-rc2', '1.0-rc10', '1.1'].map((text) => Version.fromStr(text))
    const sorted = [...ordered].reverse().sort((left, right) => left.compare(right))
    assert.deepEqual(sorted.map(String), ordered.map(String))
    assert.equal(new Version(5, 0, 2).compare(new Version(5, 0, '2.1')), -1)
    assert.equal(new Version(5, 0, '2.1').compare(new Version(5, 0, 10)), -1)
    // rc01 and rc1 spell one number, so their bytes break the tie.
    assert.equal(new Version(1, 0, 'rc01').compare(new Version(1, 0, 'rc1')), -1)
    ```

## The numbers are strict, the patch is best effort

The text opens with a decimal major under 65536, or it is refused. A `.`
followed by digits is the minor, held to the same bound; a `.` followed by
anything else already starts the patch, so `1.x` is `1.0.x`. A tail stating a
number is that number, whether it states it as `.250` or as a compact FIX
service pack straight after the last number read - `sp` in any case, then
digits and nothing else - so `5.0SP2` is `5.0.2`, as is `5SP2`, with no
separately stored qualifier. Any other tail, less one separating `.`, is the patch as written: a
qualifier, a fourth component, an extension pack, a dotted `SP2`. Nothing a
version states past its minor is refused or lost. The canonical text writes a
`.` before a patch opening with a letter, a digit or a `.`, and writes any other
patch straight after the minor, so `1.0rc1` renders as `1.0.rc1` while
`1.0-rc1` stays itself, and every canonical text parses back to its value.

=== "Rust"

    ```rust
    use yggdryl::Version;

    // A tail stating a number is that number.
    assert_eq!("5.0SP2".parse::<Version>()?, Version::new(5, 0, Some("2")));
    assert_eq!("5.0sp00250".parse::<Version>()?.to_string(), "5.0.250");
    assert_eq!("5.0SP0".parse::<Version>()?.patch(), None);

    // Any other tail is the patch as written, so anything naming a major parses.
    let qualified = "1.0-rc1".parse::<Version>()?;
    assert_eq!((qualified.major(), qualified.minor()), (1, 0));
    assert_eq!(qualified.patch(), Some("-rc1"));
    assert_eq!(qualified.to_string(), "1.0-rc1");
    assert_eq!("1.2.2.3".parse::<Version>()?.patch(), Some("2.3"));
    assert_eq!("1.0.SP2".parse::<Version>()?.patch(), Some("SP2"));

    // The canonical text writes a `.` before a patch opening with a letter, a
    // digit or a dot, and nothing before any other.
    assert_eq!("1.0rc1".parse::<Version>()?.to_string(), "1.0.rc1");
    assert_eq!("1.2.-1".parse::<Version>()?.to_string(), "1.2-1");

    // The major and the minor are not best effort.
    assert!("".parse::<Version>().is_err());
    assert!("65536.0".parse::<Version>().is_err());
    assert!("1.65536".parse::<Version>().is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import Version

    # A tail stating a number is that number.
    assert Version.from_str("5.0SP2") == Version(5, 0, 2)
    assert str(Version.from_str("5.0sp00250")) == "5.0.250"
    assert Version.from_str("5.0SP0").patch is None

    # Any other tail is the patch as written, so anything naming a major parses.
    qualified = Version.from_str("1.0-rc1")
    assert (qualified.major, qualified.minor, qualified.patch) == (1, 0, "-rc1")
    assert str(qualified) == "1.0-rc1"
    assert qualified == Version(1, 0, "-rc1")
    assert Version.from_str("1.2.2.3").patch == "2.3"
    assert Version.from_str("1.0.SP2").patch == "SP2"

    # The canonical text writes a "." before a patch opening with a letter, a
    # digit or a dot, and nothing before any other.
    assert str(Version.from_str("1.0rc1")) == "1.0.rc1"
    assert str(Version.from_str("1.2.-1")) == "1.2-1"

    # The major and the minor are not best effort.
    for refused in ("", "65536.0", "1.65536", "v1"):
        with pytest.raises(ValueError, match="version"):
            Version.from_str(refused)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Version } = require('yggdryl')

    // A tail stating a number is that number.
    assert.ok(Version.fromStr('5.0SP2').equals(new Version(5, 0, 2)))
    assert.equal(Version.fromStr('5.0sp00250').toString(), '5.0.250')
    assert.equal(Version.fromStr('5.0SP0').patch, null)

    // Any other tail is the patch as written, so anything naming a major parses.
    const qualified = Version.fromStr('1.0-rc1')
    assert.deepEqual([qualified.major, qualified.minor, qualified.patch], [1, 0, '-rc1'])
    assert.equal(qualified.toString(), '1.0-rc1')
    assert.ok(qualified.equals(new Version(1, 0, '-rc1')))
    assert.equal(Version.fromStr('1.2.2.3').patch, '2.3')
    assert.equal(Version.fromStr('1.0.SP2').patch, 'SP2')

    // The canonical text writes a '.' before a patch opening with a letter, a
    // digit or a dot, and nothing before any other.
    assert.equal(Version.fromStr('1.0rc1').toString(), '1.0.rc1')
    assert.equal(Version.fromStr('1.2.-1').toString(), '1.2-1')

    // The major and the minor are not best effort.
    for (const refused of ['', '65536.0', '1.65536', 'v1']) {
      assert.throws(() => Version.fromStr(refused), /version/i)
    }
    ```

| rule | behaviour |
| --- | --- |
| Layout | `u16` major, `u16` minor, `Option<SmolStr>` patch; an omitted minor is zero and an omitted patch is none |
| Kind | `text`; `VersionField`, `yggdryl.version`, and `fields.version` declare this datatype |
| Bounds | major and minor `0..=65535`; the patch is text of any length, held inline up to 23 bytes |
| Ordering | major, minor, then no patch before any patch and patches in natural order: `5 < 5.0.2 < 5.0.2.1 < 5.0.10 < 5.1`, `1.0 < 1.0-rc1 < 1.0-rc2 < 1.0-rc10` |
| Text | a strict major, a `.minor` wherever a digit follows the `.`, then the patch; `5.0.0` renders as `5` and `5.0.007` as `5.0.7` |
| Patch tail | `.250` and a compact `sp250` state `250`, held as its digits; any other tail, less one leading `.`, is the patch as written; the canonical text writes a `.` before a patch opening with a letter, a digit or a `.` |
| Storage | `Utf8` holding the canonical spelling, extension name `yggdryl.version` |
| Default | `0`, the minimum; a cast never writes it in place of a null |
| Merging | only with itself: merging into text would drop the canonicalization |
| Sorting | Arrow string order stays lexicographic; Rust `Ord`, Python comparisons, and JavaScript `compare` use the natural order |

<div class="ygg-pg" data-playground="versions" markdown="1">
Explore the parts, canonical text, hashes and rejected inputs from the
native Version example corpus.
</div>

## Edges

- `005.0.000` -> the canonical `5`, and `5.0.007` -> `5.0.7`; a version whose major or minor exceeds `65535` (`65536.0`, `1.65536`), or whose major is not a decimal number (`v1`, `.1`) -> refused at the first bad byte, the one that overflows or the first that is no digit.
- A fourth component, a qualifier, or an extension pack -> kept as the patch text, never refused: `1.2.2.3` holds `2.3`, `1.0SP2_EP240` holds `SP2_EP240`. A number past sixteen bits is a number all the same: `1.2.65536` holds `65536`.
- A dotted `SP` is text: `5.0.SP2` holds the patch `SP2` and is not `5.0SP2`. Only the compact service pack states a number, and a constructor takes `sp2` as written.
- A trailing `.` states nothing: `1.` and `1.0.` are `1`. Past that separator the text is the patch, so `1.0.0.` holds `0.`.
- No patch orders first, so `1.0` is before `1.0-rc1`: a qualifier is a patch here, not a SemVer pre-release.
- A patch past 23 bytes costs one shared allocation, which its clones share; up to that it is held inline.
- Empty text is no `Version`, and the datatype door reads an empty cell entering a non-text column as absence: a nullable column holds null, a required one refuses it.
- Fractional or out-of-range constructor arguments in Python or JavaScript -> refused without narrowing, and so is a negative number patch; a Python patch that is not an `int`, a `str` or `None` -> `TypeError` naming `patch`, never stringified.
- The canonical default is `0` (`Version::MIN`). A cast never writes it for a null: a required column refuses the null by path ([Required columns](cast.md#required-columns)).
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

Parsing a patch of up to 23 bytes, comparing and rendering need no heap
allocation, and a longer patch is one shared allocation; host wrappers and
text or Arrow projections have their own allocation costs.

Rust's native-parts case also reads all three accessors. The binding cases
measure constructor calls. The counting-allocator test
`version_allocates_only_a_patch_past_the_inline_capacity` checks the core
allocation claim independently of these timings.

```bash
cargo bench -p yggdryl --bench types -- version --warm-up-time 0.1 --measurement-time 0.2 --sample-size 10
python/.venv/bin/python python/benchmarks/types/version.py --iterations 10000
YGGDRYL_BENCH_ITERATIONS=2000 npm run --prefix node bench:types
```

On Windows, use `python/.venv/Scripts/python.exe` and set
`$env:YGGDRYL_BENCH_ITERATIONS = '2000'` before the Node command.
