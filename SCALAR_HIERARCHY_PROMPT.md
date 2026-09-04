# Scalar hierarchy brief

Replace the flat 30-variant `Scalar` enum with a three-tier hierarchy: one root
enum over families, one enum per family over its physical widths, and one
concrete final struct per representation, each reachable through a trait so
generic code monomorphizes instead of matching. Rename
`DataType::Timestamp` to `DataType::DateTime64` so the schema side spells the
value side's name.

Follow `AGENTS.md`. This is a semantic change and is **not** part of
`REORGANIZATION_PROMPT.md`, whose invariant is that the test count never moves.
Land the reorganization first: this brief assumes `types/<family>/scalars.rs`
already exists.

## Outcome

- `Scalar` is an enum over families, one variant per value family.
- Each family is an enum over its widths: `Integer`, `Floating`, `Decimal`,
  `Temporal`, `Text`, `Ascii`, `Binary`, `Geospatial`, `Nested`. No `Scalar`
  suffix — see *Naming law*.
- Every family owns a matching datatype enum, marker set, and value enum — see
  *Symmetry law*. `Scalar::dtype()` becomes exact for every value.
- Each width is a concrete final struct: `Int32`, `Decimal128`, `DateTime64`,
  `Date32`. Small, `Copy` where the payload allows, `repr(transparent)` where
  it is one field.
- `ScalarValue` (a leaf), `ScalarFamily` (a family enum), and one trait per
  family (`TemporalValue`, `IntegerValue`, …) are the drill-down contract.
- `DataType::Timestamp(TimeUnit, Option<Timezone>)` becomes
  `DataType::DateTime64 { unit: TimeUnit, timezone: Timezone }`.
- `Scalar` stays 48 bytes. `DateTime64` is 16 bytes and `Copy`.

## Non-goals

- No new datatype, no new physical representation, no change to what a value
  can hold. Only how it is spelled and reached.
- No `Box<dyn ScalarValue>` anywhere. The traits exist for monomorphization;
  the enums exist for dynamic dispatch. Neither replaces the other.
- No second value tree. `AGENTS.md` forbids it and this is one tree with
  named floors, not two.

## Read first

- `AGENTS.md`, *Generic scalar* and *Public vocabulary*.
- `rust/src/generic/scalar.rs`, `generic/typed.rs`, `generic/temporal.rs`,
  `generic/enum_scalar.rs`, `generic/timezone/`.
- `rust/src/datatype/mod.rs` lines 80-95 for the `Timestamp` variant.
- `REORGANIZATION_PROMPT.md`, *Target tree — `types/`*.

## Measured constraint

Nesting is not free. Payload layouts modeled with `rustc -O` on 1.94, with
`SmolStr` as a 24-byte inline value and `I256` as `[u8; 32]`:

| Shape | `size_of::<Scalar>()` | align |
| --- | --- | --- |
| today, one flat enum | 48 | 16 |
| nested families, `Timezone(SmolStr)` inline | **64** | 16 |
| nested families, `Arc<Timezone>` | 48 | 16 |
| nested families, `Timezone` interned to 4 bytes | **48** | 16 |
| the above, plus decimals boxed and `Arc<str>` for text | 32 | 8 |

Two conclusions, both binding:

1. **Naive nesting costs 33% per value.** Each family enum carries its own
   discriminant and `Timezone(SmolStr)` at 24 bytes makes `Temporal` 48
   on its own, so the root grows to 64. Interning `Timezone` is a
   **prerequisite**, not an optimization — see the phase below.
2. **The 32-byte row is rejected.** It requires boxing `Decimal128` and
   dropping `SmolStr`'s inline storage. Decimal prices and short symbol strings
   are the hot values in this workload; paying an allocation per price to save
   16 bytes per scalar is the wrong trade, and it violates the `AGENTS.md` rule
   that core scalar creation does not allocate.

With `Timezone` interned, nesting is size-neutral at the root and a large win
per family:

| Type | Today | After |
| --- | --- | --- |
| `Scalar` | 48 | 48 |
| a temporal value | 48 (the whole `Scalar`) | `Temporal` 24 |
| a datetime | 48 (the whole `Scalar`) | `DateTime64` 16, `Copy` |
| a date | 48 (the whole `Scalar`) | `Date32` 12, `Copy` |
| an `i32` | 48 (the whole `Scalar`) | `Int32` 4, `Copy` |

That is the point of the drill-down: code that knows it holds a datetime moves
16 bytes, not 48, and never matches a discriminant.

## Naming law

Four suffixes, one meaning each. The bare name is always the value; a suffix
always means "paired with something".

| Spelling | Means | Examples |
| --- | --- | --- |
| bare name | **the value itself** | `Int32`, `DateTime64`, `Decimal128`, `Utf8`, `Temporal`, `Integer` |
| `*Type` | **the datatype**, and the zero-sized marker naming it | `DateTime64Type`, `Decimal128Type`, `TemporalType` |
| `*Scalar` | **a convenience builder**: `TypedScalar<K>` with the marker chosen | `Int64Scalar`, `DateTime64Scalar` |
| `*Field` | **a convenience builder**: `TypedField<K>` with the marker chosen | `Int64Field`, `DateTime64Field` |

**A scalar type is never suffixed `Scalar`.** The module path already says what
it is — `types::temporal::DateTime64` is unambiguous, and
`types::temporal::DateTime64Scalar` for the same value would be noise. The
suffix is reserved for the pairing aliases, whose whole job is terse
construction:

```rust
let value: DateTime64 = DateTime64::new(epoch, TimeUnit::Microsecond, Timezone::UTC)?;
let built = DateTime64Scalar::new(value)?;   // TypedScalar<DateTime64Type>
assert_eq!(built.dtype(), &value.dtype());
```

This reverses the scalar brief's earlier line about retiring the `*Scalar`
aliases: **keep all of them**, and keep the `*Field` aliases they mirror. They
are the builders, not duplicates of the value types. What is retired is only
the *marker* spelling they used to point at, which gains the `Type` suffix.

Consequences to apply everywhere:

- Family enums lose the suffix: `Temporal`, `Integer`, `Float`, `Decimal`,
  `Geospatial`, `Nested`.
- `EnumScalar` is a value, so it becomes `Enum` in `types/enumeration.rs`.
- `TypedScalar<K>` and `arrow::StructScalar` already conform: both are
  pairings, which is what the suffix means.
- The `*Type` suffix on the datatype side becomes load-bearing rather than
  decorative — it is the only thing separating `types::temporal::Temporal`
  (the value family) from `types::temporal::TemporalType` (the datatype
  family). Do not drop it there.
- Traits take a prefix or a `*Value` / `*Family` suffix and are unaffected:
  `ScalarValue`, `ScalarFamily`, `TemporalValue`, `IntegerValue`.

Sweep at the end: every remaining `*Scalar` name in the tree is either a
pairing builder or a bug.

## Symmetry law

**Every datatype family owns the same trio, under the same name.** A family is
one folder, and that folder answers all three questions:

| File | Owns | Named |
| --- | --- | --- |
| `dtypes.rs` | the datatype family enum, one variant per member | `<F>Type` |
| `fields.rs` | one zero-sized `FieldType` marker per member | `<Member>Type` |
| `scalars.rs` | the value family enum and one concrete leaf per member | `<F>`, `<Member>` |
| `casts.rs` | that family's arms of the Arrow cast planner | — |

No family is a datatype without a value, or a value without a datatype. The
eleven families:

