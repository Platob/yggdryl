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
- `FixRegistry` does CRUD over those fields, keyed uniquely by identifier and by
  branched name.
- Identifier and name resolution are indexed and allocate nothing on a hit. A
  caller that already knows which key it holds calls the specialized accessor and
  pays no dispatch.
- The registry persists through `IOBase` into JSON shards of 100 tags each,
  under one folder per branch, in a `primitive` and a `nested` tree.
- One process-wide registry autoloads once and is the default every other type
  resolves against, from `~/.config/fix` on a real machine. The repository tracks
  its seed dictionary at `config/fix`.
- `FixMsg` is a message linked to the registry that types it.
- Every FIX implementation lives in one core module folder. Nothing FIX-specific
  is added to `field/`, `metadata.rs`, `iceberg/`, or `io/`.
- Serialization is inherited from `Field`'s JSON path and the generic `Scalar`
  codec. This module writes no serializer, parser or validator of its own; where
  an existing path falls short, it is improved generically instead.
- A core refactor lands first (Phase 0): explicitly named structured-text entry
  points that say they answer a `Scalar`, and well-known local root helpers for
  the temporary, home and config directories. FIX consumes both and adds no
  private copy of either.
- Python and JavaScript reach the whole surface in the same change, with parity
  tests and boundary benchmarks, per the `AGENTS.md` delivery order: Rust core
  and its contract first, then both extensions as redirects.

## Phase 0: core refactor, before any FIX code

Do this first, land it as its own commit, and only then start the FIX module. The
FIX work then consumes it instead of reaching past it.

Today a caller writes `yggdryl::json::from_utf8(text)` and nothing in the name
says it answers a `Scalar`. `AGENTS.md` already refuses that shape one layer
down — "every derived read and append names the core type it answers", which is
why `IOBase` has `read_scalar` and never a bare `read`. The structured-text
facades are the place that rule was not applied.

Add one explicitly named, inferring entry point per format and direction,
answering the generic `Scalar`:

```rust
pub fn from_json_scalar(input: impl Into<...>) -> Result<Scalar>;
pub fn into_json_scalar(value: &Scalar) -> Result<String>;
pub fn from_yaml_scalar(input: impl Into<...>) -> Result<Scalar>;
pub fn into_yaml_scalar(value: &Scalar) -> Result<String>;
pub fn from_toml_scalar(input: impl Into<...>) -> Result<Scalar>;
pub fn into_toml_scalar(value: &Scalar) -> Result<String>;
```

Plus the field-directed halves, because typing the parse is the whole reason FIX
uses them:

```rust
pub fn from_json_scalar_with_field(input: ..., field: &Field) -> Result<Scalar>;
// and the yaml and toml counterparts
```

Rules for this refactor:

