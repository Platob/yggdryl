# YAML

YAML documents as one [`Scalar`](../types/scalar.md) each, and a `---` stream as one per document; the bindings answer native objects unless asked for the scalar.

## Overview

| | |
| --- | --- |
| Declared by | `application/yaml`, `.yaml` |
| Build | default |
| Rust | `yggdryl::yaml`: `from_utf8`, `from_bytes`, `from_reader` and their `_with_field`, `_with_limits` and `_all` forms; `into_utf8`, `into_bytes`, `into_writer` and their `_with_formatting` and `_all` forms; `from_yaml_scalar`, `from_yaml_scalar_with_field` and `into_yaml_scalar` at the crate root; placeholders through `yggdryl::text::from_utf8_with` and a `Loading` |
| Python | `yggdryl.yaml`: `loads`, `dumps`, `dump`; `loads_all`, `load_all`, `dumps_all`, `dump_all` for `---` streams |
| JavaScript | `yaml`: `loads`, `load`, `dumps`, `dump`, and `loadStream`/`dumpStream` over a Node stream; `loadsAll`, `loadAll`, `dumpAll`, `loadAllStream`, `dumpAllStream` for `---` streams |
| Handle | a `.yaml` handle reads through `read_scalar` or `read_arrow` and writes whole through `write_scalar` or `write_arrow` ([Structured values](../holder/index.md#structured-values)) |

## Read

A document parses into the value it proves: a core tag such as `!!str` or `!!binary` types the value it marks, and any other tag is an annotation read past. A declared `field` types it exactly as it types [JSON](json.md#read). Depth, input bytes, nodes and documents are bounded - an alias expansion counts against the nodes - and a parse error names the byte it stopped at. `{{ }}` placeholders resolve after parsing and only when asked: a value that is exactly one placeholder takes the variable's type, an embedded one stays text, `| default(...)` supplies a fallback, and a name nothing resolves is refused rather than read as empty. The process environment is a second switch, off by default.

=== "Rust"

    ```rust
    use yggdryl::text::{self, Format, Loading, Placeholders};
    use yggdryl::{from_yaml_scalar, yaml, Scalar};

    // Without a field, the document is the value it proves.
    let value = from_yaml_scalar("symbol: AAPL\nquantity: 2\n")?;
    assert_eq!(value.get_key_str("symbol").and_then(Scalar::as_str), Some("AAPL"));
    assert_eq!(yaml::from_utf8("symbol: AAPL\nquantity: 2\n")?, value);

    // A `---` stream reads as one value per document.
    assert_eq!(yaml::from_utf8_all("id: 1\n---\nid: 2\n")?.len(), 2);

    // Placeholders resolve after parsing, and only when asked.
    let document = "port: \"{{ PORT }}\"\npath: \"{{ ROOT }}/logs\"\nretries: \"{{ RETRIES | default(3) }}\"\n";
    let loading = Loading::new().with_placeholders(
        Placeholders::new()
            .with_variable("PORT", Scalar::from(8080))
            .with_variable("ROOT", Scalar::from("/var")),
    );
    let resolved = text::from_utf8_with(document, Format::Yaml, &loading)?;
    assert_eq!(resolved.get_key_str("port"), Some(&Scalar::from(8080)));
    assert_eq!(resolved.get_key_str("path").and_then(Scalar::as_str), Some("/var/logs"));
    assert_eq!(resolved.get_key_str("retries"), Some(&Scalar::from(3)));
    assert_eq!(yaml::from_utf8(document)?.get_key_str("port").and_then(Scalar::as_str), Some("{{ PORT }}"));
    ```

=== "Python"

    ```python
    from yggdryl import Scalar, yaml

    # Without a field, the document is the value it proves.
    natural = yaml.loads("symbol: AAPL\nquantity: 2\n")
    assert natural == {"quantity": 2, "symbol": "AAPL"}
    assert yaml.loads("symbol: AAPL\nquantity: 2\n", cls=Scalar).kind == "struct"

    # A `---` stream reads as one value per document.
    assert list(yaml.loads_all("id: 1\n---\nid: 2\n")) == [{"id": 1}, {"id": 2}]

    # Placeholders resolve after parsing, and only when asked.
    document = 'port: "{{ PORT }}"\npath: "{{ ROOT }}/logs"\nretries: "{{ RETRIES | default(3) }}"\n'
    resolved = yaml.loads(document, placeholders={"PORT": 8080, "ROOT": "/var"})
    assert resolved == {"path": "/var/logs", "port": 8080, "retries": 3}
    assert yaml.loads(document)["port"] == "{{ PORT }}"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, yaml } = require('yggdryl')

    // Without a field, the document is the value it proves.
    const natural = yaml.loads('symbol: AAPL\nquantity: 2\n')
    assert.deepEqual(natural, { quantity: 2, symbol: 'AAPL' })
    const value = yaml.loads('symbol: AAPL\nquantity: 2\n', { scalar: true })
    assert.ok(value instanceof Scalar)
    assert.equal(value.kind, 'struct')

    // A `---` stream reads as one value per document.
    assert.deepEqual(yaml.loadsAll('id: 1\n---\nid: 2\n'), [{ id: 1 }, { id: 2 }])

    // Placeholders resolve after parsing, and only when asked.
    const document = 'port: "{{ PORT }}"\npath: "{{ ROOT }}/logs"\nretries: "{{ RETRIES | default(3) }}"\n'
    const resolved = yaml.loads(document, { placeholders: { PORT: 8080, ROOT: '/var' } })
    assert.deepEqual(resolved, { path: '/var/logs', port: 8080, retries: 3 })
    assert.equal(yaml.loads(document).port, '{{ PORT }}')
    ```

## Write

A document writes in block style unless no layout is asked for, which writes flow style on one line. A record writes its keys sorted - a parsed document's records, a Rust `Scalar::Struct` and a JavaScript object alike - while a Python `dict` crosses as a map and keeps its own order. Bytes are written `!!binary` and read back as bytes, and a stream writes `---` between its documents.

=== "Rust"

    ```rust
    use yggdryl::text::Formatting;
    use yggdryl::{into_yaml_scalar, yaml, Scalar};

    let value = Scalar::from_struct([("symbol", Scalar::from("AAPL")), ("quantity", Scalar::from(2_i64))])?;

    // Block style, the record's keys sorted.
    assert_eq!(into_yaml_scalar(&value)?, "quantity: 2\nsymbol: AAPL\n");
    assert_eq!(yaml::into_utf8(&value)?, "quantity: 2\nsymbol: AAPL\n");
    // Flow style on one line, when no layout is asked for.
    assert_eq!(yaml::into_utf8_with_formatting(&value, Formatting::compact())?, "{quantity: 2, symbol: AAPL}\n");

    // A stream writes `---` between its documents.
    let documents = [
        Scalar::from_struct([("id", Scalar::from(1))])?,
        Scalar::from_struct([("id", Scalar::from(2))])?,
    ];
    assert_eq!(yaml::into_utf8_all(&documents)?, "id: 1\n---\nid: 2\n");
    ```

=== "Python"

    ```python
    from yggdryl import Scalar, yaml

    # Block style, a record's keys sorted.
    record = yaml.loads("symbol: AAPL\nquantity: 2\n", cls=Scalar)
    assert yaml.dumps(record) == b"quantity: 2\nsymbol: AAPL\n"
    # Flow style on one line, when no layout is asked for.
    assert yaml.dumps(record, indent=None) == b"{quantity: 2, symbol: AAPL}\n"
    # A dict crosses as a map, which keeps its own order.
    assert yaml.dumps({"symbol": "AAPL", "quantity": 2}) == b"symbol: AAPL\nquantity: 2\n"

    # Bytes are tagged `!!binary`, and read back as bytes.
    assert yaml.dumps({"raw": b"ab"}) == b'raw: !!binary "YWI="\n'
    assert yaml.loads(yaml.dumps({"raw": b"ab"})) == {"raw": b"ab"}

    # A stream writes `---` between its documents.
    assert yaml.dumps_all([{"id": 1}, {"id": 2}]) == b"id: 1\n---\nid: 2\n"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { yaml } = require('yggdryl')

    const value = { symbol: 'AAPL', quantity: 2 }

    // Block style, the record's keys sorted.
    const encoded = yaml.dumps(value)
    assert.ok(Buffer.isBuffer(encoded))
    assert.equal(encoded.toString(), 'quantity: 2\nsymbol: AAPL\n')
    // Flow style on one line, when no layout is asked for.
    assert.equal(yaml.dumps(value, { indent: null }).toString(), '{quantity: 2, symbol: AAPL}\n')

    // A stream writes `---` between its documents.
    assert.equal(yaml.dumpAll([{ id: 1 }, { id: 2 }]).toString(), 'id: 1\n---\nid: 2\n')
    ```

## Performance

One Windows x86_64 release run, one fixture per runtime; compare routes within a runtime, never Python against Node.

| operation | runtime | YAML |
| --- | --- | ---: |
| field class encode | CPython | 202 us |
| field class decode | CPython | 387 us |
| bytes decode | CPython | 47.6 us |
| reader redirect | CPython | 51.5 us |
| writer redirect | CPython | 200 us |
| natural document decode | Node | 16.5 ms |
| natural document emit | Node | 24.3 ms |

```bash
python/.venv/bin/python python/benchmarks/text.py --iterations 10000
npm run --prefix node bench:text
```

### Placeholders

256-entry YAML documents, feature off and on; containerized x86_64 Linux, Criterion medians with 95% intervals.

```text
codec/placeholder/none/off  272.81 us   [271.30 us 274.52 us]
codec/placeholder/none/on   266.07 us   [265.12 us 267.21 us]
codec/placeholder/few/off   265.58 us   [264.58 us 266.86 us]
codec/placeholder/few/on    327.80 us   [325.00 us 330.56 us]
codec/placeholder/most/off  264.84 us   [262.10 us 268.46 us]
codec/placeholder/most/on   386.80 us   [384.48 us 389.17 us]
```

The guard is within run noise; substitution cost about 0.5 us per rebuilt scalar.

```bash
cargo bench -p yggdryl --bench text -- codec/placeholder
```
