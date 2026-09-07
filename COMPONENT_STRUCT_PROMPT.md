# Component structs brief — naming a group's occurrence

A repeating group's members sit in a Struct literally named `item`. That name
carries nothing: it does not say what an occurrence *is*, it makes 521 groups
indistinguishable in a catalog, and it is the reason a component has never been
a notion this crate can name. Give that Struct the component's own name —
`FixEntry`, `TrdRegTimestamp`, `PartyID` — derived from the counter that heads
it by one rule, so a group's contents are categorizable, a component is a thing
with a name, and messages and components are described by one vocabulary. Then
make the crate's own arrival record follow the same pattern: a `NoFixEntries`
list of a `FixEntry` component, recursive, materialized two Arrow levels deep
with a binary leaf below.

## How to run

- **One phase per PR.** Each `## Phase` is complete work: it compiles, its tests
  pass, its docs are written, the repository ships.
- **No paths anywhere** outside this brief's own citations. Everything is named
  by what it does and found by symbol.
- **Precedence.** `AGENTS.md` > this brief > your priors about FIX or English.
- **Decided** = settled; implement it, do not relitigate. The rejected
  alternative is recorded so you need not rediscover why it lost.
- **Verify** = the check comes before the code.
- **Rule ids** (`G1-R4`) are stable. Use them in commits, PR text and review
  replies.
- **Every Decided reason belongs in the doc comment** of the thing it decides. A
  rule explained only here is a rule the next reader undoes.

## What is landed

Reuse unchanged, found by symbol.

| notion | what it is |
| --- | --- |
| `FixBranch`, `FixId`, `FixKey` | one field's identity and the three ways a caller names one |
| `FixField` / `FixFieldMut` | the borrowed views over the `fix:` namespace |
| `FixRegistry` | tiered resolution over a primitive and a nested half, with `insert`, `update` |
| its store | `from_handle` / `write_into` over one folder handle, one shard per `tag / 100` |
| `FixMsg`, `FixEntry` | a value plus the registry that types it, and the arrival record beside it |
| the committed dictionary | the seed registry read through the store, at `config/fix` |

**L1.** A repeating group is a List field carrying the counter's own name and
`fix:tag`; its item is a non-null Struct named `item`. **This brief renames the
item and changes nothing else about a group.** The counter stays the list
holder, keeps its tag, and the decoder's counter-tag fork — `get_primitive_field`
in `resolve`, `get_nested_field` in `counter` — keeps working untouched.

**L2.** `FixRegistry::insert` admits only a field carrying `fix:tag`. The item
is a child of a list, not an entry, so nothing here changes that.

## The measured case

Every number below was computed from the shipped tree, not estimated. Report
them again at the end of each phase that moves one.

| fact | value |
| --- | --- |
| shipped group (List) fields | 521, across 63 of 63 nested shards |
| items named `item` | 521 of 521, every one a Struct |
| lists inside lists | 0 |
| counter displays starting `No` + uppercase | 521 of 521 |
| distinct derived names | 521, with 0 collisions among themselves |
| **derived name equal to a member of its own item struct** | **269 of 521 (51.6%)** |
| derived name equal to any shipped field name | 272 of 521 |
| `GROUP_TAGS` shadowed | 3 of 3 — 453, 454, 768 |
| shipped fields | 6,203 — 5,682 primitive, 521 nested |
| `config/fix` on disk | 8,013,865 bytes |

The middle row is the whole difficulty. `NoPartyIDs` derives `PartyID`, and the
struct it names holds a member `partyid` (tag 448). `folded_child` tests the
item's own name *before* recursing into it, so naming the item `PartyID` makes
`NoPartyIDs.PartyID` answer the Struct and silently shadow tag 448 — and the
three groups the fixed row schema persists whole are all in that set. **G2 makes
the walk list-transparent before G3 renames anything.** Renaming first is a
silent decoder regression: `resolve` filters a nested hit out, `field_for`
invents an untyped text column, and nothing fails.

## Non-goals

Two ideas were considered against the corpus and rejected. Do not reintroduce
either.

- **Spec-named group entities.** Naming the group `InstrmtGrp` and demoting the
  counter to a plain int. It breaks the decoder's only routing: a counter tag
  can never name a group, because 22 counter tags head more than one Orchestra
  group (146 heads twelve, 555 heads ten), covering 77 groups across 101 of the
  181 messages.
