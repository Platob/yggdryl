# Structured text values

`yggdryl::text` is the value tree every part of the project speaks, plus the four text formats that read and write it.

=== "Rust"

    ```rust
    use yggdryl::Value;
    use yggdryl::text::{Json, TextCodec};

    let quote = Json.loads(r#"{"symbol":"AAPL","price":12.5}"#)?;

    assert_eq!(quote.path("symbol").and_then(Value::as_str), Some("AAPL"));
    assert_eq!(quote.keys(), vec!["symbol", "price"]);
    assert_eq!(Json.dumps(&quote)?, r#"{"symbol":"AAPL","price":12.5}"#);
    ```

=== "Python"

    ```python
    from yggdryl import json

    quote = json.loads('{"symbol":"AAPL","price":12.5}')

    assert quote["symbol"] == "AAPL"
    assert list(quote) == ["symbol", "price"]
    assert json.dumps(quote) == b'{"symbol":"AAPL","price":12.5}'
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { json } = require('yggdryl')

    const quote = json.loads('{"symbol":"AAPL","price":12.5}')

    assert.equal(quote.symbol, 'AAPL')
    assert.equal(quote.price, 12.5)
    assert.deepEqual(json.loads(json.dumps(quote)), quote)
    ```

`Value` is not a text type. It lives in [`yggdryl::generic`](generic.md) and is re-exported as
`yggdryl::Value` and `yggdryl::text::Value`, because it is what [`json`](json.md),
[`yaml`](yaml.md), and [`toml`](toml.md) parse into, what a [`Field`](field.md) validates and
canonicalizes, and what both bindings convert their own objects into. `yggdryl::text` is the layer
above it: one dispatch over the four grammars, and one way to move a value through an
[`IOBase`](io.md) handle.

Python decodes to `dict`, `list`, `str`, `int`, `float`, `bytes`, and `None`; JavaScript decodes to
plain objects, arrays, `Map`, `Buffer`, and `Date`. JavaScript keeps one wrapper, `Value`, for the
values it has no type for at all - an exact decimal, a date, a time of day, a duration, and a
timestamp a `Date` cannot hold - and that class is also its `fromJs`/`asJs` pivot.

## What a value can be

=== "Rust"

    ```rust
    use yggdryl::Value;

    assert!(Value::Null.is_null());
    assert_eq!(Value::from("AAPL").kind(), "string");
    assert!(Value::from(1.5).is_number());

    // Width is representation, not identity.
    assert_eq!(Value::I64(1), Value::U64(1));

    // Narrowing refuses to lose magnitude rather than wrapping.
    assert_eq!(Value::from(7_i64).as_i64(), Some(7));
    assert_eq!(Value::from(i128::MAX).as_i64(), None);

    // Floats keep their exact bits: signed zeros stay apart, every NaN is one value.
    assert_ne!(Value::from(-0.0), Value::from(0.0));
    assert_eq!(Value::from(f64::NAN), Value::from(f64::NAN));
    ```

=== "Python"

    ```python
    import math

    from yggdryl import json

    assert json.loads("null") is None

    # Width is a wire detail; Python sees one int type.
    assert json.loads(json.dumps(2**70)) == 2**70

    # Floats keep their exact bits.
    assert math.copysign(1.0, json.loads(json.dumps(-0.0))) == -1.0
    assert math.isnan(json.loads(json.dumps(math.nan)))

    # Bytes survive a text format.
    assert json.loads(json.dumps(b"\x00\x01")) == b"\x00\x01"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { json } = require('yggdryl')

    assert.equal(json.loads('null'), null)

    // A value too wide for a Number arrives as a BigInt.
    assert.equal(json.loads(json.dumps(2n ** 70n)), 2n ** 70n)

    // Floats keep their exact bits.
    assert.ok(Object.is(json.loads(json.dumps(-0)), -0))
    assert.ok(Number.isNaN(json.loads(json.dumps(NaN))))

    // Bytes survive a text format.
    assert.deepEqual(json.loads(json.dumps(Buffer.from([0, 1]))), Buffer.from([0, 1]))
    ```