- These are NOT aliases, and the change must not read as one. Each is the single
  inferring entry point over the existing explicit representation methods,
  coercing at the boundary and redirecting to them — exactly the pattern
  `AGENTS.md` already sanctions for `IOBase` ("may add one inferring entry point
  over the explicit method, which coerces at the boundary and redirects to it").
  State that in the module docs so a later reader does not delete them as
  duplicates.
- One implementation. They contain coercion and a redirect, and no parsing,
  rendering, validation or limits logic of their own.
- Do NOT rename the 130 existing `from_utf8`/`from_bytes`/`from_reader`/
  `into_utf8`/`into_bytes`/`into_writer` functions and their `_all`,
  `_with_field`, `_with_limits` and `_with_formatting` modifiers. They are the
  explicit representation methods these redirect to, they are correct, and
  churning them buys nothing.
- Inference is deterministic and documented: byte-like input is content, and a
  string is content, never a path. Do not import the path-sniffing rule from the
  I/O facades; a JSON document that happens to name a file must not be read as
  one.
- Re-export the six from the crate root beside `Scalar`, so the call site reads
  `yggdryl::from_json_scalar(bytes)` without a stuttering module path.
- Amend `AGENTS.md`'s structured-text canonical-spellings bullet in the same
  change to name this family and say what it is for. The bullet currently lists
  only the representation forms, so leaving it alone would make the new surface
  undocumented by contract.
- Tests and benchmarks for the new entry points, and a docs pass wherever the
  structured-text facades are shown.

Then, in the FIX module, use ONLY these entry points for value serialization.
`from_json_scalar_with_field` is the one that types, orders, validates and
canonicalizes a message value against its root `Field`.

### Well-known local roots

`rust/src/local/` has no helper for the home directory, the config directory or
a temporary directory. It should, and this brief needs two of them, so add them
here rather than reaching around `IOBase` from the FIX module.

The duplication is already real and is what justifies the abstraction under
`AGENTS.md` ("add an abstraction only when it removes real duplication"):
`std::env::temp_dir()` is hand-spelled at roughly twenty sites across
`rust/src/`, including public doctests in `io/mod.rs`, `iceberg/mod.rs`,
`iceberg/table.rs`, `generic/holder.rs` and `expression/selector.rs`. There is no
home helper at all, so every caller that wants one would invent its own.

Add to `rust/src/local/`, answering the module's own `Folder`:

```rust
impl Folder {
    /// The platform temporary directory.
    pub fn temporary() -> Result<Self>;

    /// The current user's home directory.
    pub fn home() -> Result<Self>;

    /// The current user's configuration directory, `~/.config`.
    pub fn config() -> Result<Self>;
}
```

- `home` resolves `HOME`, falling back to `USERPROFILE` on Windows, through
  `std::env`. Do not add a dependency for this, and do not use the deprecated
  `std::env::home_dir`. With neither variable set, return a typed error naming
  both — a caller that wants to treat that as "no home" checks the result, and
  guessing a path here would be worse than failing.
- `config` is `home()` joined with `.config`, so the home rule lives in one place.
- `temporary` wraps `std::env::temp_dir()`.
- These construct a handle; they do not create the directory. Creation stays a
  write consequence, per `AGENTS.md`.
- Migrate the existing `std::env::temp_dir()` sites, doctests included, to
  `Folder::temporary()`. That is the change paying for itself, and it is what
  stops the helper from becoming a twenty-first spelling.
- Add these three to `AGENTS.md`'s canonical-spellings list for local storage, so
  the next person looking for a home directory finds them instead of writing
  `std::env` again.

The FIX registry then resolves `~/.config/fix` as `Folder::config()?.join("fix")`
and never touches `std::env` itself.

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
| `mod.rs` | `FixBranch`, `FixId`, `FixKey`, re-exports, module docs |
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

This module writes almost no infrastructure of its own. Nearly everything it
needs is already built, and reaching for a FIX-specific implementation is the
failure mode this section exists to prevent.

Before writing any helper, find the core one. The standing rule for this work:

- a value is a `Scalar`, a schema is a `Field`, a location is a `Url`, and
  storage is an `IOBase` handle — this module introduces no parallel type for
  any of them;
- serialization is `Field`'s JSON path and the generic `Scalar` codec;
- filesystem locations come from `rust/src/local/`'s well-known-root helpers, not
  from `std::env` or string paths;
- bounds come from `Limits`, errors from `yggdryl::Error`'s typed variants, and
  lookups follow `Field`'s own `get_*`/failing pairs;
- if the core helper does not exist, ADD IT TO CORE, generically, in its own
  commit — the way Phase 0 adds the scalar entry points and the local roots.
  Writing a private FIX copy is the outcome to avoid, because it is invisible to
  every other caller who needed the same thing.

A good check before adding anything to `rust/src/fix/`: would another module
want this too? If yes, it belongs in core.

Serializing a field record uses the existing `Field` JSON path:

- `Field::into_json` / `Field::into_json_bytes` and `Field::from_json` /
  `Field::from_json_bytes` (`rust/src/field/serde.rs`). A registry shard is an
  array of exactly that shape. Metadata rides along, so the whole `fix:`
  namespace persists with no extra work.
- `Field::into_json_with_formatting` when a shard should be readable on disk.
- `Metadata::into_json` / `Metadata::from_json` if a metadata-only shape is ever
  needed.

Serializing message values uses the generic `Scalar` codec:

- `from_json_scalar_with_field` (Phase 0). Passing the root `Field` is what types
  natural strings, orders records, and validates and canonicalizes in Rust, which
  is exactly the work a FIX transcriber would otherwise duplicate.
- `into_json_scalar` on the way out, and the yaml/toml counterparts wherever a
  FIX dictionary is authored in those formats.
- The representation-specific `crate::json::from_reader_with_field` /
  `into_writer` forms only where something is genuinely streamed and the
  inferring entry point cannot express it.
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
| branch | `fix:branch` | `FixBranch` | the dictionary this field belongs to; absent is `FixBranch::STANDARD`, and setting the standard one removes the key |
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
    pub fn branch(&self) -> Result<FixBranch>;   // absent is STANDARD
    pub fn id(&self) -> Result<Option<FixId>>;         // derived, never stored
    pub fn tag(&self) -> Result<Option<i32>>;
    pub fn tags(&self) -> Result<Vec<i32>>;      // empty when absent
    pub fn aliases(&self) -> FixAliases<'_>;     // lazy, allocation-free
    pub fn description(&self) -> Option<&str>;
}

