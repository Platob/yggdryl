# Plain text

Records cut from text: one record per line, or per framed chain under `framing`, each read as the [event](../graph/event.md) a line is - its `body` the payload, and a `rowheader` regex lifting typed columns off its head.

## Overview

| | |
| --- | --- |
| Declared by | `text/plain`, `.txt`, `.log` |
| Build | default |
| Rust | `yggdryl::text`: `Text<H>` over any handle with `TextOptions`, `read_text_lines` and the `TextLine` it yields, and `into_arrow_batch`, `into_arrow_reader`, `from_arrow_batch`, `from_arrow_reader` between lines and Arrow |
| Python | any `IOBase` whose name declares text, `into_text(options)`, `read_text_lines`, `TextOptions`, `TextLine` |
| JavaScript | any `IOBase` whose name declares text, `intoText(options)`, `readTextLines`, `TextOptions`, `TextLine` |
| Settings | `TextOptions`: `rowheader`, `framing`, `leading_fragment`, `max_record_byte_size`, `lstrip`, `rstrip`, `linesep`, `autotype`, `timezone`, `start_rownum`, `parse_mtime` and `rename_columns`, and the Rust-only `parse_mimetype` and `dedup_adjacent`, beside the shared batch, row and plan sections |
| Columns | the six element columns then the nine event columns of [the text line](../graph/schemas.md#the-text-line) - `curruuid` first, `currunix` seventh, `state` fifteenth - then `body: utf8 not null` (`mimetype` before it under `parse_mimetype`, `dropped_byte_size` after it under `max_record_byte_size`), then one column per row-header capture that feeds no event fact: a capture named `state`, `creaunix`, `recdunix`, `exprunix`, `prevunix`, `snapunix` or `prevuuid` - or `mtime`, under `parse_mtime` - states that fact in the event's own column |

The header comes off where the line is made, so its captures are the line's and the `body` is the payload past it. A line's bytes are decoded once: a charset the handle's media type declares - other than UTF-8 and US-ASCII - is decoded at the transport, and otherwise each byte that is not UTF-8 reads as the windows-1252 character it is ([Charsets](charsets.md)).

## Read

A read cuts each object into records - one per line, or, under `framing`, one per chain of lines a row header opens - and answers each as a row: the event the line is, its `body`, its captures.

`TextLine` exposes the [event identity](../graph/event.md#identity) and full-width `seqnum`: its UUIDv7 orders by millisecond and row-derived sequence, with the content payload seeded by `crosshashcode`. The row number is the line's [place](../fix/lifecycle.md#a-place-counts-one-instant), where it stands and never what it says: `currhashcode` is the XXH3-64 of the `body` and nothing else - no capture, state, predecessor or cross code - so two lines of byte-identical bodies share it and differ by `curruuid` alone, which sorts the lines of one millisecond by row. Its constructor takes a Python integer or JavaScript unsigned 64-bit `bigint` index; assigning Python's writable index recomputes `seqnum` and the identity.

An object's lines are one chain - they share the object as their cross code - so a read states when that chain began: every line whose own `creaunix` capture states none takes the earliest `currunix` the read has dated a line of its object by so far - never an instant after its own, the first line its own instant. A line the header does not date, of a handle with no time of its own, is dated by nothing and states no creation until a line that is dated; a `creaunix` capture that does not read as an instant stays refused by name. A line built by hand states what it is given.

Each line likewise states, as `prevunix`, the `currunix` the read dated the line cut before it by - none for the first line of an object and none after an undated line, a `prevunix` capture winning - and no `prevuuid`, neither of which a line's `curruuid` or `currhashcode` reads; each object read, each leaf of a folder or glob, starts again, and the [FIX text doors](../fix/arrow.md#a-column-is-the-caller-speaking-per-row) ignore a line's `prevunix`.

=== "Rust"

    ```rust
    use arrow_array::{Array as _, StringArray, UInt64Array};
    use yggdryl::media::IORecordOptions as _;
    use yggdryl::{IOBase as _, IOMedia as _};
    use yggdryl::holder::Buffer;
    use yggdryl::text::TextOptions;
    use yggdryl::Url;

    let text_source = Buffer::from_bytes(
        b"[INFO] id=7 first\r\n detail A\r[WARN] id=9 second\n detail B".to_vec(),
    )
    .with_media_type(Url::from_str("file:///app.log")?.media_type());

    let mut text_options = TextOptions::new();
    text_options.start_rownum = Some(1);
    text_options.set_rowheader(Some(r"^\[(?<level>[A-Z]+)\] id=(?<id>\d+) "))?;
    text_options.set_framing(true);
    let text_source = text_source.into_text_with(text_options);
    let record_options = text_source.record_options()?;

    let text_batch = text_source
        .read_arrow_reader(&record_options)?
        .next()
        .unwrap()?;
    // The fifteen event columns, then the body, then the header's captures.
    assert_eq!(text_batch.schema().fields().len(), 18);
    // The record's place is its row number, under `seqnum`.
    assert_eq!(
        text_batch
            .column_by_name("seqnum")
            .expect("the event's place")
            .as_any()
            .downcast_ref::<UInt64Array>()
            .unwrap()
            .values(),
        &[1, 3],
    );
    // The body is the record past the header the reader took off it.
    assert_eq!(
        text_batch
            .column_by_name("body")
            .expect("the record's body")
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .value(0),
        "first\n detail A",
    );
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase, TextOptions

    with tempfile.TemporaryDirectory() as directory:
        source = pathlib.Path(directory) / "app.log"
        source.write_bytes(
            b"[INFO] id=7 first\r\n detail A\r[WARN] id=9 second\n detail B"
        )

        options = TextOptions()
        options.start_rownum = 1
        options.rowheader = r"^\[(?<level>[A-Z]+)\] id=(?<id>\d+) "
        options.framing = True

        handle = IOBase(source).into_text(options)
        rows = list(handle.read_records())
        assert [row["seqnum"] for row in rows] == [1, 3]
        assert [row["body"] for row in rows] == [
            "first\n detail A",
            "second\n detail B",
        ]
        assert [row["id"] for row in rows] == [7, 9]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase, TextOptions } = require('yggdryl')

    const textRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-text-'))
    const textSource = path.join(textRoot, 'app.log')
    fs.writeFileSync(
      textSource,
      '[INFO] id=7 first\r\n detail A\r[WARN] id=9 second\n detail B',
    )

    const textOptions = new TextOptions()
    textOptions.startRownum = 1n
    textOptions.rowheader = '^\\[(?<level>[A-Z]+)\\] id=(?<id>\\d+) '
    textOptions.framing = true

    const textHandle = new IOBase(textSource).intoText(textOptions)
    const textRows = [...textHandle.readRecords()]
    assert.deepEqual(textRows.map((row) => row.seqnum), [1n, 3n])
    assert.deepEqual(
      textRows.map((row) => row.body),
      ['first\n detail A', 'second\n detail B'],
    )
    assert.deepEqual(textRows.map((row) => row.id), [7n, 9n])

    fs.rmSync(textRoot, { recursive: true, force: true })
    ```

A folder, a location ending in `/` and a glob such as `logs/*.log` read leaf by leaf, through `read_text_lines` and `row_size` as through the record reads: every text leaf beneath them in the listing's order, each the object it is - its own cross code, time, coding and row numbers - and a leaf's last line ends with its leaf. The container's [byte stream](../holder/index.md#streams-and-cursors) runs the same leaves together; a line never reads it.

=== "Rust"

    ```rust
    use yggdryl::Codec;
    use yggdryl::graph::Element as _;
    use yggdryl::holder::Holder;
    use yggdryl::text::{TextOptions, read_text_lines};

    let root = yggdryl::local::LocalFolder::temporary()?.path()?
        .join(format!("yggdryl-docs-text-leaves-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root)?;
    std::fs::write(root.join("a.log"), b"a1\na2")?;
    std::fs::write(root.join("b.log.gz"), Codec::Gzip.dump(b"b1\n")?)?;

    let lines = read_text_lines(&Holder::folder(&root)?, &TextOptions::new())?
        .collect::<yggdryl::Result<Vec<_>>>()?;
    // `a2` ends with its leaf, and the gzip leaf is its own decoded object.
    assert_eq!(lines.iter().map(|line| line.body()).collect::<Vec<_>>(), ["a1", "a2", "b1"]);
    assert!(lines[1].get_crosscode().ends_with("a.log"));
    assert!(lines[2].get_crosscode().ends_with("b.log.gz"));

    std::fs::remove_dir_all(&root)?;
    ```

=== "Python"

    ```python
    import gzip
    import pathlib
    import tempfile

    from yggdryl import IOBase, TextOptions

    root = pathlib.Path(tempfile.mkdtemp())
    (root / "a.log").write_bytes(b"a1\na2")
    (root / "b.log.gz").write_bytes(gzip.compress(b"b1\n"))

    lines = list(IOBase(root).read_text_lines(options=TextOptions()))
    # `a2` ends with its leaf, and the gzip leaf is its own decoded object.
    assert [line.body for line in lines] == ["a1", "a2", "b1"]
    assert lines[1].crosscode.endswith("a.log")
    assert lines[2].crosscode.endswith("b.log.gz")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const zlib = require('node:zlib')
    const { IOBase, TextOptions } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    fs.writeFileSync(path.join(root, 'a.log'), 'a1\na2')
    fs.writeFileSync(path.join(root, 'b.log.gz'), zlib.gzipSync('b1\n'))

    const lines = [...new IOBase(root).readTextLines(new TextOptions())]
    // `a2` ends with its leaf, and the gzip leaf is its own decoded object.
    assert.deepEqual(lines.map((line) => line.body), ['a1', 'a2', 'b1'])
    assert.ok(lines[1].crosscode.endsWith('a.log'))
    assert.ok(lines[2].crosscode.endsWith('b.log.gz'))

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Write

A write consumes the `body` column alone - non-null, non-empty text - and writes each body followed by the record terminator, `\n` unless `linesep` pins another; the event columns and the captures are the reader's to derive, so no write stores them. A body holding the terminator is refused, because it would read back as two records, and so is a `body` column that is not text.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::text::{LineSep, TextOptions};
    use yggdryl::{DataType, IOBase, IOMedia, Scalar, StructType, Url};

    let field = DataType::from(StructType::from_fields([DataType::utf8().required_field("body")])?)
        .required_field("row");
    let lines = |bodies: &[&str]| -> Vec<Scalar> {
        bodies.iter().map(|body| Scalar::from_sequence([Scalar::from(*body)])).collect()
    };

    let mut target = Buffer::new().with_media_type(Url::from_str("file:///out.log")?.media_type());
    let options = RecordOptions::from(TextOptions::new()).with_field(field.clone());

    // Each body is one line with its terminator; an append adds lines after the last.
    target.overwrite_records(lines(&["one", "two"]), &options)?;
    target.append_records(lines(&["three"]), &options)?;
    assert_eq!(target.read_all_bytes()?, b"one\ntwo\nthree\n");

    // A pinned terminator is the one written.
    let crlf = RecordOptions::from(TextOptions::new().with_linesep(LineSep::CRLF)).with_field(field);
    target.overwrite_records(lines(&["first"]), &crlf)?;
    assert_eq!(target.read_all_bytes()?, b"first\r\n");

    // A body holding the terminator would read back as two records.
    let error = target.append_records(lines(&["bad\nline"]), &options).unwrap_err();
    assert!(error.to_string().contains("without its record terminator"), "{error}");
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pytest

    from yggdryl import IOBase, TextOptions

    root = pathlib.Path(tempfile.mkdtemp())
    target = IOBase(root / "out.log")

    # Each body is one line with its terminator; an append adds lines after the last.
    target.overwrite_records([{"body": "one"}, {"body": "two"}])
    target.append_records([{"body": "three"}])
    assert target.read_bytes() == b"one\ntwo\nthree\n"

    # A pinned terminator is the one written.
    crlf = TextOptions()
    crlf.linesep = r"\r\n"
    pinned = IOBase(root / "crlf.log")
    pinned.overwrite_records([{"body": "first"}], options=crlf)
    assert pinned.read_bytes() == b"first\r\n"

    # A body holding the terminator would read back as two records.
    with pytest.raises(ValueError, match="without its record terminator"):
        target.append_records([{"body": "bad\nline"}])
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase, TextOptions } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const target = new IOBase(path.join(root, 'out.log'))

    // Each body is one line with its terminator; an append adds lines after the last.
    target.overwriteRecords([{ body: 'one' }, { body: 'two' }])
    target.appendRecords([{ body: 'three' }])
    assert.equal(target.readBytes().toString(), 'one\ntwo\nthree\n')

    // A pinned terminator is the one written.
    const crlf = new TextOptions()
    crlf.linesep = '\\r\\n'
    const pinned = new IOBase(path.join(root, 'crlf.log'))
    pinned.overwriteRecords([{ body: 'first' }], crlf)
    assert.equal(pinned.readBytes().toString(), 'first\r\n')

    // A body holding the terminator would read back as two records.
    assert.throws(() => target.appendRecords([{ body: 'bad\nline' }]), /without its record terminator/)

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Performance

### Records, lines and batches

One release run of the `text` Criterion target on one Linux x86_64 container - Intel Xeon @ 2.80 GHz, 4 cores, 15 GiB; rustc 1.97.0, release profile (thin LTO, one codegen unit) - medians of 100 samples, on 2026-10-02. The object is 10,000 lines `[INFO] id=N message N` in a `.log` buffer; where a row header is stated it is `^\[(?<level>[A-Z]+)\] id=(?<id>\d+)`, `level` read as text and `id` as an integer.

| 10,000 lines, 271 KiB | median | lines/s |
| --- | ---: | ---: |
| `regex` crate: split the lines and match the row header, building nothing (baseline) | 1.591 ms | 6.29M lines/s |
| `read_arrow_reader`, no row header | 37.79 ms | 265k lines/s |
| `read_arrow_reader`, the two captures typed | 44.67 ms | 224k lines/s |
| `read_text_lines`, no row header: the lines alone, no Arrow | 4.181 ms | 2.39M lines/s |
| `read_text_lines`, the row header taken off | 6.760 ms | 1.48M lines/s |
| `into_arrow_batch` of the 10,000 decoded lines | 40.10 ms | 249k lines/s |
| `from_arrow_batch`, the batch back to lines | 25.43 ms | 393k lines/s |

The baseline finds the lines and matches the header and builds nothing, so the gap to a record read is the record itself: the lines decode in 4.181 ms, and laying them into a batch - the fifteen element and event columns of every line beside its body - is the rest. The row header costs about a quarter of a microsecond a line. Pinning `linesep` to `\n` or `\r\n` reads at the flexible scan's pace (4.126 ms and 4.275 ms against 4.056 ms), taking a `^(?<tag>\d+)=` header off 10,000 FIX-shaped lines costs 1.740 ms over reading them bare, and `from_arrow_reader` answers its first line in 36.22 us, before the rest of the batch is read.

| `read_arrow_reader` under the row header | physical lines | `framing` |
| --- | ---: | ---: |
| 10,000 one-line records, 271 KiB | 43.67 ms | 44.60 ms |
| 4,000 records of three lines each, 275 KiB | 50.30 ms | 19.12 ms |
| one 2 MiB body over a 4 KiB `max_record_byte_size`, then one line | 4.449 ms | 4.148 ms |

Framing costs nothing on one-line records and saves the rows a chain would otherwise split into; a record past `max_record_byte_size` is drained, not held, at 450 to 480 MiB/s.

```bash
cargo bench -p yggdryl --bench text -- '^text_(records|record_framing|lines|batch)/'
```