Sixteen variants: `Null`, `Bool`, `I64`, `U64`, `I128`, `U128`, `Float`, `Decimal`, `String`,
`Bytes`, `Date`, `Time`, `Timestamp`, `Duration`, `Sequence`, and `Mapping`. Every one of them
carries its own parts, so nothing in the tree is a name over an untyped payload: a timestamp holds
its unit and its zone, a decimal holds its coefficient and its scale. Four of them are integers because a wire representation is
worth keeping, but they are one number as far as equality, ordering, and hashing are concerned -
`I64(1)` and `U64(1)` are the same key in a mapping. `as_i64` and `as_u64` narrow only when the
magnitude fits and return `None` otherwise, so nothing wraps silently.

`Float` wraps the bits rather than the `f64`. NaN payloads are normalized at construction, which
makes NaN equal to itself and orders every float totally; positive and negative zero stay distinct
so a codec can write back exactly what it read. `kind` gives one lowercase word per variant -
`null`, `boolean`, `i64`, `float`, `string`, `mapping`, and so on - and it is the spelling error
messages use for an observed value, so `expected string, got mapping` reads the same everywhere.

`Bytes` and the wide integers have no JSON, TOML, or YAML spelling. They are written into an
envelope the decoder reads back as itself, which is why the round trips above hold in all three
languages.

## Reading a shape you do not control

=== "Rust"

    ```rust
    use yggdryl::Value;
    use yggdryl::text::{Json, TextCodec};

    let order = Json.loads(r#"{"symbol":"AAPL","legs":[{"price":12},{"price":13}],"venue":null}"#)?;

    // One dotted path walks mapping keys and sequence indexes.
    assert_eq!(order.path("legs.1.price").and_then(Value::as_i64), Some(13));

    // A segment that does not resolve is absence, not an error.
    assert!(order.path("legs.9.price").is_none());
    assert!(order.path("symbol.price").is_none());

    assert_eq!(order.keys(), vec!["symbol", "legs", "venue"]);
    assert!(order.contains_key("venue"));
    assert_eq!(order.entries().count(), 3);

    // A present null counts as absent for a default.
    let fallback = Value::from("XPAR");
    assert_eq!(order.get_or("venue", &fallback), &fallback);
    ```

=== "Python"

    ```python
    from yggdryl import json

    order = json.loads('{"symbol":"AAPL","legs":[{"price":12},{"price":13}],"venue":null}')

    assert order["legs"][1]["price"] == 13
    assert list(order) == ["symbol", "legs", "venue"]
    assert "venue" in order
    assert len(order.items()) == 3

    # A missing key takes the default; a present null is still None.
    assert order.get("currency", "EUR") == "EUR"
    assert order["venue"] is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { json } = require('yggdryl')

    const order = json.loads('{"symbol":"AAPL","legs":[{"price":12},{"price":13}],"venue":null}')

    assert.equal(order.legs[1].price, 13)
    assert.ok('venue' in order)
    assert.equal(Object.entries(order).length, 3)

    // Nullish coalescing treats a present null as absent, the same as get_or.
    assert.equal(order.venue ?? 'XPAR', 'XPAR')
    assert.equal(order.currency ?? 'EUR', 'EUR')
    ```

`path` is the reason a caller rarely matches on variants. `"legs.1.price"` reads the key `legs`,
then index `1`, then the key `price`, and any segment that does not resolve - a missing key, an
out-of-range index, a scalar where a container was expected - returns `None` instead of an error.
Probing an unknown document needs no nesting.

The rest of the mapping surface behaves the same way on a value whose shape is not yet known:
`keys` collects the string keys in insertion order and skips the rest, `entries` iterates pairs,
`contains_key` answers without allocating, and all three yield nothing rather than failing when the
value is not a mapping. `get_or` is the one with an opinion: a key that is present but null counts
as absent, which is JavaScript's `??` and needs an explicit `is not None` in Python.

