---
name: yggdryl-types
description: Declare yggdryl types and values in Rust, Python and Node.js - parse DataType expressions (Arrow, SQL, Hive, Spark, FIX spellings), build and edit Field schemas (non-null Struct root, metadata, PARQUET:field_id, comment, protocol views, FIELD:enum) and read values through DataType.scalar / Field.scalar into Scalar. Use when choosing a column type (decimal, timestamp/datetime64 zone, string or bytes leaf, uuid, geometry/geography WKB, ccy/isin codes, an enumerated StringEnum column, serie/map/union), declaring a schema (Field::new, Field(...), new Field, yggdryl.int64 / fields.int64, @scalar dataclasses, into_field / intoField), adding, replacing or removing a column (set_field, remove_field, unnest_fields), an Arrow/pyarrow schema in or out (from_arrow_schema), converting values (Scalar.from_ / Scalar.from, as_py / asJs) or merging, diffing and walking schemas by path.
---

# Types: DataType, Field, Scalar

The type layer is three values and nothing else. `DataType` is a shape (no
name, no nullability, no metadata). `Field` is a `DataType` plus a name,
nullability and `<SCHEME>:<property>` metadata - and a **non-null Struct
`Field` is the only schema** there is. `Scalar` is one value, one variant per
physical width. Outside data is resolved **once** at a boundary into one of
the three - a type expression parsed, a host value read through
`DataType.scalar` / `Field.scalar` - and everything past that boundary
carries the proof instead of re-checking it. The Rust core owns every rule;
Python and JavaScript are native views of the same values, so a spelling, a
refusal and an error message are the same in all three.

A column of many values is a `Serie`, not a list of `Scalar`s: see
`yggdryl-arrow`. Install and cross-language conventions: `yggdryl`.

## Choose the door

