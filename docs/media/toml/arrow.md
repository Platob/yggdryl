# TOML rows

A TOML document as Arrow rows. `read_arrow` and `write_arrow` are the one bridge between the document and a batch.

## Contract

| Key | Value |
| --- | --- |
| Reads | `read_arrow(options)`; TOML has no top-level sequence, so the rows are the array of tables stored under the root's name |
| Writes | `write_arrow(value, mode)` writes one array of tables under the root's name, each row an inline table |
| Held | the rows the one frame encloses, on both sides |
| Mode | overwrite only: a document is one frame around its rows, so it is written whole |
| Not a record encoding | [`RecordOptions`](../options.md) names no structured format, so `read_arrow_reader` and the three write intents refuse the name |
| Field | declares the root, which is also the table name the rows are stored under |
| Bindings | Rust and Python; JavaScript binds neither call |

## Use

TOML has no top-level sequence, so the rows are the array of tables stored under the root's name; a write puts them back there, each row an inline table.

Rust and Python only.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::{IOBase, IOMedia, IOMode, Url};

    let mut handle =
        Buffer::new().with_media_type(Url::from_str("file:///quotes.toml")?.media_type());
    handle.write_all_bytes(
        b"[[row]]\nid = 1\nsymbol = \"AAPL\"\n\n[[row]]\nid = 2\nsymbol = \"MSFT\"\n",
    )?;

    // The document's own shape holds the rows; the root is the one they prove.
    let value = handle.read_arrow(None)?;
    assert_eq!((value.row_size(), value.column_size()), (Some(2), 2));

    // Written back, the rows are framed the way this format frames them.
    handle.write_arrow(value, IOMode::Overwrite, None)?;
    assert!(String::from_utf8(handle.read_all_bytes()?)?.starts_with("\"row\" = ["));
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    source = pathlib.Path(tempfile.mkdtemp()) / "quotes.toml"
    source.write_bytes(b'[[row]]\nid = 1\nsymbol = "AAPL"\n\n[[row]]\nid = 2\nsymbol = "MSFT"\n')
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
    assert handle.read_bytes().startswith(b'"row" = [')
    ```

## Edges

- `append` or `merge` -> refused naming the mode; a document is written whole.
- `read_arrow_reader`, `overwrite_arrow_reader` on a `.toml` name -> refused; TOML is not a record encoding.
- a root naming no array of tables -> the document is one row.
- a [placeholder](../placeholders.md) -> substituted before the rows are typed, so the resolved string becomes the exact value.
- JavaScript -> neither call is bound; use [`toml.loads`](scalar.md) and build the batch with Arrow JS.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib media::structured::tests
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/arrow/test_arrow_scalar.py
    ```
