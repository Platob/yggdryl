# FIX versioning, code sets, and the registry that carries them

Seven phases in dependency order. Each is complete work on its own and each
states its own contract, files, tests, benchmark and docs. Follow
`AGENTS.md`: Rust core first, no backward compatibility, one fact in one
place. No new binding surface anywhere here - the only binding work in the
whole brief is the two mechanical call sites Phase 2's `Copy` identifier
forces, and it is named there.

## What is landed

| what | where |
| --- | --- |
| `FixBranch`, `FixId`, `FixKey` | `rust/src/fix/mod.rs` |
| `FixField` / `FixFieldMut`, `FixAliases` | `rust/src/fix/field.rs` |
| `FixRegistry`, `Half`, `insert`, `update`, private `merge` at `registry.rs:866` | `rust/src/fix/registry.rs` |
| `from_handle` / `write_into` | `rust/src/fix/store.rs` |
| `AsciiEnum`, the `field:enum` document | `rust/src/datatype/ascii.rs`, `rust/src/field/mod.rs:895` |
| `DataType`, `DataTypeId` (wire ids), `LOGICAL_NAMES` | `rust/src/datatype/`, `rust/src/generic/datatype_id.rs` |
| `Scalar` | `rust/src/generic/scalar.rs:751` |
| `xxh32`, `xxh3_64`, the streaming state | `rust/src/xxhash/mod.rs:111`, `mod.rs:134`, `rust/src/xxhash/state.rs` |
| the counting allocator, one target for the process | `rust/tests/allocations.rs` |
| the seed dictionary | `config/fix/{primitive,nested}/standard/*.json` |
| published resolution numbers to beat | `docs/fix.md`, "Measured resolution cost" |

Two facts of the current code constrain several phases:

- `FixRegistry::insert` admits only a field carrying `fix:tag`
  (`registry.rs:494`). Nothing without a tag enters the registry.
- `DataTypeId::as_u8` is `self as u8` and is documented as a wire contract
  (`datatype_id.rs:275`). A new variant is **appended**, never inserted, and
  `DataTypeId::ALL` grows from 54 to 55.

---

## Prior art: `Platob/yggfin`

<https://github.com/Platob/yggfin> is a Python FIX stack (`rekep`) over the
same problem, built further along. **Do not port its shapes.** It models
components, repeating groups and namespaces as separate directories and
carries a flat `comp` string on an entry, none of which yggdryl needs: here
a component is a Struct field, a group is a List of an `item` Struct, and a
branch is a folder - one tree, no second model. What it is worth reading for
is the *use cases* it has already been forced to handle, which are cited in
the phases below where they changed a rule.

Read, in this order:

| file | what it settles |
| --- | --- |
| `python/tests/fix/test_pairs.py` | every key and value shape `from_pairs` meets in production (Phase 7) |
| `data/fix/sources.json` | per-source provenance: pinned commit, checksums, license, priority (Phase 6) |
| `data/fix/versions.json` | the declared version list and the per-version session field order (Phases 3, 6) |
| `python/tests/fix/test_registry.py`, `test_entries.py` | resolution and merge cases across namespaces |
| `python/src/rekep/fix/orchestra.py`, `quickfix.py` | the two source formats read side by side |

Clone it read-only; it is not a dependency and nothing here links against
it.

---

## Phase 1 - `Version`: a generic value, datatype, scalar and field

A version is major, minor, further numeric parts, and an optional qualifier.
It is generic: no FIX spelling, no `FIX.` prefix, no `Latest`. The FIX layer
maps its own spellings onto it in Phase 3.

### The value

`rust/src/generic/version.rs`, exported as `crate::Version` beside
`Timezone` and `MediaType`.

```rust
pub struct Version {
    /// Numeric components, major first, trailing zeros trimmed.
    parts: [u16; Version::MAX_PARTS],   // MAX_PARTS = 4
    /// How many of `parts` the canonical spelling states.
    used: u8,
    /// The qualifier and which side of the bare version it sits on.
    qualifier: Option<Qualifier>,       // { text: SmolStr, pre: bool }
}
```

Contract:

- **Canonical on parse.** Trailing zero components are trimmed, so `4.4.0`
  and `4.4` are one value with one spelling. `Display` re-renders exactly
  what `FromStr` accepts, and the round trip is a test.
- **Grammar.** `major(.part)*` then an optional qualifier, which may be
  appended directly (`5.0SP2`), dot-introduced (`5.0.SP1`), or
  hyphen-introduced (`1.0.0-rc1`). A hyphen means *pre*-release; a dot or
  nothing means *post*-release. All three canonicalize to one spelling on
  the way out, because the same version really is written four ways in the
  wild and a value with four renderings is four values: Orchestra writes
  `FIX.5.0SP2`, yggfin's `data/fix/versions.json` writes `5.0.SP1`, the
  `ApplVerID` code set writes `FIX50SP1`, and the session line is `FIXT1.1`.
  A component is decimal, at most `u16::MAX`, at most `MAX_PARTS` of them;
  over-long input, a non-decimal component and an empty qualifier are
  `Error::Parse` naming the byte position, the way every other parser in the
  repo reports.
- **Ordering.** Components numerically with an unstated component reading
  zero, then qualifier class `pre < none < post`, then the qualifier itself
  by ASCII-folded alphabetic prefix and *numeric* suffix, so `SP2 < SP10`.
  `Ord`, `Eq` and `Hash` agree, and `4.4 == 4.4.0` is false only because the
  trim already made them the same value.
- **Bounds.** `Version::MIN` is `0`, `Version::MAX` is every component at
  `u16::MAX` with no qualifier, and `MAX` is the sentinel a caller uses for
  "newer than anything named". Both are `const`.
- **No allocation** on parse, compare or render for a qualifier that fits
  `SmolStr` inline, which every FIX and semver qualifier does. Pinned in
  `rust/tests/allocations.rs`.

### The datatype

- `DataType::Version` in `rust/src/datatype/mod.rs`, placed beside `Guid`.
- `DataTypeId::Version` **appended last** in `rust/src/generic/datatype_id.rs`;
  `ALL` becomes `[Self; 55]`; `as_str` is `"version"`; `kind` is
  `DataTypeKind::String`; `fixed_byte_width` is `None`.
- Parser spelling `version` in `rust/src/datatype/parser.rs`, no alias, and
  the word is not one the Arrow/SQL grammar already owns.
- Arrow representation is `Utf8`, the canonical text, in
  `rust/src/datatype/arrow.rs`. Chosen over a fixed-width packing because a
  qualifier has no length bound and a lossy Arrow round trip is not
  acceptable; the cost is that Arrow-side lexicographic order is **not**
  version order, which the docs state and a test demonstrates. `Ord` on
  `Version` is the only ordering contract.
