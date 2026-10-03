# Excel

An Office Open XML workbook (`.xlsx`): one worksheet of it read and written as records, and the whole workbook - its sheets and every cell - for random access.

## Overview

| | |
| --- | --- |
| Declared by | `application/vnd.openxmlformats-officedocument.spreadsheetml.sheet`, `.xlsx` |
| Build | default |
| Rust | `yggdryl::excel`: `Excel<H>` over any handle with `ExcelOptions`, the free `read_field`, `read_batch_reader` and `overwrite_arrow_reader`; [`Workbook`](https://docs.rs/yggdryl/latest/yggdryl/excel/struct.Workbook.html), [`Sheet`](https://docs.rs/yggdryl/latest/yggdryl/excel/struct.Sheet.html), [`Cell`](https://docs.rs/yggdryl/latest/yggdryl/excel/struct.Cell.html), `CellRef` and `CellRange` for the [workbook](#workbook) |
| Python | any `IOBase` whose name declares a workbook; `yggdryl.excel`: `Workbook`, `Sheet`, `Cell`, `CellRef`, `CellRange`, `Row` |
| JavaScript | any `IOBase` whose name declares a workbook; `Workbook`, `Sheet`, `Cell`, `CellRef`, `CellRange` and the `excel` namespace |
| Settings | [`ExcelOptions`](https://docs.rs/yggdryl/latest/yggdryl/excel/struct.ExcelOptions.html): `sheet`, `header` and `range`, beside the shared [`RecordOptions`](index.md#options) |
| Refused | a coded name such as `.xlsx.gz` or `.xlsx.zst`: the package is deflated inside |

An Office Open XML workbook (`.xlsx`) is a ZIP package of XML parts, and one worksheet of it is the record medium: the first row of the range names the columns, every cell below is a value, and a write renders the part row by row as the batches arrive. The whole workbook is the random-access side of the same medium - [`Workbook`](https://docs.rs/yggdryl/latest/yggdryl/excel/struct.Workbook.html), [`Sheet`](https://docs.rs/yggdryl/latest/yggdryl/excel/struct.Sheet.html) and [`Cell`](https://docs.rs/yggdryl/latest/yggdryl/excel/struct.Cell.html) - any cell by its `A1` reference, a sheet's rows laid out from a `Serie` and read back as one.

## Read

Three facts about the file decide what a read answers. A number cell is a `float64`, because the file stores every number as a double: `1` reads as `1.0`, and a declared `int64` column reads it back as the integer it was written as. A cell's number format is its datatype - a serial under a date format is a `date32`, under a clock a `time32(ms)`, under a date and a clock a `datetime64(ms)`, under `[h]:mm:ss` a `duration64(ms)` - in the workbook's date system, 1900 or 1904. A boolean cell (`t="b"`) holds `1` or `0` and reads through the [boolean table](../types/numeric/boolean.md#the-one-text-reader), so a `TRUE` another writer left reads too and text the table does not read is refused naming the cell's text; an empty one is null. A number cell's text reads through the one float grammar, and an infinity or a `NaN`, which no spreadsheet stores, is refused. Text is escaped as ECMA-376 spells it: a control character, a carriage return and a literal `_x0041_` are written `_xHHHH_` and read back as themselves, which Excel does and openpyxl leaves unread.

`sheet` addresses a worksheet - the first one unless it names another, and a sheet the workbook lacks reads as the empty stream - `header` says whether the range's first row names the columns (by their letters otherwise), and `range` the cells addressed (`A3:F`, `B:D`, `2:10`). What a read costs, in calls to the handle: the package index is the archive's own two reads (its size and the tail holding the directory) and each part read once - the sheet streamed, the shared strings and the styles held for the workbook's life. An inferred read passes the part twice, once to learn the field and once to stream the rows under it; a declared read passes it once.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Float64Array, Int64Array, RecordBatch, StringArray};
    use yggdryl::holder::Buffer;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{DataType, IOMedia, MimeType, StructType};

    let field = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
        DataType::Float64.required_field("price"),
    ])?)
    .required_field("row");
    let batch = RecordBatch::try_new(
        field.clone().into_arrow_schema()?,
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])),
            Arc::new(StringArray::from(vec![Some("AAPL"), None, Some("MSFT")])),
            Arc::new(Float64Array::from(vec![187.5, 410.25, -0.5])),
        ],
    )?;
    let mut handle = Buffer::new().with_media_type(MimeType::XLSX.into());
    let options = handle.record_options()?;
    handle.overwrite_arrow_batch(batch.clone(), &options)?;

    // A number cell is a float64: the file stores every number as a double.
    assert_eq!(handle.read_arrow_field(&options)?.fields()[0].dtype(), &DataType::Float64);
    // Declared, the ids read back as the int64 they were written as.
    let declared = handle.read_arrow_reader(&options.clone().with_field(field))?.next().expect("one batch")?;
    assert_eq!(declared.column(0), batch.column(0));

    // A range's first row is its header.
    let mut narrowed = options.clone();
    narrowed.set_excel_range(Some("B1:C".parse()?))?;
    assert_eq!(handle.read_arrow_field(&narrowed)?.field_len(), 2);

    // A sheet the workbook lacks reads as the empty stream.
    let mut missing = options.clone();
    missing.set_excel_sheet(Some("Missing"))?;
    assert_eq!(handle.read_arrow_reader(&missing)?.count(), 0);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    table = pa.table({
        "id": pa.array([1, 2, 3], pa.int64()),
        "symbol": ["AAPL", None, "MSFT"],
        "price": [187.5, 410.25, -0.5],
    })
    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.xlsx")
    handle.overwrite_arrow_table(table)

    # A number cell is a float64: the file stores every number as a double.
    assert handle.read_arrow_reader().read_all().column("id").to_pylist() == [1.0, 2.0, 3.0]
    # Declared, the ids read back as the int64 they were written as.
    assert handle.read_arrow_reader(field=table.schema).read_all() == table

    # A range's first row is its header; without one the columns are named by
    # their letters, and the range leaves the header row out.
    assert handle.read_arrow_reader(range="B1:C").read_all().column_names == ["symbol", "price"]
    lettered = handle.read_arrow_reader(header=False, range="A2:C").read_all()
    assert lettered.column_names == ["A", "B", "C"]
    assert lettered.num_rows == 3

    # A sheet the workbook lacks reads as the empty stream.
    assert handle.read_arrow_reader(sheet="Missing").read_all().num_rows == 0
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { IOBase, Serie } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-excel-'))
    const table = new arrow.Table({
      id: arrow.vectorFromArray([1n, 2n, 3n], new arrow.Int64()),
      symbol: arrow.vectorFromArray(['AAPL', null, 'MSFT'], new arrow.Utf8()),
      price: arrow.vectorFromArray([187.5, 410.25, -0.5], new arrow.Float64()),
    })
    const handle = new IOBase(path.join(root, 'trades.xlsx'))
    handle.overwriteArrowTable(table)

    // A number cell is a float64: the file stores every number as a double.
    assert.deepEqual([...handle.readArrowReader().intoTable().getChild('id')], [1, 2, 3])
    // Declared, the ids read back as the int64 they were written as.
    const field = Serie.fromArrowBatch(table).field
    assert.deepEqual([...handle.readArrowReader({ field }).intoTable().getChild('id')], [1n, 2n, 3n])

    // A range's first row is its header; without one the columns are named by
    // their letters, and the range leaves the header row out.
    const narrowed = handle.readArrowReader({ range: 'B1:C' }).intoTable()
    assert.deepEqual(narrowed.schema.fields.map((column) => column.name), ['symbol', 'price'])
    const lettered = handle.readArrowReader({ header: false, range: 'A2:C' }).intoTable()
    assert.deepEqual(lettered.schema.fields.map((column) => column.name), ['A', 'B', 'C'])
    assert.equal(lettered.numRows, 3)

    // A sheet the workbook lacks reads as the empty stream.
    assert.equal(handle.readArrowReader({ sheet: 'Missing' }).intoTable().numRows, 0)

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Write

