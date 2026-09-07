# Component structs brief — naming a group's occurrence

A repeating group's members sit in a Struct literally named `item`. That name
carries nothing: it does not say what an occurrence *is*, it makes 521 groups
indistinguishable in a catalog, and it is the reason a component has never been
a notion this crate can name. Give that Struct the component's own name —
`FixEntry`, `TrdRegTimestamp`, `PartyID` — derived from the counter that heads
it by one rule, so a group's contents are categorizable, a component is a thing
with a name, and messages and components are described by one vocabulary. Then
make the crate's own arrival record follow the same pattern: a `NoFixEntries`
list of a `FixEntry` component, recursive, materialized three Arrow levels deep
with a binary leaf below holding the rest as the crate's own JSON.

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

## Phase G6 — the entry names its branch by id

Runs before G5, so the entry's member list settles before recursion appends to
it. Its id is stable; the brief's phases are ordered by the `## Order` line, not
by their numbers.

`FixEntry` holds `branch: Option<SmolStr>` — the dialect's *name*. Replace it
with `bid: i64`, the branch's digest, and make the registry publish the table
that resolves one.

**G6-R1.** Rename the member to `bid` and type it `i64`, holding the value
`FixBranch::digest` answers. It is the same number `FixId` already packs into
its low 32 bits, so an entry's dialect and a field's identity now spell their
branch identically.

**G6-R2.** Type it `int64` in Arrow, **not** `int32` or `uint32`. The digest is
a `u32`: half its range exceeds `i32::MAX` and would render negative, and Avro
has no unsigned integer type at all. `int64` is the smallest signed type that
holds every digest exactly and round-trips through Arrow IPC, Parquet and Avro.

**G6-R3.** Declare it **non-null**, and drop the `Option`. The standard branch's
digest is `0` — `STANDARD_BRANCH_DIGEST` — and `FixId::from_parts` already
refuses a non-standard branch whose digest collides with it, so `0` means the
standard branch and nothing else. Today's `None` becomes `0` exactly. The column
stops paying a validity bitmap and the member stops being an `Option`.

**G6-R4.** Set it on every entry, not only the non-standard ones. `with_branch`
currently records a name only when the branch is not standard; a fixed-width
number has nothing to save by omission, and a column that is always present is
one a reader never branches on.

**This repeals a Decided rule.** `FixEntry`'s doc comment states the branch is
held as a name "because an entry is the arrival record: a branch held as an
integer would be the one part a reader cannot read — unprintable in a debug
line, unjoinable in a column, and resolvable only by someone holding the
registry that produced it." Delete that paragraph and write the new reason in
its place: the entry is fixed-width and the registry publishes the table that
resolves a `bid`, so the number is joinable and the capture is self-describing.
`G6-R5` is what makes that true — **without it this rename is the regression the
old rule predicted.**

**G6-R5.** `FixRegistry` publishes the branch id. Two things, both required:

- an accessor pair in the landed convention —
  `get_branch_by_bid(&self, bid: i64) -> Option<&FixBranch>` beside
  `branch_by_bid(&self, bid: i64) -> Result<&FixBranch>`. The reverse map
  already exists: branches are held keyed by their `u32` digest. Narrow, look
  up, and refuse in the register of the landed absences.
- the dialect manifest carries each branch's `bid` beside its name, written on
  every dump.

**G6-R6.** Writing the `bid` into the manifest stores a fact derivable from the
name, and `N4` permits that only where a rule says so and why. The why: the
derivation is a one-way XXH32 over the folded name, so a reader joining a
capture's `bid` column against the manifest would otherwise have to reimplement
that hash bit-for-bit in its own language. The manifest is a published join key,
not a cached computation. State this in the manifest writer's doc comment.

**G6-R7.** Pin the stored copy to the derivation: a test asserting that every
`bid` in a written manifest equals the digest of that branch's folded name, for
every branch in the committed dictionary. A stored derivable fact that is not
pinned is a stored derivable fact that will drift.

**G6-R8.** Do not hash the `bid` into the message digest. The digest walks
tag, key and value and has never hashed the branch; a rename does not change
what identifies a message.

**Verify G6.** A round-trip of a capture on a non-standard dialect: assert the
`bid` column is non-null on every row, that a standard-branch row is `0`, and
that `branch_by_bid` answers the branch the message resolved in. Assert the
manifest's `bid` equals the derived digest for all committed branches. Report:
the entries column's per-row byte delta (a `u32`-wide name string becomes a
fixed 8 bytes with no validity bitmap), and `size_of::<FixEntry>()` before and
after.

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
holds is level 2; the `fixentry` a level-2 entry holds is level 3; there is no
level-4 struct.** "Three levels deep" means three `fixentry` structs on any
root-to-leaf path — seven Arrow nodes:

```text
nofixentries : list<fixentry: struct<
    tag int32, bid int64, key utf8, value utf8,
    nofixentries: list<fixentry: struct<
        tag int32, bid int64, key utf8, value utf8,
        nofixentries: list<fixentry: struct<
            tag int32, bid int64, key utf8, value utf8,
            nofixentries: binary>>>>>>
```

Three is the materialized depth, not a natural limit. It is stated in exactly
one place — the function that types both arrival columns — so a fourth level is
one constant, not a shape rewrite.

**G5-R4.** The fifth member keeps the same name at **every** level, and so does
the item struct. The meaning is identical at each — the entries this entry
holds — and the *type*, not a second name, is what says the subtree stopped
being materialized. A reader that walks one level walks them all.

**G5-R5.** Keep the four existing members exactly as G6 leaves them — `tag`,
`key` and `value` nullable, `bid` non-null — and append the fifth. Declare all
three `fixentry` items non-null, the two inner lists non-null, and the binary
leaf non-null; the outer list stays nullable as today. An empty child list means no children and an empty leaf means nothing was
truncated, so no level pays a validity bitmap.

**G5-R6.** The leaf holds everything at level 4 and below as **the crate's own
JSON**, serialized by `into_json_scalar` over the truncated subtree rendered as
a `Scalar`, and read back by `from_json_scalar`. Write no emitter and no parser:
the FIX message type already states this contract — *serialization is inherited,
not written* — and a second renderer for the arrival record is exactly the
second path `N3` forbids.

**G5-R7.** Read the leaf back **untyped**, with `from_json_scalar`, and rebuild
the entries by a walk. The subtree below the materialized depth has unbounded
depth, so no `Field` describes it and `from_json_scalar_with_field` cannot be
used; every member of an entry is an `int32` or a `utf8`, so an untyped read
loses nothing.

*Rejected:* typing the read against the level-3 `fixentry` field. It types only
one more level and then refuses the message, which is `G5-R14` inverted.

**G5-R8.** Type the leaf `binary`, holding the UTF-8 bytes of that JSON. It is
opaque to the row's readers by construction: a `utf8` leaf would invite querying
the arrival record as text and give the truncated tail a second, half-typed read
path that the materialized levels do not have.

*Noted:* `fix:lineage` and `fix:codes` do store canonical JSON as text — but
they are metadata on a field, not a column of a row, and nothing queries them
as strings.

**G5-R9.** Fold level 4 and below into the leaf in exactly one place — the
function that writes the Arrow arrival value. The builder never folds and the
Rust tree is never truncated: `children` is the whole arrival record, and the
Arrow column is a materialization of it.

**G5-R10.** Attach a member's entry under the counter's entry only when a counter
pair actually arrived; otherwise record it flat at level 1 as today. **Never
synthesize a parent nobody sent** — the entries are the arrival record.

**G5-R11.** The digest walks the tree pre-order and writes each entry's child
count as four big-endian bytes after the value's length-prefixed bytes, then
each child. Write the count for **every** entry, including zero. Without it two
messages differing only below the materialized depth hash alike, which is a
correctness bug, not an optimization. Skip an envelope tag's whole subtree, not
just its entry. Recurse over the slice directly: no intermediate allocation.

**G5-R12.** The wire emitters go pre-order — the entry, then its children — and
apply the control-byte refusal at every level. The anomaly walk goes pre-order
too, so an anomaly below level 1 is still reported; the miscount comparison is
unchanged.

**G5-R13.** The unmapped list stays **flat**: every entry no dictionary
explained, found pre-order at any depth, enters it as a level-1 entry with an
empty child list and an empty leaf. It is a view over what was not explained,
not a second copy of the tree's shape.

**G5-R14.** Refuse nothing on depth alone. An entry thirty levels down folds
into the leaf. A depth refusal would make a legal nested-group message
unreadable, and the arrival record does not do that.

**G5-R15.** **Invent no refusal.** Both failure modes already belong to the JSON
pair: a value it cannot render refuses on the way in, and a leaf that is not a
JSON document refuses on the way out, each in its own landed register. Adding a
third is a second contract for one failure. The one rule to state is that a leaf
that does not read back is **refused, never skipped** — a re-emitted line must
be one that was actually sent.

Note what `G5-R6` buys: the frame byte needs no refusal at all, because JSON
escapes a control byte instead of colliding with it. That was the only refusal a
hand-rolled wire leaf would have had to invent.

