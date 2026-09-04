# FIX versioning, code sets, and the registry that carries them

Seven phases, in dependency order. Each is complete work: it compiles, its
tests pass, its docs are written, and the repository ships at its end.

## How to run this brief

**One phase per session, one phase per PR.** Do not start a phase whose
dependencies are not merged.

**Dependency order.** `1 → 3 → 4 → 5`; `1 + 3 + 4 → 6`; `2 + 3 + 6 → 7`.
Phase 2 depends on nothing.

**Before writing code:** read the files the phase's *Files* list names, then
re-read `AGENTS.md`. Every anchor here is `path:line` against the tree at
writing time; if a line moved, find the symbol - the rule did not change.

**Precedence.** `AGENTS.md` > this brief > your priors about how FIX is
usually done. A rule marked **Decided** is settled: implement it, and do not
relitigate - the rejected alternative is recorded so you need not rediscover
why it lost. A rule marked **Verify** means the check comes before the code.

**Rules are numbered** (`P4-R3`). Cite the number in commits, PR text and
review replies.

**Never, in any phase:**

- N1. Add a public symbol this brief does not name.
- N2. Add a dependency this brief does not name.
- N3. Add backward compatibility, a deprecated alias, a shim, or a second
  path to an existing behaviour. The repository has one current contract.
- N4. Store a fact that is already derivable, unless a rule says to *and*
  says why.
- N5. Widen a phase because the next one will need it.
- N6. Leave a `TODO`, an `#[allow]`, an ignored test, or a doc example that
  does not run under `scripts/check_docs_examples.py`.
- N7. Guess where this brief says refuse, or refuse where it says fall
  through.

**A phase is done when** all of this passes:

```console
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --workspace --features "parquet iceberg"
cargo check -p yggdryl --no-default-features --lib
RUSTDOCFLAGS="-D warnings" cargo doc -p yggdryl --no-deps
python scripts/check_docs_examples.py
python -m mkdocs build --strict
```

plus the benches the phase names, and you have reported exact results,
including anything skipped and why. A phase that misses a number it promised
reports the measurement; it does not drop the promise quietly.

---

## What is landed

| what | where |
| --- | --- |
| `FixBranch`, `FixId`, `FixKey` | `rust/src/fix/mod.rs` |
| `FixField` / `FixFieldMut`, `FixAliases` | `rust/src/fix/field.rs` |
| `FixRegistry`, `Half`, `insert`, `update`, private `merge` | `rust/src/fix/registry.rs` (`:225`, `:494`, `:703`, `:715`, `:866`) |
| `from_handle` / `write_into` | `rust/src/fix/store.rs` (`:181`, `:193`) |
| `FixMsg` | `rust/src/fix/msg.rs` |
| `AsciiEnum`, the `field:enum` document | `rust/src/datatype/ascii.rs:345`, `rust/src/field/mod.rs:895` |
| `DataType`, `LOGICAL_NAMES`, the name fold | `rust/src/datatype/`, `parser.rs:1656` |
| `DataTypeId` (wire ids) | `rust/src/generic/datatype_id.rs:275` |
| `Scalar` | `rust/src/generic/scalar.rs:751` |
| value contract `dtype_scalar` | `rust/src/field/value.rs:112` |
| `xxh32`, `xxh3_64`, streaming state | `rust/src/xxhash/mod.rs:111`, `:134`, `state.rs` |
| counting allocator (one target per process) | `rust/tests/allocations.rs` |
| seed dictionary | `config/fix/{primitive,nested}/standard/*.json` |
| numbers to beat | `docs/fix.md`, "Measured resolution cost" |

Two landed facts constrain several phases:

- **L1.** `FixRegistry::insert` admits only a field carrying `fix:tag`
  (`registry.rs:494`). Nothing without a tag enters the registry - not a
  component, not a message root, not a header.
- **L2.** `DataTypeId::as_u8` is `self as u8` and is a documented wire
  contract (`datatype_id.rs:275`). A new variant is **appended**, never
  inserted; `DataTypeId::ALL` grows 54 → 55.

---

## Prior art: `Platob/yggfin`

<https://github.com/Platob/yggfin> is a Python FIX stack (`rekep`) over the
same problem, further along. **Do not port its shapes.** It models
components, repeating groups and namespaces as separate directories and
hangs a flat `comp` string off an entry; yggdryl needs none of that - a
component is a Struct field, a group is a List of an `item` Struct, a branch
is a folder. Read it for the *use cases* it was forced to handle. They are
cited below wherever they changed a rule.

| file | what it settles |
| --- | --- |
| `python/tests/fix/test_pairs.py` | every key and value shape `from_pairs` meets (P7) |
| `python/tests/fix/test_message.py`, `test_transcribe.py` | wire token rules (P7) |
| `python/tests/fix/test_entries.py` | code spelling → value translation (P4) |
| `python/tests/fix/test_fields.py` | types the generator must not narrow (P6) |
| `data/fix/sources.json` | provenance: pinned commit, checksums, licence, priority (P6) |
| `data/fix/versions.json` | declared versions, per-version session field order (P3, P6) |
| `docs/fix/repeating-groups.md` | the ULBridge payload shape (P7) |

Clone read-only. It is not a dependency and nothing links against it.

---

## Phase 1 — `Version`: a generic value, datatype, scalar and field

**Goal.** A generic version value - major, minor, further numeric parts, an
optional qualifier - with its `DataType`, `Scalar` and `Field` support.

**Depends.** Nothing.

**Files.** *Create* `rust/src/generic/version.rs` (+ tests). *Edit*
`generic/{mod,datatype_id,typed,scalar}.rs`,
`datatype/{mod,parser,arrow,serde,scalar,default,merge,compatibility,tests}.rs`,
`field/{mod,typed,value,ascii}.rs`, `field/cast/{mod,plan}.rs`,
`arrow/value.rs`, `expression/{eval,parser}.rs`, `lib.rs`,
`rust/tests/allocations.rs`, `rust/benchmarks/datatype.rs`,
`docs/{generic,datatype}.md`.

**Never.** Touch `rust/src/fix/`. No FIX spelling, no `FIX.` prefix, no
`Latest` in this phase - a caller who has never heard of FIX must be able to
use `Version`.

### Contract

```rust
// rust/src/generic/version.rs, exported as crate::Version
pub struct Version {
    parts: [u16; Version::MAX_PARTS],   // MAX_PARTS = 4, major first
    used: u8,                           // parts the canonical spelling states
    qualifier: Option<Qualifier>,       // { text: SmolStr, pre: bool }
}
impl Version {
    pub const MIN: Self;   // 0
    pub const MAX: Self;   // every part u16::MAX, no qualifier
}
```

### Rules

- **P1-R1. Canonical on parse.** Trailing zero components are trimmed:
  `4.4.0` and `4.4` are one value with one spelling. `Display` re-renders
  exactly what `FromStr` accepts.
- **P1-R2. Grammar.** `major(.part)*` then an optional qualifier, which may
  be appended (`5.0SP2`), dot-introduced (`5.0.SP1`) or hyphen-introduced
  (`1.0.0-rc1`). A hyphen means *pre*-release; a dot or nothing means
  *post*-release. All three canonicalize to one spelling.
- **P1-R3. Why three forms.** One FIX version is really written four ways:
  Orchestra `FIX.5.0SP2`, yggfin `5.0.SP1`, the `ApplVerID` code set
  `FIX50SP1`, the session line `FIXT1.1`. A value with four renderings is
  four values.
- **P1-R4. Bounds and refusals.** A component is decimal and at most
  `u16::MAX`; at most `MAX_PARTS` of them. Over-long input, a non-decimal
  component and an empty qualifier are `Error::Parse` naming the byte
  position, as every other parser in the repo reports.
- **P1-R5. Ordering.** Components numerically, an unstated component reading
  zero; then qualifier class `pre < none < post`; then the qualifier by
  ASCII-folded alphabetic prefix and *numeric* suffix, so `SP2 < SP10`.
  `Ord`, `Eq` and `Hash` agree.
- **P1-R6. `MAX` is the "newer than anything named" sentinel.** Both bounds
  are `const`.
- **P1-R7. No allocation** on parse, compare or render for a qualifier
  inside `SmolStr`'s inline buffer - which every FIX and semver qualifier
  is.
- **P1-R8. Datatype.** `DataType::Version` beside `Guid`.
  `DataTypeId::Version` **appended last** (L2); `ALL` becomes `[Self; 55]`;
  `as_str` is `"version"`; `kind` is `DataTypeKind::String`;
  `fixed_byte_width` is `None`. Parser spelling `version`, no alias.
