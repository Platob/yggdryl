# P13: design (D46) - a CBlock's NumInGroup counters dropped silently

The user's instruction (`$S/p13/user_instruction.md:1`, 2026-10-10): "ensure also in the cfb files fix
ingestion parsing it ignores silently the components having the numingroups fields since our
representations are structures and correct group list typed". Decision 30 (`$S/user_decisions.md:185-196`),
verbatim: "P13, the CBlock counters (its own small commit, first of the next lanes): [...] Read as: a
CBlock grammar - a message root, a group's entry, a folded second binding of one wire type - that
states a NumInGroup counter as a plain member beside the group it counts (or a counter the reader
knows counts a group the same structure holds) has that counter member dropped by the reader,
silently - no warning, no `FixMerge::dropped` entry - and the message, the group and every other
member kept, since a group is its typed list and its length the count; today that message is dropped
whole with a warning (`catalog.rs` validate_references, pinned by `rust/fix/tests/root/cfb.rs`
`a_message_the_catalog_will_not_hold_is_named_at_its_grammar_binding`). The catalog's own refusal
stays for a dictionary built by hand; the reader never produces one." `$S` is
`.handoff/split/scratch`. Its own small commit, the first of the next lanes.

Read on the working tree at `2902c86fe` (2026-10-10); every line number is of that tree. No command
was run to write this - another workflow holds the cargo lock - so every "today" claim is read, not
run, and the claims that need a run are marked as such. Revised the same day after review: the
ingestion fold the user named (`yggdryl fix ingest`, `add_cfb_files`) is in scope, the read-time
pre-pass is per binding, and the by-hand pin runs in the default lane.

## What the tree holds today

