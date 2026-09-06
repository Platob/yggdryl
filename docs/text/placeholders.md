<a id="jinja-style-placeholders"></a>

# Placeholders

Jinja-style `{{ }}` substitution in string values of a parsed YAML or TOML document.

## Contract

| item | rule |
| --- | --- |
| `{{ NAME }}` | resolve `NAME`; absence is an error |
| `{{ NAME \| default(LITERAL) }}` | use a JSON-scalar fallback |
| `{{{{` | emit a literal `{{` |
| Formats | [YAML](yaml.md), [TOML](toml.md); [JSON](json.md) refuses |
| Default | off; needs a mapping or the environment switch |
| Precedence | mapping over process environment |
| Environment | read only when `environment=True` |
| Order | parse, substitute, then [Field](../types/field.md) |
| Guard | bytes scanned once for `{{`; no match, no value walk |

## Use

=== "Rust"

    ```rust
    use yggdryl::text::{Format, Loading, Placeholders};
    use yggdryl::Scalar;

    let loading = Loading::new().with_placeholders(
        Placeholders::new().with_variable("HOST", Scalar::from("db.internal")),
    );
    let value = yggdryl::text::from_utf8_with(
        "host: \"{{ HOST }}\"\nport: \"{{ PORT | default(8080) }}\"\n",
        Format::Yaml,
        &loading,
    )?;

    assert_eq!(value.get_key_str("host").and_then(Scalar::as_utf8), Some("db.internal"));
    assert_eq!(value.get_key_str("port"), Some(&Scalar::from(8080_i64)));
    ```

=== "Python"

    ```python
    from yggdryl.text import yaml

    document = 'host: "{{ HOST }}"\nport: "{{ PORT | default(8080) }}"\n'
    value = yaml.loads(document, placeholders={"HOST": "db.internal"})

    assert value == {"host": "db.internal", "port": 8080}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { yaml } = require('yggdryl')

    const document = 'host: "{{ HOST }}"\nport: "{{ PORT | default(8080) }}"\n'
    const value = yaml.loads(document, {
      placeholders: { HOST: 'db.internal' },
    })

    assert.deepEqual(value, { host: 'db.internal', port: 8080 })
    ```

## Edges

- Braces the grammar would read structurally -> quote the placeholder.
- Parse runs first -> placeholders never create keys, containers, or syntax.
- Substituted string -> consumed by a decimal, binary, or temporal Field.
- Resolved secret -> an ordinary value; dumps write it and never reintroduce placeholders.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test text placeholder::
    cargo test --features "parquet iceberg" -p yggdryl --lib text::loading::
    cargo bench -p yggdryl --bench text -- codec/placeholder
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/text/test_placeholders.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/text/placeholder.test.js
    ```

## Performance

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