- **P1-R9. Arrow representation is `Utf8`,** the canonical text.
- **P1-R10. Variant sweep.** Follow the existing `Cfi` sweep:
  `datatype/{arrow,compatibility,default,merge,mod,parser,scalar,serde,tests}.rs`,
  `generic/{datatype_id,mod,typed}.rs`, `field/{ascii,mod,typed,value}.rs`,
  `field/cast/{mod,plan}.rs`, `arrow/value.rs`,
  `expression/{eval,parser}.rs`. Skip `datatype/ascii.rs` and
  `datatype/coded.rs` - a version is not an ASCII width and not a code.
  `iceberg/*` and `avro/*` reject it as they reject other types they cannot
  represent.
- **P1-R11. Value contract.** `DataType::scalar` accepts a `Scalar::String`
  that parses and **rewrites** it to `Scalar::Version`, accepts
  `Scalar::Version` unchanged, refuses everything else with expected/actual.
  Nothing re-checks what `scalar` answered.
- **P1-R12. Casts.** `Version → Utf8` renders, `Utf8 → Version` parses,
  `Version → Version` is identity. No numeric casts.

### Decided

- **Utf8 over a fixed-width packing.** A qualifier has no length bound and a
  lossy Arrow round trip is unacceptable. *Cost:* Arrow-side lexicographic
  order is **not** version order. Say so in the docs, demonstrate it in a
  test, and make `Ord` on `Version` the only ordering contract.
- **Post-release default.** `5.0SP2` sorts *after* `5.0` because FIX service
  packs are post-releases. Semver pre-release ordering is reachable through
  the hyphen form, and only through it.

### Tests

1. The grammar, including every refusal with its byte position.
2. Trailing-zero canonicalization; `Display`/`FromStr` round trip.
3. The ordering table:
   `0 < 1.0 < 4.2 < 4.4 < 5.0-rc1 < 5.0 < 5.0SP1 < 5.0SP2 < 5.0SP10 < MAX`.
4. Four spellings of one version parsing equal: `5.0SP1`, `5.0.SP1`,
   `FIX.5.0SP1` (through P3's prefix strip), `FIX50SP1` (through the
   `ApplVerID` code set).
5. serde round trip; Arrow round trip through `Field`.
6. `DataType::scalar` rewriting a `Scalar::String`.
7. Allocation case: parse, compare and render allocate nothing.

**Bench.** Parse and compare, in `rust/benchmarks/datatype.rs`.
**Docs.** The value on `docs/generic.md`; the datatype row on
`docs/datatype.md`.

---

## Phase 2 — `FixId` is one `i64`

**Goal.** Pack the identifier into eight bytes so it is its own hash key.

**Depends.** Nothing. Run it first, or in parallel with Phase 1.

**Files.** *Edit* `rust/src/fix/{mod,field,registry,store,msg}.rs`,
`rust/src/fix/tests.rs`, `python/src/fix.rs`, `node/src/fix.rs`,
`rust/tests/allocations.rs`, `rust/benchmarks/fix/resolve.rs`,
`docs/fix.md`.

**Never.** Change a serialized shape. `FixId` is derived and never stored,
so a shard written before this phase must load and round-trip
byte-identically after it. That is the test that says the phase is safe.

### Contract

```rust
pub struct FixId(i64);            // ((tag as i64) << 32) | i64::from(xxh32(branch))
pub struct FixBranch { text: SmolStr, digest: u32 }   // text declared first

impl FixId {
    pub const fn standard(tag: i32) -> Self;
    pub fn from_parts(branch: &FixBranch, tag: i32) -> Result<Self>;
    pub const fn tag(self) -> i32;
    pub const fn branch_digest(self) -> u32;
    pub const fn is_standard(self) -> bool;
}
impl FixRegistry {
    pub fn branch_of(&self, id: FixId) -> Option<&FixBranch>;
    pub fn branches(&self) -> impl Iterator<Item = &FixBranch>;
}
```

### Rules

- **P2-R1. `i64`, not `u64`.** A tag is `i32` in `0..=i32::MAX`, so bit 63
  is never set, every identifier is positive, and `Ord` on the `i64` is the
  natural order of the packed pair. The digest zero-extends, so the low half
  compares unsigned.
- **P2-R2. `Copy`, 8 bytes,** `Hash` and `Ord` without touching the heap.
  `FixKey::Id(FixId)` stops borrowing; `next_field_after` takes
  `Option<FixId>` by value.
- **P2-R3. `standard(tag)` becomes `const fn`.** The doc comment
  apologising for `SmolStr`'s `Drop` goes with the field.
- **P2-R4. The branch caches its own digest.** `xxh32`
  (`xxhash/mod.rs:111`) runs once, in `FixBranch::from_str`, the only
  constructor. `text` is declared first so derived `Eq`/`Ord` stay
  text-based; the digest is a function of the text so it can only agree.
  `MAX_LENGTH` stays 23 - the digest sits beside `SmolStr`, not inside it.
- **P2-R5. `FixBranch::STANDARD`'s digest is a literal**, pinned by a test
  asserting `xxh32(b"standard")` equals it.
- **P2-R6. `from_parts` is a shift and an or,** keeping the standard-tag
  rule it already owns plus one new refusal: a non-standard branch whose
  digest equals the standard branch's is rejected there, so `is_standard()`
  stays total.
- **P2-R7. `FixId::branch()` is deleted.** Its eight callers
  (`field.rs:195`, `registry.rs:496,499,566,740`, `store.rs:261,266,305`)
  all hold the owning field and read `fix:branch` from it, which is where
  the text lives. `branch_digest()` replaces it where only identity matters.
- **P2-R8. `Display`** renders `standard:35` for the standard branch and
  `#7f3a1c02:5001` - digest in lowercase hex - for any other. `from_str`
  still accepts `cme:5001`: it has the text and hashes it. The `fix/mod.rs`
  doctest asserting `from_str("CME:5001").to_string() == "cme:5001"` changes
  with it.
- **P2-R9. The registry keeps `branches: HashMap<u32, FixBranch>`,** filled
  on insert, so every refusal it raises still names `cme:5001`. Only an
  identifier held outside a registry renders as hex.
- **P2-R10. A branch-digest collision is a typed conflict at insert,**
  naming both spellings and the digest. A 32-bit space over a handful of
  branches, but a stated failure rather than two dictionaries silently
  aliasing.
- **P2-R11. The indexes.**

  ```rust
  ids:             HashMap<FixId, usize, BuildHasherDefault<Mix>>
  alternate_ids:   HashMap<FixId, usize, BuildHasherDefault<Mix>>
  names:           HashMap<u64,   usize, BuildHasherDefault<Mix>>
  aliases:         HashMap<u64,   usize, BuildHasherDefault<Mix>>
  positions_by_id: Vec<usize>   // ordered by FixId, iteration only
  ```

  An identifier lookup now hashes nothing: the key *is* the id.
- **P2-R12. `Mix` finalizes, it does not pass through.** The packed high
  bits are the tag, under 65536 for nearly every FIX field, so the top bytes
  are near-constant - and hashbrown takes its control byte from the top
  bits. A raw pass-through puts every standard field in one control-byte
  class. `Mix` applies one multiply-xor-shift finalizer.
- **P2-R13. Name and alias keys stay text,** hashed per probe: ASCII-fold
  into the streaming xxh3 state (`xxhash/state.rs`) in stack-sized chunks so
  no length allocates, seeded with the branch's own `xxh32` digest so a name
  cannot be found under another branch, with a distinct constant seed per
  index.
- **P2-R14. A name-digest collision is a loud refusal, never a wrong
  answer.** Two names mapping to one `u64` would silently overwrite in a
  `HashMap<u64, _>`. `insert` verifies the field at an occupied key really
  holds it and returns a typed conflict otherwise; reads re-check the same
  way, so a collision degrades to a miss. Identifier keys need no such
  check - a `FixId` *is* the key, and its only collision is P2-R10's.
- **P2-R15. Ordered iteration keeps its own structure.**
  `next_field_after`, `FixFieldIter`, `Debug`, `PartialEq` and
  `store::write_into` need an ordered walk a hash map cannot give.
  `positions_by_id` is a `Vec<usize>` kept sorted by binary-search insert:
  `O(n)` per insert on a dictionary built once and read forever, against
  `O(log n)` node chasing on every read.
- **P2-R16. Order becomes tag-major.** Within one branch it is unchanged,
  so `write_into` still produces **byte-identical shards** - a shard folder
  is one branch, a shard file one `tag / 100` bucket. What moves is the
  cross-branch walk: vendor fields interleave among standard ones by tag.
  Update the "orders branch-major" sentence in `fix/mod.rs`, the
  `next_field_after` doc, and the `FixFieldIter` docs; assert the new order
  in a test rather than leaving it to whichever map iterates first.
