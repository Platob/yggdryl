# Notion hierarchy brief

Apply one shape to every notion in the crate: `DataType`, `Field`, `Holder`,
`Media`, `Coded`, `Uri`, `Digest`. `SCALAR_HIERARCHY_PROMPT.md` is the worked
instance for `Scalar`; this file states the rule once and says what each other
notion needs to satisfy it.

Land `REORGANIZATION_PROMPT.md` first. This brief and the scalar brief can land
in either order, except that `DataType::DateTime64` and the interned `Timezone`
come from the scalar brief and are assumed here.

`SCALAR_HIERARCHY_PROMPT.md`'s *Case law* and *Naming law* govern every name
introduced here. Case law first: a type, trait, or enum variant name never
contains an underscore, and a number attaches directly — so a new family enum
is `TemporalType`, never `Temporal_Type`, and a widened leaf is `Decimal256`,
never `Decimal_256`. Naming law second:
bare name is the value, `*Type` is the datatype, `*Scalar` and `*Field` are the
convenience builders. On this side the `*Type` suffix is load-bearing — it is
what separates `types::temporal::Temporal` (the value family) from
`types::temporal::TemporalType` (the datatype family).

## The rule

> Every notion has exactly three floors.
>
> 1. **A trait** — the contract. One method set, object-safe when a caller
>    needs `dyn`, otherwise generic so it monomorphizes.
> 2. **An enum** — runtime dispatch over the implementations the core ships,
>    nested by family where a family exists.
> 3. **Concrete final structs** — one per implementation. Small, `Copy` where
>    the payload allows, `repr(transparent)` where it is one field.
>
> A caller drills from the floor it has to the floor it needs, and pays for
> nothing below it.

## When a family tier is earned

The middle floor is not free and is not automatic. Add a family enum only when
**both** hold:

- the family has **two or more** members, or is certain to within the current
  scope; and
- the family carries behavior the root cannot express — a family trait with
  real methods, a uniform predicate, or a shape that repeats per member.

A family of one is a rename, not a tier. A family that only groups names is a
comment, not a tier.

The counter-example is measured: nesting `Scalar` naively takes it from 48 to
64 bytes, because every family enum adds a discriminant and the widest payload
sets the root's size. Check `size_of` before and after every tier you add.

## Measured constraints

`rustc -O` on 1.94, payload shapes modeled with `SmolStr` as 24 bytes inline
and `I256` as `[u8; 32]`.

| Value | Today | Interned `Timezone` | Tiered + interned |
| --- | --- | --- | --- |
| `DataType` | 32 | 24 | **24** |
| `Field` | 104 | 96 | **96** |
| `Scalar` | 48 | 48 | 48 (64 without interning) |

Two readings:

- **Tiering `DataType` is size-neutral.** The nested family already dominates
  at 24 bytes (an `Arc` plus a discriminant), so the small families collapse
  into the space that was already there. `IntegerType` is 1 byte,
  `FloatingType` 1, `TextType` 1, `DecimalType` 3, `DateTime64Type` 8,
  `TemporalType` 12 — every one `Copy`.
- **`Field` does not shrink from tiering, because its bulk is not the
  datatype.** It shrinks from moving state that does not belong to it: see
  *Field*, below.

## DataType

Same three tiers as `Scalar`, in `types/<family>/dtypes.rs`.

```rust
#[non_exhaustive]
pub enum DataType {
    Null,
    Boolean,
    Integer(IntegerType),
    Floating(FloatingType),
    Decimal(DecimalType),
    Text(TextType),
    Ascii(AsciiType),
    Bytes(BytesType),
    Temporal(TemporalType),
    Nested(NestedType),
    Geospatial(GeospatialType),
}
```

| Family enum | Members | Leaves |
| --- | --- | --- |
| `IntegerType` | 8 | parameterless variants |
| `FloatingType` | 3 | parameterless variants |
| `DecimalType` | 4 | `DecimalParams { precision: u8, scale: i8 }` |
| `TextType` | 3 | `Utf8`, `LargeUtf8`, `Utf8View` |
| `AsciiType` | 3 | `Ascii32`, `Ascii64`, `Ascii128` |
| `BytesType` | 4 | `FixedSizeBinary(i32)` carries its width; the family is `Bytes`, not `Binary`, because `Binary` is one of its own members |
| `TemporalType` | 8 | `DateTime64 { unit, timezone }`, `Time32(TimeUnit)`, … carried inline |
| `NestedType` | 11 | `List(Arc<Field>)`, `Struct(Fields)`, `Union(UnionFields, UnionMode)`, `Arc<DictionaryType>`, `Arc<MapType>`, `Arc<RunEndEncodedType>`, `Variant` |
| `GeospatialType` | 2 | `Geometry(Arc<GeospatialParams>)`, `Geography(…)` |

