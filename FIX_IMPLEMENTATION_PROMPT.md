# FIX field and registry implementation brief

Implement the `fix:` protocol vocabulary on the existing `FixField`/`FixFieldMut`
protocol views, plus one `FixRegistry` that stores those fields through `IOBase`
and resolves them by tag or name. Follow `AGENTS.md`; this file contains only
FIX-specific decisions.

This is the first, deliberately small phase. Ship the base contract, keep the
resolution path fast enough to stay the hot path later, and leave elaboration for
the next session.

## Outcome

- One generic field carries every FIX shape. No second field type, and no
  separate component or repeating-group class.
- A FIX field is a core `Field` read through its `fix:` protocol view, so it
  already has a name, a `DataType`, nullability, metadata, Arrow projection,
  casting, and serde.
- `FixRegistry` does CRUD over those fields, keyed uniquely by tag and name.
- Tag and name resolution are indexed and allocate nothing on a hit. A caller
  that already knows which key it holds calls the specialized accessor and pays
  no dispatch.
- The registry persists through `IOBase` into JSON shards of 100 tags each.
- One process-wide registry autoloads once and is the default every other type
  resolves against.
- `FixMsg` is a message linked to the registry that types it.
- Every FIX implementation lives in one core module folder. Nothing FIX-specific
  is added to `field/`, `metadata.rs`, `iceberg/`, or `io/`.
- Serialization is inherited from `Field`'s JSON path and the generic `Scalar`
  codec. This module writes no serializer, parser or validator of its own; where
  an existing path falls short, it is improved generically instead.
- Python and JavaScript reach the whole surface in the same change, with parity
  tests and boundary benchmarks, per the `AGENTS.md` delivery order: Rust core
  and its contract first, then both extensions as redirects.

## Read first

- `AGENTS.md` in full.
- `rust/src/field/protocol/mod.rs` for `ProtocolField`/`ProtocolFieldMut`, the
  `for_each_well_known_protocol!` list, and the generated `FixField`/
  `FixFieldMut` newtypes this brief fills in.
- `rust/src/field/protocol/http.rs` and `rust/src/iceberg/field.rs`, the two
  existing worked examples of typed protocol vocabulary. `iceberg/field.rs` is
  the closer model: it lives outside `field::protocol` and uses the public view
  surface only.
- `rust/src/datatype/nested.rs` for `FieldKey`, the lookup-key enum `FixKey`
  mirrors.
- `rust/src/field/mod.rs` for `get_field`/`field`, the optional/failing accessor
  pair this registry repeats.
- `rust/src/iceberg/catalog/` for the collection contract `AGENTS.md` requires:
  `get`, `create`, `open_or_create`, `contains`, lazy iteration, `len`,
  `is_empty`.
- `rust/src/io/mod.rs` and `rust/src/json/` for storage and the JSON codec.
- `python/src/field.rs` and `node/src/field.rs` for how a core domain is
  mirrored at each boundary, including the protocol view classes.
- `docs/field.md` for documentation shape, and `docs/iceberg.md` for a module
  page that documents a registry-shaped collection.

## Design ideas taken from `Platob/yggfin`

Take the ideas, not the code; that implementation is far more elaborate than this
phase wants.

- Tiered resolution: canonical names first, declared aliases second. A later tier
  is consulted only when every earlier one missed, so adding an alias can never
  take a name away from a field that already claims it.
- Case folding: FIX spellings drift across versions and venues. Fold once, on the
  way in, match folded, and always answer the canonical spelling.
- Tag arithmetic picks the one shard that can hold a tag. Names have no numeric
  structure, so a name lookup needs an in-memory index rather than a shard scan.
- Disagreement between two definitions of one field is data to reconcile by
  declared priority, not an error to drop.

## Rust layout

Create `rust/src/fix/`:

| File | Responsibility |
| --- | --- |
| `mod.rs` | `FixKey`, re-exports, module docs |
| `field.rs` | `impl FixField` / `impl FixFieldMut` typed vocabulary |
| `registry.rs` | `FixRegistry`: CRUD, indexes, merge, accessors |
| `global.rs` | the autoloaded process-wide default registry |
| `store.rs` | `IOBase` shard read/write and JSON shape |
| `msg.rs` | `FixMsg` and its registry link |
| `tests.rs` | focused module edge cases |