Mappings keep arbitrary keys, not just strings, so `get_key` takes a `Value` and `get_key_str`
takes a `&str` without building a temporary. A mapping with non-string keys goes into an envelope
on the way out and comes back as a `Map` in JavaScript and as a `dict` with those same keys in
Python.

## Rebuilding a mapping

=== "Rust"

    ```rust
    use yggdryl::Value;
    use yggdryl::text::{Json, TextCodec};

    let order = Json.loads(r#"{"symbol":"AAPL","venue":null}"#)?;

    // Replacing keeps position; a new key is appended.
    let updated = order.with_key("venue", "XPAR")?.with_key("currency", "EUR")?;
    assert_eq!(updated.keys(), vec!["symbol", "venue", "currency"]);
    assert_eq!(updated.path("venue").and_then(Value::as_str), Some("XPAR"));

    let trimmed = updated.without_key("venue")?;
    assert_eq!(trimmed.keys(), vec!["symbol", "currency"]);

    // Removing something absent changes nothing.
    assert_eq!(trimmed.without_key("absent")?, trimmed);

    // Rebuilding something that is not a mapping says what it is.
    let message = Value::from("AAPL")
        .with_key("symbol", "AAPL")
        .unwrap_err()
        .to_string();
    assert!(message.contains("expected a mapping"), "{message}");
    assert!(message.contains("string"), "{message}");
    ```

=== "Python"

    ```python
    from yggdryl import json

    order = json.loads('{"symbol":"AAPL","venue":null}')

    updated = {**order, "venue": "XPAR", "currency": "EUR"}
    assert list(updated) == ["symbol", "venue", "currency"]
    assert updated["venue"] == "XPAR"

    trimmed = {key: value for key, value in updated.items() if key != "venue"}
    assert list(trimmed) == ["symbol", "currency"]

    # The rebuilt mapping still encodes.
    assert json.dumps(trimmed) == b'{"symbol":"AAPL","currency":"EUR"}'
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { json } = require('yggdryl')

    const order = json.loads('{"symbol":"AAPL","venue":null}')

    const updated = { ...order, venue: 'XPAR', currency: 'EUR' }
    assert.equal(updated.venue, 'XPAR')
    assert.equal(updated.symbol, 'AAPL')

    const { venue, ...trimmed } = updated
    assert.ok(!('venue' in trimmed))
    assert.equal(trimmed.currency, 'EUR')

    // The rebuilt object still encodes.
    assert.equal(json.loads(json.dumps(trimmed)).currency, 'EUR')
    ```

A `Value` is immutable and cheap to clone - strings, byte payloads, sequences, and mappings share
their storage - so editing returns a new value instead of mutating one. `with_key` replaces in
place when the key exists and appends when it does not, which keeps a rebuilt mapping reading in
the order it was written. `without_key` treats an absent key as nothing to do. Both refuse on a
non-mapping and name the kind they found, rather than returning the value unchanged.

## A name is not a type

=== "Rust"

    ```rust
    use yggdryl::{Value, json, yaml};

    // `type: "tag"` once named a carrier that held a free-form name over an
    // untyped payload. It names nothing now, so it is not an envelope and the
    // document is the mapping its syntax always was.
    let source = br#"{"$yggdryl":{"version":1,"type":"tag","tag":"app:Trade","value":{"symbol":"AAPL"}}}"#;
    let decoded = json::from_slice(source)?;
    assert_eq!(
        decoded.path("$yggdryl.tag").and_then(Value::as_str),
        Some("app:Trade")
    );
    // ... and it goes back out unchanged, escaped through the mapping envelope.
    assert_eq!(json::from_slice(&json::to_vec(&decoded)?)?, decoded);

    // A YAML application tag is the annotation YAML defines it to be, so the
    // node under it decodes as the plain value it annotates.
    let annotated = yaml::from_slice(b"!app:Trade {symbol: AAPL}\n")?;
    assert_eq!(annotated.path("symbol").and_then(Value::as_str), Some("AAPL"));
    ```

