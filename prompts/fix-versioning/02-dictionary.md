# The dictionary — lineage, code sets, merge, and the generator

**Goal.** Everything that fills a FIX dictionary and keeps it coherent:
per-version lineage on a field, its code set, the one merge that folds two
definitions together, the generator that writes the committed dictionary from
the published sources, and the two coded datatypes that vocabulary earns.

**Depends.** Phase 1. Then 3 → 4 → 5 → 6 → 8 in order.

> Read `00-contract.md` first: `N1`–`N7`, `L1`–`L2`, precedence, done-when.
> Each `## Phase` is one PR. Rule ids are cited from the other files.

---

## Phase 3 — FIX version handling: `fix:lineage`

**Goal.** One field answers its name and datatype at any FIX version.

**Surface.** The FIX field views (new accessors), the registry
(version-filtered reads), the module docs' property table, the tests and the
FIX page.

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
    /// The newest pedigree the dictionary holds: what "FIX Latest" means here.
    pub fn newest(&self) -> Option<(Version, Option<u32>)>;
}
```

`fix:lineage` is the one new metadata key — JSON, canonically rendered
(fixed key order, no whitespace, one text per value):

```json
{"entries":[
  {"since":"2.7","name":"LastShares","type":"int"},
  {"since":"4.3","name":"LastQty","type":"Qty"},
  {"since":"5.0SP2","ep":204,"doc":"…"}
]}
```

### Rules

- **P3-R1. FIX spellings map onto `Version`:** `FIX.4.2` → `4.2`,
  `FIX.5.0SP2` → `5.0SP2`, `FIX.2.7` → `2.7`. The `FIX.` prefix is the
  family and is not stored.
- **P3-R1b. "FIX Latest" is not a version and is never stored as one.** It
  is a moving label for the newest published version plus the extension
  packs since — at writing, FIX 5.0 SP2 with EP309, exactly what the
  Orchestra file announces as `version="FIX.5.0SP2_EP309"`. It resolves to
  the real pair `5.0SP2` + `ep = 309` the generated dictionary carries,
  never `Version::MAX`: a sentinel compares wrongly against a genuine 5.0SP2
  field and goes stale the moment an EP lands.
- **P3-R2. FIXT.1.1 is not modelled.** Session tags carry the application
  version that first defined them; the FIX page names FIXT as a known
  omission.
- **P3-R3. A pedigree is a `Version` and an optional extension pack.**
  `since` carries the version, the optional `ep` beside it the EP number,
  because the specification dates a change either way. Entries compare on
  the pair, version first, stored **oldest first** — so the newest reading
  is the last written and resolution is a scan that stops, not a sort. No
  two entries share a pair.
- **P3-R3b. Ordering within one version is by EP.** `5.0SP2` at EP204 is
  older than at EP309; ignoring the EP puts eleven years in one bucket.
- **P3-R4. `name`** is the spelling from that version on.
- **P3-R4b. Field names are the FIX name, lower-cased.** `lastqty`, not
  `LastQty`; `nopartyids`, not `NoPartyIDs`; `clordid`, not `ClOrdID`. The
  FIX name is *respected* — it is not snake-cased, abbreviated, expanded or
  re-spelled, only lower-cased — so `lastqty` is still exactly the name the
  specification gives, in one case.
  - **Nothing is lost.** Name resolution already folds ASCII case (P7-R10),
    so `field("LastQty")`, `field("lastqty")` and `field("LASTQTY")` all
    answer, and the specification's own casing needs no alias to keep
    working.
  - **The human label is a different fact and stays where it is.** The
    generic `display` key already carries `Client order ID` beside `clordid`
    and `Average price` beside `avgpx`. Do not put `ClOrdID` there: a label
    is prose for a reader, a name is an identifier, and the seed dictionary
    already keeps them apart.
  - **Three things are never lower-cased:** a `MsgType`, which is
    case-bearing (`A` is Logon and `a` is QuoteStatusRequest — P8-R3); a
    code's wire value, for the same reason; and a comp id, which is stored
    as it arrives and compared folded (P2-R9d).
  - The law binds what a generator writes (P6-R7b) and what a parser builds
    (P7-R12b). Until Phase 6 regenerates it, the committed dictionary is
    still mixed-case and resolves identically — do not hand-edit it into
    line.
- **P3-R5. `type`** is the FIX datatype name from that version on, in the
  spelling the grammar already resolves (`Qty`, `int`, `UTCTimestamp`), so
  the decoder needs no second table and the document stays readable.
- **P3-R6. `deprecated: true` and `removed: true`** mark the states the
  specification gives them; a `removed` entry ends the field's life. A
  version that stops naming a field has removed it — the generator writes
  that entry, a reader never infers it.
- **P3-R7. Every key beyond `since` is optional.** An entry stating only
  `since` means "present, unchanged", which most versions are.
- **P3-R8. The lineage is the authority; two derivations are computed by the
  writer so they cannot drift.** (a) The field's own `name()` and `dtype()`
  MUST equal the newest entry's — `set_lineage` refuses a disagreement,
  naming both sides. (b) `fix:aliases` is rewritten from the lineage's
  historical names on the same call, so a query by an old spelling resolves
  through the index that already exists.
- **P3-R9. No `fix:since`, `fix:until` or `fix:deprecated` key.** `since()`
  is the first entry's, `until()` the `removed` one's, derived on read the
  way `FixId` is derived from branch and tag (N4).
- **P3-R10. The registry stays version-agnostic.** It holds every tag ever
  defined; a version is a **filter on the read**. That is what "defined in
  one version, available in the others" means. No registry-wide default; a
  caller who wants one holds a `Version` beside the registry.
- **P3-R11. `versions()` and `newest()` are derived,** not stored. The first
  means a dictionary cannot claim a version no field is dated in; the second
  is the greatest pedigree pair any lineage carries, and is the whole of
  what "FIX Latest" means for that dictionary (P3-R1b).
- **P3-R12. The transcoding boundary, precisely.** The lineage carries
  enough to rename and retype a field between versions, and Phase 7 spends
  it: `convert_into` walks a message by `defined_at` / `name_at` /
  `dtype_at` alone (P7-R59). What stays out of this brief is the
  *expression-driven* layer — the CBlock `normalization-binding` with its
  conditions, lookups and value mappings, which needs an evaluator. Do not
  start that; state the line in the module docs, because "transcoding" names
  both and only one of them is here.

### Tests

Tag 32 is the worked case:

1. `since()` is `2.7`.
2. `name_at(4.2)` is `LastShares`; `name_at(4.3)` and `name_at(newest())`
   are `LastQty` — the second asked at the real newest, never at `MAX`.
3. `dtype_at(4.0)` is `Int32`; `dtype_at(4.4)` is `decimal64(18,8)`.
4. Two entries at one version and different EPs order by EP (P3-R3b);
   `newest()` answers the real pair, not a sentinel (P3-R1b).
5. A query by `LastShares` and by `LastQty` answers the same field.
6. `field_at(&"4.2", "LastQty")` refuses.
7. A lineage disagreeing with the field's own name is refused (P3-R8a).
8. A field with no lineage answers `None` everywhere and resolves as before.
9. Canonical JSON round trip; a malformed document names its byte position.
10. `fix:aliases` matches the lineage projection for every field in the
    committed dictionary (P3-R8b).

**Docs.** A new section on the FIX page; a `fix:lineage` row in the module
docs' property table.

---

## Phase 4 — code sets: `FixEnumValue`, `fix:codes`, spelling translation

**Goal.** A field carries its FIX code set, and any spelling of a code
reaches the wire value.

**Surface.** A new code-set module inside the FIX module, plus the field
views that read and write it. Tests, the counting-allocator target, a new
FIX benchmark group, the FIX page.

**Never.** Touch `AsciiEnum` or the `field:enum` document.

### Contract

```rust
pub struct FixEnumValue {
    name: SmolStr,                 // symbolic name, "Buy"
    value: SmolStr,                // wire value, "1"
    description: Option<SmolStr>,
    aliases: Vec<SmolStr>,         // venue and per-version spellings
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

`fix:codes`, canonical JSON, the code set's name and id beside its values:

```json
{"name":"SideCodeSet","id":54,"codes":[
  {"name":"Buy","value":"1","since":"2.7","ep":254,"doc":"Buy; …"},
  {"name":"Sell","value":"2","since":"2.7","ep":254}
]}
```

### Rules

- **P4-R1. Ordered by wire value,** so one code set is one text however
  built.
- **P4-R2. Two names may share a value** — that is an alias, the rule
  `AsciiEnum` already states. Two entries may not share a name.
- **P4-R3. Nothing on the hot path builds the whole map or allocates.**
  `codes()` is a lazy iterator of borrowed views, built the way `FixAliases`
  is; `code(value)` scans and stops, using `memchr` for record boundaries
  rather than parsing JSON structurally.
- **P4-R4. The borrowed scan is safe only because the rendering is canonical
  and validated on the way in.** Pin it: a hand-edited document with
  reordered keys is refused, not mis-scanned.
- **P4-R5. `code_value` resolves in this order,** composing `code` then
  `code_by_name` the way `get_field` composes the two halves:
  1. **The text as a wire value, exactly.** `4` is `4`. A spelling that is
     already a legal code is never reinterpreted as somebody's name. The
     early-exit fast path.
  2. **The folded symbolic name, then any alias.** The fold is the crate's
     one fold — casefold, then drop everything that is not a letter or a
     digit — so `PercentageWaivedCashDiscount`,
     `percentage_waived_cash_discount`, `PERCENTAGE WAIVED CASH DISCOUNT`
     and the hyphenated form are one spelling.
  3. **The leading parenthesized abbreviation of the description.**
     `"Good Till Date (GTD)"` answers `gtd`.
- **P4-R6. Two traps in tier 3, both cases.** A *numeric* parenthesization
  is a tag cross-reference, never a spelling — `"Broken date; SettlDate (64)
  is required"` must leave `64` alone. Only the abbreviation on the leading
  phrase counts — `"Swap Value Factor (SVP) through a central counterparty
  (CCP)"` answers `svp`, not `ccp`.
- **P4-R7. An unresolved spelling falls through unchanged; never an error.**
  `code_value` answers `None` and the caller keeps its text, because a venue
  sends codes no dictionary lists exactly as it sends fields no dictionary
  names.
- **P4-R8. An ambiguous spelling resolves to nothing.** Two codes folding to
  one spelling (`Cross`, `cross!`) answer `None` rather than whichever the
  scan met first. So the name tier does **not** early-exit: it runs the whole
  set and answers only on exactly one match — affordable because tier 1 is
  the hot path, and a spelling lookup comes from human or JSON input.
- **P4-R9. Version-scoped.** `code_value_at` / `code_name_at` skip a code
  whose `since` is later, or whose `deprecated` is at or before, the version
  asked for. A 4.2 message cannot resolve a name added in 4.4.
- **P4-R9b. A code's pedigree is Phase 3's pair,** stored as real numbers.
  Many codes are dated by EP alone: `BasisPoints` is "Added EP208", not
  "added in Latest". Never write a moving label into a stored pedigree.
- **P4-R10. JSON, not separator-delimited text,** because metadata values
  may not contain control characters.

### Decided

- **`fix:codes` is a second key, not a second copy.** `AsciiEnum` is name →
  ASCII value and packs every member through the field's own width, so it
  accepts only ASCII-width and coded datatypes. Most FIX code sets sit on
  `int`, `Boolean` or `String` fields and cannot use it at all, and none can
  carry a description or a pedigree. A field may carry both; neither derives
  from the other, so N4 holds.
- **No English expansion.** *Rejected:* yggfin's `identifier` → `id`, which
  makes `shortcodeid` reach `"Short code identifier"`. A guess about
  English, not about FIX.
- **Tier 3 is provisional.** Drop it if it proves noisy over the generated
  dictionary; P4-R6's two cases decide.

### Tests

Fixture A — `SideCodeSet`: `Buy`=1, `Sell`=2 (FIX.2.7, EP254),
`Undisclosed`=7 (4.1), `CrossShort`=9 (4.2), `CrossShortExempt`=A (4.3).
Fixture B — `CommTypeCodeSet` (tag 13, `char`): `PerUnit`=1, `Percent`=2,
`Absolute`=3, `PercentageWaivedCashDiscount`=4,
`PercentageWaivedEnhancedUnits`=5, `PointsPerBondOrContract`=6,
`BasisPoints`=7 (EP208), `AmountPerContract`=8.

1. Lookup by value, by name, by folded name; an alias pair sharing a value.
2. `4` → `4` (tier 1); all four foldings of
   `PercentageWaivedCashDiscount` → `4`; `PercentageWaivedEnhancedUnits` →
   `5`, so a shared prefix does not collide.
3. `code_name("4")` → `PercentageWaivedCashDiscount`.
4. An unknown spelling falls through (P4-R7); an ambiguous pair answers
   `None` (P4-R8).
5. Tier 3: `gtd` → `6`; `64` and `ccp` left alone (P4-R6).
6. Version filter hides `CrossShort` at `4.1`; `BasisPoints` unresolvable
   before it existed; a deprecated code hidden at and after its version.
7. Canonical JSON round trip; malformed names its byte position; a
   reordered-key document refused (P4-R4).
8. Allocation: `code()` on a 300-code set allocates nothing.

**Bench.** `code()` against a `HashMap` baseline built from the same
document, so the scan is defended with a number.

---

## Phase 5 — `FixFieldMut::merge_with`

**Goal.** One optimized, FIX-aware merge of two definitions of one field.

**Surface.** The FIX field views (the new merge) and the registry (its
`update` calls it; its private merge helper is deleted). Tests, the
counting-allocator target, the FIX mutation benchmark group, the FIX page.

**Never.** Leave the private merge helper in place (N3), or add a priority
or source field to any core type.

### Why the current path goes

The registry's private `merge` builds a new `Metadata`, walks it into the
field, then reads back and rewrites `fix:tags` and `fix:aliases` — three
metadata rewrites and a `Vec<String>` of every key.
`ProtocolFieldMut::merge_with` is worse: it collects every held property
name into an owned `String`, then scans `O(n*m)`.

### Contract

```rust
impl FixFieldMut<'_> {
    /// Folds another definition of the same field into this one.
    pub fn merge_with(&mut self, other: &FixField<'_>) -> Result<()>;
}
```

### Rules

- **P5-R1. Per-key rules, because "merge" alone decides nothing.**

