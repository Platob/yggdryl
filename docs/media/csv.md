# Delimited-text records

`CsvOptions` reads a `text/csv` resource as rows and columns, and `Csv<H>` adds
the positional row and cell access a delimited resource can answer.

## Contract

| option | contract |
| --- | --- |
| `separator` | the byte between two cells; default `,` |
| `quote` | the byte that opens and closes a quoted cell; default `"`, `None` reads every cell literally |
| `escape` | the byte that escapes the next byte inside a quoted cell; unset doubles the quote, as RFC 4180 spells |
| `comment` | a line opening with this byte carries no record; unset reads every line |
| `linesep` | exact record terminator; unset accepts LF, CRLF, or CR and writes LF |
| `header` | the first record names the columns; default `true`, `false` names them `column_1`, `column_2`, … |
| `null` | the cell text that spells absence in an *unquoted* cell; default the empty cell |
| `trim` | unquoted cells drop their edge ASCII whitespace; default `false` |
| `autotype` | infer column datatypes from the cells; default `true`, `false` reads every column as `utf8` |
| `infer_row_size` | rows inference reads; default `DEFAULT_INFER_ROW_SIZE` (1024), `None` reads every row |
| `timezone` | zone applied to inferred offset-free timestamps |

One byte cannot carry two roles: a separator, quote, escape, or comment marker
that collides with another, or with a terminator byte, is refused when it is set.

!!! note "Rust-only"

    Python and JavaScript reach a CSV resource through the shared record
    surface; `CsvOptions` and the positional methods are Rust-only.

## Use

The header names the columns and the cells type them.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::media::csv::CsvOptions;
    use yggdryl::media::IORecordOptions as _;
    use yggdryl::{DataType, IOBase, IOMedia, Url};

    let source = Buffer::from_bytes(
        b"symbol,quantity,traded_on\nBRN,120,2024-01-02\nWTI,80,2024-01-03\n".to_vec(),
    )
    .with_media_type(Url::from_str("file:///trades.csv")?.media_type());

    let options = CsvOptions::new();
    let field = yggdryl::media::csv::read_field(&source, &options)?;
    assert_eq!(field.get_field(0).unwrap().dtype(), &DataType::Utf8);
    assert_eq!(field.get_field(1).unwrap().dtype(), &DataType::Int64);
    assert_eq!(field.get_field(2).unwrap().dtype(), &DataType::Date32);

    let mut rows = 0;
    for batch in source.read_arrow_reader(&options.into())? {
        rows += batch?.num_rows();
    }
    assert_eq!(rows, 2);
    ```

## Columns

A header cell names its column; an empty header cell falls back to its
positional name. Inference folds each sampled cell into the column with
[`DataType::merge_with`](../types/field.md), so text is the floor rather than a
refusal.

| cells | column |
| --- | --- |
| `true`, `FALSE` | `boolean` |
| whole numbers fitting signed 64 bits | `int64` |
| finite decimals or exponents | `float64` |
| ISO dates | `date32` |
| ISO clock readings | `time64(us)` |
| ISO datetimes without an offset | `datetime64(us)` in `timezone`, naive unless one is set |
| ISO timestamps with an offset | `datetime64(us, UTC)` unless `timezone` names another |
| anything else, or two readings that do not meet | `utf8` |

Every inferred column is nullable: the sample is a prefix, and an unsampled row
may still spell absence. A declared `dtype` turns inference off. The header still
says which cell is which, so a declaration whose columns are in another order
still reads the right cells: the emitted column keeps the header's name and takes
the declared datatype by that name, and the shared cast reorders and completes
from there. A declared column the cells cannot spell is read as text and
converted by that same cast, so a `decimal128(12, 2)` column reads without a
second text-to-value path.

## Absence

Absence is the `null` spelling on an unquoted cell, and nothing else. A quoted
cell is content whatever it spells, which is what lets one resource hold both an
absent value and an empty string under the default spelling.

| cell | value |
| --- | --- |
| bare, equal to `null` | absent |
| quoted, equal to `null` | that text |
| bare, empty, `null` unset from the default | the empty string |

A write is the same rule read backwards: an absent value is written as the `null`
spelling, and a present value that would spell it is quoted.

## Positional access

A delimited row is addressable in the bytes that store it. `Csv<H>` holds a
sparse row index - one anchor every `stride` rows, the stride doubling so the
index stays flat at 32 KiB whatever the resource's size - so reaching a row is
one positional read plus a bounded forward scan.

| method | answers |
| --- | --- |
| `read_row_scalar` | one row's ordered column values |
| `read_cell_bytes` | one cell's exact bytes, unescaped |
| `read_cell_text` | one cell as text |
| `read_cell_scalar` | one cell as the value its column declares |
| `read_row_byte_range` | where a row sits in the resource, terminator included |
| `write_cell_bytes`, `write_cell_text`, `write_cell_scalar` | replace one cell in place |
| `write_row_scalar` | replace one row, keeping the terminator it had |
| `append_row_scalar` | add one row after the final record |
| `remove_row` | remove one row, closing the gap |

A replacement of the same width is one positional write. Any other width splices:
the tail moves in bounded windows, so nothing is held in memory.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::media::csv::Csv;
    use yggdryl::{IOBase, Scalar, Url};

    let mut media = Csv::new(
        Buffer::from_bytes(b"symbol,quantity\nBRN,120\nWTI,80\n".to_vec())
            .with_media_type(Url::from_str("file:///trades.csv")?.media_type()),
    );

    assert_eq!(media.read_cell_text(1, 0)?.as_deref(), Some("WTI"));
    assert_eq!(media.read_cell_scalar(1, 1)?, Some(Scalar::from(80_i64)));
    assert_eq!(media.read_row_byte_range(0)?, Some(16..24));

    // Wider than the cell it replaces: the tail moves and nothing is re-encoded.
    media.write_cell_scalar(1, 1, &Scalar::from(1_000_000_i64))?;
    assert_eq!(
        media.handle().read_all_bytes()?,
        b"symbol,quantity\nBRN,120\nWTI,1000000\n"
    );

    media.remove_row(0)?;
    assert_eq!(media.handle().read_all_bytes()?, b"symbol,quantity\nWTI,1000000\n");
    ```