=== "Python"

    ```python
    from yggdryl import json, yaml

    tagged = (
        '{"$yggdryl":{"version":1,"type":"tag","tag":"app:Trade",'
        '"value":{"symbol":"AAPL"}}}'
    )
    assert json.loads(tagged) == {
        "$yggdryl": {
            "version": 1,
            "type": "tag",
            "tag": "app:Trade",
            "value": {"symbol": "AAPL"},
        }
    }

    # A YAML application tag annotates a node; the node is what arrives.
    assert yaml.loads("!app:Trade {symbol: AAPL}\n") == {"symbol": "AAPL"}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { json, yaml } = require('yggdryl')

    const tagged =
      '{"$yggdryl":{"version":1,"type":"tag","tag":"app:Trade","value":{"symbol":"AAPL"}}}'
    assert.deepEqual(json.loads(tagged), {
      $yggdryl: {
        version: 1,
        type: 'tag',
        tag: 'app:Trade',
        value: { symbol: 'AAPL' },
      },
    })

    // A YAML application tag annotates a node; the node is what arrives.
    assert.deepEqual(yaml.loads('!app:Trade {symbol: AAPL}\n'), { symbol: 'AAPL' })
    ```

There is no tag carrier. A name over an untyped payload is not a type, because nothing checks that
the payload matches the name, so a value that read `app:Trade` told a caller only that some producer
had written those nine characters. Every kind a `Value` holds is instead a variant carrying its own
parts, and `Value::data_type` reads the datatype straight off the variant for exactly that reason.

What remains is a rule about names a document does carry. Each `$yggdryl` envelope kind names a
`Value` variant; a `type` that names nothing the value model holds is not an envelope, so its
mapping decodes as the mapping it is and is written back through the mapping envelope unchanged. In
YAML, `!yggdryl/bytes` and its siblings still select the kind they name, and every other tag is read
as the annotation YAML defines it to be. Nothing on the write path emits a tag in any format, so no
round trip through this crate produces one.

## One value against one datatype

!!! note "Rust only"
    `TypedValue` is a core value the bindings do not project yet.

`Value::data_type` names the datatype a value already is, and a [`Field`](field.md) validates a whole
row against a schema. `TypedValue` is the pair in between: one value and one datatype, checked
against each other, for a caller holding a single value with no row and no schema around it.

```rust
use yggdryl::{DataType, TypedValue, Value};

let price = TypedValue::from_parts(DataType::Int64, Value::from(7_i64))?;
assert_eq!(price.data_type(), &DataType::Int64);
assert_eq!(price.value(), &Value::I64(7));

// The value is checked against the datatype, through the same walk a column
// value takes, so a pairing that exists is one that holds.
assert!(TypedValue::from_parts(DataType::Int64, Value::from("seven")).is_err());

// A value can also name its own datatype.
assert_eq!(
    TypedValue::from_value(Value::from(1.5))?.data_type(),
    &DataType::Float64
);

// A null is accepted by every datatype: nullability belongs to the field that
// holds the column, not to the value in it.
let missing = TypedValue::from_parts(DataType::Int64, Value::Null)?;
assert!(missing.is_null());
assert!(!price.is_null());
```

The last one is the rule the whole value model follows. A `Value` accepts a null wherever a value
goes, and the schema beside it says whether that was allowed - which is why inference reports a
column of nulls as `null` and a null among real values as the datatype of the others, made nullable.

## A typed value per datatype

!!! note "Rust only"
    The typed value markers are core values the bindings do not project yet.

A pairing that has not been narrowed holds any datatype. A caller who knows which datatype is
coming can say so in the type instead: `TypedValue<K>` takes the same compile-time markers a
[`Field`](field.md) takes, and there is one alias per datatype.