  | key | rule |
  | --- | --- |
  | `fix:branch`, `fix:tag` | MUST agree; a disagreement is a typed refusal naming both. Identity is not merged. |
  | `fix:tags` | union, incoming first, order kept, deduplicated |
  | `fix:aliases` | union, ASCII-folded, incoming first — then **rewritten from the merged lineage** so P3-R8b holds |
  | `fix:description` | **never compared.** Incoming wins when it has one; stored kept when it does not |
  | `fix:lineage` | merged by pedigree: union, incoming wins an equal pair, re-sorted oldest-first, re-validated against the merged name and datatype (P3-R8a) |
  | `fix:codes` | merged by wire value: incoming wins a shared value, stored keeps codes only it has, pedigree carried through, re-rendered canonically once |
  | any other `fix:` key | incoming wins; stored keeps what only it has |

- **P5-R2. Descriptions are never compared** — the longest value a field
  carries, and comparing two costs more than the write it would save.
- **P5-R3. One metadata write.** Build the merged map, write once, never
  touch the field between reads. Three rewrites and their Arrow
  invalidations collapse into one.
- **P5-R4. No key-name allocation.** The `fix:` key set is a `const` list
  beside the field views; walk it, never collect held names into `String`s.
- **P5-R5. Atomic.** A refusal leaves the field exactly as it was.
- **P5-R6. `FixRegistry::update` calls it,** and the private helper is
  deleted. No second merge path survives.

### Decided

- **Precedence is the caller's ordering, not a field on the merge.** Several
  sources describe one tag — FIX Latest, a QuickFIX dictionary, a vendor
  orchestration — and yggfin resolves it with a `priority` per source. The
  generator merges lowest priority first, so the highest-priority source is
  the last `incoming` and wins by P5-R1. One concept, in the one place that
  knows about sources.

### Tests

1. Every row of P5-R1 as its own case.
2. Two fields with different long descriptions: the incoming's survives and
   nothing else moved.
3. A tag disagreement refused, both sides named.
4. A merge adding nothing leaves the field byte-identical.
5. Allocation bound on a merge of two realistic fields.

**Bench.** The new merge against the deleted one's behaviour, over the
committed dictionary.

---

## Phase 6 — the source: FIX Latest into the committed dictionary

**Goal.** Generate and commit the dictionary the other six phases describe.

**Surface.** A generator script beside the repository's existing Python
tooling, and a generated constants module beside the registry. It *writes*
the committed dictionary, a provenance manifest and a branch manifest beside
its trees. It *edits* the registry (the two constant accessors), the tests
and the FIX page.

**Never.** Add an HTTP client, or any dependency, to the crate manifest
(N2). The generator is a script; the crate only ever reads the committed
output through `from_handle`.

### Sources

Browsable, for checking by eye: <https://orchimate.org/fixtrading/fix-latest>
— FIX Latest as of EP309, Orchestra v1.0 — with `/fields`, `/codeSets`,
`/datatypes`, `/messages`, `/components`, `/groups`, `/revisions`; field
pages at `/fields/<Name>`, code sets at `/codeSets/<Name>CodeSet`. Its
"Orchimate MCP" helps for interactive lookups. **HTML is never scraped.**

Machine-readable source of record, under
`https://raw.githubusercontent.com/FIXTradingCommunity/orchestrations/<commit>/`
(percent-encode the space): `FIX Standard/OrchestraFIXLatest.xml`,
`OrchestraFIX44.xml`, `OrchestraFIX42.xml`.

Versions Orchestra does not publish — 4.0, 4.1, 4.3, 5.0, 5.0SP1, 5.0SP2 —
come from the QuickFIX data dictionaries at
<https://github.com/quickfix/quickfix/tree/master/spec> (`FIX40.xml` …
`FIX50SP2.xml`): names, types and enum values per version, enough for
lineage and the only public per-version set that is.