- Every match the variant must reach, from the existing `Cfi` sweep:
  `datatype/{arrow,compatibility,default,merge,mod,parser,scalar,serde,tests}.rs`,
  `generic/{datatype_id,mod,typed}.rs`,
  `field/{ascii,mod,typed,value}.rs`, `field/cast/{mod,plan}.rs`,
  `arrow/value.rs`, `expression/{eval,parser}.rs`. Skip `datatype/ascii.rs`
  and `datatype/coded.rs`: a version is not an ASCII width and not a code.
  `iceberg/*` and `avro/*` reject it the way they reject other types they
  cannot represent.

### The scalar and the field

- `Scalar::Version(Version)` in `rust/src/generic/scalar.rs`.
- `DataType::scalar` is the one value contract: it accepts a
  `Scalar::String` that parses and **rewrites it** into `Scalar::Version`,
  accepts a `Scalar::Version` unchanged, and refuses everything else with
  expected/actual. `Field::scalar` adds nullability and name. Nothing
  re-checks a value `scalar` already answered.
- Casts in `field/cast/`: `Version -> Utf8` renders, `Utf8 -> Version`
  parses, `Version -> Version` is identity. No numeric casts.
- `DataType::Version.required_field("spec")` and the default value
  (`Version::MIN`) work like every other scalar.

### Tests, benchmark, docs