| Task | Rust | Python | JavaScript |
| --- | --- | --- | --- |
| parse a type expression | `DataType::from_str("decimal(18,4)")?` | `DataType("decimal(18,4)")` (also a `str`/`int` hint or a `pa.DataType`) | `DataType.from('decimal(18,4)')` |
| canonical text, identity | `to_string()`, `id()`, `kind()` | `str(t)`, `t.id`, `t.kind` | `t.toString()`, `t.id`, `t.kind` |
| decimal by precision | `DataType::decimal(18, 4)?` | `DataType.decimal(18, 4)` | `fields.decimal(name, 18, 4)` (no `DataType.decimal`) |
| time width by unit | `DataType::time(TimeUnit::Millisecond)?` | `DataType.time("ms")` | `DataType.time('ms')` |
| zoned instant | `DataType::datetime64(TimeUnit::Nanosecond, Timezone::UTC)?` | `DataType('datetime64(ns,"UTC")')`, `yggdryl.datetime64(name, "ns", "UTC")` | `fields.datetime64(name, 'ns', 'UTC')` |
| string / bytes leaf | `DataType::sized_utf8(32)?`, `fixed_ascii(4)?`, `fixed_binary(16)?` | `DataType.string(charset=, bound=)`, `DataType.fixed_ascii(4)`, `DataType.bytes(bound=16)` | `DataType.string({ charset, max })`, `DataType.fixedAscii(4)`, `DataType.bytes({ max: 16 })` |
| one field | `Field::new("px", dtype, false)`, `dtype.required_field("px")` | `Field("px", "float64", nullable=False)`, `yggdryl.float64("px", nullable=False)` | `new Field('px', 'float64', false)`, `fields.float64('px', { nullable: false })` |
| field with metadata | `Field::from_parts(name, dtype, nullable, [(k, v)])?` | `Field(name, dtype, metadata={k: v})` | `new Field(name, dtype, nullable, { k: v })` |
| parse a field | `Field::from_str("px float64 NOT NULL")?` | `Field.from_str(...)` | `Field.from(...)` |
| a schema | `DataType::from(StructType::from_fields([..])?).required_field("row")` | `yggdryl.struct("row", [..], nullable=False)` | `fields.struct('row', [..], { nullable: false })` |
| schema from a class | `StructType` + typed leaves (`Int64Field::unit`) | `@scalar` class, `Class.into_field()`, `field(obj)` | `static get intoStructField()`, `intoField(Class)` |
| check a schema root | `root.validate_struct_root()?` | `root.validate_struct_root()` | no `validateStructRoot`: check `f.dtype.id === 'struct' && !f.nullable` (only `intoField(Class)` checks a class's `intoStructField`) |
| a value under a type | `field.scalar(v)?`, `dtype.scalar(v)?` | `field.scalar(v)`, `dtype.scalar(v)` | `field.scalar(v)`, `dtype.scalar(v)` |
| infer from a host value | `Scalar::from(7_i64)`, `Scalar::from_struct([..])?` | `Scalar.from_(v)`, `Scalar.from_struct({..})` | `Scalar.from(v)` |
| back to host | `as_i64()`, `as_str()`, `as_decimal()`, ... | `s.as_py()` | `s.asJs()` |
| what the type side cannot say | `Scalar::from_decimal(i256, scale)`, `Scalar::from_duration(n, unit, zone)?` | `Scalar.decimal(coef, scale)`, `Scalar.duration(n, unit)` | `Scalar.decimal(coefBigInt, scale)`, `Scalar.duration(n, unit)`, `Scalar.float(v, width)` |
| a named row, borrowed | `FieldRecord::new(&root, row)?`, `FieldScalar::new(&field, v)?` | Rust only | Rust only |
| child by name, position, path | `root["id"]`, `root[1]`, `get_field_by_path("a.b")`, `index_of("b")` | `root["id"]`, `root[1]`, `get_field_by_path("a.b")`, `index_of("b")` | `root.field('id')`, `getFieldAt(1)`, `getFieldByPath('a.b')`, `indexOf('b')` |
| add, replace or remove a column | `root.set_field("venue", f)?` (an unknown name appends, a known one replaces in place), `set_field_by_path("a.b", f)?`, `remove_field("id")?` (returns it); `DataType::with_fields([..])?` (same arity) | `root["venue"] = f`, `del root["id"]` | `root.setField('venue', f)`, `setFieldByPath('a.b', f)`, `removeField('id')` |
| flatten or explode a schema | `unnest_fields()` (dotted leaf names), `explode_fields()` | `unnest_fields()`, `explode_fields()` | `unnestFields()`, `explodeFields()` |
| raw metadata | `insert_metadata(k, v)?`, `get_metadata(k)` | `field.metadata[k] = v` | `field.set(k, v)`, `field.get(k)` |
| reserved properties | `set_parquet_field_id(17)`, `set_comment(..)?` | `set_parquet_field_id(17)`, `set_comment(..)` | `setParquetFieldId(17)`, `setComment(..)` |
| one protocol's keys | `as_iceberg_mut().insert("doc", ..)?` | `field.iceberg["doc"] = ..` | `field.iceberg.set('doc', ..)` |
| an enumerated column (`FIELD:enum`) | `StringEnum::from_members("Side", [("BUY", "B"), ("SELL", "S")])?` + `Field::new("side", DataType::fixed_ascii(4)?, false).try_with_string_enum(&side)?`; `string_enum()?`; `StringEnum::from_logical_name("ccy")?` | `StringEnum("Side", {"BUY": "B", "SELL": "S"})` + `field.set_string_enum(side)`; `field.string_enum`; `StringEnum.from_logical_name("ccy")`; `yggdryl.enums.Ccy` / `Country` bases | `new StringEnum('Side', { BUY: 'B', SELL: 'S' })` + `field.setStringEnum(side)`; `field.stringEnum`; `StringEnum.fromLogicalName('ccy')` |
| compare, diff | `equals(&o, true)`, `show_diffs(&o, true, false)` | `equals(o, with_metadata=False)`, `show_diffs(o)` | `equals(o, false)`, `showDiffs(o)` |
| merge two schemas | `a.merge_with(&b, true)?` | `a.merge_with(b)` | `a.mergeWith(b)` |
| stable value hash | `stable_hash()` | `stable_hash()` | `stableHash()` (a `bigint`) |
| schema as a document | `into_json()?` / `Field::from_json`, YAML, TOML | `into_json()` / `from_json`, `into_dict`, YAML, TOML | `toJSON()` / `Field.fromJSON` (JSON only) |
| one value as bytes | `into_value_bytes()`, `Scalar::decode_value_bytes(&b)?` | `into_value_bytes()`, `Scalar.from_value_bytes(b)`, `pickle` | `intoValueBytes()`, `Scalar.fromValueBytes(b)` |
| Arrow schema in and out | `Field::from_arrow_field(&f)?`, `into_arrow_field()?` | `Field.from_arrow(f)`, `Field.from_arrow_schema(s, name=)`, `into_arrow()`, `into_arrow_schema()` | schemas cross with batches (`yggdryl-arrow`): `Serie.fromArrowBatch(batch).field`; `DataType.fromArrow(t)` / `Field.fromArrow(f)` read only Arrow JS's text, so a field loses `nullable: false` and its extension |
| canonical default | `default_value()?` | `default_scalar()` | `defaultJSValue()` |
| engine compatibility | `into_scheme_compat(&Scheme::SPARK)?` | `into_scheme_compat("spark")` | `intoSchemeCompat('spark')` |

Every spelling the grammar reads - Arrow, SQL, Hive, Spark and FIX names, the
string and byte leaves, the legacy `list` words - is in
[references/spellings.md](references/spellings.md).

## Rules for fast, correct use

1. **Resolve the type once, outside the loop.** Parse a `DataType`/`Field`
   (or build it from a factory) at the boundary and reuse it; a per-row
   `from_str`, a per-row `Field(...)` or a per-row schema lookup is the
   defect. Cloning a `DataType` never allocates, and children are shared.
2. **Values enter only through `DataType.scalar` / `Field.scalar`.** That one
   door narrows an integer to its width, restates a decimal at its scale and a
   temporal at its unit, trims a fixed ASCII slot, applies nullability and
   canonicalizes a row. Never pre-cast through pyarrow, Arrow JS, `Number()`,
   `float()` or `str()`: those do not know the rules and pick another reading.
3. **Name the width, unit, scale and zone on the type, not the value.**
   `time32(ms)`, `datetime64(ns,"UTC")`, `decimal(18,4)`, `sized_utf8(32)`.
   The `Scalar` statics exist only for what the type side cannot say: a
   decimal coefficient, a duration whose width follows its count, and in
   JavaScript an integral float.
4. **A non-null Struct `Field` is the schema.** There is no schema class and
   no second row type. A row is the ordered sequence in declaration order;
   named input (a record, a dataclass, a JS object) canonicalizes to it, and a
   child it does not name takes that child's default (at `Field.scalar` /
   `DataType.scalar` only; record writers - `overwrite_records`,
   `overwriteRecords` - refuse a row missing a required child). `validate_struct_root`
   refuses a nullable root.