impl FixFieldMut<'_> {
    pub fn set_branch(&mut self, branch: &FixBranch) -> Result<()>;
    pub fn set_id(&mut self, id: &FixId) -> Result<()>;   // both halves, atomically
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

Identity is `FixId`: a `FixBranch` and a tag, rendered and parsed
`branch:tag`, derived on every read from `fix:branch` and `fix:tag` and
never stored. `FixId::from_parts` is the one place the standard-tag rule lives:
a tag below `FixId::STANDARD_TAG_LIMIT` (5000) is one the FIX specification
assigns, so it forces `FixBranch::STANDARD`; the standard branch holds
any tag. Every producer of an identity reaches the rule through that
constructor and none re-checks it.

```rust
pub enum FixKey<'a> {
    Tag(i32),        // the standard branch
    Id(&'a FixId),   // any branch, exactly
    Name(&'a str),   // the standard branch, folded
}
```

With `From<i32>`, `From<&'a FixId>`, `From<&'a str>`, and `From<&'a String>`,
exactly as `FieldKey` does, so `registry.field(35)` and
`registry.field("MsgType")` are both one call. A colon-bearing string is a name,
never an identifier: `From` cannot fail, so parsing there would need a silent
fallback.

Resolution order is fixed and documented, and never leaves one branch:

1. canonical identifier, then alternate identifiers in stored order;
2. canonical name folded, then aliases in stored order.

Each of those indexes is split into a primitive and a nested half by the same
`is_nested` predicate the storage uses, and each tier reads the primitive half
before the nested one. That is locality only: the identity space is one, every
write checks both halves, and the split is a partition of each index rather
than a fifth tier above them.

A tag query never consults names and a name query never consults tags. Either
answers the canonical field. A bare tag and a bare name are the standard
branch, stated rather than resolved by walking whichever dictionaries happen
to be loaded.

## Registry

Unique identity is the `FixId` and, separately, the pair
`(branch, folded name)`. Two fields may share neither. Two branches may
define the same name and the same tag; a conflict is only ever within one
branch, and every conflict message names it.

