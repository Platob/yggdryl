---
name: yggdryl-documents
description: Parses and writes JSON, JSON Lines, YAML (multi-document), TOML and XML documents as yggdryl Scalar values or native objects, typed and validated by an optional Field, bounded by limits, with opt-in {{ }} and environment-variable placeholders. Use when loading or validating a config file, calling json/yaml/toml/xml loads, dumps, loads_all / loadsAll, dump_all / dumpAll, load_all / loadAll, from_json_scalar / into_json_scalar / from_json_scalar_with_field (and the yaml/toml/xml twins), json::from_utf8 / from_reader / into_writer, decoding into a dataclass with cls=, typing decimals/dates with field=, mapping XML attributes (@name) and #text, inferring a document's format, or read_scalar / write_scalar on a handle. Covers Rust, Python and Node.js.
---

# Documents

Four structured text codecs - JSON (and JSON Lines), YAML, TOML, XML - over
the one `Scalar`. A document decodes to a `Scalar` (Rust) or to natural host
values (Python, JavaScript; `cls=Scalar` / `{ scalar: true }` for the core
value), and a value encodes to UTF-8 bytes in the format's ordinary shapes: no
envelopes or private wire forms (YAML's standard `!!binary` for bytes is the
one tag written), and anything a format cannot spell is refused rather than
approximated.

Hold one model: **without a field a document proves only its own types; a
field types it.** JSON numbers, YAML scalars, TOML's native dates and XML's
all-text leaves are what the text proves. A declared `Field` (or, in Python, a
dataclass `cls`) reads that natural value once under its datatype - decimal at
its scale, date at its unit, integer narrowed - orders a record into the
field's columns, and refuses what does not fit with a located error.

## Choose the door

| Task | Rust | Python | JavaScript |
| --- | --- | --- | --- |
| decode one document | `from_json_scalar(input)?`, `json::from_utf8(s)?`, `json::from_bytes(b)?` | `json.loads(source)` | `json.loads(content)` |
| decode to the exact core value | (always a `Scalar`) | `json.loads(source, cls=Scalar)` | `json.loads(content, { scalar: true })` |
| encode one document | `into_json_scalar(&v)?`, `json::into_utf8(&v)?`, `json::into_bytes(&v)?` | `json.dumps(v)` -> `bytes`; `json.dump(v, utf8=True)` -> `str` | `json.dumps(v)` -> `Buffer` |
| type by a field | `from_json_scalar_with_field(input, &field)?`, `json::from_utf8_with_field(s, &field)?` | `json.loads(source, field=field)` | `json.loads(content, { field })` |
| decode into a class | - | `json.loads(source, cls=Dataclass)` (`@scalar` dataclass) | - |
| read a file | `json::from_reader(File::open(p)?)?` | `json.loads(pathlib.Path(p))`, `json.loads(open(p, "rb"))` | `json.load(pathToFileURL(p))`, `json.load(fd)` |
| write a file or stream | `json::into_writer(&v, File::create(p)?)?` | `json.dump(v, "out.json")`, `json.dump(v, stream)` | `json.dump(v, 'out.json')`, `await json.dump(v, writable)` |
| async byte source | - | - | `await json.load(readable)`, `json.loadStream(readable)` |
| JSON Lines / YAML stream, whole | `json::from_lines_utf8(s)?`, `yaml::from_utf8_all(s)?`, `into_utf8_all(&vs)?` | `json.loads_all(src)`, `yaml.loads_all(src)`, `dumps_all(vs)` | `json.loadsAll(src)`, `yaml.loadsAll(src)`, `dumpAll(vs)` |
| JSON Lines / YAML stream, lazy | `json::from_lines_reader_iter(&mut r)`, `yaml::from_reader_iter(&mut r)`, `json::into_writer_all(vs, w)?` (no cap) | `json.load_all(path_or_reader)`, `dump_all(vs, dest)` (at most 1,024 values per call) | `for await (const d of json.loadAll(readable))`, `dumpAll(vs, writable)` (at most 1,024 values per call) |
| lazy read past 1,024 documents / 64 MiB | `json::from_lines_reader_iter_with_limits(&mut r, Limits::new(..))`, `yaml::from_reader_iter_with_limits` | `json.load_all(src, max_documents=..., max_input_bytes=...)` | `json.loadAll(readable, { maxDocuments, maxInputBytes })` |
| field-typed row back to a named record | `field.into_natural_value(row)?` | `loads(src, field=f)` (no `cls=Scalar`) | `loads(src, { field })` (no `scalar: true`) |
| whitespace-separated JSON values | `json::from_utf8_all(s)?` | - | - |
| XML attribute / own text keys | `xml::ATTRIBUTE_PREFIX` (`@`), `xml::TEXT_KEY` (`#text`) | `"@id"`, `"#text"` | `'@id'`, `'#text'` |
| check a value is writable before writing | `toml::validate_for_write(&v)?`, `xml::validate_for_write(&v)?` | - (the `dumps` refusal) | - (the `dumps` refusal) |
| limits | `json::from_utf8_with_limits(s, Limits::new(depth, bytes, nodes, docs))?` | `max_depth=`, `max_input_bytes=`, `max_nodes=`, `max_documents=` | `{ maxDepth, maxInputBytes, maxNodes, maxDocuments }` |
| `{{ }}` placeholders | `text::from_utf8_with(s, Format::Yaml, &Loading::new().with_placeholders(p))?` | `yaml.loads(s, placeholders={...}, environment=False)` | `yaml.loads(s, { placeholders, environment: false })` |
| indentation | `json::into_utf8_with_formatting(&v, Formatting::indented(2))?`, `Formatting::compact()` | `dumps(v, indent=2)`, `indent="\t"`, `indent=None` | `dumps(v, { indent: 2 })`, `'\t'`, `null` |
| infer the format | `text::from_bytes_inferred(b)?` -> `(Format, Scalar)` | `yggdryl.text.codec.from_io(src)`, `codec.into_io(v, dest)` | `codec.from(src)`, `codec.into(v, dest)` |
| document on a handle (media type picks codec and gzip/zstd) | `handle.read_scalar(Some(&field))?`, `write_scalar(&v)?` | `IOBase(p).read_scalar(field)`, `write_scalar(v)` | `handle.readScalar(field)`, `writeScalar(v)` |