5. **Metadata is `<SCHEME>:<property>` text on the one field map.** Typed
   accessors (`parquet_field_id`, `comment`, `location`, `display`) and the
   protocol views (`iceberg`, `digest`, `partition`, ...) read and write that
   same map; keep no parallel dict. Every write validates first and a failed
   one leaves the field unchanged.
6. **Subscripting a field reaches a child, never metadata.** `field["x"]` is
   the child `x`; metadata is `field.metadata["x"]` (Python) or
   `field.get('x')` (JavaScript). A path string is parsed once by `FieldPath`:
   `a.b` is two levels, `"a.b"` quoted is one child named `a.b`.
7. **Equality and hashing are by value, across widths.** `int32 7 ==
   uint8 7`, `10.50 == 10.5`, and `stable_hash` is the same number in all
   three languages. A naive and a zoned instant are different values; a code
   and a plain string of the same bytes are different values.
8. **Branch on identity strings, not on classes.** `DataType.kind` and
   `Scalar.family` name the family (`integer`, `temporal`, `text`, `code`,
   `nested`, ...); `id` names the exact leaf (`time32`, `decimal128`);
   `Scalar.kind` names the width tag (`i32`, `d128`, `sized_ascii`).
9. **Arithmetic is checked and exact.** Overflow, division by zero, an inexact
   decimal quotient and an undefined operand pair are four distinct errors
   (JavaScript `err.code` `ERR_YGGDRYL_*`); integer `/` stays an integer at the
   shared width; `+` on text concatenates; nothing wraps or rounds silently.
