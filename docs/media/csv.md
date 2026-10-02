# CSV

Delimited text as RFC 4180 writes it: a header naming the columns, then one record per row, the cells separated by a comma - or by a tab under `.tsv`.

## Overview

| | |
| --- | --- |
| Declared by | `text/csv`, `.csv`; `text/tab-separated-values`, `.tsv` - the same medium under a tab |
| Build | default |
| Rust | `yggdryl::csv`: `Csv<H>` over any handle with `CsvOptions`; a `.csv`, `.tsv` or `.csv.gz` name composes it, and `RecordOptions` reads and sets its dialect as `csv_<setting>` / `set_csv_<setting>` - `header` / `set_header`, which Excel shares, apart |
| Python, JavaScript | any `IOBase` whose name declares CSV, through the [calls every medium answers](index.md#read); the dialect is a set of `RecordOptions` properties |
| Settings | the [dialect](#dialect): `separator`, `quote`, `escape`, `comment`, `header`, `null_values`, `trim`, `infer_row_size` |
| Coding, charset | the handle's: `trades.csv.gz` is gzip by name and `;charset=windows-1252` a declared charset |

CSV rides the ordinary record surface: the header names the columns, a declared `field` is the contract every cell is read under and a bounded sample of the records types them where none is declared. Compression and the charset are the handle's, and the dialect is a set of `RecordOptions` properties, so no call takes a format argument.

## Read

The header names the columns. A declared `field` types every cell, each read through its column's value door; without one, the first `infer_row_size` records (1,024) are sampled to type each column, answered first, and the rest streams under the field they inferred. A read holds one batch at a time, and the dialect is a property of one read, so a document another writer saved with `;` reads with `separator=";"` and nothing else.

=== "Rust"

    ```rust
    use yggdryl::holder::Holder;
    use yggdryl::local::LocalFolder;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{DataType, IOMedia, Scalar, StructType};

    let root = LocalFolder::temporary()?.path()?.join("yggdryl-docs-csv-read");
    std::fs::create_dir_all(&root)?;

    // A `;` document another writer saved: the separator is a property of one
    // read, and the sample types each column.
    let path = root.join("trades.csv");
    std::fs::write(&path, b"id;symbol\n1;AAPL\n2;\n3;\"\"\n")?;
    let handle = Holder::local(&path)?;
    let mut dialect = handle.record_options()?;
    dialect.set_csv_separator(b';')?;
    assert_eq!(
        handle.read_arrow_field(&dialect)?.dtype(),
        &DataType::from_str("struct<id: int64, symbol: utf8>")?
    );
    // Under the default dialect the same header is one column.
    assert_eq!(handle.read_arrow_field(&handle.record_options()?)?.field_len(), 1);

    // A declared field is the contract every cell is read under: an empty cell
    // is null and a quoted empty cell the empty text.
    let field = DataType::from(StructType::from_fields([
        DataType::Int32.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])?)
    .required_field("trade");
    let mut symbols = Vec::new();
    for records in handle.read_arrow(Some(&dialect.clone().with_field(field)))? {
        let records = records?;
        let symbol = records.child("symbol").expect("a symbol column");
        for row in 0..symbol.len() {
            symbols.push(symbol.scalar(row)?);
        }
    }
    assert_eq!(symbols, [Scalar::from("AAPL"), Scalar::Null, Scalar::from("")]);

    std::fs::remove_dir_all(&root)?;
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    with tempfile.TemporaryDirectory() as directory:
        path = pathlib.Path(directory) / "trades.csv"

        # A `;` document another writer saved: the separator is a property of one
        # read, and the sample types each column.
        path.write_bytes(b'id;symbol\n1;AAPL\n2;\n3;""\n')
        handle = IOBase(path)
        inferred = handle.read_arrow_field(separator=";")
        assert [(child.name, str(child.dtype)) for child in inferred.dtype] == [
            ("id", "int64"),
            ("symbol", "utf8"),
        ]
        # Under the default dialect the same header is one column.
        assert len(handle.read_arrow_field().dtype) == 1

        # A declared field is the contract every cell is read under: an empty cell
        # is null and a quoted empty cell the empty text.
        field = pa.schema([pa.field("id", pa.int32(), nullable=False), pa.field("symbol", pa.string())])
        table = handle.read_arrow_reader(separator=";", field=field).read_all()
        assert table.schema.field("id").type == pa.int32()
        assert table.column("symbol").to_pylist() == ["AAPL", None, ""]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { Field, IOBase, fields } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-csv-'))
    const file = path.join(root, 'trades.csv')

    // A `;` document another writer saved: the separator is a property of one
    // read, and the sample types each column.
    fs.writeFileSync(file, 'id;symbol\n1;AAPL\n2;\n3;""\n')
    const handle = new IOBase(file)
    assert.deepEqual(
      Array.from(handle.readArrowField({ separator: ';' }).dtype, (child) => [child.name, String(child.dtype)]),
      [
        ['id', 'int64'],
        ['symbol', 'utf8'],
      ],
    )
    // Under the default dialect the same header is one column.
    assert.equal(handle.readArrowField().dtype.length, 1)

    // A declared field is the contract every cell is read under: an empty cell
    // is null and a quoted empty cell the empty text.
    const field = fields.struct('trade', [new Field('id', 'int32', false), Field.from('symbol: utf8')], {
      nullable: false,
    })
    assert.deepEqual(
      [...handle.readRecords({ separator: ';', field })].map((row) => row.symbol),
      ['AAPL', null, ''],
    )

    fs.rmSync(root, { recursive: true, force: true })
    ```

- Inference climbs one ladder per column over the sampled non-null cells - `boolean`, `int64`, `float64`, `date32`, `datetime64(ns, UTC)` (ISO 8601 with a `T` or a space, an offset or `Z` optional, a naive spelling read as UTC), else `utf8` - a cell fitting a rung when that datatype's value door reads it (`true`/`false` in any case, a sign, an exponent, `NaN`, `inf`, blanks around it), and the empty text `""` proving no rung; a column whose sample is all null is `utf8`; every inferred column is nullable and the root is named after the options (`row`). The sampled rows are answered first and the rest streams under that field. An inferred datatype is a reading of the sample, not a contract, so a later cell its column cannot read is refused rather than nulled, under `safe` or not: `$[4].i: expected int64, the datatype the first 2 records (infer_row_size) infer, got "x" in row 6 of <url>; declare the column or raise infer_row_size to sample it`.
- A declared field with a header matches columns by name: a column the field does not name is skipped, a nullable column the header does not state is null, a required one is refused at `$.header`. Without a header the field names the columns by position, and a declared column past the first record's width is missing as an unstated one is: null where nullable, refused at `$.header` where required. Every cell is read through its column's value door, `Field::scalar` over its text, so it reads as the same text does anywhere else. A cell a required column cannot read is refused at `$[row].<column>` naming the cell and the line; a nullable column takes it as null under `safe`, the [cast rule](../types/cast.md#required-columns). A `datetime64` column declared with a zone reads a naive spelling, which its door has no reading for, as a wall clock in that zone, as a text capture's [`autotype`](text.md) does.
- A record with more or fewer cells than the header is refused by row - `$[1]: expected 2 cells, got 3 in row 3 of <url>`, the path the 0-based data row and the reason its physical line - the rows before it answered first, nothing widened.
- A blank record is a separator rather than a record, the last record may lack its terminator, a UTF-8 byte-order mark at the start is framing, and bytes after a closing quote are content. A stream ending inside a quoted cell is refused at the record the quote opened in - `$[1]: expected a closing quote, got the end of the stream inside the cell opened in row 3 of <url>`, or `$.header` - by every read and by `row_size`, rather than read as one cell holding every record after it.

## Write

A write renders every leaf as the text it reads back from and writes the header first, once. A null is the first `null_values` spelling - the empty cell, by default - and the empty text is quoted, so the two read back apart. An append writes after the stored rows and never repeats the header; a merge keys through `merge_by`.

=== "Rust"

    ```rust
    use yggdryl::holder::Holder;
    use yggdryl::local::LocalFolder;
    use yggdryl::media::{IORecordOptions, Media};
    use yggdryl::{Codec, DataType, IOMedia, Scalar, StructType};

    struct Trade(i64, Option<&'static str>);

    impl From<Trade> for Scalar {
        fn from(row: Trade) -> Self {
            Scalar::from_sequence([Scalar::from(row.0), row.1.map_or(Scalar::Null, Scalar::from)])
        }
    }

    let root = LocalFolder::temporary()?.path()?.join("yggdryl-docs-csv-write");
    std::fs::create_dir_all(&root)?;
    let field = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])?)
    .required_field("trade");

    // The name says CSV and gzip: the medium composes over the coding, and the
    // rows cross the declared field's contract on their way to the cells.
    let mut coded = Holder::local(root.join("trades.csv.gz"))?.into_declared_media();
    assert!(matches!(&coded, Holder::Media(media) if matches!(media.as_ref(), Media::Csv(_))));
    let options = coded.record_options()?.with_field(field);
    coded.overwrite_records([Trade(1, Some("AAPL")), Trade(2, None), Trade(3, Some(""))], &options)?;
    let stored = std::fs::read(root.join("trades.csv.gz"))?;
    assert_eq!(&stored[..2], &[0x1F, 0x8B]);
    // A null is the empty cell and the empty text is quoted, so the two read back apart.
    assert_eq!(Codec::Gzip.load(&stored)?, b"id,symbol\n1,AAPL\n2,\n3,\"\"\n");

    // An append writes after the stored rows and never repeats the header.
    coded.append_records([Trade(4, Some("MSFT"))], &options)?;
    let appended = Codec::Gzip.load(&std::fs::read(root.join("trades.csv.gz"))?)?;
    assert_eq!(appended, b"id,symbol\n1,AAPL\n2,\n3,\"\"\n4,MSFT\n");

    std::fs::remove_dir_all(&root)?;
    ```

=== "Python"

    ```python
    import gzip
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    with tempfile.TemporaryDirectory() as directory:
        root = pathlib.Path(directory)
        field = pa.schema([pa.field("id", pa.int64(), nullable=False), pa.field("symbol", pa.string())])

        # The name says CSV and gzip: the handle composes the medium over the
        # coding, and the rows cross the declared field's contract.
        coded = IOBase(root / "trades.csv.gz")
        assert type(coded).__name__ == "Csv"
        options = coded.record_options()
        options.field = field
        rows = [{"id": 1, "symbol": "AAPL"}, {"id": 2, "symbol": None}, {"id": 3, "symbol": ""}]
        coded.overwrite_records(rows, options=options)
        # A null is the empty cell and the empty text is quoted, so the two read back apart.
        assert gzip.decompress((root / "trades.csv.gz").read_bytes()) == b'id,symbol\n1,AAPL\n2,\n3,""\n'
        assert list(IOBase(root / "trades.csv.gz").read_records(options=options)) == rows

        # An append writes after the stored rows and never repeats the header.
        coded.append_records([{"id": 4, "symbol": "MSFT"}], options=options)
        assert gzip.decompress((root / "trades.csv.gz").read_bytes()) == (
            b'id,symbol\n1,AAPL\n2,\n3,""\n4,MSFT\n'
        )
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const zlib = require('node:zlib')
    const { Field, IOBase, fields } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-csv-'))
    const field = fields.struct('trade', [new Field('id', 'int64', false), Field.from('symbol: utf8')], {
      nullable: false,
    })
    const stored = () => zlib.gunzipSync(fs.readFileSync(path.join(root, 'trades.csv.gz'))).toString()

    // The name says CSV and gzip: the handle composes the medium over the
    // coding, and the rows cross the declared field's contract.
    const coded = new IOBase(path.join(root, 'trades.csv.gz'))
    assert.equal(String(coded.recordOptions().mimeType), 'text/csv')
    const rows = [{ id: 1n, symbol: 'AAPL' }, { id: 2n, symbol: null }, { id: 3n, symbol: '' }]
    coded.overwriteRecords(rows, { field })
    // A null is the empty cell and the empty text is quoted, so the two read back apart.
    assert.equal(stored(), 'id,symbol\n1,AAPL\n2,\n3,""\n')
    assert.deepEqual([...new IOBase(path.join(root, 'trades.csv.gz')).readRecords({ field })], rows)

    // An append writes after the stored rows and never repeats the header.
    coded.appendRecords([{ id: 4n, symbol: 'MSFT' }], { field })
    assert.equal(stored(), 'id,symbol\n1,AAPL\n2,\n3,""\n4,MSFT\n')

    fs.rmSync(root, { recursive: true, force: true })
    ```

- Writing renders every leaf as the text it reads back from: text as itself, numbers, booleans, temporals (`NaN`, `inf`, `-inf` included), codes and UUIDs in their canonical spelling, bytes as base64, a nested value as compact JSON, which a declared nested column reads back; a cell it cannot spell is refused at `$[row].<column>`. A null is its first `null_values` spelling verbatim, so a spelling holding the separator, the quote or a line break, or opening the record with the comment byte, is refused at `$.null_values` where a null is written. A record is never a blank line: the only cell of a record, empty, is written `""` - the empty text, which a column that is not text reads as its null - and a null alone in its record under a text column needs a spelling that is not empty, refused at `$[row].<column>` otherwise.
- A write onto a stored document completes onto its header, never onto its sample, since a CSV stores text: the header's names in its order, each typed as the incoming column of that name, so the rows render as an overwrite renders them; a header column the rows do not carry is written empty, and a column the header does not name is refused at `$.header`, naming it and the header. Without a header the columns are positions: the rows are written as they arrive, and a stored record of another width is refused at `$`, naming both counts. `append` writes after the tail and reads no stored row; `merge` keys through `merge_by` and reads the stored cells under the same columns, refusing at `$[row].<column>` - rather than nulling and writing back lost - a cell its column cannot read.

## Dialect

The dialect is one `RecordOptions` property per setting. It reads `None`/`null` on another encoding's options, and setting it there is refused.

```text
options.set_csv_separator(b';')?          // Rust RecordOptions: csv_<setting> reads, set_csv_<setting> validates
CsvOptions::new().with_separator(b';')?   // Rust CsvOptions: <setting>, set_<setting>, with_<setting>; tsv() the tab
options.separator = ";"                   // Python property; a byte role is one character or one byte
handle.read_records(separator=";")        // Python: every read and write takes a setting by name, on a copy
options.withSeparator(';')                // JavaScript property or with<Setting>: nullValues, inferRowSize
handle.readRecords({ separator: ';' })    // JavaScript: every read and write takes it in the options object
```

| Setting | Default | Rule |
| --- | --- | --- |
| `separator` | `,`; `\t` under `text/tab-separated-values` | the byte between two cells: one ASCII byte, neither a line break nor a byte another role holds; a tab separator makes the options `text/tab-separated-values` |
| `quote` | `"` | wraps a cell holding the separator, the quote, a line break, a leading or trailing blank, a null spelling (the empty text, by default) or a comment byte opening a record; a quote inside is doubled; `None`/`null` quotes nothing on write, reads a quote as content, and refuses by name a cell that would need quoting |
| `escape` | none | set, the byte after it inside a quoted cell is content, a quote inside is written behind it and the escape byte itself is doubled; unset, a quote inside a quoted cell is doubled (RFC 4180) |
| `comment` | none | a record opening with it is skipped and never counted |
| `header` | on | read: the first record names the columns, an empty name `column_<i>` and a name stated twice refused at `$.header`; off: `column_1`, `column_2`, ...; write: the names are written first, once, an append never repeating them |
| `null_values` | `[""]` | the spellings of an absent value: an unquoted cell spelling one is null and a null is written as the first; a quoted cell is never one, so `""` is the empty text; a spelling listed twice is refused at `$.null_values`, and an empty list leaves a null nothing to be written as, refused at `$[row].<column>` |
| `trim` | off | ASCII blanks around an unquoted cell, and around a quoted one's quotes, are dropped before it is read |
| `infer_row_size` | 1024 | the records sampled to type each column when no field is declared; `0` is refused at `$.infer_row_size` |
| `linesep` | `\n` | Rust only, `CsvOptions::with_linesep`: the terminator a write ends each record with; a read accepts `\n` and `\r\n` whatever it says, and a `\r` alone ends nothing |

## Edges

- While a `Csv` handle is open its schema is cached, and answered only for options reading the document as the ones it was read under: another separator, header, quote, escape, comment, `null_values`, `trim` or `infer_row_size` reads it afresh.
- An empty document declares no schema - `read_arrow_field` is refused at `$.csv` - and reads as no rows; declared, it is the declared schema and no rows.
- `row_size` counts the records in one pass and reads no cell; `column_size` is the header's width; both, like `read_arrow_field`, cost one `pstream_bytes` of the handle ([Call counts](../holder/index.md#call-counts)).

## Performance

No table yet: the release run of the command below writes it, on the machine it names, and nothing here is measured in a debug build or edited by hand.

```bash
cargo bench -p yggdryl --bench media -- csv
```