Feature-gate nothing. `Scheme::FIX` and the `FixField` newtypes exist in the
default build, and `json` and `io` are unconditional.

### Isolation

Every FIX implementation lives under `rust/src/fix/`. This is a hard boundary,
not a preference, and `rust/src/iceberg/field.rs` is the precedent to copy: the
`FixField`/`FixFieldMut` newtypes are minted by the protocol list in
`field/protocol/mod.rs`, but their `impl` blocks belong here. An inherent impl on
a crate-local type is legal from any module of the crate, which is exactly what
makes the isolation possible.

Concretely, this change adds nothing FIX-specific to:

- `rust/src/field/` — no FIX vocabulary, no FIX key constants, no FIX branch;
- `rust/src/metadata.rs` — the `fix:` namespace already exists in the protocol
  list and `validate_entry` needs no new arm, because FIX properties are
  ordinary namespaced text;
- `rust/src/iceberg/`, `rust/src/io/`, `rust/src/generic/` — untouched;
- `rust/src/lib.rs` — one `mod fix;` and one `pub use` block, nothing else.

FIX property key constants live in `rust/src/fix/field.rs` and stay private to
the module. If something in `fix/` seems to need a change elsewhere in core,
that is the signal to stop and re-read the model, not to widen a core type. The
one sanctioned exception is a generic improvement to an existing core path, as
described next.

### Leverage what exists

This module writes almost no serialization of its own. Everything it needs is
already built, and reaching for a FIX-specific implementation is the failure
mode this section exists to prevent.

Serializing a field record uses the existing `Field` JSON path:

- `Field::into_json` / `Field::into_json_bytes` and `Field::from_json` /
  `Field::from_json_bytes` (`rust/src/field/serde.rs`). A registry shard is an
  array of exactly that shape. Metadata rides along, so the whole `fix:`
  namespace persists with no extra work.
- `Field::into_json_with_formatting` when a shard should be readable on disk.
- `Metadata::into_json` / `Metadata::from_json` if a metadata-only shape is ever
  needed.

Serializing message values uses the generic `Scalar` codec:

- `crate::json::from_bytes_with_field` / `from_utf8_with_field` /
  `from_reader_with_field` — the field-directed forms. Passing the root `Field`
  is what types natural strings, orders records, and validates and canonicalizes
  in Rust, which is exactly the work a FIX transcriber would otherwise duplicate.
- `crate::json::into_utf8` / `into_bytes` / `into_writer` on the way out.
- `Limits` for bounded parsing. Do not invent FIX-specific bounds.
- The reader and writer forms for anything streamed; nothing here materializes a
  document it could stream.

Therefore, do NOT add: a `Serialize`/`Deserialize` impl for a FIX type, a FIX
JSON writer or reader, a FIX value parser, a FIX validator, a second limits
type, an envelope, a version marker, or any private wire representation. A FIX
field is a `Field` and a FIX message value is a `Scalar`; both already serialize.

If an existing path genuinely cannot express something this module needs,
IMPROVE THAT PATH rather than working around it. The improvement must be
generic — justified for every caller, named in the core's own vocabulary, with
its own tests, and never reachable only from FIX. Land it as its own commit so
the core change is reviewable apart from the FIX work. A FIX-shaped special case
added to `field/serde.rs` or `json/` is the same violation as a FIX-specific
writer, only harder to find later.

## Data model

A FIX field is a `Field` whose metadata carries the `fix:` namespace. Every
property below is read and written only through `FixField`/`FixFieldMut`;
callers never spell `fix:` at a call site.

| Property | Key | Type | Meaning |
| --- | --- | --- | --- |
| tag | `fix:tag` | `i32` | canonical FIX tag |
| tags | `fix:tags` | ordered `i32` list | alternate tags, highest priority first |
| aliases | `fix:aliases` | ordered name list | alternate names, highest priority first |
| description | `fix:description` | text | the specification's own wording |

The canonical name is the `Field`'s own `name()`; do not duplicate it under a
`fix:` key. The datatype is the `Field`'s own `dtype()`. The nice display name is
the generic `display` metadata key on `Field`, not a FIX property.