`rust/src/generic/version/tests.rs` (or the module's own `tests`): the
grammar including every refusal with its byte position, canonicalization of
trailing zeros, the full ordering table
(`0 < 1.0 < 4.2 < 4.4 < 5.0-rc1 < 5.0 < 5.0SP1 < 5.0SP2 < 5.0SP10 < MAX`),
the four spellings of one version all parsing equal (`5.0SP1`, `5.0.SP1`,
`FIX.5.0SP1` through the FIX layer's prefix strip, and `FIX50SP1` through
the `ApplVerID` code set),
`Display`/`FromStr` round trip, serde round trip, the Arrow round trip
through `Field`, and `DataType::scalar` rewriting a string. Add the
allocation case. Bench parse and compare in `rust/benchmarks/datatype.rs`.
Document the value on `docs/generic.md` and the datatype row on
`docs/datatype.md`; both pages' examples run under
`scripts/check_docs_examples.py`.

---

## Phase 2 - `FixId` is one `i64`

Independent of every other phase; do it first if two people are working.

### The representation

`FixId` stops being a branch and a tag side by side and becomes the packed
key itself: the tag in the high 32 bits, an `xxh32` of the branch text in
the low 32.

```rust
pub struct FixId(i64);   // ((tag as i64) << 32) | i64::from(xxh32(branch))
```

- `i64` and not `u64` because a tag is an `i32` in `0..=i32::MAX`, so bit 63
  is never set, every identifier is positive, and `Ord` on the `i64` is the
  natural order of the packed pair. The digest is zero-extended, so the low
  half compares unsigned.
- `Copy`, 8 bytes, `Hash` and `Ord` without touching the heap. `FixKey::Id`
  stops borrowing (`FixKey::Id(FixId)`), and `next_field_after` takes
  `Option<FixId>` by value.
- `FixId::standard(tag)` becomes a `const fn` - the doc comment that
  currently apologises for `SmolStr`'s `Drop` goes away with the field.
- **Nothing on disk changes.** `FixId` is derived from `fix:branch` and
  `fix:tag` on every read and never stored (`fix/mod.rs`), so no shard, no
  metadata key and no serialized shape moves. The change is entirely
  in-memory representation.

### `FixBranch` carries its own digest

`xxh32` (`rust/src/xxhash/mod.rs:111`) runs once, where the branch is built,
not once per identifier:

```rust
pub struct FixBranch { text: SmolStr, digest: u32 }   // text first
```

`text` is declared first so the derived `Eq` and `Ord` stay text-based; the
digest is a function of the text, so it can only ever agree. `FixBranch::from_str`
is the only constructor and computes it there. `FixBranch::STANDARD` is a
`const`, so its digest is a literal pinned by a test asserting
`xxh32(b"standard")` equals it - the same shape as any other constant the
code cannot compute at compile time. `MAX_LENGTH` stays 23: the digest is
beside `SmolStr`, not inside it.

`FixId::from_parts(branch, tag)` is then a shift and an or, and it keeps the
standard-tag rule it already owns, plus one new refusal: a non-standard
branch whose digest equals the standard branch's is rejected there, so
`FixId::is_standard()` - which is `digest == STANDARD_BRANCH_DIGEST` - stays
total.

### What the branch text costs

The digest is one-way, so **a bare `FixId` can no longer name its branch**.
This is the price of the representation and it is paid explicitly, not
papered over:

- `FixId::branch()` is deleted. Its eight callers
  (`fix/field.rs:195`, `fix/registry.rs:496,499,566,740`,
  `fix/store.rs:261,266,305`) all have the owning field in hand and read
  `fix:branch` from it, which is where the text actually lives.
  `FixId::branch_digest()` replaces it where only identity matters.
- `Display` renders `standard:35` for the standard branch and
  `#7f3a1c02:5001` - the digest in lowercase hex - for any other. The
  doctest in `fix/mod.rs` asserting `FixId::from_str("CME:5001")?.to_string()`
  is `"cme:5001"` changes with it. `from_str` still accepts `cme:5001`: it
  has the text and hashes it.
- `FixRegistry` keeps `branches: HashMap<u32, FixBranch>`, filled on insert,
  and `branch_of(&FixId) -> Option<&FixBranch>` recovers the spelling, so
  every refusal the registry raises still names `cme:5001` and only an
  identifier from outside any registry renders as hex. Two branches whose
  digests collide are a typed conflict at insert naming both spellings and
  the digest - a 32-bit space over a handful of branches, but a stated
  failure rather than a silent aliasing of two dictionaries.
- Rejected alternative: a process-wide branch intern table, so a bare
  `FixId` could render itself. It buys prettier `Debug` for a global lock or
  a leak on the hot path, and the registry already knows every branch it
  holds.

### The indexes

`Half` (`registry.rs:225`) holds four indexes per nestedness. Today an
identifier probe is `O(log n)` through a `BTreeMap<FixId, usize>`, each
comparison touching a branch `SmolStr` before the tag, and a name probe is
SipHash over branch plus folded name. With the packed identifier there is
nothing left to hash on an identifier lookup - the key *is* the id:

```rust
ids:            HashMap<FixId, usize, BuildHasherDefault<Mix>>
alternate_ids:  HashMap<FixId, usize, BuildHasherDefault<Mix>>
names:          HashMap<u64, usize,   BuildHasherDefault<Mix>>
aliases:        HashMap<u64, usize,   BuildHasherDefault<Mix>>
positions_by_id: Vec<usize>   // ordered by FixId, iteration only
```

- **`Mix` is a finalizer, not a pass-through.** The packed value's high bits
  are the tag, which is under 65536 for nearly every FIX field, so the top
  bytes are constant - and hashbrown takes its control byte from the top
  bits. Passing the raw `i64` through would put every standard field in one
  control-byte class. `Mix` applies one multiply-xor-shift finalizer, which
  is a few cycles and spreads both the bucket index and the control byte.
  Pin it with a test asserting the control-byte spread over the
  `config/fix` dictionary, not just that lookups answer.
- **Name and alias keys** stay text and are hashed per probe: ASCII-fold
  into the streaming xxh3 state (`rust/src/xxhash/state.rs`) in stack-sized
  chunks so no length allocates, seeded with the branch's own `xxh32` digest
  so a name can never be found under another branch, and with a distinct
  constant seed per index so one crafted key cannot land in both.
- **A digest collision is a loud refusal, never a wrong answer.** Two
  distinct names mapping to one `u64` would silently overwrite in a
  `HashMap<u64, _>`: `insert` verifies the field at an occupied key really
  holds it and returns a typed conflict naming both fields and the key
  otherwise, and reads re-check the same way, so a collision degrades to a
  miss on the read path. Identifier keys need no such check - a `FixId` is
  the whole key, and its only collision is the branch-digest one the branch
  table already refuses.
- **Ordered iteration keeps its own structure.** `next_field_after`,
  `FixFieldIter`, `Debug`, `PartialEq` and `store::write_into` depend on an
  ordered walk that a hash map cannot give. `positions_by_id` is a
  `Vec<usize>` kept sorted by binary-search insert: `O(n)` per insert on a
  dictionary built once and read forever, against `O(log n)` node chasing on
  every read.

### The order changes, and only across branches

Packed tag-major, `FixId`'s order is now tag then branch digest, where it
was branch then tag. Within one branch it is unchanged, so
**`store::write_into` produces byte-identical shards**: a shard folder is
one branch and a shard file is one `tag / 100` bucket, ordered by tag either
way. What moves is the cross-branch walk - `next_field_after`, `iter`,
`Debug`, `PartialEq` interleave vendor fields among standard ones by tag
instead of listing each dictionary in turn. Update the sentence in
`fix/mod.rs` that says an identifier "orders branch-major", the
`next_field_after` doc that says "ascending canonical identifier", and the
`FixFieldIter` docs, and assert the new order in a test rather than leaving
it to whichever map iterates first.

### The bindings

`python/src/fix.rs` and `node/src/fix.rs` only parse an id from text and
hold one as an iterator cursor (`after: Option<CoreFixId>`, passed as
`.as_ref()`); neither renders one back. `FixId` becoming `Copy` turns those
two call sites into by-value, and `STANDARD_TAG_LIMIT` is untouched. That is
the whole binding impact and it is mechanical - no new binding surface, no
`FixId` class on either side.

### Tests, benchmark, docs

Every existing registry test must pass unchanged except the two facts that
genuinely moved - `Display` for a vendor branch, and cross-branch iteration
order. Add: the packing round trip (`from_parts` then `tag()` and
`branch_digest()`) over the tag bounds `0`, `STANDARD_TAG_LIMIT`,
`i32::MAX`; `standard(tag)` in a `const` context; the pinned
`xxh32(b"standard")` constant; a branch-digest collision refused at insert
with both spellings named; a name-digest collision refused; ordering across
tags and branches; `write_into` byte-identical to a shard written before the
change. Keep `rust/tests/allocations.rs` green - lookups allocate nothing
today and must still.

Re-run `cargo bench -p yggdryl --bench fix` and **replace** the table in
`docs/fix.md`. The numbers to beat are the ones published there: 32.3 ns
primitive tag hit, 93.1 ns nested tag hit, 65.8 ns alternate tag hit, 72.2 ns
miss, 128.1 ns vendor identifier hit over 1034 fields, 81.8 ns name hit. A
phase that does not beat them is reported with the measurement, not merged
quietly.

---

## Phase 3 - FIX version handling: `fix:lineage`

Needs Phase 1.

### What a FIX version is here

The FIX layer maps its spellings onto `Version` in `rust/src/fix/`:
`FIX.4.2` is `4.2`, `FIX.5.0SP2` is `5.0SP2`, `FIX.2.7` is `2.7`, and
`FIX.Latest` is `Version::MAX`. The `FIX.` prefix is not stored: it is the
family, and this phase carries exactly one family. FIXT.1.1, the session
line, is **not** modelled - session tags carry the application version that
first defined them, and `docs/fix.md` names FIXT as a known omission.

### One new metadata key

`fix:lineage`, a JSON document, canonically rendered the way
`AsciiEnum::into_json` is - fixed key order, no whitespace, one text per
value:

```json
{"entries":[
  {"since":"2.7","name":"LastShares","type":"int"},
  {"since":"4.3","name":"LastQty","type":"Qty"}
]}
```

- `since` is a `Version`. Entries are ordered ascending and no two share a
  `since`.
- `name` is the field's spelling from that version on.
- `type` is the FIX datatype name from that version on, stored in the
  spelling `DataType::from_str` already resolves (`Qty`, `int`,
  `UTCTimestamp`), so the decoder needs no second table and the document
  stays readable.
- Optional per-entry `deprecated: true` and `removed: true` mark the states
  the specification gives them; a `removed` entry ends the field's life at
  that version.
- Every key beyond `since` is optional: an entry that states only `since` is
  "present, unchanged", which is what most versions are.

**The lineage is the authority and nothing duplicates it.** Two derivations
are computed by the writer, never by the caller, so they cannot drift:

1. The field's own `name()` and `dtype()` must equal the newest entry's.
   `FixFieldMut::set_lineage` refuses a lineage that disagrees, naming both
   sides.
2. `fix:aliases` is rewritten from the lineage's historical names on the
   same call, so `registry.field("LastShares")` resolves to the tag-32 field
   through the index that already exists. A test asserts the projection for
   every field in `config/fix`.

There is no `fix:since`, no `fix:until`, no `fix:deprecated` key:
`FixField::since()` is the first entry's `since` and `FixField::until()` the
`removed` one's, both derived on read the way `FixId` is derived from branch
and tag.

### The accessors

On `FixField`, borrowed and allocation-free wherever the answer is a slice
of the stored document:

| method | answers |
| --- | --- |
| `lineage()` | a lazy iterator of borrowed entries, like `FixAliases` |
| `since()` / `until()` | the `Version` bounds, `None` when no lineage |
| `defined_at(&Version)` | whether the tag exists at that version |
| `name_at(&Version)` | the spelling at that version |
| `dtype_at(&Version)` | the `DataType` at that version, resolved through `DataType::from_str` |

On `FixRegistry`:

| method | answers |
| --- | --- |
| `field_at(&Version, key)` | the field, refusing a key not defined at that version |
| `get_field_at(&Version, key)` | the same as an `Option` |
| `versions()` | the sorted set of versions any lineage mentions |

`versions()` is derived, not stored - it is what yggfin keeps as
`versions.json`'s `declared` list, and deriving it means a dictionary cannot
claim a version no field is dated in. Phase 7's inference and any
"is this version covered" check read it.

The registry itself stays version-agnostic: it holds every tag ever defined
and a version is a **filter on the read**, which is what "defined in one
version, available in the others" means. A registry-wide default version is
not introduced; a caller that wants one holds a `Version` beside the
registry.

### The boundary this phase does not cross

The lineage carries enough to rename and retype a value between two
versions. Actually rewriting a message - walking a root `Field`, renaming
children, casting values - is the transcoding layer, and it belongs with the
`.cfb` `normalization-binding` phase. Do not start it here; state the
boundary in the module docs.

### Tests and docs

Tag 32 is the worked case and goes in `rust/src/fix/tests.rs` verbatim:
`since()` is `2.7`; `name_at(4.2)` is `LastShares`; `name_at(4.3)` and
`name_at(Version::MAX)` are `LastQty`; `dtype_at(4.0)` is `Int32` and
`dtype_at(4.4)` is `decimal64(18,8)`; `registry.field("LastShares")` and
`registry.field("LastQty")` are the same field; `field_at(&"4.2", "LastQty")`
refuses. Plus: a lineage disagreeing with the field's own name is refused; a
field with no lineage answers `None` everywhere and resolves as before; the
JSON round trip is canonical; a malformed document names its byte position.
Document on `docs/fix.md` under a new section, and add the `fix:lineage` row
to the property table at the top of `rust/src/fix/mod.rs`.

---

## Phase 4 - code sets: `FixEnumValue` and `fix:codes`

Needs Phase 1 for the pedigree versions, Phase 3 for the version filter.

### Why not `field:enum`

`AsciiEnum` (`datatype/ascii.rs:345`) is name to ASCII value and nothing
else, and `Field::set_ascii_enum` packs every member through
`DataType::ascii_packed`, so it only accepts ASCII-width and coded
datatypes. Most FIX code sets sit on `int`, `Boolean` or `String` fields and
cannot use it at all, and none of them can carry a description or a
pedigree. So `fix:codes` is a second key carrying facts `field:enum` cannot
hold - not a second copy of the same fact. A field may carry both; nothing
derives one from the other.

### The value

`rust/src/fix/enums.rs`:

```rust
pub struct FixEnumValue {
    name: SmolStr,                 // symbolic name, "Buy"
    value: SmolStr,                // wire value, "1"
    description: Option<SmolStr>,
    since: Option<Version>,        // Orchestra `added`
    deprecated: Option<Version>,   // Orchestra `deprecated`
    ep: Option<u32>,               // Orchestra `updatedEP`, pedigree only
    sort: Option<u32>,             // Orchestra `sort`
    group: Option<SmolStr>,        // Orchestra `group`
}
```

Two symbolic names may share a wire value - that is what an alias is, the
same rule `AsciiEnum` already states. Two entries may not share a name.

### The document

`fix:codes`, canonical JSON, code-set name and id beside the values so one
document is the whole code set:

```json
{"name":"SideCodeSet","id":54,"codes":[
  {"name":"Buy","value":"1","since":"2.7","ep":254,"doc":"Buy; …"},
  {"name":"Sell","value":"2","since":"2.7","ep":254}
]}
```

Ordered by wire value so the rendering is one text per code set however it
was built.

### Optimized handling through `FixField`

The hot path is one lookup per wire value while decoding, over a code set
that can hold several hundred codes. Nothing on that path may build the
whole map or allocate:

- `FixField::codes() -> FixCodes<'field>` - a lazy iterator of
  `FixCode<'field>`, borrowed views over slices of the stored document,
  built the way `FixAliases` is built over `Split`.
- `FixField::code(value: &str) -> Option<FixCode<'field>>` - scans for the
  wire value and stops, using `memchr` (already a dependency) to find record
  boundaries rather than parsing JSON structurally.
- `FixField::code_by_name(name: &str)`, ASCII-folded.
- `FixField::code_at(&Version, value: &str)` - the same scan, skipping a
  code whose `since` is later or whose `deprecated` is at or before the
  version asked for.
- `FixCode` exposes `name`, `value`, `description`, `since`, `deprecated`,
  `ep`, `sort`, `group` and an owning `to_owned() -> FixEnumValue`.
- `FixFieldMut::set_codes(&[FixEnumValue])`, `remove_codes`, and a
  `try_with_codes` mirroring the existing `try_with_ascii_enum` shape.

The borrowed scan is only safe because the writer's rendering is canonical
and validated on the way in; say so in the doc comment, and pin it with a
test that a hand-edited document with reordered keys is refused rather than
mis-scanned.

Metadata values may not contain control characters
(`rust/src/metadata.rs:1293`), which is why this is JSON with escaping and
not a separator-delimited record text.

### Tests, benchmark, docs

`SideCodeSet` from the FIX Latest repository is the fixture: `Buy`=1,
`Sell`=2 added FIX.2.7 updated EP254, `Undisclosed`=7 added FIX.4.1,
`CrossShort`=9 added FIX.4.2, `CrossShortExempt`=A added FIX.4.3. Assert:
lookup by value, by name, by name folded, an alias pair sharing a value, the
version filter hiding `CrossShort` at `4.1`, a deprecated code hidden at and
after its version, canonical JSON round trip, and a malformed document
naming its byte position. Add an allocation case: `code()` on a 300-code set
allocates nothing. Add a bench group to `rust/benchmarks/fix/` for
`code()` against a `HashMap` baseline built from the same document, so the
scan is defended with a number. Document on `docs/fix.md`.

---

## Phase 5 - `FixFieldMut::merge_with`

Needs Phases 3 and 4 to know what it is merging.

`registry.rs:866`'s private `merge` is the whole current story and it is
expensive: `Metadata::merge_with` builds a new `Metadata`, `set_metadata`
walks it into the field, then `fix:tags` and `fix:aliases` are each read
back and rewritten - three metadata rewrites and a `Vec<String>` of every
key. `ProtocolFieldMut::merge_with` (`field/protocol/mod.rs:466`) is worse
on this path: it collects every held property name into an owned `String`
per key, then does an O(n*m) scan.

Replace both, for FIX, with one pass that knows the `fix:` namespace is
small and fixed:

```rust
impl FixFieldMut<'_> {
    /// Folds another definition of the same field into this one.
    pub fn merge_with(&mut self, other: &FixField<'_>) -> Result<()>;
}
```

Per-key rules, each one explicit because "merge" alone decides nothing:

| key | rule |
| --- | --- |
| `fix:branch`, `fix:tag` | must agree; a disagreement is a typed refusal naming both, because identity is not merged |
| `fix:tags` | union, incoming first, order kept, deduplicated |
| `fix:aliases` | union, ASCII-folded comparison, incoming first, order kept - then **rewritten from the merged lineage** so Phase 3's derivation still holds |
| `fix:description` | **never compared.** A description is the longest value a field carries and comparing two of them costs more than the write it would save: incoming wins when it has one, stored is kept when it does not |
| `fix:lineage` | merged by `since`: entries union, incoming wins an equal `since`, result re-sorted ascending and re-validated against the merged field's own name and datatype |
| `fix:codes` | merged by wire value: incoming wins a shared value, stored keeps codes only it has, pedigree carried through; the result is re-rendered canonically once |
| any other `fix:` key | incoming wins, stored keeps what only it has |

Everything else the merge must guarantee:

- **One metadata write.** Build the merged map, write it once, never touch
  the field between reads. The current three rewrites and their
  `invalidate_arrow` calls collapse into one.
- **No key-name allocation.** The `fix:` key set is a `const` list in
  `fix/field.rs`; walk it, never collect held names into `String`s.
- **Atomic.** A refusal leaves the field exactly as it was, which is what
  every other mutation in this repo promises.
- **Precedence is the caller's ordering, not a field on the merge.** Several
  sources describe one tag - FIX Latest, a QuickFIX dictionary, a vendor
  orchestration - and yggfin resolves that with a `priority` per source in
  `sources.json`. Do not add a priority to the core: the generator merges
  lowest priority first, so the highest-priority source is the last
  `incoming` and wins by the rule already stated. One concept, in the one
  place that knows about sources.
- `FixRegistry::update` calls it and the private `merge` at `registry.rs:866`
  is deleted - no second merge path survives, per the no-compatibility rule.

Tests: every rule above as its own case, including two fields whose
descriptions differ and are both long (assert the incoming's survives and
that nothing else moved); a tag disagreement refused with both sides named;
a merge that leaves the field byte-identical when the incoming adds nothing.
Add an allocation case bounding a merge of two realistic fields, and a bench
group comparing the new merge against the deleted one's behaviour over the
`config/fix` dictionary.

---

## Phase 6 - the source: FIX Latest into `config/fix`

Needs Phases 1, 3 and 4. This is the phase that fills the dictionary the
other five describe.

### Where the data comes from

The browsable rendering the work is checked against is
<https://orchimate.org/fixtrading/fix-latest> - FIX Latest as of EP309,
Orchestra v1.0 - with `/fields`, `/codeSets`, `/datatypes`, `/messages`,
`/components`, `/groups` and `/revisions` indexes, field pages at
`/fields/<Name>` and code set pages at `/codeSets/<Name>CodeSet`. Its
"Orchimate MCP" is useful for interactive lookups while implementing.

The machine-readable source of record is the same content as Orchestra XML,
and that is what the script reads - HTML is never scraped:

| version | file |
| --- | --- |
| FIX Latest | `FIX Standard/OrchestraFIXLatest.xml` |
| FIX 4.4 | `FIX Standard/OrchestraFIX44.xml` |
| FIX 4.2 | `FIX Standard/OrchestraFIX42.xml` |

all under
`https://raw.githubusercontent.com/FIXTradingCommunity/orchestrations/master/`
(percent-encode the space). The versions Orchestra does not publish - 4.0,
4.1, 4.3, 5.0, 5.0SP1, 5.0SP2 - come from the QuickFIX data dictionaries at
<https://github.com/quickfix/quickfix/tree/master/spec> (`FIX40.xml` …
`FIX50SP2.xml`), which carry field names, types and enum values per version:
enough for lineage, and the only public per-version set that is.