Vendor and community orchestrations come from Orchestra Hub,
`https://orchestrahub.org/api/v3/repos/<owner>/<repo>/revisions/<id>/download`.

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
base; documentation hangs off `fixr:annotation`/`fixr:documentation`.
`added="FIX.2.7"` and `updatedEP="254"` are exactly P1's `Version` and P4's
`ep`. `replaced`/`replacedEP` is the rename axis, but FIX Latest records
only current names — the historical spelling lives in the per-version files,
which is why the script reads all of them.

### Rules

- **P6-R1. Every source URL is pinned to a commit, never `master`.** yggfin
  does exactly this — FIX Latest at
  `.../orchestrations/099914dd0edd49a699326f0441776d6e21cfaf93/…`, QuickFIX
  at `.../quickfix/3536699e830e65f875df4a50b647a6d3bad3b884/…` — because a
  dictionary regenerated from `master` is not reproducible and its diff is
  unreviewable.
- **P6-R2. `--source`** takes a local directory or a URL base so a run is
  reproducible offline; the default is the pinned set.
- **P6-R2b. Record the real version label, never "Latest".** The Orchestra
  file announces its own — `version="FIX.5.0SP2_EP309"` — and that is what
  the provenance stores and what every pedigree is dated against: version
  `5.0SP2`, EP `309`. Nothing in the output says "Latest": the label moves
  and the dictionary does not (P3-R1b). A regeneration against a later EP
  changes those numbers, which is the diff a reviewer wants.
