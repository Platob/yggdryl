# TOML

One natural record document backed by the shared Rust codec.

## Contract

| | |
| --- | --- |
| Root | one table as a sorted `Record`; repeated writes are byte-identical |
| Proves | strings, `i64`, `f64`, booleans, arrays, tables, four date/time forms |
| Lacks | null, non-string keys, scalar root, streams, private markers |
| Exact | decimal scale, binary, string temporals, Struct order need a [`Field`](../types/field.md) |
| Selector | Python `cls=Scalar`, JavaScript `{ scalar: true }`; omitted returns natural mappings |
| Limits | byte, depth, decoded-node, document; nullable binding options, core defaults |
| Errors | name TOML and a byte offset; `validate_for_write` rejects before a destination opens |
| `IOBase` | `from_io` / `into_io` infer TOML and outer [coding](../coding/index.md) from the media type |

## Use

Rust returns `Scalar`; bindings redirect native mappings through the same codec.

=== "Rust"

    ```rust
    use yggdryl::{Scalar};
    use yggdryl::text::toml;

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
    from yggdryl import Scalar
    from yggdryl.text import toml

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

## Inferring entry point

`from_toml_scalar`, `from_toml_scalar_with_field`, and `into_toml_scalar` are TOML's [inferring entry points](index.md), answering a `Record`.

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
    from yggdryl import Scalar
    from yggdryl.text import toml

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

Unspellable Scalars are rejected before a destination opens, never encoded into a side format.

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

A Struct Field yields a row `Sequence` in Rust; bindings restore field names, and Python may add `cls=YourDataclass`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar};
    use yggdryl::text::toml;

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

    from yggdryl import Field, types
    from yggdryl.text import toml

    row = types.struct(
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

### Dates and times

Every native temporal carries a `TimeUnit` and a non-null [`Timezone`](../types/numeric.md).

| value | TOML behavior |
| --- | --- |
| offset date-time | `DateTime64` instant; a local date-time uses `Timezone::NAIVE` |
| date, local time | zone-free |
| outside the grammar, or named zone | ISO string or count, never a rewritten offset |
| bindings | closest lossless native temporal; `Scalar` keeps the rest |

## Documents and streams

Rust `from_utf8`, `from_bytes`, `from_reader` decode one document; `into_utf8`, `into_bytes`, `into_writer` encode one. Binding `loads` takes content, paths, descriptors, file URLs, and readers; `dump` returns bytes or text or writes directly.

=== "Rust"

    ```rust
    use yggdryl::text::toml;

    let value = toml::from_utf8("id = 1\n")?;
    let mut destination = Vec::new();
    toml::into_writer(&value, &mut destination)?;

    assert_eq!(toml::from_bytes(&destination)?, value);
    ```

=== "Python"

    ```python
    import io

    from yggdryl.text import toml

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

`Formatting::indented(n)` lays array items out vertically; `Formatting::compact()` adds no layout.

=== "Rust"

    ```rust
    use yggdryl::text::Formatting;
    use yggdryl::{Scalar};
    use yggdryl::text::toml;

    let value = Scalar::from_record([(
        "items",
        Scalar::from_sequence([
            Scalar::from(1_i64),
            Scalar::from(2_i64),
            Scalar::from(3_i64),
        ]),
    )])?;
    let laid_out =
        toml::into_utf8_with_formatting(&value, Formatting::indented(2))?;
    let compact = toml::into_utf8_with_formatting(&value, Formatting::compact())?;

    assert_ne!(laid_out, compact);
    assert_eq!(toml::from_utf8(&laid_out)?, value);
    ```

=== "Python"

    ```python
    from yggdryl.text import toml

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

Keys are always quoted, so dots, spaces, and syntax-like names round-trip unchanged.

## Placeholders

Opt-in, inside quoted strings, substituted after parsing and before Field interpretation; see [Placeholders](placeholders.md).

=== "Rust"

    ```rust
    use yggdryl::text::{Format, Loading, Placeholders};
    use yggdryl::Scalar;

    let loading = Loading::new().with_placeholders(
        Placeholders::new()
            .with_variable("HOST", Scalar::from("db.internal"))
            .with_variable("PORT", Scalar::from(5432_i64)),
    );
    let value = yggdryl::text::from_utf8_with(
        "host = \"{{ HOST }}\"\nport = \"{{ PORT }}\"\n",
        Format::Toml,
        &loading,
    )?;

    assert_eq!(value.get_key_str("host").and_then(Scalar::as_utf8), Some("db.internal"));
    assert_eq!(value.get_key_str("port"), Some(&Scalar::from(5432_i64)));
    ```

=== "Python"

    ```python
    from yggdryl.text import toml

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

## Edges

- empty or comment-only document -> empty `Record`.
- null, non-record root, non-string key, `i64` overflow, excessive depth -> `validate_for_write` error, no partial destination.
- an existing file name as text -> parsed as TOML; fails as a bare word.
- Rust `_all` forms -> exactly one value; bindings expose no `loads_all`, `dump_all`, or streams.
- user key spelled like a private marker -> ordinary data.
- placeholder mapping -> wins over the environment, never read unless enabled.
- Arrow batches -> [Text records](../media/text.md), which refuse keyed merge.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test text toml::
    cargo test --features "parquet iceberg" -p yggdryl --lib text::toml::
    cargo bench -p yggdryl --bench text -- codec/toml
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/text/toml
    python/.venv/bin/python python/benchmarks/text.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/text/toml.test.js
    npm run --prefix node bench:text
    ```