| Folder | Members | `<F>Type` | `<F>` | `Scalar` variant |
| --- | --- | --- | --- | --- |
| `boolean/` | `Null`, `Boolean` | — | — | `Null`, `Boolean(Boolean)` |
| `integer/` | 8 | `IntegerType` | `Integer` | `Integer(Integer)` |
| `floating/` | 3 | `FloatingType` | `Floating` | `Floating(Floating)` |
| `decimal/` | 4 | `DecimalType` | `Decimal` | `Decimal(Decimal)` |
| `temporal/` | 8 | `TemporalType` | `Temporal` | `Temporal(Temporal)` |
| `text/` | `Utf8`, `LargeUtf8`, `Utf8View` | `TextType` | `Text` | `Text(Text)` |
| `ascii/` | `Ascii32`, `Ascii64`, `Ascii128` | `AsciiType` | `Ascii` | `Ascii(Ascii)` |
| `binary/` | `Binary`, `FixedSizeBinary`, `LargeBinary`, `BinaryView` | `BinaryType` | `Binary` | `Binary(Binary)` |
| `nested/` | 11 | `NestedType` | `Nested` | `Nested(Nested)` |
| `geospatial/` | `Geometry`, `Geography` | `GeospatialType` | `Geospatial` | `Geospatial(Geospatial)` |

Three exceptions, and only three. Each is stated in its module doc:

- **`boolean/` holds two families.** `Null` and `Boolean` are both
  parameterless, so neither earns a family enum, and neither would fill a
  folder. They share one.
- **`Integer` has ten members where `IntegerType` has eight.** Arrow has no
  128-bit integer, so `Int128` and `UInt128` are value-only and infer to
  `Decimal(n, 0)`. They cost the family 16 bytes extra (32 rather than 16);
  demote them behind a pointer only if a benchmark says the integer family's
  size is hot.
- **`Enum` has no datatype family.** An enum scalar is a representation choice
  over an integer or dictionary column — like nullability, it rides on the
  datatype the column already has. It stays a tier-1 variant with no
  `<F>Type` counterpart.

### This fixes a correctness bug

`Scalar::dtype()` is lossy today. `generic/inference.rs` maps:

```rust
Self::String(_) => Ok(DataType::Utf8),                       // LargeUtf8, Utf8View lost
Self::Bytes(_) | Self::Geospatial(_) => Ok(DataType::Binary), // width class and geospatial lost
```

A `LargeUtf8` value round-trips back as `Utf8`; a `BinaryView` value comes back
as `Binary`; and a geometry comes back as `Binary` under a comment claiming
*"the datatype model has no geospatial family yet"* — stale, since
`DataType::Geometry` and `DataType::Geography` both exist.

Once every member has its own leaf, `dtype()` reads the leaf and is exact for
every value. Delete the three lossy arms; do not keep a fallback.

### Symmetry is free

Measured with `rustc -O`, the full eleven-family set:

| Family | Bytes | | Family | Bytes |
| --- | --- | --- | --- | --- |
| `Boolean` | 1 | | `Temporal` | 24 |
| `Enum` | 2 | | `Binary` | 24 |
| `Floating` | 16 | | `Geospatial` | 24 |
| `Integer` | 32 | | `Nested` | 24 |
| `Decimal` | 48 | | `Text` / `Ascii` | 32 |

`size_of::<Scalar>()` stays **48**. `Decimal` at 48 and align 16 already sets
the ceiling, so `Text`, `Ascii`, and `Binary` becoming real families costs
nothing at the root while each becomes a small value in its own right.

### Consequences

- `DataTypeKind`'s variants become exactly the family list: `String` is renamed
  `Text`, `Ascii` splits out of it, and the seven nested kinds — `List`,
  `Struct`, `Union`, `Map`, `Dictionary`, `RunEndEncoded`, `Variant` — collapse
  into `Nested`, with `NestedType` answering which shape and `is_wrapper`
  moving onto it.
- `Floating` replaces today's `Float` enum, which already has the family shape
  and only needs the folder's name. Same absorption as `Integer`.
- `types::text::Text` and `media::text::Text<H>` are two types in two layers.
  That is the last remaining `Text` collision — `generic::Text` is already
  becoming `text::Structured` — and neither is re-exported bare at the crate
  root.

## Tiers

### Tier 0 — datatype markers