Parameters ride inline in the family enum's variants; a separate leaf struct
exists only where the parameters are shared, as `DecimalParams` is. That keeps
the `*Type` suffix free for the zero-sized `FieldType` markers in `fields.rs`,
which are `<Member>Type` — `Int32Type`, `DateTime64Type`. A family name is
never a member name, so `IntegerType` and `Int32Type` never collide.

`NestedType` is where the recursion lives and where it terminates: a
`StructType` holds `Fields`, each `Field` holds a `DataType`, and a branch ends
the first time it reaches a parameterless leaf such as `IntegerType::I32`.
Every other family is a leaf by construction — that is what makes the walk
finite without a depth counter, and the parser's recursion limit stays the
guard against a hostile *input*, not against the model.

Traits, mirroring the scalar brief:

```rust
pub trait DataTypeValue: Copy + Debug + Display + Eq + Ord + Hash {
    type Family: DataTypeFamily;
    const ID: DataTypeId;
    const KIND: DataTypeKind;
    fn into_dtype(self) -> DataType;
    fn from_dtype(dtype: &DataType) -> Option<Self>;
}

pub trait DataTypeFamily: Clone + Debug + Display + Eq + Ord + Hash {
    const KIND: DataTypeKind;
    fn id(&self) -> DataTypeId;
    fn into_dtype(self) -> DataType;
    fn from_dtype(dtype: &DataType) -> Option<&Self>;
}

```

**No per-family traits on the datatype side.** The family enums are 1-to-12
byte `Copy` values whose accessors — `unit()`, `timezone()`, `precision()`,
`children()` — are inherent methods on the enum and on each leaf. A trait would
abstract over nothing: the generic code that matters is `T: TemporalValue` on
the *value* side, and it reaches the datatype through `Self::Type`. `AGENTS.md`
allows an abstraction only when it removes real duplication; this one would
not, and it would also force an awkward second name beside `TemporalType`.

`DataTypeKind` becomes exactly the tier-1 discriminant and stops being a
hand-maintained parallel list. Derive it: `DataType::kind()` matches eleven
arms, not forty-seven. Delete every `matches!` over variant groups that
`DataTypeKind` now answers.

Its variants become the family list, per the *Symmetry law* in
`SCALAR_HIERARCHY_PROMPT.md`: `String` is renamed `Text`, `Ascii` splits out of
it, and the seven nested kinds collapse into `Nested` with `NestedType`
answering the shape. `is_wrapper` moves onto `NestedType`, where a wrapper is
what it describes.

## Field

**`Field` stays one struct. It does not become an enum.**

Two reasons, one measured and one contractual:

- A field's bulk is family-independent — name 24 bytes, metadata 8, Arrow cache
  16, nullability, dictionary state. An enum over families would duplicate all
  of it per variant, or push it behind a pointer that every access then chases.
  Tiering `Field` costs bytes and buys nothing: the family question is already
  one hop away at `field.dtype()`, which is now itself tiered.
- `AGENTS.md` forbids it outright: *"A non-null Struct `Field` is the only row
  schema. Do not add another row/schema class or schema accessor."* A
  `FieldEnum` over narrowed fields is a second schema class.

So `Field` gets the **trait floor** and the **narrowing floor**, and the enum
floor lives on the `DataType` it holds. That is the same shape as `Scalar`,
read correctly: the enum sits on the thing that has families, the traits sit on
the thing that carries them.

```rust
/// Everything field-shaped answers this, whether owned, narrowed, or borrowed.
pub trait FieldValue {
    fn name(&self) -> &str;
    fn dtype(&self) -> &DataType;
    fn nullable(&self) -> bool;
    fn metadata(&self) -> &Metadata;
    fn kind(&self) -> DataTypeKind { self.dtype().kind() }
}
```