10. **Borrowed views allocate nothing (Rust).** `as_*`, `as_fields`,
    `FieldScalar::infer` (borrows the prebuilt shared field),
    `FieldRecord::get*`/`names`/`iter`; `into_*` is the allocating form, and
    `into_arrow*` consumes - clone first.
11. **Defaults: nullable unless stated.** Python `nullable=True`, JavaScript
    `nullable: true`; Rust always states it (`Field::new(.., nullable)`,
    `nullable_field` / `required_field`). An omitted optional argument takes
    its signature default (`upscale=True`, `with_metadata=True`); `None` /
    `null` passed as a value is a null, which a required field refuses by path.
    An empty text cell entering a non-text column is null too.
12. **`merge_with` is the one promotion table.** `upscale=True` widens
    losslessly (int32+int64 -> int64, a one-sided child -> nullable);
    `False` meets at the tightest type. Decimal beside float, and two temporal
    families, are refused rather than re-encoded.
13. **JavaScript integers: pass `bigint` past 2^53.** `int64`/`uint64` values
    and decimal coefficients cross as `bigint`; a `Number` above
    `Number.MAX_SAFE_INTEGER` arrives as a float and an integer column refuses
    it. `asJs()` answers a `Number` when the value fits and a `bigint` when it
    does not.
14. **Python hashing locks a wrapper.** The first `hash(field)` freezes that
    `Field`/`DataType` wrapper against mutation; `copy.copy` gives a mutable
    one, and `stable_hash()` never locks. `@scalar` fields are frozen.
15. **Many values are a column, not a loop.** `Field.scalar` is for one value
    or one row; thousands of them land as a `Serie` under the field
    (`Serie.from_scalars` / Arrow doors, `yggdryl-arrow`), which proves the
    column once and shares buffers instead of building a `Scalar` per cell.

## Pitfalls

- Python `DataType("float64").scalar(100)` -> refused (`expected float64,
  got i64`): pass `100.0`. JavaScript cannot write an integral float, so
  `new DataType('float64').scalar(100)` is refused too: pass
  `Scalar.float(100)` or a non-integral number.
- A Python `float` into a decimal column is refused (`expected unscaled
  decimal integer, got f64`): pass `Decimal("12.5")`, the text `"12.5"`, or an
  `int`. Same in JavaScript: pass `'12.5'` or a `bigint`.
- A Python `dict` is a **map**, not a row: `root.scalar({"id": 7})` is refused
  (`expected struct sequence, got map`). Pass a list in declaration order, a
  dataclass instance, or `Scalar.from_struct({...})`. A JavaScript plain
  object *is* a record and reads as a row; a JS `Map` is a map.
- Bare `decimal` is the fixed leaf (38 digits at scale 18, extension
  `yggdryl.decimal`), not `decimal(p,s)`; `DataType::DECIMAL` in Rust is its
  `decimal128(38,18)` storage, a different datatype.
- Bare `variant` is the Parquet Variant datatype; `variant(a:int64,b:utf8)`
  is dense-union sugar. The parenthesis is the whole difference.
- `list<int64>` still parses but displays `serie(...)`: never compare type
  strings against `list`. Compare `DataType` values, or `id == "serie"`.
- `utf8(32)` is `sized_utf8(32)` (a maximum); `char(8)` is `fixed_utf8(8)`
  (NUL-padded width); `large_utf8(64)` and `duration(ms)` are refused. A bound
  counts **bytes**: `sized_utf8(4)` refuses `Grüß`.
- `timestamp` is `datetime64(us)` (naive wall clock); `timestamp_ltz` is
  `datetime64(us,"UTC")`. A naive `datetime` into a zoned column, or an
  offset-carrying text into a naive one, is refused.
- A code is not a string: `ccy` is its own datatype (`kind == "code"`,
  `string_parameters is None`), not `fixed_ascii(3)`; `isin`, `cusip`,
  `sedol`, `figi` check their digit; these four and `bbg`, `ric` have no
  default value (`default_scalar()` raises), so a record that omits such a
  required child is refused rather than defaulted - make the child nullable
  or always supply it.
