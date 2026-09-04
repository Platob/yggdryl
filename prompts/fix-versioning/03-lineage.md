# Phase 3 — FIX version handling: `fix:lineage`

**Goal.** One field answers its name and datatype at any FIX version.

**Depends.** Phase 1.

> **Read `00-contract.md` first.** It is short and binding: the never-list
> `N1`–`N7`, the landed facts `L1`–`L2`, the precedence rule, and the command
> block that says a phase is done. Nothing below repeats it.
>
> **Never, in short:** no public symbol or dependency this brief does not
> name; no compatibility shim or second path; no fact stored that is already
> derivable; no widening for the next phase; no `TODO`, `#[allow]` or ignored
> test; and never guess where a rule says refuse, or refuse where it says
> fall through.

---

**Surface.** The FIX module: the field views (the new accessors), the
registry (the version-filtered reads), and the module docs' property table.
The FIX module's tests and the FIX documentation page.

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

`fix:lineage` is the one new metadata key, a JSON document rendered
canonically the way `AsciiEnum::into_json` is - fixed key order, no
whitespace, one text per value:

```json
{"entries":[
  {"since":"2.7","name":"LastShares","type":"int"},
  {"since":"4.3","name":"LastQty","type":"Qty"},
  {"since":"5.0SP2","ep":204,"doc":"…"}
]}
```

### Rules

- **P3-R1. FIX spellings map onto `Version` in `rust/src/fix/`:** `FIX.4.2`
  → `4.2`, `FIX.5.0SP2` → `5.0SP2`, `FIX.2.7` → `2.7`. The `FIX.` prefix is
  the family and is not stored.
- **P3-R1b. "FIX Latest" is not a version and is never stored as one.** It
  is a *moving label* for the newest published application version plus the
  extension packs applied since - at the time of writing, FIX 5.0 SP2 with
  EP309, which is exactly what the Orchestra file announces as
  `version="FIX.5.0SP2_EP309"`. So it resolves to the real pair `5.0SP2` and
  `ep = 309`, taken from the dictionary that was generated, never to
  `Version::MAX`. A sentinel would compare wrongly against a genuine 5.0SP2
  field and would go stale the moment an EP lands.
- **P3-R2. FIXT.1.1 is not modelled.** Session tags carry the application
  version that first defined them. the FIX documentation page names FIXT as a known
  omission.
- **P3-R3. A pedigree is a `Version` and an optional extension pack.**
  `since` carries the version; the optional `ep` beside it carries the EP
  number, because the specification dates a change either way - some
  entries state a version, some state only an EP against the version in
  force. Entries compare on the pair, version first, and are stored
  **oldest first**, so the newest reading is the last written and resolution
  is a scan that stops, not a sort. No two entries share a pair.
- **P3-R3b. Ordering within one version is by EP.** `5.0SP2` at EP204 is
  older than `5.0SP2` at EP309, and a lineage that ignored the EP would put
  eleven years of changes in one indistinguishable bucket.
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
- **P3-R11. `versions()` and `newest()` are derived,** not stored - the
  first is yggfin's `versions.json` `declared` list, and deriving it means a
  dictionary cannot claim a version no field is dated in. `newest()` is the
  greatest pedigree pair any lineage carries, and it is the whole of what
  "FIX Latest" means for that dictionary (P3-R1b): a generated dictionary
  answers `5.0SP2` with its own EP, and a hand-built one answers whatever it
  actually holds.
- **P3-R12. The transcoding boundary.** The lineage carries enough to rename
  and retype a value between two versions. Actually rewriting a message -
  walking a root `Field`, renaming children, casting values - belongs to the
  `.cfb` `normalization-binding` phase. Do not start it. State the boundary
  in the module docs.

### Tests

Tag 32 is the worked case, verbatim in the FIX module's tests:

1. `since()` is `2.7`.
2. `name_at(4.2)` is `LastShares`; `name_at(4.3)` and `name_at(newest())`
   are `LastQty` - the second asked at the dictionary's real newest version,
   never at `Version::MAX`.
3. `dtype_at(4.0)` is `Int32`; `dtype_at(4.4)` is `decimal64(18,8)`.
4. Two entries at one version and different EPs order by EP (P3-R3b).
5. `newest()` answers the real pair the generated dictionary carries -
   `5.0SP2` with its EP - and not a sentinel (P3-R1b).
6. `registry.field("LastShares")` and `registry.field("LastQty")` are the
   same field.
7. `field_at(&"4.2", "LastQty")` refuses.
8. A lineage disagreeing with the field's own name is refused (P3-R8a).
9. A field with no lineage answers `None` everywhere and resolves as before.
10. Canonical JSON round trip; a malformed document names its byte position.
11. `fix:aliases` matches the lineage projection for every field in
    the committed dictionary (P3-R8b).

**Docs.** A new section on the FIX documentation page; a `fix:lineage` row in the
property table at the top of the FIX module docs.

---

## Handoff

Phases 4, 5, 6 and 7 all read the lineage:

- `name_at` / `dtype_at` / `defined_at` - Phase 7 builds every field through
  them (`P7-R17`).
- `versions()` and `newest()` (`P3-R11`) - Phase 7 checks coverage against
  the first and falls back to the second (`P7-R15.3`).
- The `set_lineage` derivation of `fix:aliases` (`P3-R8b`) - Phase 5's merge
  must preserve it, and Phase 6's generator writes through it.
- The pedigree pair - version and EP (`P3-R3`) - and `newest()`, the real
  latest the dictionary holds, which Phase 7 falls back to rather than to a
  sentinel (`P3-R1b`).