Implemented by `Field`, `TypedField<K>`, `TypedFieldRef<'_, K>`,
`ProtocolField<'_>`, and `ProtocolFieldMut<'_>` — the five field-shaped values
that exist today and each reimplement these accessors.

Family traits sit above it and are implemented by `TypedField<K>` for every `K`
whose `KIND` matches, so a narrowed field reaches its family's parameters with
no match and no `Option`:

```rust
pub trait TemporalField: FieldValue { fn unit(&self) -> TimeUnit;
                                      fn timezone(&self) -> Timezone; }
pub trait DecimalField:  FieldValue { fn precision(&self) -> u8; fn scale(&self) -> i8; }
pub trait NestedField:   FieldValue { fn children(&self) -> &Fields; }
```

```rust
let at: TypedField<DateTime64Type> = row.get_as("ts")?;
let unit = at.unit();          // no match, no Option, no error path
```

### Field slimming

Measured, and worth doing while the family tiers are being written:

| Field shape | Bytes |
| --- | --- |
| today | 104 |
| interned `Timezone` (from the scalar brief) | 96 |
| plus `dictionary_id` and `dictionary_is_ordered` moved into `DictionaryType` | **88** |
| plus the Arrow cache boxed | 72 |

`dictionary_id: i64` and `dictionary_is_ordered: bool` are on every field of
every schema and mean something on exactly one datatype family. They belong to
`NestedType::Dictionary(Arc<DictionaryType>)`, which every dictionary field
already carries. That is 16 bytes off every field in every schema, and it makes
the invariant structural instead of documented.

Do **not** box the Arrow cache for the last 16 bytes. `AGENTS.md` requires
cached complete Arrow projections, and an allocation on first projection is a
worse trade than the bytes.

## Holder

The family tier is earned here: the enum is 10 flat variants encoding a
backend × role matrix that the module tree already spells out.

```rust
#[non_exhaustive]
pub enum Holder {
    Buffer(Buffer),
    Local(LocalHolder),
    ArrowFs(ArrowFsHolder),
    Wrapped(WrappedHolder),
}

pub enum LocalHolder   { Folder(local::Folder), Path(local::Path), File(local::File) }
pub enum ArrowFsHolder { Folder(arrowfs::Folder), Path(arrowfs::Path), File(arrowfs::File) }
pub enum WrappedHolder { Buffered(Box<Buffered<Holder>>), Text(Box<Text<Holder>>), Media(Box<Media>) }
```

Backend-first, not role-first, because `AGENTS.md` makes the backend the module
unit — *"Storage backends are sibling module folders containing `Path`,
`Folder`, and `File`"* — so adding S3 is one `Holder` variant plus one folder,
never three variants scattered through a flat list. The `ArrowFolder` /
`ArrowPath` / `ArrowFile` prefix naming disappears: the family already says it.

The trait floor exists (`IOBase`, `IOFolder`, `IOFile`, `IOPath`); add
`StorageBackend` naming the three roles a backend supplies, so the family enums
are generated shape rather than three hand-written triples.

## Media

**Do not add a family tier to `Media`.** It fails the second half of the rule:
`Parquet`, `Text`, and `Iceberg` would each be a family of one, and the
grouping carries no behavior the root cannot express.

What `Media` is missing is the **trait floor above `IOMedia`** — the
capabilities that differ per encoding and are today discovered by matching on
the variant or by probing:

```rust
/// A media whose encoding stores per-column statistics and can skip reads.
pub trait ColumnarMedia: IOMedia {
    fn statistics(&self, field: &Field) -> Result<ColumnStatistics>;
    fn prune(&self, predicate: &Expression) -> Result<Expression>;   // returns the residual
}

/// A media planned from metadata rather than read end to end.
pub trait TableMedia: IOMedia {
    fn snapshot(&self) -> Result<SnapshotId>;
    fn plan(&self, predicate: &Expression) -> Result<ScanPlan>;
}

/// A media whose rows are physical lines.
pub trait LineMedia: IOMedia {
    fn options(&self) -> &TextOptions;
    fn set_options(&mut self, options: TextOptions) -> Result<()>;
}
```

`Parquet` implements `ColumnarMedia`, `Iceberg` implements `TableMedia`, `Text`
implements `LineMedia`, and `Media` exposes `as_columnar()`, `as_table()`,
`as_line()` returning `Option<&dyn …>`. Pushdown then asks a capability instead
of asking which encoding it is, and a second columnar encoding costs one impl.

