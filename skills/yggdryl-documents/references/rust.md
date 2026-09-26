# yggdryl-documents in Rust

The four codecs are modules - `yggdryl::{json, yaml, toml, xml}` - plus the inferring entry points re-exported at the crate root (`from_json_scalar`, `from_json_scalar_with_field`, `into_json_scalar` and the YAML/TOML/XML twins). Placeholders, limits, formatting and format inference live in `yggdryl::text`. No feature flag is needed.

## Parse and write one document

`from_<fmt>_scalar` takes any byte-like input (`&str`, `&[u8]`, `Vec<u8>`) as content and answers a `Scalar`; `into_<fmt>_scalar` writes it back. The explicit forms (`json::from_utf8`, `from_bytes`, `into_utf8`, `into_bytes`) are the same codec.

```rust
use yggdryl::json;
use yggdryl::{from_json_scalar, into_json_scalar, Scalar};

let value = from_json_scalar(r#"{"symbol":"AAPL","quantity":100}"#)?;
assert_eq!(value.get_key_str("symbol").and_then(Scalar::as_str), Some("AAPL"));

// A record is written with its keys sorted: output is deterministic.
let encoded = into_json_scalar(&value)?;
assert_eq!(encoded, r#"{"quantity":100,"symbol":"AAPL"}"#);
assert_eq!(json::from_bytes(encoded.as_bytes())?, value);
assert_eq!(json::into_utf8(&value)?, encoded);
```

## Type the document with a field

`*_with_field` types natural text (decimals, dates, exact widths), orders a record into the field's column order and validates it; a record answers an ordered `Scalar::Serie` row.

```rust
use yggdryl::{from_json_scalar_with_field, from_yaml_scalar_with_field, into_json_scalar, DataType, Field, Scalar, StructType};

let field = DataType::from(StructType::from_fields([
    DataType::decimal128(10, 2)?.required_field("px"),
    DataType::date32().required_field("day"),
    DataType::Int8.required_field("n"),
])?)
.required_field("trade");

let row = from_json_scalar_with_field(r#"{"n":7,"day":"2024-01-02","px":"12.50"}"#, &field)?;
assert_eq!(row.get(0).as_deref(), Some(&Scalar::d128(1250, 2)));
assert_eq!(row.get(1).expect("a day").dtype()?, DataType::date32());
assert_eq!(row.get(2).as_deref(), Some(&Scalar::from(7_i8)));

// The same field reads every format the same way.
let amount = Field::new("amount", DataType::decimal128(10, 2)?, false);
assert_eq!(from_yaml_scalar_with_field("'12.50'\n", &amount)?, Scalar::d128(1250, 2));

// A value the field cannot hold is refused with its location.
let refused = from_json_scalar_with_field(r#"{"n":700,"day":"2024-01-02","px":"1"}"#, &field).unwrap_err();
assert!(refused.to_string().contains("$.trade.n"), "{refused}");

// The row is positional, so it writes back as an array; restore the names first.
let cfg = DataType::from(StructType::from_fields([DataType::Int16.required_field("port")])?).required_field("cfg");
let typed = from_json_scalar_with_field(r#"{"port":5}"#, &cfg)?;
assert_eq!(into_json_scalar(&typed)?, "[5]");
assert_eq!(into_json_scalar(&cfg.into_natural_value(typed)?)?, r#"{"port":5}"#);
```

## Read from a file and write to a writer

`from_reader` takes any `std::io::Read`, `into_writer` any `std::io::Write`; neither closes what it is given. A path is never inferred from a string.

```rust
use std::fs::File;
use std::io::BufReader;

use yggdryl::{toml, Scalar};

let path = std::env::temp_dir().join("yggdryl-skill-documents-config.toml");
let value = toml::from_utf8("title = \"yggdryl\"\n\n[owner]\nname = \"Ada\"\n")?;
toml::into_writer(&value, File::create(&path)?)?;

let read = toml::from_reader(BufReader::new(File::open(&path)?))?;
assert_eq!(read, value);
assert_eq!(
    read.get_key_str("owner").and_then(|owner| owner.get_key_str("name")).and_then(Scalar::as_str),
    Some("Ada"),
);
std::fs::remove_file(&path)?;
```

