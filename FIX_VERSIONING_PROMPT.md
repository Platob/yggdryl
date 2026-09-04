# FIX versioning, code sets, and the registry that carries them

Six phases in dependency order. Each is complete work on its own and each
states its own contract, files, tests, benchmark and docs. Follow
`AGENTS.md`: Rust core first, no bindings in this brief, no backward
compatibility, one fact in one place.

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
| `xxh3_64`, `xxh3_64_with_seed`, streaming state | `rust/src/xxhash/mod.rs:134`, `rust/src/xxhash/state.rs` |
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
- **Grammar.** `major(.part)*` then an optional qualifier: appended directly
  (`5.0SP2`) it is a *post*-release; introduced by `-` (`1.0.0-rc1`) it is a
  *pre*-release. A component is decimal, at most `u16::MAX`, at most
  `MAX_PARTS` of them; over-long input, a non-decimal component and an empty
  qualifier are `Error::Parse` naming the byte position, the way every other
  parser in the repo reports.
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
`Display`/`FromStr` round trip, serde round trip, the Arrow round trip
through `Field`, and `DataType::scalar` rewriting a string. Add the
allocation case. Bench parse and compare in `rust/benchmarks/datatype.rs`.
Document the value on `docs/generic.md` and the datatype row on
`docs/datatype.md`; both pages' examples run under
`scripts/check_docs_examples.py`.

---

## Phase 2 - `FixId` resolution on xxh3

Independent of every other phase; do it first if two people are working.

### What is there now

`Half` (`registry.rs:225`) holds four indexes per nestedness:
`ids: BTreeMap<FixId, usize>`, `alternate_ids: BTreeMap<FixId, usize>`,
`names: HashMap<BranchedKey, usize>`, `aliases: HashMap<BranchedKey, usize>`.
An identifier probe is `O(log n)` comparisons, each comparing a branch
`SmolStr` before the tag. A name probe is SipHash over branch plus folded
name. Lookups already allocate nothing
(`rust/tests/allocations.rs::a_fix_registry_lookup_allocates_nothing`), so
this phase buys compare and hash cost, not allocations. The numbers to beat
are published in `docs/fix.md`: 32.3 ns primitive tag hit, 93.1 ns nested tag
hit, 65.8 ns alternate tag hit, 72.2 ns miss, 128.1 ns vendor identifier hit
over 1034 fields, 81.8 ns name hit.

### The change

Key every index by a 64-bit digest and store it in a `HashMap` whose hasher
passes the digest through unchanged, because xxh3 already avalanches and
re-hashing an avalanched key with SipHash is pure waste:

```rust
ids:            HashMap<u64, usize, BuildHasherDefault<PassThrough>>
alternate_ids:  HashMap<u64, usize, BuildHasherDefault<PassThrough>>
names:          HashMap<u64, usize, BuildHasherDefault<PassThrough>>
aliases:        HashMap<u64, usize, BuildHasherDefault<PassThrough>>
positions_by_id: Vec<usize>   // ordered by canonical FixId, iteration only
```

- **The identifier digest is one call, no rendering and no buffer:**
  `xxh3_64_with_seed(branch.as_bytes(), tag as u64)`. The tag rides in the
  seed, the branch is already a contiguous `&[u8]` inside `SmolStr`'s inline
  buffer, and nothing is formatted. One code path for the standard branch
  and a vendor one; no special case, no `branch:tag` text built on the hot
  path. (`FixId`'s rendered form stays `branch:tag` - that is the display
  contract and it is untouched.)
- **The name digest folds while it feeds.** ASCII-fold the query into the
  streaming xxh3 state from `rust/src/xxhash/state.rs` in stack-sized chunks
  so no length allocates, seeded with the branch's own digest so a name can
  never be found under another branch. A distinct constant seed per index
  keeps one crafted key from landing in all four.
- **A digest collision is a loud refusal, never a wrong answer.** Two
  distinct keys mapping to one `u64` would silently overwrite in a
  `HashMap<u64, _>`, so `insert` verifies: on an occupied digest whose
  stored field does not hold that key, return a typed conflict naming both
  fields and both keys. Reads verify too - a hit re-checks the field at that
  position actually holds the key - so a collision degrades to a miss on the
  read path and to a refusal on the write path. Over ten thousand fields the
  probability is about 2.7e-15; the point is that the failure mode is
  stated, not that it happens.
- **Ordered iteration keeps its own structure.** `next_field_after`,
  `FixFieldIter`, `Debug`, `PartialEq` and `store::write_into` all depend on
  ascending canonical-identifier order, which a digest map cannot give.
  `positions_by_id` is a `Vec<usize>` kept sorted by binary-search insert:
  `O(n)` per insert on a dictionary built once and read forever, against
  `O(log n)` node chasing on every read. It exists for iteration only and
  the doc comment says so.

### Tests, benchmark, docs

Every existing registry test must pass unchanged - the observable contract
does not move. Add: a digest-collision refusal (construct it by stubbing the
digest function in a test-only path, or by asserting the verification branch
directly); iteration order identical to the previous implementation over a
shuffled insert order; `write_into` byte-identical to a shard written before
the change. Re-run `cargo bench -p yggdryl --bench fix` and **replace** the
table in `docs/fix.md`; a phase that does not beat those numbers is reported
with the measurement, not merged silently. Keep the allocation case green.

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

- `--source` takes a local directory or a URL base so a run is reproducible
  offline; the default is the raw GitHub base above.
- Output is exactly the shard layout `from_handle` reads:
  `config/fix/{primitive,nested}/<branch>/<tag/100>.json`, each a JSON array
  of core field documents ordered by canonical identifier - byte-identical
  to what `write_into` would produce, asserted by a test that loads the
  generated tree and writes it back.
- Provenance goes in `config/fix/source.json` - source URLs, the EP number,
  the retrieval date, and the per-version file list. `from_handle` descends
  only `primitive/` and `nested/` (`store.rs:193`), so a leaf beside them is
  never listed and never refused; it must not be named `records`, which is
  the retired-layout tripwire at `store.rs:181`.
- Datatypes resolve through `DataType::LOGICAL_NAMES`, which is already the
  FIX Latest datatype table (`docs/datatype.md`), so `Qty` is
  `decimal64(18,8)` and `SeqNum` is `int64` with no second mapping. A FIX
  datatype the table does not hold is a hard failure of the script, not a
  silent `utf8`.
- Repeating groups become the nested tree exactly as they do today: a List
  of a non-null `item` Struct whose `fix:tag` is the counter's.

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

## Verification

Per `AGENTS.md`, before handoff on any phase: `cargo fmt`, warning-free
Clippy, workspace tests with default features and `parquet iceberg`, Rust
1.85 default and `--no-default-features --lib`, rustdoc with warnings
denied, the `fix` and `datatype` benches, `scripts/check_docs_examples.py`,
and `python -m mkdocs build --strict`. Report exact results and exact
skipped checks. Rust-only: no Python or Node binding work in this brief.
