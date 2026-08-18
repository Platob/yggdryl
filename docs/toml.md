# TOML

`yggdryl::toml` reads and writes one TOML document as the shared [`Value`](text.md).

=== "Rust"

    ```rust
    use yggdryl::{Value, toml};

    let source = "title = \"yggdryl\"\ncount = 3\n\n[owner]\nname = \"Ada\"\n";
    let value = toml::from_str(source)?;

    assert_eq!(value.get_key_str("title"), Some(&Value::from("yggdryl")));
    assert_eq!(value.get_key_str("count"), Some(&Value::I64(3)));
    let owner = value.get_key_str("owner").unwrap();
    assert_eq!(owner.get_key_str("name"), Some(&Value::from("Ada")));

    assert_eq!(toml::from_slice(&toml::to_vec(&value)?)?, value);
    ```

=== "Python"

    ```python
    from yggdryl import toml

    source = 'title = "yggdryl"\ncount = 3\n\n[owner]\nname = "Ada"\n'
    value = toml.loads(source)

    assert value == {"title": "yggdryl", "count": 3, "owner": {"name": "Ada"}}
    assert value["owner"]["name"] == "Ada"
    assert toml.loads(toml.dumps(value)) == value
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { toml } = require('yggdryl')

    const source = 'title = "yggdryl"\ncount = 3\n\n[owner]\nname = "Ada"\n'
    const value = toml.loads(source)

    assert.deepEqual(value, { title: 'yggdryl', count: 3, owner: { name: 'Ada' } })
    assert.equal(value.owner.name, 'Ada')
    assert.deepEqual(toml.loads(toml.dumps(value)), value)
    ```

There is no TOML-specific value type. What comes back is the same `Value` that
[json](json.md) and [yaml](yaml.md) produce, which is what lets a document change format
without changing shape. Rust takes its input as borrowed text (`from_str`), bytes
(`from_slice`), or a reader (`from_reader`) and writes through `to_vec` or `to_writer`.
Python's `loads` accepts bytes, a `str` of content, a `pathlib.Path`, or a readable file
object, and `load` is the same function under the name the standard library uses.
JavaScript splits the two: `loads`/`dumps` take content, `load`/`dump` take a path, file
descriptor, stream, or URL.

## Table order

=== "Rust"

    ```rust
    use yggdryl::{Value, toml};

    let value = Value::from_mapping([
        (Value::from("zeta"), Value::I64(1)),
        (Value::from("beta"), Value::I64(2)),
        (
            Value::from("alpha"),
            Value::from_mapping([(Value::from("deep"), Value::I64(3))])?,
        ),
    ])?;

    let encoded = toml::to_vec(&value)?;
    assert_eq!(
        String::from_utf8(encoded.clone())?,
        "\"zeta\" = 1\n\"beta\" = 2\n\"alpha\" = {\"deep\" = 3}\n",
    );

    let decoded = toml::from_slice(&encoded)?;
    assert_eq!(decoded, value);
    let keys: Vec<&str> = decoded
        .mapping_iter()
        .map(|(key, _)| key.as_str().unwrap())
        .collect();
    assert_eq!(keys, ["zeta", "beta", "alpha"]);
    ```

=== "Python"

    ```python
    from yggdryl import toml

    value = {"zeta": 1, "beta": 2, "alpha": {"deep": 3}}

    encoded = toml.dumps(value)
    assert encoded == b'"zeta" = 1\n"beta" = 2\n"alpha" = {"deep" = 3}\n'
    assert list(toml.loads(encoded)) == ["zeta", "beta", "alpha"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { toml } = require('yggdryl')

    const value = { zeta: 1, beta: 2, alpha: { deep: 3 } }

    const encoded = toml.dumps(value)
    assert.equal(
      encoded.toString('utf8'),
      '"zeta" = 1\n"beta" = 2\n"alpha" = {"deep" = 3}\n',
    )

    // Decoding rebuilds a plain object out of a sorted map.
    assert.deepEqual(Object.keys(toml.loads(encoded)), ['alpha', 'beta', 'zeta'])
    ```

A mapping is written in the order it holds its entries, never sorted, and a decoded
document holds them in the order the file listed them. The emitter takes the one shape
that makes this true for every input: every key is quoted, every sub-table is inline, and
one entry is one line. So there are no `[section]` headers on the way out, no reflowing,
and the bytes are a function of the mapping alone.