List-valued properties store as comma-separated text, because metadata values are
text by contract. Parse on read, render on write, and reject an empty element or
a duplicate. Stored order is priority order and must be preserved.

Accessors, following `AGENTS.md` vocabulary:

```rust
impl FixField<'_> {
    pub fn tag(&self) -> Result<Option<i32>>;
    pub fn tags(&self) -> Result<Vec<i32>>;      // empty when absent
    pub fn aliases(&self) -> FixAliases<'_>;     // lazy, allocation-free
    pub fn description(&self) -> Option<&str>;
}

impl FixFieldMut<'_> {
    pub fn set_tag(&mut self, tag: i32) -> Result<()>;
    pub fn set_tags(&mut self, tags: &[i32]) -> Result<()>;
    pub fn set_aliases<I, S>(&mut self, aliases: I) -> Result<()>;
    pub fn set_description(&mut self, value: impl Into<String>) -> Result<()>;
}
```

## Nesting, without a second type

Repeating groups and components are already expressible and must not grow their
own type in this phase:

- a component is a Struct `Field` whose children are its members;
- a repeating group is a List `Field` whose item is that Struct;
- the group's counter tag is the group field's own `fix:tag`.

Shape the transcription path against that model now, even though the transcriber
itself is out of scope, so a later key/value walk over nested structures needs no
model change.

## Keys and resolution

```rust
pub enum FixKey<'a> {
    Tag(i32),
    Name(&'a str),
}
```

With `From<i32>`, `From<&'a str>`, and `From<&'a String>`, exactly as `FieldKey`
does, so `registry.field(35)` and `registry.field("MsgType")` are both one call.

Resolution order is fixed and documented:

1. canonical tag, then alternate tags in stored order;
2. canonical name folded, then aliases in stored order.

A tag query never consults names and a name query never consults tags. Either
answers the canonical field.

## Registry

Unique identity is the pair `(tag, name)`. Two fields may share neither.

```rust
impl FixRegistry {
    // Specialized: the caller already knows which key it holds. No enum, no
    // dispatch, no `Into` conversion. These carry the implementation.
    pub fn get_field_by_tag(&self, tag: i32) -> Option<&Field>;
    pub fn field_by_tag(&self, tag: i32) -> Result<&Field>;
    pub fn get_field_by_name(&self, name: &str) -> Option<&Field>;
    pub fn field_by_name(&self, name: &str) -> Result<&Field>;
    pub fn get_field_by_path(&self, path: &str) -> Option<&Field>;
    pub fn field_by_path(&self, path: &str) -> Result<&Field>;

    // Generic: the key is only known at runtime. Matches `FixKey` once and
    // redirects to the specialized method. It adds no behavior of its own.
    pub fn get_field<'key>(&self, key: impl Into<FixKey<'key>>) -> Option<&Field>;
    pub fn field<'key>(&self, key: impl Into<FixKey<'key>>) -> Result<&Field>;

    pub fn contains<'key>(&self, key: impl Into<FixKey<'key>>) -> bool;
    pub fn insert(&mut self, field: Field) -> Result<Option<Field>>;
    pub fn update(&mut self, field: Field) -> Result<()>;
    pub fn remove<'key>(&mut self, key: impl Into<FixKey<'key>>) -> Option<Field>;
    pub fn iter(&self) -> FixFieldIter<'_>;
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
}
```

`get_field` answers `None`; `field` raises absence. This is `Field`'s own
`get_field_at`/`field_at`, `get_field_by_path`/`field_by_path`,
`get_field`/`field` shape, repeated deliberately: same verbs, same optional and
failing halves, same `_by_<what>` suffix for a known key type.

The generic pair is sugar over the specialized ones and must never be the place
a lookup is implemented. A duplicated lookup body behind the enum is the failure
mode to avoid.

`field_by_path` walks a dotted path through nested components and repeating
groups: it resolves the first segment in the registry, then delegates to
`Field::get_field_by_path` for the remainder, so nesting has exactly one path
resolver in the codebase. `NoPartyIDs.PartyID` reaches the group member;
`NoPartyIDs` reaches the group itself.