- **Taking Orchestra's name for the item.** Orchestra names the *collection*,
  never the occurrence: 453 is `Parties`, 454 is `SecAltIDGrp`, and 63 of 580
  group names are still plural after dropping `Grp`. It also cannot be a
  drop-in — its Grp-stripped name agrees with the rule on only 354 of 575
  resolvable groups, and 21 shipped counters carry a tag with 2–12 competing
  Orchestra names. **The derivation is therefore the source for 521 of 521, not
  a fallback for a minority.** Build no name table and no per-tag override.

## Phase G1 — the singular rule

One pure function, one rule, no table of exceptions.

```rust
/// Returns the component name one repeating-group counter heads.
pub(crate) fn component_name(counter_display: &str) -> Option<SmolStr>;
```

**G1-R1.** Read the counter's `display` spelling, never its folded name. The
fold destroys the uppercase run the strip and the Latin arm both depend on.

**G1-R2.** Strip a leading `No` only where the next character is ASCII
uppercase; otherwise answer `None` and let the caller name the item with the
counter's own name. Measured: 521 of 521 strip, and the 10 shipped `No…`
non-counters (`NotifyBrokerOfCredit`, `NonCashDividendTreatment`, …) are not
List-typed and never reach the rule.

**G1-R3.** Apply the rule only where the datatype is a List. "A name beginning
`no`" is not a counter test — it admits those 10 non-counters and 5 tagged
counters that ship no group body (tags 82, 1342, 1499, 1669, 1919).

**G1-R4.** Singularize the stripped stem in seven ordered arms, first hit wins:

| # | arm | rule | corpus |
| --- | --- | --- | --- |
| 1 | Latin | `matrices`→`matrix`, `indices`→`index`, `appendices`→`appendix`, `vertices`→`vertex`, matched case-insensitively at the end of the stem, longest first | 2 |
| 2 | already singular | stem does not end in a byte-exact lowercase `s` → unchanged | 29 |
| 3 | `ss` | stem ends `ss` → unchanged | 0 |
| 4 | `ies` | stem longer than 4 → drop `ies`, append `y` | 9 |
| 5 | `sses` | drop `es` | 3 |
| 6 | sibilant `es` | stem ends `x`/`ch`/`sh`/`zz` before `es` → drop `es` | 0 |
| 7 | otherwise | drop exactly the final `s`, nothing more | 478 |

**G1-R5.** Arm 1 preserves the case of the first replaced character.
`NoContractualMatrices` answers `ContractualMatrix`, never `Contractualmatrix`.
This arm and this arm alone rewrites inside the stem, and it is the only place
case can be lost. It fires exactly twice in the whole dictionary
(`NoContractualMatrices`, `NoLegContractualMatrices`), which is why it is the
arm that will be got wrong.

**G1-R6.** Arm 2 tests a **byte-exact lowercase `s`**. Measured: 492 stems end
in lowercase `s`, 28 in neither case, and exactly one — `NoSideTrdRegTS` — in
uppercase `S`. Case-folding that test would answer `SideTrdRegT` and lose a
letter from an acronym.

**G1-R7.** No arm removes more than the final `s` except arms 1, 4 and 5, each
of which requires its full suffix. That, and nothing else, protects a trailing
uppercase run: `IDs` matches no multi-letter arm, so `NoPartyIDs` answers
`PartyID`. Measured: 44 counters end in `IDs` and 0 of them match any
multi-letter arm. **Do not add an acronym step** — it is a no-op on 521 of 521.

**G1-R8.** Ship no override table, no per-tag exception and no
specification-name lookup. Report the census that shows none is needed.

**Decided — two names the rule gets arguably wrong, and both stand.**

- `NoLinesOfText` (tag 33) answers `LinesOfText`, still plural. The rule cannot
  singularize an inner head noun, and Orchestra agrees with it (`LinesOfTextGrp`).
  It is the only one of 521 derived names that stays plural. State it in the
  function's doc comment rather than special-casing one tag.
- `NoOfSecSizes` answers `OfSecSize`, a stem that is not a noun phrase because
  the strip left a preposition. It is the only such case, and Orchestra is no
  help (`SecSizesGrp` is itself plural).