- **P2-R17. Binding impact is two call sites and mechanical.**
  `python/src/fix.rs` and `node/src/fix.rs` only parse an id from text and
  hold one as a cursor (`after: Option<CoreFixId>`, passed `.as_ref()`);
  neither renders one back. `Copy` makes those by-value.
  `STANDARD_TAG_LIMIT` is untouched. No new binding surface.

### Decided

- **A bare `FixId` cannot name its branch.** The digest is one-way. Paid
  explicitly through P2-R7/R8/R9. *Rejected:* a process-wide branch intern
  table so a bare id could render itself - it buys prettier `Debug` for a
  global lock or a leak on the hot path, and the registry already knows
  every branch it holds.

### Tests

1. Packing round trip (`from_parts` → `tag()`, `branch_digest()`) at tag
   bounds `0`, `STANDARD_TAG_LIMIT`, `i32::MAX`.
2. `standard(tag)` in a `const` context.
3. The pinned `xxh32(b"standard")` constant.
4. A branch-digest collision refused at insert, both spellings named.
5. A name-digest collision refused.
6. Ordering across tags and across branches.
7. `write_into` byte-identical to a shard written before the change.
8. Every existing registry test passes unchanged, except the two facts that
   genuinely moved: vendor-branch `Display`, and cross-branch iteration
   order.
9. `rust/tests/allocations.rs` stays green - lookups allocate nothing today
   and must still.
10. `Mix` control-byte spread over the `config/fix` dictionary, not just
    that lookups answer.

**Bench.** `cargo bench -p yggdryl --bench fix`, then **replace** the
`docs/fix.md` table. Numbers to beat: 32.3 ns primitive tag hit, 93.1 ns
nested tag hit, 65.8 ns alternate tag hit, 72.2 ns miss, 128.1 ns vendor
identifier hit over 1034 fields, 81.8 ns name hit.

---

## Phase 3 — FIX version handling: `fix:lineage`

**Goal.** One field answers its name and datatype at any FIX version.

**Depends.** Phase 1.

**Files.** *Edit* `rust/src/fix/{mod,field,registry}.rs`,
`rust/src/fix/tests.rs`, `docs/fix.md`.

**Never.** Add a second version key, a registry-wide default version, or a
message transcoder. This phase carries facts; it does not move values.

### Contract

```rust
impl FixField<'_> {
    pub fn lineage(&self) -> FixLineage<'_>;              // lazy, borrowed
    pub fn since(&self) -> Option<Version>;
    pub fn until(&self) -> Option<Version>;
    pub fn defined_at(&self, at: &Version) -> bool;
    pub fn name_at(&self, at: &Version) -> Option<&str>;
    pub fn dtype_at(&self, at: &Version) -> Result<Option<DataType>>;
}
impl FixFieldMut<'_> { pub fn set_lineage(&mut self, …) -> Result<()>; }
impl FixRegistry {
    pub fn field_at<'k>(&self, at: &Version, key: impl Into<FixKey<'k>>) -> Result<&Field>;
    pub fn get_field_at<'k>(&self, at: &Version, key: impl Into<FixKey<'k>>) -> Option<&Field>;
    pub fn versions(&self) -> Vec<Version>;
}
```

`fix:lineage` is the one new metadata key, a JSON document rendered
canonically the way `AsciiEnum::into_json` is - fixed key order, no
whitespace, one text per value:

```json
{"entries":[
  {"since":"2.7","name":"LastShares","type":"int"},
  {"since":"4.3","name":"LastQty","type":"Qty"}
]}
```

### Rules

- **P3-R1. FIX spellings map onto `Version` in `rust/src/fix/`:** `FIX.4.2`
  → `4.2`, `FIX.5.0SP2` → `5.0SP2`, `FIX.2.7` → `2.7`, `FIX.Latest` →
  `Version::MAX`. The `FIX.` prefix is the family and is not stored.
- **P3-R2. FIXT.1.1 is not modelled.** Session tags carry the application
  version that first defined them. `docs/fix.md` names FIXT as a known
  omission.
- **P3-R3. `since` is a `Version`;** entries are **oldest first** and no two
  share a `since`, so the newest reading is the last written and resolution
  is a scan that stops, not a sort.
- **P3-R4. `name` is the spelling from that version on.**
- **P3-R5. `type` is the FIX datatype name from that version on,** in the
  spelling `DataType::from_str` already resolves (`Qty`, `int`,
  `UTCTimestamp`), so the decoder needs no second table and the document
  stays readable.
- **P3-R6. `deprecated: true` and `removed: true`** mark the states the
  specification gives them; a `removed` entry ends the field's life at that
  version. A version that simply stops naming a field has removed it - the
  generator writes that entry, a reader never infers it.
- **P3-R7. Every key beyond `since` is optional.** An entry stating only
  `since` means "present, unchanged", which most versions are.
- **P3-R8. The lineage is the authority; two derivations are computed by the
  writer so they cannot drift.** (a) The field's own `name()` and `dtype()`
  MUST equal the newest entry's - `set_lineage` refuses a disagreement,
  naming both sides. (b) `fix:aliases` is rewritten from the lineage's
  historical names on the same call, so `registry.field("LastShares")`
  resolves through the index that already exists.
- **P3-R9. No `fix:since`, `fix:until` or `fix:deprecated` key.** `since()`
  is the first entry's `since` and `until()` the `removed` one's, derived on
  read the way `FixId` is derived from branch and tag (N4).
- **P3-R10. The registry stays version-agnostic.** It holds every tag ever
  defined; a version is a **filter on the read**. That is what "defined in
  one version, available in the others" means. No registry-wide default
  version; a caller who wants one holds a `Version` beside the registry.
- **P3-R11. `versions()` is derived,** not stored - it is yggfin's
  `versions.json` `declared` list, and deriving it means a dictionary cannot
  claim a version no field is dated in.
- **P3-R12. The transcoding boundary.** The lineage carries enough to rename
  and retype a value between two versions. Actually rewriting a message -
  walking a root `Field`, renaming children, casting values - belongs to the
  `.cfb` `normalization-binding` phase. Do not start it. State the boundary
  in the module docs.

### Tests

Tag 32 is the worked case, verbatim in `rust/src/fix/tests.rs`:

1. `since()` is `2.7`.
2. `name_at(4.2)` is `LastShares`; `name_at(4.3)` and `name_at(MAX)` are
   `LastQty`.
3. `dtype_at(4.0)` is `Int32`; `dtype_at(4.4)` is `decimal64(18,8)`.
4. `registry.field("LastShares")` and `registry.field("LastQty")` are the
   same field.
5. `field_at(&"4.2", "LastQty")` refuses.
6. A lineage disagreeing with the field's own name is refused (P3-R8a).
7. A field with no lineage answers `None` everywhere and resolves as before.
8. Canonical JSON round trip; a malformed document names its byte position.
9. `fix:aliases` matches the lineage projection for every field in
   `config/fix` (P3-R8b).

**Docs.** A new section on `docs/fix.md`; a `fix:lineage` row in the
property table at the top of `rust/src/fix/mod.rs`.

---

## Phase 4 — code sets: `FixEnumValue`, `fix:codes`, and spelling translation

**Goal.** A field carries its FIX code set, and any spelling of a code
reaches the wire value.

**Depends.** Phase 1 (pedigree versions), Phase 3 (version filter).

**Files.** *Create* `rust/src/fix/enums.rs`. *Edit*
`rust/src/fix/{mod,field}.rs`, `rust/src/fix/tests.rs`,
`rust/tests/allocations.rs`, `rust/benchmarks/fix/` (new group),
`docs/fix.md`.

**Never.** Touch `AsciiEnum` or the `field:enum` document.

### Contract

```rust
pub struct FixEnumValue {
    name: SmolStr,                 // symbolic name, "Buy"
    value: SmolStr,                // wire value, "1"
    description: Option<SmolStr>,
    aliases: Vec<SmolStr>,         // venue spellings, per-version spellings
    since: Option<Version>,        // Orchestra `added`
    deprecated: Option<Version>,   // Orchestra `deprecated`
    ep: Option<u32>,               // Orchestra `updatedEP`
    sort: Option<u32>,             // Orchestra `sort`
    group: Option<SmolStr>,        // Orchestra `group`
}

impl FixField<'field> {
    pub fn codes(&self) -> FixCodes<'field>;
    pub fn code(&self, value: &str) -> Option<FixCode<'field>>;
    pub fn code_by_name(&self, name: &str) -> Option<FixCode<'field>>;
    pub fn code_at(&self, at: &Version, value: &str) -> Option<FixCode<'field>>;
    pub fn code_value(&self, text: &str) -> Option<&'field str>;
    pub fn code_name(&self, value: &str) -> Option<&'field str>;
    pub fn code_value_at(&self, at: &Version, text: &str) -> Option<&'field str>;
    pub fn code_name_at(&self, at: &Version, value: &str) -> Option<&'field str>;
}
impl FixFieldMut<'_> {
    pub fn set_codes(&mut self, codes: &[FixEnumValue]) -> Result<()>;
    pub fn remove_codes(&mut self) -> Result<Option<Vec<FixEnumValue>>>;
}
```

