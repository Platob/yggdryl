# P8: design (D41) - the lifecycle matches on one shared identifier

The user's rule (`p8/user_instruction.md`, `user_decisions.md` 12): a current element matches a
previous **alive** element of the same `MarketDataKind`, of the same `Side` where the kind is sided
(`MarketDataKind::is_sided`, `rust/market/src/marketdatakind.rs:171-173`: `ORDR`, `EXEC`), sharing at
least one identifier the current states - through an index keyed by the value, no scan - and the
predecessor's values propagate onto the follower. Read on the tree at `bbbe10ae6` + P9's working
tree; P9 touches none of the walk's files (`p9/d42_design.md:772`: "the P8 identifier index" is
not in P9), and P7 (`p7/d40_design.md`) names neither `iterator.rs` nor `enrich.rs`.

## Today's rule, in the code

| Fact | Where |
| --- | --- |
| A chain is `(crossuuid, MarketDataKind)` | `rust/market/src/graph/iterator.rs:642` |
| The name index: type -> value -> `Vec<(Side, Chain)>`, two levels so a probe borrows the element's own `&str` | `iterator.rs:583-594` |
| What names a chain: an identifier whose type `IdType::is_chain_identity` (the ten: `rust/market/src/idtype.rs:479-493`) under its type, and a lineage field's value (`origclordid`, `origorderid`, `origtradeid`, `tradereportrefid`: the last of the base's parents) under the base | `chain_names`, `iterator.rs:1327-1340`; `origin_of`, `:41-44` |
| The side a name is alive on: a sided chain's side, an **unsided element's own tag** (`Side::tagged`, `rust/market/src/side.rs:171-176`) - "FIX scopes an `MDEntryID(278)` by its `MDEntryType(269)`" | `walked_slot`, `iterator.rs:135-142, 230-232, 379-382`; used at `:753` (filing) and `:1155-1184` (lookup) |
| What an element cites: its own cross element where alive; a side-less sided element the one live side of its base code (`bases`, `:600`, a `Vec<Chain>` per base); each chain a name it goes by is held by; a sibling split off the same message never | `identity_of`, `iterator.rs:1122-1211` |
| One holder per name, side and kind: the first live chain that stated it, until it ends; a chain identity's old value stays the chain's after a replace | `settle`, `iterator.rs:752-794`; `retire`, `:855-884` |
| An execution cites nothing by a name or a base: "a chain of its own, whatever its state" | `walked_joins`, `iterator.rs:143-148, 233-235, 383-386`; used at `:1125` |
| Two chains cited = a conflict, never a pick: the element stands under its own identity, `note_conflict` (a `FixAnomaly` under `crosscode`, `rust/fix/src/msg.rs:7313-7320`), one `warned!` per kind | `iterator.rs:998-1009, 1185-1203`; `Cited`, `:1362-1386` |
| Alive: `State::is_live` (rank < 80, `rust/src/state.rs:199-201`) and not past `exprunix` | `is_alive`, `iterator.rs:1442-1449` |
| Re-key onto the chain: the live side where none is stated, then the live stored cross code, one finalize | `rekeyed`, `iterator.rs:1414-1424`; `follow_identity`, `rust/market/src/graph/market.rs:1253-1263`; FIX writes the side as `Side(54)`, `msg.rs:7300-7311` |
| Propagation (`with_previous` -> `following_operation`, `market.rs:968-983`): `execunix`, `prevpx`, `prevqty` (`follow_market`, `:1268-1286`); hidden qty, currency, a sided element's side, a quote's legs, `marketdatatype`, unit, ticker, every `securityids` key the follower lacks (unless it names another instrument), strike, `origccy`, `instcode`, CFI, MIC, every `metadata` key it lacks (`chain_market`, `:1310-1395`); `ordqty`, `cumqty`, `avgpx`, `timeinforce`, `tradable`, the parents of each stated identifier (`follow_parents`), **the identifiers `is_followed_identifier` admits**, every party id (`chain_operation`, `:1728-1780`) | |
| `is_followed_identifier`: a plain holder every type but `mdentryrefid` (`market.rs:887-889`); a FIX message its registry's `FIX:idmap` `follow` flags - `orderid` and `secondaryorderid` alone (`config/fix/fields`, read by `scratchpad/idmap.py`) - and their parents (`msg.rs:7277-7291`) | |
| The FIX walk: `LifecycleMessage` over `EventIterator`, delegating every door (`rust/fix/src/enrich.rs:508-700, 1056-1166`); `inherit_side`/`inherit_order_links` inside `with_previous` and `restating` (`:460-506, 575-625`) | |