- **P6-R3. Output is exactly the shard layout the store reads:** a
  `primitive` and a `nested` tree, a folder per branch, one shard per
  `tag / 100` bucket, each a JSON array ordered by canonical identifier,
  byte-identical to what `write_into` produces.
- **P6-R4. Provenance in a manifest beside the trees,** one record per
  source: `source_id`, `format` (`orchestra` | `quickfix`), pinned `url`,
  `sha256` of the bytes, `sha256` of the definitions produced, `branch`,
  `version` label, `priority`, and **`license_url`**. The licence is not
  bookkeeping: this commits a derived copy of FIX Trading Community and
  QuickFIX material and the attribution must travel with it. The two
  checksums give CI a drift test whose second half needs no network.
- **P6-R5. A leaf beside the trees is safe:** the store descends only the
  two trees, so it is never listed. It must not carry the retired-layout
  name the store trips on.
- **P6-R5b. Fill the branch manifest, not just the shards.** Every source
  that is a dialect states what P2-R9b bundles: a vendor orchestration its
  FIX version, a `.cfb` its version *and* its `SenderCompID`/`TargetCompID`.
  The standard branch declares nothing (P2-R9c) — it is the specification,
  not a counterparty.
- **P6-R6. Pull one vendor dictionary into a non-standard branch in the same
  run,** so P2's digest table and P7's branch inference have real data under
  them rather than a fixture.