Order survives encoding in all three languages, but only Rust and Python read it back.
The JavaScript binding materializes a string-keyed table as a plain object built from a
sorted map, so `Object.keys` on a decoded table is alphabetical rather than the
document's order.

## The type mapping

=== "Rust"

    ```rust
    use yggdryl::{Value, toml};

    let value = toml::from_str(concat!(
        "text = \"café\"\n",
        "integer = 7\n",
        "hex = 0x2a\n",
        "float = 1.5\n",
        "infinite = inf\n",
        "negative_zero = -0.0\n",
        "flag = true\n",
        "array = [1, \"two\"]\n",
        "table = { nested = 1 }\n",
        "moment = 1979-05-27T07:32:00Z\n",
    ))?;

    assert_eq!(value.get_key_str("text").and_then(Value::as_str), Some("café"));
    assert_eq!(value.get_key_str("integer"), Some(&Value::I64(7)));
    assert_eq!(value.get_key_str("hex"), Some(&Value::I64(42)));
    assert_eq!(value.get_key_str("float").and_then(Value::as_f64), Some(1.5));
    assert_eq!(
        value.get_key_str("infinite").and_then(Value::as_f64),
        Some(f64::INFINITY),
    );
    assert_eq!(
        value
            .get_key_str("negative_zero")
            .and_then(Value::as_f64)
            .map(f64::to_bits),
        Some((-0.0_f64).to_bits()),
    );
    assert_eq!(value.get_key_str("flag"), Some(&Value::Bool(true)));
    assert_eq!(value.get_key_str("array").map(Value::len), Some(2));
    assert_eq!(
        value.get_key_str("table").unwrap().get_key_str("nested"),
        Some(&Value::I64(1)),
    );
    assert_eq!(value.get_key_str("moment").unwrap().kind(), "timestamp");
    ```

=== "Python"

    ```python
    import datetime as dt
    import math

    from yggdryl import toml

    value = toml.loads(
        'text = "café"\n'
        "integer = 7\n"
        "hex = 0x2a\n"
        "float = 1.5\n"
        "infinite = inf\n"
        "negative_zero = -0.0\n"
        "flag = true\n"
        'array = [1, "two"]\n'
        "table = { nested = 1 }\n"
        "moment = 1979-05-27T07:32:00Z\n"
    )

    assert isinstance(value["text"], str)
    assert isinstance(value["integer"], int) and not isinstance(value["integer"], bool)
    assert value["hex"] == 42
    assert isinstance(value["float"], float)
    assert value["infinite"] == math.inf
    assert math.copysign(1.0, value["negative_zero"]) == -1.0
    assert value["flag"] is True
    assert value["array"] == [1, "two"]
    assert value["table"] == {"nested": 1}
    assert value["moment"] == dt.datetime(1979, 5, 27, 7, 32, tzinfo=dt.timezone.utc)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Value, toml } = require('yggdryl')

    const value = toml.loads(
      'text = "café"\n' +
        'integer = 7\n' +
        'hex = 0x2a\n' +
        'float = 1.5\n' +
        'infinite = inf\n' +
        'negative_zero = -0.0\n' +
        'flag = true\n' +
        'array = [1, "two"]\n' +
        'table = { nested = 1 }\n' +
        'moment = 1979-05-27T07:32:00Z\n',
    )

    assert.equal(typeof value.text, 'string')
    assert.equal(value.integer, 7)
    assert.equal(value.hex, 42)
    assert.equal(value.float, 1.5)
    assert.equal(value.infinite, Infinity)
    assert.ok(Object.is(value.negative_zero, -0))
    assert.equal(value.flag, true)
    assert.deepEqual(value.array, [1, 'two'])
    assert.deepEqual(value.table, { nested: 1 })
    assert.ok(value.moment.equals(Value.timestamp(296638320n, 's', 'UTC')))
    ```

