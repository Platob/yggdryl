# TOML

TOML is one natural record document backed by the shared Rust codec.

## Text media and Arrow batches

A TOML document is a raw structured value, not line-record media. For streamed
Arrow batches with explicit overwrite and append behavior, use [Text
media](text.md#plain-text-records). Text deliberately refuses keyed
merge because a line has no stable row identity.

## Raw shared-Scalar access

Rust returns `Scalar`; Python and JavaScript redirect native mappings through
the same codec.

=== "Rust"

    ```rust
    use yggdryl::{toml, Scalar};

    let value = toml::from_utf8(
        "title = \"yggdryl\"\ncount = 3\n\n[owner]\nname = \"Ada\"\n"
    )?;
    let encoded = toml::into_utf8(&value)?;

    assert_eq!(
        value.get_key_str("title").and_then(Scalar::as_utf8),
        Some("yggdryl")
    );
    assert_eq!(toml::from_utf8(&encoded)?, value);
    ```

=== "Python"

    ```python
    from yggdryl import Scalar, toml

    source = 'title = "yggdryl"\ncount = 3\n\n[owner]\nname = "Ada"\n'
    natural = toml.loads(source)
    value = toml.loads(source, cls=Scalar)

    assert value.kind == "record"
    assert value.as_py() == natural == {
        "count": 3,
        "owner": {"name": "Ada"},
        "title": "yggdryl",
    }
    assert toml.loads(toml.dumps(value)) == natural
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, toml } = require('yggdryl')

    const source = 'title = "yggdryl"\ncount = 3\n\n[owner]\nname = "Ada"\n'
    const natural = toml.loads(source)
    const value = toml.loads(source, { scalar: true })
    const encoded = toml.dumps(value)

    assert.ok(value instanceof Scalar)
    assert.equal(value.kind, 'record')
    assert.deepEqual(value.asJs(), natural)
    assert.ok(Buffer.isBuffer(encoded))
    assert.deepEqual(toml.loads(encoded), natural)
    ```

Tables become sorted `Record` values. Empty or comment-only TOML is an empty
Record. Python `cls=Scalar` and JavaScript `{ scalar: true }` expose that exact
tree; omitting the selector returns natural mappings. Deterministic order makes
repeated writes byte-identical.

### One inferring entry point

`yggdryl::from_toml_scalar`, `from_toml_scalar_with_field` and
`into_toml_scalar` are TOML's [inferring entry points](text.md#raw-document-codecs)
over `from_bytes`, `from_bytes_with_field` and `into_utf8`; the answer is a
`Record` because a TOML root is a table. Text that names an existing file is
parsed as TOML, so a bare file name fails as the bare word it is.

=== "Rust"

    ```rust
    use yggdryl::{from_toml_scalar, into_toml_scalar, Scalar};

    let value = from_toml_scalar(
        "title = \"yggdryl\"\ncount = 3\n\n[owner]\nname = \"Ada\"\n"
    )?;
    let encoded = into_toml_scalar(&value)?;

    assert_eq!(from_toml_scalar(encoded.as_bytes())?, value);
    assert_eq!(
        value.get_key_str("title").and_then(Scalar::as_utf8),
        Some("yggdryl")
    );
    ```

=== "Python"

    ```python
    from yggdryl import Scalar, toml

    source = 'title = "yggdryl"\ncount = 3\n\n[owner]\nname = "Ada"\n'
    value = toml.loads(source, cls=Scalar)
    encoded = toml.dumps(value)

    assert toml.loads(encoded, cls=Scalar) == value
    assert value.as_py()["title"] == "yggdryl"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { toml } = require('yggdryl')

    const source = 'title = "yggdryl"\ncount = 3\n\n[owner]\nname = "Ada"\n'
    const value = toml.loads(source, { scalar: true })
    const encoded = toml.dumps(value)

    assert.ok(toml.loads(encoded, { scalar: true }).equals(value))
    assert.equal(value.asJs().title, 'yggdryl')
    ```

## Natural values and exact Fields

TOML proves strings, signed 64-bit integers, 64-bit floats, booleans, arrays,
tables, and its four date/time forms. It has no null, arbitrary-key mapping,
scalar document root, or multi-document stream.

| input or native value | natural TOML behavior |
| --- | --- |
| table / inline table | sorted `Record` |
| date | `Date32` |
| local time | `Time32` or `Time64` |
| local or offset date-time | `DateTime64` with explicit timezone |
| `D128`, `D256` | quoted scale-preserving string |
| bytes / geospatial | quoted base64 string |
| duration | quoted ISO duration |
| null | error |
| integer outside `i64` | error |
| Mapping with non-string keys | error |

There is no private marker envelope or private tagged table. A user key with that
same spelling is ordinary application data. Scalars TOML cannot spell are rejected
before a destination is opened, not encoded into a hidden side format.

TOML's own date/time syntax produces temporal Scalars without a schema. Exact
decimal width and scale, binary, string-encoded temporal values, and Struct
child order require a [`Field`](field.md).

=== "Rust"

    ```rust
    use yggdryl::{toml, DataType, Field, Scalar};

    let amount = Field::new("amount", DataType::decimal128(8, 2)?, false);
    let row = Field::new(
        "row",
        DataType::from_fields([amount])?,
        false,
    );
    let decoded = toml::from_utf8_with_field("amount = '12.50'\n", &row)?;

    assert_eq!(decoded.as_sequence().unwrap()[0], Scalar::d128(1_250, 2));
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import Field, fields, toml

    row = fields.struct(
        "row",
        [Field("amount", "decimal128(8, 2)", nullable=False)],
        nullable=False,
    )

    assert toml.loads("amount = '12.50'\n", field=row) == {
        "amount": Decimal("12.50")
    }
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields, toml } = require('yggdryl')

    const row = fields.struct(
      'row',
      [fields.decimal128('amount', 8, 2, { nullable: false })],
      { nullable: false },
    )
    const decoded = toml.loads("amount = '12.50'\n", { field: row })

    assert.equal(decoded.amount.kind, 'd128')
    assert.equal(decoded.amount.unscaled, 1250n)
    ```

A Struct Field canonicalizes its Record input into a row `Sequence` in Rust.
Python and JavaScript restore the field names; Python can also pass both
`field=` and `cls=YourDataclass` to materialize its dataclass wrapper.

### Dates and times

Every native temporal carries a `TimeUnit` and non-null `Timezone`.
`Timezone::NAIVE` marks a local wall-clock value. TOML offset date-times become
`DateTime64` instants; local date-times use the same variant with `NAIVE`.
Date and time-of-day values are zone-free.

Native values use TOML date/time tokens when their unit and range fit the TOML
grammar. Otherwise they use their natural ISO string or count, which a Field
can interpret exactly. Named zones that TOML cannot express remain ISO strings
instead of being silently rewritten to the zone's current offset.

Bindings use their closest native temporal where it is lossless. The explicit
`Scalar` wrapper retains resolutions and zone semantics the host language
cannot represent.

## Documents and streams

Rust `from_utf8`, `from_bytes`, and `from_reader` decode one TOML document;
`into_utf8`, `into_bytes`, and `into_writer` encode one. The `_all` forms exist
only for generic dispatch and require exactly one value.

Python and JavaScript intentionally do not expose `loads_all`, `dump_all`, or
multi-document streams for TOML. Their `loads` accepts content, paths, and
readers; `dump` returns bytes/text or writes directly.

=== "Rust"

    ```rust
    use yggdryl::toml;

    let value = toml::from_utf8("id = 1\n")?;
    let mut destination = Vec::new();
    toml::into_writer(&value, &mut destination)?;

    assert_eq!(toml::from_bytes(&destination)?, value);
    ```

=== "Python"

    ```python
    import io

    from yggdryl import toml

    destination = io.BytesIO()
    toml.dump({"id": 1}, destination)

    assert toml.loads(destination.getvalue()) == {"id": 1}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { pathToFileURL } = require('node:url')
    const { toml } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-toml-'))
    const target = path.join(root, 'value.toml')
    toml.dump({ id: 1 }, target)

    assert.deepEqual(toml.load(pathToFileURL(target)), { id: 1 })
    fs.rmSync(root, { recursive: true, force: true })
    ```

## Formatting

TOML keeps objects as inline tables and can lay array items out vertically.
`Formatting::indented(n)` indents those nested items for readability;
`Formatting::compact()` requests no extra layout. Whitespace changes, meaning
does not.

=== "Rust"

    ```rust
    use yggdryl::text::Formatting;
    use yggdryl::{toml, Scalar};

    let value = Scalar::from_record([(
        "items",
        Scalar::from_sequence([Scalar::I64(1), Scalar::I64(2), Scalar::I64(3)]),
    )])?;
    let laid_out =
        toml::into_utf8_with_formatting(&value, Formatting::indented(2))?;
    let compact = toml::into_utf8_with_formatting(&value, Formatting::compact())?;

    assert_ne!(laid_out, compact);
    assert_eq!(toml::from_utf8(&laid_out)?, value);
    ```

=== "Python"

    ```python
    from yggdryl import toml

    value = {"items": [1, 2, 3]}
    laid_out = toml.dumps(value, indent=2)
    compact = toml.dumps(value, indent=None)

    assert laid_out != compact
    assert toml.loads(laid_out) == toml.loads(compact) == value
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { toml } = require('yggdryl')

    const value = { items: [1, 2, 3] }
    const laidOut = toml.dumps(value, { indent: 2 })
    const compact = toml.dumps(value, { indent: null })

    assert.notDeepEqual(laidOut, compact)
    assert.deepEqual(toml.loads(laidOut), toml.loads(compact))
    ```

Keys are always quoted, so dots, spaces, and names that resemble TOML syntax
round-trip without changing table structure.

## Placeholders

TOML placeholders are opt-in and must be inside quoted strings. Substitution
runs after parsing and before optional Field interpretation.

=== "Rust"

    ```rust
    use yggdryl::text::{Format, Loading, Placeholders};
    use yggdryl::Scalar;

    let loading = Loading::new().with_placeholders(
        Placeholders::new()
            .with_variable("HOST", Scalar::from("db.internal"))
            .with_variable("PORT", Scalar::I64(5432)),
    );
    let value = yggdryl::text::from_utf8_with(
        "host = \"{{ HOST }}\"\nport = \"{{ PORT }}\"\n",
        Format::Toml,
        &loading,
    )?;

    assert_eq!(value.get_key_str("host").and_then(Scalar::as_utf8), Some("db.internal"));
    assert_eq!(value.get_key_str("port"), Some(&Scalar::I64(5432)));
    ```

=== "Python"

    ```python
    from yggdryl import toml

    document = '[database]\nhost = "{{ HOST }}"\nport = "{{ PORT }}"\n'
    value = toml.loads(
        document,
        placeholders={"HOST": "db.internal", "PORT": 5432},
    )

    assert value["database"] == {"host": "db.internal", "port": 5432}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { toml } = require('yggdryl')

    const document = 'host = "{{ HOST }}"\nport = "{{ PORT }}"\n'
    const value = toml.loads(document, {
      placeholders: { HOST: 'db.internal', PORT: 5432 },
    })

    assert.deepEqual(value, { host: 'db.internal', port: 5432 })
    ```

A supplied mapping wins over environment lookup. The environment is never read
unless explicitly enabled. See
[structured text](text.md#jinja-style-placeholders) for syntax and measured
overhead.

## Limits and errors

Nullable binding options expose Rust's byte, depth, decoded-node, and document
limits under snake-case/camel-case names; omission uses the core defaults.
`validate_for_write` checks the natural TOML projection before writing, so a
null, non-record root, non-string table key, out-of-range integer, or excessive
depth cannot leave a partial destination.

Parse and validation errors name TOML and a byte offset.

## `IOBase` and content coding

Generic `from_io` / `into_io` infer TOML and any outer coding from the handle's
media type. Python accepts `PathLike` sources and destinations; JavaScript
accepts paths, file descriptors, file URLs, and streams.
