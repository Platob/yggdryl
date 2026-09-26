# yggdryl-documents in Python

`from yggdryl import json, yaml, toml, xml` - each a module with `dumps`/`dump`/`loads`; JSON and YAML add `dumps_all`/`dump_all`/`loads_all`/`load_all`. There is no `load`: `loads` already takes content, a `pathlib.Path` or a readable file object. `yggdryl.text.codec` holds the format-inferring `from_io`/`into_io`.

## Parse and write one document

`loads` answers natural Python values (`dict`, `list`, `int`, ...); `cls=Scalar` answers the exact core `Scalar`. `dumps` answers UTF-8 **bytes**, records written with sorted keys.

```python
from yggdryl import Scalar, json

natural = json.loads('{"symbol":"AAPL","quantity":100}')
assert natural == {"quantity": 100, "symbol": "AAPL"}

value = json.loads(b'{"symbol":"AAPL","quantity":100}', cls=Scalar)
assert value.kind == "struct"
assert value.as_py() == natural

encoded = json.dumps(natural)
assert encoded == b'{"quantity":100,"symbol":"AAPL"}'
assert json.dump(natural, utf8=True) == '{"quantity":100,"symbol":"AAPL"}'
```

## Type the document with a field or a dataclass

`field=` types natural text - decimals, dates, exact widths - and validates it; `cls=` builds a `@scalar` dataclass. The same field reads all four formats the same way.

```python
import datetime
import decimal

import pytest

from yggdryl import Field, Scalar, json, scalar, yaml

field = Field("trade", "struct<px: decimal(10,2) not null, day: date32 not null, n: int8 not null>", nullable=False)
row = json.loads('{"n": 7, "day": "2024-01-02", "px": "12.50"}', field=field)
assert row == {"px": decimal.Decimal("12.50"), "day": datetime.date(2024, 1, 2), "n": 7}

exact = json.loads('{"n": 7, "day": "2024-01-02", "px": "12.50"}', field=field, cls=Scalar)
assert exact.kind == "serie"  # an ordered row in the field's column order
# That row is positional: dumped back it is an array. Dump the natural value.
cfg = Field("cfg", "struct<port: int16 not null>", nullable=False)
assert json.dumps(json.loads('{"port": 5}', field=cfg, cls=Scalar)) == b"[5]"
assert json.dumps(json.loads('{"port": 5}', field=cfg)) == b'{"port":5}'
with pytest.raises(ValueError, match=r"\$\.cfg\.port"):
    yaml.loads("port: 99999\n", field=cfg)

@scalar(frozen=True)
class Trade:
    px: decimal.Decimal
    day: datetime.date

assert yaml.loads("px: '12.50'\nday: '2024-01-02'\n", cls=Trade) == Trade(decimal.Decimal("12.50"), datetime.date(2024, 1, 2))
assert json.dumps(Trade(decimal.Decimal("1.5"), datetime.date(2024, 1, 2))) == b'{"day":"2024-01-02","px":"1.5"}'

# A mismatch names Class.field; errors="default" falls back to declared defaults.
@scalar
class Quote:
    size: int = 0

with pytest.raises(TypeError, match="Quote.size"):
    json.loads('{"size": "big"}', cls=Quote)
assert json.loads('{"size": "big"}', cls=Quote, errors="default") == Quote(0)
```

## Read from a file and write to a file or stream

A `str` **source** is always document content; pass `pathlib.Path` (or a readable object) for a file. A `str` **destination** of `dump` is a path, because output has no content/path ambiguity. Caller-owned streams are never closed.

```python
import io
import pathlib
import tempfile

import pytest

from yggdryl import json

with tempfile.TemporaryDirectory() as directory:
    path = pathlib.Path(directory) / "trade.json"
    json.dump({"id": 7}, str(path))
    assert path.read_bytes() == b'{"id":7}'

    assert json.loads(path) == {"id": 7}
    with open(path, "rb") as stream:
        assert json.loads(stream) == {"id": 7}

    # A str naming the file is parsed as JSON text, and that text is no document.
    with pytest.raises(ValueError):
        json.loads(str(path))

text = io.StringIO()
json.dump({"id": 7}, text)
assert text.getvalue() == '{"id":7}'
```

## JSON Lines and YAML document streams

`dumps_all`/`loads_all` handle whole streams (JSON Lines for `json`, `---` documents for `yaml`); `load_all` reads a path or file object lazily, one document at a time, and `dump_all` writes one at a time. The limits cover the whole stream: by default `load_all` stops after 1,024 documents or 64 MiB in total, so raise `max_documents` / `max_input_bytes` for a large trusted stream. `dumps_all` / `dump_all` refuse more than 1,024 values per call with no option to lift it; write chunks to one open stream.