Storage and indexes:

- fields in one `Vec<Field>` at stable positions;
- `BTreeMap<i32, usize>` over every canonical and alternate tag, ordered so shard
  writes and range scans stay cheap;
- `HashMap<Box<str>, usize>` over every folded canonical name and alias.

Both indexes hold positions, so a lookup is one map probe plus one index. Fold
name keys once, at insert; never allocate on a hit. `insert` rejects a tag or a
folded name already claimed by a different field, naming both, per the atomic
mutation rule.

`update` merges rather than replaces. Merging two definitions of one field:

- the incoming field wins a property both declare;
- the stored field keeps a property only it declares;
- `tags` and `aliases` concatenate, incoming first, deduplicated, order kept;
- a datatype disagreement is a typed error naming both, never a silent widening.

Merge in place, touch only what changed, and leave the registry unchanged on
failure.

## Storage

The registry reads and writes through `IOBase` alone; no direct filesystem.

- Root is a folder handle. Fields live under `records/fix/fields/`.
- Shard index is `tag / 100`, so tags 0-99 are `0.json` and 100-199 are `1.json`.
  A tag reaches exactly one shard by arithmetic.
- A shard file is a JSON array of the core `Field` JSON shape, ordered by tag,
  produced and consumed by `Field::into_json_bytes`/`Field::from_json_bytes`.
  The shard file itself is the only thing this module composes: an array of
  already-serialized fields. See "Leverage what exists" — no envelope, version
  marker, or private wire form, and no FIX serializer.
- Alternate tags do not fan a field across shards. It is written once, under its
  canonical tag's shard; alternate tags are index entries only.
- Loading builds both indexes. A name query is answered from the index, never by
  scanning shards.
- Writes are whole-shard through the handle's ordinary byte write, so a failed
  write leaves the prior shard intact.

Keep per-shard loading lazy where that stays simple; if it does not, load every
shard on open and say so in the module docs.

## Default registry

One process-wide registry, autoloaded once, is what every caller gets when it
does not name one. Callers that build their own keep passing it explicitly; the
default exists so the common path needs no plumbing.

```rust
impl FixRegistry {
    /// Returns the process-wide registry, loading it on first use.
    pub fn global() -> &'static Arc<FixRegistry>;

    /// Installs the process-wide registry before anything resolves it.
    ///
    /// # Errors
    ///
    /// Returns an error when the default has already been resolved, so the
    /// value every caller saw cannot change underneath them.
    pub fn install_global(registry: FixRegistry) -> Result<()>;
}
```

Back it with a `OnceLock<Arc<FixRegistry>>`, the pattern `Metadata`'s empty
snapshot and `Field`'s Arrow cache already use. `Arc` because `FixMsg` links to
it and must not borrow a static for its whole life.

Autoload resolution is deterministic and documented in one place, first match
wins:

1. a registry installed by `install_global`;
2. the folder named by `YGGDRYL_FIX_REGISTRY`, opened through `IOBase` like any
   other URL, so a local path, `mem://`, or a remote backend all work;
3. the empty registry.

Environment access is the one concession, and it is read exactly once, at
resolution. A malformed or unreadable location is a typed error surfaced from
`global()`, never a silent fallback to empty: a registry that quietly loads
nothing turns every later lookup into a wrong answer instead of a failure.

Do not autoload on module init and do not spawn a thread. Loading happens on the
first `global()` call, on the calling thread.

Also give the ordinary builder path, because tests and embedders need it:

```rust
impl FixRegistry {
    pub fn new() -> Self;
    pub fn from_handle(handle: &dyn IOBase) -> Result<Self>;
    pub fn from_fields<I: IntoIterator<Item = Field>>(fields: I) -> Result<Self>;
}
```

`from_handle` is the same reader the autoload path uses; there is one loader.

## FixMsg

A FIX message is a value plus the registry that types it. Design it now, and
keep this phase to construction and lookup — parsing the wire and transcribing
key/value pairs is the next session's work.

```rust
pub struct FixMsg {
    registry: Arc<FixRegistry>,
    field: Field,     // root Struct field: the message's resolved schema
    value: Scalar,    // Scalar::Record of resolved values
}
```

