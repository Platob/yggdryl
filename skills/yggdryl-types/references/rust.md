# yggdryl-types in Rust

Everything is at the crate root: `use yggdryl::{DataType, Field, Scalar,
StructType, TimeUnit, Timezone};`. The type layer needs no feature; Arrow
interop is always compiled (`arrow_schema` types appear in the Arrow doors).
Errors are `yggdryl::Error` (`?` into `Box<dyn Error>`), typed and located.

## Parse a datatype once and read it back

`DataType::from_str` is an inherent method (no `FromStr` import needed) that
reads every Arrow, SQL, Hive, Spark and FIX spelling; `Display` is the one
canonical text and round-trips. `id()` is the exact leaf, `kind()` its family.

```rust
use yggdryl::{DataType, DataTypeId, DataTypeKind};

let amount = DataType::from_str("numeric(18, 4)")?;
assert_eq!(amount.to_string(), "decimal64(18,4)");
assert_eq!(amount.id(), DataTypeId::Decimal64);
assert_eq!(amount.kind(), DataTypeKind::Decimal);
assert_eq!(DataType::from_str(&amount.to_string())?, amount);

// Every dialect's spelling is one value: compare values, never text.
assert_eq!(DataType::from_str("bigint")?, DataType::Int64);
assert_eq!(DataType::from_str("list<int64>")?, DataType::from_str("array<int64>")?);
assert_eq!(DataType::from_str("timestamp")?.to_string(), "datetime64(us)");

// A family is the range of identifier bytes its kind owns.
assert!(DataTypeKind::Decimal.contains(amount.id()));
assert_eq!(DataTypeId::Time32.temporal_family(), Some("time"));

// A refusal names the byte where parsing stopped.
let error = DataType::from_str("large_utf8(64)").unwrap_err().to_string();
assert!(error.contains("at byte"), "{error}");
```

## Put the width, unit, scale and zone on the type

The family constructors validate once and pick the width: `decimal(p, s)` the
narrowest backing integer, `time(unit)` `time32` or `time64`. A string or byte
leaf is the whole declaration; a number on a plain leaf is a maximum.

```rust
use yggdryl::{Charset, DataType, StringType, TimeUnit, Timezone};

assert_eq!(DataType::decimal(9, 2)?, DataType::decimal32(9, 2)?);
assert_eq!(DataType::decimal(38, 4)?, DataType::decimal128(38, 4)?);
assert_eq!(DataType::time(TimeUnit::Millisecond)?, DataType::time32(TimeUnit::Millisecond)?);
assert!(DataType::time32(TimeUnit::Nanosecond).is_err());

let at = DataType::datetime64(TimeUnit::Nanosecond, Timezone::UTC)?;
assert_eq!(at.to_string(), "datetime64(ns,\"UTC\")");
assert_eq!(at.datetime_type().map(|leaf| leaf.timezone()), Some(Timezone::UTC));

let latin = DataType::from_str("string(windows-1252,32)")?;
assert_eq!(latin, DataType::sized_cp1252(32)?);
assert_eq!(latin.string_parameters(), Some(StringType::SizedCp1252String(32)));
assert_eq!(latin.charset(), Some(Charset::Cp1252));
assert_eq!(DataType::from_str("utf8(32)")?, DataType::sized_utf8(32)?);
assert_eq!(DataType::fixed_ascii(4)?.fixed_byte_width(), Some(4));
assert_eq!(DataType::from_str("binary(16)")?, DataType::sized_binary(16)?);

// Bare `decimal` is the fixed leaf; `DataType::DECIMAL` is only its storage.
assert_eq!(DataType::from_str("decimal")?, DataType::Decimal);
assert_eq!(DataType::DECIMAL, DataType::decimal128(38, 18)?);
assert_ne!(DataType::DECIMAL, DataType::Decimal);
```

## Build a field and a schema

A schema is a non-null struct `Field`: `DataType::from(StructType::from_fields
(..)?)` then `required_field(name)`. `Index` reaches a child by name or
position; `get_field_by_path` walks dots; `index_of` is an exact name match.