## Evidence: the capture

Over the equivalence snapshot (`rust/fix/tests/root/equivalence.snapshot`, written by
`rust/fix/tests/root/codec.rs:4063-4088` over `rust/tests/support/ulbridge.log`, 144 bridge lines,
151 parsed messages), two readings (`p8/evidence/evidence.py` over the walked rows, `evidence2.py` over the parsed
messages, `idmap.py` the dictionary's `FIX:idmap`/`FIX:parents` flags; each run with
`python3 -I <script> <path>`):

- **The parsed messages in instant order** (126 of a chained kind: 68 `ORDR`, 57 `EXEC`, 1 `TRAD`):
  11 `ORDR` arrivals join a live chain, every one by its own cross code (`OrderID(37)`); **0** join
  by a chain-identity name; **0** share an identifier of any other type - `execid`, `trdmatchid`,
  `tvtic`, `secondaryorderid`, a bridge's `market:orderid`/`omsdealer:orderid`/`ultrader:clordid` -
  with a live element of their kind and side under another cross code. Every `EXEC` row is
  `FILLED` (terminal) and never alive.
- **The 39 walked rows**: `prevuuid` on `lifecycle[010]`, `[012]`, `[016]`, `[033]` (the walk's
  expiration) and `[035]` (a `35=9` reject joining its `35=F` by `clordid`/`orderid`/`origclordid`),
  every one a join today's rule and the user's agree on. Conflicts recorded: **0** (`grep -c cites`,
  `grep -c anomal` over the snapshot).
- **The rows that share an order's identifiers and do not join are of another kind**: `[014]`
  (`0:0:00079132557GLXC0.9`, `UKNW`, a bridge-typed line), `[019]` -> `[023]` (`XM8NNITE383`,
  `UKNW` then `ORDR`), `[024]` (`35=n`, `SESS`, `18:0:XM8NNITE384` `FILLED` with `clordid` and
  `execid`), `[036]` (`35=cancelreject`, `UKNW`). The user's "same kind" keeps them apart; see
  "Put to the user" 6.