`fix:codes`, canonical JSON, the code set's own name and id beside its
values:

```json
{"name":"SideCodeSet","id":54,"codes":[
  {"name":"Buy","value":"1","since":"2.7","ep":254,"doc":"Buy; …"},
  {"name":"Sell","value":"2","since":"2.7","ep":254}
]}
```

### Rules

- **P4-R1. Ordered by wire value,** so one code set is one text however it
  was built.
- **P4-R2. Two names may share a value** - that is an alias, the rule
  `AsciiEnum` already states. Two entries may not share a name.
- **P4-R3. Nothing on the hot path builds the whole map or allocates.**
  `codes()` is a lazy iterator of borrowed `FixCode<'field>` views, built
  the way `FixAliases` is built; `code(value)` scans and stops, using
  `memchr` for record boundaries rather than parsing JSON structurally.
- **P4-R4. The borrowed scan is safe only because the writer's rendering is
  canonical and validated on the way in.** Say so in the doc comment, and
  pin it: a hand-edited document with reordered keys is refused, not
  mis-scanned.
- **P4-R5. `code_value` resolves in this order.**
  1. **The text as a wire value, exactly.** `4` is `4`. A spelling that is
     already a legal code is never reinterpreted as somebody's name. This is
     the early-exit fast path.
  2. **The folded symbolic name, then any alias.** The fold is the crate's
     one fold - casefold, then drop everything that is not a letter or a
     digit - so `PercentageWaivedCashDiscount`,
     `percentage_waived_cash_discount`, `PERCENTAGE WAIVED CASH DISCOUNT`
     and `percentage-waived-cash-discount` are one spelling.
  3. **The leading parenthesized abbreviation of the description.**
     `"Good Till Date (GTD)"` answers `gtd`.
- **P4-R6. Two traps in tier 3, both cases.** A *numeric* parenthesization
  is a tag cross-reference and never a spelling - `"Broken date; SettlDate
  (64) is required"` must leave `64` alone. Only the abbreviation attached
  to the leading phrase counts - `"Swap Value Factor (SVP) through a central
  counterparty (CCP)"` answers `svp`, not `ccp`.
- **P4-R7. An unresolved spelling falls through unchanged. Never an error.**
  `code_value` answers `None` and the caller keeps its text, because a venue
  sends codes no dictionary lists exactly as it sends fields no dictionary
  names.
- **P4-R8. An ambiguous spelling resolves to nothing.** Two codes folding to
  one spelling (`Cross`, `cross!`) make that spelling answer `None` rather
  than whichever the scan met first. So the name tier does **not**
  early-exit: it runs the whole code set and answers only on exactly one
  match. Affordable because tier 1 is the hot path and does early-exit; a
  spelling lookup comes from human or JSON input, not a wire stream.
- **P4-R9. Version-scoped.** `code_value_at` / `code_name_at` skip a code
  whose `since` is later, or whose `deprecated` is at or before, the version
  asked for. A 4.2 message cannot resolve a name added in 4.4.
- **P4-R10. JSON, not a separator-delimited record text,** because metadata
  values may not contain control characters (`rust/src/metadata.rs:1293`).

### Decided

- **`fix:codes` is a second key, not a second copy.** `AsciiEnum` is name →
  ASCII value and nothing else, and `set_ascii_enum` packs every member
  through `ascii_packed`, so it accepts only ASCII-width and coded
  datatypes. Most FIX code sets sit on `int`, `Boolean` or `String` fields
  and cannot use it at all, and none can carry a description or a pedigree.
  A field may carry both; neither derives from the other (so N4 is
  satisfied).
- **No English expansion.** yggfin expands `identifier` → `id` so
  `shortcodeid` reaches `"Short code identifier"`. *Rejected:* a guess about
  English, not about FIX.
- **Tier 3 is provisional.** If the parenthesized-abbreviation tier proves
  noisy over the generated dictionary, drop it. P4-R6's two cases decide.

### Tests

Fixture A - `SideCodeSet`: `Buy`=1, `Sell`=2 (FIX.2.7, EP254),
`Undisclosed`=7 (FIX.4.1), `CrossShort`=9 (FIX.4.2), `CrossShortExempt`=A
(FIX.4.3).

Fixture B - `CommTypeCodeSet` (tag 13, `char`): `PerUnit`=1, `Percent`=2,
`Absolute`=3, `PercentageWaivedCashDiscount`=4,
`PercentageWaivedEnhancedUnits`=5, `PointsPerBondOrContract`=6,
`BasisPoints`=7 (EP208), `AmountPerContract`=8.

1. Lookup by value, by name, by folded name; an alias pair sharing a value.
2. `4` → `4` (tier 1).
3. All four foldings of `PercentageWaivedCashDiscount` → `4`.
4. `PercentageWaivedEnhancedUnits` → `5`: a shared prefix does not collide.
5. `code_name("4")` → `PercentageWaivedCashDiscount`.
6. An unknown spelling falls through (P4-R7).
7. An ambiguous pair answers `None` (P4-R8).
8. Tier 3: `gtd` → `6`; `64` left alone; `ccp` left alone (P4-R6).
9. Version filter hides `CrossShort` at `4.1`; `BasisPoints` unresolvable
   before it existed; a deprecated code hidden at and after its version.
10. Canonical JSON round trip; a malformed document names its byte position;
    a reordered-key document refused (P4-R4).
11. Allocation case: `code()` on a 300-code set allocates nothing.

**Bench.** New group: `code()` against a `HashMap` baseline built from the
same document, so the scan is defended with a number.
**Docs.** `docs/fix.md`.

---

## Phase 5 — `FixFieldMut::merge_with`

**Goal.** One optimized, FIX-aware merge of two definitions of one field.

**Depends.** Phases 3 and 4 (it merges what they add).

**Files.** *Edit* `rust/src/fix/{field,registry}.rs`, `rust/src/fix/tests.rs`,
`rust/tests/allocations.rs`, `rust/benchmarks/fix/mutate.rs`, `docs/fix.md`.

**Never.** Leave the private `merge` at `registry.rs:866` in place (N3), or
add a priority or source field to any core type.

### Contract

```rust
impl FixFieldMut<'_> {
    /// Folds another definition of the same field into this one.
    pub fn merge_with(&mut self, other: &FixField<'_>) -> Result<()>;
}
```

### Why the current path is replaced

`registry.rs:866`'s `merge` builds a new `Metadata`, walks it into the field
with `set_metadata`, then reads back and rewrites `fix:tags` and
`fix:aliases` - three metadata rewrites and a `Vec<String>` of every key.
`ProtocolFieldMut::merge_with` (`field/protocol/mod.rs:466`) is worse: it
collects every held property name into an owned `String`, then scans
`O(n*m)`.

### Rules

- **P5-R1. Per-key rules, because "merge" alone decides nothing.**

  | key | rule |
  | --- | --- |
  | `fix:branch`, `fix:tag` | MUST agree; a disagreement is a typed refusal naming both. Identity is not merged. |
  | `fix:tags` | union, incoming first, order kept, deduplicated |
  | `fix:aliases` | union, ASCII-folded comparison, incoming first - then **rewritten from the merged lineage** so P3-R8b still holds |
  | `fix:description` | **never compared.** Incoming wins when it has one; stored is kept when it does not |
  | `fix:lineage` | merged by `since`: union, incoming wins an equal `since`, re-sorted oldest-first, re-validated against the merged name and datatype (P3-R8a) |
  | `fix:codes` | merged by wire value: incoming wins a shared value, stored keeps codes only it has, pedigree carried through, re-rendered canonically once |
  | any other `fix:` key | incoming wins; stored keeps what only it has |

- **P5-R2. Descriptions are never compared** because a description is the
  longest value a field carries and comparing two costs more than the write
  it would save.
- **P5-R3. One metadata write.** Build the merged map, write it once, never
  touch the field between reads. Three rewrites and their `invalidate_arrow`
  calls collapse into one.