### What the Orchestra XML gives

Verbatim from `repositorytypes.xsd`, the pedigree every element carries:

```xml
<xs:attributeGroup name="entityAttribGrp">
  <xs:attribute name="added" type="fixr:Version_t"/>
  <xs:attribute name="addedEP" type="fixr:EP_t"/>
  <xs:attribute name="updated" type="fixr:Version_t"/>
  <xs:attribute name="updatedEP" type="fixr:EP_t"/>
  <xs:attribute name="deprecated" type="fixr:Version_t"/>
  <xs:attribute name="deprecatedEP" type="fixr:EP_t"/>
  <xs:attribute name="replaced" type="fixr:Version_t"/>
  <xs:attribute name="replacedEP" type="fixr:EP_t"/>
</xs:attributeGroup>
```

and the two element shapes this phase reads:

```xml
<xs:complexType name="fieldType">
  <xs:attribute name="id" type="fixr:id_t"/>
  <xs:attribute name="name" type="fixr:Name_t"/>
  <xs:attribute name="type" type="fixr:Name_t"/>
  <xs:attribute name="codeSet" type="fixr:Name_t"/>
  <xs:attribute name="minInclusive" type="xs:string"/>
  <xs:attribute name="maxInclusive" type="xs:string"/>
  <xs:attribute name="presence" type="fixr:presence_t"/>
  <xs:attribute name="value" type="xs:string"/>
</xs:complexType>

<xs:complexType name="codeType">
  <xs:attribute name="value" type="xs:token" use="required"/>
  <xs:attribute name="sort" type="xs:nonNegativeInteger"/>
  <xs:attribute name="group" type="xs:string"/>
</xs:complexType>
```