So on this capture the user's rule moves **no row** of the snapshot: it widens what *can* match
(every identifier type, an execution's chain) without changing any join the capture makes. The
design below is therefore judged on the FIX semantics of each identifier type and on cost, not on a
row count.

## What changes

### 1. The identifier types that name a chain

Today the ten chain identities (`is_chain_identity`) plus a lineage value under its base. The
user's words: "any identifier of current is in previous". Read on the `identifiers` map alone -
`Operation::get_identifiers` - never `securityids` (an ISIN is every order of the instrument) nor
`partyids` (an account is every order of the client), which `identifier_set` already files apart
(`market.rs:870-879`).

Decided: **every type of the `identifiers` map names the chain that states it, but three** whose
value names something other than its own chain:

| Excluded | Why |
| --- | --- |
| `quotereqid` (`QuoteReqID(131)`) | every dealer answering one request states it: two `QUOT` chains would fold (`docs/fix/lifecycle.md:277`, `idtype.rs:456-463`) |
| `mdreqid` (`MDReqID(262)`) | every entry of one subscription states it: a book's entries would fold |
| the previous-value / hierarchy slot `parent{type}` - `parent_of()` at a place before the last of its base's `parents()` (`parentorderid`, `parenttradeid`; `idtype.rs:388-403`) | the walk itself writes it from the chain's previous value (same chain, already named), and a bridge spells a parent order by it (`omsdealer:parentorderid=-8NNI7PB-00`, `lifecycle[006]`): two child orders of one parent, same kind and side, would fold |

A lineage type - the last parent (`origclordid`, `origorderid`, `origtradeid`,
`tradereportrefid`) - keeps today's reading: its value is the chain's first value, looked up and
filed **under the base** (`origin_of`, `iterator.rs:41-44`), and under itself too, as every other
type is. Matching stays by **type and value**, whatever the source (`market:orderid=X` matches
`orderid=X`, as today's `(type, value)` reading does, `docs/fix/lifecycle.md:277`); a value equal
across two types (`execid=1705` and an `orderid=1705`, `lifecycle[001]`) is no match - see "Put to
the user" 1.

The vocabulary owns the rule, one owner: `IdType::is_chain_name(&self) -> bool` beside
`is_chain_identity` (`idtype.rs:479`), non-`const` because it reads `parent_of()`; Rust-only like
`is_chain_identity` (`.api-inventory.txt:7360`). `is_chain_identity` keeps its other role - which
types have parents and follow - unchanged.

### 2. The index: key and bound

Keep the two-level `named: HashMap<IdType, HashMap<String, Vec<(Side, Chain)>>>`
(`iterator.rs:594`): a probe is two hash lookups borrowing the element's own `&IdType` and `&str` -
the user's "optimally": one probe per stated identifier, nothing per live chain - and the slot
`Vec` holds at most one entry per `(kind, side)` holding the name (two sides x the kinds in flight),
so the filter at `:1164-1184` is constant, not a scan. The logical key is therefore
`(IdType, value) -> (kind, side) -> Chain`, read as `(kind, side-if-sided, type, value)`.

**What a slot holds moves.** Today every name a chain ever stated stays filed until the chain ends.
Under "in previous" - the live element's identifiers - a chain files, per type:

- a **chain identity**'s every value it went by (today's rule, kept: a late report citing the old
  `ClOrdID` after a replace must still find the order, `iterator.rs:1335-1347`, `:2040`);
- any **other type**'s value the live element holds **now** - one record per type per chain: on
  `settle`, the value the chain filed under that type and the element no longer holds is released
  (`names_of`, `:603`, already lists the chain's filed `(type, name)` pairs, so the release is one
  walk of that `Vec` and one `retain` on the slot, as `retire` does at `:860-873`).

Bound: live chains x (the chain identities each stated over its life + the other types its live
element holds) - **never one per report**, which is what `a_per_report_reference_names_no_chain`
(`rust/market/tests/graph/iterator.rs:1275-1333`) pins today at "two records, whatever the count"
and re-pins at three (`orderid`, `clordid`, the one `execid` the live report states).

### 3. The side

`(kind, side-if-sided)` exactly as the user said: a sided kind's `stored_side`
(`marketdatakind.rs:181-183`), `Side::Unknown` for every other kind. The quote's **tag slot** goes:
`walked_slot` (`iterator.rs:135-142, 230-232, 379-382`) and the `on(slot)`/`on(Unknown)` fallback
(`:1170-1183`) are deleted, `settle` files under `walked_side` (`:753`), and an unsided element's
name is alive on one slot. Evidence: the capture holds no quote or book entry; FIX defines
`MDEntryID(278)` as the unique entry identifier, not one per entry type. A side-less element of a
sided kind keeps today's reading: it cites the one live side of its base code (`:1146-1154`) and
every side a name of it is alive on; two sides is the conflict.

### 4. Two live chains cited

**Keep the conflict** (`iterator.rs:1185-1203`; `docs/fix/lifecycle.md:281-283` "nothing merges two
chains"). Evidence: 0 conflicts on the capture under either rule. A rank between cites (a chain
identity over a per-report reference) would be a pick, which the page forbids for the reason it
gives - a wrong join re-keys a message onto a stranger and folds its facts into it - and the one
fixture a rank would save (two side-less orders whose reports share one `TrdMatchID`,
`:1275-1300`) is unreal: a match's two reports are the buy's and the sell's and state `Side(54)`, so
the side rule already keeps them apart. The fixture is re-spelled sided. See "Put to the user" 3.

### 5. The end of a chain

Unchanged: alive is `is_alive` (`iterator.rs:1442-1449`) - a live state (`State::is_live`, rank <
80) and not past its deadline; a `FILLED`, `CANCELED`, `REJECTED`, `EXPIRED` element retires its
chain and its names (`retire`, `:855-884`), and the next element under its identity starts afresh.
The user's "ALIVE" is this.

### 6. Executions

`walked_joins` (`iterator.rs:143-148, 233-235, 383-386, 1125-1127`) goes: an `EXEC` element matches
a live `EXEC` of its side sharing an identifier, as every kind does - the user's rule names no
exception, and the kind rule already keeps a fill off its order. Evidence: every execution of the
capture is `FILLED` and so never alive; nothing moves. `an_execution_joins_no_chain_by_a_name`
(`tests/graph/iterator.rs:1532`) and `a_live_execution_keeps_its_orders_name_record` (`:1557`)
stand as written - both are the kind rule - with the first's sentence re-spelled ("by a name of
another kind").

### 7. Propagation

**Unchanged in shape and in set**: `following_operation` (`market.rs:968-983`) already carries
every market and operation fact the follower states nothing of (table above), every `metadata`
key, every security identifier and every party id it lacks, and the parents of each identifier it
states. The one gap the user's "propagate values" could name is the identifiers a FIX follower
takes: the `FIX:idmap` `follow` flags admit `orderid` and `secondaryorderid` alone
(`msg.rs:7277-7291`). Decided: left as the dictionary states it - the flags are the generator's
(`IDMAP_SOURCES`, `scripts/generate_fix_dictionary.py`, `docs/fix/registry.md:1101`), a
regeneration fetches the standard, and no capture row lacks a chain identifier its predecessor
holds (`lifecycle[010]`, `[012]`, `[016]`, `[035]` carry their chains'). Widening it is "Put to the
user" 5.

### 8. The cost

Per element: one `named` probe per stated identifier (two hash lookups, borrowed), one `alive`
probe per cited chain, one `bases` probe for a side-less sided element, one `names_of` walk on
settle; a name the chain already filed re-settles allocation-free (`iterator.rs:755-763`), a new
name costs its `String` key and slot once. Pins:

- `rust/fix/tests/allocations.rs` `FIX_PIPELINE_COSTS` `lifecycle` (`:1541-1578`, 12/12/11 on this
  tree; P9 phase 4c is settling 20/16/16 - P8 starts from P9's final numbers): expected to
  **stand** - the three lines are terminal, the walk retires each and files no name
  (`:1530-1538`); a move is a defect, never a re-pin.
- New, `rust/market/tests/allocations.rs`: one walk step - an order's report stating its chain's
  `orderid` and a new `execid` - allocates alike with 8 and with 1024 live chains, each going by
  four names (the "no scan" pin, at two corpus sizes as every `allocations` row).
- `cargo bench -p yggdryl-fix --bench fix -- lifecycle --quick` (`rust/fix/benchmarks/fix/pipeline.rs:554, 575`,
  `decoded_lifecycle*` `:233-361`, `ulbridge.rs:173 parse_lifecycle`): direction only, before and
  after; no page states a number for them.

### 9. Pins expected to move, each with its sentence

| Pin | Moves to |
| --- | --- |
| `tests/graph/iterator.rs:1275` `a_per_report_reference_names_no_chain` | `a_per_report_reference_names_the_live_report_alone`: two sided orders one match filled share a `TrdMatchID` and stay two chains by their sides; a report's `execid` names its chain while it is the live statement and is released by the next report, so a chain of sixty-four reports holds three records (`orderid`, `clordid`, one `execid`), whatever the count; `quotereqid`, `mdreqid` and `parentorderid` name none |
| `:1335` `a_replace_files_both_values_under_the_base_and_no_lineage_slot` | `..._and_under_the_lineage_slot`: `origclordid=C1` filed under `clordid` and under `origclordid` |
| `:1768` `a_quote_holding_both_sides_goes_by_its_names_as_an_untagged_one`, third clause | a bid and an offer going by one `quoteid` are one chain: the second tagged statement follows the first (the tag slot is gone) |
| `:1349` `a_conflict_is_warned_and_files_no_name_another_chain_holds` | stands |
| `:660`, `:1479`, `:1599`, `:1675`, `:1838`, `:1877`, `:1935`, `:2040`, `:2089`, `:2142` | stand (own code, sides, base, siblings, lineage, conflict) |
| `rust/market/tests/root/idtype.rs` | gains the `is_chain_name` rows: every chain identity and `execid`, `trdmatchid`, `tvtic`, `mdentryid`, `mdentryrefid`, `secondaryexecid`, a `regtradeid`, an `Other` word true; `quotereqid`, `mdreqid`, `parentorderid`, `parenttradeid` false; `origclordid`, `origorderid`, `tradereportrefid` true |
| `rust/fix/tests/root/codec.rs` equivalence snapshot | **expected unchanged** (0 rows move); a change is a defect to read, not a regeneration |
| `rust/fix/tests/root/enrich.rs:1514, 1604, 1683, 1724, 1926, 1961, 2017, 3603` | stand; one new test: a venue's report stating only the bridge's `oms:orderid` the order was placed under joins the order |
| `rust/fix/tests/allocations.rs:1541` lifecycle 12/12/11 | stands (see 8) |
| `python/tests/test_fix.py:1263-1271`, `node/tests/fix.test.js:2510` | stand (own-code chains, a terminal fill) |

## Implementation plan

One commit, phases in layer order; the market phase is startable the moment P9 is pushed - P9 and
P7 touch none of these files. Smoke after every edit; the whole run once at the end.

**Phase 1 - `yggdryl-market`** (`rust/market/src/`)

1. `idtype.rs`: `pub fn is_chain_name(&self) -> bool` beside `is_chain_identity` (`:479`), with
   its rustdoc rows; `rust/market/tests/root/idtype.rs` the rows above.
   Smoke: `cargo test -p yggdryl-market --test root idtype`; `cargo test -p yggdryl-market --doc idtype::IdType::is_chain_name`.
2. `graph/iterator.rs`: `chain_names` (`:1327-1340`) filters by `is_chain_name`, a lineage type
   under its base and itself; delete `walked_joins` (`:143-148, 233-235, 383-386`) and its use
   (`:1125-1127`); delete `walked_slot` (`:135-142, 230-232, 379-382`), `settle` files under
   `walked_side` (`:753`), `identity_of` keeps the `Unknown`-side branch (`:1164-1168`) for every
   unsided element and `on(side)` alone for a sided one (`:1170-1184`); `settle` releases a
   non-chain-identity type's former value through `names_of` (`:754-794`); the type doc
   (`:408-520`), the `named` field doc (`:583-594`) and `identity_of`'s (`:1093-1121`) re-spelled.
   Smoke: `cargo check -p yggdryl-market --all-targets`; `cargo test -p yggdryl-market --test graph iterator`;
   `cargo test -p yggdryl-market --test graph iterator --features internals` (the `naming` module, `tests/graph/iterator.rs:1185`);
   `cargo test -p yggdryl-market --doc graph::iterator::EventIterator`.
3. `rust/market/tests/graph/iterator.rs`: the pins of §9, plus: a bridge-sourced `oms:orderid`
   names the chain; `execid` released at the next report; `quotereqid`/`mdreqid`/`parentorderid`
   name none; a live execution followed by another execution of its side sharing `orderid`.
   `rust/market/tests/allocations.rs`: the 8/1024 walk-step row.
   Smoke: `cargo test -p yggdryl-market --test allocations walk`.
4. `.api-inventory.txt`: `is_chain_name` row near `:7360`; the `EventIterator` row (`:1838`) and
   `is_followed_identifier` (`:1616`) re-spelled. `python scripts/check_api_inventory.py`.

**Phase 2 - `yggdryl-fix`** (no source change expected: `LifecycleMessage` delegates,
`FixMsg::follow_identity` and `note_conflict` stand)

5. `rust/fix/tests/root/enrich.rs`: the `oms:orderid` join test.
   Smoke: `cargo test -p yggdryl-fix --test root enrich`; `cargo test -p yggdryl-fix --test root the_codec_answers_what_it_answered`
   (must pass unchanged); `cargo test -p yggdryl-fix --test allocations a_real_line_costs_the_same_at_every_stage_every_time`.
6. `cargo bench -p yggdryl-fix --bench fix -- lifecycle --quick` before and after, direction noted
   in the handoff.

**Phase 3 - docs** (`mkdocs build --strict`; `python scripts/check_docs_examples.py --lang rust`
for the lifecycle page's blocks)

7. `docs/fix/lifecycle.md:273-283` (the two sections: names = every identifier but the three, the
   live statement's values for a non-chain-identity type, no execution exception, no tag slot);
   `docs/graph/event.md:308-312` (Live set, Sides, No side, Names, Conflict rows);
   `docs/graph/operation.md:89`; `docs/graph/identifier.md:608, 614` (+ an `is_chain_name` row);
   `docs/types/enum/marketdatakind.md:350`; `skills/yggdryl-market-data/references/python.md:168-176`,
   `javascript.md:147-155` ("never an `execid`" goes).

**Phase 4 - whole run, push, CI**

8. `cargo fmt --all`; `cargo clippy -p yggdryl-market -p yggdryl-fix --all-targets --no-deps -- -D warnings`;
   `cargo test -p yggdryl-market --all-targets --no-fail-fast`; `cargo test -p yggdryl-fix --all-targets --no-fail-fast`;
   both `--doc`; `python scripts/check_api_inventory.py`; push; read `CI result`.

Bindings: nothing - `is_chain_identity` is Rust-only and so is `is_chain_name`; the Python and Node
lifecycle doors redirect to the same walk.

## Put to the user

1. **Type and value, or value alone?** Recommended: by type and value, whatever the source (today's
   reading; `oms:orderid=X` already matches `orderid=X`). The alternative - "any identifier value of
   current is in previous" literally - joins `execid=1705` to an order whose `orderid` is `1705`.
2. **The three exclusions** (`quotereqid`, `mdreqid`, the `parent{type}` slot). Recommended: keep
   them - each value names many chains or another chain by construction, and the capture's bridge
   states `parentorderid=-8NNI7PB-00` on its child orders. Literal "any identifier" folds dealers'
   quotes, a subscription's entries and sibling child orders.
3. **Two chains cited**: keep the conflict (recommended; 0 on the capture; the page's "never a
   pick"), or rank a chain identity over a per-report reference.
4. **The quote tag slot** goes (a bid and an offer under one `quoteid`/`mdentryid` are one chain,
   the literal "side if sided"). Recommended; the alternative keeps `walked_slot` as a refinement
   within "no side".
5. **Propagated identifiers**: unchanged (the dictionary's `follow` flags: `orderid`,
   `secondaryorderid`) - recommended; or every chain identity follows by default, which moves the
   generator's `IDMAP_SOURCES` and regenerates the dictionary (a fetch of the standard).
6. **Out of P8's scope, seen in the capture**: the messages that share an order's identifiers and
   do not join are of kind `UKNW`/`SESS` (`35=n`, `35=cancelreject`, a bridge's own types,
   `lifecycle[014]`, `[019]`, `[024]`, `[036]`). A kind inferred from the identifiers a message
   states would be a dictionary/P7 question (`FIX:msgcat` on those types), not the walk's.
7. **Executions chain by a shared identifier** when alive (the literal rule, `walked_joins` gone).
   Nothing in the capture is an alive execution; say if executions should stay chains of their own.