```rust
impl FixRegistry {
    // Specialized: the caller already knows which key it holds. No enum, no
    // dispatch, no `Into` conversion. These carry the implementation.
    pub fn get_field_by_id(&self, id: &FixId) -> Option<&Field>;  // carries the implementation
    pub fn field_by_id(&self, id: &FixId) -> Result<&Field>;
    pub fn get_field_by_tag(&self, tag: i32) -> Option<&Field>;   // = by_id(&FixId::standard(tag))
    pub fn field_by_tag(&self, tag: i32) -> Result<&Field>;
    pub fn get_field_by_name(&self, branch: &FixBranch, name: &str) -> Option<&Field>;
    pub fn field_by_name(&self, branch: &FixBranch, name: &str) -> Result<&Field>;
    pub fn get_field_by_path(&self, branch: &FixBranch, path: &str) -> Option<&Field>;
    pub fn field_by_path(&self, branch: &FixBranch, path: &str) -> Result<&Field>;

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

- Root is a folder handle. Shards live in two trees,
  `<root>/primitive/<branch>/<shard>.json` and
  `<root>/nested/<branch>/<shard>.json`: `primitive` holds the fields whose
  datatype is one scalar value and `nested` the components and repeating
  groups, decided by `field.dtype().is_nested()` and nothing FIX-specific. The
  branch level sits above the shard level because a shard index is only unique
  inside one dictionary. The root already names FIX, so no `fix` segment is
  repeated inside it. Both trees are optional: a dictionary of only scalars
  writes no `nested/`, and a root with neither loads as the empty registry. The
  record is authoritative and the folder is layout; a field whose `fix:branch`
  contradicts its folder, or whose datatype contradicts its tree, is a typed
  error naming both, and a leaf directly under a tree root is a typed error
  rather than a folder skipped into an empty load.
- Two roots are conventional, and they are different things:
  - `config/fix/` in this repository, tracked in git. This is the seed
    dictionary: the field definitions the project ships, and what the tests,
    benchmarks and docs examples resolve against. Keep it small and legible —
    it is read by humans in diffs.
  - `~/.config/fix/` on a real machine. This is the production default, the one
    `FixRegistry::global()` reaches when nothing else is configured. It is not
    tracked, not created by the build, and not seeded automatically.
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
3. `~/.config/fix`, the production default, when that folder exists;
4. the empty registry.

Step 3 is `Folder::config()?.join("fix")`, using the helper Phase 0 adds. The FIX
module does not read `HOME` itself, does not import `std::env`, and does not
build a path by string concatenation. When `Folder::config()` fails because the
machine has no home variable set, skip step 3 rather than guessing a path.

Step 3 is the ONE place absence is not an error. A missing `~/.config/fix` means
the machine has no dictionary installed, which is an ordinary first-run state, so
it falls through to the empty registry. Every other failure is loud: a
`YGGDRYL_FIX_REGISTRY` that is set but unreadable, and a `~/.config/fix` that
exists but is malformed, are both typed errors surfaced from `global()`. Never
fall back to empty on a corrupt read — a registry that quietly loads nothing
turns every later lookup into a wrong answer instead of a failure.

Environment access is the concession this design makes, and both variables are
read exactly once, at resolution.

The repository's own `config/fix` is NOT in this order. Nothing resolves a
registry by walking up from the current directory looking for a repo: that would
make behavior depend on where a process was started. Tests and docs examples
point at `config/fix` explicitly, through `from_handle` or by setting
`YGGDRYL_FIX_REGISTRY`.

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
  generic `get`/`value` over `FixKey`, plus `get_by_id`/`by_id` and a derived
  `branch()`. Resolution goes through the linked registry, never through a
  private copy of the rules. A bare tag or name resolves in a two-step tier -
  the message's own branch, when the identifier that would name is legal at
  all, then the standard one - and an identifier does not tier.
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
- Both accessor halves keep their names: `get_field_by_id`, `field_by_id`,
  `get_field_by_tag`, `field_by_tag`, `get_field_by_name`, `field_by_name`,
  `get_field_by_path`, `field_by_path`, `get_field`, `field`. `FixBranch` and
  `FixId` cross as strings, coerced once at the boundary; no new binding class. Preserve argument order, defaults and error semantics.
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
- the resolution order end to end: the env var beats `~/.config/fix`, an absent
  `~/.config/fix` falls through to empty, a present but malformed one ERRORS, and
  an unset home variable skips step 3 instead of guessing a path. Point `HOME` at
  a temporary directory rather than touching the developer's real one.
- the tracked `config/fix` seed loads: open it with `from_handle`, resolve a tag,
  a name and an alias, and assert the shard layout is exactly
  `config/fix/primitive/standard/<shard>.json` beside
  `config/fix/nested/standard/4.json`, which holds the one repeating group.
- `FixMsg` links the global registry by default, keeps an explicitly supplied one,
  retains an unknown tag, and rejects a value its field refuses;
- isolation: a check that nothing under `rust/src/fix/` is referenced from
  `field/`, `metadata.rs`, `iceberg/` or `io/`, and that
  `cargo check -p yggdryl --no-default-features --lib` still passes;
- a round trip proving serialization is inherited: a field with the full `fix:`
  namespace, a Struct component and a List repeating group survive
  `Field::into_json_bytes` then `Field::from_json_bytes` unchanged, and a
  `FixMsg` value survives `into_json_scalar` then `from_json_scalar_with_field`
  typed and ordered. If either needs a FIX-side fixup to pass, that is a core gap
  to fix in core, not to patch here;
- Phase 0 in its own right: each of the six entry points round-trips, the
  field-directed halves type and order against a root `Field`, inference treats a
  string as content and never as a path, and each redirects to the existing
  explicit method rather than parsing on its own;
- Phase 0's local roots: `Folder::temporary()`, `Folder::home()` and
  `Folder::config()` answer handles without creating anything, `config()` is
  `home()` joined with `.config`, and `home()` errors naming both variables when
  neither `HOME` nor `USERPROFILE` is set. Drive these with a temporary `HOME`,
  never the developer's real one;
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