- A Python `uuid.UUID` passed to `Scalar.from_` infers as text: declare the
  column `uuid` and read through it. A JavaScript `Date` is
  `datetime64(ms,"UTC")` and a `date32` column refuses it.
- A `StringEnum` (`FIELD:enum`) is only accepted on a `fixed_ascii(n)` field
  with n <= 16 or on a registered code; on `utf8` it is refused ("at most 16
  bytes"). It is a declared vocabulary, not a validator: `field.scalar("X")`
  is accepted on a `Side` enum field. Check membership yourself
  (`side.get_member(v)` / `getMember(v)` answers the member name or none)
  when non-members must fail.
- JavaScript `asJs()` on a decimal (and on values with no JS spelling) answers
  the `Scalar` itself: read `unscaled`/`scale`, or `toString()`.
- `DataType.from_arrow(extension_type)` loses the extension name (a bare Arrow
  datatype carries no metadata): import the **field** to keep `ccy`, `uuid`,
  `decimal`, `version` identity.
- JavaScript `Field.fromArrow(arrowJsField)` parses the field's `toString()`:
  `c: Utf8` not-null under `yggdryl.ccy` comes back as a nullable `utf8`.
  Take the schema from `Serie.fromArrowBatch(batch).field` instead.
- Python has no `DataType.decimal128`/`DataType.index_of`: exact decimal widths
  are field factories (`yggdryl.decimal128(name, p, s)`) and child positions
  are `Field.index_of`. JavaScript has no `DataType.decimal`.
- `DataType.kind` is the family: `DataType.time("ms").kind` is `temporal`
  and the leaf is `id == "time32"`. `Scalar.kind` is a width tag (`i64` for a
  bare Python `int`, `d128`); the value's family is `Scalar.family`.

## Language references

- Rust: [references/rust.md](references/rust.md) - read when writing Rust.
- Python: [references/python.md](references/python.md) - read when writing
  Python, including `@scalar` dataclasses and `Annotated` options.
- JavaScript: [references/javascript.md](references/javascript.md) - read
  when writing Node.js, including `fields.*` and `bigint` handling.
- Spellings: [references/spellings.md](references/spellings.md) - every
  accepted type expression, alias and logical name, and the "one number, one
  leaf" rule.

## Deeper

- Overview and family index: https://platob.github.io/yggdryl/types/
- Core: [DataType](https://platob.github.io/yggdryl/types/datatype/),
  [Field](https://platob.github.io/yggdryl/types/field/),
  [Scalar](https://platob.github.io/yggdryl/types/scalar/),
  [Paths](https://platob.github.io/yggdryl/types/paths/),
  [Protocol metadata](https://platob.github.io/yggdryl/types/protocol/)
- Families: [numeric](https://platob.github.io/yggdryl/types/numeric/),
  [decimal](https://platob.github.io/yggdryl/types/numeric/decimal/),
  [temporal](https://platob.github.io/yggdryl/types/temporal/),
  [time zone](https://platob.github.io/yggdryl/types/temporal/timezone/),
  [strings & bytes](https://platob.github.io/yggdryl/types/text/),
  [string](https://platob.github.io/yggdryl/types/text/string/),
  [codes](https://platob.github.io/yggdryl/types/codes/),
  [nested](https://platob.github.io/yggdryl/types/nested/),
  [union](https://platob.github.io/yggdryl/types/nested/union/),
  [geospatial](https://platob.github.io/yggdryl/types/geospatial/)
- Single types: [UUID](https://platob.github.io/yggdryl/types/uuid/),
  [Version](https://platob.github.io/yggdryl/types/version/),
  [media types](https://platob.github.io/yggdryl/types/mediatype/),
  [variant](https://platob.github.io/yggdryl/types/variant/),
  [value stream](https://platob.github.io/yggdryl/types/value-stream/)
- Sibling skills: `yggdryl-arrow` (a column of values, casts, pyarrow /
  Arrow JS), `yggdryl-records` (schemas applied to files), `yggdryl-documents`
  (JSON/YAML/TOML/XML values under a field), `yggdryl-expressions`
  (`FieldPath` in selectors and filters), `yggdryl-hashing` (`DIGEST:`
  holders, row digests).
