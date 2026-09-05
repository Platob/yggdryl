# Extensions

Two native packages hand Python and JavaScript the same values the Rust core holds.

## Contract

| key | value |
| --- | --- |
| Owns | the [Python](python.md) and [JavaScript](javascript.md) packages over one core |
| Python | PyO3; every argument accepts its obvious Python spelling |
| JavaScript | Node-API; conventional JavaScript casing and protocols |
| Behaviour | documented once, on the [core pages](../index.md) |
| Binding page | the boundary only: what the package adds, and how it converts |

## Use

=== "Python"

    ```python
    from yggdryl import DataType, Field, Url

    # Every argument accepts the obvious Python spelling of itself.
    schema = Field("row", DataType.from_fields([Field("id", "int64", nullable=False)]), nullable=False)
    location = Url.from_path("C:/market data/trades.arrows")

    assert schema.dtype.id == "struct"
    assert str(location) == "file:///C:/market%20data/trades.arrows"
    assert str(location.media_type.base) == "application/vnd.apache.arrow.stream"
    ```

=== "JavaScript"

    ```javascript
    const { DataType, Field, Url } = require('yggdryl')
    const assert = require('node:assert/strict')

    const schema = new Field(
      'row',
      DataType.fromFields([new Field('id', 'int64', false)]),
      false,
    )

    assert.equal(schema.dtype.kind, 'nested')
    assert.equal(String(Url.fromPath('C:/market data/trades.arrows')),
      'file:///C:/market%20data/trades.arrows')
    ```

## Pages

| page | owns |
| --- | --- |
| [Python](python.md) | The PyO3 boundary per layer family, with build, test, and benchmark commands |
| [JavaScript](javascript.md) | The Node-API boundary per layer family, with build, test, and benchmark commands |

## Edges

- `npm test --prefix node` -> `node --test "tests/**/*.test.js"`, then `tsc --noEmit`.
- `scripts/check_docs_examples.py --lang` -> `rust`, `python`, `javascript`, or `all`; default `all`.
- The checker's interpreter -> `python/.venv/Scripts/python.exe`, else `python/.venv/bin/python`.

## Commands

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests
    python scripts/check_docs_examples.py --lang python
    ```

=== "JavaScript"

    ```bash
    npm test --prefix node
    python scripts/check_docs_examples.py --lang javascript
    ```