**Verify G5.** A message nested four groups deep: assert three materialized
levels, a non-empty leaf, that the leaf parses back through `from_json_scalar`
to the entries that were folded, and that the re-emitted wire equals the input
byte for byte. Assert two messages differing only at level 4 — below the
materialized depth, inside the leaf — produce different digests. Report:
`size_of::<FixEntry>()` before and after, the per-row Arrow cost on a capture
whose entries never nest, and how many of the shipped 521 groups nest at all
(the corpus has 0 lists inside lists, so the third level is reached only by a
dialect or a live capture — say which fixture exercises it).

## Phase G7 — a lineage states a resolved datatype

`fix:lineage` records what a field was called and typed at each version. Its
`type` key holds **the FIX datatype name** — `int`, `NumInGroup`, `char`,
`String`, `XID` — while the field's own datatype in the same shard is already
the crate's serialized `DataType`. One document, two vocabularies, and the
lineage's half is the one nothing else uses.

The cost is not cosmetic. FIX renames a datatype far more often than it changes
one, and every rename is persisted as a version in a field's history:

| across all 1,564 lineages | |
| --- | --- |
| entries | 1,926 |
| type changes | 323 |
| **…that resolve to the same `DataType`** | **208 (64.4%)** |
| …that are genuine retypes | 115 |
| entries removable outright | 206 (10.7% of all entries) |

The top offenders: `char → String` ×62, `String → XID` ×34, `String → XIDREF`
×29, `int → NumInGroup` ×21, `float → Price` ×12, `UTCDateOnly → LocalMktDate`
×12. Not one is a retype. Every FIX datatype name in the shipped lineages
resolves through the crate's table, with **0 unknowns**, so the harmonization is
total rather than best-effort.

**G7-R1.** Persist a lineage entry's type as the datatype **resolved through the
crate's own `DataType`**, never as the FIX datatype name. One representation —
the one the field's own datatype already uses.

**G7-R2.** Two FIX spellings that resolve to one `DataType` are one value. An
entry that would record only a spelling change is **not written**.

**G7-R3.** The first entry establishes `since` and never collapses. A field
whose lineage would collapse to nothing keeps that first entry: "a field with no
lineage is defined at every version", so dropping a lineage entirely changes
meaning rather than compacting it.

**G7-R4.** An entry does not collapse when it differs from its predecessor in
`name`, `deprecated`, `removed`, `doc` or `ep`. Only the type is harmonized; a
per-version wording or a deprecation is history, not noise.

**G7-R5.** Accept that the FIX datatype name per version is no longer
recoverable — the mapping is many-to-one. It is a spelling of a type, not the
type; the crate's `DataType` is what every reader of a lineage actually wants,
and resolving the name to one is work the projection does on every version read
today. **Removing that lookup from the version-read path is the second reason
for this phase.**

**G7-R6.** The collapse is a **write-side** normalization, in one place. A
hand-authored dictionary and a `.cfb` dialect both build lineages without the
generator, so the rule cannot live in the generator alone; a read-side skip
would leave the document holding entries the contract calls impossible.

## Phase G8 — one string, one instant

Blocks on G7. Two families still duplicate after G7, and they are the two the
specification is loosest about.

**The temporal family carries two time widths and a time typed as text.**
`LocalMktTime` resolves to `time32(second)` while `UTCTimeOnly` and `Time`
resolve to `time64(nanosecond)`, and `TZTimeOnly` — a time of day — resolves to
`fixed_ascii(16)`, text rather than a temporal type at all.

**G8-R1.** Resolve every FIX time-of-day to `time64(nanosecond)`. That folds
`time32(second)` (47 fields) into the one time type and moves `TZTimeOnly`
(6 fields) out of `fixed_ascii(16)`. The cost is four bytes a value on those 47
fields; the gain is that a time is a time everywhere.

**G8-R2.** `TZTimeOnly` loses its zone offset, and that offset is why it was
text. State it as an accepted loss with the reason: Arrow has no
time-with-offset type, the alternative is a time nothing can compare or
aggregate, and the untranslated wire text survives in the arrival record
regardless.

**G8-R3.** Correct the two values that claim a zone they do not have.
`LocalMktDatetime` and `LocalMktTime` are *local market* values resolving to a
UTC-tagged type; a local value tagged UTC is wrong in a way that silently shifts
a timestamp.

**Then the string family — the sharper rule.** A field FIX later types as a
datetime was a datetime all along; the specification had simply not named it
yet. `String → LocalMktTime` occurs 45 times, `String → UTCTimestamp` 7,
`String → LocalMktDate` 3, `String → TZTimestamp` 1.