```python
import io

import pytest

from yggdryl import json, yaml

assert json.dumps_all([{"id": 1}, {"id": 2}]) == b'{"id":1}\n{"id":2}\n'
assert list(json.loads_all(b'{"id":1}\n{"id":2}\n')) == [{"id": 1}, {"id": 2}]

assert yaml.dumps_all([{"id": 1}, {"id": 2}]) == b"id: 1\n---\nid: 2\n"
assert list(yaml.loads_all("id: 1\n---\nid: 2\n")) == [{"id": 1}, {"id": 2}]

# Lazily from a reader: memory holds one document, not the stream.
source = io.BytesIO(b'{"id":1}\n{"id":2}\n{"id":3}\n')
assert [row["id"] for row in json.load_all(source)] == [1, 2, 3]

sink = io.BytesIO()
json.dump_all(({"id": n} for n in range(3)), sink)
assert sink.getvalue().count(b"\n") == 3

# The default limits bound the whole stream: 1,024 documents in total.
lines = b"".join(b'{"id":%d}\n' % n for n in range(2000))
with pytest.raises(ValueError, match="document limit"):
    list(json.load_all(io.BytesIO(lines)))
assert len(list(json.load_all(io.BytesIO(lines), max_documents=10**9))) == 2000

# The writers cap one call at 1,024 values; chunk into one open stream.
rows = [{"id": n} for n in range(2000)]
with pytest.raises(ValueError, match="1024"):
    json.dumps_all(rows)
sink = io.BytesIO()
for start in range(0, len(rows), 1024):
    json.dump_all(rows[start:start + 1024], sink)
assert sink.getvalue().count(b"\n") == 2000
```

## TOML: one record per document

TOML's root is a table and it has exactly one document: there is no `dumps_all`, and a root that is not a mapping is refused.

```python
import datetime

import pytest

from yggdryl import toml

source = 'title = "yggdryl"\nsince = 2024-01-02\n\n[owner]\nname = "Ada"\n'
value = toml.loads(source)
assert value == {"owner": {"name": "Ada"}, "since": datetime.date(2024, 1, 2), "title": "yggdryl"}
assert toml.loads(toml.dumps(value)) == value

with pytest.raises(ValueError, match="record"):
    toml.dumps([1, 2])
assert not hasattr(toml, "dumps_all")
```

## XML: attributes, text, repeated elements

The document is the mapping naming its root element: `@name` is an attribute, `#text` an element's own text beside attributes or children, a repeated element a list, `<a/>` is `None` and `<a></a>` is `""`, every leaf text. A `field` - or a dataclass `cls` - types the root element's value.

```python
from yggdryl import Field, scalar, xml

source = '<order id="7"><symbol>AAPL</symbol><leg>1</leg><leg>2</leg><note/></order>'
natural = xml.loads(source)
assert natural == {"order": {"@id": "7", "symbol": "AAPL", "leg": ["1", "2"], "note": None}}
assert xml.dumps(natural) == b'<order id="7"><leg>1</leg><leg>2</leg><note/><symbol>AAPL</symbol></order>'
assert xml.loads('<a x="1">hi<b></b></a>') == {"a": {"#text": "hi", "@x": "1", "b": ""}}

field = Field("order", "struct<@id: int32 not null, symbol: utf8 not null, leg: serie<int32> not null, note: utf8>", nullable=False)
assert xml.loads(source, field=field) == {"@id": 7, "symbol": "AAPL", "leg": [1, 2], "note": None}

@scalar(frozen=True)
class Order:
    symbol: str
    leg: list[int]

# One repeated element read once is still a one-item list under the class.
assert xml.loads("<order><symbol>AAPL</symbol><leg>1</leg></order>", cls=Order) == Order("AAPL", [1])
```

## Bound untrusted input

`max_depth`, `max_input_bytes`, `max_nodes` and `max_documents` bound one decode - for `load_all`, the whole stream (defaults 128, 64 MiB, 1,000,000, 1,024); YAML alias expansion counts against `max_nodes`. A breach raises `ValueError` naming the byte.

```python
import pytest

from yggdryl import json

with pytest.raises(ValueError, match="depth"):
    json.loads("[[[1]]]", max_depth=2)
with pytest.raises(ValueError, match="node limit"):
    json.loads("[1,2,3]", max_nodes=2)
with pytest.raises(ValueError, match="byte limit"):
    json.loads('"abcdef"', max_input_bytes=3)
with pytest.raises(ValueError, match="document limit"):
    list(json.loads_all(b"1\n2\n3\n", max_documents=2))

# Syntax errors carry the byte position too.
with pytest.raises(ValueError, match="byte 8"):
    json.loads('{"a": 1,}')
```