| TOML | `Value` | Python | JavaScript |
| --- | --- | --- | --- |
| string | `String` | `str` | `string` |
| integer | `I64` | `int` | `number` |
| float | `Float` | `float` | `number` |
| boolean | `Bool` | `bool` | `boolean` |
| array | `Sequence` | `list` | `Array` |
| table, inline table | `Mapping` | `dict` | `object` |
| offset date-time | `Timestamp` | `datetime.datetime` | `Value` |
| local date-time | `Timestamp` | `datetime.datetime` | `Date` |
| local date | `Date` | `datetime.date` | `Value` |
| local time | `Time` | `datetime.time` | `Value` |

A TOML integer is signed 64-bit and decodes as `I64` in every radix TOML writes; a
literal outside that range is an error rather than a silent float. A TOML float is an
`f64`, so `inf` and `-0.0` arrive as themselves rather than as text.

## Values TOML has no syntax for

=== "Rust"

    ```rust
    use yggdryl::{Value, toml};

    let value = Value::from_mapping([
        (Value::from("missing"), Value::Null),
        (Value::from("blob"), Value::from(vec![0_u8, 255])),
        (Value::from("huge"), Value::U128(u128::MAX)),
    ])?;

    let encoded = toml::to_vec(&value)?;
    assert!(
        String::from_utf8(encoded.clone())?
            .contains("\"missing\" = { \"$yggdryl\" = { version = 1, type = \"null\" } }")
    );
    assert_eq!(toml::from_slice(&encoded)?, value);

    // A TOML root is a table, so a non-table root is wrapped the same way.
    let root = Value::from("scalar root");
    assert_eq!(toml::from_slice(&toml::to_vec(&root)?)?, root);

    // A user table that only looks like an envelope stays user data.
    let lookalike = Value::from_mapping([(
        Value::from("$yggdryl"),
        Value::from_mapping([
            (Value::from("version"), Value::I64(1)),
            (Value::from("type"), Value::from("null")),
        ])?,
    )])?;
    assert_eq!(toml::from_slice(&toml::to_vec(&lookalike)?)?, lookalike);
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import toml

    value = {"missing": None, "blob": b"\x00\xff", "price": Decimal("1.25")}

    encoded = toml.dumps(value)
    assert b'"missing" = { "$yggdryl" = { version = 1, type = "null" } }' in encoded
    assert toml.loads(encoded) == value

    # A TOML root is a table, so a non-table root is wrapped the same way.
    assert toml.loads(toml.dumps("scalar root")) == "scalar root"

    # A user table that only looks like an envelope stays user data.
    lookalike = {"$yggdryl": {"version": 1, "type": "null"}}
    assert toml.loads(toml.dumps(lookalike)) == lookalike
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { toml } = require('yggdryl')

    const value = {
      missing: null,
      blob: Buffer.from([0, 255]),
      seen: new Map([[1, 'one']]),
    }

    const encoded = toml.dumps(value).toString('utf8')
    assert.ok(
      encoded.includes('"missing" = { "$yggdryl" = { version = 1, type = "null" } }'),
    )

    const decoded = toml.loads(encoded)
    assert.equal(decoded.missing, null)
    assert.deepEqual(decoded.blob, value.blob)
    assert.ok(decoded.seen instanceof Map)
    assert.equal(decoded.seen.get(1), 'one')

    // A TOML root is a table, so a non-table root is wrapped the same way.
    assert.equal(toml.loads(toml.dumps('scalar root')), 'scalar root')

    // A user table that only looks like an envelope stays user data.
    const lookalike = { $yggdryl: { version: 1, type: 'null' } }
    assert.deepEqual(toml.loads(toml.dumps(lookalike)), lookalike)
    ```

The shared value is wider than TOML. `Null`, `U64`, `I128`, `U128`, `Bytes`, `Decimal`,
`Duration`, a mapping whose keys are not all strings, and a temporal TOML cannot spell
have no TOML spelling, so each is written as an inline table under the
reserved key `$yggdryl` carrying `version`, a `type`, and its payload - bytes as base64,
the wide integers and a decimal coefficient as decimal text, a unit and a count as a
two-element array. Decoding recognizes an envelope only on an exact match of that shape,
so the escaped lookalike above survives as the table the caller wrote.

Every `type` an envelope may carry names one `Value` variant. A `type` that names nothing
the value model holds is not an envelope, which is why a document written by an older
producer that still spells `type = "tag"` decodes as the ordinary mapping its syntax
always was, and is written back through the `mapping` envelope unchanged.