```rust
use yggdryl::generic::{Int64Value, TimestampValue, Utf8Value};
use yggdryl::{DataType, TimeUnit, TypedValue, Value};

// A statically known datatype needs only its value.
let price = Int64Value::new(Value::from(7_i64))?;
assert_eq!(price.data_type(), &DataType::Int64);

// The marker is checked, and so is the value against it.
assert!(Int64Value::new(Value::from("seven")).is_err());
assert!(Int64Value::try_from_parts(DataType::Utf8, Value::from("seven")).is_err());
assert_eq!(Utf8Value::try_from_value(Value::from("AAPL"))?.value(), &Value::from("AAPL"));

// A parameterized datatype keeps its parameters in the pairing, not the marker.
let at = TimestampValue::try_from_parts(
    DataType::Timestamp(TimeUnit::Microsecond, None),
    Value::timestamp(0, TimeUnit::Microsecond, None)?,
)?;
assert_eq!(at.data_type(), &DataType::Timestamp(TimeUnit::Microsecond, None));

// Narrowing and widening move the same two halves between markers.
let dynamic: TypedValue = at.into_any();
assert!(dynamic.try_into_typed::<yggdryl::field::binary::Utf8>().is_err());
```

The marker is zero-sized, so `Int64Value` is the same two words `TypedValue` is; it narrows what the
type system will accept, not what the value costs. `AnyType` is the marker that accepts every
datatype, and it is what a bare `TypedValue` carries - which is why `from_parts` and `from_value`
stay the dynamic spellings and the narrowed ones are `try_from_parts` and `try_from_value`.

## Four formats, one surface

=== "Rust"

    ```rust
    use yggdryl::MimeType;
    use yggdryl::text::{Json, Jsonl, TextCodec, Toml, Yaml};

    let quote = Json.loads(r#"{"symbol":"AAPL"}"#)?;

    // One value, four grammars, one set of methods.
    assert_eq!(Json.dumps(&quote)?, r#"{"symbol":"AAPL"}"#);
    assert_eq!(Toml.loads(&Toml.dumps(&quote)?)?, quote);
    assert_eq!(Yaml.loads(&Yaml.dumps(&quote)?)?, quote);

    assert_eq!(Json.mime_type(), MimeType::JSON);
    assert_eq!(Jsonl.mime_type(), MimeType::JSON_LINES);
    assert!(Jsonl.is_multi_document());
    assert!(!Toml.is_multi_document());

    // Bytes and readers are the same operation on a different carrier.
    let bytes = Json.dump_vec(&quote)?;
    assert_eq!(Json.load_slice(&bytes)?, quote);
    assert_eq!(Json.read(bytes.as_slice())?, quote);

    let rows = Jsonl.loads_all("{\"id\":1}\n{\"id\":2}\n")?;
    assert_eq!(rows.len(), 2);
    ```

=== "Python"

    ```python
    from yggdryl import json, toml, yaml

    quote = {"symbol": "AAPL"}

    assert json.dumps(quote) == b'{"symbol":"AAPL"}'
    assert toml.loads(toml.dumps(quote)) == quote
    assert yaml.loads(yaml.dumps(quote)) == quote

    # JSON and YAML hold many documents; TOML holds exactly one.
    assert json.dumps_all([{"id": 1}, {"id": 2}]) == b'{"id":1}\n{"id":2}\n'
    assert list(json.loads_all(b'{"id":1}\n{"id":2}\n')) == [{"id": 1}, {"id": 2}]
    assert list(yaml.loads_all("a: 1\n---\na: 2\n")) == [{"a": 1}, {"a": 2}]
    assert not hasattr(toml, "dumps_all")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { json, toml, yaml } = require('yggdryl')

    const quote = { symbol: 'AAPL' }

    assert.equal(json.dumps(quote).toString(), '{"symbol":"AAPL"}')
    assert.deepEqual(toml.loads(toml.dumps(quote)), quote)
    assert.deepEqual(yaml.loads(yaml.dumps(quote)), quote)

    // JSON and YAML hold many documents; TOML holds exactly one.
    assert.equal(json.dumpAll([{ id: 1 }, { id: 2 }]).toString(), '{"id":1}\n{"id":2}\n')
    assert.deepEqual(json.loadsAll('{"id":1}\n{"id":2}\n'), [{ id: 1 }, { id: 2 }])
    assert.deepEqual(yaml.loadsAll('a: 1\n---\na: 2\n'), [{ a: 1 }, { a: 2 }])
    assert.equal(toml.dumpAll, undefined)
    ```