| Fact | Where |
| --- | --- |
| The reader's contract: "A nested grammar resolves its opening counter to int32 in the vocabulary and becomes a separately named Serie of components that states the counter as its `FIX:counter`: the count is the serie's length, so no message lists the counter beside it" | `rust/fix/src/cfb.rs:31-36`; the `from_cfb_file` rustdoc `:432-437` |
| `read_grammar`, the one recursive function over a root grammar and every nested one: every `tag-constraint` is pushed through `push_member` as it is, a nested `grammar` becomes a group through `grouped` and is pushed beside its siblings, an empty one is dropped with the counterless warning. Nothing compares a plain member to a sibling group's counter | `cfb.rs:2272-2326` (`push_member` at `:2296-2304`, the nested group `:2305-2311`); `push_member` renames a duplicate name `{name}2`, `rust/fix/src/catalog.rs:3287-3304` |
| `grouped`: the nested grammar's first child is the counter (`children.remove(0)`, `:2344`), refused where it is itself a group (`:2346-2362`); its tag is read (`:2378`) and the vocabulary entry retyped `Int32` (`:2384-2391`); the item struct is built from the remaining children and `FIX:counter` set on the group - the one place the reader sets it (`:2427-2439`, `set_counter` at `:2436`). The group's nullability is its own opening counter's, `group.set_nullable(counter.is_nullable())` (`:2433`). It never looks at the parent's other children nor at the entry's own | `cfb.rs:2334-2446` |
| `read_binding`: the root's children come from `read_grammar(1, ..)` and become the root struct directly; a `StructType` refusal drops the message | `cfb.rs:2235-2248` |
| A constraint clones the vocabulary entry at its current type and overrides only nullability - `required` read as a flag, an expression read as not required, `field.set_nullable(!required)` (`:2534-2540`) - so a counter stated before its nested grammar is a clone at the pre-retype type and one stated after is already `Int32` | `cfb.rs:2507-2515,2534-2545`; a tag the vocabulary never declared is declared utf8 and entered in `positions`, so `grouped` can still retype it, `:2554-2575` |
| `catalog_members`' scalar arm accepts a member whose type differs from the dictionary's where the dictionary's is `Int32` ("A nested grammar retypes its counter int32 once it is read") and rewrites it to the known field. So a bare counter of either order survives into the catalogued struct | `cfb.rs:3301-3326` (comment `:3307-3309`) |
| The refusal comes from the catalog, one struct level at a time: `catalog_entry` -> `insert_definition` -> `validate_definition` -> `validate_references`, which collects the counters of a struct's children (a `FIX:group` child's definition's counter, an inline group's own `FIX:counter`) and refuses any non-nested child whose tag is among them, `Error::InvalidRecord` at `{struct}.{child}`, "expected no NumInGroup counter beside the group it counts, whose length is its count, got {name} ({tag})" | `cfb.rs:3033`; `catalog.rs:1215,1273,2421-2451` |
| That error drops the **whole message**: `catalogued` takes back the definitions the walk wrote and `dictionary` warns `message "D": <error>; the message is dropped and every field it declared kept`, located at the binding | `cfb.rs:946-963,858-877`; `dropped` is `log::warn!`, `:2759-2761` |
| A second grammar-binding of one wire type folds through `fold_definition_into(Components, root, Some(&mut drops))`, which ends in `resolve_catalog` + `validate_catalog`; its `?` throws the staged registry away, so a bare counter in either binding beside the other's group drops the whole second binding with the message-dropped warning | `cfb.rs:986-1030` (fold at `:1006`); `catalog.rs:1473-1501` (`validate_catalog` `:1500`) |
| **The ingestion path the user named.** `yggdryl fix ingest` is `store.registry_mut().add_cfb_files(&locations, dialect)`; `add_cfb_file` is `from_cfb_file` then `merge_with`, and so is `add_json_file`. `add_cfb_files` parses every file side by side, folds each into one staged set of documents and settles once; where that `settle` refuses the union it warns `the N cblock files folded do not resolve together (<error>); folding them again one at a time`, refolds each file settled before the next, and names the file the union cannot hold in `FixMerge::failed` | `cli/src/fix.rs:471`; `rust/fix/src/registry.rs:2126-2130,2194-2232,2242-2300`; `rust/fix/src/store.rs:568-572` |
| **Every fold with another dictionary reaches `merge_children_at` with `drops.is_some()`**: `merge_with` -> `fold_registry` -> `fold_source` -> `merge_catalog` -> `fold_document(.., &mut Some(&mut *dropped))`, and the reader's second binding through `fold_definition_into(.., Some(&mut drops))`. That is the gate the two counter arms already read ("Only a fold with another dictionary reads it so, as `beside` does: a definition written alone is refused as before"). The hand-built doors pass `None`: `insert`, `insert_definition`, `create_definition`, `restate_definition`, `fold_definition` (`fold_definition_into(.., None)`), and the store loader, which is `create_definition` | `registry.rs:1830-1856,2048`; `catalog.rs:1459-1460,1473-1501,1514-1520,1678-1775,2554,2717`; `store.rs:684,937` |
| **How the union is refused today.** In `merge_children_at` an incoming non-nested member no stored member answers to by name takes the `None` arm: `group_counter(documents, child)` is none, so `merged.push(child.clone())` appends it beside the held group; the held committed D holds `parties`, the group on 453, so a vendor file listing a plain `<tag-constraint name="453"/>` in D folds beside it, and `settle` -> `resolve_catalog` + `validate_catalog` -> `validate_references` refuses: `merge_with`, `add_cfb_file` and `add_json_file` refuse whole, `add_cfb_files` warns and leaves the whole file in `failed`. A file holding the group folded after a file holding the bare counter is the same refusal (the group is pushed beside the held member) | `catalog.rs:1745-1771,2424-2450,2510-2514`; `config/fix/components/newordersingle.json:72-78`; `registry.rs:2206-2217` |
| A member of a document is a compacted reference: a `Null` placeholder keeping its name, its nullability and the `FIX:field`/`FIX:group` name it reads (`compact`, `reference`); a scalar member's tag is its referenced field's, read as `referenced_dtype` reads the type - `self.definition(Fields, name)` - and `group_counter(documents, member)` answers a group reference's counter | `store.rs:199,381-386`; `catalog.rs:2043-2053,2876-2891` |
| The CBlock grammar has no reference mechanism: every group is inline and carries `FIX:counter`; once catalogued, a member is a `FIX:group` reference and a structurally alike definition is reused (`reuse`, debug-logged) | `cfb.rs:38-49,3043-3049,3170-3190,3282-3296` |
| The one pin: message D states `<tag-constraint name="555" />` then `<grammar><tag-constraint name="555" /><tag-constraint name="556" /></grammar>`; one warning, located `line 9, column 28: message "D"`, carrying the catalog's sentence and `; the message is dropped and every field it declared kept`; `roots.len() == 1` ("message 8 still binds"); tags 55, 555, 556 kept | `rust/fix/tests/root/cfb.rs:1046-1091` |
| The sentence "no NumInGroup counter beside the group it counts" is asserted nowhere else: once that pin moves, the catalog's refusal has no test. The by-hand helpers that would carry one exist - `dictionary()`, `reads()`, `component()`, `group()` over `insert_definition` - but `rust/fix/tests/root.rs:470-472` declares `root/catalog.rs` under `#[cfg(feature = "internals")]`, and that file imports `yggdryl_fix::internals::catalog::structural_key` at its top (`:13`), so without `--features internals` the whole module compiles out and a `--test root <filter>` matches 0 tests and reports ok | grep over `rust/fix/tests`, `python/tests`, `node/tests`, `cli/tests`: only `cfb.rs:1079`; `rust/fix/tests/root/catalog.rs:1-64,93-156`; `rust/fix/tests/root.rs:470-472` |
| The reader's own logging ladder: a drop is warn (`:2759`), a second binding's fold info (`:1000`), a structural reuse or relaxation debug (`:3176-3184`); "Warn stays reserved for what is actually lost" | `cfb.rs`; `docs/fix/registry.md:1385` |
| The test logger records warn and above only, and `warned::during` answers those records | `rust/fix/tests/root.rs:87,134-141` |
| The nested-grammar pin that stands: the group `legs` holds `FIX:counter` 555, no `nolegs` member at the root, tag 555 an `Int32` field of the dictionary | `rust/fix/tests/root/cfb.rs:507-556` |
| The fold pin that stands: three bindings, `8`, `S Inbound`, `S Outbound`, groups on one counter, `warnings.is_empty()`, `roots.len() == 3` | `rust/fix/tests/root/cfb.rs:6036-6085` |
| Every `add_cfb_files` pin stands in `rust/fix/tests/root/cfb.rs` (the one test file naming it); `rust/fix/tests/root/registry.rs` names it nowhere | grep over `rust/fix/tests`, `cli/tests` |
| The committed store never holds a counter beside its group - the generator lists only `fieldRef`, `componentRef` and `groupRef`; 1219 counter members went when a group became its list alone - so the dictionary hash `857_326_152_662_128_339` and its census are a pin a reader change must not move | `rust/fix/tests/root/store.rs:3456-3463,3557`; `scripts/generate_fix_dictionary.py:385-391,429-450` |
| The row-schema refusal `refuse_counters_beside_groups` is `column_plan_reading`'s, for a caller's schema and a `FixMsg` root, never on the CBlock path; it stays | `rust/fix/src/schema.rs:742-746,867-906`; `docs/fix/message.md:13` |
| The bindings and the CLI forward to the core doors with no logic of their own, and no binding fixture holds a nested grammar | `python/src/fix.rs:463-469,577-584,617-628`; `node/src/fix.rs:490-498`; `cli/src/fix.rs:471`; `cli/tests/fix.rs:738-741`; `python/tests/test_fix.py:1875-1879,2595-2598`; `node/tests/fix.test.js` (no binding) |
| The docs sentences stating today's behaviour: "A message whose grammar states a NumInGroup counter beside the nested grammar that counter opens is the vendor's contradiction, not a fold's: the message is dropped with every field it declared kept [...]"; the `add_cfb_files` paragraph ("where the union does not resolve, the files fold again one at a time [...] the file the union cannot hold is the one left out by name"); the Counters row ("one listing the counter beside the group is refused") | `docs/fix/registry.md:27,42,1373,1383`; `docs/fix/index.md:480-485`; `python/yggdryl/fix.py:232-238`; `skills/yggdryl-fix/SKILL.md:215-220,328-329` |

