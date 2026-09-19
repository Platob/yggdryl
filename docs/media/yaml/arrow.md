# YAML rows

A YAML document stream as Arrow rows. `read_arrow` and `write_arrow` are the one bridge between documents and a batch.

## Contract

| Key | Value |
| --- | --- |
| Reads | `read_arrow(options)` reads every document in the stream, then answers one batch |
| Writes | `write_arrow(value, mode)` writes one document per row, `---` separated |
| Held | nothing but the batch being encoded: the frame is per document, so rows stream out |
| Mode | overwrite only: a document set is written whole |
| Not a record encoding | [`RecordOptions`](../options.md) names no structured format, so `read_arrow_reader` and the three write intents refuse the name |
| Field | declares the root the rows land under; without one the rows name the root their contents prove |
| Bindings | Rust and Python; JavaScript binds neither call |

## Use

Every document in the stream is one row, and a write emits one document per row, so nothing but the current batch is ever held.

Rust and Python only.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::{IOBase, IOMedia, IOMode, Url};

    let mut handle =
        Buffer::new().with_media_type(Url::from_str("file:///quotes.yaml")?.media_type());
    handle.write_all_bytes(b"id: 1\nsymbol: AAPL\n---\nid: 2\nsymbol: MSFT\n")?;

    // The document's own shape holds the rows; the root is the one they prove.
    let value = handle.read_arrow(None)?;
    assert_eq!((value.row_size(), value.column_size()), (Some(2), 2));

    // Written back, the rows are framed the way this format frames them.
    handle.write_arrow(value, IOMode::Overwrite, None)?;
    assert!(String::from_utf8(handle.read_all_bytes()?)?.contains("---"));
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    source = pathlib.Path(tempfile.mkdtemp()) / "quotes.yaml"
    source.write_bytes(b"id: 1\nsymbol: AAPL\n---\nid: 2\nsymbol: MSFT\n")
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
    assert b"---" in handle.read_bytes()
    ```

## Edges

- `append` or `merge` -> refused naming the mode; the document set is written whole.
- `read_arrow_reader`, `overwrite_arrow_reader` on a `.yaml` name -> refused; YAML is not a record encoding.
- one document that is a sequence -> its items are the rows; any other single document is one row.
- a [placeholder](../placeholders.md) -> substituted before the rows are typed, so the resolved string becomes the exact value.
- JavaScript -> neither call is bound; use [`yaml.loads`](scalar.md) and build the batch with Arrow JS.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test media -- structured::
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/arrow/test_arrow_scalar.py
    ```