Unchanged in role, renamed for collision. The `FieldType` zero-sized markers in
`types/<family>/fields.rs` take a `Type` suffix so the bare name is free for the
value struct: `Int8Type`, `DateTime64Type`, `Utf8Type`, `Decimal128Type`.
`TypedField<K>` and `TypedScalar<K>` keep taking them.

### Tier 1 — `Scalar`

`types/scalar.rs`. One variant per value family.

```rust
#[non_exhaustive]
pub enum Scalar {
    Null,
    Boolean(Boolean),
    Integer(Integer),
    Floating(Floating),
    Decimal(Decimal),
    Temporal(Temporal),
    Text(Text),
    Ascii(Ascii),
    Binary(Binary),
    Geospatial(Geospatial),
    Nested(Nested),
    Enum(Enum),
}
```

A family with exactly one representation carries the concrete struct directly
and has no tier-2 enum — `Binary` and `Boolean` are the two.

Datatype families and value families are **not** 1:1, and the code says so
once: `Utf8`, `LargeUtf8`, `Utf8View`, and `Ascii32/64/128` are seven datatypes
over one `Utf8` value; `Dictionary` and `RunEndEncoded` are encodings of their
value type and have no value family at all. `types/<family>/scalars.rs` is
where a datatype family contributes its arms, which is why the two lists differ.

### Tier 2 — family enums

`types/<family>/scalars.rs`, each `#[non_exhaustive]`.

One variant per member of the datatype family, per the *Symmetry law*.

| Family enum | Variants | Concrete leaves |
| --- | --- | --- |
| `Integer` | 10 | `Int8` `Int16` `Int32` `Int64` `UInt8` `UInt16` `UInt32` `UInt64` `Int128` `UInt128` |
| `Floating` | 3 | `Float16` `Float32` `Float64` |
| `Decimal` | 4 | `Decimal32` `Decimal64` `Decimal128` `Decimal256` |
| `Temporal` | 8 | `Date32` `Date64` `Time32` `Time64` `DateTime64` `Duration32` `Duration64` `Interval` |
| `Text` | 3 | `Utf8` `LargeUtf8` `Utf8View` |
| `Ascii` | 3 | `Ascii32` `Ascii64` `Ascii128` |
| `Binary` | 4 | `Binary` `FixedSizeBinary` `LargeBinary` `BinaryView` |
| `Geospatial` | 2 | `Geometry` `Geography` |
| `Nested` | 11 | `List` `ListView` `FixedSizeList` `LargeList` `LargeListView` `Struct` `Union` `Variant` `Dictionary` `Map` `RunEndEncoded` |

Two family names already exist in the tree and are absorbed rather than
duplicated. `Float { F16, F32, F64 }` is today's copyable width view and is
exactly the floating family — rename it `Floating` for the folder and reuse it.
`Integer` is today a sign-and-magnitude struct; the family enum takes the name
and absorbs `is_negative`, `magnitude`, and `as_i128` as methods computed from
the variant, so cross-width comparison keeps its normalized key and one public
type disappears.

`Decimal32` and `Decimal64` values are `i32` and `i64` coefficients, not
narrowed `i128`. `Nested`'s eleven leaves share three storage shapes —
`Sequence`, `Mapping`, `Record` — so its leaves are newtypes over those three,
each naming its own datatype. That is what makes `Scalar::dtype()` exact for a
`ListView` or a `Map` instead of collapsing both to a sequence.

### Tier 3 — concrete final structs

One per physical representation. `Copy` wherever the payload is; `Eq`, `Ord`,
`Hash`, `Display`, serde on every one, with the same total order the flat enum
has today.

```rust
#[repr(transparent)] pub struct Int32(i32);
pub struct Decimal128 { coefficient: i128, scale: i8 }
pub struct DateTime64 { count: i64, unit: TimeUnit, timezone: Timezone }
pub struct Date32     { count: i32, unit: TimeUnit, timezone: Timezone }
#[repr(transparent)] pub struct Utf8(SmolStr);
#[repr(transparent)] pub struct Binary(Arc<[u8]>);
#[repr(transparent)] pub struct Sequence(Arc<[Scalar]>);
```