`fixr:codeSet` carries `type` plus the id, name and pedigree from the entity
base; documentation hangs off `fixr:annotation`/`fixr:documentation` on
fields, code sets and individual codes.

`added="FIX.2.7"` and `updatedEP="254"` are exactly Phase 1's `Version` and
Phase 4's `ep`. `replaced`/`replacedEP` is the rename axis, but FIX Latest
records only current names - the historical spelling lives in the
per-version files, which is why the script reads all of them.

### The script

`scripts/build_fix_registry.py`, beside the repo's existing Python tooling
(`check_docs_examples.py`, `check_avro_interop.py`). **Not crate code**: the
core has no HTTP client and must not gain one; the dictionary is a build
input, generated and committed, and the crate only ever reads
`config/fix/**.json` through `FixRegistry::from_handle`.

- **Every source URL is pinned to a commit, never `master`.** yggfin's
  `data/fix/sources.json` does exactly this - the FIX Latest file at
  `.../orchestrations/099914dd0edd49a699326f0441776d6e21cfaf93/FIX%20Standard/OrchestraFIXLatest.xml`
  and the QuickFIX one at `.../quickfix/3536699e830e65f875df4a50b647a6d3bad3b884/spec/FIX50SP2.xml`
  - because a dictionary regenerated from `master` is not reproducible and
  its diff is unreviewable.