## Writes

Rows render through Arrow's own array formatter, the same one a partition
directory name goes through, so a value spells itself one way across the crate.
A cell is quoted only when its bytes need it.

| write | behavior |
| --- | --- |
| overwrite | header, when `header` is set, then every row; the value is staged whole and published once, so a batch that fails to render leaves the resource as it was |
| append | rows after the current final record, with no second header; a missing terminator is added first. A resource holding no records has no header either, so it is filled rather than appended to |
| append column order | the stored header decides it, so a batch naming the same columns in another order still lands in the columns it named |
| merge | supported: a delimited row has identity, unlike a text line |
| coded handle | the whole value is re-encoded, because a coding has no addressable tail |

## Edges

- A quoted cell holding the record terminator -> physical lines are joined with the exact bytes the resource held, not the terminator a write would produce.
- A record whose width is not the columns' width -> refused, naming the row, the expected count, and the actual one.
- A blank line or a comment line -> carries no record and advances no row number.
- A numeric with leading zeros (`007`) -> `utf8`: the padding is part of an identifier.
- A whole number too wide for signed 64 bits -> `utf8`, rather than losing digits to a double.
- A cell equal to `null` but quoted -> that text, not absence; a present value that would spell absence is quoted on write, so the round trip keeps both.
- A cell that is not UTF-8 -> refused, naming the byte offset; a header name that is not UTF-8 is refused the same way.
- A quoted cell whose quote never closes before the resource ends -> read as content, so a truncated file still yields every complete row before it.
- `infer_row_size` -> a bound, not a hint: a row past it never widens a column.
- A positional write on a coded resource (`trades.csv.gz`) -> refused by name; a coding's output offset is not addressable in its input. Positional *reads* still work, by decoding the prefix.
- `Media::open` on a `text/csv` name -> `Media::Csv`; `RecordOptions::for_media_type` answers `RecordOptions::Csv`.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib media::csv::tests
    cargo bench -p yggdryl --bench media -- csv_read
    cargo bench -p yggdryl --bench media -- csv_positional
    cargo bench -p yggdryl --bench media -- csv_write
    cargo bench -p yggdryl --bench media -- csv_inference
    ```