`Float16`, `Float32`, `Float64`, and `I256` already exist and keep their
definitions; they only gain the trait impls. `EnumScalar` keeps its definition
and is renamed `Enum` in `types/enumeration.rs`, per the naming law.

## Traits

```rust
/// A concrete final scalar value: one datatype variant, one representation.
pub trait ScalarValue:
    Sized + Clone + Debug + Display + Eq + Ord + Hash + Send + Sync + 'static
{
    /// The family enum this value is a variant of. `Self` when the family has
    /// exactly one representation.
    type Family: ScalarFamily;
    /// The zero-sized marker naming this value's datatype variant.
    type Type: FieldType;

    const ID: DataTypeId;
    const KIND: DataTypeKind;

    fn dtype(&self) -> DataType;
    fn into_family(self) -> Self::Family;
    fn from_family(family: &Self::Family) -> Option<&Self>;
    fn into_scalar(self) -> Scalar;
    fn from_scalar(value: &Scalar) -> Option<&Self>;
}

/// One family of scalar values, over its physical widths.
pub trait ScalarFamily: Sized + Clone + Debug + Display + Eq + Ord + Hash {
    const KIND: DataTypeKind;

    fn id(&self) -> DataTypeId;
    fn dtype(&self) -> DataType;
    fn into_scalar(self) -> Scalar;
    fn from_scalar(value: &Scalar) -> Option<&Self>;
}
```

One trait per family, implemented by every leaf of that family and by the
family enum itself, so the same call reads at either floor:

```rust
pub trait TemporalValue: ScalarValue {
    const FAMILY: TemporalFamily;
    const BIT_WIDTH: u8;
    fn count(&self) -> i64;
    fn unit(&self) -> TimeUnit;
    fn timezone(&self) -> Timezone;
    fn with_unit(self, unit: TimeUnit) -> Result<Self>;
    fn with_timezone(self, timezone: Timezone) -> Result<Self>;
}

pub trait IntegerValue:  ScalarValue { const SIGNED: bool; const BIT_WIDTH: u8;
                                       fn as_i128(&self) -> i128;
                                       fn from_i128(value: i128) -> Result<Self>; }
pub trait FloatingValue: ScalarValue { const BIT_WIDTH: u8; fn as_f64(&self) -> f64; }
pub trait DecimalValue:  ScalarValue { fn coefficient(&self) -> I256; fn scale(&self) -> i8;
                                       fn rescale(self, scale: i8) -> Result<Self>; }
pub trait TextValue:     ScalarValue { fn as_str(&self) -> &str; }
pub trait BytesValue:    ScalarValue { fn as_bytes(&self) -> &[u8]; }
pub trait NestedValue:   ScalarValue { fn len(&self) -> usize;
                                       fn children(&self) -> Children<'_>; }
```

The drill-down, all three floors, no allocation and no `dyn`:

```rust
let value: Scalar = /* … */;
let Scalar::Temporal(temporal) = &value else { return };   // tier 1 -> 2
let Temporal::DateTime64(at) = temporal else { return };        // tier 2 -> 3
let epoch: i64 = at.count();                                // tier 3, 16 bytes, Copy

fn round<T: TemporalValue>(value: T, unit: TimeUnit) -> Result<T> { value.with_unit(unit) }
```

`TemporalRef<'a>` is retired. It existed only because there was no concrete
struct to borrow; `Temporal` is that value now, and it implements the family
half of `TemporalValue` directly. `Scalar::as_temporal` returns
`Option<&Temporal>`.

## Renames