- **P6-R7. Datatypes resolve through `LOGICAL_NAMES`,** already the FIX
  Latest datatype table: `Qty` is `decimal64(18,8)`, `SeqNum` is `int64`, no
  second mapping. A FIX datatype the table does not hold is a hard failure,
  never a silent `utf8`.
- **P6-R7b. Every field name is written lower-cased** (P3-R4b), in the
  shards and in the lineage entries alike, so the field's own name and its
  newest lineage entry still agree (P3-R8a). Assert over the whole generated
  dictionary that no name holds an uppercase byte, and that no two fields
  collide once lower-cased — as no two FIX fields differ only by case, that
  assertion is what proves it.
- **P6-R8. The generator narrows nothing the standard did not.**
  `CheckSum(10)` is three digits with leading zeros and stays `String`;
  `BodyLength(9)` is `Length`, so a malformed one nulls a field rather than
  costing a batch (P7-R24); `MonthYear` stays `ascii(8)` because `202608` is
  a month and `202608w2` a week; `SecureData(91)`, `XmlData(213)` and
  `Signature(89)` are `binary` though the wire carries them as text. Each is
  a case in yggfin's `test_fields.py` because each was got wrong once.
- **P6-R9. Repeating groups become the nested tree as today:** a List of a
  non-null `item` Struct whose `fix:tag` is the counter's.