- `--source` takes a local directory or a URL base so a run is reproducible
  offline; the default is the pinned set.
- Output is exactly the shard layout `from_handle` reads:
  `config/fix/{primitive,nested}/<branch>/<tag/100>.json`, each a JSON array
  of core field documents ordered by canonical identifier - byte-identical
  to what `write_into` would produce, asserted by a test that loads the
  generated tree and writes it back.
- Provenance goes in `config/fix/sources.json`, one record per source,
  modelled on yggfin's: `source_id`, `format` (`orchestra` or `quickfix`),
  `url` (pinned), `sha256` of the bytes fetched, `sha256` of the definitions
  produced from them, `branch` (yggfin calls it `namespace`), `version`
  label, `priority`, and **`license_url`**. The licence field is not
  bookkeeping: this commits a derived copy of the FIX Trading Community's
  and QuickFIX's material into the repository, and the attribution has to
  travel with it. The two checksums also give CI a drift test that needs no
  network for the second half. `from_handle` descends only `primitive/` and
  `nested/` (`store.rs:193`), so a leaf beside them is never listed and
  never refused; it must not be named `records`, which is the retired-layout
  tripwire at `store.rs:181`.
- **A vendor branch is a first-class source, not a hypothetical.** Community
  and vendor orchestrations are published through Orchestra Hub -
  `https://orchestrahub.org/api/v3/repos/<owner>/<repo>/revisions/<id>/download`
  - which is how yggfin loads its `fixtrading-udf` and `clear-street`
  namespaces. Pull one such dictionary into a non-standard `FixBranch` in
  the same run, so the branch machinery, Phase 2's digest table and Phase
  7's branch inference all have real data under them rather than a fixture.
- Datatypes resolve through `DataType::LOGICAL_NAMES`, which is already the
  FIX Latest datatype table (`docs/datatype.md`), so `Qty` is
  `decimal64(18,8)` and `SeqNum` is `int64` with no second mapping. A FIX
  datatype the table does not hold is a hard failure of the script, not a
  silent `utf8`.
- Repeating groups become the nested tree exactly as they do today: a List
  of a non-null `item` Struct whose `fix:tag` is the counter's.

### The standard header and trailer

`FixMsg` lays a message out flat (Phase 7), so the two components the
specification wraps every message in are carried here as **order**, not as
nesting.

FIX Latest's `StandardHeader` is 28 fields plus the `HopGrp` group, in this
order: 8 BeginString, 9 BodyLength, 35 MsgType, 1128 ApplVerID, 1156
ApplExtID, 1129 CstmApplVerID, 49 SenderCompID, 56 TargetCompID, 115
OnBehalfOfCompID, 128 DeliverToCompID, 34 MsgSeqNum, 50 SenderSubID, 142
SenderLocationID, 57 TargetSubID, 143 TargetLocationID, 116 OnBehalfOfSubID,
144 OnBehalfOfLocationID, 129 DeliverToSubID, 145 DeliverToLocationID, 43
PossDupFlag, 97 PossResend, 52 SendingTime, 122 OrigSendingTime, 212
XmlDataLen, 213 XmlData, 347 MessageEncoding, 369 LastMsgSeqNumProcessed.
`StandardTrailer` (component id 1025) is 10 CheckSum alone in FIX Latest -
but 4.2 and 4.4 put 93 SignatureLength and 89 Signature ahead of it, so the
**cross-version trailer is those three in that order** and each field's own
lineage says which versions it existed in. The same rule covers the header,
and the union is wider than FIX Latest's 28: yggfin's `versions.json`
records 90 SecureDataLen and 91 SecureData in the 4.0 through 4.4 headers,
which FIX Latest no longer lists at all. Generating the constant from FIX
Latest alone would silently drop them, so it is generated from **every**
scraped version, in canonical order, with `defined_at` deciding what a given
version may carry.

yggfin also stores a `required` flag per session field per version. Do not
add a parallel table for it: presence is already `nullable` on the field a
version resolves to, so the flag belongs in the lineage entry Phase 3
already writes, and a test asserts the generated header matches yggfin's
`versions.json` ordering and required flags for 4.2 and 4.4.

Both land as generated `const` tag lists in `rust/src/fix/header.rs`:

```rust
pub const STANDARD_HEADER_TAGS:  &[i32] = &[8, 9, 35, 1128, /* … */ 369];
pub const STANDARD_TRAILER_TAGS: &[i32] = &[93, 89, 10];
```

with `FixRegistry::standard_header_tags()` and `standard_trailer_tags()`
reading them. They are **not registry entries**: a component has no tag and
`insert` admits only a field carrying one (`registry.rs:494`), and inventing
a synthetic tag to smuggle a component in would put a fiction in the
identity space. The component names `StandardHeader` and `StandardTrailer`
and their ids are dropped, and the module docs name that loss. Every field
*in* them is an ordinary registry entry by its own tag, which is all a flat
layout needs.

The generator writes the constants and their source EP in the same run that
writes the shards, and a test asserts every tag in both constants resolves
in the generated `config/fix`.

### The worked case

Tag 32, end to end, and the test that proves the whole brief:

- FIX Latest says id 32, name `LastQty`, type `Qty`, added `FIX.2.7`.
- `OrchestraFIX42.xml` and `FIX42.xml` say tag 32 is `LastShares`, type
  `Qty` in 4.2 and `int` in 4.0/4.1.
- The generated field is named `LastQty`, datatype `decimal64(18,8)`,
  `fix:tag` 32, `fix:aliases` containing `LastShares`, and a `fix:lineage`
  of `{"since":"2.7","name":"LastShares","type":"int"}`,
  `{"since":"4.2","name":"LastShares","type":"Qty"}`,
  `{"since":"4.3","name":"LastQty","type":"Qty"}`.

