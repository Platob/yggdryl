# Avro schemas

One parsed schema value: the retained JSON document, the Parsing Canonical Form, the fingerprint that names it, and the logical types it can annotate.

## Contract

| Item | Behaviour |
| --- | --- |
| Owns | `avro::Schema`, `avro::MAX_SCHEMA_DEPTH`; Python `avro.Schema`, JavaScript `avro.Schema` |
| Parses | namespaces, aliases, defaults, and recursive references, all resolved at parse time; a named-type reference stays a reference, which keeps a recursive schema finite |
| Retains | the complete JSON document, so an attribute this implementation does not model survives a parse and a [write](write.md) |
| Canonical form | `into_canonical_form` strips whitespace, attribute order, docs, unknown attributes, logical annotations, aliases, and defaults |
| Fingerprint | CRC-64-AVRO over the canonical form, little-endian on the wire; it names a writer schema in a store and keys a cached [`Resolution`](read.md#reading-with-a-different-schema) |
| Identity | equality, total ordering, and `stable_hash` use the complete retained document, not the fingerprint |
| Logical types | `date`, `time-millis`/`micros`, `timestamp-millis`/`micros`/`nanos`, `local-timestamp-*`, `uuid` over string and fixed(16), `decimal` over bytes and fixed, and `duration` decode as typed values |
| Limits | depth bounds schema nesting; a caller's `max_depth` only tightens `MAX_SCHEMA_DEPTH`, which is 384, never widens it ([Blocks](blocks.md#codecs-and-limits)) |
| Cloning | cheap: a clone shares the node tree, the name registry, and the JSON the schema round-trips as |

## Use

A `Schema` resolves namespaces, aliases, defaults, and recursive references at parse time; a named-type reference stays a reference, which keeps a recursive schema finite.

=== "Rust"

    ```rust
    use yggdryl::avro::Schema;

    let schema = Schema::from_str(
        r#"{"type": "record", "name": "trade", "doc": "one fill", "fields": [
            {"name": "symbol", "type": "string"},
            {"name": "qty", "type": "long", "field-id": 2}
        ]}"#,
    )?;

    assert!(!schema.clone().into_canonical_form().contains("doc"));
    assert_eq!(schema.fingerprint().to_le_bytes()[0], 0xF5);
    let text = String::from_utf8(yggdryl::json::into_bytes(&schema.into_json())?)?;
    assert!(text.contains("field-id"));
    ```

=== "Python"

    ```python
    from yggdryl.media import avro

    document = {
        "type": "record",
        "name": "trade",
        "doc": "one fill",
        "fields": [
            {"name": "symbol", "type": "string"},
            {"name": "qty", "type": "long", "field-id": 2},
        ],
    }
    schema = avro.Schema(document)

    assert "doc" not in schema.into_canonical_form()
    assert schema.fingerprint().to_bytes(8, "little")[0] == 0xF5
    assert schema.into_json()["fields"][1]["field-id"] == 2
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { avro } = require('yggdryl')

    const schema = new avro.Schema({
      type: 'record',
      name: 'trade',
      doc: 'one fill',
      fields: [
        { name: 'symbol', type: 'string' },
        { name: 'qty', type: 'long', 'field-id': 2 },
      ],
    })

    assert.ok(!schema.canonicalForm.includes('doc'))
    assert.equal(Number(schema.fingerprint & 0xffn), 0xf5)
    assert.equal(schema.intoJSON().fields[1]['field-id'], 2)
    ```

`fingerprint` hashes the Parsing Canonical Form with CRC-64-AVRO, which strips whitespace, attribute order, docs, unknown attributes, logical annotations, aliases, and defaults. Equality, total ordering, and `stable_hash` use the complete retained JSON document instead.

## Logical types

`date`, `time-millis`/`micros`, `timestamp-millis`/`micros`/`nanos`, `local-timestamp-*`, `uuid` over string and fixed(16), `decimal` over bytes and fixed, and `duration` decode as typed values.

=== "Rust"

    ```rust
    use yggdryl::TimeUnit;
    use yggdryl::holder::Buffer;
    use yggdryl::{Timezone, Scalar};
    use yggdryl::json;
    use yggdryl::avro;

    let schema = json::from_utf8(
        r#"{"type": "record", "name": "row", "fields": [
            {"name": "day", "type": {"type": "int", "logicalType": "date"}},
            {"name": "at", "type": {"type": "long", "logicalType": "timestamp-micros"}},
            {"name": "price", "type": {"type": "bytes", "logicalType": "decimal",
                                        "precision": 10, "scale": 2}}
        ]}"#,
    )?;
    let row = Scalar::from_struct([
        (
            "day",
            Scalar::date32_in(19_782, TimeUnit::Day, Timezone::NAIVE)?,
        ),
        ("at", Scalar::datetime64(
            1_700_000_000_000_000,
            TimeUnit::Microsecond,
            Timezone::UTC,
        )?),
        ("price", Scalar::d128(18_750, 2)),
    ])?;

    let mut handle = Buffer::new();
    avro::write_container(&mut handle, &schema, &[], &[row.clone()])?;
    assert_eq!(avro::read_container(&handle)?.rows[0], row);
    ```

=== "Python"

    ```python
    from datetime import date, datetime, timezone
    from decimal import Decimal

    from yggdryl.media import avro

    schema = {
        "type": "record",
        "name": "row",
        "fields": [
            {"name": "day", "type": {"type": "int", "logicalType": "date"}},
            {"name": "at", "type": {"type": "long", "logicalType": "timestamp-micros"}},
            {"name": "price", "type": {"type": "bytes", "logicalType": "decimal",
                                        "precision": 10, "scale": 2}},
        ],
    }
    row = {
        "day": date(2024, 2, 29),
        "at": datetime(2023, 11, 14, 22, 13, 20, tzinfo=timezone.utc),
        "price": Decimal("187.50"),
    }

    decoded = avro.loads(avro.dumps([row], schema)).rows[0]
    assert decoded == row
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, avro } = require('yggdryl')

    const decimal = {
      type: 'bytes',
      logicalType: 'decimal',
      precision: 10,
      scale: 2,
    }
    const value = Scalar.decimal(18750n, 2)
    const decoded = avro.loadsSingle(avro.dumpsSingle(value, decimal), decimal)

    assert.ok(decoded instanceof Scalar)
    assert.equal(decoded.kind, 'd64')
    assert.equal(decoded.unscaled, 18750n)
    assert.equal(decoded.scale, 2)
    ```

A date is `Date32`, a timestamp is `DateTime64` with `UTC`, and a decimal keeps its exact coefficient and scale.

## Edges

- an unknown logical annotation, or attributes invalid for its underlying type -> degrades to the underlying type, never an error.
- a decimal wider than 38 digits -> keeps its raw bytes; `duration` keeps its twelve bytes, being a month/day/millisecond triple.
- same fingerprint, different retained JSON -> distinct schema values; the bindings' `equals`, comparison, and hash follow the JSON identity.
- a named-type reference -> stays a reference, so a recursive schema parses and stays finite; the [record surface](write.md#contract) still refuses to spell one as Arrow.
- a schema nested deeper than the depth bound -> refused at parse time; `max_depth` tightens `MAX_SCHEMA_DEPTH` and cannot widen it.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib avro::tests
    cargo test --features "parquet iceberg" -p yggdryl --test media avro::
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- codec/avro_types
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/media/test_avro.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/media/avro.test.js
    ```