- **P6-R10. Priority ordering.** Merge sources lowest priority first so the
  highest wins by P5-R1.

### The standard header and trailer

- **P6-R11. Order, not nesting.** `FixMsg` lays a message flat (P7), so both
  components are generated `const` tag lists in a module beside the
  registry, read through `standard_header_tags()` and
  `standard_trailer_tags()`.
- **P6-R12. They are not registry entries.** A component has no tag and
  `insert` admits only a field carrying one (L1); a synthetic tag would put
  a fiction in the identity space. The component names and ids are dropped,
  and the module docs name that loss. Every field *in* them is an ordinary
  entry by its own tag.
- **P6-R13. The constants are the union across every scraped version,** in
  canonical order, with `defined_at` deciding what a version may carry. FIX
  Latest alone is not enough: yggfin's `versions.json` records
  `SecureDataLen(90)` and `SecureData(91)` in the 4.0–4.4 headers, which FIX
  Latest no longer lists, and the trailer is `CheckSum` alone in Latest but
  `SignatureLength(93)`, `Signature(89)`, `CheckSum(10)` in 4.2 and 4.4.
- **P6-R14. Requiredness rides the lineage, not a parallel table** —
  presence is already `nullable` on the field a version resolves to.

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

Tag 32, end to end: FIX Latest says id 32, `LastQty`, type `Qty`, added
`FIX.2.7`; `OrchestraFIX42.xml` and `FIX42.xml` say tag 32 is `LastShares`,
`Qty` in 4.2 and `int` in 4.0/4.1. The generated field is `LastQty`,
`decimal64(18,8)`, `fix:tag` 32, `fix:aliases` containing `LastShares`, and
a `fix:lineage` of `{"since":"2.7","name":"LastShares","type":"int"}`,
`{"since":"4.2","name":"LastShares","type":"Qty"}`,
`{"since":"4.3","name":"LastQty","type":"Qty"}`.