- **P5-R4. No key-name allocation.** The `fix:` key set is a `const` list in
  `fix/field.rs`; walk it, never collect held names into `String`s.
- **P5-R5. Atomic.** A refusal leaves the field exactly as it was.
- **P5-R6. `FixRegistry::update` calls it,** and the private `merge` is
  deleted. No second merge path survives.

### Decided

- **Precedence is the caller's ordering, not a field on the merge.** Several
  sources describe one tag - FIX Latest, a QuickFIX dictionary, a vendor
  orchestration - and yggfin resolves it with a `priority` per source in
  `sources.json`. The generator merges lowest priority first, so the
  highest-priority source is the last `incoming` and wins by P5-R1. One
  concept, in the one place that knows about sources.

### Tests

1. Every row of P5-R1 as its own case.
2. Two fields with different long descriptions: the incoming's survives and
   nothing else moved.
3. A tag disagreement refused, both sides named.
4. A merge that leaves the field byte-identical when the incoming adds
   nothing.
5. Allocation case bounding a merge of two realistic fields.

**Bench.** The new merge against the deleted one's behaviour, over the
`config/fix` dictionary, in `rust/benchmarks/fix/mutate.rs`.

---

## Phase 6 — the source: FIX Latest into `config/fix`

**Goal.** Generate and commit the dictionary the other six phases describe.

**Depends.** Phases 1, 3 and 4.

**Files.** *Create* `scripts/build_fix_registry.py`, `rust/src/fix/header.rs`.
*Write* `config/fix/**`, `config/fix/sources.json`. *Edit*
`rust/src/fix/{mod,registry}.rs`, `rust/src/fix/tests.rs`, `docs/fix.md`.

**Never.** Add an HTTP client, or any dependency, to `rust/Cargo.toml` (N2).
The generator is a script; the crate only ever reads the committed output
through `FixRegistry::from_handle`.

### Sources

Browsable rendering, for checking work by eye:
<https://orchimate.org/fixtrading/fix-latest> - FIX Latest as of EP309,
Orchestra v1.0 - with `/fields`, `/codeSets`, `/datatypes`, `/messages`,
`/components`, `/groups`, `/revisions`; field pages at `/fields/<Name>`,
code sets at `/codeSets/<Name>CodeSet`. Its "Orchimate MCP" is useful for
interactive lookups. **HTML is never scraped.**

Machine-readable source of record, under
`https://raw.githubusercontent.com/FIXTradingCommunity/orchestrations/<commit>/`
(percent-encode the space):

| version | file |
| --- | --- |
| FIX Latest | `FIX Standard/OrchestraFIXLatest.xml` |
| FIX 4.4 | `FIX Standard/OrchestraFIX44.xml` |
| FIX 4.2 | `FIX Standard/OrchestraFIX42.xml` |

Versions Orchestra does not publish - 4.0, 4.1, 4.3, 5.0, 5.0SP1, 5.0SP2 -
come from the QuickFIX data dictionaries at
<https://github.com/quickfix/quickfix/tree/master/spec> (`FIX40.xml` …
`FIX50SP2.xml`), which carry names, types and enum values per version:
enough for lineage, and the only public per-version set that is.

Vendor and community orchestrations come from Orchestra Hub,
`https://orchestrahub.org/api/v3/repos/<owner>/<repo>/revisions/<id>/download`
- how yggfin loads its `fixtrading-udf` and `clear-street` namespaces.

### What the Orchestra XML gives

Verbatim from `repositorytypes.xsd`:

```xml
<xs:attributeGroup name="entityAttribGrp">
  <xs:attribute name="added"        type="fixr:Version_t"/>
  <xs:attribute name="addedEP"      type="fixr:EP_t"/>
  <xs:attribute name="updated"      type="fixr:Version_t"/>
  <xs:attribute name="updatedEP"    type="fixr:EP_t"/>
  <xs:attribute name="deprecated"   type="fixr:Version_t"/>
  <xs:attribute name="deprecatedEP" type="fixr:EP_t"/>
  <xs:attribute name="replaced"     type="fixr:Version_t"/>
  <xs:attribute name="replacedEP"   type="fixr:EP_t"/>
</xs:attributeGroup>

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

`fixr:codeSet` carries `type` plus id, name and pedigree from the entity
base; documentation hangs off `fixr:annotation`/`fixr:documentation` on
fields, code sets and individual codes. `added="FIX.2.7"` and
`updatedEP="254"` are exactly P1's `Version` and P4's `ep`.
`replaced`/`replacedEP` is the rename axis, but FIX Latest records only
current names - the historical spelling lives in the per-version files,
which is why the script reads all of them.

### Rules

- **P6-R1. Every source URL is pinned to a commit, never `master`.** yggfin
  does exactly this - FIX Latest at
  `.../orchestrations/099914dd0edd49a699326f0441776d6e21cfaf93/…` and
  QuickFIX at `.../quickfix/3536699e830e65f875df4a50b647a6d3bad3b884/…` -
  because a dictionary regenerated from `master` is not reproducible and its
  diff is unreviewable.
- **P6-R2. `--source`** takes a local directory or a URL base so a run is
  reproducible offline; the default is the pinned set.
- **P6-R3. Output is exactly the shard layout `from_handle` reads:**
  `config/fix/{primitive,nested}/<branch>/<tag/100>.json`, each a JSON array
  of core field documents ordered by canonical identifier, byte-identical to
  what `write_into` would produce.
- **P6-R4. Provenance in `config/fix/sources.json`,** one record per source:
  `source_id`, `format` (`orchestra` | `quickfix`), pinned `url`, `sha256`
  of the bytes fetched, `sha256` of the definitions produced, `branch`,
  `version` label, `priority`, and **`license_url`**. The licence field is
  not bookkeeping: this commits a derived copy of FIX Trading Community and
  QuickFIX material into the repository and the attribution must travel with
  it. The two checksums give CI a drift test whose second half needs no
  network.
- **P6-R5. The provenance file is safe beside the trees.** `from_handle`
  descends only `primitive/` and `nested/` (`store.rs:193`), so a leaf
  beside them is never listed. It MUST NOT be named `records`, the
  retired-layout tripwire at `store.rs:181`.
- **P6-R6. Pull one vendor dictionary into a non-standard `FixBranch` in the
  same run,** so P2's digest table and P7's branch inference have real data
  under them rather than a fixture.
- **P6-R7. Datatypes resolve through `DataType::LOGICAL_NAMES`,** already
  the FIX Latest datatype table (`docs/datatype.md`): `Qty` is
  `decimal64(18,8)`, `SeqNum` is `int64`, no second mapping. A FIX datatype
  the table does not hold is a hard failure of the script, never a silent
  `utf8`.
- **P6-R8. The generator narrows nothing the standard did not.**
  `CheckSum(10)` is three digits with leading zeros and stays `String`;
  `BodyLength(9)` is `Length`, so a malformed one nulls a field rather than
  costing a batch (P7-R24); `MonthYear` stays `ascii(8)` because `202608` is
  a month and `202608w2` a week, neither a point in time; `SecureData(91)`,
  `XmlData(213)` and `Signature(89)` are `binary` though the wire carries
  them as text. Each is a case in yggfin's `test_fields.py` because each was
  got wrong once.
- **P6-R9. Repeating groups become the nested tree as they do today:** a
  List of a non-null `item` Struct whose `fix:tag` is the counter's.
- **P6-R10. Priority ordering.** Merge sources lowest priority first so the
  highest-priority source wins by P5-R1.

### The standard header and trailer

- **P6-R11. They are order, not nesting.** `FixMsg` lays a message out flat
  (P7), so both components are carried as generated `const` tag lists in
  `rust/src/fix/header.rs`, read through
  `FixRegistry::standard_header_tags()` and `standard_trailer_tags()`:

  ```rust
  pub const STANDARD_HEADER_TAGS:  &[i32] = &[8, 9, 35, 1128, /* … */ 369];
  pub const STANDARD_TRAILER_TAGS: &[i32] = &[93, 89, 10];
  ```

- **P6-R12. They are not registry entries.** A component has no tag and
  `insert` admits only a field carrying one (L1); a synthetic tag would put
  a fiction in the identity space. The names `StandardHeader` and
  `StandardTrailer` and their ids are dropped, and the module docs name that
  loss. Every field *in* them is an ordinary registry entry by its own tag.
- **P6-R13. The constants are the union across every scraped version,** in
  canonical order, with `defined_at` deciding what a given version may
  carry. FIX Latest alone is not enough: yggfin's `versions.json` records
  `SecureDataLen(90)` and `SecureData(91)` in the 4.0–4.4 headers, which FIX
  Latest no longer lists, and the trailer is `CheckSum` alone in Latest but
  `SignatureLength(93)`, `Signature(89)`, `CheckSum(10)` in 4.2 and 4.4.
- **P6-R14. Requiredness rides the lineage, not a parallel table.** yggfin
  stores a `required` flag per session field per version; here presence is
  already `nullable` on the field a version resolves to, so the flag belongs
  in the P3 lineage entry.

FIX Latest's `StandardHeader`, in order: 8 BeginString, 9 BodyLength,
35 MsgType, 1128 ApplVerID, 1156 ApplExtID, 1129 CstmApplVerID,
49 SenderCompID, 56 TargetCompID, 115 OnBehalfOfCompID, 128 DeliverToCompID,
34 MsgSeqNum, 50 SenderSubID, 142 SenderLocationID, 57 TargetSubID,
143 TargetLocationID, 116 OnBehalfOfSubID, 144 OnBehalfOfLocationID,
129 DeliverToSubID, 145 DeliverToLocationID, 43 PossDupFlag, 97 PossResend,
52 SendingTime, 122 OrigSendingTime, 212 XmlDataLen, 213 XmlData,
347 MessageEncoding, 369 LastMsgSeqNumProcessed — plus the `HopGrp` group.
`StandardTrailer` is component id 1025.

### The worked case

Tag 32, end to end:

- FIX Latest: id 32, name `LastQty`, type `Qty`, added `FIX.2.7`.
- `OrchestraFIX42.xml` and `FIX42.xml`: tag 32 is `LastShares`, type `Qty`
  in 4.2 and `int` in 4.0/4.1.
- Generated field: name `LastQty`, datatype `decimal64(18,8)`, `fix:tag` 32,
  `fix:aliases` containing `LastShares`, and a `fix:lineage` of
  `{"since":"2.7","name":"LastShares","type":"int"}`,
  `{"since":"4.2","name":"LastShares","type":"Qty"}`,
  `{"since":"4.3","name":"LastQty","type":"Qty"}`.

### Tests

1. Load `config/fix` and assert every line of the worked case.
2. Load the generated tree and write it back: byte-identical (P6-R3).
3. Every tag in both header constants resolves in the generated dictionary.
4. The generated header matches yggfin's `versions.json` ordering and
   required flags for 4.2 and 4.4 (P6-R13, P6-R14).
5. The two checksums in `sources.json` match a regeneration from the same
   pinned inputs.

**Docs.** Provenance, version coverage, the FIXT omission (P3-R2), the
dropped component names (P6-R12) and the regeneration command, on
`docs/fix.md`.

---

## Phase 7 — explicit halves, `FixEntry`, `from_pairs`, and the text readers

**Goal.** Build a typed, lossless `FixMsg` from key/value pairs, from FIX
text, or from a ULBridge body.

**Depends.** Phase 2 (identifier), Phase 3 (version), Phase 6 (dictionary).

**Files.** *Create* `rust/src/fix/entry.rs`. *Edit*
`rust/src/fix/{mod,registry,msg}.rs`, `rust/src/fix/tests.rs`,
`rust/tests/allocations.rs`, `rust/benchmarks/fix/` (new group),
`docs/fix.md`.

**Never.** Write a second parser for anything. The readers split and
delegate; `from_pairs` is the only builder; `Field::scalar` is the only
value contract; the fold is the crate's one fold (P4-R5.2).

### Contract

```rust
impl FixRegistry {
    pub fn get_primitive_field<'k>(&self, key: impl Into<FixKey<'k>>) -> Option<&Field>;
    pub fn primitive_field<'k>(&self, key: impl Into<FixKey<'k>>) -> Result<&Field>;
    pub fn get_nested_field<'k>(&self, key: impl Into<FixKey<'k>>) -> Option<&Field>;
    pub fn nested_field<'k>(&self, key: impl Into<FixKey<'k>>) -> Result<&Field>;
}

