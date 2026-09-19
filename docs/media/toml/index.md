# TOML

One natural record document backed by the shared Rust codec.

## Contract

| | |
| --- | --- |
| Root | one table as a sorted `Record`; repeated writes are byte-identical |
| Proves | strings, `i64`, `f64`, booleans, arrays, tables, four date/time forms |
| Lacks | null, non-string keys, scalar root, streams, private markers |
| Exact | decimal scale, binary, string temporals, Struct order need a [`Field`](../../types/field.md) |
| Selector | Python `cls=Scalar`, JavaScript `{ scalar: true }`; omitted returns natural mappings |
| Limits | byte, depth, decoded-node, document; nullable binding options, snake_case in Python and camelCase in JavaScript, core defaults when omitted |
| Errors | name TOML and a byte offset; `validate_for_write` rejects before a destination opens |
| `IOBase` | `from_io` / `into_io` infer TOML and outer [coding](../../coding/index.md) from the media type |

## Surfaces

| Page | Owns |
| --- | --- |
| [Scalars](scalar.md) | `loads` / `dumps`, the one string-key record, formatting, `read_scalar` / `write_scalar` over a handle |
| [Arrow](arrow.md) | the document as Arrow rows: one array of tables under the root's name |

The format-agnostic facade, limits, and `Format` vocabulary are on [Structured documents](../structured.md).

## Use

Rust returns `Scalar`; bindings redirect native mappings through the same codec.

=== "Rust"

    ```rust
    use yggdryl::text::toml;
    use yggdryl::{from_toml_scalar, into_toml_scalar, Scalar};

    let source = "title = \"yggdryl\"\ncount = 3\n\n[owner]\nname = \"Ada\"\n";
    let value = toml::from_utf8(source)?;
    let encoded = toml::into_utf8(&value)?;

    assert_eq!(
        value.get_key_str("title").and_then(Scalar::as_str),
        Some("yggdryl")
    );
    assert_eq!(toml::from_utf8(&encoded)?, value);

    // The crate-root inferring entry points answer the same Record, from text or bytes.
    assert_eq!(from_toml_scalar(source)?, value);
    assert_eq!(from_toml_scalar(into_toml_scalar(&value)?.as_bytes())?, value);
    ```

=== "Python"

    ```python
    from yggdryl import Scalar
    from yggdryl.text import toml

    source = 'title = "yggdryl"\ncount = 3\n\n[owner]\nname = "Ada"\n'
    natural = toml.loads(source)
    value = toml.loads(source, cls=Scalar)
    encoded = toml.dumps(value)

    assert value.kind == "record"
    assert value.as_py() == natural == {
        "count": 3,
        "owner": {"name": "Ada"},
        "title": "yggdryl",
    }
    assert toml.loads(encoded) == natural
    assert toml.loads(encoded, cls=Scalar) == value
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
    assert.ok(toml.loads(encoded, { scalar: true }).equals(value))
    ```

## Inferring entry point

`from_toml_scalar`, `from_toml_scalar_with_field`, and `into_toml_scalar` are TOML's [inferring entry points](../structured.md#raw-document-codecs), answering a `Record`; the [Use](#use) example shows them answering what the explicit form answers. The bindings' `loads` and `dumps` are that entry.

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

Every native temporal carries a `TimeUnit` and a non-null [`Timezone`](../../types/temporal.md#timezone).

| value | TOML behavior |
| --- | --- |
| offset date-time | `DateTime64` instant; a local date-time uses `Timezone::NAIVE` |
| date, local time | zone-free |
| outside the grammar, or named zone | ISO string or count, never a rewritten offset |
| bindings | closest lossless native temporal; `Scalar` keeps the rest |

## Placeholders

Opt-in, inside quoted strings, substituted after parsing and before Field interpretation, at any table depth; each quoted placeholder becomes the variable's own typed value. [Placeholders](../placeholders.md) shows a TOML table taking a string and an integer variable.

## Edges

- empty or comment-only document -> empty `Record`.
- null, non-record root, non-string key, `i64` overflow, excessive depth -> `validate_for_write` error, no partial destination.
- an existing file name as text -> parsed as TOML; fails as a bare word.
- Rust `_all` forms -> exactly one value; bindings expose no `loads_all`, `dump_all`, or streams.
- user key spelled like a private marker -> ordinary data.
- placeholder mapping -> wins over the environment, never read unless enabled.
- Arrow batches -> [Text records](../text/index.md), which refuse keyed merge.

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