```rust
use yggdryl::{DataType, Field, StructType};

let trade = DataType::from(StructType::from_fields([
    DataType::Int64.required_field("id"),
    Field::new("price", DataType::from_str("decimal(18, 4)")?, false),
    DataType::from(StructType::from_fields([DataType::Mic.nullable_field("mic")])?)
        .nullable_field("venue"),
])?)
.required_field("trade");
trade.validate_struct_root()?;

assert_eq!(trade["id"].dtype(), &DataType::Int64);
assert_eq!(trade[1].name(), "price");
assert_eq!(trade["venue"]["mic"].dtype(), &DataType::Mic);
assert_eq!(trade.get_field_by_path("venue.mic").map(Field::name), Some("mic"));
assert_eq!(trade.index_of("price"), Some(1));
assert_eq!(trade.field_len(), 3);
assert!(trade.get_field_by_path("missing").is_none());

// The same field from text; a nullable root is a struct column, not a schema.
assert_eq!(Field::from_str("id int64 NOT NULL")?, trade["id"]);
assert!(trade.clone().with_nullable(true).validate_struct_root().is_err());
```

## Declare a schema from typed leaves

Rust has no class decorator: the typed field aliases (`Int64Field`,
`StringField`, `DecimalField`, ...) are `Field` variants, so building one is
the proof, `into_field` widens it and `FieldValue::from_field` narrows back.

```rust
use yggdryl::FieldValue as _;
use yggdryl::{DataType, DecimalField, DecimalType, Int64Field, StringField, StringType, StructType};

let id = Int64Field::unit("id", false);
let symbol = StringField::new("symbol", StringType::SizedAsciiString(12), true);
let price = DecimalField::new("price", DecimalType::Decimal128 { precision: 18, scale: 4 }, false);
assert_eq!(symbol.dtype(), &DataType::sized_ascii(12)?);
assert_eq!(price.dtype(), &DataType::decimal128(18, 4)?);

let root = DataType::from(StructType::from_fields([
    id.into_field(),
    symbol.into_field(),
    price.into_field(),
])?)
.required_field("Trade");
root.validate_struct_root()?;

// Narrowing is a match on the variant, never a check that could be skipped.
assert!(Int64Field::from_field(&root["id"]).is_some());
assert!(Int64Field::from_field(&root["symbol"]).is_none());
```

## Read a value through a column

`Field::scalar` / `DataType::scalar` is the one value door: it narrows to the
declared width, reads text under the type, applies nullability and refuses what
the type cannot hold.

```rust
use yggdryl::{DataType, Field, Scalar};

let qty = Field::new("qty", DataType::Int16, false);
let value = qty.scalar(42_i64)?;
assert!(matches!(value, Scalar::Int16(_)));   // narrowed to the column
assert_eq!(value.kind(), "i16");
assert_eq!(qty.scalar("42")?, value);         // text reads under the type

let refused = qty.scalar(Scalar::Null).unwrap_err().to_string();
assert!(refused.contains("non-nullable field received null"), "{refused}");
assert!(qty.scalar(70_000_i64).is_err());     // never wraps

// An integer is not a float: name the float, or declare the column.
assert!(DataType::Float64.scalar(100_i64).is_err());
assert_eq!(DataType::Float64.scalar(100.0_f64)?.kind(), "f64");
assert!(Field::new("note", DataType::utf8(), true).scalar(Scalar::Null)?.is_null());
```

## Build a row

A row is the ordered `Scalar` sequence of the struct's children. Named input
is a sorted `Scalar::from_struct` record; the field canonicalizes it to
declaration order and fills a child it does not name with that child's default.

```rust
use yggdryl::{DataType, Scalar, StructType};

let trade = DataType::from(StructType::from_fields([
    DataType::Int64.required_field("id"),
    DataType::utf8().nullable_field("symbol"),
])?)
.required_field("trade");

let named = Scalar::from_struct([("symbol", Scalar::from("AAPL")), ("id", Scalar::from(7_i32))])?;
let row = trade.scalar(named)?;
assert_eq!(row, Scalar::from_sequence([Scalar::from(7_i64), Scalar::from("AAPL")]));

// An ordered sequence is already the row.
assert_eq!(trade.scalar(vec![Scalar::from(7_i64), Scalar::from("AAPL")])?, row);
assert_eq!(
    trade.scalar(Scalar::from_struct([("id", Scalar::from(7_i64))])?)?,
    Scalar::from_sequence([Scalar::from(7_i64), Scalar::Null]),
);
assert_eq!(row.len(), 2);
assert!(Scalar::from_struct([("id", Scalar::from(1_i64)), ("id", Scalar::from(2_i64))]).is_err());
```