### Tests

1. Load the committed dictionary and assert every line of the worked case.
2. Load the generated tree and write it back: byte-identical (P6-R3).
3. Every tag in both header constants resolves in the generated dictionary;
   the vendor branch's manifest entry carries its declared version and the
   standard branch's carries nothing (P6-R5b, P2-R9c).
4. The generated header matches yggfin's `versions.json` ordering and
   required flags for 4.2 and 4.4 (P6-R13, P6-R14).
4b. No name in the generated dictionary holds an uppercase byte, and no two
    collide once lower-cased (P6-R7b); a lookup by the specification's own
    casing still answers (P3-R4b); `display` still carries the human label.
5. The two checksums match a regeneration from the same pinned inputs.

**Docs.** Provenance, version coverage, the FIXT omission (P3-R2), the
dropped component names (P6-R12) and the regeneration command, on the FIX
page.


---

## Phase 8 — coded FIX datatypes: `Side` and `MsgType`

**Goal.** Two universal, cross-version coded datatypes over a fixed ASCII
width, so a side and a message type are one packed integer rather than a
string compared byte by byte.

**Depends.** Phase 4 (code sets), Phase 6 (a generated vocabulary).

**Surface.** Two variants in the datatype layer and their discriminants, a
generated vocabulary constant beside the other prebuilt code vocabularies,
and the generator line that types tags 35 and 54 with them. Tests, the
datatype benchmark, the datatype and FIX pages.

**Never.** Add a *constraint*. These declare a vocabulary, not a whitelist
(P8-R5).

### Rules

- **P8-R1. They join the coded family, they do not invent a mechanism.**
  `Country`, `Currency`, `Mic` and `Cfi` are already parameter-free variants
  over a fixed ASCII width whose members are a `const` listing prebuilt in
  the enum registry, reachable by logical name. `Side` and `MsgType` are two
  more of exactly that. `DataTypeId` gains two variants **appended** (L2).
- **P8-R2. Width 4, for both.** FIX's longest `MsgType` wire value is two
  characters (`AA`, `AB`, `BZ`), `Side`'s is one. Four bytes packs into a
  `u32` through the existing ASCII packing, which makes a side or a message
  type a `Copy`, orderable, allocation-free key — and leaves headroom for a
  vendor code of three or four. A value longer than the width is the same
  refusal any fixed-ASCII field gives.
- **P8-R3. `MsgType` is case-sensitive, and this is the rule that will be
  got wrong.** `A` is Logon and `a` is QuoteStatusRequest; `Q` is
  DontKnowTrade and `q` is OrderMassCancelRequest; `S` is Quote and `s` is
  NewOrderCross. The crate's one fold — which serves field names, keys and
  code *spellings* (P4-R5.2) — must **never** touch a `MsgType` wire value.
  Only its symbolic name folds. Assert the six colliding pairs above as one
  test, because a single stray fold turns a quote into a cross.
- **P8-R4. The vocabulary is the union across every scraped version,**
  generated by Phase 6 the way the header constants are (P6-R11), so one
  datatype answers for every version and a 4.2 message and a newest one
  agree about what `D` means. Per-member pedigree stays in the field's
  `fix:codes` document (P4); the datatype's own listing is names to values
  and nothing more, because a datatype is parameter-free and a listing is a
  constant every reader answers identically.