## The decision

1. **What is dropped.** A plain (non-nested) member of a structure whose tag is the `FIX:counter`
   of a group the same structure holds - the message root, or the entry a group repeats - at every
   depth, stated before or after the group, stated once or several times (`push_member` renames
   the second `nolegs2`, `catalog.rs:3287-3304`, so the strip reads the **tag**, `FixField::tag()`,
   never the name), and whether the group and the counter come from one binding, from two bindings
   of one wire type (`wire_value(root.name())`, `msgtype.rs:80-82`: `S Inbound` and `S Outbound` are
   `S`), or from two dictionaries folded - a CBlock file into the held store, two CBlock files, a
   JSON dictionary. A definition reached by name is the same case once more: a CBlock has no
   reference (`cfb.rs:38-49`); the inline group carries its counter (`:2436`), and a structural reuse
   happens in `catalog_members` (`:3170-3190`), after the read-time strip, so the member that then
   reads the held definition by `FIX:group` has no counter beside it either - `validate_references`'
   reference arm (`catalog.rs:2431-2434`) stays as the hand-built guard. The user's words give the
   reason, and the representation states it: "our representations are structures and correct group
   list typed" - the group is its typed list and its length is its count, so the member says what the
   list already says, and one fact has one owner.
2. **What is kept, and why.** A bare counter with **no group in its structure** - its group in
   another message (path f: message `8` states `555` and only `D` opens a grammar with it), or in no
   message at all (path g) - is **kept as a member**: as an `int32` member where any grammar in the
   file opened a group with it (the vocabulary entry was retyped, `cfb.rs:2384-2391`, and
   `catalog_members`' `Int32` rule resolves the clone, `:3307-3325`), else at its vocabulary type. The
   reason is the same rule read the other way: where no typed list is in the structure, nothing else
   owns the count the wire carries, so dropping the member would lose a fact rather than a duplicate;
   and it is what the catalog accepts today, one level at a time (`catalog.rs:2427-2450`). The user's
   "silently" covers the duplicate, not a fact stated once. The member keeps standing until a group
   on its counter reaches its structure - from a later binding or a later file - and the fold then
   takes it out (item 4b), because the duplicate is decided where the union is known.
3. **A group's own counter restated inside its entry** (`<grammar><tag 555/><tag 555/><tag 556/></grammar>`)
   is **kept**, as today: an `Int32` member of the entry (no group at that level counts 555, so
   `validate_references` passes it; `catalog_members` resolves it). Decision 30 drops a counter "the
   reader knows counts a group the same structure holds", and the entry does not hold the group that
   repeats it; on the wire a 555 inside a leg occurrence is not provably the outer count. Dropping it
   too - the entry being the structure the group repeats, the reader knowing the tag it just took
   (`:2378`) - is the alternative, put to the user (#2), taken only on a yes recorded as a decision
   before the commit.
4. **Where: two sites, each the one its case passes through.** Both read the tag, never the name,
   and both set nothing but the struct they rebuild.
   - **(a) Read time, per binding.** A pre-pass in `Parse::dictionary` over `self.roots` - between
     the vocabulary loop's end and `let held = std::mem::take(&mut self.roots)` (`cfb.rs:858-861`),
     before the `catalogued` loop and so before any `insert_definition` and its `validate_definition`
     (`catalog.rs:1273`) - as one `Parse` method, `drop_counters_beside_groups(&mut self)`. It walks
     each root on its own: at every struct - the root, and the entry of every nested group at any
     depth - it collects the `FixField::counter()` of the struct's nested members (an unreadable one
     read as none, as `group_counter` reads it, `catalog.rs:2884-2891`) and rebuilds the struct
     without the plain members whose tag is among them - `StructType::from_fields(kept)` set back
     through `set_dtype` on the struct, or through `DataType::serie(item)` on the group, as
     `catalog_members` rebuilds them (`:3256,3281`). One walk per root, nothing per row; a struct
     holding no group is passed over untouched, and nothing is read across bindings: a binding's
     plain counter whose group another binding of the wire type holds is left standing here and
     taken out by (b) when the two are folded, since only the fold knows whether that group survives.
     Nothing else moves: `self.vocabulary` keeps the counter as an `Int32` field of the dictionary
     (pinned, `tests/root/cfb.rs:507-517`), `grouped` keeps retyping it, `catalog_members`' `Int32`
     rule stays (path f needs it), `written`/`structures` are untouched.
   - **(b) Fold time, against the folded structure.** One final pass at the end of
     `merge_children_at` (`catalog.rs:1678-1775`, before `Ok(merged)`), gated by `drops.is_some()` -
     the gate the two counter arms already read - over `merged`: collect the `group_counter(documents,
     member)` of every member of `merged`, then leave out every non-nested member - whichever side
     it came from, the stored one or the incoming one - whose tag is among them, the tag read as the
     member's referenced field's (`self.definition(Fields, name)`, the way `referenced_dtype` reads
     its type, `catalog.rs:2043-2048`; a member reading no field, or a field the dictionary does not
     hold, is left as it is and the catalog's own validation says the rest). The pass runs after
     every incoming member of the structure has been folded, so it decides against what the
     structure actually holds: a group the fold passed over as a named drop (`reconcile`) is not in
     `merged`, and the counter stays; a binding whose whole fold refuses (`?` at `cfb.rs:1006`) or a
     file whose fold refuses (`fold_cfb_files`, rolled back alone) leaves the staged dictionary as it
     was, the counter in it. It runs where the union is known, so it covers in one place: the second
     binding of one wire type in both orders (the first binding's bare counter already registered
     when the second brings the group, and the reverse - no `restate_definition`), `add_cfb_file`
     into a held dictionary, `add_cfb_files` over two files in both URL orders, `merge_with` and
     `add_json_file`. With it, `settle` -> `validate_catalog` never sees the duplicate on a fold,
     `fold_cfb_files` never warns "do not resolve together" for it, and no file lands in
     `FixMerge::failed` for it.
   - Why not one pre-pass across bindings: it would decide before the fold has run, and the fold can
     still lose the group it stripped the counter for - then the message holds neither, and the
     count, stated once, is gone with no record. The site that sees the result is the fold.
5. **Silent.** Neither site calls `self.dropped` (warn), `FixMerge::dropped` or any log record:
   the literal reading of "silently", and decision 30's "no warning, no `FixMerge::dropped` entry".
   The reader's ladder agrees: warn is "reserved for what is actually lost" (`docs/fix/registry.md:1385`)
   and nothing is lost here. The alternative - one `log::debug!` per left-out member, the level the
   reader uses for a reuse or a relaxation that loses nothing (`cfb.rs:3176-3184`), free when no
   logger listens - reaches Python's `logging` and the terminal handler whenever a caller enables
   `DEBUG`, so it is not literally silent; it is put to the user (#3) and added only on a yes.
   Either choice keeps every pin, since the test logger records warn and above (`tests/root.rs:87`).
   No info record either: info is the fold of a second binding (`:1000`), which still happens and is
   still logged there.
6. **The left-out member's `required` carries onto the group.** A constraint's `required` is the
   one attribute `read_constraint` keeps (`cfb.rs:2534-2540`), and the group's nullability is its
   nested grammar's own opening counter's (`:2433`); a compacted member keeps its nullability too
   (`compact`, `store.rs:381-386`). So `<tag-constraint name="555" required="Y"/>` beside a grammar
   whose opening 555 is optional states that the group is required, and leaving the member out
   would lose that statement silently. Rule, at both sites: **the group is required where either
   statement requires it** - the nested grammar's counter or the plain member left out - and
   nullable only where both leave it optional; the group's `FIX:counter`, display and component
   are untouched. Put to the user (#4); the alternative is the nested grammar's own presence alone.
7. **The catalog's refusal stays** (`catalog.rs:2424-2450`) for a dictionary built by hand -
   `insert`, `insert_definition`, `create_definition`, `restate_definition`, `fold_definition`, the
   store loader (`create_definition`, `store.rs:684,937`) - every door passing `None` for `drops`,
   and gets the pin it has never had, in the default lane (tests, below). What the gate does not
   keep: `merge_with` of two hand-built dictionaries, one holding the bare counter and the other the
   group, folds under `drops.is_some()` and leaves the member out silently rather than refusing - the
   same gate under which it already folds a group by its counter (`catalog.rs:1745-1771`), and the
   door `add_cfb_file` and `add_json_file` are. Put to the user (#5): the alternative is a flag only
   the CBlock and JSON file doors set, which `fold_registry` would have to carry for them. The
   `FixMsg`/row-schema refusal (`schema.rs:867-906`) is untouched.
8. **Not in P13.** Nothing of the user's named surface. The pre-pass over an entry's own restated
   counter (#2) and the debug record (#3) wait on the user's answers.

## The module doc, the docs and the skills

- `cfb.rs:31-36` ("so no message lists the counter beside it") gains: "and a `tag-constraint` that
  restates that counter beside the grammar is left out of the message silently: the member states
  what the serie's length states, and its `required` is carried onto the group."
- `cfb.rs:71-84`, "What is lost, by name", gains one item at its head: "a `tag-constraint` on a
  nested grammar's counter stated beside that grammar, at any depth (the group is its list and its
  length the count, so the member repeats the list; the tag stays an `int32` field of the dictionary,
  and the group is required where the constraint or the grammar's own counter requires it)".
- `cfb.rs:304-331` ("What is dropped, and what is refused"): "a message the catalog will not hold
  keeps every field the file declared" stays only where the branch stays reachable (test (h)); one
  clause says the counter beside its group is not among the drops but a silent omission.
- `cfb.rs:453-470` (`from_cfb_file` rustdoc, `# Errors`): one clause after "a nested grammar with no
  counter": "a constraint restating a nested grammar's counter beside it is left out of the message
  with no warning".
- `catalog.rs` `merge_children_at`'s comment, beside the two counter arms: "and a plain member on a
  counter a member of the merged structure counts a group by is left out, whichever side stated it:
  the group is its list and its length the count. Only a fold with another dictionary reads it so; a
  definition written alone is refused as before." `registry.rs` `add_cfb_files`' rustdoc (`:2184-2188`,
  "The resolution is the one place two files' contributions can refuse each other") gains "a counter
  one file lists beside a group another holds is not such a refusal: the member is left out"; the
  `merge_with` rustdoc (`:1826-1829`) the same clause.
- `docs/fix/registry.md:1383` rewritten: "A message whose grammar states a NumInGroup counter beside
  the nested grammar that counter opens - before it or after it, at the root or inside an entry, in
  one binding or across two bindings of one wire type - is read **without that member**, silently: the
  group is its list and its length the count, so the message, the group and every other member are
  kept, the tag stays an `int32` field of the dictionary, the group is required where either
  statement requires it, and nothing is warned or counted in `dropped`. A counter whose group is in
  another message, or in none, is an ordinary member - until a fold brings a group on its counter
  into that structure, which takes it out the same way." `:1373` (the `add_cfb_files` paragraph)
  gains, after "the file the union cannot hold is the one left out by name": "a counter one file
  lists beside a group another file, or the held dictionary, holds is not such a case: the member is
  left out and the file folds"; the `merge_with` section gains the same sentence. `:27` (Counters
  row) gains "a CBlock grammar or a fold listing one is read without it; a definition written alone
  that lists one is refused"; `:1375`'s list gains the counter as what is left out rather than
  dropped; `:1381`'s consequence list keeps the message-dropped consequence only if test (h) proves
  the branch reachable, and loses it otherwise.
- `docs/fix/index.md:480-485` and `python/yggdryl/fix.py:232-238`: one clause each, "a CBlock
  grammar or a fold that lists one beside its group is read without it".
- `skills/yggdryl-fix/SKILL.md:215-220`: one sentence after "What a CBlock states that the reader
  cannot keep is dropped [...]": "a counter a grammar states beside the group it counts is left out
  silently, the group's length being its count, in the file and in every fold"; `:328-329` ("a
  registry definition that does is refused") gains "- by hand; a CBlock grammar or a fold that does
  is read without the counter". The three `references/*.md` recipes (`rust.md:749-750`,
  `python.md:662`, `javascript.md:635`) state the rule for a hand-built registry and need no edit.
- `docs/fix/cli.md:160-174` (`ingest`): one clause only where the page states the failed-file
  consequence; read at phase 2. `docs/fix/store.md`: no change (the loader keeps refusing).
- `.api-inventory.txt:655` (`from_cfb_file`'s row): no row added or retired; its sentence gains the
  same clause beside "a nested grammar with no counter"; the `add_cfb_files` and `merge_with` rows
  the fold clause. `.api-bindings.txt`: nothing.
- `mkdocs.yml`: nothing; no example block changes (every sentence above is prose), so
  `check_docs_examples.py` has nothing new to run.

## The tests, red first

The first red step, before anything else, is test (h) as it stands re-cased on the empty type:
`cargo test --locked -p yggdryl-fix --test root a_message_the_catalog_will_not_hold` with the fixture
below. If it reaches the `Err` arm of `dictionary` (`cfb.rs:866-877`), the arm and the docs sentence
stay and (h) pins them; if it does not - `catalogue` handling `type=""` another way, through
`message_name`, the UNKNOWN reading or `stamp` - the arm is dead code once the counter is stripped,
and goes in the same commit with its docs sentence (`registry.md:1381`'s consequence, `cfb.rs:304-331`'s
clause), a `catalogued` error then propagated as the refusal of `from_cfb_file`. The sentence that no
other `validate_*` refusal is reachable from the reader is asserted by reading alone and not proven;
(h) does not rest on it.

In `rust/fix/tests/root/cfb.rs` unless named otherwise, each over `warned::during` asserting
`warnings.is_empty()` and reading the roots with `children()` (`:370`), the group's `FixField::counter()`
and `registry.field_by_tag(555)` as `:507-556` does.

| Test | Fixture | Pins |
| --- | --- | --- |
| (a) `a_counter_stated_before_the_group_it_counts_is_left_out_silently` - the re-cased `:1046-1091` | the same body (message `8` with `55`; `D` with `55`, plain `555`, then the nested grammar opening `555` with `556`) | no warning; `roots.len() == 2`; `children(D) == ["symbol", "legs"]`; `legs` carries `FIX:counter` 555, its item `leg` holds `["legcurrency"]`; `field_by_tag(555)` is `Int32`; tags 55, 555, 556 kept; `registry.msgtype("D")` (the catalogued form) lists no member on tag 555 |
| (b) `..._after_the_group_...` | the plain `555` after the nested grammar, and once more stated twice (`555` before and after) | the same; both statements gone, no `nolegs2` |
| (c) `a_counter_inside_an_entry_beside_the_nested_group_it_counts_is_left_out` | the CBLOCK shape (`:40-95`): inside the grammar on `555`, a plain `604` beside the nested grammar opening `604` | the item `leg` holds `["legcurrency", "legsecurityaltidgrp"]` as `:507-556` pins; `field_by_tag(604)` `Int32` |
| (d) `a_counter_one_binding_states_beside_the_other_bindings_group_is_left_out` | the fold fixture (`:6036-6054`): `S Inbound` with the group on `555`, `S Outbound` with a plain `555` and no group; then the reverse order of the two bindings | no warning (the fold is info); `roots.len() == 3`; `registry.msgtype("S")` reads `legs` with the folded members and no member on tag 555 - in both orders, through site (b) |
| (d2) `a_counter_stays_where_the_fold_passes_the_group_over` | two bindings of `S`: the first with a plain `555`, the second bringing a group on 555 the fold passes over as a named drop (`reconcile`: a held member of the group's name reading another field, the fixture found at the `reconcile` arm; where no fixture reaches it, the binding whose fold refuses whole at `cfb.rs:1006`) | `registry.msgtype("S")` keeps the `int32` member on 555; the drop or the message-dropped warning is the one record |
| (e) `a_counter_beside_a_group_read_by_name_is_left_out` | two messages binding one group alike (the reuse case), the second also stating the bare counter | the second message's catalogued root holds the `FIX:group` reference and no member on tag 555; one definition, debug-logged reuse untouched |
| (f) `a_counter_whose_group_another_message_opens_is_an_ordinary_member` | `8` states plain `555` and no group; `D` opens a grammar with `555` | no warning; `8`'s member `nolegs` is `Int32`, `FIX:field` the dictionary's; `D` as in (a) |
| (f2) `a_required_counter_beside_an_optional_group_makes_the_group_required` | `D` with `<tag-constraint name="555" required="Y"/>` then a grammar whose opening 555 states no `required`; and the mirror, an optional plain 555 beside a grammar whose opening 555 is required | `legs` is non-nullable in both; the item untouched |
| (g) `a_counter_restated_inside_its_own_entry_is_kept` | `<grammar><tag 555/><tag 555/><tag 556/></grammar>` | the item holds `["nolegs", "legcurrency"]`, `nolegs` an `Int32` member; no warning - today's behaviour, pinned; under a yes on #2 the pin flips to `["legcurrency"]` |
| (h) `a_message_the_catalog_will_not_hold_is_named_at_its_grammar_binding` re-cased on `<grammar-binding type="">` | `8` as today, then a binding whose `type` is empty, holding `55` | one warning, located by `assert_located` (`:1096-1115`), naming `message ""`, containing `FIX:msgtype` and `; the message is dropped and every field it declared kept`; `roots.len() == 1`; tag 55 kept. **Unrun, and run first:** by reading, `read_binding` keeps the empty `type` as the root name (`:2216-2224`), `wire_value("")` is `""` (`msgtype.rs:80-82`), `set_msgtype` refuses it through `validate_code` (`:96-103`, `rust/fix/src/field.rs:753-756`) inside `catalogue` (`cfb.rs:985`), and the `Err` arm of `dictionary` (`:866-877`) drops it - a whitespace-only `type` is *not* refused (`split_whitespace().next()` is `None`); the exact `Display` of `InvalidMetadataValue` was not read, so the pin asserts the key and the consequence, and the first run says the rest, or says the arm is dead (above) |
| (i) `rust/fix/tests/root/catalog.rs`, top level: `a_counter_beside_the_group_it_counts_is_refused_by_hand` | `dictionary()`, a `party` component, `parties = group(.., 453, "party")` inserted, then a component `order` of `[reads("nopartyids"), parties occurrence]` through `insert_definition(Components, ..)` | `Err(InvalidRecord)` at `order.nopartyids`, the sentence `expected no NumInGroup counter beside the group it counts, whose length is its count, got nopartyids (453)`; the registry unchanged after the refusal - decision 30's "stays for a dictionary built by hand", pinned for the first time, in the default lane |
| (j) `rust/fix/tests/root/catalog.rs`, top level: `a_fold_leaves_out_a_counter_beside_the_group_the_other_dictionary_holds` | two hand-built dictionaries over `dictionary()`: one whose `order` holds `parties`, the other whose `order` holds `reads("nopartyids")` alone; `merge_with` each way | `Ok`, `merge.dropped` empty, `order` reads `parties` and no member on 453, the group required where either side's statement is; and the mirror, the held `order` with the member and the incoming with the group |
| (k) `add_cfb_file` into a held dictionary, beside the existing `add_cfb_files` pins in `root/cfb.rs`: `a_cfb_file_listing_a_counter_beside_the_held_group_folds_without_it` | a dictionary holding D with `parties` (the committed store through `FixRegistry::seeded()`/the test's loader, or `dictionary()`'s shape with a D root); a CBlock whose D states a plain `<tag-constraint name="453"/>` and opens no grammar | `Ok`, no warning, `failed` empty, `dropped` empty, D reads `parties` with no member on 453 |
| (l) `add_cfb_files` over two files, `root/cfb.rs`: `two_cfb_files_one_listing_the_counter_the_other_opening_the_group_fold_as_one` | file `a.cfb` with D opening the parties grammar on 453, file `b.cfb` with D listing a plain 453; then the files named so the bare-counter file sorts first (`a.cfb` the plain one, `b.cfb` the group) | `Ok`, no warning ("do not resolve together" never logged), `merge.sources == 2`, `failed` empty, D reads `parties` with no member on 453 - in both URL orders |
| (m) `cli/tests/fix.rs`, beside `:728-742`: `ingest_folds_a_counter_beside_the_stores_group_without_it` | `yggdryl fix ingest` of the plain-453 CBlock of (k) into a store holding the committed D | exit 0, no `failed` line, the store's D unchanged in its members |

`root/catalog.rs` is restructured for (i) and (j), as AGENTS asks for a file that pins both kinds:
the helpers (`dictionary()`, `reads()`, `component()`, `group()`) and the two tests at the file's top
level, ungated, and the `structural_key` tests with their `yggdryl_fix::internals::catalog::structural_key`
import in their own `#[cfg(feature = "internals")] mod internal`; `rust/fix/tests/root.rs:470-472`
no longer gates the module. Without this the smoke for (i) compiles the file out and reports 0 tests ok.

Pins that must not move: `the_committed_dictionary_hashes_to_one_pinned_value` (`store.rs:3557`,
`857_326_152_662_128_339`) and its census, `the_committed_store_carries_the_crate_dump`
(`store.rs:58`) - the committed store is the generator's and holds no such member; `a_nested_grammar_is_its_group_alone_...`
(`cfb.rs:507-556`); `a_second_binding_of_one_wire_type_folds_...` (`:6036-6085`);
`the_structural_exceptions_are_a_statement_or_a_named_drop` (`:986-1032`, the counterless-group
and dangling-tag warnings are other drops); `nesting_past_the_guard_...` (`:1498-1546`, forty grammars
each opening with 555 and no plain 555); every existing `add_cfb_files` and `merge_with` pin in
`root/cfb.rs` and `root/registry.rs` (none states a counter beside a group); the benchmark
`rust/fix/benchmarks/fix/cblock.rs` (one group on 5000, no plain 5000); the CLI, Python and Node
CBlock fixtures (no nested grammar); the equivalence snapshot and `FIX_PIPELINE_COSTS` (the reader
is not in them); `rust/fix/tests/iobase_calls.rs` and `allocations.rs` - the fold pass reads
documents already in hand and the dictionary's own fields, no handle.

## Bindings and inventories

None: Python, Node and the CLI redirect to `from_cfb_file`/`add_cfb_file(s)`/`merge_with`
(`python/src/fix.rs:463-469,577-584,617-628`; `node/src/fix.rs:490-498`; `cli/src/fix.rs:471`), no
signature moves, no door is added. No inventory row is added or retired (`.api-inventory.txt:655,1210-1212`
keep their names; `.api-bindings.txt:181-183` unchanged); `python scripts/check_api_inventory.py` is
run because the rows' prose is edited.

## Implementation plan

One small commit; phases in AGENTS order, each settled narrowly.

| Phase | Files | Smoke (exact) |
| --- | --- | --- |
| 0 the reachability run | `rust/fix/tests/root/cfb.rs` (test (h) re-cased) | `cargo test --locked -p yggdryl-fix --test root a_message_the_catalog_will_not_hold` - red or green says whether the `Err` arm of `dictionary` stays (above) |
| 1 FIX core | `rust/fix/src/cfb.rs` (`Parse::drop_counters_beside_groups`, called at `:858-861`, the `required` carry; the four doc edits; the dead arm deleted if phase 0 says so); `rust/fix/src/catalog.rs` (the final pass of `merge_children_at` under `drops.is_some()`, the `required` carry, its comment); `rust/fix/src/registry.rs` (the two rustdoc clauses); `rust/fix/tests/root/cfb.rs` (tests a-h, k, l; the re-cased `:1046-1091`); `rust/fix/tests/root/catalog.rs` restructured (tests i, j); `rust/fix/tests/root.rs:470-472` (the gate removed) | `cargo check -p yggdryl-fix --all-targets`; `cargo test --locked -p yggdryl-fix --test root cfb` (red first on a-f2, k, l, then green); `cargo test --locked -p yggdryl-fix --test root a_counter_beside_the_group_it_counts_is_refused_by_hand` and `--test root a_fold_leaves_out_a_counter` in the default lane, then once more with `--features internals` to prove the `internal` module still runs; `cargo test --locked -p yggdryl-fix --test root the_committed_dictionary_hashes_to_one_pinned_value` and `--test root the_committed_store_carries_the_crate_dump` (must not move; a moved hash is a defect, never re-pinned); `cargo test -p yggdryl-fix --doc cfb` and `--doc registry`; `cargo test --locked -p yggdryl-fix --test allocations` and `--test iobase_calls` unfiltered are left to the whole run |
| 2 docs, skills, inventory | `docs/fix/registry.md:27,1373,1375,1383` and the `merge_with` section, `docs/fix/index.md:480-485`, `docs/fix/cli.md:160-174` where it states the consequence, `python/yggdryl/fix.py:232-238`, `skills/yggdryl-fix/SKILL.md:215-220,328-329`, `.api-inventory.txt:655` and the `add_cfb_files`/`merge_with` rows | `python -m mkdocs build --strict --config-file mkdocs.yml`; `python scripts/check_api_inventory.py` |
| 3 the chain (background, one log) | `cli/tests/fix.rs` (test m) | `cargo test --locked -p yggdryl-fix --all-targets --all-features --no-fail-fast`; `cargo clippy -p yggdryl-fix --all-targets --no-deps -- -D warnings`; `RUSTDOCFLAGS='-D warnings' cargo doc -p yggdryl-fix --no-deps`; `cargo test --locked -p yggdryl-cli --test fix` (`cli/tests/fix.rs:728-742` pins the tag-532 conflict, nothing a counter moves; test m beside it); `python/.venv/bin/python -m pytest python/tests/test_fix.py -k "cfb or cblock" -q` after `maturin develop` (`python/tests/test_fix.py:1865-1881,2586-2635`: no pin moves, run to prove it); Node: nothing pinned, the Node.js CI job proves the doors; `python scripts/check_docs_examples.py --lang rust` only if an example block is touched (none planned) |

Then `cargo fmt --all` once, the commit (the attribution lines the harness states), one push, the CI
run read to `CI result`: a change under `rust/fix/` runs `Leaf` (fix) in both lanes, `Rust format`,
`API inventories`, `Documentation examples (Rust)`, the Python wheel and the `pyarrow>=18` leg, the
Node addon and binding, and `CLI tests` (AGENTS "How a run is planned"); the docs edits add
`docs.yml`.

## Put to the user (interpretations taken; say if another was meant)

1. **"the components having the numingroups fields"** is read as decision 30 reads it - the counter
   *member* is dropped and the component kept - not as "drop the component that lists a counter".
   The user's reason ("our representations are structures and correct group list typed") is the
   count's one owner being the list, which keeps the component.
2. **A group's own counter restated inside its entry is kept** (`<grammar><555/><555/><556/></grammar>`),
   as today, an `int32` member of the entry - decision 30's scope, the entry not holding the group
   that repeats it. The alternative drops it too, the reader knowing the tag it just took as the
   group's counter; taken only on a yes recorded as a decision before the commit.
3. **No log record at all** for a member left out - the literal "silently", decision 30's "no warning,
   no `FixMerge::dropped` entry". The alternative is one `log::debug!` per member, the reader's level
   for what loses nothing, which reaches Python's `logging` whenever `DEBUG` is on; either keeps every
   pin, since the test logger records warn and above.
4. **The left-out member's `required` carries onto the group**: the group is required where the
   constraint or the grammar's own opening counter requires it. The alternative is the nested
   grammar's own presence alone, which loses a `required="Y"` stated on the bare constraint.
5. **The fold leaves the member out under the gate it already folds groups by, `drops.is_some()`** -
   so the user's ingestion (`yggdryl fix ingest`, `add_cfb_files`, `add_cfb_file`), `add_json_file`
   and `merge_with` of any two dictionaries, hand-built ones included, fold without the duplicate
   and never refuse the union or name the file in `failed` for it; the catalog's refusal stays for
   every door that writes a definition alone (`insert`, `insert_definition`, `create_definition`,
   `restate_definition`, the store loader). The alternative keeps `merge_with` of two hand-built
   dictionaries refusing, through a flag only the file doors set and `fold_registry` carries.
6. **A bare counter whose group is in another message, or in none, is kept** as an ordinary member -
   `int32` where any grammar of the file opened a group with it, else at its vocabulary type - never
   dropped and never warned: no list in its structure owns the count. It is left out only once a fold
   brings a group on its counter into that structure.
7. **Two sites, each where its case is decided**: the per-binding pre-pass in `Parse::dictionary` for
   what one grammar states, and the final pass of `merge_children_at` for what two bindings, two
   files or two dictionaries state together; no pre-pass across bindings, which would decide before
   the fold has run and can lose the group it stripped the counter for.
8. **The message-dropped branch** is run first on an empty `grammar-binding type=""`: it stays and is
   pinned where the run reaches it, and is deleted with its docs sentence where it does not - no
   dead arm is kept on a reading alone.

## Taken (decision 35, 2026-10-10)

The readings above are settled by `user_decisions.md` item 35: **#2 dropped** - a group's own
counter restated inside its entry is a NumInGroup field in a component and is left out too (test
(g) asserts the drop, not the keep); **#3 nothing logged**; **#4 the required flag carries** onto
the group; **#5 the `drops.is_some()` gate**; **#8 the arm deleted** with its sentences where test
(h) cannot reach it. Implement these, not the alternatives.