## Borrow a typed row or value (Rust only)

`FieldRecord` pairs one borrowed struct field with one `FieldScalar` per
child; `FieldScalar` pairs one borrowed field with the value its contract
answered. Nothing copies the schema per value, and `FieldScalar::infer`
borrows the prebuilt shared field of a leaf datatype.

```rust
use yggdryl::{DataType, Field, FieldRecord, FieldScalar, Scalar, StructType};

let trade = DataType::from(StructType::from_fields([
    DataType::Int64.required_field("id"),
    DataType::utf8().nullable_field("symbol"),
])?)
.required_field("trade");

let record = FieldRecord::new(
    &trade,
    Scalar::from_struct([("symbol", Scalar::from("AAPL")), ("id", Scalar::from(7_i32))])?,
)?;
assert_eq!(record.names().collect::<Vec<_>>(), ["id", "symbol"]);
assert_eq!(record["id"].as_i64(), Some(7));
assert_eq!(record.as_str("symbol"), Some("AAPL"));
assert!(record.get("SYMBOL").is_none()); // a name matches exactly, as index_of does
assert_eq!(
    record.into_scalar(),
    Scalar::from_sequence([Scalar::from(7_i64), Scalar::from("AAPL")]),
);

let price = Field::new("price", DataType::Int32, false);
let held = FieldScalar::new(&price, 7_i64)?;
assert!(matches!(held.value(), Scalar::Int32(_)));
assert_eq!(FieldScalar::parse_str(&price, "42")?.as_i64(), Some(42));
assert!(FieldScalar::new(&price, Scalar::Null).is_err());
assert_eq!(FieldScalar::infer(Scalar::from(7_i64))?.dtype(), &DataType::Int64);
```

## Infer a value and read it back

`Scalar::from` keeps a native value's width; the cross-width readers
(`as_i64`, `as_i128`, `as_f64`, `as_decimal`, `temporal_*`) answer `None`
rather than wrapping. Equality, order and `stable_hash` are by value across
widths, never across kinds.

```rust
use yggdryl::{DataType, DataTypeKind, Scalar, i256};

let seven = Scalar::from(7_i32);
assert_eq!(seven.kind(), "i32");
assert_eq!(seven.family(), DataTypeKind::Integer);
assert_eq!(Scalar::from(1.5_f64).kind(), "f64");
assert_eq!(Scalar::from("AAPL").kind(), "string");

// One number at two widths is one value and one stable hash.
assert_eq!(Scalar::from(7_u8), seven);
assert_eq!(Scalar::from(7_u8).stable_hash(), seven.stable_hash());
assert!(Scalar::from(1_i64) < Scalar::from(2_i64));
assert_ne!(Scalar::from(1_i32), Scalar::from(1.0_f64)); // kinds stay apart

assert_eq!(seven.as_i64(), Some(7));
assert_eq!(Scalar::from(-1_i64).as_u64(), None);
assert_eq!(Scalar::d128(1_250, 2).as_decimal(), Some((i256::from_i128(1_250), 2)));

// The datatype and the field a value names, inferred without a schema.
assert_eq!(Scalar::from(7_i64).dtype()?, DataType::Int64);
assert_eq!(Scalar::from(42_i64).inferred_scalar_field()?.name(), "value");
```

## Decimals, durations and checked arithmetic

`Scalar::from_decimal` and `Scalar::from_duration` pick the narrowest width
for a coefficient or a count - the two things the type side cannot state.
Arithmetic is `checked_*` (or the `Result` operator traits): exact or an error.

