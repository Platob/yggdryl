# yggdryl-documents in JavaScript

`const { json, yaml, toml, xml, codec } = require('yggdryl')` - frozen namespaces with `loads`/`load`/`dumps`/`dump` and the stream forms; `json` and `yaml` add `loadsAll`/`loadAll`/`dumpAll`. Options travel in one trailing object: `{ field, scalar, maxDepth, maxInputBytes, maxNodes, maxDocuments, indent }`, plus `{ placeholders, environment }` for YAML, TOML and XML.

## Parse and write one document

`loads` answers plain JavaScript values; `{ scalar: true }` answers the exact native `Scalar`. `dumps` answers a UTF-8 `Buffer`, records written with sorted keys.

```javascript
const assert = require('node:assert/strict')
const { Scalar, json } = require('yggdryl')

const natural = json.loads('{"symbol":"AAPL","quantity":100}')
assert.deepEqual(natural, { quantity: 100, symbol: 'AAPL' })

const value = json.loads(Buffer.from('{"symbol":"AAPL","quantity":100}'), { scalar: true })
assert.ok(value instanceof Scalar)
assert.equal(value.kind, 'struct')
assert.deepEqual(value.asJs(), natural)

const encoded = json.dumps(natural)
assert.ok(Buffer.isBuffer(encoded))
assert.equal(encoded.toString(), '{"quantity":100,"symbol":"AAPL"}')
```

## Type the document with a field

`{ field }` types natural text - decimals, dates, exact widths - and validates it. A value with no JavaScript spelling (a decimal, a `date32`) stays a native `Scalar`; timestamps become `Date`; `{ field, scalar: true }` answers the ordered row.

```javascript
const assert = require('node:assert/strict')
const { Field, Scalar, json, yaml } = require('yggdryl')

const field = new Field(
  'trade',
  'struct<px: decimal(10,2) not null, day: date32 not null, n: int8 not null, at: timestamp(ms, UTC)>',
  false,
)
const row = json.loads('{"n": 7, "day": "2024-01-02", "px": "12.50", "at": "2024-01-02T03:04:05Z"}', { field })
assert.ok(row.px instanceof Scalar)
assert.equal(row.px.kind, 'd64')
assert.equal(row.day.kind, 'date32')
assert.equal(row.n, 7)
assert.ok(row.at instanceof Date)

const exact = json.loads('{"n": 7, "day": "2024-01-02", "px": "12.50"}', {
  field: new Field('trade', 'struct<px: decimal(10,2) not null, day: date32 not null, n: int8 not null>', false),
  scalar: true,
})
assert.equal(exact.kind, 'serie') // an ordered row in the field's column order

// The same field reads every format the same way.
const amount = new Field('amount', 'decimal(10,2)', false)
assert.ok(yaml.loads("'12.50'\n", { field: amount }).equals(json.loads('"12.50"', { field: amount })))
```

## Read from a file and write to a file or stream

A string **source** is always document content; a location is a `file:` URL or a file descriptor. A string **destination** of `dump` is a path. An async iterable source or a writable destination returns a `Promise`; caller-owned streams are never closed.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { Readable, Writable } = require('node:stream')
const { pathToFileURL } = require('node:url')
const { json } = require('yggdryl')

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-'))
const file = path.join(root, 'trade.json')
json.dump({ id: 7 }, file)
assert.equal(fs.readFileSync(file, 'utf8'), '{"id":7}')

assert.deepEqual(json.load(pathToFileURL(file)), { id: 7 })
const fd = fs.openSync(file, 'r')
assert.deepEqual(json.load(fd), { id: 7 })
fs.closeSync(fd)

// A string naming the file is parsed as JSON text, and that text is no document.
assert.throws(() => json.loads(file))

;(async () => {
  assert.deepEqual(await json.load(Readable.from(['{"id":', '42}'])), { id: 42 })
  const chunks = []
  await json.dump({ id: 42 }, new Writable({
    write(chunk, _encoding, done) {
      chunks.push(chunk)
      done()
    },
  }))
  assert.equal(Buffer.concat(chunks).toString(), '{"id":42}')
  fs.rmSync(root, { recursive: true, force: true })
})().catch((error) => {
  console.error(error)
  process.exit(1)
})
```

## JSON Lines and YAML document streams

`dumpAll`/`loadsAll` handle whole streams (JSON Lines for `json`, `---` documents for `yaml`); `loadAll` over an async byte source is a lazy async iterator that holds one document at a time.

```javascript
const assert = require('node:assert/strict')
const { Readable } = require('node:stream')
const { json, yaml } = require('yggdryl')