*Rejected:* fixing either by hand. A one-entry exception table is a second
source of truth for one rule, and `N4` forbids storing what the rule derives.

**Verify G1.** A unit test per arm, including the four with zero corpus
occurrences — they exist for the next dictionary, and an untested arm is an
untested rule. Then one test asserting the rule reproduces all 521 shipped item
names from their counters. Report: arm census (2 / 29 / 9 / 3 / 0 / 0 / 478 =
521), distinct derived names (521), self-collisions (0).

## Phase G2 — the walk stops spelling the item

Blocks G3. Landing G3 first shadows 269 paths silently.

**G2-R1.** Resolve a List in the FIX walk by recursing into its item **without
consuming a path segment and without ever matching the item's own name**. Delete
the `folds_equal(item.name(), name)` arm in `folded_child` rather than reordering
it, and make `descend` step through a List/LargeList before it consults
`Field::get_field_by_path`, so the exact-case route cannot match the item either.

**G2-R2.** Leave the core `DataType::get_field_by_path` and
`get_field_by_name` untouched, with their doctests. The transparency is the FIX
walk's; `orders.item.price` is a core contract pinned by four nested-field tests
and a doctest, and non-FIX users keep it.

**G2-R3.** `NoPartyIDs.item.PartyRole` answers `None` after this phase. Declare
it breaking; add no alias, no fallback, no second spelling. Delete every
FIX-side `.item.` spelling rather than aliasing the segment — **14 sites**, and
leave the 10 non-FIX ones alone.

**G2-R4.** The mutation spelling changes with the read spelling, in the same
commit: `453.item.trdregtimestamptype` becomes
`453.trdregtimestamp.trdregtimestamptype` once G3 lands. One spelling changed,
never two kept.

**Verify G2.** A test asserting `NoPartyIDs.PartyID` answers tag 448 while the
item is *still* named `item`, and the same after G3. Then the decisive one:
walk all 269 shadowed groups and assert every derived-name path answers the
member field, not the Struct. Report 269 of 269.

## Phase G3 — the item takes the component's name

**G3-R1.** Name a group's item with the derived component name, folded to ASCII
lowercase, at **three sites**: the decoder's group arm, the CBlock group
builder, and the dictionary generator. Not five — the other two `item` literals
build a Utf8 stand-in and a cloned scalar.

**G3-R2.** A counter that arrived with no members keeps the same derived name
over its Utf8 stand-in, so one group has one item name whatever the item's type.

**G3-R3.** A tag that arrived twice *outside* a group has no counter and no
component: name its item with the repeated field's own name and run no rule on
it. These two shapes must stay distinguishable.

**G3-R4.** The item carries the name and **nothing else**: `metadata = {}`, no
`display`, no `fix:component`. The spelling is the same derivation applied to
the counter's own `display` one level up, and `N4` forbids storing it twice.
Measured: name only costs +10,296 bytes (+0.13% of `config/fix`); a `display`
key on each item costs +39,869 (+0.50%).

*Rejected:* a `display` on each item, for symmetry with fields. A field's
display is not derivable; an item's is.

**G3-R5.** One test proves the generator's emission equals the Rust rule for all
521 shipped groups. Two hosts of one rule drift the moment nothing compares them.

**G3-R6.** The item name is **never identity**. No digest, dedup key, equality
check or drift check may read it. Say so in the doc comment, with the reason:
the Avro writer derives its record name from the item's name while the Avro
reader rebuilds every list child as `item`, so the name does not survive an Avro
hop. **Change no Avro code in this phase** — record the asymmetry, do not fix it
here.

**G3-R7.** Change the docs generator's group test from `item.name === 'item'` to
a structural one (the child is a Struct with members), then regenerate the docs
assets. Its CI drift check fails otherwise and 521 group records silently lose
their group marking.

**G3-R8.** Repeal the `item` clause where the dictionary brief states it, and
rewrite in the same commit: the FIX module doc, the 7 Rust doc comments that
state the contract without spelling `item`, the 9 runnable doc examples that
construct one, and the 5 prose files that describe it.

**Verify G3.** `cargo test --workspace`; the docs-example checker in all three
languages; the docs asset drift check. Report: 521 shard lines changed across 63
of 63 shards, 63 of 128 provenance checksums updated, `config/fix` 8,013,865 →
8,024,161 bytes.

## Phase G4 — the crate's own group