## Rules for fast, correct use

1. **A string source is content, never a path.** Rust `from_*_scalar`,
   Python `loads`, JavaScript `loads`/`load` parse a string as the document.
   Name a file with `pathlib.Path`, a `file:` URL or descriptor, or a Rust
   `Read`er. Output is the opposite: a string destination of `dump` is a path.
2. **Declare the field once, at the boundary.** Parse the `Field` once and
   pass the object; it types decimals, dates, timestamps and exact widths in
   the same pass as the parse. Re-casting natural values afterwards (or
   through pyarrow/Arrow JS) repeats work and loses the value rules.
3. **Stream multi-document input.** `load_all` / `loadAll(readable)` /
   `from_*_reader_iter` hold one document at a time; `loads_all` / `loadsAll`
   / `from_*_all` materialize the list. JSON Lines and YAML are the only
   multi-document formats. The limits cover the whole stream, not each
   document: by default a lazy read stops after 1,024 documents or 64 MiB in
   total. For a large trusted stream raise `max_documents` / `max_input_bytes`
   (`maxDocuments` / `maxInputBytes`; Rust
   `json::from_lines_reader_iter_with_limits(&mut r, Limits::new(128, usize::MAX, 1_000_000, usize::MAX))`).
   For bulk rows, use `read_arrow` from `yggdryl-records`.
4. **Bound untrusted input.** Defaults are depth 128 (48 in JavaScript,
   which is also its ceiling for reads and writes), 64 MiB, 1,000,000 nodes,
   1,024 documents; lower them for untrusted payloads. YAML alias
   expansion counts nodes, so a billion-laughs document fails at the limit.
   Parser hard ceilings: JSON/YAML/XML depth 384, TOML 64, YAML flow 255,
   JavaScript `maxDepth` 1..48.
5. **Placeholders are opt-in, the environment doubly so.** Substitution runs
   only with `placeholders` (or `environment`); environment variables are read
   only with `environment=True` / `with_environment(true)`. YAML, TOML and XML
   only - JSON refuses them. Substitution walks the parsed value, so byte
   positions in errors stay exact; quote a placeholder in YAML.
6. **Output is deterministic.** Records are written with sorted keys, one
   natural shape per value; the encoder never closes a caller's stream.
7. **XML is text until typed.** Every leaf decodes as text; a repeated element
   is a sequence; one occurrence is a single value unless a field (or a
   dataclass) declares a sequence, which then reads it as a one-item list and
   an absent element as the empty list.
8. **Inference order is fixed.** Explicit format, then path suffix, then
   content: JSON, XML (well-formed, opening with `<`), TOML (complete,
   non-empty), YAML. JSON Lines is never inferred from content.
9. **A handle's document is one call.** `read_scalar` / `write_scalar` pick
   the codec and any outer gzip/zlib/zstd from the media type (`.json.gz`),
   on any backend - no manual decompress-then-parse.
10. **Errors are located.** A syntax error or a breached limit names the
    format and the byte (`invalid json data at byte 8: trailing comma`); a
    field refusal names the path from the root field (`invalid record value
    at $.cfg.port: expected int16, got u64`); a Python dataclass mismatch is a
    `TypeError` naming `Class.field`. Fix the input it names; `errors="default"`
    (Python `cls=` only) falls back to a field's declared default instead.
11. **Build classes only when you need them.** Reconstructing a dataclass
    costs far more than the parse (CPython JSON on the docs' fixture: 19.5 us
    to decode bytes, 340 us to decode into a field class). Stay with natural
    values or `Scalar` on hot paths.
12. **Writers emit natural shapes only.** Exact decimals are written as
    strings. Binary is base64 text in JSON, TOML and XML, and reads back as
    that text unless a `binary` field types it; YAML writes binary as a
    `!!binary` scalar and reads it back as bytes. A TOML
    datetime as a TOML datetime, an XML leaf as text; a value with no
    spelling in the format (a TOML root that is not a record, a nested
    sequence in XML) is refused rather than approximated.
