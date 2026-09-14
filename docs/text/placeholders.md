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

A quoted placeholder becomes the variable's own typed value, a default fills a name the mapping lacks, and a TOML table substitutes at any depth.

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
        "host: \"{{ HOST }}\"\nport: \"{{ PORT }}\"\ntimeout: \"{{ TIMEOUT | default(30) }}\"\n",
        Format::Yaml,
        &loading,
    )?;
    assert_eq!(value.get_key_str("host").and_then(Scalar::as_str), Some("db.internal"));
    assert_eq!(value.get_key_str("port"), Some(&Scalar::from(5432_i64)));
    assert_eq!(value.get_key_str("timeout"), Some(&Scalar::from(30_i64)));

    let table = yggdryl::text::from_utf8_with(
        "[database]\nhost = \"{{ HOST }}\"\nport = \"{{ PORT }}\"\n",
        Format::Toml,
        &loading,
    )?;
    let database = table.get_key_str("database").expect("the database table");
    assert_eq!(database.get_key_str("host").and_then(Scalar::as_str), Some("db.internal"));
    assert_eq!(database.get_key_str("port"), Some(&Scalar::from(5432_i64)));
    ```

=== "Python"

    ```python
    from yggdryl.text import toml, yaml

    placeholders = {"HOST": "db.internal", "PORT": 5432}

    document = 'host: "{{ HOST }}"\nport: "{{ PORT }}"\ntimeout: "{{ TIMEOUT | default(30) }}"\n'
    value = yaml.loads(document, placeholders=placeholders)
    assert value == {"host": "db.internal", "port": 5432, "timeout": 30}

    # Unquoted braces are YAML flow-mapping syntax, parsed before substitution runs.
    assert isinstance(yaml.loads("port: {{ PORT }}\n", placeholders=placeholders)["port"], dict)

    document = '[database]\nhost = "{{ HOST }}"\nport = "{{ PORT }}"\n'
    table = toml.loads(document, placeholders=placeholders)
    assert table["database"] == {"host": "db.internal", "port": 5432}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { toml, yaml } = require('yggdryl')

    const placeholders = { HOST: 'db.internal', PORT: 5432 }

    const document = 'host: "{{ HOST }}"\nport: "{{ PORT }}"\ntimeout: "{{ TIMEOUT | default(30) }}"\n'
    const value = yaml.loads(document, { placeholders })
    assert.deepEqual(value, { host: 'db.internal', port: 5432, timeout: 30 })

    const table = toml.loads('[database]\nhost = "{{ HOST }}"\nport = "{{ PORT }}"\n', { placeholders })
    assert.deepEqual(table.database, { host: 'db.internal', port: 5432 })
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