```rust
use yggdryl::{DataType, Decimal, Field, Scalar, TimeUnit, Timezone, i256};

let price = Scalar::d128(1_050, 2);
assert_eq!(price.into_decimal_utf8().as_deref(), Some("10.50"));
assert_eq!(price, Scalar::d128(105, 1)); // normalized equality
assert_eq!(Scalar::from_decimal(i256::from_i128(1_250), 2), Scalar::d128(1_250, 2));

let amount = Field::new("amount", DataType::decimal(10, 2)?, true);
assert_eq!(amount.scalar("12.5")?.decimal_unscaled_at(2), Some(1_250));
assert!(amount.scalar(12.5_f64).is_err()); // a float is inexact: refused

assert_eq!(Scalar::d128(1, 0).checked_div(&Scalar::d128(2, 0))?, Scalar::d128(5, 1));
assert!(Scalar::d128(1, 0).checked_div(&Scalar::d128(3, 0)).is_err()); // inexact
assert!(Scalar::from(1_i64).checked_div(&Scalar::from(0_i64)).is_err());
assert!(Scalar::from(i64::MAX).checked_add(&Scalar::from(1_i64)).is_err());
assert_eq!(Scalar::from(-1_i8).checked_add(&Scalar::from(2_u8))?, Scalar::from(1_i16));

// The fixed `decimal` leaf holds eighteen fractional digits.
let px: Decimal = "82.5".parse()?;
assert_eq!((px * Decimal::from_int(1_000)).to_string(), "82500");
assert_eq!(DataType::Decimal.scalar(Scalar::d128(825, 1))?, Scalar::from(px));

let long = Scalar::from_duration(i64::from(i32::MAX) + 1, TimeUnit::Second, Timezone::NAIVE)?;
assert!(matches!(long, Scalar::Duration64(_)));
```

## Temporal values and zones

A datetime column carries its unit and zone; ISO text or a bare count at the
column's unit reads into it. Text with no offset is refused by a zoned column.

```rust
use yggdryl::{DataType, Field, Scalar, TimeUnit, Timezone};

let at = Field::new("at", DataType::datetime64(TimeUnit::Nanosecond, Timezone::UTC)?, false);
let value = at.scalar("2024-01-02T00:00:00Z")?;
assert_eq!(value.temporal_count(), Some(1_704_153_600_000_000_000));
assert_eq!(value.temporal_unit(), Some(TimeUnit::Nanosecond));
assert_eq!(value.temporal_timezone(), Some(Timezone::UTC));
assert_eq!(value.id().temporal_family(), Some("datetime"));
assert_eq!(at.scalar(1_704_153_600_000_000_000_i64)?, value);
assert!(at.scalar("2024-01-02T00:00:00").is_err()); // no offset, zoned column

let micros = Scalar::datetime64(1, TimeUnit::Microsecond, Timezone::UTC)?;
assert_eq!(micros.temporal_count_at(TimeUnit::Nanosecond), Some(1_000));
assert_ne!(micros, Scalar::datetime64(1, TimeUnit::Microsecond, Timezone::NAIVE)?);

assert_eq!(DataType::date32().scalar("1970-01-02")?, Scalar::date32(1));
assert_eq!(
    DataType::Timezone.scalar("Asia/Calcutta")?,
    DataType::Timezone.scalar("Asia/Kolkata")?,
);
```

## Strings, bytes, codes and identifiers

A string value keeps the leaf of its column but compares by its characters; a
bound counts stored bytes. A registered code is its own datatype and value,
never a string.

```rust
use yggdryl::{DataType, Scalar, Str, Uuid};

let bounded = DataType::sized_ascii(4)?;
let usd = bounded.scalar("USD")?;
assert_eq!(usd.as_str(), Some("USD"));
assert_eq!(usd.dtype()?, bounded);          // the value keeps its leaf
assert_eq!(usd, Scalar::from("USD"));       // equality reads the characters
assert!(bounded.scalar("EURO!").is_err());
assert!(DataType::sized_utf8(4)?.scalar("Grüß").is_err()); // six bytes

let slot = DataType::fixed_ascii(4)?.scalar("USD")?;
assert_eq!(slot, Scalar::FixedAsciiString(Str::new("USD"), 4));

let ccy = DataType::ccy();
assert_eq!(ccy.code_width(), Some(3));
assert!(ccy.string_parameters().is_none());
assert_eq!(ccy.scalar("USD")?.kind(), "ccy");
assert_ne!(ccy.scalar("USD")?, Scalar::from("USD")); // a code is not a string
assert!(DataType::Isin.scalar("US0378331006").is_err()); // check digit

let text = "01912d68-783e-7c9a-b1f2-0123456789ab";
let id = DataType::uuid().scalar(text)?;
assert_eq!(id, Scalar::Uuid(Uuid::from_bytes(text.to_uppercase().as_bytes())?));
assert_eq!(
    DataType::from_str("binary(2)")?.scalar(vec![1_u8, 2])?.as_bytes(),
    Some(&[1_u8, 2][..]),
);
```

## Nested values: serie, map, union, dictionary