13. **Async sources in JavaScript.** `load(readable)` buffers one bounded
    document because Node's async reader cannot feed the synchronous parser;
    `loadAll(readable)` / `loadAllStream` stay incremental and honour
    backpressure, each complete document still parsed by the native codec.
14. **Each format keeps its own limits.** YAML tags are annotations and are
    ignored; anchors and aliases expand under the node budget. TOML follows
    its native root table, `i64` integers, date/time types and single
    document. XML skips comments, processing instructions and the DOCTYPE,
    keeps name prefixes as spelled, and refuses an entity a declaration would
    have defined; on a handle, a non-UTF-8 charset is read from the XML
    declaration when neither the media type nor a byte-order mark states one.

## Pitfalls

- `json.loads("config.json")` / `json.load('config.json')` /
  `from_json_scalar("config.json")` - parses the text `config.json` (and
  fails). Pass `pathlib.Path("config.json")`, `pathToFileURL(...)`, or open
  the file.
- Python `json.load(...)` - does not exist; `loads` takes paths and readers,
  `load_all` streams.
- Expecting `str` from Python `dumps` / JavaScript `dumps` - they return
  `bytes` / `Buffer`.
- Expecting `Decimal`, `date` or `int64` from an untyped document - JSON and
  YAML integers come back as plain ints (`u64`/`i64` in Rust), quoted decimals
  and JSON/YAML dates as strings. Pass `field=` or a dataclass `cls=`.
- `toml.dumps([...])`, `toml.dumps_all`, `xml.dumps({"a": 1, "b": 2})` -
  TOML needs a record root and one document; XML needs exactly one root entry.
- Unquoted YAML placeholder `port: {{ PORT }}` - that is a flow mapping, not a
  string. Write `port: "{{ PORT }}"`.
- `placeholders=` on `json.loads` - a `TypeError` in Python, a refusal in
  JavaScript and Rust; JSON is interchange, template with YAML or TOML.
- Reading rows from a `.jsonl`/`.yaml`/`.xml` handle with
  `read_arrow_reader` / `*_records` - not a record encoding; use these codecs,
  or `read_arrow` / `write_arrow` from `yggdryl-records`.
- Expecting a `set` or `uuid.UUID` back: a document carries shapes, never
  class names; a set reads as a list, a UUID as text. Rebuild with `cls=`.
- Rust: calling `get_key_str` on a field-typed record - with a field the
  answer is an ordered `Scalar::Serie` row; read cells with `get(i)` in the
  field's column order.
- Parsing, then casting in pyarrow/Arrow JS to recover types - one `field=`
  on the parse does it under the crate's value rules, once.
- Hand-rolling gzip around `json.dumps` for a `.json.gz` file - name the
  handle `trade.json.gz` and call `write_scalar`.
- Writing JSON Lines with `json.dumps(rows)` - that is one JSON array; use
  `dumps_all` / `dumpAll` / `json::into_utf8_all`.
- More than 1,024 values through Python `dumps_all` / `dump_all` or
  JavaScript `dumpAll` - refused, and no option lifts the cap (`maxDocuments`
  is ignored on write). Write chunks of at most 1,024 to one open stream, or
  write records with `write_arrow` (`yggdryl-records`). Rust
  `json::into_writer_all` has no cap.
- Writing back a field-typed row (`*_with_field`, `read_scalar(Some(&field))`,
  `cls=Scalar` / `{ scalar: true }` with a field) - it emits a positional
  array such as `[5]`. In Rust restore the names first with
  `into_json_scalar(&field.into_natural_value(row)?)`; in Python and
  JavaScript dump the natural value (no `cls=Scalar` / `scalar: true`).

## Language references

- `references/rust.md` - read for Rust: module functions, `_with_field` / `_with_limits` / `_all` / `_iter` forms, `text::Loading`, `Formatting`.
- `references/python.md` - read for Python: `loads`/`dumps` keywords, `cls=` dataclasses, `load_all`, `yggdryl.text.codec`.
- `references/javascript.md` - read for Node.js: option objects, `Buffer`, async `load`/`loadAll`, `codec`.

## Deeper

- JSON: https://platob.github.io/yggdryl/media/#json
- YAML (and placeholder cost): https://platob.github.io/yggdryl/media/#yaml
- TOML: https://platob.github.io/yggdryl/media/#toml
- XML (mapping, record medium): https://platob.github.io/yggdryl/media/#xml
- Structured values on a handle: https://platob.github.io/yggdryl/holder/#structured-values
- Values and fields: https://platob.github.io/yggdryl/types/scalar/, https://platob.github.io/yggdryl/types/field/
- Sibling skills: `yggdryl-types` (fields, dataclasses, `Scalar`), `yggdryl-storage` (handles, `read_scalar`, codings, charsets), `yggdryl-records` (rows in Arrow IPC, Parquet, Avro, text, Iceberg; `write_arrow` for document rows).