## JSON Lines and YAML document streams

`json::from_lines_*` is strict newline-delimited JSON; `yaml::from_utf8_all` reads `---`-separated documents; `into_*_all` writes them. `from_*_reader_iter` decodes lazily, one document at a time. Its limits cover the whole stream (by default 1,024 documents or 64 MiB in total); the `_with_limits` form lifts them for a large trusted stream. `into_writer_all` writes any iterator with no document cap.

```rust
use std::io::Cursor;

use yggdryl::text::Limits;
use yggdryl::{json, yaml, Scalar};

let rows = json::from_lines_utf8("{\"id\":1}\n{\"id\":2}\n")?;
assert_eq!(rows.len(), 2);
// Whitespace-separated values, not strictly one per line.
assert_eq!(json::from_utf8_all("{\"id\":1} {\"id\":2}")?, rows);
assert_eq!(json::into_utf8_all(&rows)?, "{\"id\":1}\n{\"id\":2}\n");

let documents = yaml::from_utf8_all("id: 1\n---\nid: 2\n")?;
assert_eq!(documents[1].get_key_str("id"), Some(&Scalar::from(2)));
assert_eq!(yaml::into_utf8_all(&documents)?, "id: 1\n---\nid: 2\n");

// Lazily, from any reader: memory holds one document, not the stream.
let mut source = Cursor::new(b"{\"id\":1}\n{\"id\":2}\n{\"id\":3}\n".to_vec());
let mut ids = Vec::new();
for value in json::from_lines_reader_iter(&mut source) {
    ids.push(value?.get_key_str("id").and_then(Scalar::as_i64).expect("an id"));
}
assert_eq!(ids, [1, 2, 3]);

// The default limits bound the whole stream: 1,024 documents in total.
let lines: String = (0..2000).map(|n| format!("{{\"id\":{n}}}\n")).collect();
let mut source = Cursor::new(lines.as_bytes());
assert!(json::from_lines_reader_iter(&mut source).any(|value| value.is_err()));
let mut source = Cursor::new(lines.as_bytes());
let trusted = Limits::new(128, usize::MAX, 1_000_000, usize::MAX);
assert_eq!(json::from_lines_reader_iter_with_limits(&mut source, trusted).count(), 2000);

// The writer takes any iterator and has no document cap.
let mut sink = Vec::new();
json::into_writer_all((0..2000_i64).map(Scalar::from), &mut sink)?;
assert_eq!(sink.iter().filter(|byte| **byte == b'\n').count(), 2000);
```

## TOML: one record per document

TOML's root is a table and it has one document; anything else is refused when written.

```rust
use yggdryl::{toml, Scalar};
use yggdryl::{from_toml_scalar, into_toml_scalar};

let source = "title = \"yggdryl\"\ncount = 3\n\n[owner]\nname = \"Ada\"\n";
let value = from_toml_scalar(source)?;
assert_eq!(value.get_key_str("count"), Some(&Scalar::from(3)));
assert_eq!(from_toml_scalar(into_toml_scalar(&value)?.as_bytes())?, value);

// A root that is not a record cannot be spelled; check before opening a destination.
let list = Scalar::from_sequence([Scalar::from(1), Scalar::from(2)]);
assert!(toml::validate_for_write(&list).is_err());
assert!(toml::into_utf8(&list).unwrap_err().to_string().contains("record"));
```

## XML: attributes, text, repeated elements

The document is the record naming its root element: `@name` is an attribute, `#text` an element's own text beside attributes or children, a repeated element a sequence, `<a/>` null and `<a></a>` empty text, every leaf text. A field types the root element's value.

