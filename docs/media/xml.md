# XML records

An XML document read and written as rows, and one row at a time by position.

## Contract

| Key | Value |
| --- | --- |
| Owns | `media::xml::Xml`, `XmlOptions`, `RowIndex`, `RowSpan`, and the free functions over [`IOBase`](../holder/iobase/records.md) |
| Rows | the document element holds one element per row; a row's children, attributes and own text are its columns, mapped exactly as [the codec](../text/xml.md) maps them |
| Names | `root` is the document element and `row` the row element; a read discovers both from the document, and a write keeps a stored document's own names unless the options declare others |
| Schema | declared through `options.field`, which projects and casts; with none declared the rows are scanned and answer nullable `utf8` columns, structs for nested elements and lists for repeated ones |
| Position | `read_row_index` reads the byte span of every row, decoding no value; a row is then addressable |
| In place | a replacement of the same byte length is one `pwrite` where the old row was; another length moves only the bytes after it, in bounded chunks |
| Append | rewrites the document element's end tag, not the document |
| Index | restated across positional writes rather than read again, so a run of them costs one scan |
| Coding | a content coding is the handle's business; rows still read and write, but a row has no address until the bytes are the document's own |
| Errors | name the row and the byte offset inside it |
| Bindings | Rust owns the positional surface; Python and JavaScript reach an XML document's rows through the shared [`IOMedia`](../holder/iobase/records.md) methods, and Python names the encoding as `yggdryl.media.Xml` |

## Rows through the shared surface

An XML handle answers the ordinary record methods, so nothing above it knows the
encoding.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};
use yggdryl::holder::Buffer;
use yggdryl::media::xml::Xml;
use yggdryl::{DataType, IOBase, IOMedia, Url};

let field = DataType::from_fields([
    DataType::Int64.required_field("id"),
    DataType::Utf8.nullable_field("symbol"),
])?
.required_field("row");
let arrow = field.clone().into_arrow_schema()?;
let batch = RecordBatch::try_new(
    Arc::clone(&arrow),
    vec![
        Arc::new(Int64Array::from(vec![1, 2])),
        Arc::new(StringArray::from(vec![Some("AAPL"), None])),
    ],
)?;

let handle = Buffer::new().with_media_type(Url::from_str("file:///rows.xml")?.media_type());
let mut media = Xml::new(handle).with_field(field);
let options = media.record_options()?;
media.overwrite_arrow_reader(yggdryl::arrow::batch_reader(arrow, [batch]), &options)?;

assert_eq!(
    String::from_utf8(media.read_all_bytes()?)?,
    "<rows>\n  <row><id>1</id><symbol>AAPL</symbol></row>\n  <row><id>2</id><symbol/></row>\n</rows>"
);
assert_eq!(media.row_size()?, 2);
```

## One row, by position

A document carries no offset table, so the index is read once and every later ask
is an offset. Only the row being asked for is read and parsed.

```rust
use yggdryl::holder::Buffer;
use yggdryl::media::xml::Xml;
use yggdryl::{IOBase, Scalar, Url};

let mut handle = Buffer::new().with_media_type(Url::from_str("file:///rows.xml")?.media_type());
handle.write_all_bytes(
    b"<rows>\n  <row><id>1</id></row>\n  <row><id>2</id></row>\n</rows>",
)?;
let mut media = Xml::new(handle);

assert_eq!(
    media.read_row_scalar(1)?,
    Scalar::from_record([("id", Scalar::from("2"))])?
);

// The replacement is the same length, so it is written exactly where the old
// row was and no byte after it moves.
media.write_row_scalar(1, &Scalar::from_record([("id", Scalar::from("9"))])?)?;
assert_eq!(
    String::from_utf8(media.read_all_bytes()?)?,
    "<rows>\n  <row><id>1</id></row>\n  <row><id>9</id></row>\n</rows>"
);

media.append_row_scalars([Scalar::from_record([("id", Scalar::from("3"))])?])?;
media.remove_row(0)?;
assert_eq!(media.read_row_index()?.len(), 2);
```

## The index

`read_row_index` answers where every row begins and ends, which is what makes a
row addressable at all. An opened handle reads it once and keeps it; a closed one
answers a fresh scan.

```rust
use yggdryl::holder::Buffer;
use yggdryl::media::xml::Xml;
use yggdryl::{IOBase, Url};

let mut handle = Buffer::new().with_media_type(Url::from_str("file:///rows.xml")?.media_type());
handle.write_all_bytes(b"<rows><row><id>1</id></row></rows>")?;
let media = Xml::new(handle);

let index = media.read_row_index()?;
assert_eq!(index.root(), Some("rows"));
assert_eq!(index.row(), Some("row"));
let span = index.get(0).expect("one row");
assert_eq!(
    String::from_utf8(media.read_range_bytes(span.start, span.byte_size() as usize)?)?,
    "<row><id>1</id></row>"
);
```

## Options

`XmlOptions` carries the shared [`RecordOptions`](options.md) settings plus the two
names and the layout that describe the wire.

| Setting | Meaning |
| --- | --- |
| `root` | the document element a write creates; a read uses whatever the document has |
| `row` | the element one row is written as; a read uses the first element under the document element |
| `formatting` | the default puts each row on its own line, an explicit width lays out its children too, and no indent writes the document without layout |
| `field` | the declared schema, which projects the columns and casts the text |
