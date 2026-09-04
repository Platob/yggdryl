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
  version that first defined them. the FIX documentation page names FIXT as a known
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

Tag 32 is the worked case, verbatim in the FIX module's tests:

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
   the committed dictionary (P3-R8b).

**Docs.** A new section on the FIX documentation page; a `fix:lineage` row in the
property table at the top of the FIX module docs.

---

## Handoff

Phases 4, 5, 6 and 7 all read the lineage:

- `name_at` / `dtype_at` / `defined_at` - Phase 7 builds every field through
  them (`P7-R17`).
- `versions()` (`P3-R11`) - Phase 7's inference checks coverage against it.
- The `set_lineage` derivation of `fix:aliases` (`P3-R8b`) - Phase 5's merge
  must preserve it, and Phase 6's generator writes through it.
- `Version::MAX` as `FIX.Latest` (`P3-R1`) - Phase 7 falls back to it.