```rust
use yggdryl::xml;
use yggdryl::{from_xml_scalar, from_xml_scalar_with_field, into_xml_scalar, Field, Scalar};

let source = "<order id=\"7\"><symbol>AAPL</symbol><leg>1</leg><leg>2</leg><note/></order>";
let value = from_xml_scalar(source)?;
let order = value.get_key_str("order").expect("the root element");
assert_eq!(order.get_key_str(&format!("{}id", xml::ATTRIBUTE_PREFIX)).and_then(Scalar::as_str), Some("7"));
assert_eq!(order.get_key_str("leg").and_then(Scalar::as_sequence).map(<[Scalar]>::len), Some(2));
assert_eq!(order.get_key_str("note"), Some(&Scalar::Null));
assert_eq!(from_xml_scalar(into_xml_scalar(&value)?.as_bytes())?, value);

// One root entry per document: two are refused before anything is written.
let two_roots = Scalar::from_struct([("a", Scalar::from(1)), ("b", Scalar::from(2))])?;
assert!(xml::validate_for_write(&two_roots).is_err());

let field = Field::from_str(
    "order: struct<@id: int32 not null, symbol: utf8 not null, leg: serie<int32> not null, note: utf8> not null",
)?;
assert_eq!(
    from_xml_scalar_with_field(source, &field)?,
    Scalar::from_sequence([
        Scalar::from(7),
        Scalar::from("AAPL"),
        Scalar::from_sequence([Scalar::from(1), Scalar::from(2)]),
        Scalar::Null,
    ]),
);
```

## Bound untrusted input

`Limits::new(max_depth, max_input_bytes, max_nodes, max_documents)` goes through the `*_with_limits` forms; the defaults are 128, 64 MiB, 1,000,000 and 1,024. YAML alias expansion counts against `max_nodes`. A breach is an error naming the byte.

```rust
use yggdryl::text::Limits;
use yggdryl::{json, yaml};

let shallow = Limits::new(2, 1024, 100, 10);
let refused = json::from_utf8_with_limits("[[[1]]]", shallow).unwrap_err();
assert!(refused.to_string().contains("depth"), "{refused}");

let small = Limits::new(64, 8, 100, 10);
assert!(json::from_utf8_with_limits(r#"{"a":"long enough"}"#, small).is_err());

let few = Limits::new(64, 1024, 100, 2);
assert!(yaml::from_utf8_all_with_limits("a: 1\n---\na: 2\n---\na: 3\n", few).is_err());
```

## Resolve `{{ }}` placeholders in configuration

Off unless asked: `Loading::with_placeholders` resolves YAML, TOML and XML string values after parsing. A value that is exactly one placeholder takes the variable's type; an embedded one stays text; an unresolved name is an error; the environment is a second switch (`Placeholders::with_environment`), off by default. JSON refuses placeholders.

```rust
use yggdryl::text::{self, Format, Loading, Placeholders};
use yggdryl::Scalar;

let document = "port: \"{{ PORT }}\"\npath: \"{{ ROOT }}/logs\"\nretries: \"{{ RETRIES | default(3) }}\"\n";
let loading = Loading::new().with_placeholders(
    Placeholders::new()
        .with_variable("PORT", Scalar::from(8080))
        .with_variable("ROOT", Scalar::from("/var")),
);
let value = text::from_utf8_with(document, Format::Yaml, &loading)?;
assert_eq!(value.get_key_str("port"), Some(&Scalar::from(8080)));
assert_eq!(value.get_key_str("path").and_then(Scalar::as_str), Some("/var/logs"));
assert_eq!(value.get_key_str("retries"), Some(&Scalar::from(3)));

// Without the opt-in the text is untouched; a missing name is refused.
let plain = text::from_utf8_with(document, Format::Yaml, &Loading::new())?;
assert_eq!(plain.get_key_str("port").and_then(Scalar::as_str), Some("{{ PORT }}"));
let empty = Loading::new().with_placeholders(Placeholders::new());
assert!(text::from_utf8_with("a: \"{{ MISSING }}\"\n", Format::Yaml, &empty).is_err());
assert!(text::from_utf8_with("{\"a\":\"{{ PORT }}\"}", Format::Json, &loading).is_err());
```