pub struct FixEntry {
    pub tag: Option<i32>,      // None when the key named no field
    pub branch: Option<i32>,   // xxh32 of the branch; None is standard/unresolved
    pub key: Option<SmolStr>,  // the arriving key, kept only when it is not the tag
    pub value: SmolStr,        // exactly as it arrived; never absent
}
impl FixEntry { pub fn id(&self) -> Option<FixId>; }

impl FixMsg {
    pub fn entries(&self) -> &[FixEntry];
    pub fn anomalies(&self) -> impl Iterator<Item = FixAnomaly<'_>>;
    pub fn into_text(&self, sep: char) -> String;

    pub fn from_pairs<'a, I>(
        registry: Arc<FixRegistry>, entries: I,
        branch: Option<&FixBranch>, version: Option<Version>,
    ) -> Result<Self> where I: IntoIterator<Item = (&'a str, &'a str)>;

    pub fn from_text(text: &str) -> Result<Self>;
    pub fn from_fixtext(
        registry: Arc<FixRegistry>, text: &str, sep: char,
        branch: Option<&FixBranch>, version: Option<Version>,
    ) -> Result<Self>;
    pub fn from_ultext(
        registry: Arc<FixRegistry>, body: &[u8],
        branch: Option<&FixBranch>, version: Option<Version>,
    ) -> Result<Self>;
}
```

`FixMsg` gains one field: `entries: Vec<FixEntry>`.

### The halves

- **P7-R1.** `position_by_id` (`registry.rs:703`) and `position_by_name`
  (`:715`) each hard-code the same four-way probe - primitive canonical,
  nested canonical, primitive alternate, nested alternate. Expose the halves
  and compose: `get_field` becomes
  `get_primitive_field(key).or_else(|| get_nested_field(key))`. Each half
  accessor probes both tiers, canonical first. `get_field_by_tag`,
  `get_field_by_id`, `get_field_by_name` and `get_field_by_path` redirect
  the same way; none keeps a probe chain of its own.
- **P7-R2. This is not tidying.** A transcriber resolving a wire tag wants a
  scalar; today an unknown tag pays all four probes - 32.3 ns for a
  primitive hit against 72.2 ns for a miss. `from_pairs` asks only
  `get_primitive_field`, so an unknown tag costs one probe.

### `FixEntry`

- **P7-R3.** `id()` folds tag and branch into a `FixId` with one shift-or
  (P2-R1), so a resolved entry addresses the registry without hashing.
- **P7-R4.** `branch: None` means the standard branch *and* "not resolved
  yet" - which is why this is not simply a `FixId`. `Option<i32>` costs 8
  bytes, not 4: `0` is a legal digest, so there is no niche.
- **P7-R5. The value owns.** A `FixMsg` holds its entries and outlives the
  text it was read from, so a borrowed value would force `FixMsg<'a>` on
  every caller and on both bindings, which hold one across an FFI boundary.
  `SmolStr`'s 23 inline bytes cover a side, a price, a symbol and a 21-byte
  `UTCTimestamp`, so the common entry allocates nothing. Readers still split
  into borrowed `(&str, &str)`; the single materialization is in
  `from_pairs`.
- **P7-R6. `tag` is optional and `key` exists** because an unresolved key
  has no tag. `VenueOwnThing=x` survives in the tree (P7-R12) and `entries`
  is the wire record, so it must hold it too. `key` is `None` for the
  common resolved pair, so nothing is stored twice.
- **P7-R7. The value is never typed inside the entry.** Typing happens once,
  in `from_pairs`, through `Field::scalar`.

### `FixMsg` carries its entries

- **P7-R8. `entries` is not the row restated,** and this is the one place
  the brief admits two facts about one thing (N4). The row is the
  *interpretation*: values typed, codes translated, names canonical, groups
  nested, header ordered. `entries` is what *arrived*: raw text, arrival
  order, untranslated, including pairs no dictionary explained. Neither
  derives from the other - a translated `4` cannot say whether the wire
  carried `4` or `PercentageWaivedCashDiscount` - so lossless re-emission is
  impossible from the row alone. That is what makes `into_text` and the
  round trip (P7-R28) work. **Say exactly this in the doc comment**, or a
  reader will assume one of the two is redundant.
- **P7-R9. Populated by `from_pairs`, and so by all three readers.** Empty
  for a message built through `new` or `with_registry`; with no entries,
  `into_text` and `anomalies` fall back to the row and say so.

### What a key may be

| key | means |
| --- | --- |
| `54`, `"54"` | a tag, through the strict `parse_tag` (`fix/field.rs:44`), which refuses `+35` and `3x` |
| `Side`, `side`, `SIDE`, `" Side "` | a name, trimmed and folded |
| `msg_type`, `msg-type`, `Msg Type` | the same name: separators fold away too |
| `Instrument.Symbol` | a path, through the existing `get_field_by_path` |
| `PartyID[0]`, `PartyID[1]` | one field, two occurrences, in order |
| `NoPartyIDs[0].PartyID` | a group entry: which group, which occurrence, which member |
| `VenueOwnThing` | an unknown name, **kept** |
| `""`, `"   "` | dropped |