| Was | Becomes | Why |
| --- | --- | --- |
| `DataType::Timestamp(TimeUnit, Option<Timezone>)` | `DataType::DateTime64 { unit, timezone }` | matches `Scalar::DateTime64`, and drops an `Option` the value side never had |
| `DataTypeId::Timestamp` | `DataTypeId::DateTime64` | follows the variant |
| `field::temporal::Timestamp` (marker) | `types::temporal::DateTime64Type` | marker suffix; frees `DateTime64` for the value |
| `TimestampScalar` (builder alias) | `DateTime64Scalar` | follows the variant; the alias itself is kept |
| `TemporalRef<'a>` | — | retired; `&Temporal` |
| `Scalar::I8(i8)` … and 29 siblings | `Scalar::Integer(Integer::I8(Int8))` … | the hierarchy |
| `EnumScalar` | `Enum` | a value, so no `Scalar` suffix |
| `Integer` (sign/magnitude struct) | `Integer` (family enum) | the enum takes the name and the methods; the struct is deleted |
| `Float` (width view) | `Floating` (family enum) | already the right shape; rename to the folder and reuse |
| `Scalar::String(SmolStr)` | `Scalar::Text(Text)` / `Scalar::Ascii(Ascii)` | the width class stops being lost |
| `Scalar::Bytes(Arc<[u8]>)` | `Scalar::Binary(Binary)` | four members, exact `dtype()` |
| `DataTypeKind::String` | `DataTypeKind::Text` | the kind list becomes the family list |
| `DataTypeKind::{List, Struct, Union, Map, Dictionary, RunEndEncoded, Variant}` | `DataTypeKind::Nested` | `NestedType` answers the shape; `is_wrapper` moves onto it |

`Timezone` becomes non-optional on the datatype, exactly as it already is on
the scalar: `Timezone::NAIVE` is the explicit spelling for a wall-clock column,
and `DataType::datetime64(unit, timezone)` is the constructor. Delete the
`Option` handling at every call site rather than defaulting it.

`TypedScalar<K>` stays. A concrete struct fixes the *representation*, not every
datatype parameter — `Decimal128` carries a scale but not a precision, so the
value-plus-datatype pairing still has a job.

### Grammar

`datetime64` is the canonical spelling and what `Display` writes. `timestamp`
stays **accepted** as an input spelling and is not a compatibility alias: it is
the Arrow/SQL/Hive/Spark word, and `AGENTS.md` already requires the grammar to
accept foreign forms and display them as the core datatype. Add it to the same
foreign-spelling path that already accepts SQL forms; do not add a second
canonical name and do not add a deprecation.

## Prerequisite phase — intern `Timezone`

Required by the measurement above, and it is the change that makes every
temporal struct `Copy`.

`Timezone(SmolStr)` at 24 bytes becomes a 4-byte handle into the registry that
`timezone/registry.rs` already owns:

```rust
#[repr(transparent)]
#[derive(Clone, Copy, Eq, PartialEq, Hash)]
pub struct Timezone(NonZeroU32);
```

- `NAIVE` and `UTC` are const handles 1 and 2, so the two hot zones need no
  lookup and stay usable in `const` contexts.
- Fixed offsets occupy a reserved handle range encoding the offset in minutes,
  so `+05:30` interns without touching the registry map.
- Any other name is interned on first use; the registry never evicts, so a
  handle is valid for the process lifetime and `Copy` is sound.
- `Ord` and `Display` resolve the handle to its canonical name, keeping the
  stable name order the current `Ord` gives. `Eq` and `Hash` stay by handle,
  which is why interning must be canonicalizing: two spellings of one zone must
  produce one handle. Prove it with a test over the alias table.
- Serde stays the canonical name string. The handle is never serialized, never
  crosses the C Data Interface, and never appears in a binding.

Land this alone, with the full suite green, before any tier work.

## Phases

Each phase is one commit and ends green under default features and
`--features "parquet iceberg"`.