assert.equal(json.dumpAll([{ id: 1 }, { id: 2 }]).toString(), '{"id":1}\n{"id":2}\n')
assert.deepEqual(json.loadsAll('{"id":1}\n{"id":2}\n'), [{ id: 1 }, { id: 2 }])

assert.equal(yaml.dumpAll([{ id: 1 }, { id: 2 }]).toString(), 'id: 1\n---\nid: 2\n')
assert.deepEqual(yaml.loadsAll('id: 1\n---\nid: 2\n'), [{ id: 1 }, { id: 2 }])

;(async () => {
  const ids = []
  for await (const row of json.loadAll(Readable.from(['{"id":1}\n{"id"', ':2}\n{"id":3}\n']))) {
    ids.push(row.id)
  }
  assert.deepEqual(ids, [1, 2, 3])
})().catch((error) => {
  console.error(error)
  process.exit(1)
})
```

## TOML: one record per document

TOML's root is a table and it has exactly one document: `toml` has no `loadsAll`/`dumpAll`, and a root that is not a record is refused.

```javascript
const assert = require('node:assert/strict')
const { toml } = require('yggdryl')

const value = toml.loads('title = "yggdryl"\ncount = 3\n\n[owner]\nname = "Ada"\n')
assert.deepEqual(value, { count: 3, owner: { name: 'Ada' }, title: 'yggdryl' })
assert.deepEqual(toml.loads(toml.dumps(value)), value)

assert.throws(() => toml.dumps([1, 2]), /record/)
assert.equal(toml.loadsAll, undefined)
```

## XML: attributes, text, repeated elements

The document is the object naming its root element: `@name` is an attribute, `#text` an element's own text beside attributes or children, a repeated element an array, `<a/>` is `null` and `<a></a>` is `''`, every leaf text. A `field` types the root element's value.

```javascript
const assert = require('node:assert/strict')
const { Field, xml } = require('yggdryl')

const source = '<order id="7"><symbol>AAPL</symbol><leg>1</leg><leg>2</leg><note/></order>'
const natural = xml.loads(source)
assert.deepEqual(natural, { order: { '@id': '7', symbol: 'AAPL', leg: ['1', '2'], note: null } })
assert.equal(
  xml.dumps(natural).toString(),
  '<order id="7"><leg>1</leg><leg>2</leg><note/><symbol>AAPL</symbol></order>',
)
assert.deepEqual(xml.loads('<a x="1">hi<b></b></a>'), { a: { '#text': 'hi', '@x': '1', b: '' } })

const field = new Field(
  'order',
  'struct<@id: int32 not null, symbol: utf8 not null, leg: serie<int32> not null, note: utf8>',
  false,
)
assert.deepEqual(xml.loads(source, { field }), { '@id': 7, symbol: 'AAPL', leg: [1, 2], note: null })

// One repeated element read once is still a one-item array under the field.
assert.deepEqual(xml.loads('<order id="1"><symbol>X</symbol><leg>5</leg></order>', { field }).leg, [5])
```

## Bound untrusted input

`maxDepth` (1..48 in JavaScript), `maxInputBytes`, `maxNodes` and `maxDocuments` bound one decode; YAML alias expansion counts against `maxNodes`. A breach throws, naming the byte.

```javascript
const assert = require('node:assert/strict')
const { json } = require('yggdryl')

assert.throws(() => json.loads('[[[1]]]', { maxDepth: 2 }), /depth/)
assert.throws(() => json.loads('[1,2,3]', { maxNodes: 2 }), /node limit/)
assert.throws(() => json.loads('"abcdef"', { maxInputBytes: 3 }), /byte limit/)
assert.throws(() => json.loadsAll('1\n2\n3\n', { maxDocuments: 2 }), /document limit/)

// Syntax errors carry the byte position too.
assert.throws(() => json.loads('{"a": 1,}'), /byte 8/)
```

## Resolve `{{ }}` placeholders in configuration

YAML, TOML and XML take `{ placeholders, environment }`; both off by default. A value that is exactly one placeholder takes the variable's type, an embedded one stays text, `| default(...)` supplies a fallback, and an unresolved name throws. JSON refuses the pair by name.