Those payloads are the same ones [json](json.md) and [yaml](yaml.md) write, so a document
converted between the three formats carries the identical envelope body.

That is also how a language keeps a value TOML has no shape for. JavaScript's `Map` above
has keys that are not strings, so it crosses in the mapping envelope and comes back as a
`Map`; Python's `Decimal` is a `Value::Decimal`, so it crosses in the decimal envelope and
keeps its scale rather than its class name. See [python](extensions/python.md) and
[javascript](extensions/javascript.md) for the object each binding builds.

## Dates and times

=== "Rust"

    ```rust
    use yggdryl::{TimeUnit, Timezone, Value, toml};

    let value = toml::from_str(concat!(
        "offset = 1979-05-27T07:32:00Z\n",
        "local = 1979-05-27T07:32:00\n",
        "day = 1979-05-27\n",
        "clock = 07:32:00\n",
    ))?;

    // An offset reading is an instant: the count is UTC and the zone is the offset.
    assert_eq!(
        value.get_key_str("offset"),
        Some(&Value::timestamp_in(296_638_320, TimeUnit::Second, Some(Timezone::UTC))),
    );
    // A local reading carries no zone, which is how a naive reading is spelled.
    assert_eq!(
        value.get_key_str("local"),
        Some(&Value::timestamp_in(296_638_320, TimeUnit::Second, None)),
    );
    assert_eq!(value.get_key_str("day"), Some(&Value::date(3_433)));
    assert_eq!(
        value.get_key_str("clock"),
        Some(&Value::time(27_120, TimeUnit::Second)),
    );

    // Each form goes back out in the syntax it arrived in.
    let encoded = String::from_utf8(toml::to_vec(&value)?)?;
    assert!(encoded.contains("\"offset\" = 1979-05-27T07:32:00Z\n"));
    assert!(encoded.contains("\"day\" = 1979-05-27\n"));

    // A zone that names a place is not an offset, so it spells the classic
    // string - offset and bracketed name - instead of being rewritten as the
    // offset that place happens to be at.
    let paris = Value::from_mapping([(
        Value::from("at"),
        Value::timestamp(296_638_320, TimeUnit::Second, Some("Europe/Paris"))?,
    )])?;
    let encoded = String::from_utf8(toml::to_vec(&paris)?)?;
    assert!(encoded.contains(r#""at" = "1979-05-27T09:32:00+02:00[Europe/Paris]""#));
    ```

=== "Python"

    ```python
    import datetime as dt

    from yggdryl import toml

    value = toml.loads(
        "offset = 1979-05-27T07:32:00Z\n"
        "local = 1979-05-27T07:32:00\n"
        "day = 1979-05-27\n"
        "clock = 07:32:00\n"
    )

    assert value["offset"] == dt.datetime(1979, 5, 27, 7, 32, tzinfo=dt.timezone.utc)
    assert value["local"] == dt.datetime(1979, 5, 27, 7, 32)
    assert value["day"] == dt.date(1979, 5, 27)
    assert value["clock"] == dt.time(7, 32)

    # Each form goes back out in the syntax it arrived in.
    assert b'"day" = 1979-05-27\n' in toml.dumps(value)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Value, toml } = require('yggdryl')

    const value = toml.loads(
      'offset = 1979-05-27T07:32:00Z\n' +
        'local = 1979-05-27T07:32:00\n' +
        'day = 1979-05-27\n' +
        'clock = 07:32:00\n',
    )

    // A Date is a naive count of milliseconds, which is what a local
    // date-time is, so that one form arrives as a Date.
    assert.ok(value.local instanceof Date)
    assert.equal(value.local.toISOString(), '1979-05-27T07:32:00.000Z')

    // The other three have no JavaScript object of their own, so they stay
    // native values rather than being rounded into a Date.
    assert.ok(value.offset.equals(Value.timestamp(296638320n, 's', 'UTC')))
    assert.ok(value.day.equals(Value.date(3433)))
    assert.ok(value.clock.equals(Value.time(27120n, 's')))

    // Each form goes back out in the syntax it arrived in.
    assert.match(toml.dumps(value).toString('utf8'), /"day" = 1979-05-27\n/)
    ```