| # | Phase | Content |
| --- | --- | --- |
| 1 | intern `Timezone` | the prerequisite above; assert `size_of::<Timezone>() == 4` |
| 2 | `Timestamp` → `DateTime64` | the datatype variant, `DataTypeId`, the marker, constructors, grammar, Arrow import/export, serde, 309 Rust call sites and 172 in `python`/`node`/`docs` |
| 3 | traits | add `ScalarValue`, `ScalarFamily`, and the seven family traits with no enum change yet; implement them for the existing flat variants through temporary shims inside `types/` only |
| 4 | leaves | add the concrete structs per family in `types/<family>/scalars.rs`; give each its `ScalarValue` impl and its family trait impl |
| 5 | family enums | add the tier-2 enums; delete the phase-3 shims |
| 6 | root | reshape `Scalar` to the 11 family variants; rewrite every construction, match, and conversion in the crate |
| 7 | ports | Arrow scalar/array boundary, the cast planner, expression eval, Avro/JSON/YAML/TOML codecs, xxhash canonical feed, Iceberg scalar rendering |
| 8 | bindings | Python and JavaScript: the drill-down is Rust-only, so both expose the same flat conversions they do today and gain `family()` and `id()` accessors |
| 9 | docs | `docs/types.md` scalar section, the tier tables, `AGENTS.md` *Generic scalar*, `.api-inventory.txt`, `.api-bindings.txt` |

Phase 6 is the one that cannot be split; land phases 3-5 so that it is a
mechanical rewrite against traits that already compile.

## Verification

Per phase, the `REORGANIZATION_PROMPT.md` gate, plus:

```
cargo test --locked --manifest-path rust/Cargo.toml --workspace --all-targets --features "parquet iceberg"
cargo bench --locked --manifest-path rust/Cargo.toml --bench types --features "parquet iceberg"
```

Assertions to add and keep:

```rust
const _: () = assert!(size_of::<Timezone>() == 4);
const _: () = assert!(size_of::<Scalar>() == 48);
const _: () = assert!(size_of::<DateTime64>() == 16);
const _: () = assert!(size_of::<Temporal>() == 24);
const _: () = assert!(size_of::<Binary>() == 24);
const _: () = assert!(size_of::<Floating>() == 16);
const _: () = assert!(size_of::<Int32>() == 4);
```

`size_of::<Scalar>()` never exceeding 48 is the guard that keeps a later
variant from silently undoing the interning.

Behavioral invariants, each with a test:

- Total order, equality, and hash over `Scalar` are identical to the flat enum
  for every pair in the existing corpus. Order is the pair the reorganization
  cannot check for you: assert it against a recorded fixture, not against the
  new implementation.
- Canonical `Display` output is byte-identical except `timestamp` →
  `datetime64`.
- Serde round-trips are byte-identical except the same word.
- `Timezone` handle equality agrees with name equality across the full alias
  table, including fixed offsets and unregistered IANA names.
- `--no-default-features --lib` still compiles: the tiers and traits are
  Arrow-free; only `casts.rs` is gated.
- Allocation counts from `tests/allocations.rs` do not rise. Interning
  `Timezone` should lower them.

## Completion

- `Scalar` has 11 variants, each naming a family.
- Every family folder holds `dtypes.rs`, `fields.rs`, `scalars.rs`, and
  `casts.rs`, and the three names agree: `<F>Type`, `<F>`, `<Member>Type`.
- `Scalar::dtype()` is exact for every value: the three lossy arms in
  `generic/inference.rs` are gone with no fallback, and a round trip through
  `Scalar` preserves `LargeUtf8`, `BinaryView`, `Geometry`, and `Map`.
- `DataTypeKind::ALL` is the family list and nothing else.
- Every physical representation has one concrete struct implementing
  `ScalarValue` and its family trait.
- `rg -n "Timestamp" rust/src` matches only Arrow's own foreign type and the
  grammar's foreign-spelling table.
- `TemporalRef` is gone with no shim, and the `*Scalar` / `*Field` builder
  aliases are all present and pointing at `*Type` markers.
- `rg -n "(struct|enum) [A-Za-z0-9_]+Scalar" rust/src` matches only pairings.
- The size assertions above are in the tree and passing.
- `AGENTS.md` *Generic scalar* describes the three tiers and names
  `DateTime64` on both the datatype and the value.