Every nested constructor takes its child fields. A map value is built with
`from_mapping`; a union value is `[type_id, payload]` and a bare payload enters
the one member that accepts it; a dictionary cell is the decoded value.

```rust
use yggdryl::{DataType, DataTypeId, Field, Scalar};

let levels = DataType::serie(DataType::Float64.nullable_field("item")).nullable_field("levels");
assert_eq!(levels.dtype().id(), DataTypeId::Serie);
let two = levels.scalar(Scalar::from_sequence([Scalar::from(1.5_f64), Scalar::from(2.5_f64)]))?;
assert_eq!(two.len(), 2);

let lookup = Field::new("lookup", DataType::map_of(DataType::utf8(), DataType::Int64, false)?, true);
let map = lookup.scalar(Scalar::from_mapping([(Scalar::from("a"), Scalar::from(1_i64))])?)?;
assert!(matches!(map, Scalar::Map(_)));

let payload = Field::new(
    "p",
    DataType::dense_union([DataType::Int64.required_field("n"), DataType::utf8().nullable_field("t")])?,
    true,
);
assert_eq!(payload.scalar(7_i64)?, Scalar::from_sequence([Scalar::from(0_i64), Scalar::from(7_i64)]));
assert_eq!(payload.scalar("hi")?, Scalar::from_sequence([Scalar::from(1_i64), Scalar::from("hi")]));

let codes = Field::new("codes", DataType::dictionary(DataType::Int16, DataType::utf8())?, true);
assert_eq!(codes.scalar("AAPL")?.dtype()?, DataType::utf8());
assert!(DataType::dictionary(DataType::utf8(), DataType::utf8()).is_err()); // integer keys only
```

## Metadata and protocol properties

Metadata is one `<SCHEME>:<property>` map on the field. Typed accessors and
the `as_<protocol>` / `as_<protocol>_mut` views borrow it; every write
validates first and a failure leaves the field unchanged.

```rust
use yggdryl::{DataType, Field, Scheme, StructType, Url};

let mut price = Field::from_parts("price", DataType::decimal(18, 4)?, false, [("source", "feed")])?;
price.set_parquet_field_id(17);
price.set_comment("closing price")?;
price.set_location(Url::from_str("s3://warehouse/bars/data.arrow")?);
price.set_property(&Scheme::POSTGRES, "type", "numeric(18,4)")?;
price.as_iceberg_mut().insert("doc", "closing price")?;

assert_eq!(price.parquet_field_id()?, Some(17));
assert_eq!(price.get_metadata("PARQUET:field_id"), Some("17"));
assert_eq!(price.comment(), Some("closing price"));
assert!(price.location()?.is_some());
assert_eq!(price.get_property(&Scheme::POSTGRES, "type"), Some("numeric(18,4)"));
assert_eq!(price.get_metadata("ICEBERG:doc"), Some("closing price"));
assert_eq!(price.as_iceberg().get("doc"), Some("closing price"));
assert_eq!(price.get_metadata("source"), Some("feed"));

let row = DataType::from(StructType::from_fields([
    DataType::Int32.required_field("year"),
    DataType::Float64.nullable_field("px"),
])?)
.required_field("row")
.with_partition_fields(&["year"])?;
assert_eq!(row.partition_field_names().collect::<Vec<_>>(), ["year"]);
assert_eq!(row["year"].get_metadata("FIELD:partition"), Some("true"));
```

## Compare, diff and merge schemas

`equals` answers yes or no, `show_diffs` why (a lazy iterator); `merge_with`
is the one promotion table - `true` widens losslessly, `false` meets at the
tightest type.

```rust
use yggdryl::{DataType, Field};

let left = Field::from_parts("price", DataType::Float64, false, [("venue", "XPAR")])?;
let right = Field::from_parts("price", DataType::Float64, true, [("venue", "XNAS")])?;
assert!(!left.equals(&right, true));
assert!(left.equals(&Field::new("price", DataType::Float64, false), false));
assert_eq!(
    left.show_diffs(&right, true, false).collect::<Vec<_>>(),
    ["≠ $.nullable: false → true", "≠ $.metadata[\"venue\"]: \"XPAR\" → \"XNAS\""],
);
assert_eq!(left.show_diff(&left, true, true), "✓ equal");

let merged = DataType::from_str("struct<id:int32 not null,venue:utf8>")?
    .merge_with(&DataType::from_str("struct<id:int64 not null,px:float64>")?, true)?;
assert_eq!(merged["id"].dtype(), &DataType::Int64);
assert!(merged["px"].is_nullable()); // one-sided child arrives nullable
assert_eq!(DataType::Int32.merge_with(&DataType::Int64, false)?, DataType::Int32);
assert!(DataType::decimal(10, 2)?.merge_with(&DataType::Float64, true).is_err());
```