Revisit the family tier when a second columnar encoding exists. Not before.

## Conformance sweep

| Notion | Trait | Enum | Leaves | Work |
| --- | --- | --- | --- | --- |
| value | `ScalarValue` + 6 family traits | `Scalar` → `Temporal`, `Integer`, `Float`, `Decimal`, `Geospatial`, `Nested` | `Int32`, `DateTime64`, … | `SCALAR_HIERARCHY_PROMPT.md` |
| datatype | `DataTypeValue`, `DataTypeFamily` | `DataType` → `TemporalType`, `IntegerType`, … | `DateTime64Type`, `DecimalParams`, … | this brief |
| schema unit | `FieldValue` + family traits | — (by design) | `Field`, `TypedField<K>` | this brief |
| storage | `IOBase`, roles, `StorageBackend` | `Holder` → backend enums | `Buffer`, `local::File`, … | this brief |
| record encoding | `IOMedia` + capability traits | `Media` (flat, deliberately) | `Ipc`, `Parquet`, `Avro`, `Text` | this brief |
| content coding | `IOBase` | `Coded` (flat, 3 members) | `Gzip`, `Zlib`, `Zstd` | conforms |
| identifier | `FromStr`, `Display` | — (newtype narrowing) | `Uri`, `Url(Uri)`, `Urn(Uri)` | conforms |
| digest | `Digester` | `DigestAlgorithm` | the four resumable states | conforms |
| expression | — | `Expression` (recursive, by node class) | leaf nodes | conforms |

Three of nine already conform. `Uri`/`Url`/`Urn` narrow by newtype rather than
by enum, which is the same rule with a cheaper middle floor — document it as
conforming rather than converting it.

## Phases

| # | Phase | Content |
| --- | --- | --- |
| 1 | `DataTypeValue` / `DataTypeFamily` traits | added against the flat enum, shims inside `types/` only |
| 2 | family enums | the eight tier-2 enums and their leaves in `types/<family>/dtypes.rs` |
| 3 | `DataType` root | reshape to 10 variants; rewrite every construction and match; `DataTypeKind` becomes the discriminant |
| 4 | `FieldValue` | one trait, five implementors, delete the duplicated accessors |
| 5 | family field traits | `TemporalField`, `DecimalField`, `NestedField` on `TypedField<K>` |
| 6 | field slimming | dictionary state into `DictionaryType`; assert `size_of::<Field>() == 88` |
| 7 | `Holder` | backend family enums; drop the `Arrow*` prefixes; add `StorageBackend` |
| 8 | media capabilities | `ColumnarMedia`, `TableMedia`, `LineMedia`; route pushdown and planning through them |
| 9 | bindings and docs | Python/JS accessors, `AGENTS.md`, inventories |

Phase 3 is the unsplittable one. Land 1-2 first so it is mechanical.

## Verification

Per phase, the `REORGANIZATION_PROMPT.md` gate. Assertions to add and keep:

```rust
const _: () = assert!(size_of::<DataType>() == 24);
const _: () = assert!(size_of::<Field>() == 88);
const _: () = assert!(size_of::<TemporalType>() == 12);
const _: () = assert!(size_of::<IntegerType>() == 1);
```

Behavioral invariants, each with a test:

- `DataType` total order, equality, hash, `Display`, and serde are unchanged
  against a recorded fixture — not against the new implementation.
- `DataTypeKind::from(&dtype)` agrees with the tier-1 discriminant for all 47
  variants.
- `TypedField<K>` is still pointer-sized where it is today
  (`rust/tests/field/typed.rs` already asserts this; keep it).
- Cloning a `DataType` still allocates nothing at any depth.
- The parser's recursion limit still rejects the same inputs at the same depth.

## Completion

- Every notion in the sweep table conforms or is documented as deliberately
  flat, with the reason in its module doc.
- No family enum exists with one member.
- Every name obeys the naming law: `rg -n "(struct|enum) [A-Za-z0-9_]+Scalar" rust/src`
  matches only pairings, and every family enum is bare-named.
- Every name obeys the case law: no underscore in any type, trait, class, or
  enum variant name in any of the three languages.
- The size assertions are in the tree and passing.
- `AGENTS.md` states the three-floor rule and the earned-family test once, and
  every notion-specific layout rule it replaces is deleted.