- **P7-R10. Separator folding.** The registry folds ASCII case only today
  (`fix/mod.rs`), so a renderer emitting `msg_type` or `Msg Type` misses a
  field that exists. Extend the FIX name fold to drop `_`, `-` and space -
  the fold `LOGICAL_NAMES` already uses (`datatype/parser.rs:1656`), so one
  rule serves both. No two FIX fields differ only by a separator; assert
  that over the generated `config/fix` as the test that lets the change in.
- **P7-R11. An unknown *tag* is kept** as a nullable `utf8` field under its
  decimal spelling - `FixMsg`'s existing rule.
- **P7-R12. An unknown *name* is kept too,** as a nullable `utf8` field
  under its own spelling. Every venue sends fields no dictionary has, and
  dropping them loses data. Resolved fields come first in the root, unknown
  ones after, so the schema stays stable when a dictionary later learns the
  name.
- **P7-R13. An empty value drops its pair.** `54=` is a malformed message,
  not an absent side.
- **P7-R14. Order and repetition are the message.** A tag appearing twice
  stays two entries in input order; a map keyed by tag would lose a
  repeating group. P3's duplicate-name suffix rule names the second and
  later children.

### Inferring version and branch

- **P7-R15. Version, when the caller names none.** Each step is a FIX rule,
  not a heuristic.
  1. **Tag 1128 `ApplVerID`** - the application version. Code set:
     `0`=FIX27, `1`=FIX30, `2`=FIX40, `3`=FIX41, `4`=FIX42, `5`=FIX43,
     `6`=FIX44, `7`=FIX50, `8`=FIX50SP1, `9`=FIX50SP2, `10`=FIXLatest. It
     wins because under FIXT.1.1 the session version says nothing about the
     application version. The symbolic spelling (`FIX44`) is accepted
     through P4's `code_by_name`.
  2. **Tag 8 `BeginString`** - `FIX.4.0` … `FIX.4.4` give `4.0` … `4.4`.
     `FIXT.1.1` is a session version and names no application version, so it
     **falls through** rather than being taken literally.
  3. Otherwise `Version::MAX` - FIX Latest, which is what no version marker
     means.
- **P7-R16. Branch, when the caller names none.** Resolve every entry in
  `FixBranch::STANDARD`; nothing missed means standard and there is no
  second pass - the common case costs one probe per entry. Otherwise retry
  only the *missed* tags against each branch the registry holds
  (`branches()`, free from P2-R9) and take the branch resolving the most; a
  tie goes to the lowest branch name, so the answer is deterministic; a
  branch resolving none is never chosen and its tags stay unknown. A caller
  who passes a branch gets it, with no guessing at all.

### Building the message

One pass over the resolved entries:

- **P7-R17.** The field is the registry's, cloned, with `name_at(version)`
  and `dtype_at(version)` where a lineage exists, and the field's own name
  and datatype where it does not.
- **P7-R18.** It is non-null in this message's schema, because the value is
  present.
- **P7-R19.** The value passes through the code set first:
  `code_value_at(&version, entry.value).unwrap_or(entry.value)`, so
  `CommType=PercentageWaivedCashDiscount` stores `4` and
  `MsgType=NewOrderSingle` stores `D`, while an unexplained spelling is
  carried through untouched (P4-R7).
- **P7-R20.** Then `field.scalar(Scalar::from(translated))`, with P7-R24 on
  refusal. Nothing re-checks what `scalar` answered.
- **P7-R21.** Order is `STANDARD_HEADER_TAGS`, then the body in entry order,
  then `STANDARD_TRAILER_TAGS` - flat, no `StandardHeader` Struct (P6-R11).
- **P7-R22.** `FixMsg::with_registry` finishes it, so existing validation
  and canonicalization are not bypassed.
- **P7-R23. An empty dictionary is a supported input, not an error.** With
  nothing resolvable, every name stays a name, every tag stays its decimal
  spelling, and `by_name` still finds what was put in - which is what makes
  this usable on a venue whose dictionary is not loaded yet.
- **P7-R24. A value that will not type is null, not a failure.**
  `field.scalar` refuses a value the datatype cannot hold - a `BodyLength`
  that is not digits, a mangled timestamp. That must not cost the message:
  (a) the row's field is **null**; (b) the raw text stays in `entries`
  exactly as it arrived; (c) the refusal is reported through `anomalies()`;
  (d) `from_pairs` still answers `Ok`. A parse error is raised only for
  input that is not a message at all. A null nobody can explain is worse
  than the value that actually arrived.

### Groups

- **P7-R25. Repeating groups are in scope, because the key carries the
  location.** A key spelled `NoPartyIDs[0].PartyID` states group, occurrence
  and member, so no grammar is needed - and yggdryl already has the pieces:
  a group is a List of a non-null `item` Struct, and
  `Field::set_field_by_path` writes into one. `from_pairs` builds **real
  nesting** from indexed keys, where yggfin keeps a flat `comp` string
  because its field model has no list of structs to put it in.
- **P7-R26. Out of scope: inferring a group from repetition alone.** Bare
  `448=A`, `448=B` with no index and no group key produces two sibling
  occurrences of `PartyID`, not a reconstructed `NoPartyIDs`. Reassembling
  that needs the message grammar, which is the `.cfb` phase's. Say so in the
  module docs.

### Encode direction

- **P7-R27. Wire spellings belong to the FIX layer, never to
  `DataType::scalar`.** yggfin pins them: a float is never exponent notation
  (`1e-7` writes `0.0000001`), a `UTCTimestamp` is
  `20260821-10:30:00.123456`, a date is `20260821`, a time is
  `10:30:00.000000`, a boolean is `Y` or `N`.
  **Verify first** whether `DataType::scalar` accepts a `Scalar::String` for
  `Boolean` and the temporals at all (`field/value.rs:112`). Where it does
  not, the FIX layer parses the wire spelling into the right `Scalar` before
  calling `scalar`; where it does, check the spelling it accepts is FIX's.
  Either way the generic value contract learns no FIX spelling -
  `LOGICAL_NAMES` is deliberately a *type* table, not a *value* one.
- **P7-R28. The two ways in must agree.**
  `from_text(built.into_text('|')) == built` is a test in this phase.

### The three readers, one builder

`from_pairs` borrows both halves of a pair, so a splitting iterator feeds it
with no copy. Each reader splits, rewrites its dialect into the key forms
`from_pairs` already understands, and hands the iterator over. One nesting
builder, one fold, one code translation under all three.

- **P7-R29. `from_text` picks the dialect by one token, never by sniffing.**
  Take the bytes before the first `=`: all ASCII digits means
  `from_fixtext`, and the separator is SOH when the text holds one and `|`
  otherwise; anything else means `from_ultext`. `from_text` is the
  convenience over `FixRegistry::global()` with everything inferred. Empty
  text is a typed error, not an empty message.
- **P7-R30. `from_fixtext`.** Split on `sep` with `memchr`, then each
  segment at its first `=`. A trailing empty segment is tolerated - a wire
  message ends with the separator. A segment with no `=` is dropped, as an
  empty key is. Duplicate tags stay in arrival order. Every key and value is
  a slice of the input.

#### `from_ultext`

ULBridge writes names, not tags, and packs a repeating group into one pair
(yggfin, `docs/fix/repeating-groups.md`):

```text
#NOPARTYIDS=1|#NOPARTYIDS[0]=PARTYID=SYNTH-01<sub>PARTYIDSOURCE=shortcodeid<sub>PARTYROLE=executingsystem|
```

where `<sub>` is `\x04\x03`, EOT then ETX.

- **P7-R31.** Pairs split on `|`, then at the first `=`. Keys are names in
  any case and reach their field through the P7-R10 fold.
- **P7-R32.** A key opening with `#` names a group: `#NOPARTYIDS=1` is the
  counter; `#NOPARTYIDS[0]=…` is entry 0, and its *value* is a run of member
  pairs.
- **P7-R33.** Members inside an entry split on `\x04\x03`.
- **P7-R34. And sometimes on nothing at all.** ULBridge may omit the
  separator after the first member while keeping the index:
  `#NoPartyIDs[0]=PartyID=P-1PartyIDSource=DPartyRole=3`. Split by scanning
  for the next member name the group's own field declares, taking the
  **longest declared match** so `PartyIDSource` beats `PartyID`. Only that
  group's declared members are candidates, which keeps the scan bounded and
  the result explainable.