## Control the output layout

`into_*_with_formatting` takes `Formatting::indented(n)` or `Formatting::compact()`; the default is each format's own layout (compact JSON, block YAML).

```rust
use yggdryl::text::Formatting;
use yggdryl::{json, Scalar};

let value = Scalar::from_struct([("a", Scalar::from_sequence([Scalar::from(1), Scalar::from(2)]))])?;
assert_eq!(json::into_utf8(&value)?, r#"{"a":[1,2]}"#);
assert_eq!(json::into_utf8_with_formatting(&value, Formatting::indented(2))?, "{\n  \"a\": [\n    1,\n    2\n  ]\n}");
```

## Detect the format of unknown content

`text::from_bytes_inferred` tries JSON, then XML (well-formed and opening with `<`), then TOML (complete and non-empty), then YAML, and answers the format beside the value. JSON Lines is never inferred from content.

```rust
use yggdryl::text::{self, Format};
use yggdryl::Scalar;

let (format, value) = text::from_bytes_inferred(b"title = \"yggdryl\"\n")?;
assert_eq!(format, Format::Toml);
assert_eq!(value.get_key_str("title").and_then(Scalar::as_str), Some("yggdryl"));

assert_eq!(text::from_bytes_inferred(b"{\"a\":1}")?.0, Format::Json);
assert_eq!(text::from_bytes_inferred(b"<a>1</a>")?.0, Format::Xml);
assert_eq!(text::from_bytes_inferred(b"a: 1\n")?.0, Format::Yaml);
```

## Read or write the document a handle holds

`read_scalar(field)` / `write_scalar(value)` on any `IOBase` pick the codec and any outer gzip, zlib or zstd from the handle's media type. Handles and backends: `yggdryl-storage`.

```rust
use yggdryl::holder::Buffer;
use yggdryl::{Field, IOBase, Scalar, Url};

let mut handle = Buffer::new().with_media_type(Url::from_str("file:///trade.json.gz")?.media_type());
handle.write_scalar(&Scalar::from_struct([
    ("quantity", Scalar::from(2_i64)),
    ("symbol", Scalar::from("AAPL")),
])?)?;

let field = Field::from_str("trade: struct<quantity: int32 not null, symbol: utf8 not null> not null")?;
let row = handle.read_scalar(Some(&field))?;
assert_eq!(row.get(0).as_deref(), Some(&Scalar::from(2_i32)));
assert_eq!(handle.read_scalar(None)?.get_key_str("symbol").and_then(Scalar::as_str), Some("AAPL"));
```

## Gotchas in Rust

- Input to `from_<fmt>_scalar` is always content: a `&str` naming a file is parsed as that text. Open the file and use `from_reader`.
- Without a field a record is a sorted `Scalar::Struct` (`get_key_str`); with one it is an ordered `Scalar::Serie` row (`get(i)`).
- Without a field, JSON and YAML integers decode as `u64` (non-negative) or `i64`, TOML's as `i64`, and quoted decimals and JSON/YAML dates stay text (TOML's native dates are dates); declare the field for exact widths, decimals, dates and timestamps. Scalar equality compares values across widths.
- `json::from_utf8_all` reads whitespace-separated values; `json::from_lines_*` is strict JSON Lines. `into_utf8_all` writes JSON Lines.
- A `_with_field` row writes back as a positional array (`[5]`); `into_json_scalar(&field.into_natural_value(row)?)` restores the names.
- A lazy reader's limits count the whole stream, not each document: 1,024 documents or 64 MiB by default; use `from_lines_reader_iter_with_limits` / `yaml::from_reader_iter_with_limits` for a large trusted stream.
- A field refusal names the path from the root field (`$.cfg.port`).
- Bytes are base64 text in JSON, TOML and XML and read back as text unless a `binary` field types them; YAML writes `!!binary` and reads back bytes.
- TOML and XML are one document each; XML writes one root and refuses a key that is not an XML name or a sequence inside a sequence.