A Rust test in `rust/src/fix/tests.rs` loads `config/fix` and asserts every
line of that, so the committed dictionary and the accessors are checked
together.

Document the provenance, the version coverage, the FIXT omission and the
regeneration command on `docs/fix.md`.

---

## Phase 7 - the halves made explicit, `FixEntry`, and `FixMsg::from_pairs`

Needs Phase 2 for the identifier, Phase 3 for the version, and Phase 6 for a
dictionary to resolve against.

### The registry's two halves become public

`position_by_id` (`registry.rs:703`) and `position_by_name`
(`registry.rs:715`) each hard-code the same four-way probe: primitive
canonical, nested canonical, primitive alternate, nested alternate. The
chain is written twice and no caller can ask for one half.

Expose the halves and compose the generic accessor out of them:

```rust
pub fn get_primitive_field<'k>(&self, key: impl Into<FixKey<'k>>) -> Option<&Field>;
pub fn primitive_field<'k>(&self, key: impl Into<FixKey<'k>>) -> Result<&Field>;
pub fn get_nested_field<'k>(&self, key: impl Into<FixKey<'k>>) -> Option<&Field>;
pub fn nested_field<'k>(&self, key: impl Into<FixKey<'k>>) -> Result<&Field>;
```

Each probes exactly one half through both tiers - canonical first, alternate
only on a miss - and `get_field` becomes
`get_primitive_field(key).or_else(|| get_nested_field(key))`, so the
half-order rule lives in one line instead of being retyped in two private
chains. `get_field_by_tag`, `get_field_by_id`, `get_field_by_name` and
`get_field_by_path` redirect the same way; none of them keeps a probe chain
of its own.

This is not tidying. A transcriber resolving a wire tag wants a scalar and
nothing else, and today an unknown tag pays all four probes: the published
numbers are 32.3 ns for a primitive hit against 72.2 ns for a miss, so the
nested half is most of what a miss costs. `from_pairs` below asks only for
`get_primitive_field`, and an unknown tag costs one probe.

Tests: each accessor answers only from its half over a registry holding a
scalar and a group that would both match a key; `get_field` answers exactly
what it answered before over every existing case; the miss cost drops in the
`resolve` bench group, and `docs/fix.md`'s table gains the two half-probe
rows.

### `FixEntry`: one wire pair, resolved

`rust/src/fix/entry.rs`:

```rust
pub struct FixEntry<'a> {
    /// The decimal FIX tag.
    pub tag: i32,
    /// `xxh32` of the branch name, or `None` for the standard branch.
    pub branch: Option<i32>,
    /// The value exactly as it arrived. Never absent - an absent field is an
    /// entry that does not exist.
    pub value: &'a str,
}
```

- The digest is the same `xxh32` Phase 2's `FixBranch` already caches,
  reinterpreted to `i32`; `FixEntry::id()` folds it into a `FixId` with one
  shift-or, so an entry addresses the registry without hashing anything.
- `None` is the standard branch *and* "not resolved yet", which is what
  makes the type usable before `from_pairs` has decided a branch. That is
  the whole reason it is not simply a `FixId`.
- 32 bytes: `Option<i32>` costs 8, not 4, because `0` is a legal digest and
  the niche is therefore unavailable. A bare `i32` defaulting to the
  standard digest would be 24 and lose the unresolved state; the extra word
  buys that state and is worth it here.
- The value stays `&'a str` and is never typed inside the entry. Typing
  happens once, in `from_pairs`, through `Field::scalar` - the one value
  contract - so no FIX value parser is written twice.

### `FixMsg::from_pairs`

```rust
pub fn from_pairs<'a, I>(
    registry: Arc<FixRegistry>,
    entries: I,
    branch: Option<&FixBranch>,
    version: Option<Version>,
) -> Result<Self>
where
    I: IntoIterator<Item = (&'a str, &'a str)>;
```

**What a key may be.** yggfin's `python/tests/fix/test_pairs.py` is the
list, and every line of it is a shape a venue actually sends:

| key | means |
| --- | --- |
| `54`, `"54"` | a tag, through the strict `parse_tag` at `fix/field.rs:44`, which refuses `+35` and `3x` |
| `Side`, `side`, `SIDE`, `" Side "` | a name, trimmed and folded |
| `msg_type`, `msg-type`, `Msg Type` | the same name: **separators fold away too** |
| `Instrument.Symbol` | a path, resolved by the `get_field_by_path` that already exists |
| `PartyID[0]`, `PartyID[1]` | one field, two occurrences, in order |
| `NoPartyIDs[0].PartyID` | a group entry: which group, which occurrence, which member |
| `VenueOwnThing` | an unknown name, **kept** |
| `""`, `"   "` | dropped |

Two of those change rules stated earlier in this brief:

- **Separator folding.** The registry folds ASCII case only today
  (`fix/mod.rs`). A renderer that emits `msg_type` or `Msg Type` then misses
  a field that exists. Extend the FIX name fold to drop `_`, `-` and space -
  which is not a new rule but the fold `DataType`'s logical names already use
  (`datatype/parser.rs:1656`), so one folding rule serves both. No two FIX
  fields differ only by a separator, so nothing collides; assert that over
  the generated `config/fix` as the test that lets the change in.
- **An unknown name is kept, not refused.** The earlier draft made it a
  typed error. That is wrong for the reason yggfin states in one line:
  every venue sends fields no dictionary has, and dropping them loses data.
  Keep it as a nullable `utf8` field under its own spelling, exactly as an
  unknown *tag* is already kept under its decimal one. Resolved fields come
  first in the root, unknown ones after, so the schema stays stable when a
  dictionary later learns the name.

**An empty value drops its pair.** `54=` is a malformed message, not an
absent side, so a pair whose value is empty never becomes a field.

**Order and repetition are the message.** A tag appearing twice stays two
entries in input order - a map keyed by tag would silently lose a repeating
group - and Phase 3's duplicate-name suffix rule names the second and later
children.

**Inferring the version**, when the caller names none, in this order, and
each step is a FIX rule rather than a heuristic:

1. tag **1128 `ApplVerID`**, the application version, whose code set is
   exactly `0`=FIX27, `1`=FIX30, `2`=FIX40, `3`=FIX41, `4`=FIX42, `5`=FIX43,
   `6`=FIX44, `7`=FIX50, `8`=FIX50SP1, `9`=FIX50SP2, `10`=FIXLatest. It wins
   because under FIXT.1.1 the session version says nothing about the
   application version. The symbolic spelling (`FIX44`) is accepted too,
   through Phase 4's `code_by_name`.