The arrival record follows the pattern it describes.

**G4-R1.** Build both arrival columns from one function, and name the item
struct `fixentry` at every level.

**G4-R2.** The arrival column becomes a counter-named list, spelled
`nofixentries`, with `display` `NoFixEntries`. The second column, holding what
no dictionary explained, becomes `nounmappedfixentries` / `NoUnmappedFixEntries`.
Keep both public constant *symbols* — they name the role, and renaming a symbol
the work does not need is forbidden; only their values move.

*Rejected:* keeping the columns spelled `entries` and `unmapped`. The pattern is
what this brief exists to establish, and a crate that describes every FIX group
as a counter-named list while spelling its own two differently is a crate with
two contracts.

**G4-R3.** Name the item `fixentry` in **both** columns, though the rule would
derive `UnmappedFixEntry` from the second counter. The crate is the naming
authority for its own component, and one component does not get two names.

**G4-R4.** Replace the three bare `"entries"` literals in the batch reader with
the constant. A column name spelled twice is the second path that lets a rename
half-land. Leave the identically-spelled `fix:lineage` JSON key alone — same
spelling, different thing.

**G4-R5.** `NoFixEntries` gets **no** `fix:tag`: not in the crate's field set,
not in the registry, not in a shard. It is built by the schema and exists only
in the fixed row.

**Verify G4.** The batch round-trip; the Python and JavaScript column
assertions; the two Rust column assertions. Report every assertion site moved
(11).

## Phase G5 — the recursive entry

**G5-R1.** The Rust `FixEntry` gains `children: Vec<FixEntry>` as its **last**
field, so member 4 in the struct is member 4 in Arrow and the batch writer's
positional reads stay valid. Keep all **seven** derives. Use `Vec`, not
`Box<[_]>` or `Arc<[_]>`: it already supplies the indirection that makes the
type sized, and an empty `Vec` allocates nothing, so the overwhelmingly common
flat entry does not regress.

**G5-R2.** Keep the three-argument constructor filling an empty `Vec`. Add
exactly one public accessor and one crate-private push. Add no public
constructor taking children.

**G5-R3.** State the depth convention once, in these words: **the column's List
is level 0; the `fixentry` it holds is level 1; the `fixentry` a level-1 entry
holds is level 2; there is no level-3 struct.** "Two levels deep" means two
`fixentry` structs on any root-to-leaf path — five Arrow nodes:

```text
nofixentries : list<fixentry: struct<
    tag int32, branch utf8, key utf8, value utf8,
    nofixentries: list<fixentry: struct<
        tag int32, branch utf8, key utf8, value utf8,
        nofixentries: binary>>>>
```

**G5-R4.** The fifth member keeps the same name at both levels. The meaning is
identical — the entries this entry holds — and the *type*, not a second name,
is what says the subtree was not materialized.

**G5-R5.** Keep the four existing members exactly as they are, all nullable, and
append the fifth. Declare the inner list and both items non-null and the binary
leaf non-null; the outer list stays nullable as today. An empty child list means
no children and an empty leaf means nothing was truncated, so neither pays a
validity bitmap.

**G5-R6.** The leaf holds the **wire bytes** of everything at level 3 and below:
`key=value` pairs framed with `0x01`, in arrival order, verbatim. Not a
serialized document, not a re-typed rendering. Frame with `0x01` whatever
separator arrived — the leaf is a fragment with no frame to infer a dialect
from, and the reader already normalizes an escaped frame to `0x01`.

**G5-R7.** Produce the leaf with the same emitter the message uses and read it
back with the same parser. One emitter, one parser, both altitudes. A second of
either is forbidden.

**G5-R8.** Fold level 3 and below into the leaf in exactly one place — the
function that writes the Arrow arrival value. The builder never folds and the
Rust tree is never truncated: `children` is the whole arrival record, and the
Arrow column is a materialization of it.

**G5-R9.** Attach a member's entry under the counter's entry only when a counter
pair actually arrived; otherwise record it flat at level 1 as today. **Never
synthesize a parent nobody sent** — the entries are the arrival record.

**G5-R10.** The digest walks the tree pre-order and writes each entry's child
count as four big-endian bytes after the value's length-prefixed bytes, then
each child. Write the count for **every** entry, including zero. Without it two
messages differing only below the materialized depth hash alike, which is a
correctness bug, not an optimization. Skip an envelope tag's whole subtree, not
just its entry. Recurse over the slice directly: no intermediate allocation.