- **P8-R5. A value outside the listing is still storable.** These are
  declared vocabularies, exactly as `Mic` is: a venue's own message type or
  a side no version defines is held, not refused. The listing is what
  `code_name` answers from, not a gate on the value. Refusing here would
  drop data, which this brief has refused to do three times already
  (P7-R11, P7-R12, P4-R7).
- **P8-R6. The generator types tag 35 as `MsgType` and tag 54 as `Side`.**
  That is not the narrowing P6-R8 forbids — the standard itself declares
  both as code sets, so this is honouring the declaration rather than
  guessing at one. Every other code set stays its base type: two datatypes,
  not one per code set, because only these two are addressed constantly
  enough to earn a variant.
- **P8-R7. The P1-R11 invariant table applies unchanged,** and so does
  P1-R10: `DataType` is `#[non_exhaustive]` with wildcard arms everywhere, so
  a green build proves nothing. Ten invariants, ten tests, twice.

### Decided

- **Two variants, not a generic `Coded(name)` datatype.** *Rejected:* a
  parameterized coded type keyed by code-set name. It would make the
  datatype carry a name the registry already owns, and every reader would
  need the registry to know what a value means — the opposite of a
  parameter-free discriminant that compares and hashes without touching
  nested state.
- **Width 4 rather than 2.** *Rejected:* the tight fit. Two bytes packs into
  a `u16` and holds every standard `MsgType`, and the first venue with a
  three-character code would need a datatype change, which is a wire
  contract change (L2). Four is one `u32` either way.

### Tests

1. `ascii_packed` round trip for `D`, `AB`, `1`, `A`, at width 4.
2. The six case-colliding `MsgType` pairs resolving to six distinct values,
   and the crate fold never applied to a wire value (P8-R3).
3. A `Side` and a `MsgType` outside the listing stored and read back
   (P8-R5).
4. Resolution by logical name; the prebuilt listing identical between two
   readers.
5. The P1-R11 invariant table, once per variant (P8-R7).
6. The generated dictionary types tag 35 and tag 54 with them (P8-R6).

**Bench.** A packed `MsgType` compare against the `utf8` compare it
replaces, in the datatype benchmark target.

---

## Handoff

**From Phase 3.** Phases 4–7 read the lineage: `name_at` / `dtype_at` /
`defined_at`, which Phase 7 builds every field through (P7-R17) and converts
messages by (P7-R59); `versions()`
and `newest()` (P3-R11), which Phase 7 checks coverage against and falls back
to (P7-R15.4); the pedigree pair (P3-R3); the `set_lineage` derivation of
`fix:aliases` (P3-R8b), which Phase 5 must preserve and Phase 6 writes
through.

**From Phase 4.** `code_value_at` (P4-R5, P4-R9), which Phase 7 translates
every value through before typing (P7-R19); `code_by_name`, which resolves
`ApplVerID`'s symbolic spellings (P7-R15.1); the crate's one fold (P4-R5.2),
which Phase 7 folds keys with (P7-R10); the fall-through rule (P4-R7), which
Phase 7 depends on not being an error path; `fix:codes` merge semantics for
Phase 5.

**From Phase 5.** Phase 6 merges several sources describing one tag and
relies on one pass with P5-R1's per-key rules. Precedence is merge order,
lowest priority first (P6-R10), so nothing in the core learns about sources.

**From Phase 6.** Phase 7 needs the committed dictionary with lineages and
code sets populated; `STANDARD_HEADER_TAGS` and `STANDARD_TRAILER_TAGS`
(P6-R11) as the message layout order (P7-R21); at least one non-standard
branch with its manifest entry (P6-R6, P6-R5b), so branch inference (P7-R16)
is tested against real data; and `data`-typed tags (P6-R8), which P7-R50
reads to find length-prefixed fields.

**From Phase 8.** Phase 9 lifts a side and a message type as packed keys
rather than strings (P8-R2), and the auto-enrich rules key on `MsgType`
(P9-R3). Nothing else depends on it: a dictionary generated before it types
tags 35 and 54 as their base types and still loads.
