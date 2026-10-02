# JSON

JSON documents as one [`Scalar`](../types/scalar.md) each, and JSON Lines as one per line; the bindings answer native objects unless asked for the scalar.

## Overview

| | |
| --- | --- |
| Declared by | `application/json`, `.json`; `application/x-ndjson`, `.jsonl` for JSON Lines |
| Build | default |
| Rust | `yggdryl::json`: `from_utf8`, `from_bytes`, `from_reader` and their `_with_field`, `_with_limits` and `_all` forms, `from_lines_*` for JSON Lines; `into_utf8`, `into_bytes`, `into_writer` and their `_with_formatting` and `_all` forms; `from_json_scalar`, `from_json_scalar_with_field` and `into_json_scalar` at the crate root |
| Python | `yggdryl.json`: `loads`, `dumps`, `dump`; `loads_all`, `load_all`, `dumps_all`, `dump_all` for JSON Lines |
| JavaScript | `json`: `loads`, `load`, `dumps`, `dump`, and `loadStream`/`dumpStream` over a Node stream; `loadsAll`, `loadAll`, `dumpAll`, `loadAllStream`, `dumpAllStream` for JSON Lines |
| Handle | a `.json` handle reads through `read_scalar` or `read_arrow` and writes whole through `write_scalar` or `write_arrow` - one value, so no `RecordOptions` ([Structured values](../holder/index.md#structured-values)) |
| Refused | `{{ }}` placeholders: JSON is a data interchange format, so substitution is [YAML](yaml.md#read), [TOML](toml.md#read) and [XML](xml.md#read)'s alone |

## Read

A document parses into the value it proves - an object a record, an array a sequence, a number an integer or a float, a string text - and nothing more: a date stays text until a field says otherwise. A declared `field` types natural text (decimals, dates, exact widths), orders a record into its columns and validates it, a refusal naming the path. Depth, input bytes, nodes and documents are bounded, and a parse error names the byte it stopped at.

=== "Rust"

    ```rust
    use yggdryl::json;
    use yggdryl::{from_json_scalar, from_json_scalar_with_field, DataType, Scalar, StructType};

    // Without a field, the document is the value it proves.
    let value = from_json_scalar(r#"{"symbol":"AAPL","quantity":100}"#)?;
    assert_eq!(value.get_key_str("symbol").and_then(Scalar::as_str), Some("AAPL"));
    assert_eq!(json::from_bytes(br#"{"symbol":"AAPL","quantity":100}"#)?, value);

    // A field types natural text, orders the record into its columns and validates it.
    let field = DataType::from(StructType::from_fields([
        DataType::decimal128(10, 2)?.required_field("px"),
        DataType::date32().required_field("day"),
        DataType::Int8.required_field("n"),
    ])?)
    .required_field("trade");
    let row = from_json_scalar_with_field(r#"{"n":7,"day":"2024-01-02","px":"12.50"}"#, &field)?;
    assert_eq!(row.get(0).as_deref(), Some(&Scalar::decimal128(1250, 2)));
    assert_eq!(row.get(1).expect("a day").dtype()?, DataType::date32());
    assert_eq!(row.get(2).as_deref(), Some(&Scalar::from(7_i8)));
    let refused = from_json_scalar_with_field(r#"{"n":700,"day":"2024-01-02","px":"1"}"#, &field).unwrap_err();
    assert!(refused.to_string().contains("$.trade.n"), "{refused}");

    // JSON Lines is one value per line.
    let rows = json::from_lines_utf8("{\"id\":1}\n{\"id\":2}\n")?;
    assert_eq!(rows.len(), 2);
    ```

=== "Python"

    ```python
    import datetime
    import decimal

    import pytest

    from yggdryl import Field, Scalar, json

    # Without a field, the document is the value it proves.
    natural = json.loads('{"symbol":"AAPL","quantity":100}')
    assert natural == {"quantity": 100, "symbol": "AAPL"}
    value = json.loads(b'{"symbol":"AAPL","quantity":100}', cls=Scalar)
    assert value.kind == "struct"
    assert value.as_py() == natural

    # A field types natural text, orders the record into its columns and validates it.
    field = Field("trade", "struct<px: decimal(10,2) not null, day: date32 not null, n: int8 not null>", nullable=False)
    row = json.loads('{"n": 7, "day": "2024-01-02", "px": "12.50"}', field=field)
    assert row == {"px": decimal.Decimal("12.50"), "day": datetime.date(2024, 1, 2), "n": 7}
    with pytest.raises(ValueError, match=r"\$\.trade\.n"):
        json.loads('{"n": 700, "day": "2024-01-02", "px": "1"}', field=field)

    # JSON Lines is one value per line.
    assert list(json.loads_all(b'{"id":1}\n{"id":2}\n')) == [{"id": 1}, {"id": 2}]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, Scalar, json } = require('yggdryl')

    // Without a field, the document is the value it proves.
    const natural = json.loads('{"symbol":"AAPL","quantity":100}')
    assert.deepEqual(natural, { quantity: 100, symbol: 'AAPL' })
    const value = json.loads(Buffer.from('{"symbol":"AAPL","quantity":100}'), { scalar: true })
    assert.ok(value instanceof Scalar)
    assert.equal(value.kind, 'struct')
    assert.deepEqual(value.asJs(), natural)

    // A field types natural text, orders the record into its columns and validates it.
    const field = new Field('trade', 'struct<px: decimal(10,2) not null, day: date32 not null, n: int8 not null>', false)
    const row = json.loads('{"n": 7, "day": "2024-01-02", "px": "12.50"}', { field })
    assert.equal(row.px.kind, 'd64')
    assert.equal(row.day.kind, 'date32')
    assert.equal(row.n, 7)
    assert.throws(() => json.loads('{"n": 700, "day": "2024-01-02", "px": "1"}', { field }), /\$\.trade\.n/)

    // JSON Lines is one value per line.
    assert.deepEqual(json.loadsAll('{"id":1}\n{"id":2}\n'), [{ id: 1 }, { id: 2 }])
    ```

## Write

A record writes its keys sorted - a parsed document's records, a Rust `Scalar::Struct` and a JavaScript object alike - so the same value always writes the same bytes; a Python `dict` crosses as a map and keeps its own order. Output is one line unless an indentation - a number of spaces, or a tab - is asked for. Bytes are written as base64 text. JSON Lines writes one value per line.

=== "Rust"

    ```rust
    use yggdryl::json;
    use yggdryl::text::Formatting;
    use yggdryl::{into_json_scalar, Scalar};

    let value = Scalar::from_struct([
        ("symbol", Scalar::from("AAPL")),
        ("legs", Scalar::from_sequence([Scalar::from(1), Scalar::from(2)])),
    ])?;

    // One line, the record's keys sorted: the same value writes the same bytes.
    let encoded = into_json_scalar(&value)?;
    assert_eq!(encoded, r#"{"legs":[1,2],"symbol":"AAPL"}"#);
    assert_eq!(json::into_utf8(&value)?, encoded);

    // Indented on request.
    assert_eq!(
        json::into_utf8_with_formatting(&value, Formatting::indented(2))?,
        "{\n  \"legs\": [\n    1,\n    2\n  ],\n  \"symbol\": \"AAPL\"\n}"
    );

    // JSON Lines writes one value per line.
    let rows = [
        Scalar::from_struct([("id", Scalar::from(1))])?,
        Scalar::from_struct([("id", Scalar::from(2))])?,
    ];
    assert_eq!(json::into_utf8_all(&rows)?, "{\"id\":1}\n{\"id\":2}\n");
    ```

=== "Python"

    ```python
    from yggdryl import Scalar, json

    # One line, a record's keys sorted: the same value writes the same bytes.
    record = json.loads('{"symbol":"AAPL","legs":[1,2]}', cls=Scalar)
    assert json.dumps(record) == b'{"legs":[1,2],"symbol":"AAPL"}'
    assert json.dump(record, utf8=True) == '{"legs":[1,2],"symbol":"AAPL"}'
    # A dict crosses as a map, which keeps its own order.
    assert json.dumps({"symbol": "AAPL", "legs": [1, 2]}) == b'{"symbol":"AAPL","legs":[1,2]}'

    # Indented on request: a number of spaces, or a tab.
    assert json.dumps(record, indent=2) == b'{\n  "legs": [\n    1,\n    2\n  ],\n  "symbol": "AAPL"\n}'
    assert json.dumps(record, indent="\t").startswith(b'{\n\t"legs"')

    # JSON Lines writes one value per line.
    assert json.dumps_all([{"id": 1}, {"id": 2}]) == b'{"id":1}\n{"id":2}\n'
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, json } = require('yggdryl')

    const value = { symbol: 'AAPL', legs: [1, 2] }

    // One line, the record's keys sorted: the same value writes the same bytes.
    const encoded = json.dumps(value)
    assert.ok(Buffer.isBuffer(encoded))
    assert.equal(encoded.toString(), '{"legs":[1,2],"symbol":"AAPL"}')
    assert.equal(json.dumps(Scalar.from(value)).toString(), encoded.toString())

    // Indented on request: a number of spaces, or a tab.
    assert.equal(json.dumps(value, { indent: 2 }).toString(), '{\n  "legs": [\n    1,\n    2\n  ],\n  "symbol": "AAPL"\n}')
    assert.ok(json.dumps(value, { indent: '\t' }).toString().startsWith('{\n\t"legs"'))

    // JSON Lines writes one value per line.
    assert.equal(json.dumpAll([{ id: 1 }, { id: 2 }]).toString(), '{"id":1}\n{"id":2}\n')
    ```

## Performance

### Rust codec

One release run of the `text` Criterion target's `codec/json` group on one Linux x86_64 container - Intel Xeon @ 2.80 GHz, 4 cores, 15 GiB; rustc 1.97.0, release profile (thin LTO, one codegen unit) - medians of 100 samples, on 2026-10-02. The record is `{symbol: "MSFT", quantity: 120, price: 413.75, tags: ["closing", "auction"]}`; the typed one a `decimal256(76, 4)`, a `datetime64(s, UTC)` and three bytes, read back under its field. The [YAML](yaml.md#rust-codec), [TOML](toml.md#rust-codec) and [XML](xml.md#rust-codec) pages carry the same rows from the same run.

| door | document | bytes | median | throughput |
| --- | --- | ---: | ---: | ---: |
| `into_bytes` | record | 76 | 414.3 ns | 174.9 MiB/s |
| `into_utf8` | record | 76 | 426.3 ns | 170.0 MiB/s |
| `into_json_scalar` | record | 76 | 435.1 ns | 166.6 MiB/s |
| `into_writer` | record | 76 | 251.0 ns | 288.7 MiB/s |
| `from_bytes` | record | 76 | 1.416 us | 51.2 MiB/s |
| `from_json_scalar` | record | 76 | 1.364 us | 53.1 MiB/s |
| `from_utf8` | record | 76 | 1.416 us | 51.2 MiB/s |
| `into_bytes` | typed | 66 | 1.056 us | 59.6 MiB/s |
| `from_bytes_with_field` | typed | 66 | 2.926 us | 21.5 MiB/s |
| `from_json_scalar_with_field` | typed | 66 | 2.889 us | 21.8 MiB/s |
| `from_bytes` | 64 levels deep | 129 | 10.29 us | 12.0 MiB/s |
| `from_lines_utf8` | 1,000 JSON lines | 3,890 | 61.93 us | 59.9 MiB/s |
| `LinesReader`, one value at a time | 1,000 JSON lines | 3,890 | 67.21 us | 55.2 MiB/s |

```bash
cargo bench -p yggdryl --bench text -- codec/json/
```

### Bindings

One Windows x86_64 release run, one fixture per runtime; compare routes within a runtime, never Python against Node.

| operation | runtime | JSON |
| --- | --- | ---: |
| field class encode | CPython | 150 us |
| field class decode | CPython | 340 us |
| bytes decode | CPython | 19.5 us |
| reader redirect | CPython | 26.8 us |
| writer redirect | CPython | 141 us |
| natural document decode | Node | 9.37 ms |
| natural document emit | Node | 18.0 ms |

```bash
python/.venv/bin/python python/benchmarks/text.py --iterations 10000
npm run --prefix node bench:text
```