**G8-R4.** When a lineage passes from a string type to a temporal one, **the
earlier entry adopts the temporal type.** Back-typing, not forward-filling: the
later, more specific type propagates backward over the string entries preceding
it, which then collapse under `G7-R2`.

**G8-R5.** This is sound because a `DataType` is how *this crate* represents a
value, not what the specification called it. FIX has always sent the value as
ASCII on the wire; the datatype is the crate's parse target, and the same wire
spelling parses to the same instant at 4.2 as at 5.0. Back-typing states what
the crate would always have done, not a claim about the specification.

**G8-R6.** Back-type only from a string to a **temporal** type. Stop at every
other type: `utf8 → boolean` (9), `utf8 → int32` (6), `utf8 → currency` (3),
`utf8 → mic` (3), and `utf8 → country`/`msgtype`/`side`/`msgdirection` are
genuine refinements where the earlier values really were free text, and a
back-type would assert a constraint the older values do not meet.

**G8-R7.** Verify the back-type rather than assuming it: for every field it
touches, the earlier version's wire spelling must parse to the adopted type
through the crate's own reader. A back-type that cannot parse is a refusal, not
a silent retype.

**What G7 and G8 achieve together**, measured on the shipped dictionary:

| | today | after G7 | after G8 |
| --- | --- | --- | --- |
| adjacent typed pairs | 362 | 362 | 362 |
| collapsing | 208 | 208 | **270** |
| surviving retypes | 115 | 115 | **53** |
| removable entries | 206 | 206 | **268** (13.9%) |
| back-typed string entries | — | — | 62 |

The 62 back-types are `String` adopting `time64(nanosecond)` ×51,
`datetime64(nanosecond, UTC)` ×8 and `date32` ×3.

**G8-R8.** The endpoint is checkable, so check it. After G8 the corpus holds
**zero** temporal→temporal, string→string and string→temporal retypes, and all
53 survivors are genuine cross-family changes: `int32 → float64` ×13,
`utf8 → boolean` ×9, `int32 → utf8` ×8, `utf8 → int32` ×6, `int32 → int64` ×6,
`float64 → utf8` ×1, and the ten refinements to a validated code type. A test
asserts that census exactly.

**Verify G7 and G8.** Regenerate the dictionary and report the table above,
measured rather than restated. Assert the surviving-retype census exactly.
Assert no field lost its first entry, and none that had a lineage now has none.
Assert every back-typed field's earlier wire spelling parses to the adopted
type. Report the `config/fix` byte delta and the provenance checksums changed.

**Never, in G7 and G8.**

- **NG16.** Keep the FIX datatype name anywhere in a lineage "for reference".
- **NG17.** Collapse an entry differing in `name`, `deprecated`, `removed`,
  `doc` or `ep`.
- **NG18.** Drop a field's first entry, or leave with no lineage a field that
  had one.
- **NG19.** Back-type from a string to anything but a temporal type.
- **NG20.** Back-type without proving the earlier wire spelling parses.
- **NG21.** Collapse in the generator only. A dialect builds lineages too.

## Order

`G1 → G3`; `G2 → G3`; `G3 → G4 → G6 → G5`; `G7 → G8`. G1, G2 and G7 block on
nobody. G6 precedes G5 so the entry's member list settles before recursion
appends to it, and the entry's Arrow shape and fixtures are rewritten once per
phase rather than the same members twice. G7 and G8 touch the lineage and the
datatype table alone, so they may land alongside the component work in either
order — but both regenerate the committed dictionary, so land them adjacent to
each other and not interleaved with G3.

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
- **NG11.** Refuse a message for nesting deeper than the materialized depth.
- **NG12.** Write a second serializer, parser or refusal for the leaf. The
  crate's JSON pair renders and reads it; serialization is inherited, not
  written.
- **NG13.** Keep the branch's name on the entry beside its `bid`, or make the
  `bid` nullable. One spelling, always present, `0` for the standard branch.
- **NG14.** Rename the entry's branch member without publishing the resolution
  table in the same phase. The number alone is the regression the repealed rule
  predicted.
- **NG15.** Hash the `bid` into the message digest.

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
| leaf JSON round-trip | folded entries recovered, byte-for-byte re-emission |
| entries column per-row delta | name string → fixed 8 bytes, no validity bitmap |
| manifest `bid` vs derived digest | equal for every committed branch |
| lineage entries removed | 268 of 1,926 (13.9%) |
| surviving retypes | 115 → 53, census asserted exactly |
| string entries back-typed | 62 |
