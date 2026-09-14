# JSON rows

A JSON document as Arrow rows. `read_arrow` and `write_arrow` are the one bridge between a document and a batch.

## Contract

| Key | Value |
| --- | --- |
| Reads | `read_arrow(options)`; an array document holds the rows, any other document is one row; JSON Lines reads every line, then one batch |
| Writes | `write_arrow(value, mode)`; JSON frames every row in one array, JSON Lines writes one document per line |
| Held | `.json` holds the rows its one frame encloses; `.jsonl` streams, holding no more than the batch being encoded |
| Mode | overwrite only: a document is one frame around its rows, so it is written whole |
| Not a record encoding | [`RecordOptions`](../options.md) names no structured format, so `read_arrow_reader` and the three write intents refuse the name |
| Field | declares the root the rows land under; without one the rows name the root their contents prove |
| Bindings | Rust and Python; JavaScript binds neither call |

## Use

An array document holds the rows, and a write frames them in one array again. Under a `.jsonl` name the same rows are one document per line, streamed rather than held.

Rust and Python only.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::{IOBase, IOMedia, IOMode, Url};

    let mut handle =
        Buffer::new().with_media_type(Url::from_str("file:///quotes.json")?.media_type());
    handle.write_all_bytes(br#"[{"id":1,"symbol":"AAPL"},{"id":2,"symbol":"MSFT"}]"#)?;

    // The document's own shape holds the rows; the root is the one they prove.
    let value = handle.read_arrow(None)?;
    assert_eq!((value.row_size(), value.column_size()), (Some(2), 2));

    // Written back, the rows are framed the way this format frames them.
    handle.write_arrow(value, IOMode::Overwrite, None)?;
    assert_eq!(&handle.read_all_bytes()?[..2], b"[{");
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    source = pathlib.Path(tempfile.mkdtemp()) / "quotes.json"
    source.write_bytes(b'[{"id":1,"symbol":"AAPL"},{"id":2,"symbol":"MSFT"}]')
    handle = IOBase(source)

    # The document's own shape holds the rows; the root is the one they prove.
    value = handle.read_arrow()
    assert value.shape == "batch"
    assert value.as_py() == [
        {"id": 1, "symbol": "AAPL"},
        {"id": 2, "symbol": "MSFT"},
    ]

    # Written back, the rows are framed the way this format frames them.
    handle.write_arrow(value.into_arrow_table())
    assert handle.read_bytes().startswith(b"[{")
    ```

## Edges

- `append` or `merge` -> refused naming the mode; a document is written whole.
- `read_arrow_reader`, `overwrite_arrow_reader` on a `.json` name -> refused; JSON is not a record encoding.
- a document that is not an array -> one row.
- a row shape no `Field` proves -> inferred from what the document proves, which is the rule every schemaless read follows.
- `quotes.json.gz` -> the coding comes off before the parser, as on every [structured read](scalar.md).
- JavaScript -> neither call is bound; use [`json.loads`](scalar.md) and build the batch with Arrow JS.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib media::structured::tests
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/arrow/test_arrow_scalar.py
    ```