TOML's four temporal forms decode to the temporal values themselves. An offset date-time
is a `Timestamp` carrying the offset as its zone; a local date-time is the naive reading,
a `DateTime`, which is how "no zone at all" is spelled everywhere in the project. A local
date is a `Date` and a local time is a `Time`. The count is exact, and its unit is the
coarsest one that keeps every digit the spelling carries, so `07:32:00` is seconds and
`07:32:00.123456789` is nanoseconds.

Encoding spells the reading again from those parts, so a document written in TOML's
canonical form leaves as the same bytes. What does not survive is a spelling TOML has more
than one of: `1979-05-27 07:32:00.123000000` and `1979-05-27T07:32:00.123` are one
instant and both leave as the second, `+00:00` leaves as `Z`, and a leap second reads as
the second that follows it, because a count from the epoch has no room for one.

A temporal TOML has no syntax for is never reinterpreted to fit. A timestamp whose zone
names a place rather than an offset spells the classic string with the bracketed name -
`"1979-05-27T09:32:00+02:00[Europe/Paris]"` - and a duration, which TOML cannot spell at
all, spells `"PT90S"`; both come back as the strings they are, and a schema is what
recovers the typed reading. A year outside TOML's four digits or a clock reading outside
one day has no classic spelling either, so it takes the `$yggdryl` envelope and comes
back typed. A reading TOML spells but the value cannot hold - year 9999 to nanosecond
precision is wider than an `i64` count of nanoseconds - is a decode error rather than a
quietly truncated fraction.

Each binding then materializes what its own language actually has. Python has a type for
all four forms. JavaScript has only `Date`, which is a naive count of whole milliseconds
and so is exactly a local date-time; the other three stay native values rather than being
rounded into one.

## Exactly one document

=== "Rust"

    ```rust
    use std::io::Cursor;

    use yggdryl::{Value, toml};

    // The root is a table, so an empty or comment-only document is an empty table.
    let empty = Value::from_mapping([])?;
    assert_eq!(toml::from_str("# nothing to see\n")?, empty);
    assert!(toml::to_vec(&empty)?.is_empty());

    // The reader is an iterator that yields exactly one document.
    let mut reader = toml::Reader::new(Cursor::new(b"id = 1"));
    assert!(reader.next().unwrap().is_ok());
    assert!(reader.next().is_none());

    // `to_writer_all` rejects zero and two before it writes a byte.
    let one = Value::from_mapping([(Value::from("id"), Value::I64(1))])?;
    let mut output = Vec::new();
    assert!(toml::to_writer_all(&mut output, [one.clone(), one.clone()]).is_err());
    assert!(output.is_empty());
    toml::to_writer_all(&mut output, std::iter::once(&one))?;
    assert_eq!(toml::from_slice_all(&output)?, vec![one]);
    ```

=== "Python"

    ```python
    from yggdryl import toml

    # The root is a table, so an empty or comment-only document is an empty table.
    assert toml.loads("# nothing to see\n") == {}
    assert toml.dumps({}) == b""

    # There is no multi-document pair, only the single-document one.
    assert toml.load is toml.loads
    assert not hasattr(toml, "loads_all")
    assert not hasattr(toml, "dumps_all")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { toml } = require('yggdryl')

    // The root is a table, so an empty or comment-only document is an empty table.
    assert.deepEqual(toml.loads('# nothing to see\n'), {})
    assert.equal(toml.dumps({}).length, 0)

    // There is no multi-document pair, only the single-document one.
    assert.equal(toml.loadsAll, undefined)
    assert.equal(toml.dumpAll, undefined)
    ```

TOML has no document separator, so the Python and JavaScript facades ship none of the
`_all` pair that [json](json.md) and [yaml](yaml.md) carry. Rust keeps the plural entry
points so [text](text.md) can dispatch over every format uniformly, and they stay honest
about the format: `from_str_all` returns a one-element `Vec`, `Reader` is an
`ExactSizeIterator` with one item, and `to_writer_all` fails on a count other than one
rather than emitting something no TOML parser would read back.

## Laying out a dump