`Json`, `Jsonl`, `Toml`, and `Yaml` are unit structs, and `TextCodec` is the trait all four answer:
`loads`/`dumps` for text, `load_slice`/`dump_vec` for bytes, `read`/`write` for a reader or writer,
`load`/`dump` for an [`IOBase`](io.md) handle, and an `_all` form alongside for the formats that
hold more than one value. The format picks the grammar; nothing else about the call changes. A
format value where a codec is expected is the whole configuration - `Json.loads(text)` reads as the
format itself - and [`with_limits`](#bounds-on-untrusted-input) returns a `Limited<Json>` that
answers the same trait.

`Format` is the enum underneath: `Json`, `JsonLines`, `Yaml`, `Toml`. The free functions
`text::from_str`, `from_slice`, `from_reader`, `to_vec`, and `to_writer` take one and dispatch to
[`json`](json.md), [`yaml`](yaml.md), or [`toml`](toml.md), which is what the trait methods call.
`Format::mime_type` maps each to the [`MimeType`](enums.md) a written value carries.

Only JSON Lines and YAML hold more than one document. `is_multi_document` says which, and the
bindings enforce it by simply not exposing an `_all` form on `toml`.

## Inferring the format

=== "Rust"

    ```rust
    use yggdryl::text::{from_str_inferred, infer_format};
    use yggdryl::{Format, Value};

    // Valid JSON wins, because most JSON is also valid YAML.
    assert_eq!(infer_format(br#"{"symbol":"AAPL"}"#)?, Format::Json);
    assert_eq!(infer_format(b"symbol = \"AAPL\"\n")?, Format::Toml);
    assert_eq!(infer_format(b"symbol: AAPL\n")?, Format::Yaml);

    // Inferring and decoding is one parse, not two.
    let (format, value) = from_str_inferred("symbol = \"AAPL\"\n")?;
    assert_eq!(format, Format::Toml);
    assert_eq!(value.path("symbol").and_then(Value::as_str), Some("AAPL"));

    // A name decides too, and JSON Lines needs one.
    assert_eq!(Format::from_path("events.jsonl")?, Format::JsonLines);
    assert_eq!(Format::from_extension(".YML")?, Format::Yaml);
    assert_eq!(Format::from_str("application/toml")?, Format::Toml);
    ```

=== "Python"

    ```python
    from yggdryl import json, toml, yaml

    # There is nothing to infer: the module you import is the format.
    quote = {"symbol": "AAPL"}
    assert json.loads('{"symbol":"AAPL"}') == quote
    assert toml.loads('symbol = "AAPL"\n') == quote
    assert yaml.loads("symbol: AAPL\n") == quote
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { codec } = require('yggdryl')

    // codec.from infers the grammar from the content itself.
    assert.deepEqual(codec.from('{"symbol":"AAPL"}'), { symbol: 'AAPL' })
    assert.deepEqual(codec.from('symbol = "AAPL"\n'), { symbol: 'AAPL' })
    assert.deepEqual(codec.from('symbol: AAPL\n'), { symbol: 'AAPL' })

    // An explicit format overrides content.
    assert.deepEqual(codec.from('symbol = "AAPL"\n', { format: 'toml' }), { symbol: 'AAPL' })
    ```

`infer_format` decides from content alone, in a fixed order: valid JSON first, because JSON is a
subset of YAML and reporting YAML for `{"a":1}` would be useless; then empty or comment-only input,
which stays YAML; then TOML, only when the complete document parses; then YAML for everything left.
JSON Lines is never inferred from content, because arbitrary newlines are ambiguous - it needs an
explicit `Format` or a `.jsonl` suffix.

`from_str_inferred` and `from_slice_inferred` return the format alongside the value, having parsed
once. Deciding first with `infer_format` and then parsing again does the same work twice.

A name is the other source. `Format::from_str` accepts a format name, a
[`MimeType`](enums.md) spelling, or an alias (`jsonl`, `ndjson`, `yml`); `from_extension` takes one
with or without its dot; `from_path` takes the final extension of a path. All three are
case-insensitive.

JavaScript's `codec.from` is the binding for this: it infers from content, or from a path suffix
when handed a path, and takes an explicit `format` option over either. Python has no inference
entry point at all - the module you import is the format.

## Bounds on untrusted input

=== "Rust"

    ```rust
    use yggdryl::Limits;
    use yggdryl::text::{Json, TextCodec};

    let strict = Json.with_limits(Limits::new(2, 1024, 64, 4));
    assert_eq!(strict.limits().max_depth(), 2);

    // A root container is depth 1.
    assert!(strict.loads(r#"{"a":[1]}"#).is_ok());
    assert!(strict.loads(r#"{"a":[[1]]}"#).is_err());

    // A bare format value uses the defaults.
    assert_eq!(Json.limits(), Limits::default());
    assert_eq!(Limits::default().max_depth(), 128);
    assert_eq!(Limits::default().max_input_bytes(), 64 * 1024 * 1024);
    assert_eq!(Limits::default().max_documents(), 1_024);
    ```

=== "Python"

    ```python
    from yggdryl import json

    # The default bound is enforced, not advisory.
    assert json.loads("[" * 100 + "]" * 100) is not None

    try:
        json.loads("[" * 200 + "]" * 200)
    except ValueError as error:
        assert "nesting depth limit exceeded" in str(error)
    else:
        raise AssertionError("the depth limit was not applied")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { json } = require('yggdryl')

    // maxDepth tightens the same bound the core applies.
    assert.deepEqual(json.loads('{"a":[1]}', { maxDepth: 2 }), { a: [1] })

    assert.throws(
      () => json.loads('{"a":[[1]]}', { maxDepth: 2 }),
      /nesting depth limit exceeded/,
    )
    ```

`Limits` is four numbers: `max_depth` for structural nesting, `max_input_bytes` for one decoder
invocation, `max_nodes` for scalar and container nodes in a document, and `max_documents` for a
stream. They default to 128, 64 MiB, one million, and 1024, and they apply on every path - a bare
`Json.loads` is already bounded. `TextCodec::with_limits` returns a `Limited<C>` carrying tighter
ones, and `load_with_limits`, `load_all_with_limits`, and the `_with_limits` free functions take
them directly.

The bindings expose the bound they can usefully vary. JavaScript takes `maxDepth` per call, capped
at 48; Python applies the defaults.

## Failures carry a byte position

=== "Rust"

    ```rust
    use yggdryl::text::{Json, TextCodec, Toml};

    let message = Json.loads(r#"{"symbol": "#).unwrap_err().to_string();
    assert!(message.contains("invalid json data at byte 11"), "{message}");

    let message = Toml.loads("symbol = ").unwrap_err().to_string();
    assert!(message.contains("invalid toml data at byte 9"), "{message}");
    ```

=== "Python"

    ```python
    from yggdryl import json, toml

    try:
        json.loads('{"symbol": ')
    except ValueError as error:
        assert "invalid json data at byte 11" in str(error)

    try:
        toml.loads("symbol = ")
    except ValueError as error:
        assert "invalid toml data at byte 9" in str(error)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { json, toml } = require('yggdryl')

    assert.throws(() => json.loads('{"symbol": '), /invalid json data at byte 11/)
    assert.throws(() => toml.loads('symbol = '), /invalid toml data at byte 9/)
    ```

Every parse failure is `Error::Codec { format, position, reason }`, and `position` is a byte offset
into the input the decoder was handed - not a line and column, and not a character index. Formats
that report line and column internally convert before the error leaves the parser, so one number
means the same thing across all four grammars and survives translation into a Python `ValueError`
or a JavaScript `Error` unchanged.

An offset is bounded by the input length, so it stays usable for slicing even when the failure is
"unexpected end of input". When a binding frames a multi-document stream itself, it rewrites the
position to the offset in the whole stream and keeps the per-document one alongside it.

## Through a storage handle

=== "Rust"

    ```rust
    use yggdryl::io::{Buffer, IOBase};
    use yggdryl::text::{Plan, dump, load};
    use yggdryl::{Codec, Format, Url, Value};

    let mut handle =
        Buffer::new().with_media_type(Url::from_str("file:///quote.json.gz")?.media_type());

    // The name is the whole configuration.
    let plan = Plan::infer(&handle)?;
    assert_eq!(plan.format(), Format::Json);
    assert_eq!(plan.codec(), Codec::Gzip);

    let quote = Value::from_mapping([(Value::from("symbol"), Value::from("AAPL"))])?;
    dump(&mut handle, &quote)?;

    // The stored bytes really are gzip, and reading decompresses them.
    assert_eq!(&handle.as_slice()[..2], &[0x1F, 0x8B]);
    assert_eq!(load(&handle)?, quote);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import json, toml

    with tempfile.TemporaryDirectory() as directory:
        quote = pathlib.Path(directory) / "quote.json"
        json.dump({"symbol": "AAPL"}, quote)

        assert quote.read_bytes() == b'{"symbol":"AAPL"}'
        assert json.load(quote) == {"symbol": "AAPL"}

        # The suffix picks the reader; a str path works too.
        table = pathlib.Path(directory) / "quote.toml"
        toml.dump({"symbol": "AAPL"}, str(table))
        assert toml.load(table) == {"symbol": "AAPL"}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { codec, json } = require('yggdryl')

    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-text-'))
    const quote = path.join(directory, 'quote.json')

    json.dump({ symbol: 'AAPL' }, quote)
    assert.equal(fs.readFileSync(quote).toString(), '{"symbol":"AAPL"}')
    assert.deepEqual(json.load(quote), { symbol: 'AAPL' })

    // codec.from takes the format from the suffix.
    assert.deepEqual(codec.from(quote), { symbol: 'AAPL' })
    fs.rmSync(directory, { recursive: true, force: true })
    ```

An [`IOBase`](io.md) handle already knows where its bytes live and what they are, so `load` and
`dump` take one argument and no format. A `Plan` is what they derive from it: the `Format` and the
[`Codec`](enums.md) content coding, both read off the handle's [`MediaType`](enums.md). A
location-addressed handle gets that media type from its compound filename, so `trades.json.gz`
decompresses on read and recompresses on write with nothing said about either - as does
`trades.yaml.zst`, through [`gzip`](gzip.md) and [`zstd`](zstd.md) respectively.

The two sources are not weighed the same way. Content codings have byte signatures, so `Plan::detect`
lets the payload override a handle that claims plain JSON but holds gzip. Structured text has no
signature and `{` identifies neither JSON nor YAML, so the declared media type decides the format
and content is consulted only when the name says nothing. `Plan::infer` uses the media type alone;
`Plan::new` takes both explicitly. `dump_with_level` picks the compression level, and a failed dump
leaves the handle's previous bytes untouched, because encoding completes before anything is written.

The bindings have no handle type. `load` and `dump` take a path, a string, bytes, or a file object
directly, and derive the format from the path suffix or the content - which means they read and
write plain text and leave content coding to the caller. `TextCodec::load` and `TextCodec::dump`
are the per-format form of the same two functions, using that format instead of the handle's.

<!-- notebooks: generated by scripts/build_docs_notebooks.py -->

## Notebooks

Every example on this page, as a notebook generated from these blocks and
shipped unexecuted:
[Rust](notebooks/text-rust.ipynb){ download },
[Python](notebooks/text-python.ipynb){ download },
[JavaScript](notebooks/text-javascript.ipynb){ download }.

<!-- /notebooks -->