```javascript
const assert = require('node:assert/strict')
const { json, yaml } = require('yggdryl')

const document = 'port: "{{ PORT }}"\npath: "{{ ROOT }}/logs"\nretries: "{{ RETRIES | default(3) }}"\n'
const value = yaml.loads(document, { placeholders: { PORT: 8080, ROOT: '/var' } })
assert.deepEqual(value, { path: '/var/logs', port: 8080, retries: 3 })

// Off unless asked: the text is left as written.
assert.equal(yaml.loads(document).port, '{{ PORT }}')

assert.throws(() => yaml.loads('a: "{{ MISSING }}"\n', { placeholders: {} }), /MISSING/)
assert.throws(() => json.loads('{"a": "{{ PORT }}"}', { placeholders: { PORT: 1 } }), /not a json one/)
```

## Control the output layout

`{ indent }` on `dumps`/`dump`: omitted is the format's default (compact JSON, block YAML), a number is spaces per level, `'\t'` tabs, `null` no layout.

```javascript
const assert = require('node:assert/strict')
const { json, yaml } = require('yggdryl')

const value = { a: { b: [1, 2] } }
assert.equal(json.dumps(value).toString(), '{"a":{"b":[1,2]}}')
assert.ok(json.dumps(value, { indent: 2 }).toString().startsWith('{\n  "a": {\n    "b"'))
assert.ok(json.dumps(value, { indent: '\t' }).toString().startsWith('{\n\t"a"'))
assert.equal(yaml.dumps(value, { indent: null }).toString(), '{a: {b: [1, 2]}}\n')
```

## Detect the format of unknown content

`codec.from` infers from an explicit `format`, then a path suffix, then the content: JSON, XML (well-formed and opening with `<`), TOML (complete and non-empty), YAML. JSON Lines is never inferred from content; `codec.into` writes by the destination's suffix.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { pathToFileURL } = require('node:url')
const { codec } = require('yggdryl')

assert.deepEqual(codec.from('title = "x"\n'), { title: 'x' })
assert.deepEqual(codec.from(Buffer.from('<a>1</a>')), { a: '1' })
assert.deepEqual(codec.from('a: 1\n'), { a: 1 })
assert.deepEqual(codec.from('{"a":1}\n{"a":2}\n', { format: 'jsonl' }), [{ a: 1 }, { a: 2 }])

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-'))
const target = path.join(root, 'config.yaml')
codec.into({ a: 1 }, target)
assert.equal(fs.readFileSync(target, 'utf8'), 'a: 1\n')
assert.deepEqual(codec.from(pathToFileURL(target)), { a: 1 })
fs.rmSync(root, { recursive: true, force: true })
```

## Read or write the document a handle holds

`IOBase.readScalar(field)` / `writeScalar(value)` pick the codec and any outer gzip, zlib or zstd from the handle's media type, on any backend. Handles and backends: `yggdryl-storage`.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { IOBase, Scalar } = require('yggdryl')

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-'))
const handle = new IOBase(path.join(root, 'trade.json.gz'))
handle.writeScalar({ quantity: 2, symbol: 'AAPL' })

const field = 'trade: struct<quantity: int32 not null, symbol: utf8 not null> not null'
assert.deepEqual(handle.readScalar(field), { quantity: 2, symbol: 'AAPL' })
const value = handle.readScalar({ field, scalar: true })
assert.ok(value instanceof Scalar)
assert.equal(value.kind, 'serie')
fs.rmSync(root, { recursive: true, force: true })
```

## Gotchas in JavaScript

- `json.load('trades.json')` parses the text `trades.json`; a file is `pathToFileURL(p)` or a descriptor.
- `dumps`/`dump` answer a `Buffer`; `.toString()` for text.
- `json.load(readable)` buffers one bounded document because Node's async reader cannot feed a synchronous parser; `loadAll(readable)` stays incremental.
- Decimals and `date32` values under a field stay native `Scalar`s (JavaScript has no decimal); read them with `.asJs()`, `String(...)` or `{ scalar: true }`.
- Integers above `Number.MAX_SAFE_INTEGER` arrive as `bigint`; pass `bigint` in for exact 64-bit values.
- `environment: true` reads `process.env`; never turn it on for documents that are later dumped or logged.