Every dump method has a `_with_formatting` companion taking one shared
[`Formatting`](text.md#laying-out-a-dump) value, and every existing method delegates to it with the
default - so no output changes a byte unless a caller asks. Formatting changes **bytes, never
meaning**: parsing any formatting of the same value yields an equal value, and dumping the same value
under the same formatting twice is byte-identical.

TOML's whitespace is largely insignificant, so this affects readability and nothing else - the
parse is identical either way. What an indent actually reaches is the indentation of **array**
entries, the one nested structure every version of the grammar lets span lines. Inline *tables* stay
on one line, because multi-line inline tables are a TOML 1.1 addition and a 1.0 reader would refuse
the document; `$yggdryl` envelope bodies stay flat for the same reason.

=== "Rust"

    ```rust
    use yggdryl::generic::Value;
    use yggdryl::text::Formatting;

    let value = Value::from_mapping([
        (Value::String("id".into()), Value::I64(1)),
        (Value::String("tags".into()), Value::from_sequence([Value::String("a".into())])),
    ])?;

    assert_eq!(yggdryl::toml::to_vec(&value)?, b"\"id\" = 1\n\"tags\" = [\"a\"]\n");
    assert_eq!(
        yggdryl::toml::to_vec_with_formatting(&value, Formatting::indented(2))?,
        b"\"id\" = 1\n\"tags\" = [\n  \"a\",\n]\n",
    );

    // Formatting changes bytes, never meaning.
    assert_eq!(
        yggdryl::toml::from_slice(
            &yggdryl::toml::to_vec_with_formatting(&value, Formatting::indented(2))?,
        )?,
        value,
    );
    ```

=== "Python"

    ```python
    from yggdryl import Field

    field = Field("id", "int64", nullable=False)

    # TOML has no null: an unset optional attribute is omitted, never faked.
    assert '"nullable" = false' in field.to_toml()
    assert "dictionary_id" not in field.to_toml()

    # Round-trip and idempotence hold for every setting.
    for indent in (None, 2):
        text = field.to_toml(indent=indent)
        assert Field.from_toml(text) == field
        assert field.to_toml(indent=indent) == text
    ```

=== "JavaScript"

    !!! note "Rust first"
        The layout option lands in the JavaScript facades once the core surface settles.


## Failures

=== "Rust"

    ```rust
    use yggdryl::{Error, Limits, Value, toml};

    let source = "ok = 0\nnested = { a = 1, a = 2 }\n";
    match toml::from_str(source).unwrap_err() {
        Error::Codec {
            format,
            position,
            reason,
        } => {
            assert_eq!(format, "toml");
            assert_eq!(position, source.rfind("a = 2").unwrap());
            assert!(reason.contains("duplicate"));
        }
        other => panic!("unexpected error: {other}"),
    }

    assert!(toml::from_str("big = 9223372036854775808").is_err());

    // Depth is measured on the wire projection, where a mapping TOML cannot key
    // costs four containers: the wrapper table, the body, the entry array, and
    // the pair array.
    let arbitrary = |depth: usize| {
        (0..depth).fold(Value::from("payload"), |value, _| {
            Value::from_mapping([(Value::Bool(true), value)]).expect("one key")
        })
    };
    let budget = Limits::new(48, 1024, 1024, 1);
    assert!(toml::validate_for_write_with_limits(&arbitrary(12), budget).is_ok());
    assert!(toml::validate_for_write_with_limits(&arbitrary(13), budget).is_err());

    // A temporal TOML spells itself is a leaf, so it costs no container at all.
    let day = Value::from_mapping([(Value::from("day"), Value::date(3_433))])?;
    assert!(toml::validate_for_write_with_limits(&day, Limits::new(1, 1024, 1024, 1)).is_ok());

    // A decimal costs the envelope table, its body, and the array that body holds.
    let price = Value::from_mapping([(Value::from("price"), Value::decimal(125, 2))])?;
    assert!(toml::validate_for_write_with_limits(&price, Limits::new(3, 1024, 1024, 1)).is_err());
    assert!(toml::validate_for_write_with_limits(&price, Limits::new(4, 1024, 1024, 1)).is_ok());

    // That check runs before anything is written.
    assert_eq!(toml::MAX_PARSER_DEPTH, 64);
    let mut output = Vec::new();
    assert!(toml::to_writer(&mut output, &arbitrary(64)).is_err());
    assert!(output.is_empty());
    ```

=== "Python"

    ```python
    from yggdryl import toml

    try:
        toml.loads("ok = 0\nnested = { a = 1, a = 2 }\n")
    except ValueError as error:
        assert "toml" in str(error)
        assert "duplicate" in str(error)
    else:
        raise AssertionError("duplicate keys must be rejected")

    try:
        toml.loads("big = 9223372036854775808")
    except ValueError:
        pass
    else:
        raise AssertionError("an out-of-range integer must be rejected")

    # Depth is checked before anything is written.
    deep = None
    for index in range(32):
        deep = {index: deep}
    try:
        toml.dumps(deep)
    except ValueError as error:
        assert "hard limit" in str(error)
    else:
        raise AssertionError("an over-deep value must be rejected")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { toml } = require('yggdryl')

    assert.throws(() => toml.loads('ok = 0\nnested = { a = 1, a = 2 }\n'), /duplicate/i)
    assert.throws(() => toml.loads('big = 9223372036854775808'), /toml/i)

    let deep = { value: 1 }
    for (let index = 0; index < 49; index += 1) deep = { nested: deep }
    assert.throws(() => toml.dumps(deep), /depth/i)
    ```

Every decode failure is one `Error::Codec` carrying `format: "toml"`, the byte offset in
the original input, and a reason. The offset points into the source the caller handed
over, so a duplicate key reports the second occurrence rather than the start of the table
that holds it.

Encoding measures depth on the wire projection, not on the source value, because a value
with no TOML syntax gains an envelope table and a body table on the way out - and a
mapping TOML cannot key gains an entry array and a pair array on top, so the 13th nesting
above overflows a budget of 48 that the `Value` itself is nowhere near. An envelope
whose payload is an array rather than one scalar, which is how a unit and a count and a
coefficient and a scale travel, is one container more again, so a decimal or a duration
costs three levels where a null costs two. A value TOML spells itself costs none, which
now includes every temporal that fits TOML's own date-time syntax. The hard ceiling is
`MAX_PARSER_DEPTH`, so a document this library writes is one it can read back, and the
budget that accepts a value on the way out is the same budget that accepts it on the way
back. `validate_for_write` is that check on its own, and every write runs it before
touching the destination, so a rejected value never emits a byte.

## Placeholders

`loads` resolves Jinja-*style* `{{ NAME }}` placeholders when a caller asks it to - the grammar and
the security notes are on the [structured text](text.md#jinja-style--placeholders) page. TOML
requires typed values, so a placeholder already has to sit inside a quoted string; it reaches table
values and table keys alike, and a scalar that is exactly one placeholder adopts the resolved
value's own type rather than staying text:

=== "Rust"

    ```rust
    use yggdryl::text::{Format, Loading, Placeholders};
    use yggdryl::Value;

    let placeholders = Placeholders::new()
        .with_variable("HOST", Value::from("db.internal"))
        .with_variable("PORT", Value::I64(5432));
    let loading = Loading::new().with_placeholders(placeholders);

    let document = "[database]\nhost = \"{{ HOST }}\"\nport = \"{{ PORT }}\"\n";
    let value = yggdryl::text::from_str_with(document, Format::Toml, &loading)?;
    let database = value.get_key_str("database").expect("the table");
    assert_eq!(database.get_key_str("host").and_then(Value::as_str), Some("db.internal"));
    assert_eq!(database.get_key_str("port"), Some(&Value::I64(5432)));
    ```

=== "Python"

    ```python
    from yggdryl import toml

    document = '[database]\nhost = "{{ HOST }}"\nport = "{{ PORT }}"\n'
    value = toml.loads(document, placeholders={"HOST": "db.internal", "PORT": 5432})
    assert value["database"] == {"host": "db.internal", "port": 5432}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { toml } = require('yggdryl')

    const document = '[database]\nhost = "{{ HOST }}"\nport = "{{ PORT }}"\n'
    const value = toml.loads(document, {
      placeholders: { HOST: 'db.internal', PORT: 5432 },
    })
    assert.deepEqual(value.database, { host: 'db.internal', port: 5432 })
    ```

<!-- notebooks: generated by scripts/build_docs_notebooks.py -->

## Notebooks

Every example on this page, as a notebook generated from these blocks and
shipped unexecuted:
[Rust](notebooks/toml-rust.ipynb){ download },
[Python](notebooks/toml-python.ipynb){ download },
[JavaScript](notebooks/toml-javascript.ipynb){ download }.

<!-- /notebooks -->