## Serialize a schema, move a value as bytes

One structural model backs JSON, YAML and TOML (`into_value` is the `Scalar`
under all three). A value is one self-describing byte stream that reads back
leaf for leaf, width included.

```rust
use yggdryl::{DataType, Field, Scalar};

let field = Field::from_parts("price", DataType::decimal(9, 2)?, false, [("venue", "XPAR")])?;
assert_eq!(Field::from_json(&field.clone().into_json()?)?, field);
assert_eq!(Field::from_yaml(&field.clone().into_yaml()?)?, field);
assert_eq!(Field::from_toml(&field.clone().into_toml()?)?, field);
assert!(format!("{field:#}").starts_with("price: decimal32(9,2), required"));

let value = Scalar::from(7_i32);
let bytes = value.into_value_bytes();
assert!(matches!(Scalar::decode_value_bytes(&bytes)?, Scalar::Int32(_)));
let row = Scalar::from_struct([("symbol", Scalar::from("AAPL")), ("size", Scalar::from(100_i64))])?;
assert_eq!(Scalar::decode_value_bytes(&row.into_value_bytes())?, row);

// A datatype casts on the way in and on the way out.
let wide = DataType::Int64.encode_value_bytes(&Scalar::from(7_i32))?;
assert_eq!(DataType::utf8().decode_value_bytes(&wide)?, Scalar::from("7"));
```

## Cross an Arrow schema

A `Field` crosses `arrow_schema::Field` losslessly, extension identity
included. A bare Arrow datatype has no metadata, so import the field to keep a
code, UUID or fixed decimal. `into_arrow_*` consumes: clone first.

```rust
use yggdryl::{DataType, Field, Scheme, TimeUnit, Timezone};

let venue = Field::new("venue", DataType::Mic, false);
let arrow = venue.clone().into_arrow_field()?;
assert_eq!(arrow.data_type(), &arrow_schema::DataType::Utf8);
assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.mic");
assert_eq!(Field::from_arrow_field(&arrow)?, venue); // identity kept
assert_eq!(DataType::from_arrow_datatype(arrow.data_type())?, DataType::utf8()); // lost

let stamp = DataType::datetime64(TimeUnit::Microsecond, Timezone::UTC)?;
assert_eq!(
    stamp.clone().into_arrow_datatype()?,
    arrow_schema::DataType::Timestamp(arrow_schema::TimeUnit::Microsecond, Some("UTC".into())),
);

// Engine rewrites and canonical defaults come from the same core.
assert_eq!(DataType::UInt8.into_scheme_compat(&Scheme::SPARK)?, DataType::Int16);
assert_eq!(DataType::utf8().default_value()?, yggdryl::Scalar::from(""));
```

## Gotchas in Rust

- `DataType::from_str`, `Field::from_str`, `Url::from_str` are inherent and
  return `yggdryl::Result`; the `FromStr` trait (`"decimal".parse()`) works too.
- `into_arrow_datatype`, `into_arrow_field`, `into_json`, `into_value`
  consume `self`: clone a value you still need.
- A hand-built enum (`DataType::Time32(TimeUnit::Nanosecond)`) can hold what
  no constructor produces; `validate()` and every Arrow projection refuse it.
  Use the constructors.
- `Scalar::from(7_i32)` stays `Int32`; a column narrows or widens it through
  `scalar`, and the readers never wrap (`as_u64` on a negative is `None`).
- `DataType::DECIMAL` is `decimal128(38,18)` storage, not `DataType::Decimal`.
- `Field::new` validates nothing; `Field::from_parts` validates metadata;
  `validate_struct_root` is the schema check.
- `FieldRecord` names match exactly (`index_of`); a dotted path never reaches a
  cell - walk `record["leg"].get(0)`.
- Protocol views are borrows (`as_iceberg`, `as_digest_mut`, ...): typed
  vocabulary lives there, never as a `Field` method.