**G5-R11.** The wire emitters go pre-order — the entry, then its children — and
apply the control-byte refusal at every level. The anomaly walk goes pre-order
too, so an anomaly below level 1 is still reported; the miscount comparison is
unchanged.

**G5-R12.** The unmapped list stays **flat**: every entry no dictionary
explained, found pre-order at any depth, enters it as a level-1 entry with an
empty child list and an empty leaf. It is a view over what was not explained,
not a second copy of the tree's shape.

**G5-R13.** Refuse nothing on depth alone. An entry thirty levels down folds
into the leaf. A depth refusal would make a legal nested-group message
unreadable, and the arrival record does not do that.

**G5-R14.** Two refusals, in the register of the landed ones:

- folding a key or value below the materialized depth that carries the frame
  byte — `expected a value the frame byte can separate, got one carrying it`;
- reading back a non-empty leaf that yields no pair — `expected a frame of
  key=value pairs, got bytes that split into none`. Refuse rather than skip: a
  re-emitted line must be one that was actually sent.

**Verify G5.** A message with a group inside a group inside a group: assert two
materialized levels, a non-empty leaf, and that the re-emitted wire equals the
input byte for byte. Assert two messages differing only at level 3 produce
different digests. Report: `size_of::<FixEntry>()` before and after, and the
per-row Arrow cost on a capture whose entries never nest.

## Order

`G1 → G3`; `G2 → G3`; `G3 → G4 → G5`. G1 and G2 block on nobody and may land in
either order.

## Never, in this change

Beyond `N1`–`N7` in the repository contract.

- **NG1.** Rename an item before the walk stops matching an item's name. 269 of
  521 paths shadow silently, and the decoder answers an untyped text column
  rather than failing.
- **NG2.** Touch the core `DataType` walk or its doctests to fix a FIX problem.
- **NG3.** Add an alias, fallback or second spelling for `.item.`.
- **NG4.** Build a name table, a per-tag override or a specification-name
  lookup for item names.
- **NG5.** Read an Orchestra group name as an item name. It names the
  collection.
- **NG6.** Let the item's name reach a digest, dedup key, equality check or
  drift check.
- **NG7.** Fix the Avro reader/writer asymmetry in G3. Record it; it is its own
  work.
- **NG8.** Give the item a `display`, a `fix:tag`, or any metadata at all.
- **NG9.** Give a component struct a registry entity, identifier or accessor.
  It stays the named child of a list. *(An explicit component/group lookup
  surface does not exist today — the registry is entirely field-shaped and the
  documentation has no section for one. That is real, and it is the phase after
  this brief: it is only buildable once components have names, which is what
  this work delivers.)*
- **NG10.** Synthesize a parent entry for a group whose counter never arrived.
- **NG11.** Refuse a message for nesting deeper than two levels.

## Done when

```console
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --workspace --features "parquet iceberg"
cargo check -p yggdryl --no-default-features --lib
RUSTDOCFLAGS="-D warnings" cargo doc -p yggdryl --no-deps
python scripts/check_docs_examples.py --lang rust
python scripts/check_docs_examples.py --lang python
python scripts/check_docs_examples.py --lang javascript
node scripts/build_docs_fix.js --check
python -m mkdocs build --strict
```

passes, plus the benches the phase names — and you have reported exact results
including anything skipped.

## Report these numbers

A phase that misses a number it promised reports the measurement; it does not
drop the promise.

| number | expected |
| --- | --- |
| arm census of the singular rule | 2 / 29 / 9 / 3 / 0 / 0 / 478 = 521 |
| distinct derived names, self-collisions | 521, 0 |
| shadowed paths proven to answer the member | 269 of 269 |
| item names renamed in the shards | 521, across 63 of 63 nested shards |
| provenance checksums updated | 63 of 128 |
| `config/fix` size | 8,013,865 → 8,024,161 bytes (+0.13%) |
| FIX-side `.item.` sites deleted | 14 (of 24 repo-wide; the other 10 are not FIX's) |
| arrival-column assertion sites moved | 11 |
| `size_of::<FixEntry>()` | before and after |
| per-row Arrow cost on a never-nesting capture | measured, not estimated |