- **P7-R35.** Residue that will not split stays as one unknown key,
  verbatim. Never dropped, never fatal.
- **P7-R36.** Indices may be partial or out of order - `[2]` before `[0]`,
  with gaps. Occurrences are built by index, not arrival; a gap is a null
  occurrence.
- **P7-R37.** It then rewrites into the key forms `from_pairs` takes -
  `#NOPARTYIDS[0]=PARTYID=…` becomes `("NoPartyIDs[0].PartyID", "…")` - and
  builds no tree of its own. Values translate through the code set like any
  other, so `PARTYIDSOURCE=shortcodeid` stores `P` and
  `PARTYROLE=executingsystem` stores `16`, while
  `PARTYROLE=orderoriginatorsystem` is stored verbatim under tag 452.

#### Token rules both readers obey

Each row is a case in yggfin's `test_message.py` or `test_transcribe.py` -
a line some venue really sent.

| # | rule | why |
| --- | --- | --- |
| P7-R38 | A token splits at its **first** `=` only. | `Text=a;b` is one value with a semicolon, not two fields. |
| P7-R39 | `G[0]=M=v` and `G[0].M=v` are one field, two prints. | A group has one shape; two spellings must not make two. |
| P7-R40 | `#` marks where a key **starts**, not which field it is. | `#54=x` is a rendered key spelled with digits, **not** tag 54. |
| P7-R41 | `#A=1#B=2` has no separator: the next `#` ends the previous value. | ULBridge omits separators; the marker is the boundary. |
| P7-R42 | Tag mode is ASCII digits only. | A bracket, dot or `#` means a rendered key, so `453[0]` is never tag 453. |
| P7-R43 | A digit key overflowing `i32` is not a tag. | An epoch-millis key looks like digits; `parse_tag` already drops it. |
| P7-R44 | Trim ASCII whitespace only. | A non-breaking space is part of the value; trimming Unicode returns a tag never sent. |
| P7-R45 | Nothing after `10=<checksum>` is part of the message. | Log lines carry pair-shaped noise after the trailer. |
| P7-R46 | One `a=b` alone is a sentence, not a message. | Require two tokens, or an `8=`/`35=` lead, so prose does not parse. |
| P7-R47 | Two values under one key stay two. | It is a group or a rewrite; collapsing picks one, and picking is a guess. |
| P7-R48 | "Not a message" and "a message that said nothing" are different answers. | The empty message is `Ok` with no entries; unparseable input is an error. |

#### `data` fields are read by length, not by separator

- **P7-R49.** FIX types a field `data` **because its value may contain the
  separator**. `RawData(96)`, `XmlData(213)`, `SecureData(91)` and
  `Signature(89)` each follow a length field - `RawDataLength(95)`,
  `XmlDataLen(212)`, `SecureDataLen(90)`, `SignatureLength(93)` - and that
  length, not the next SOH, says where the value ends. A reader that
  tokenizes first loses the message.
- **P7-R50.** The registry says which tags are `data` (`DataType::Binary`
  after P6-R8), so nothing hard-codes the four pairs.
- **P7-R51.** When the stated length and the next separator disagree, **take
  the separator**: a writer that miscounted has stated two things and the
  delimiter is the safer. Record it through `anomalies()`.
- **P7-R52.** Venues put `NAME=VALUE` pairs inside `XmlData(213)` though the
  standard calls it an XML stream. Not this phase's job - the value is kept
  whole - but a nested pair addressed `XmlData.ClOrdID` must later resolve
  the way `NoPartyIDs.PartyID` does.

#### Anomalies are derived, never a second state

- **P7-R53.** A counter disagreeing with the entries it introduces, a group
  that would not split cleanly, a value that would not type - all real, none
  fatal. `anomalies()` derives them on demand by comparing the counter value
  (an ordinary value at its own tag) with the List's length, the way `FixId`
  is derived rather than stored. No error channel on `FixMsg`, nothing to
  keep in step, and a caller who never asks pays nothing.

### Optimization the phase is judged on

- **P7-R54.** One `Vec<FixEntry>`, one `Vec<Field>`, one `Vec<Scalar>`, each
  reserved from the iterator's `size_hint` before the walk. No per-entry
  `String`, no per-entry map. The `Vec<FixEntry>` is the one the message
  keeps: built once, moved in, never cloned.
- **P7-R55.** A resolved entry allocates nothing - integers for `tag` and
  `branch`, `None` for `key`, and a value inside `SmolStr`'s inline buffer.
- **P7-R56.** Only `get_primitive_field` is probed for scalars; the nested
  half is reached only for a counter tag.
- **P7-R57.** Header ordering reads a precomputed tag-to-position table, not
  a scan of `STANDARD_HEADER_TAGS` per entry.
- **P7-R58.** The readers copy nothing: every key and value is a slice of
  the input and splitting uses `memchr`.

### Tests

**Keys and values.**
1. Tag-keyed and name-keyed pairs producing the identical message.
2. `" Side "`, `msg_type`, `msg-type`, `MSG_TYPE`, `Msg Type` all reaching
   their field (P7-R10).
3. `Instrument.Symbol` resolving through the path.
4. `PartyID[0]` and `PartyID[1]` staying two ordered occurrences.
5. `NoPartyIDs[0].PartyID` building a List of one `item` Struct, not a flat
   name (P7-R25).
6. An unknown name surviving beside a known one, known first (P7-R12).
7. An unknown tag kept as `utf8` under its decimal name (P7-R11).
8. Empty and blank keys dropped; an empty value dropped (P7-R13).
9. The same pairs built against an empty registry (P7-R23).

**Version and branch.**
10. `ApplVerID` beating `BeginString`; `BeginString="FIXT.1.1"` falling
    through to Latest; an explicit `version` overriding both (P7-R15).
11. Branch inference picking the vendor dictionary that resolves the misses,
    the tie rule, and an explicit branch suppressing inference (P7-R16).
12. Tag 32 keyed `LastShares` in a `4.2` message and `LastQty` in a Latest
    one, both answering the same value.
13. Header and trailer ordering with a body field interleaved in the input.

**Token rules.**
14. Every row of P7-R38…R48, one case each.
15. `#54=x` reaching the field whose *rendered key* is `54`, never tag 54.
16. `G[0]=M=v` and `G[0].M=v` answering equal messages.
17. A lone `a=b` refused as not-a-message, while an empty-but-valid message
    answers `Ok` with no entries.
18. A `data` field whose value contains the separator, read by its length
    field; a miscounted length taking the separator and appearing in
    `anomalies()` (P7-R49, P7-R51).
19. A `BodyLength` of `abc` nulling that field while the raw text stays in
    `entries` (P7-R24).
20. Tag 555 at two nesting levels in one TradeCaptureReport, neither
    guessed.

**Codes.**
21. `("CommType", "PercentageWaivedCashDiscount")` and
    `("13", "percentage_waived_cash_discount")` both storing `4`.
22. `("MsgType", "NewOrderSingle")` storing `D`; `("CommType", "4")`
    unchanged; an unexplained spelling stored verbatim.
23. A name added after the message's inferred version refusing to translate.

**Readers and entries.**
24. The ULBridge payload verbatim, with `\x04\x03` and with the separator
    omitted, both producing one `NoPartyIDs` occurrence of four members.
25. `PARTYIDSOURCE` translating while `PARTYROLE=orderoriginatorsystem`
    survives untranslated.
26. Out-of-order and gapped indices (P7-R36).
27. A counter of `2` against one entry appearing in `anomalies()` while the
    message still reads.
28. `entries()` holding every pair in arrival order with the untranslated
    spelling, beside a row holding the translated code (P7-R8).
29. An unresolved key in `entries` with `tag` `None` and `key` set (P7-R6).
30. `from_fixtext` over SOH-separated and `|`-separated captures of one
    message answering equal messages.
31. `from_text` picking the dialect from `35=D|…` against `MSGTYPE=D|…`.
32. `from_text(built.into_text('|')) == built` (P7-R28).

**Halves.**
33. Each half accessor answering only from its half, over a registry holding
    a scalar and a group that would both match a key.
34. `get_field` answering exactly what it answered before, over every
    existing case.

**Bench.** A NewOrderSingle of ~15 pairs, an ExecutionReport of ~30, and a
300-pair message; tag-keyed against name-keyed; branch and version given
against inferred; the readers benched beside `from_pairs` so the split cost
is visible separately from the build. Report per-message and per-pair cost;
table on `docs/fix.md`, which also gains the two half-probe rows.

**Allocations.** A 30-pair tag-keyed build of short values allocates the
three reserved vectors and nothing per entry (P7-R54, P7-R55).