A write renders the part row by row as the batches arrive, with no row held past its batch: the first row names the columns while `header` is on, and each value is the cell its datatype spells - a boolean as `t="b"`, a date, a time, a naive datetime or a duration as its serial under the number format that reads it back, a float that is not a number as `#NUM!`, text inline - while a zoned datetime, an interval, a decimal, a code or bytes is written as the text the XML codec spells, and a nested value as its JSON. A write into an opened package keeps every other part as it was, the sheets it does not touch included, and `sheet` naming one the workbook lacks adds it beside the others; it reads the package twice, once for the field the rows are shaped onto and once to carry the other parts across. A coded name such as `.xlsx.gz` is refused before a byte is written - the package is deflated inside, as Parquet's pages are - and a read and `Workbook::open` refuse it too, naming the coding to drop.

=== "Rust"

    ```rust
    use yggdryl::excel::Workbook;
    use yggdryl::holder::Buffer;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{DataType, IOBase, IOMedia, MimeType, Scalar, StructType, Url};

    let trades = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])?)
    .required_field("row");
    let notes = DataType::from(StructType::from_fields([DataType::utf8().required_field("note")])?)
        .required_field("row");
    let note = |text: &str| Scalar::from_sequence([Scalar::from(text)]);

    // The first worksheet, unless `sheet` names another.
    let mut handle = Buffer::new().with_media_type(MimeType::XLSX.into());
    let options = handle.record_options()?.with_field(trades);
    handle.overwrite_records(
        [
            Scalar::from_sequence([Scalar::from(1_i64), Scalar::from("AAPL")]),
            Scalar::from_sequence([Scalar::from(2_i64), Scalar::Null]),
        ],
        &options,
    )?;

    // A sheet the workbook lacks is added beside the others, which stay as they were;
    // an append adds rows under the sheet's own.
    let mut on_notes = handle.record_options()?.with_field(notes);
    on_notes.set_excel_sheet(Some("Notes"))?;
    handle.overwrite_records([note("a"), note("b")], &on_notes)?;
    handle.append_records([note("c")], &on_notes)?;

    let workbook = Workbook::from_bytes(handle.read_all_bytes()?)?;
    assert_eq!(workbook.sheet_names(), ["Sheet1", "Notes"]);
    assert_eq!(workbook.sheet("Notes")?.scalar("A4".parse()?), Scalar::from("c"));
    assert_eq!(workbook.sheet("Sheet1")?.scalar("B2".parse()?), Scalar::from("AAPL"));

    // A coding around the package is refused, and nothing is written.
    let mut coded = Buffer::new().with_media_type(Url::from_str("file:///book.xlsx.gz")?.media_type());
    let refused = coded.overwrite_records([note("d")], &on_notes).unwrap_err();
    assert!(refused.to_string().contains("expected an uncompressed xlsx handle"), "{refused}");
    assert_eq!(coded.size(), 0);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa
    import pytest

    from yggdryl import IOBase
    from yggdryl.excel import Workbook

    root = pathlib.Path(tempfile.mkdtemp())
    handle = IOBase(root / "book.xlsx")

    # The first worksheet, unless `sheet` names another.
    handle.overwrite_arrow_table(pa.table({"id": pa.array([1, 2], pa.int64()), "symbol": ["AAPL", None]}))

    # A sheet the workbook lacks is added beside the others, which stay as they
    # were; an append adds rows under the sheet's own.
    handle.overwrite_arrow_table(pa.table({"note": ["a", "b"]}), sheet="Notes")
    handle.append_arrow_table(pa.table({"note": ["c"]}), sheet="Notes")

    workbook = Workbook.open(handle)
    assert workbook.sheet_names == ["Sheet1", "Notes"]
    assert workbook["Notes"]["A4"].as_py() == "c"
    assert workbook["Sheet1"]["B2"].as_py() == "AAPL"

    # A coding around the package is refused, and nothing is written.
    coded = IOBase(root / "book.xlsx.gz")
    with pytest.raises(ValueError, match="expected an uncompressed xlsx handle"):
        coded.overwrite_arrow_table(pa.table({"note": ["d"]}))
    assert coded.size() == 0
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { IOBase, Workbook } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-excel-'))
    const handle = new IOBase(path.join(root, 'book.xlsx'))
    const notes = (values) => new arrow.Table({ note: arrow.vectorFromArray(values, new arrow.Utf8()) })

    // The first worksheet, unless `sheet` names another.
    handle.overwriteArrowTable(
      new arrow.Table({
        id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
        symbol: arrow.vectorFromArray(['AAPL', null], new arrow.Utf8()),
      }),
    )

    // A sheet the workbook lacks is added beside the others, which stay as they
    // were; an append adds rows under the sheet's own.
    handle.overwriteArrowTable(notes(['a', 'b']), { sheet: 'Notes' })
    handle.appendArrowTable(notes(['c']), { sheet: 'Notes' })

    const workbook = Workbook.open(handle)
    assert.deepEqual(workbook.sheetNames, ['Sheet1', 'Notes'])
    assert.equal(workbook.sheet('Notes').cell('A4').value.asJs(), 'c')
    assert.equal(workbook.sheet('Sheet1').cell('B2').value.asJs(), 'AAPL')

    // A coding around the package is refused, and nothing is written.
    const coded = new IOBase(path.join(root, 'book.xlsx.gz'))
    assert.throws(() => coded.overwriteArrowTable(notes(['d'])), /expected an uncompressed xlsx handle/)
    assert.equal(coded.size(), 0)

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Workbook

Random access reads and edits any cell by its `A1` reference, lays a sheet's rows out from a `Serie` and reads them back as one, adds, renames and removes sheets, and writes the package back with every other part as it was; a sheet is parsed on first access and held for the workbook's life.

=== "Rust"

    ```rust
    use yggdryl::excel::{CellRef, Sheet, Workbook};
    use yggdryl::holder::Buffer;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{DataType, IOBase, IOMedia, MimeType, Scalar, Serie, StructType};

    let field = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
        DataType::Date32.required_field("traded"),
    ])?)
    .required_field("row");
    let rows = Serie::from_scalars(field.clone(), [
        Scalar::from_sequence([Scalar::from(1_i64), Scalar::from("AAPL"), Scalar::date32(19_723)]),
        Scalar::from_sequence([Scalar::from(2_i64), Scalar::Null, Scalar::date32(19_724)]),
    ])?;

    // The record path: one worksheet, written and read like every medium.
    let mut handle = Buffer::new().with_media_type(MimeType::XLSX.into());
    let options = handle.record_options()?.with_field(field.clone());
    handle.overwrite_arrow_batch(rows.clone().into_arrow_batch()?, &options)?;
    let read = handle.read_arrow_reader(&options)?.map(|batch| batch.unwrap().num_rows()).sum::<usize>();
    assert_eq!(read, 2);
    // Inferred, the id column is the float64 the file holds.
    let inferred = handle.read_arrow_field(&handle.record_options()?)?;
    assert_eq!(inferred.fields()[0].dtype(), &DataType::Float64);
    assert_eq!(inferred.fields()[2].dtype(), &DataType::Date32);

    // The random-access path: any cell of any sheet, and a sheet as a Serie.
    let mut workbook = Workbook::from_bytes(handle.read_all_bytes()?)?;
    let sheet = workbook.sheet_mut("Sheet1")?;
    assert_eq!(sheet.scalar("B2".parse()?), Scalar::from("AAPL"));
    assert_eq!(sheet.scalar(CellRef::new(2, 1)), Scalar::Null);
    sheet.set_cell("D1".parse()?, "note")?;
    sheet.set_cell("D2".parse()?, 2.5)?;
    let back = sheet.clone().into_serie(Some(&field), true, Default::default())?;
    assert_eq!(back.len(), 2);

    let mut notes = Sheet::new("Notes")?;
    notes.write_serie("A1".parse()?, &rows, true)?;
    workbook.insert_sheet(notes)?;
    let reopened = Workbook::from_bytes(workbook.into_bytes()?)?;
    assert_eq!(reopened.sheet_names(), ["Sheet1", "Notes"]);
    assert_eq!(reopened.sheet("Sheet1")?.scalar("D2".parse()?), Scalar::from(2.5));
    ```

=== "Python"

    ```python
    import datetime
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase
    from yggdryl.excel import Sheet, Workbook

    with tempfile.TemporaryDirectory() as folder:
        path = pathlib.Path(folder) / "trades.xlsx"
        table = pa.table({
            "id": pa.array([1, 2], pa.int64()),
            "symbol": ["AAPL", None],
            "traded": [datetime.date(2024, 1, 1), datetime.date(2024, 1, 2)],
        })

        # The record path: one worksheet, written and read like every medium.
        handle = IOBase(path)
        handle.overwrite_arrow_table(table)
        assert handle.read_arrow_reader(field=table.schema).read_all() == table
        inferred = handle.read_arrow_reader().read_all()
        assert inferred.column("id").to_pylist() == [1.0, 2.0]
        handle.overwrite_arrow_table(pa.table({"note": ["a"]}), sheet="Notes")

        # The random-access path: any cell of any sheet, and a sheet as a Serie.
        workbook = Workbook.open(path)
        assert workbook.sheet_names == ["Sheet1", "Notes"]
        sheet = workbook["Sheet1"]
        assert sheet["B2"].as_py() == "AAPL"
        assert sheet["B3"] is None
        assert sheet["C2"].format == "date"
        sheet["D1"] = "note"
        sheet["D2"] = 2.5
        assert sheet.into_serie(table.schema).into_arrow_table() == table
        workbook.insert_sheet(Sheet.from_serie("Copy", table))
        workbook.write_into(path)
        assert Workbook.open(path)["Sheet1"]["D2"].as_py() == 2.5
        assert IOBase(path).read_arrow_reader(sheet="Copy").read_all().num_rows == 2
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { DataType, IOBase, Serie, Sheet, Workbook } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-excel-'))
    const file = path.join(root, 'trades.xlsx')
    const table = new arrow.Table({
      id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
      symbol: arrow.vectorFromArray(['AAPL', null], new arrow.Utf8()),
    })

    // The record path: one worksheet, written and read like every medium.
    const handle = new IOBase(file)
    handle.overwriteArrowTable(table)
    const field = Serie.fromArrowBatch(table).field
    assert.deepEqual([...handle.readArrowReader(handle.recordOptions().withField(field)).intoTable().getChild('id')], [1n, 2n])
    assert.deepEqual([...handle.readArrowReader().intoTable().getChild('id')], [1, 2])
    handle.overwriteArrowTable(new arrow.Table({ note: arrow.vectorFromArray(['a'], new arrow.Utf8()) }), handle.recordOptions().withSheet('Notes'))

    // The random-access path: any cell of any sheet, and a sheet as a Serie.
    const workbook = Workbook.open(file)
    assert.deepEqual(workbook.sheetNames, ['Sheet1', 'Notes'])
    const sheet = workbook.sheet('Sheet1')
    assert.equal(sheet.cell('B2').value.asJs(), 'AAPL')
    assert.equal(sheet.cell('B3'), null)
    sheet.setCell('D1', 'note')
    sheet.setCell('D2', new DataType('date32').scalar('2024-01-02'))
    assert.equal(sheet.cell('D2').format, 'date')
    assert.equal(sheet.intoSerie(field).asJs().length, 2)
    workbook.insertSheet(Sheet.fromSerie('Copy', table))
    workbook.writeInto(file)
    assert.equal(Workbook.open(file).sheet('Sheet1').cell('D1').value.asJs(), 'note')
    assert.equal(new IOBase(file).readArrowReader(handle.recordOptions().withSheet('Copy')).intoTable().numRows, 2)
    fs.rmSync(root, { recursive: true, force: true })
    ```

`rust/tests/iobase_calls.rs` pins the record doors' call counts and `rust/tests/excel/workbook.rs` the workbook's, and `rust/tests/allocations.rs` pins that a cell read, a reference parse and a range test allocate nothing.

## Performance

### Record and workbook doors

One release run of the `media` Criterion target on one Linux x86_64 container - Intel Xeon @ 2.80 GHz, 4 cores, 15 GiB; rustc 1.97.0, release profile (thin LTO, one codegen unit) - medians of 100 samples, on 2026-10-02. The workbook holds 10,000 rows of `id: int64`, `symbol: utf8` with every fifth row null, `price: float64`, `live: boolean` and `traded: date32`, written by this crate.

| 10,000 rows, five columns | median | rows/s |
| --- | ---: | ---: |
| `overwrite_arrow_batch` into a new workbook | 47.16 ms | 212k rows/s |
| `read_arrow_reader`, declared field | 55.58 ms | 180k rows/s |
| `read_arrow_field`, then `read_arrow_reader`: inferred | 143.7 ms | 69.6k rows/s |
| `Workbook::from_bytes`, then one cell | 56.29 ms | - |
| `Workbook::from_bytes`, then the sheet `into_serie` | 74.76 ms | 134k rows/s |
| `Sheet::from_serie` | 15.26 ms | 655k rows/s |

The inferred read passes the part twice - once for the field, once for the rows - and costs about two and a half declared reads. One cell through `Workbook` parses its sheet whole on first access, so it costs what a declared read does, and every cell after it is answered from the parsed sheet.

```bash
cargo bench -p yggdryl --bench media -- media/excel
```

### Against openpyxl

One run of `python/benchmarks/media/excel.py --repeat 5` on the same host: CPython 3.11.15, PyArrow 25.0.1, openpyxl 3.1.5, the release extension. Every case reads or writes a workbook this crate wrote - 10,000 or 100,000 rows of an `int64` key, one of eight symbols, a price, a flag and a `date32` - after the rows openpyxl reads were compared with this crate's, value for value: openpyxl walks a read-only sheet, one Python object per cell, and writes a write-only one row by row. Each time is the best of five after a warm-up, and the ratio is openpyxl's over this crate's, so above one is in this crate's favor.

| case | openpyxl | yggdryl | ratio |
| --- | ---: | ---: | ---: |
| read 100,000 rows, declared field | 6,791 ms | 563.1 ms | 12.1x |
| read 100,000 rows, inferred field | 6,531 ms | 1,008 ms | 6.5x |
| write 100,000 rows | 8,474 ms | 954.6 ms | 8.9x |
| read 10,000 rows, declared field | 597.6 ms | 58.7 ms | 10.2x |
| read 10,000 rows, inferred field | 635.9 ms | 99.3 ms | 6.4x |
| write 10,000 rows | 843.3 ms | 87.9 ms | 9.6x |
| open the workbook and read one cell | 205.6 ms | 57.5 ms | 3.6x |
| a sheet of 10,000 rows from a `Serie` and back, against openpyxl reading every row | 699.3 ms | 37.7 ms | 18.5x |

```bash
VIRTUAL_ENV=python/.venv python/.venv/bin/python -m maturin develop --release -m python/Cargo.toml
python/.venv/bin/python -m pip install openpyxl
python/.venv/bin/python python/benchmarks/media/excel.py --repeat 5
```