## Resolve `{{ }}` placeholders in configuration

YAML, TOML and XML `loads` take `placeholders=` (a mapping) and `environment=` (read the process environment); both off by default. A value that is exactly one placeholder takes the variable's type, an embedded one stays text, `| default(...)` supplies a fallback, and an unresolved name raises. JSON has no placeholder arguments.

```python
import pytest

from yggdryl import json, yaml

document = 'port: "{{ PORT }}"\npath: "{{ ROOT }}/logs"\nretries: "{{ RETRIES | default(3) }}"\n'
value = yaml.loads(document, placeholders={"PORT": 8080, "ROOT": "/var"})
assert value == {"path": "/var/logs", "port": 8080, "retries": 3}

# Off unless asked: the text is left as written.
assert yaml.loads(document)["port"] == "{{ PORT }}"

with pytest.raises(ValueError, match="MISSING"):
    yaml.loads('a: "{{ MISSING }}"\n', placeholders={})
with pytest.raises(TypeError):
    json.loads('{"a": "{{ PORT }}"}', placeholders={"PORT": 1})
```

## Control the output layout

`indent=` on `dumps`/`dump`: omitted is the format's default (compact JSON, block YAML), a number is spaces per level, `"\t"` tabs, `None` no layout.

```python
from yggdryl import json, yaml

value = {"a": {"b": [1, 2]}}
assert json.dumps(value) == b'{"a":{"b":[1,2]}}'
assert json.dumps(value, indent=2).startswith(b'{\n  "a": {\n    "b"')
assert json.dumps(value, indent="\t").startswith(b'{\n\t"a"')
assert yaml.dumps(value, indent=None) == b"{a: {b: [1, 2]}}\n"
```

## Detect the format of unknown content

`yggdryl.text.codec.from_io` infers from an explicit `format=`, then a path suffix, then the content: JSON, XML (well-formed and opening with `<`), TOML (complete and non-empty), YAML. JSON Lines is never inferred from content.

```python
import pathlib
import tempfile

from yggdryl.text import codec

assert codec.from_io('title = "x"\n') == {"title": "x"}
assert codec.from_io(b"<a>1</a>") == {"a": "1"}
assert codec.from_io("a: 1\n") == {"a": 1}
assert list(codec.from_io(b'{"a":1}\n{"a":2}\n', format="jsonl")) == [{"a": 1}, {"a": 2}]

with tempfile.TemporaryDirectory() as directory:
    target = pathlib.Path(directory) / "config.yaml"
    codec.into_io({"a": 1}, target)
    assert target.read_bytes() == b"a: 1\n"
    assert codec.from_io(target) == {"a": 1}
```

## Read or write the document a handle holds

`IOBase.read_scalar(field)` / `write_scalar(value)` pick the codec and any outer gzip, zlib or zstd from the handle's media type, on any backend. Handles and backends: `yggdryl-storage`.

```python
import pathlib
import tempfile

from yggdryl import IOBase, Scalar

handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trade.json.gz")
handle.write_scalar({"quantity": 2, "symbol": "AAPL"})

field = "trade: struct<quantity: int32 not null, symbol: utf8 not null> not null"
assert handle.read_scalar(field) == {"quantity": 2, "symbol": "AAPL"}
assert handle.read_scalar(field, cls=Scalar).kind == "serie"
assert handle.read_scalar() == {"quantity": 2, "symbol": "AAPL"}
```

## Gotchas in Python

- `json.loads("trades.json")` parses the JSON string `"trades.json"` - or fails; it never opens a file. Use `pathlib.Path`.
- `dumps` returns `bytes`; use `dump(value, utf8=True)` or `.decode()` for `str`.
- There is no `json.load`/`yaml.load`; `loads` takes paths and readers. `load_all` is the lazy stream reader.
- JSON and YAML integers without a field come back as Python `int`, quoted decimals and JSON/YAML dates as `str`; declare `field=` or a dataclass for `Decimal`, `date`, `datetime`.
- Bytes are base64 text in JSON, TOML and XML (`json.dumps(b"ab") == b'"YWI="'`) and read back as `str` unless a `binary` field types them; YAML writes `!!binary` and reads back `bytes`.
- `dumps_all` / `dump_all` refuse more than 1,024 values per call and take no `max_documents`; `load_all`'s limits count the whole stream, not each document.
- A field-typed `cls=Scalar` row dumps as a positional array (`b"[5]"`); dump the natural value instead.
- A document carries shapes, never class names: a `set` reads back as a `list`, a `uuid.UUID` as its text; classes are built only through `cls=`.
- `placeholders=` is a YAML/TOML/XML argument; `environment=True` reads `os.environ` - never turn it on for untrusted documents that are later dumped or logged.
