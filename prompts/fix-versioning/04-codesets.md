# Phase 4 — code sets: `FixEnumValue`, `fix:codes`, and spelling translation

**Goal.** A field carries its FIX code set, and any spelling of a code
reaches the wire value.

**Depends.** Phases 1 and 3.

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

**Surface.** A new code-set module inside the FIX module, plus the field
views that read and write it. The FIX module's tests, the
counting-allocator target, a new FIX benchmark group, and the FIX
documentation page.

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
  values may not contain control characters (the metadata layer refuses them).

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
**Docs.** the FIX documentation page.

---

## Handoff

Phases 5, 6 and 7 take:

- `code_value_at` (`P4-R5`, `P4-R9`) - Phase 7 translates every value through
  it before typing (`P7-R19`).
- `code_by_name` - Phase 7's `ApplVerID` symbolic spellings resolve through
  it (`P7-R15.1`).
- The crate's one fold (`P4-R5.2`) - Phase 7 folds keys with the same one
  (`P7-R10`).
- The fall-through rule (`P4-R7`) - Phase 7 depends on it not being an error
  path.
- `fix:codes` merge semantics for Phase 5 (`P5-R1`).