- The registry link is an `Arc`, cloned from `FixRegistry::global()` when the
  caller does not supply one. A message carries the dictionary it was resolved
  against, so a later re-resolution cannot silently use a different one.
- The schema is a core Struct `Field`, per `AGENTS.md`: a non-null Struct `Field`
  is the only row schema. Do not add a second schema type.
- Values are one `Scalar::Record`, canonicalizing to an ordered
  `Scalar::Sequence` against the root field exactly as every other row does.
- Accessors mirror the registry's: `get_by_tag`/`by_tag`, `get_by_name`/
  `by_name`, `get_by_path`/`by_path`, each answering a `&Scalar`, plus the
  generic `get`/`value` over `FixKey`. Resolution goes through the linked
  registry, never through a private copy of the rules.
- An unknown tag is retained, not dropped: keep it under its rendered tag name
  with the registry's fallback datatype. Losing an unknown tag loses the message.

Constructors for this phase:

```rust
impl FixMsg {
    pub fn new(field: Field, value: Scalar) -> Result<Self>;              // global registry
    pub fn with_registry(registry: Arc<FixRegistry>, field: Field, value: Scalar) -> Result<Self>;
    pub const fn registry(&self) -> &Arc<FixRegistry>;
    pub const fn as_field(&self) -> &Field;
    pub const fn as_value(&self) -> &Scalar;
}
```

Both constructors validate the value against the field with the existing
`Field::validate_value`/`canonicalize_value`; no FIX-specific validator.

Serialization is inherited, not written: a message is a `Field` plus a `Scalar`,
so `Field::into_json` renders the schema and `crate::json::into_utf8` renders the
value, while `crate::json::from_bytes_with_field` reads a value back already
typed, ordered and canonicalized against that field. `FixMsg` needs no serde impl
of its own.

## Python

Stabilize the Rust contract first, then mirror it. No FIX logic in Python:
resolution, folding, merging, sharding and validation stay native.

- `FixRegistry` as a native wrapper with the mapping and collection protocols
  Python expects: `__len__`, `__contains__`, `__iter__`, `__getitem__` raising
  `KeyError` for absence. `__getitem__` accepts an `int` tag or a `str` name and
  coerces to `FixKey` once, at the boundary.
- Both accessor halves keep their names: `get_field_by_tag`, `field_by_tag`,
  `get_field_by_name`, `field_by_name`, `get_field_by_path`, `field_by_path`,
  `get_field`, `field`. Preserve argument order, defaults and error semantics.
- The FIX vocabulary reaches Python through the existing protocol view class the
  `fix` accessor already returns, so `field.fix.tag` and `field.fix.aliases` are
  the spellings. Do not add a second field wrapper.
- `FixRegistry.global_()` or a module-level accessor for the process default —
  pick the spelling that reads as Python and document it once. Installing the
  default is explicit and fails the same way the core does.
- `FixMsg` with `registry`, `field` and `value` properties, `__getitem__` over
  tag/name/path, and iteration over resolved pairs.
- Storage takes the same `IOBase`/`Url`/path-like values every other Python entry
  point takes, coerced once at the boundary.
- Immutable wrappers implement stable equality, hash, order, pickle, copy and
  repr. `FixRegistry` is mutable, so follow Python's hash contract for it.

## JavaScript

Same rule: camelCase at the boundary only, and no reimplementation.

- `FixRegistry` with `get`, `has`, `size`, and lazy iteration, plus
  `getFieldByTag`, `fieldByTag`, `getFieldByName`, `fieldByName`,
  `getFieldByPath`, `fieldByPath`, `getField`, `field`.
- Tag arguments cross as JS `number`; reject a non-integer or out-of-`i32` value
  at the boundary rather than truncating. Watch the u64/u32 and i64/i32 casts —
  a silent narrowing here is the classic N-API bug in this repo.
- The FIX vocabulary reaches JS through the existing `fix` protocol view.
- The process default and `FixMsg` mirror the Python surface with JS spellings.
- Implement stable `toString`, JSON, equality, hash and clone behavior, and
  regenerate the type declarations; a hand-maintained declaration and a
  prototype patch must move in the same change as the class they name.

## Tests

Cover:

- property round trips, including empty and single-element lists, and rejection
  of an empty element, a duplicate, and a malformed tag;
- case-insensitive name and alias resolution answering the canonical spelling;
- tier order: an alias never shadows another field's canonical name;
- insert conflict on a duplicate tag and on a duplicate folded name;
- the merge truth table, including datatype disagreement and order preservation;
- shard arithmetic at 0, 99, 100, and a large tag, round-tripped through a memory
  handle;
- a Struct component and a List repeating group surviving a store round trip with
  children and tags intact;
- atomic failure: a rejected insert or merge leaves indexes and store unchanged;
- the specialized and generic accessors answer identically for every key, and
  `field_by_path` resolves into a component and into a repeating group member;
- `install_global` before first use wins, `install_global` after a resolved
  `global()` fails, and an unreadable `YGGDRYL_FIX_REGISTRY` errors rather than
  loading empty. Isolate these: they touch process-wide state, so they need a
  serialized test or a separate integration test binary.
- `FixMsg` links the global registry by default, keeps an explicitly supplied one,
  retains an unknown tag, and rejects a value its field refuses;
- isolation: a check that nothing under `rust/src/fix/` is referenced from
  `field/`, `metadata.rs`, `iceberg/` or `io/`, and that
  `cargo check -p yggdryl --no-default-features --lib` still passes;
- a round trip proving serialization is inherited: a field with the full `fix:`
  namespace, a Struct component and a List repeating group survive
  `Field::into_json_bytes` then `Field::from_json_bytes` unchanged, and a
  `FixMsg` value survives `crate::json::into_utf8` then
  `from_utf8_with_field` typed and ordered. If either needs a FIX-side fixup to
  pass, that is a core gap to fix in core, not to patch here;
- Python and JavaScript parity for every accessor: same answers, same argument
  order, same error type for absence, and the same canonical spelling returned
  for a case-insensitive hit;
- boundary coercion at each extension: an `int`/`number` tag, a `str`/`string`
  name, a path-like storage location, and rejection of a non-integer or
  out-of-`i32` tag rather than a silent narrowing.

## Benchmarks

Measure release builds for:

- tag hit, name hit, alias hit, and miss, each with the counting allocator
  proving the hit path allocates nothing;
- the specialized accessor against the generic one, so the dispatch the
  specialized form exists to avoid is measured rather than assumed;
- `field_by_path` at one, two, and three segments;
- insert and merge over a realistic dictionary;
- open and full load against shard count, plus first-call `global()` cost;
- resolution against a plain `HashMap<i32, Field>` baseline, so the index
  structure has to earn itself;
- Python and JavaScript boundary benchmarks for tag hit, name hit and full load,
  reported against the native numbers so the crossing cost is visible rather than
  hidden.

## Documentation

- Add `docs/fix.md`: the contract, the metadata table, resolution order, the
  specialized and generic accessor pairs, the default-registry resolution order
  including the environment variable, the `FixMsg` link, the nesting shape, and
  the storage layout, then the smallest runnable example.
- Add it to `mkdocs.yml` nav, and to `docs/field.md`'s protocol section as the
  third worked vocabulary beside HTTP and Iceberg.
- Every example uses Rust, Python and JavaScript tabs in that order, expressing
  the same operation idiomatically, each self-contained with an assertion.
  Nothing here is Rust-only, so no tab may be omitted.
- Extend `docs/extensions/python.md` and `docs/extensions/javascript.md` with the
  boundary behavior only: coercion, error mapping, and the tag-width rejection.
- Embed the generated benchmark results on the `docs/fix.md` method sections.

## Completion

Run the checks `AGENTS.md` requires for the touched surface: formatting,
warning-free Clippy, workspace tests with default features and with
`parquet iceberg`, the 1.85 and `--no-default-features --lib` checks, rustdoc
with warnings denied, the new benchmarks, both extension suites and their release
boundary benchmarks, the generated declaration and stub surfaces, docs examples,
and the strict mkdocs build.

A maturin or napi build that reports success without producing an importable
module is a false green: import the module and call one method before claiming an
extension suite passed. Hand off only the outcome, changed surfaces, verification,
remaining caveats, and the exact next action.