2. tag **8 `BeginString`**: `FIX.4.0` … `FIX.4.4` give `4.0` … `4.4`.
   `FIXT.1.1` is a session version and names no application version, so it
   falls through rather than being taken literally.
3. otherwise `Version::MAX` - FIX Latest, which is what a dictionary with no
   version marker means.

**Inferring the branch**, when the caller names none:

1. resolve every entry in `FixBranch::STANDARD`. Nothing missed means the
   branch is standard and there is no second pass - the common case costs
   one probe per entry.
2. otherwise retry only the *missed* tags against each branch the registry
   holds (`FixRegistry::branches()`, free from Phase 2's digest table) and
   take the branch resolving the most of them; a tie goes to the lowest
   branch name so the answer is deterministic; a branch resolving none is
   never chosen and its tags stay unknown.

A caller who passes a branch gets that branch and no guessing happens at
all.

**Building the message**, one pass over the resolved entries:

- the field is the registry's, cloned, with `name_at(version)` and
  `dtype_at(version)` when a lineage exists and the field's own name and
  datatype when it does not;
- it is non-null in this message's schema, because the value is present;
- the value is `field.scalar(Scalar::from(entry.value))?` and nothing
  re-checks what `scalar` already answered;
- order is `STANDARD_HEADER_TAGS`, then the body in entry order, then
  `STANDARD_TRAILER_TAGS` - flat, no `StandardHeader` Struct, which is what
  Phase 6 generates those constants for;
- `FixMsg::with_registry` finishes it, so the existing validation and
  canonicalization are not bypassed.

**An empty dictionary is a supported input, not an error.** With nothing
resolvable, every name stays a name, every tag stays its decimal spelling,
and `by_name` still finds what was put in - which is what makes the function
usable on a venue whose dictionary has not been loaded yet.

**The wire spellings belong to the FIX layer, never to `DataType::scalar`.**
The decode direction here takes text in; the encode direction that follows
has to put FIX's own spellings back out, and yggfin pins them: a float is
never exponent notation (`1e-7` writes as `0.0000001`), a `UTCTimestamp` is
`20260821-10:30:00.123456`, a date is `20260821`, a time is
`10:30:00.000000`, a boolean is `Y` or `N`. First **verify whether
`DataType::scalar` accepts a `Scalar::String` for `Boolean` and the
temporals at all** (`field/value.rs:112`); where it does not, the FIX layer
parses the wire spelling into the right `Scalar` before calling `scalar`,
and where it does, check the spelling it accepts is FIX's. Either way the
generic value contract learns no FIX spelling - `LOGICAL_NAMES` is the FIX
vocabulary yggdryl carries, and that is deliberately a *type* table, not a
*value* one.

**The two ways in must agree.** Whatever `from_pairs` builds has to read
back identically once a wire parser exists - yggfin pins this as
`from_text(built.into_text("|")) == built`. The wire parser is a later
phase; write the invariant into the module docs now so it is not discovered
as a contradiction later.

**Repeating groups are in scope, because the key carries the location.**
The earlier draft ruled them out on the grounds that finding a group's
boundary needs the message type's grammar. That is true only of a *bare*
tag stream. A key spelled `NoPartyIDs[0].PartyID` states the group, the
occurrence and the member, so no grammar is needed - and yggdryl already
has everything to build the result properly: a group is a List of a non-null
`item` Struct, and `Field::set_field_by_path` writes into one. So
`from_pairs` builds **real nesting** from indexed keys, where yggfin has to
keep a flat `comp` string beside the entry because its field model has no
list of structs to put it in.

What stays out of scope is inferring a group from repetition alone: bare
`448=A`, `448=B` with no index and no group key produces two sibling
occurrences of `PartyID`, not a reconstructed `NoPartyIDs`. Reassembling
*that* is the wire parser's job and it needs the grammar, which is the
`.cfb` phase's. Say so in the module docs.

### Optimization the phase is judged on

- One `Vec<FixEntry>`, one `Vec<Field>`, one `Vec<Scalar>`, each reserved
  from the iterator's `size_hint` before the walk. No per-entry `String`, no
  per-entry map.
- A tag-keyed entry allocates nothing before the value is typed.
- Only `get_primitive_field` is probed for scalars; the nested half is
  reached only for a counter tag.
- Header ordering reads a precomputed tag-to-position table, not a scan of
  `STANDARD_HEADER_TAGS` per entry.

### Tests, benchmark, docs

Cases: tag-keyed and name-keyed pairs producing the identical message;
`ApplVerID` beating `BeginString`; `BeginString="FIXT.1.1"` falling through
to Latest; an explicit `version` overriding both markers; branch inference
picking the vendor dictionary that resolves the misses, and the tie rule;
an explicit branch suppressing inference; an unknown tag kept as `utf8`
under its decimal name; an unknown name refused; tag 32 keyed as
`LastShares` in a `4.2` message and as `LastQty` in a Latest one, both
answering the same value; header and trailer ordering with a body field
interleaved in the input.

Then, straight from `test_pairs.py`, because they are the ones a first
implementation gets wrong: `" Side "`, `msg_type`, `msg-type`, `MSG_TYPE`
and `Msg Type` all reaching tag 35 or 54; `Instrument.Symbol` resolving
through the path; `PartyID[0]` and `PartyID[1]` staying two ordered
occurrences; `NoPartyIDs[0].PartyID` building a List of one `item` Struct
rather than a flat name; an unknown name surviving beside a known one, with
the known one first; an empty and a blank key dropped; an empty value
dropped; the same pairs built against an empty registry.

New bench group in `rust/benchmarks/fix/`: a NewOrderSingle of ~15 pairs, an
ExecutionReport of ~30, and a 300-pair message; tag-keyed against
name-keyed; branch and version given against inferred. Report per-message
and per-pair cost and put the table on `docs/fix.md`. Add a case to
`rust/tests/allocations.rs` bounding the allocations of a 30-pair tag-keyed
build to the three reserved vectors plus what the values themselves cost.

---

## Verification

Per `AGENTS.md`, before handoff on any phase: `cargo fmt`, warning-free
Clippy, workspace tests with default features and `parquet iceberg`, Rust
1.85 default and `--no-default-features --lib`, rustdoc with warnings
denied, the `fix` and `datatype` benches, `scripts/check_docs_examples.py`,
and `python -m mkdocs build --strict`. Report exact results and exact
skipped checks. Only Phase 2 touches the bindings, and only to keep them
compiling; every other phase is Rust-only.
