# P12: design (D45) - fill accounting along an order's chain

The user's instruction (2026-10-10 ~09:40 UTC, `$S/p12/user_instruction.md`, verbatim there):
"refine fixmessage lifecycle to find a stable way to refine the state for partiallyfilled orders
executed leveraging the with_previous and compare the execid to when if different updates the
real consumed and update the remaining quantities, and if it hits 0 it changes the part fill like
states to fully filled". Recorded as decision 26 (`$S/user_decisions.md:145-153`), with decision
27 (`:154-160`, the one definition of `quantity`) landing in the same slice. It lands **after
wave 1 and over P8's match** (decisions 12 `:44`, 13 `:48`, 25 `:139` of `$S/user_decisions.md`: a
current element matches the previous alive element of its kind, instrument and side sharing one
identifier of the same type and value; `$S/p8/d41_design.md`), and over decision 21 (`:108`, the 500 ms
official-clock rule, which is why the capture's frame hop of order `00079132558GLXC0` is a
delivery of its own, `python/tests/test_fix.py:734-745`). Its own design, its own commit.

Every claim names the file and line it was read from. **Line numbers are from the working tree
at `483644b1c` while P8 is uncommitted and still being edited** (`git status`: `iterator.rs`,
`msg.rs`, `idtype.rs`, `side.rs` modified); the implementing session re-reads each after P8
lands. Nothing was built or run to write this but the Python probe under `$S/p12/`
(`fills_probe.py` -> `probe_walked.tsv`, `probe_parsed.tsv`, against the installed extension).

`$S` is `.handoff/split/scratch`.

This revision applies a review of the first draft (nineteen findings, each verified against the
sources it cites; the two rejected in part are said so under [Put to the user](#put-to-the-user)).
The one change of direction: the first draft let a stated `CumQty(14)`/`LeavesQty(151)` outrank the
count, which made the user's rule do nothing on conforming FIX - both tags are required on every
`ExecutionReport` and the parse fills `14 = 38 - 151` (`native_derivations.rs:311-322`), so the
ledger would never have set `cumqty` on real traffic. D45.2 now anchors on the chain's first stated
total and counts from there.

## The user's asks, each mapped to a decision

| # | The ask (the user's words) | Decision |
| --- | --- | --- |
| 1 | "refine fixmessage lifecycle to find a stable way" | D45.4: the rule is the lifecycle walk's (`EventIterator`, `rust/market/src/graph/iterator.rs`), the one place that holds a chain's memory; the FIX lifecycle (`FixCodec::lifecycle`, `rust/fix/src/codec.rs:2729`) is that walk over `LifecycleMessage` and inherits it; FIX adds one thing only, the reading of what a report is (`Fill`, D45.1) off its own tags |
| 2 | "leveraging the with_previous" | D45.4, D45.5: the walk's step is `with_previous` (`iterator.rs:1115-1136`); the accounting runs on what it answered, before the element is settled as the chain's live statement - the shape `UPDATED` already takes (`:1127-1134`). `with_previous` itself does not change |
| 3 | "compare the execid to when if different updates the real consumed" | D45.1: the chain's ledger - every fill it counted by `execid` with the quantity it counted, and what they add up to over the chain's anchor - held by the walk per live order chain; a fill whose `execid` (or `secondaryexecid`) is new adds its `lastqty`; a repeated one adds nothing, moves nothing, promotes nothing; a bust subtracts the fill it refers to, a correction replaces it |
| 4 | "and update the remaining quantities" | D45.1, D45.2: `cumqty = anchor + counted` and `leavesqty = accepted ordqty - cumqty` on every fill the ledger counts; a stated total is adopted only as the chain's anchor (its first statement, or one that agrees with the count), a later disagreeing one is warned and does not move the ledger |
| 5 | "if it hits 0 it changes the part fill like states to fully filled" | D45.3: a count that reaches the accepted `ordqty` turns `IN_PROGRESS`, `PARTIALLY_FILLED` or `TRADE` into `FILLED`, never the reverse; a stated `leavesqty` 0 alone promotes nothing - the venue states what a remainder became |
| 6 | decision 27: "coherent overall definition for generic quantity field" | D45.6: on an order, a quote and a book entry `quantity` is what is still available (`leavesqty`, today's rule); on an execution and a trade it is what executed (`lastqty`, today `None`) |
| 7 | "stable" | D45.8: the same set of fills gives the same result whatever order they arrive in, within the walk's sorted order; a replay, a twin, a view, a status reply, an expiry and a late copy of an ended chain's fill count nothing |

## Today, in the code

| Fact | Where |
| --- | --- |
| A follower takes its predecessor's `ordqty`, `cumqty`, `avgpx` where it states none - "never a last fill, which no rise in `cumqty` invents" | `rust/market/src/graph/market.rs:105` (the implication table), `:1715-1727` (`chain_operation`'s doc) |
| `chain_operation` carries `ordqty` always (fill-if-absent, `:1729-1733` - so a chain's `ordqty` is whatever the last statement said, a pending replace's `OrderQty` included); `cumqty` only where this statement's `lastqty` is none or zero (`:1734`); `avgpx` only where the two `cumqty` agree (`:1741`); `leavesqty` never; `lastqty` never added. A follower reporting a fill and no `cumqty` ends with `cumqty` `None` and `leavesqty` `None`. Identifiers the follower lacks are carried where `is_followed_identifier` says so (`:1762-1768`), and the generic answer is every type but `mdentryrefid` (`:887-889`) - so a bare `OrderEvent` follower takes its predecessor's `execid` | `market.rs:1728-1783`; pinned by `rust/market/tests/graph/market.rs:2151` `an_operation_follower_takes_the_cumulative_fill_of_its_chain`, the fill clause `:2192-2203` (`fill.get_cumqty() == None`, `get_leavesqty() == None`) |
| `following_operation`: refuses its own predecessor or a later one, then `fill_execution`, `follow_timed`, `follow_market`, `follow_operation`, finalize where anything moved | `market.rs:968-983`; `FixMsg::with_previous` delegates to it after the session-event merge check (`rust/fix/src/msg.rs:6741-6746`); `LifecycleMessage::with_previous` adds `inherit_order_links` and `inherit_side` first (`rust/fix/src/enrich.rs:576-603`) |
| The one "consumed since the predecessor" rule today is the iceberg's hidden part: kept less the rise in `cumqty`, else this statement's `lastqty`, floored at 0 - run inside `with_previous`, where a follower's `cumqty` is still `None`, so it falls back to `lastqty` whatever the execid | `market.rs:686-713` (`follow_hidden`) |
| `Standing::of(state)`: `Unknown` -> Unstated, `FILLED` -> Filled, the cancel band + `DONE_FOR_DAY`/`EXPIRED` -> Canceled, other non-live -> Ended, pending or rank 20 -> Fresh, every other live state (`PARTIALLY_FILLED`, `TRADE`, `IN_PROGRESS`, ...) -> Working. Working fills the third of `ordqty`/`cumqty`/`leavesqty`; Filled forces `leavesqty` 0 and `cumqty = ordqty` where `cumqty` is absent. An `EXEC`/`TRAD` kind reads as Working whatever its state. **No path moves the state when leaves reaches 0** | `rust/market/src/graph/facts.rs:199-208` (`Standing`), `:222-241` (`of`), `:249-257` (`reports_fills`: `EXEC`, `EXEB`, `TRAD`, `TRDB` and nothing else - so `false` for `BOOK`, `SESS`, `UKNW`, a quote request, not only `ORDR`/`QUOT`), `:753-758` (`set_standing`), `:807-815` (`leaves_moved`: an order's `quantity` follows its `leavesqty`), `:819-930` (`orders_filled`; Working arm `:844-874`, Filled arm `:877-897`); `EventFacts::set_state` stamps the standing `:1632-1636` |
| Following folds the state to the higher rank; `PARTIALLY_FILLED` 4001 / `TRADE` 4002 (rank 40) sit below `FILLED` 8003 (rank 80); `is_live` is rank < 80; `is_execution` is exactly `PARTIALLY_FILLED`, `TRADE`, `FILLED` - true of a status reply, a restatement or a done-for-day report that reads `PARTIALLY_FILLED` as much as of a fill | `rust/src/state.rs:96-100` (`IN_PROGRESS` 4000 "Working, with some of it done", `PARTIALLY_FILLED`, `TRADE`, `TRADE_CORRECT`, `TRADE_CANCEL`), `:121` (`FILLED`), `:155-158` (`rank`), `:167-173` (`merge_with`), `:195-197` (`is_live`), `:229-231` (`is_execution`); `rust/src/graph/element.rs:1176-1177` (`fold_lifecycle` takes `merge_with`), `:607-636` (`follow_timed`) |
| A message's state is the first of `STATUS_TAGS` `[39, 150, 1036, ...]` that reads - **`39` before `150`**, and `OrdStatus(39)` is required, so a bust `150=H 39=1` or a correction `150=G 39=1` reads `PARTIALLY_FILLED`, a status reply `150=I 39=1` too - else what its msgtype asks for; tags 39 and 150 read **wire codes only** (`1` PartiallyFilled, `2` Filled, `F` Trade), so a bridge's `ORDSTATUS=partfilled` reads `None` and `ExecType` `F` decides: `TRADE`. The code set's own reading by name (`FixCodeSet::code_by_name`, `rust/fix/src/codes.rs:1180`) resolves `filled` to `2` at the parse (snapshot `:704`, `39=2` on the bridge row of order 558) and `partfilled` to nothing | `rust/fix/src/state.rs:44-47`, `:71`; `rust/src/state.rs:291-316` (`STATE_CODES`), `:322-327`, `:356-357` (`partfill`/`partfilled`); `msg.rs:2477-2488` (`state_market` reads `STATUS_TAGS`); `msg.rs:2338-2341` (`explicit_execution_type`: `150` in `F`/`1`/`2`), `:2363-2381` (`reports_execution`: that, a new `AE`, never `AD`/`AQ`/`AR`) |
| "Leaves 0 means filled" exists only as a per-message parse rule: on a `35=8/9` with `ExecType` `F`/`G`, `OrdStatus(39)` fills `2` where `LeavesQty(151)` is 0 and `1` where leaves and `CumQty` are positive; `CumQty(14)` fills `OrderQty - LeavesQty` on a live or filled report; `LeavesQty(151)` fills 0 once ended, else `ordqty - cumqty`. Each fills an absent tag only, on the row, so a derived `14` is indistinguishable from a stated one past the parse - and since `14` and `151` are required on every `ExecutionReport`, a conforming report always carries both past it | `rust/fix/src/native_derivations.rs:13-14`, `:24`, `:31` (the table), `:243`, `:248`, `:256` (`fill!`), `:274-275` (`AGREED`, `TRADES`), `:311-322` (`cumulative_quantity`), `:376-393` (`order_status`), `:199` (`put`) |
| `FixMsg`'s fills are lifted at settle under `fact::FILLS` - `lastqty`, `avgpx`, `cumqty`, `leavesqty` - then `ordqty`, then `lastpx`; `settle_orders` runs when `ORDERED` or `STATE` moved; the setters mark the fact stated | `msg.rs:116` (`FILLS`), `:129` (`ORDERED`), `:2651-2657`, `:2694-2696`, `:7112-7142` (`set_lastqty`..`set_leavesqty` mark `FILLS`), `:6784-6787` (`set_state` marks `STATE`) |
| A report of an execution splits once at the parse: the report becomes its order's `ORDR` (its quote's `QUOT` with a `QuoteID`) and keeps its own state and fills; a clone is refiled `EXEC`, `FILLED`, cross code `ExecID(17)` as given, else `TradeID=..`, else `<base>\|Execution=<hash>` - so each execid is its own `EXEC` chain (`8:<side>:<execid>`), ended at once | `rust/fix/src/market.rs:1216-1259` (doc), `:1260-1319` (`split`; the base `:1296-1306`), `:1324-1333` (`refile_executed`); `market.rs:932-935` (`operation`: an `EXEC` leaf reads `FILLED`) |
| `ExecID(17)` maps into `identifiers` as `execid` with **no follow flag**, and FIX itself says it "will be 0 (zero) for ExecType (150)=I (Order Status)"; `SecondaryExecID(527)` maps as `secondaryexecid` - "the ExecID (17) used by an exchange", so one fill can carry two ids across two routes; `ExecRefID(19)` ("used with Trade, Trade Cancel and Trade Correct") has no `FIX:idmap`; `MultiLegReportingType(442)` has its code set and nothing in `rust/fix/src/` reads it (`grep -rn 442 rust/fix/src` finds nothing). Under P8 `execid` is a chain name (`is_chain_name` true, matched while it is the live report's) but no chain identity; `IdType::ExecId::parents()` is empty, "a per-report reference" | `config/fix/fields/000000000.json:236-252` (`:243-248` the idmap entry, `:250` the description), `:269-280` (19), `:526-540` (`:538` `follow: true` on `orderid`); `000000005.json:377-387` (527); `000000004.json:583-589` (442); `msg.rs:7359-7373` (`is_followed_identifier`); `rust/market/src/idtype.rs:343`, `:475`, `:481-494`, `:522`, `:534-545` |
| The walk: `Live { element, arrived, passed }` per chain `(crossuuid, kind)`; `settle` records a live element and its names, or retires the identity where it is not alive (`is_alive`: `state.is_live()` and not past its deadline); a retired chain's statements are findable only by the identity they arrived under and only until the walk passes that instant; a later copy of an already-counted execid at another instant starts a fresh chain with `prev` none. `warned!` deduplicates on `(site, what, subject)` and builds its detail on the first occurrence only; the conflict warning keys by the kind | `iterator.rs:591-662` (`EventIterator`), `:665` (`Chain`), `:672-676` (`Live`), `:765-883` (`settle`; alive branch `:777-851`, retire branch `:863-882`), `:924-940` (`retire`), `:1055-1160` (`walk_source`: a passed statement restated `:1100`, a twin `:1105`, out of order `:1106-1113`, follow + `UPDATED` `:1115-1136`, a first statement `:1137-1140`, `rekeyed`/`created`/`settle` `:1145-1158`), `:1073-1077` (the conflict `warned!`, subject `walked_kind().as_str()`), `:1505-1512` (`is_alive`); `rust/src/logging/warning.rs:55-80` (`warn`: `key(site, what, subject)`), `:120-131` (`warned!`); the dedup window drops a repeat by `uuid` alone, swept past a bound (`enrich.rs:1004-1061`, `Window { span, held, watermark, sweep_at }`; `codec.rs:678` `DEFAULT_DEDUP_WINDOW_MS` 60 s) |

### The capture (144 lines, 151 parsed, 41 walked, 23 market data: 9 executions, 14 order events)

Seven distinct execids; two are walked twice and counted twice (`$S/p12/probe_walked.tsv`;
`python/tests/test_fix.py:723-754` pins 144/151/41/42/23/9+14; `rust/fix/tests/root/ulbridge.rs:490`,
`:611`, `:1506` pin 41, `:1530` 23):

| Order (ordqty) | Walked | Evidence |
| --- | --- | --- |
| `00079132557GLXC0` (600, BUYS) | #11 execid 457, last 21, **stated** cum 340 / leaves 260 `PARTIALLY_FILLED`, prev none (319 traded before the capture); #13 execid 467, last 57, stated 397 / 203, prev #11 (340 + 57 = 397, the count agrees); #15 execid 468, last 75, cum 600 / leaves 0 `FILLED`, prev #13 - **its frame states `151=0` and no `14`, no `39`, no `11`** (`528=R`, `9502=From Exchange`): the 600 and the `FILLED` are the parse's derivation, while the count gives 340 + 57 + 75 = 472 and leaves 128; #19 at .762: execid 467 **again** (frame `40219`: `151=0`, no `14`/`39`/`11`, `60=20260814` date-only so dated by `SendingTime`, decision 21) as `ORDR` `FILLED` cum 600, prev none - a fresh chain, #15 having retired; #20 its `EXEC` twin, `lastqty` 57 counted a second time (executions total 210 against 153 distinct); #17 a typeless bridge row (`UKNW`, `0:0:00079132557GLXC0.9`) restating 457 outside every chain. **One fill cannot leave this order both at 0 (`40219`, From Exchange) and at 203 (`40220`, To Client, the same execid): the From Exchange `151=0` describes another level - the exchange-side child, whose `ClOrdID` the frame does not carry - and the parse's `14 = 600` is that level's, not the order's** | `probe_walked.tsv` rows 11-20; `rust/tests/support/ulbridge.log:6` (`40218`: 17=457 14=340 39=1 151=260), `:35` (`40219`: 17=467 151=0, no 14/39/11, `528=R`, `442=1`, `9502=From Exchange`), `:36` (bridge row `LEAVESQTY=0`, no `CUMQTY`/`ORDSTATUS`), `:56` (`40220`: 17=467 14=397 39=1 151=203, `11=00079132557GLXC0.9`, `528=A`, `9502=To Client`), `:73` (`40221`: 17=468 151=0, no 14/39/11, `From Exchange`), `:74`; the snapshot's lifecycle rows `rust/fix/tests/root/equivalence.snapshot:22137` ([011] state), `:22153` (cum 340), `:22532`/`:22548`/`:22530` ([013] state, cum 397, prevuuid), `:22930`/`:22946`/`:22928` ([015] `FILLED`, 600, prevuuid), `:23573`/`:23589` ([019] `FILLED`, 600), `:23774`/`:23790` ([020]) |
| `00079132558GLXC0` (300) | #6 at .599000: execid 461, last 235, stated cum 300 / 0 `FILLED` (65 traded before the capture); #9 at .599743 the same execid - the frame hop decision 21 dates by `SendingTime` - a fresh chain with prev none; #10 its `EXEC`, a second one under `8:1:00064703461GBYZ0`: executions total 470 against ordqty 300 | rows 6-7, 9-10; `ulbridge.log:1-5`; `test_fix.py:735-739` |
| `20260814_CQ9_LIAPUS_9623` (3,000,000) | one report, execid 1705, last 24,000, cum 230,000, leaves 2,770,000 - walked state **`TRADE`**, not `PARTIALLY_FILLED`: the session merge kept the bridge statement `39=partfilled`, which `from_status` cannot read, so `150=F` decided; the FIX frame (`150=1 39=1`) parses `PARTIALLY_FILLED` | rows 3-4; `probe_parsed.tsv` rows 132-137; `ulbridge.log:123-126`; snapshot `:20923` ([001] `TRADE`), `:21015` ([002]) |
| `00079132541GLXC0` (36) | execid 546, last 36 = cum 36 = ordqty, `FILLED`, consistent | rows 31-32 |
| `00079132559GLXC0` (400) | `NEW` -> `UPDATED` -> `EXPIRED`, no fill | rows 8, 18, 35 |
| others | a `35=n` message (`SESS`, `18:0:XM8NNITE384`) carries execid 00064703463GBYZ0 (last 120, cum 120, `FILLED`) and joins no order chain; the `AE` trade 830850681 splits an `EXEC` with `lastqty` 0 | rows 30, 33-34; `probe_parsed.tsv` row 127 |

Where today's walk is wrong, by the user's rule: (1) a duplicate execid counts twice (461 at #9/#10,
467 at #19/#20) - the capture's most visible duplication, 470 executed against an ordqty of 300 for
order 558 and 210 against 153 distinct for order 557; (2) a partial follower stating `lastqty` and
no `cumqty` gets neither `cumqty` nor `leavesqty` (`market.rs:1734`, pinned at
`tests/graph/market.rs:2199-2203`; the capture has no such row because every bridge or audit fill
states `14` or `151`); (3) fill 468 holds a derived cum 600 against a count of 472 - the count is
the order's, the 600 is the exchange-side level's; (4) order 9623's partial reads `TRADE`; (5) no
partial state in the capture sits with leaves 0 - every leaves-0 report already reads `FILLED`
through `native_derivations.rs:388-390` or a stated `39=2`, so the promotion rule changes **no
capture row** and is pinned by new tests.

What P12 changes on the capture, then: order 557 ends at cum 472 / leaves 128 `PARTIALLY_FILLED`
(#15 corrected from the derived 600 / 0 `FILLED`, one warning), its chain stays alive and #19 is
a restatement of fill 467 - counts nothing, starts no chain; #9 is a restatement of fill 461;
their `EXEC` twins #10 and #20 are restatements too (D45.8), so the walk yields 39 elements, 21
market data, 7 executions. These are the moves the user can see; the first draft moved none, which
the review read as the design doing the opposite of the instruction.

## D45.1 - the ledger: an order's chain counts the fills it saw once

Each live chain of kind **`MarketDataKind::Order`** - the user asked about orders; a quote's two
legs share one unsided chain (`14:0:Q-1`), so one ledger over both would sum the bid's and the
ask's hits against one `ordqty` and end the quote from one leg ("Put to the user" 10); `EXEC`/`TRAD`
are the fill (`facts::reports_fills`, `facts.rs:249-257`) and every other kind reports no fill -
carries a **ledger** in the walk:

```rust
// rust/market/src/graph/market.rs, beside `follow_hidden` (the consumed-since precedent, :686-713)
/// What a chain remembers of its fills: each fill counted by its execution identifier with the
/// quantity it counted, what they add up to over the chain's anchor, and the order quantity the
/// venue accepted. Bounded at `MAX_COUNTED` fills (4096, one venue's fills of one order in a day
/// with room; the 4096 of `Repeats` is the precedent): past it a fill is counted into `consumed`
/// and not remembered, warned once per kind, so a later copy of it would count again - said in
/// the warning.
#[derive(Clone, Debug, Default)]
pub(crate) struct Fills {
    /// Every fill counted - its `execid` and, where stated, its `secondaryexecid`, each with the
    /// quantity counted under it - sorted by id, so a report's is one binary search; a venue's
    /// sixteen-byte id is held inline (`yggdryl::Str`, `INLINE_CAPACITY`, `rust/src/string.rs:55`).
    /// Appended where the new id sorts last (a venue's ids ascend), inserted otherwise.
    counted: Vec<(Str, Decimal)>,
    /// The total the chain's first statement anchored on: what traded before the walk saw the
    /// chain - a capture opened mid-life - adopted once, then never from a report (D45.2).
    anchor: Option<Decimal>,
    /// `anchor + sum(counted)`, kept beside the vector so a step reads it.
    consumed: Option<Decimal>,
    /// The order quantity the venue accepted: stated by a report that is neither a request nor
    /// in the pending band, so a `35=G`'s `OrderQty` moves it only once the replace is accepted.
    ordqty: Option<Decimal>,
}
/// What a report states of its own fill before anything is followed: the facts the venue wrote.
#[derive(Clone, Debug)]
pub(crate) struct OwnFills {
    cumqty: Option<Decimal>,
    leavesqty: Option<Decimal>,
    ordqty: Option<Decimal>,
    hiddenqty: Option<Decimal>,
    execid: Option<Str>,
    secondaryexecid: Option<Str>,
    fill: Fill,
}
/// What the report is, as its holder reads it - the one classification the accounting trusts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Fill {
    /// A fill of `qty` under `execid`, the venue's first statement of it as far as the report says.
    New { execid: Str, qty: Decimal },
    /// A bust of the fill `refid` names (FIX `ExecType` `H`, `TRADE_CANCEL`): subtract that entry.
    Bust { refid: Str },
    /// A correction of the fill `refid` names (`ExecType` `G`, `TRADE_CORRECT`): that entry becomes `qty`.
    Correct { refid: Str, qty: Decimal },
    /// A fill the report states a quantity for but no identifier: counted nowhere, warned.
    Unidentified { qty: Decimal },
    /// Not a fill: an acknowledgement, a status reply, a restatement, a replace, a cancel, a
    /// done-for-day, a pending report, a leg report of a multi-leg - whatever `lastqty` it repeats.
    NotAFill,
}
/// The one accounting: `this` as the statement after the chain `fills` remembers, `kept` the live
/// element's hidden part before this statement; whether anything moved.
pub(crate) fn account_fills<E: Event + Operation + ?Sized>(
    this: &mut E, own: &OwnFills, kept: Option<Decimal>, fills: &mut Fills,
) -> bool
```

`Fill` is answered by a provided `Operation` hook, `fill_of(&self) -> Fill`, in the shape of
`follow_identity` and `note_conflict` (`market.rs`, the two walk hooks): the generic default
reads `lastqty` positive and `execid`, answering `New` where both stand, `Unidentified` where
the quantity stands alone, else `NotAFill` - it has no `ExecType` to read, and `State::is_execution`
cannot tell a fill from a status reply that reads `PARTIALLY_FILLED`. **`FixMsg` overrides it**
(`rust/fix/src/msg.rs`, beside `reports_execution`, `:2363-2381`), reading the message's own tags
and never the lifecycle state:

| `ExecType(150)` (and the pre-4.3 `ExecTransType(20)` where `150` is absent) | `Fill` |
| --- | --- |
| `F` Trade, `1` PartialFill, `2` Fill - what `reports_execution` answers true for - with `LastQty(32)` positive and `ExecID(17)` stated and not `0` | `New { execid, qty: lastqty }`; `qty` is `CumQty(14)` less the chain's `consumed` where `32` is absent (FIX: `LastQty` is the rise in `CumQty`), read by the accounting rather than the hook (`New` with `qty` 0 says "take the rise") |
| the same with `32` positive and no `17`, or `17=0` (FIX's own value for a status reply, `000000000.json:250`, never an identity) | `Unidentified { qty }` where `150` is `F`/`1`/`2`, else `NotAFill` |
| `H` TradeCancel (`20=H` too) with `ExecRefID(19)` | `Bust { refid: 19 }`; no `19`: `NotAFill`, warned once per kind (a bust naming no fill) |
| `G` TradeCorrect (`20=G`) with `19` and `32` | `Correct { refid: 19, qty: 32 }`; no `19` or no `32`: `NotAFill`, warned |
| `MultiLegReportingType(442)` `2` (a leg of a multi-leg) on any of the above | `NotAFill`: a leg's quantity is in leg units under its ratio and its report carries the parent's `OrderID` and side, so `identity_of` files it under the parent; only the multi-leg (`442=3`) or single-security (`442=1`, the capture's) report counts |
| `0` New, `3` DoneForDay, `4` Canceled, `5` Replaced, `6` PendingCancel, `A` PendingNew, `B` Calculated, `C` Expired, `D` Restated, `E` PendingReplace, `I` OrderStatus, `J`..`L`, and anything else, whatever `32` repeats | `NotAFill` |

`ExecRefID(19)` is read off the message by tag (`get_by_tag(19)`), so no dictionary key moves
(the hash, the dump and the census stand, D45.10); mapping it into `identifiers` as `execrefid`
is "Put to the user" 5.

`account_fills`, in this order, each step writing its target directly through the setter with
`overwrite` (so `FixMsg` marks `fact::FILLS`/`fact::STATE` stated and its row carries the fact,
`msg.rs:7130-7142`, `:6784-6787`; nothing reaches the wire, as `UPDATED` reaches none):

1. **Not an order's statement**: `this.marketdatakind() != Order` -> nothing moves, the ledger
   untouched. An execution's quantities are the fill it is (`facts.rs:819-826`); its `EXEC` chain
   is keyed by its own execid and counts nothing (P8 §6, `$S/p8/d41_design.md:142-150`).
2. **The accepted order quantity**: where `own.ordqty` is stated and the report's state is
   neither a request nor pending (`State::is_pending`, else the `Standing` `Fresh` band,
   `facts.rs:222-241`), `fills.ordqty = own.ordqty`; the chain's first statement anchors it
   whatever its state, so an order opened on its `35=D` has one. `leavesqty` is computed against
   `fills.ordqty`, never the `ordqty` `chain_operation` carried (`:1729-1733`), which a pending
   `35=G` moves before the venue accepts it.
3. **The fill**, by `own.fill` against the ledger:
   - `New { execid, qty }`: **Repeated** where `execid`, or `own.secondaryexecid`, is among
     `counted` (either id of either kind: a broker's `17=B1 527=X1` and the exchange's `17=X1` are
     one fill) -> nothing is added, nothing of the chain's quantities moves, nothing promotes; the
     report keeps its own stated `cumqty`/`leavesqty` on its row, warned under the kind where
     they disagree with the ledger's, else it takes the ledger's (a follower reporting no fill,
     `:1734-1739`). A Repeated report is otherwise a restatement of a fill the chain already
     stated. Else **New**: `qty` 0 reads the rise `own.cumqty - fills.consumed` (none where either
     is absent: then `Unidentified`); `consumed += qty` (`None + q = q` only where the anchor is
     settled in step 4 - the two are done together: a first statement anchors first, then adds);
     `counted` gains `(execid, qty)` and, where stated, `(secondaryexecid, qty)`, under the bound.
   - `Bust { refid }`: the entry under `refid` (either id) is removed and its quantity taken off
     `consumed`; no entry: warned under the kind, nothing moves. The bust's own `execid` is
     recorded with quantity 0, so its copy is Repeated.
   - `Correct { refid, qty }`: the entry becomes `qty`, `consumed` moves by the difference; no
     entry: counted as `New { execid: refid, qty }` (the original was before the capture), warned
     under the kind. Its own `execid` recorded at 0 as a bust's is.
   - `Unidentified { qty }`: counted nowhere, warned once per kind ("Put to the user" 9): FIX
     requires `ExecID(17)` on every report, and a fill the walk cannot tell from a resend is not
     added. The report's own `cumqty`/`leavesqty` stand where stated, else the ledger's.
   - `NotAFill`: nothing counted. A status reply, a restatement or a done-for-day repeating
     `LastQty` adds nothing.
4. **`cumqty`** (D45.2): the ledger's anchor is set once - on the chain's first statement that
   states `cumqty` (a capture opened mid-life: order 557's 340 beside a `lastqty` of 21 - the
   anchor is `stated - qty` where this statement is itself a New fill, so the fill is counted and
   the total agrees), or, while no anchor stands, on the first statement whose stated `cumqty`
   equals the count so far (the chain opened on an ack stating `14=0`: anchor 0). After that, on
   every statement the accounting moved or that states nothing: `this.set_cumqty(consumed, true)`.
   A stated `cumqty` that disagrees with `anchor + counted` **does not move the ledger**: a
   `CumQty` only ever rises but for a bust or a correction, both of which the ledger does itself,
   and a disagreeing statement is the other route's level (the capture's From Exchange frames)
   or a replay. It is warned once under the kind, the detail naming the chain's cross code, the
   execid, the counted and the stated value; the row keeps the venue's number where the report is
   not a New fill (a restatement is the venue's own words) and takes the ledger's where it is (the
   count is "the real consumed" the user asked for). `fills.consumed` is `None` before the first
   fill and the anchor - a chain of acknowledgements alone sets no `cumqty`.
5. **`leavesqty`**: on a statement the accounting moved, with `fills.ordqty` known ->
   `set_leavesqty(ordqty - cumqty, true)`, **floored at 0**; on a statement it did not move, a
   stated `leavesqty` stands and an unstated one is `ordqty - cumqty` where both are known. A
   count past `fills.ordqty` is an **overfill**: warned under the kind (chain, execid, counted,
   ordered), `leavesqty` 0 and `cumqty` the count, and **never promoted** (step 6) - the fills
   are the facts but the walk cannot say whether the order is done, so the partial state stands
   until a stated `leavesqty` or `OrdStatus` says filled. Written explicitly rather than left to
   `orders_filled`'s Working arm (`facts.rs:844-849`), which refills leaves only where it is none
   or equals `ordqty`. On an `ORDR`, `set_leavesqty` moves `quantity` with it (`facts.rs:807-815`,
   decision 27).
6. **The state** (D45.3): `fills.ordqty` known, `cumqty == ordqty` exactly (the count, after a
   New fill, a bust or a correction - never a Repeated report, never a stated `leavesqty` alone),
   the state partial-fill-like, and the live element's state not `PENDING_REPLACE` or
   `PENDING_CANCEL` -> `this.set_state(State::Filled)`.
7. **The hidden part**: where `own.hiddenqty` is unstated and `kept` (the live element's) is held,
   `set_hiddenqty(kept - delta, false)` floored at 0, `delta` what this step added to `consumed`
   (0 for a Repeated report, a bust's or a correction's signed move). `follow_hidden`
   (`market.rs:686-713`) ran inside `with_previous` with a follower's `cumqty` still `None` and
   fell back to this statement's `lastqty`, taking a Repeated fill off the hidden part a second
   time; the accounting rewrites what it wrote, so the one consumed-since rule is the ledger's
   where a ledger exists and `follow_hidden`'s for a bare `with_previous`.

Steps 3 to 7 run only where the element's standing after `with_previous` is Working or Fresh
(`state.is_live()`); an ended standing - a stated `FILLED`, a cancel - is left to `orders_filled`'s
own arms (`facts.rs:877-897`: a `FILLED` forces leaves 0), its fill counted (step 3) so a late
copy reads Repeated, and warned where the count disagrees (D45.3). Returns whether anything moved;
the walk finalizes once where it did (as `UPDATED`, `iterator.rs:1133`), so the element's
`hashcode` and `uuid` digest the facts it took.

`avgpx` is untouched: `chain_operation:1741-1746` carries it only where the two `cumqty` agree,
and `orders_filled:922-929` fills it from `lastpx` where all that traded is the last fill.

## D45.2 - anchor plus count: a stated total opens the chain, the count carries it

**Decided: the chain's first stated total is adopted as its anchor; after it the count is the
truth and a disagreeing statement is warned.** The first draft had the opposite - every stated
`CumQty(14)`/`LeavesQty(151)` standing over the count - and the review showed it would do nothing
the user asked for: both tags are required on every `ExecutionReport`, the parse fills `14` from
`38 - 151` where a frame omits it (`native_derivations.rs:311-322`, `put` `:199` marks nothing
derived), so on conforming traffic the ledger would never have written a `cumqty` and would only
have warned. Evidence, both ways:

- A capture starts mid-life: order 557's first walked report states cum 340 with `lastqty` 21
  (`ulbridge.log:6`) - 319 traded before; order 558's one report cum 300 with 235 (`:1-5`); order
  9623 cum 230,000 with 24,000. A count alone would say 21, 235, 24,000. **The anchor keeps the
  venue's running total where the walk has nothing of its own**: the chain's first statement.
- The stated numbers are not always the order's: execid 467 arrives with `151=0` on the From
  Exchange frame (`:35`, no `14`, no `39`, no `11`) and with `14=397 151=203` on the To Client
  frame (`:56`). One fill cannot leave one order both at 0 and at 203; the exchange-side frame
  states its own level. Under the first draft order 557 ended `FILLED` at a derived 600 (fill 468,
  `:73`, the same frame shape) while its distinct execids add up to 472. **After the anchor, the
  count is the order's and a statement that disagrees is warned, not adopted**: order 557 ends at
  cum 472 / leaves 128 `PARTIALLY_FILLED`, with no later report to close it - the capture ends
  there, and that is what the capture says of this order.

Adoption therefore happens exactly once per chain (step 4 of D45.1), and only upward from a
count of nothing. A per-tag derived mark on `FixMsg` - which would let the count outrank a `14`
the parse derived while a venue's own `14` stood - is not needed under this rule and is not added
("Put to the user" 1 says what it would change).

The warning is a `warned!` (deduplicated in `Repeats`, bounded at 4096 keys, `rust/src/logging/`
per AGENTS `logging/`), **keyed by the kind** (`walked_kind().as_str()`, as the conflict warning at
`iterator.rs:1073` is), the chain's cross code, the execid, the counted and the stated value in
the lazily built detail: `warn` builds the detail on the first occurrence only
(`warning.rs:55-72`) and counts the rest, so a bridge whose every From Exchange frame disagrees
costs one line and one atomic count per fill, no `String` per report (D45.9). It is **not** a
`FixAnomaly`: an anomaly is content a message digests (`msg.rs:7397-7398` records the conflict as
one under `crosscode`), and a walk's reading of the venue's numbers is not the message's fault
("Put to the user" 6).

## D45.3 - the state: a partial fill that has nothing left is filled

The partial-fill-like members - every `State` that says some of the order traded and it is still
working (`rust/src/state.rs:96-98`):

| Member | Code | Why it counts |
| --- | --- | --- |
| `IN_PROGRESS` | 4000 | "Working, with some of it done" |
| `PARTIALLY_FILLED` | 4001 | FIX `OrdStatus`/`ExecType` `1` |
| `TRADE` | 4002 | FIX `ExecType` `F`, "Trade (partial fill or fill)": what an order's report reads where its `OrdStatus` could not be read - order 9623's case, `probe_walked.tsv` row 3 |

Each becomes **`FILLED`** (8003, `:121`) and nothing else: one target, as FIX's `OrdStatus` `2` is
the one filled status. **The condition is the count's**: `fills.ordqty` known and `cumqty ==
ordqty` after a New fill, a bust or a correction (D45.1 step 6). Never promoted:

- a **stated `leavesqty` 0 with `cumqty` below `ordqty`**: the first draft read "leaves 0 because
  `cumqty >= ordqty`", which holds only for a leaves the count derived. A venue sends `39=1
  151=0 14=40 38=100` for the remainder of an IOC or FOK (`59=3`/`4`) expiring, and the capture's
  From Exchange frames state a child level's 0 - `orders_filled`'s Filled arm would then end the
  row `FILLED` at 40 of 100 and retire a live chain. Left as stated and warned under the kind: the
  venue states what the remainder became;
- an **overfill** the count alone produced (D45.1 step 5): the walk has two fills that may be one
  under two ids it was not told of, or a stale order quantity; the state stands and is warned;
- a **Repeated report**: a resend promotes nothing (the first draft adopted its stated `cumqty`
  and could promote `TRADE` to `FILLED` off a From Exchange copy of 467 carrying the derived
  600/0 - D45.8 says what a replay may do: nothing);
- while the live element is **`PENDING_REPLACE` or `PENDING_CANCEL`**: the quantity the venue
  will accept is not known;
- `TRADE_CORRECT` (4003) and `TRADE_CANCEL` (4004): a bust or a correction moves the ledger and
  never counts its own execid as a fill; a correction that brings the count to `ordqty` is the
  venue's to state as `FILLED` (and it does, with `39=2`). Because `STATUS_TAGS` reads `39` before
  `150` (`fix/state.rs:71`) and `39` is required, a conforming bust `150=H 39=1` reads
  `PARTIALLY_FILLED`, so the exclusion is **by `Fill`, not by state** - the first draft excluded
  the two states and the exclusion never took effect;
- `NEW`/`UPDATED`/`REPLACED` and the pending band, which report no fill (a `39=0` beside a positive
  `lastqty` is the venue's contradiction, left as stated); `TRADE_IN_CLEARING_HOLD` and the rest of
  rank 40, which are not fills of this order.

The reverse never happens: a `FILLED` whose count says leaves > 0 stays `FILLED` and is warned
(D45.2's disagreement), as FIX's `OrdStatus` `2` is terminal (`Standing::Filled`,
`facts.rs:877-897`). An order of `ordqty` 0, or a report whose count is empty, promotes nothing.
Promotion goes through `Event::set_state`, so the standing stamps `Filled` (`facts.rs:753-758`)
and `orders_filled` forces `leavesqty` 0 (`:884-887`; `cumqty` already equals `ordqty`). A
promoted `FILLED` is not alive (`is_alive`, `iterator.rs:1505-1512`), so `settle` retires the chain
exactly as a stated `FILLED` does - and leaves its fills in the tombstone (D45.8), so the next
copy of one of them starts nothing.

Precedent and shape: the walk already refines a state after `with_previous` - a `NEW` over a live
new-like predecessor becomes `UPDATED` (`iterator.rs:1127-1134`, `docs/types/enum/state.md:337`).
D45.3 is the second such refinement, written beside it.

## D45.4 - where the rule lives: the walk, one function, one FIX hook

**Owner: `rust/market/src/graph/iterator.rs`** holds the ledger (`Live` gains `fills: Fills`,
`:672-676`) and the tombstone (D45.8), and calls the one accounting function `account_fills` of
`rust/market/src/graph/market.rs` (beside `follow_hidden`, the only other "consumed since" rule,
`:686-713`). The one thing `rust/fix/` adds for the rule is `FixMsg`'s `fill_of` (D45.1): what a
report is, read off `150`/`20`, `17`, `19`, `32`, `442`, `527` - the FIX crate knows its wire,
the market crate does not. `FixCodec::lifecycle` walks `LifecycleMessage` through `EventIterator`
(`enrich.rs:509-626`, `:1070`), and `Walked`'s generic implementation over `Event + Operation +
Clone` (`iterator.rs:180-215`) answers the two new hooks - `walked_own_fills(&self) -> OwnFills`
and `walked_account_fills(&mut self, &OwnFills, kept, &mut Fills) -> bool` - for it, as
`MarketData`'s does through `as_event_operation_mut` (`:290-345`). Why not the alternatives:

- **Not `chain_operation` / `with_previous`** (`market.rs:1728`): a step sees `this` and
  `previous` only, and `previous` holds its own execid alone - `execid` has no follow flag
  (`000000000.json:243-248`) and P8's index keeps a per-report reference for the live statement
  only, released by the next (`iterator.rs:892-922`) - so a duplicate that is not the predecessor's
  twin (fill 457 arriving after 467) compares "different" and is added again. The user's "compare
  the execid" needs the chain's whole set, which only the walk holds. `with_previous` stays as it
  is; the one-step variant is "Put to the user" 2.
- **Not the FIX lifecycle** (`enrich.rs`): the rule is about any `Operation`'s chain
  (`OrderEvent`s walked without FIX pin it), and the FIX crate would duplicate the quantity
  arithmetic `facts.rs` owns. FIX contributes the reading of its own tags, nothing of the
  arithmetic.
- **Not a held fact on the element**: a `Vec` of counted execids on every row would grow with the
  chain, be digested or deliberately excluded, and be carried along the chain - the memory belongs
  to the walk, as the chain's names do (`iterator.rs:596-621`).

How `with_previous` is used - the step in `walk_source` (`iterator.rs:1115-1136`), amended. The
ledger is taken out of `Live` **before** the match, because `self.alive.get(&identity)` lends
`live` for the arm and a `get_mut` inside it would not compile:

```rust
let mut fills = self.alive.get_mut(&identity).map(|live| std::mem::take(&mut live.fills)).unwrap_or_default();
let own = element.walked_own_fills();                 // the venue's own numbers and the fill's ids, read before anything follows
let mut element = match self.alive.get(&identity) {
    Some(live) if live.arrived == arrived => element.walked_restating(&live.element),
    Some(live) if element.is_before(&live.element) => { /* :1106-1113, unchanged: counted nowhere */ .. }
    Some(live) => {
        let stated_new = element.walked_state() == Some(&State::New);
        let kept = live.element.walked_hiddenqty();   // what the iceberg kept back before this statement
        let mut element = element.clone().with_previous(&live.element).unwrap_or(element);
        let mut moved = element.walked_account_fills(&own, kept, &mut fills);  // D45.1-D45.3, live standings only
        if stated_new && live.element.walked_state().is_some_and(State::is_new_like) {
            element.walked_set_state(State::Updated); moved = true;            // today's refinement, :1127-1134
        }
        if moved { element.finalize(); }
        element
    }
    None => {
        element.walked_fill_execution();
        if self.tombstoned(&element, &own) { return element.walked_restating_ended(..); } // D45.8: an ended chain's fill, counted nothing, no chain started
        if element.walked_account_fills(&own, None, &mut fills) { element.finalize(); }   // a chain's first statement anchors
        element
    }
};
// fills travels to `settle(identity, &element, arrived, fills)`: kept on the re-inserted `Live` (:829-841),
// moved into the tombstone with the chain's base cross code on retire (:863), never cloned per step
```

`OwnFills` is read **before** `with_previous` because following carries the predecessor's `cumqty`
onto a follower reporting no fill (`:1734-1739`) and a FIX follower's `set_cumqty` marks it stated
(`msg.rs:7131`) - and because the generic `is_followed_identifier` carries every identifier type
but `mdentryrefid` (`market.rs:887-889`), so after the step an `OrderEvent` fill stating no
`execid` would wear its predecessor's and read Repeated, silently (D45.11 #6 pins the warning the
first draft would have failed). The report's `execid`, `secondaryexecid` and its `Fill` are the
venue's words alone. `settle` gains the `fills` argument: `Live { element, arrived, passed, fills }`
is rebuilt there (`:829-841`) and the retire branch moves the ledger's ids into the tombstone
(`:863-882`) - "released when the chain ends" in the walk's live state, remembered for the window
(D45.8).

The paths that count nothing, each already its own arm: a twin or a passed statement
(`walked_restating`, `:1100`, `:1105`: another statement of the same event), an element before the
live one (`:1106-1113`: follows nothing), a grid view (`walked_restamp`, a copy of the live element), an expiry the walk makes
(`expire`, `:1024-1052`: `walked_clear_fill` leaves no `lastqty`), an element the walk does not chain
(`is_walked` false: the `UKNW` and `SESS` rows, `probe_walked.tsv` rows 17, 21, 30), and now a
fill of an ended chain within the window (D45.8).

## D45.5 - what stays: `with_previous`, the parse, the dictionary

- `chain_operation` (`market.rs:1728-1783`) and its doc (`:1715-1727`) stand; the implication
  table's `:105` row stays true of a bare follower. `tests/graph/market.rs:2151` stands, its fill
  clause (`:2192-2203`) kept with a sentence: a bare `with_previous` holds no ledger; the walk
  counts. `follow_hidden` stands for a bare follower; under a walk the accounting rewrites its
  answer (D45.1 step 7).
- The parse derivations stand (`native_derivations.rs:243`, `:248`, `:256`): per message, FIX's own
  definitions, and what makes a lone `151=0` a `FILLED` on its own row - which the walk now reads
  as a disagreeing statement (D45.2) and retires only where the count agrees. Moving them into the
  walk would make a message's row depend on the walk that read it.
- `FIX:idmap` follow flags stand (P8 §7, decision 13): `execid` is not followed; the ledger
  replaces the need. `ExecRefID(19)` and `MultiLegReportingType(442)` are read by tag inside
  `FixMsg::fill_of`, so **no field and no `FIX:` key moves**: the dictionary hash, the crate dump
  and the census stand (D45.10). Whether the generic followed set should exclude `execid` too
  (its doc at `idtype.rs:343` says "a per-report reference") is "Put to the user" 11; reading
  the ids before `with_previous` makes the accounting right either way.
- The split (`fix/market.rs:1260-1319`) stands: the `EXEC` leaf of a duplicate report is still
  split; the walk reads it as a restatement where its execid's chain ended within the window
  (D45.8), yielding it once.

## D45.6 - decision 27: one definition of `quantity`

| Kind (`reports_fills`) | `quantity` is | Today | Change |
| --- | --- | --- | --- |
| `ORDR`, `QUOT`, a book entry (`false`) | what is still available: an order's `leavesqty`, a quote leg's or an entry's remaining size | `facts.rs:807-815` (`leaves_moved`): `quantity` follows `leavesqty`; a quote's legs their own | none |
| `EXEC`, `TRAD` and their batches (`true`) | what executed: `lastqty` | `None` unless stated (`rust/fix/tests/root/enrich.rs:2520-2521`, `:2528`: `execution.get_quantity() == None`; `tests/graph/market.rs:2093-2110`: "no quantity of its own") | `quantity` **follows `lastqty`** on a fills-reporting kind |

Implementation, one owner: `MarketFacts` (`facts.rs:43`) gains `fill_moved(before)` beside
`leaves_moved` - where `reports_fills(self.kind)` and `lastqty` moved, `follow(&mut self.quantity,
before, after)` then `quantity_moved` (`:548`), never the `Fact::Quantity` a setter stated
(`:1026-1036`); `set_lastqty` (`:1228-1233`) calls it; `set_marketdatakind` re-derives the quantity
by the kind's rule once the kind is stamped, because the FIX split refiles a clone `EXEC` after its
facts are set (`fix/market.rs:1324-1333`, `settle_refiled`) and refiles the report `ORDR`
(`:1283-1290`). A sided execution's `quantity` then quotes its side's `bidqty`/`askqty` as every
sided element's does (`market.rs:96`). The Arrow rows (`graph/arrow.rs`), the book fold and the docs
follow the fact, nothing of their own. Every `EXEC`/`TRAD` row's `uuid` and `hashcode` move with
it, and **so does every book that records an execution among its `events`**: a book digests its
events after its delta (`python/tests/test_fix.py:786-789`, `rust/fix/tests/root/ulbridge.rs:1838-1848`),
so its `hashcode` follows its executions' - and a `BookIterator::with_filter` predicate over
`quantity` now matches an execution by what it executed. The pins this moves are in D45.10.

## D45.7 - a bridge's `ORDSTATUS=partfilled` (order 9623)

A code-set word is read by the set under the crate's fold, with the refusal naming the set
(AGENTS §1, Ownership); `from_status` (`rust/fix/src/state.rs:44`) stays the wire-code reader of
tags 39 and 150 and gains no fallback - the first draft's `.or_else(State::from_spelling)` would
have been a second reader, accepting every `State` spelling on a FIX status tag. The bridge's word
is resolved where `filled` already is: the code set's reading by name
(`FixCodeSet::code_by_name`, `rust/fix/src/codes.rs:1180`, which folds `filled` onto the
`OrdStatus` member `Filled` and writes `39=2`, snapshot `:704`). `code_by_name` gains the one
alias table of the bridge's short words, in `codes.rs` beside it, read after the member names:
`partfilled`/`partfill` -> `PartiallyFilled`, `pendnew` -> `PendingNew`, `pendcancel` ->
`PendingCancel`, `pendreplace` -> `PendingReplace`, `doneday` -> `DoneForDay` - the same words
`rust/src/state.rs:352-360` already knows as `State` spellings, resolved here to the set's member
so the row carries `39=1` and `from_status` reads it as it reads any frame. A word neither the
members nor the table spell is refused naming the set, as today. Nothing in the dictionary moves
(the alias table is code, not a `FIX:` key), so the hash stands.

Evidence: the bridge writes `ORDSTATUS=partfilled` (`ulbridge.log:124`), the merged message reads
`TRADE` off `150=F` where its FIX frame reads `PARTIALLY_FILLED` (`probe_parsed.tsv` rows 132 vs
134). In P12 because the user's ask is the state of partially filled orders and this is one
misread; its pins are small (D45.10) and the promotion (D45.3) covers `TRADE` anyway, so the two
land together or the second is "Put to the user" 7.

## D45.8 - out of order, replays, twins, late copies: stable

- **Arrival order**: the walk sorts a source it was not told is sorted (`EventIterator::new`,
  `iterator.rs:690-700`) and reads a sorted one in its order; the ledger is fed in instant order, so
  the same set of fills gives the same `cumqty`/`leavesqty`/state whatever order they arrived in.
  Pinned by a test feeding the same fills in two orders to an unsorted walk (D45.11 #3).
- **An element before the live one** on a walk told sorted (`:1106-1113`) follows nothing and is
  counted nowhere - today's contract for a source that lied about its order.
- **A replay of a fill** - the same execid at another instant, content differing (a hop's
  `SendingTime`) - is `Repeated`: adds nothing, moves no quantity of the chain, promotes nothing,
  adopts nothing into the ledger. Where it states no `cumqty` it takes the chain's; where it
  states one that disagrees it keeps it on its row and is warned. The first draft adopted a
  Repeated report's stated `cumqty`, and a replay of E1 (cum 40) sorted after E2 (cum 70) - exactly
  what decision 21 does to a date-only `60` frame dated by its `SendingTime` - would have set the
  ledger back to 40 and a following unstated E3 of 30 to 70 instead of 100. Pinned (#2, #10).
- **A twin** (same `uuid`) restates (`:1105`) and never reaches the accounting; the window
  then yields it once (`enrich.rs:1031-1061`).
- **A late copy of a fill after its chain ended** (#9, #19 in the capture): the walk keeps a
  **tombstone** of ended chains' fills - `(base cross code, counted id) -> the uuid of the
  statement that counted it`, every id of the chain's ledger moved there when `settle` retires the
  chain - for the codec's dedup window of event time (`FixCodec::DEFAULT_DEDUP_WINDOW_MS`, 60 s,
  `codec.rs:678`; the walk takes the span as the codec does, a nonpositive span remembering none),
  swept as `enrich.rs`'s `Window` sweeps (`:1004-1061`: a watermark, a table swept past a size
  bound, nothing per element). A statement whose `Fill` names a tombstoned id under its base
  cross code is **a restatement of the ended chain**: it counts nothing, starts no live chain
  from a count, and is marked another statement of the fill (`rekeyed` to that uuid, as a passed
  statement is at `:1100`), so the window yields it once. The first draft let such a copy start a
  fresh chain, and a copy stating `lastqty` and no `14`/`151` (the bridge row `:36` without
  `LEAVESQTY`) would have counted it as a live order's first fill: order 557's 467 copy as cum 57
  / leaves 543 alive - and `ORDR` is a booked kind, so the book fold would have rested 543 on the
  bid until expiry. The bound: at most the ids of the chains ended within the window, 24 bytes
  and one `Uuid` each, swept with the watermark.
- **The `EXEC` twins** (#10, #20): an `EXEC` chain is keyed by its execid and ends at once
  (`FILLED`, `market.rs:932-935`), so its one statement goes into the tombstone as the chain's
  own - and a second `EXEC` under the same execid within the window is, by the same rule, a
  restatement of that execution, yielded once. This is P12b's rule (the first draft deferred it)
  brought in because the tombstone gives it for free and because it is the capture's most visible
  duplication: 41 / 23 / 9 become **39 / 21 / 7**, the books' events deduplicated, the snapshot's
  `[010]` and `[020]` the twins'. It is the one change of the capture's counts; the pins are in
  D45.10 and it is "Put to the user" 4.
- **The window** (`enrich.rs`) is untouched; it keys by `uuid` and knows no execid. The tombstone
  is the walk's, under the same span, so a restatement the walk marks is one the window drops.

## D45.9 - the cost

Per walked `ORDR` statement: one `fill_of` (a `FixMsg`'s reads four tags by `get_by_tag`, a
hash lookup each), two `Identifiers::get` (`execid`, `secondaryexecid`: binary searches over the
report's few identifiers), one or two binary searches over the chain's `counted`, one tombstone
lookup per chain-starting statement (a `HashMap` probe), two or three `Decimal` operations, the
setters it already pays for a carried `cumqty`. **Allocation**: a New fill pushes one `(Str,
Decimal)` into `counted` - inline for a venue id of at most `INLINE_CAPACITY` bytes (the capture's
are sixteen), heap only past it - appended where it sorts last (a venue's ids ascend; the
capture's do) and inserted otherwise; the `Vec` grows by doubling, so a chain of N fills allocates
O(log N) times for its ledger and nothing per report. A warning allocates its detail `String` on
the first occurrence of its `(what, kind)` key only. **Bounds, stated**: the ledger is per live
chain, at most `MAX_COUNTED` (4096) entries of 40 bytes each, released into the tombstone when
the chain ends; the tombstone holds the ids of the chains ended within the window, swept with the
watermark past `Window`'s size bound; every other structure of the walk is unchanged in bound (P8
§8). No clone of the ledger per step: `mem::take` out of `Live`, handed back at `settle`.

Pins (D45.11 writes the new rows red first, `rust/market/tests/allocations.rs` beside
`a_lifecycle_walk_step_allocates_alike_at_8_and_1024_live_chains`, `:436-477`):

| Row | Claim |
| --- | --- |
| `a_counted_fill_allocates_alike_at_8_and_1024_fills_already_counted` | the step of a new fill on a chain holding 8 counted fills and on one holding 1024 differ by at most one allocation (a doubling), at two corpus sizes as every `allocations` row |
| `a_repeated_fill_allocates_nothing_beyond_the_step` | the same step with an execid already counted allocates what the step allocated before P12 (`counted_once_and_repeated`) |
| `an_ended_chains_fill_allocates_nothing_beyond_the_step` | a late copy of a tombstoned fill: the probe and the rekey, no chain, no ledger |
| `a_lifecycle_walk_step_allocates_alike_at_8_and_1024_live_chains` (`:436`) | **stands**: its reports state `PARTIALLY_FILLED` with execids and no `lastqty`, so `fill_of` answers `NotAFill` and nothing is counted |
| `rust/fix/tests/allocations.rs` `FIX_PIPELINE_COSTS` lifecycle 22/16/16 (`:1525-1555`, decision 15) | **to re-read, not re-pinned blind**: its three lines are terminal (`FILLED`); the report's fill is now counted into a ledger that goes straight to the tombstone (one `Vec` of one entry, one tombstone insert) - if the count moves by those two, the sentence says so; a move by more is a defect |
| `cargo bench -p yggdryl-fix --bench fix -- lifecycle --quick` (the `lifecycle` benches `$S/p8/d41_design.md:174-176` names) | direction only, before and after; no page states a number |

## D45.10 - the pins that move, each accounted

| Pin | Moves | Why |
| --- | --- | --- |
| `rust/fix/tests/root/equivalence.snapshot` lifecycle `[001]` (`:20915-20943`): `state` `:20923` `TRADE` -> `PARTIALLY_FILLED`, its `hashcode` and `uuid`; `[002]`'s `srcuuids` (its source is `[001]`, `fix/market.rs:1336-1341`) and whatever of its identity digests them; the parsed rows of the same message and its twins (`probe_parsed.tsv` rows 132-137: the `.911` statements and their `EXEC`s; find them with `grep -n "20260814_CQ9_LIAPUS_9623" rust/fix/tests/root/equivalence.snapshot`) - the parsed rows now carry `39=1` | state, hashcode, uuid, srcuuids, `fixentries` | D45.7: `39=partfilled` read as the set's `1` |
| the same snapshot, every `EXEC` and `TRAD` row stating a `lastqty`: lifecycle `[002]`, `[007]`, `[010]`, `[012]`, `[014]`, `[016]`, `[020]`, `[028]` (a sided execution: `quantity` and `bidqty`), `[033]`, `[034]` (`lastqty` 0: `quantity` 0, side `UKNW`, no leg), and the parsed rows they and their twins come from | `quantity` (and `bidqty`/`askqty`), `hashcode`, `uuid` (`feed_market` feeds `quantity`, `market.rs:1144-1145`) | D45.6 |
| the same snapshot, the `ORDR` lifecycle rows of order 557: `[015]` (`:22930`/`:22946`) `FILLED` 600 / 0 -> `PARTIALLY_FILLED` 472 / 128, `quantity` 128, its `hashcode`/`uuid`; `[019]` (`:23573`/`:23589`) a fresh `FILLED` chain -> a restatement of `[013]` (its `uuid`), so the walk yields it once with `[020]`; `[011]` and `[013]` byte for byte (340 anchors, 397 agrees) | state, cum, leaves, quantity, hashcode, uuid, prevuuid of what followed | D45.2 (anchor plus count), D45.8 (the tombstone) |
| the same snapshot, `[009]`/`[010]` (order 558's hop and its `EXEC`): restatements of `[006]`/`[007]`, yielded once | the rows go, the counts move | D45.8 |
| the same snapshot, every `ORDR` lifecycle row of a fill the count agrees with, and every row of orders 541, 559, 9623 but D45.7's | **stand**; a move is a defect to read before the snapshot is written | - |
| `rust/fix/tests/root/enrich.rs:2495-2530` `a_fill_split_report_keeps_its_leaves_as_its_quantity`: `:2520` `execution.get_quantity() == None` -> `Some(40)`, `:2521` `get_bidqty() == None` -> `Some(40)` (side 1), `:2528` `messages[1].get_quantity() == None` -> `Some(60)` (side 2: `askqty` 60) | re-pinned with the sentence of D45.6 | D45.6 |
| `rust/market/tests/graph/market.rs:2093-2110` `an_execution_quantity_is_never_its_orders_leaves` | stands (no `lastqty` set), gains the clause: `set_lastqty(40)` -> `quantity` 40, `set_quantity(50, true)` stands over it | D45.6 |
| `node/tests/fix.test.js:3924-3925` `buy.quantity`/`sell.quantity` `null` -> the two sides' `lastqty` | re-pinned | D45.6 |
| `rust/fix/tests/root/state.rs:13-19` table | gains `(39, "partfilled", None)` - `from_status` reads wire codes alone - and the code set's reading pinned in `rust/fix/tests/root/codes.rs`: `code_by_name("partfilled")` is `1`, `"nonsense"` is none | D45.7 |
| `python/tests/test_fix.py:723-754` (144/151/**41 -> 39**/42/**23 -> 21**/**9 -> 7**+14, 13 books), `rust/fix/tests/root/ulbridge.rs:490`, `:611`, `:1506` (41 -> 39), `:1530` (23 -> 21); `node/tests/fix.test.js:3895` where it states a count | re-pinned with D45.8's sentence | D45.8: #9/#10 and #19/#20 restate |
| every book hash pinned over a book that records an execution among its `events` - `rust/fix/tests/root/ulbridge.rs` (find them: `grep -n "get_hashcode(), [0-9_]*)" rust/fix/tests/root/ulbridge.rs`), `python/tests/test_fix.py:809` and its neighbours (the last book's, the reject's, holds no execution and **stands**), `node/tests/fix.test.js` where a book hash is stated, the snapshot's book rows | re-pinned with D45.6's sentence where the book holds an execution: its `events` digest `quantity` | D45.6 |
| `python/tests/test_fix.py:853`, `:931` (`TRADE` on a `TRAD` row) | **stand** | the `TRAD` kind's state is untouched by D45.7 (`150=F` on a trade with no `39`) |
| `rust/fix/tests/root/enrich.rs:1553`, `:1604` (redelivered acknowledgements and fills restate), `:2416` (a canceled report derives no `cumqty`), `:2474` (pending leaves), `rust/fix/tests/root/market.rs:230`, `:283`, `:405`, `:1128` (the split and its states), `rust/market/tests/graph/market.rs:1697`, `:1959`, `:2010` (the standing fills) | **stand** | restating counts nothing; the parse is untouched; the standing is untouched |
| `.api-inventory.txt:1626` (`following_operation`), `:1082` (`FixMsg`), `:1722` (`OperationEvent`) | the `EventIterator` row gains the sentence "counts each order chain's fills once by execid over the chain's first stated total, fills `cumqty`/`leavesqty` from the count, reads a partial fill whose count reaches its order quantity as `FILLED`, and reads a fill of a chain ended within the window as a restatement"; `Operation` gains `fill_of`; `rust/fix/src/codes.rs`'s `FixCodeSet::code_by_name` row gains the alias sentence | D45.1, D45.4, D45.7 |
| the dictionary hash, the crate dump, the census (`rust/fix/tests/root/store.rs`) | **stand**: no field, no `FIX:` key moves (`19` and `442` are read by tag, the alias table is code) | - |

The snapshot is regenerated **once**, after D45.1-D45.8 are all in, by the equivalence test's
writer (`rust/fix/tests/root/codec.rs:3579-3595`, `YGGDRYL_FIX_EQUIVALENCE_WRITE=1`), its diff read
row by row against this table before it is committed.

## D45.11 - tests, red first

In `rust/market/tests/graph/iterator.rs` (the rule is the walk's; `OrderEvent` fixtures as
`allocation_live_order` builds them, `tests/allocations.rs:410-421`; `dec()` as `graph/market.rs`
spells it; each fixture states its `execid` on its own row and nothing of its predecessor's, the
ids read before `with_previous`), each written and run red before the code:

| # | Test | Red because |
| --- | --- | --- |
| 1 | `a_partial_fill_then_one_to_zero_reads_filled`: `O-1` ordqty 100 `NEW` (`14=0` anchors 0); report execid `E1` lastqty 40 `PARTIALLY_FILLED`, no `cumqty` -> cum 40, leaves 60, `PARTIALLY_FILLED`, `quantity` 60; report `E2` lastqty 60 `PARTIALLY_FILLED` -> cum 100, leaves 0, **`FILLED`**, `walk.alive()` empty, `prevuuid` the first report's | today cum `None`, leaves `None`, state `PARTIALLY_FILLED`, chain alive |
| 2 | `a_repeated_execid_counts_nothing`: `E1` 40, then `E1` again at a later instant (lastqty 40, no `cumqty`) -> cum 40, leaves 60, `PARTIALLY_FILLED`; then `E2` 60 -> `FILLED` | today the repeat's cum `None` |
| 3 | `fills_count_alike_whatever_order_they_arrive_in`: an unsorted walk (`sorted = false`) fed `E2`, `E1`, `E1`-again lands on the same (cum, leaves, state) as `E1`, `E2` | - (the sort already does it; pins the claim) |
| 4 | `an_overfill_floors_leaves_at_zero_stays_partial_and_is_warned`: ordqty 100, `E1` 60, `E2` 60 -> cum 120, leaves 0, **`PARTIALLY_FILLED`** (never promoted from an overfill), chain alive, one warning under the kind (asserted as `a_conflict_is_warned_and_files_no_name_another_chain_holds` asserts its, `iterator.rs` per `$S/p8/d41_design.md:189`) | today leaves `None` |
| 5 | `a_stated_total_anchors_the_chain_and_the_count_carries_it`: order 557's shape - `E1` lastqty 21 stated cum 340, ordqty 600 -> 340/260, no warning (the anchor 319 + 21); `E2` 57 -> 397/203 (stated 397 agrees, no warning); `E3` 75 stated cum 600 leaves 0 `FILLED` -> the row reads **472 / 128 `PARTIALLY_FILLED`**, chain alive, **one** warning naming `472` and `600` | the count |
| 6 | `a_fill_stating_no_execid_counts_nothing`: lastqty 30, no `execid` -> `cumqty` the chain's (40), leaves 60, one warning; the fixture's predecessor states `E1`, so the test also pins that following carried nothing the accounting read | today cum `None` |
| 7 | `a_trade_state_report_with_nothing_left_reads_filled` (`TRADE` -> `FILLED` at the count), `a_filled_order_whose_count_falls_short_stays_filled` (never the reverse, warned), `a_stated_leaves_of_zero_below_the_order_quantity_is_not_filled` (`39=1 151=0 14=40 38=100`: stays `PARTIALLY_FILLED` at 40/0 as stated, warned, alive) | the first |
| 8 | `an_execution_chain_counts_no_fill`: two `ExecutionEvent`s under one execid chain keep their own `lastqty`, `cumqty` untouched | - (pins D45.1 step 1) |
| 9 | `a_fill_counted_by_the_walk_is_the_chains_first_where_it_starts_one`: a first report stating lastqty 30 and no `cumqty` -> cum 30, leaves `ordqty - 30` | today `None` |
| 10 | `a_replayed_fill_sorted_late_adopts_nothing`: `E1` (lastqty 40, cum 40), `E2` (30, cum 70), `E1` replayed at a later instant (cum 40), `E3` (30, unstated) -> **100 / 0 `FILLED`**; the replay's row keeps 40, warned | the first draft's adoption |
| 11 | `a_bust_subtracts_the_fill_it_names` and `a_correction_replaces_it`: ordqty 100, `E1` 40, `E2` 30, then an `OrderEvent` whose `fill_of` is `Bust { refid: E2 }` (the generic hook cannot say so, so the fixture is a `MarketData::Fix`-free test type implementing `Operation` with `fill_of` overridden, as the market tests build their own fixtures) -> cum 40, leaves 60, alive, not `FILLED`; then `Correct { refid: E1, qty: 50 }` -> 50 / 50; a bust naming no counted fill: warned, unchanged | today counted as fills |
| 12 | `a_status_reply_repeating_the_last_quantity_counts_nothing`: `E1` 40, then a report with `fill_of` `NotAFill` stating lastqty 40 `PARTIALLY_FILLED` -> 40 / 60, no change | today's draft counted it |
| 13 | `a_pending_replace_moves_no_accepted_quantity_and_promotes_nothing`: 100, `E1` 40, a `PENDING_REPLACE` stating ordqty 50, `E2` 10 unstated -> cum 50, leaves 50, **alive**, `PARTIALLY_FILLED`; then a `REJECTED` replace (`35=9`'s state) and `E3` 50 -> 100 / 0 `FILLED` | the first draft promoted at the pending 50 |
| 14 | `a_fill_under_two_identifiers_counts_once`: `E1` with `execid B1` and `secondaryexecid X1` lastqty 40, then a report `execid X1` lastqty 40 -> 40 / 60, Repeated | today counted twice |
| 15 | `a_late_copy_of_an_ended_chains_fill_starts_no_chain`: `E1` 40, `E2` 60 -> `FILLED`, chain retired; a copy of `E1` (lastqty 40, no `cumqty`) 10 s later -> a restatement (its `uuid` the first `E1`'s), `walk.alive()` empty; the same copy 61 s later -> a fresh chain (the window passed), its own first fill | today a fresh live chain at 40 / 60 |
| 16 | `a_repeated_fill_leaves_the_hidden_part_where_it_was`: an iceberg `hiddenqty` 100, `E1` 40 -> 60; `E1` again -> 60 (not 20) | `follow_hidden` takes it twice |
| 17 | `a_two_leg_report_counts_only_the_multileg_statement`: a fixture whose `fill_of` answers `NotAFill` for a leg and `New` for the multi-leg report -> one count | - (pins the hook's contract; the FIX reading of `442` is `enrich.rs`'s test) |
| 18 | `a_quote_chain_counts_no_fill`: a `QuoteEvent` hit on its bid with lastqty, no promotion, no ledger | - (pins the scope: `Order` alone) |

`rust/market/tests/graph/market.rs`: `an_operation_follower_takes_the_cumulative_fill_of_its_chain`
(`:2151`) keeps `:2199-2203` with the sentence "a bare follower holds no ledger: the walk counts";
`an_execution_quantity_is_never_its_orders_leaves` (`:2093`) gains D45.6's clause; `fill_of`'s
generic default pinned (`New`, `Unidentified`, `NotAFill`).
`rust/market/tests/graph/facts.rs` (mirrors `facts.rs`): `set_lastqty` on an `EXEC` holder moves
`quantity`, on an `ORDR` holder not; `set_marketdatakind` re-derives it.

`rust/fix/tests/root/msg.rs` (mirrors `msg.rs`): `fill_of` over the wire - `150=F 17=E1 32=40`
`New`; `150=I 17=0 32=40` `NotAFill`; `150=H 19=E2 39=1` `Bust`; `150=G 19=E1 32=50` `Correct`;
`150=F 442=2` `NotAFill`; `150=F 32=40` and no `17` `Unidentified`; `20=H 19=E2` (no `150`) `Bust`.
`rust/fix/tests/root/enrich.rs` (mirrors `enrich.rs`, the lifecycle door): a capture - `35=D`
ordqty 100; ack `150=0 39=0 37=O1 14=0 151=100`; fill `17=E1 150=F 39=1 32=40 31=10 14=40
151=60`; the same fill redelivered at a later `52` (a hop) -> cum 40, counts nothing, yielded
once (a restatement of the first, D45.8); a status reply `150=I 39=1 17=0 32=40 14=40 151=60` ->
unchanged; a bust `150=H 39=1 17=E3 19=E1 32=40 14=0 151=100` -> cum 0, leaves 100, alive; fill
`17=E4 32=100 14=100 151=0 39=2` -> `FILLED`, the chain ended; a copy of `E4` 5 s later with
`151=0` and no `14` -> a restatement, nothing alive; a later `35=D` under `11=A1` starting afresh.
Then order 557's four frames from the capture through `lifecycle` -> 472 / 128 `PARTIALLY_FILLED`,
one warning. And D45.7: a bridge row with `ORDSTATUS=partfilled` and `EXECTYPE=trade` reads
`PARTIALLY_FILLED` with `39=1` in its `fixentries`.
`rust/fix/tests/root/codes.rs` gains the alias rows; `rust/fix/tests/root/state.rs:13-19` the
`None` row of D45.10.

Bindings (parity, no new door): `python/tests/test_fix.py` gains the same capture beside `:723`
(`codec.lifecycle`, `msg.cumqty`, `msg.leavesqty`, `msg.state`) and re-pins the counts (39/21/7)
with D45.8's sentence; `node/tests/fix.test.js` re-pins `:3924-3925` and `:3895`'s count and keeps
green (no JavaScript door is added, AGENTS §4).

## Implementation plan

One commit, phases in layer order, each a partition of files by path; a phase is settled by its
build check and the one suite it touched; the whole run leads the chain when the last phase holding
the cargo lock is settled. `cargo fmt --all` once after the last edit. The change is under
`rust/market/src/` and `rust/fix/src/`, so CI runs the `market` and `fix` leaves, the bindings and
the CLI (AGENTS §2, the leaf table).

| Phase | Files (disjoint) | Smoke (exact) | Pins expected to move | Must not move |
| --- | --- | --- | --- | --- |
| 1 - market core | `rust/market/src/graph/market.rs` (`Fills`, `OwnFills`, `Fill`, `account_fills` beside `follow_hidden`; `Operation::fill_of` beside `follow_identity`/`note_conflict`; the implication table `:105-109` gains the ledger row and D45.6's `quantity` row), `graph/facts.rs` (`fill_moved`, `set_marketdatakind` re-deriving `quantity`), `graph/iterator.rs` (`Live.fills`, the tombstone, the two `Walked` hooks and both impls, `walk_source` as D45.4, `settle(.., fills)`, the module doc `:415-470` gains the paragraph); tests first: `rust/market/tests/graph/iterator.rs` (D45.11 #1-#18), `graph/market.rs` (`:2093`, `:2192-2203` sentences, `fill_of`), `graph/facts.rs`, `tests/allocations.rs` (the three rows of D45.9) | `cargo check -p yggdryl-market --all-targets`; `cargo test -p yggdryl-market --test graph iterator`; `--test graph market`; `--test graph facts`; `--test allocations fill`; `--test allocations lifecycle`; `RUSTDOCFLAGS='-D warnings' cargo doc -p yggdryl-market --no-deps` | `graph/market.rs:2093` clause, the three new `allocations` rows | `a_lifecycle_walk_step_allocates_alike...` (`:436`), every other `graph` test |
| 2 - FIX | `rust/fix/src/msg.rs` (`impl Operation for FixMsg`: `fill_of` beside `reports_execution`, `:2363-2381`), `rust/fix/src/codes.rs` (the alias table beside `code_by_name`, `:1180`; D45.7); `rust/fix/src/state.rs` untouched; tests: `rust/fix/tests/root/msg.rs`, `root/codes.rs`, `root/state.rs`, `root/enrich.rs` (the lifecycle captures, the bridge word; `:2520-2528` re-pinned), `root/market.rs` (run, not edited), `root/ulbridge.rs` (41 -> 39, 23 -> 21, the book hashes) | `cargo check -p yggdryl-fix --all-targets --keep-going --message-format=short`; `cargo test -p yggdryl-fix --test root msg`; `--test root codes`; `--test root state`; `--test root enrich`; `--test root market`; `--test root ulbridge`; `--test allocations`; `cargo test -p yggdryl-fix --doc` | `enrich.rs:2520-2528`, `ulbridge.rs:490`/`:611`/`:1506`/`:1530` and its execution-holding book hashes; `FIX_PIPELINE_COSTS` lifecycle read (D45.9), re-pinned only by the two the ledger and the tombstone account for | the dictionary hash and the census (`--test root store`) |
| 3 - the snapshot, once | `rust/fix/tests/root/equivalence.snapshot` | `YGGDRYL_FIX_EQUIVALENCE_WRITE=1 cargo test -p yggdryl-fix --test root equivalence` (the test of `codec.rs:3579`), then `git diff --stat` and the row-by-row read against D45.10, then `cargo test -p yggdryl-fix --test root codec` | the rows D45.10 names, no other | every `ORDR` fill row the count agrees with; the dictionary hash and the census |
| 4 - Python | `python/tests/test_fix.py` (the lifecycle parity test, the counts, the book hashes); `python/src/`, `python/yggdryl/` untouched (no door) | `VIRTUAL_ENV=python/.venv python/.venv/bin/python -m maturin develop -m python/Cargo.toml`; `python/.venv/bin/python -m pytest python/tests/test_fix.py -x -q`; the `mypy --strict` line of AGENTS §3 | `:723-754` (39/21/7), the book hashes holding an execution | `:809` (the reject's book holds none), `:853`, `:931` |
| 5 - Node kept green | `node/tests/fix.test.js:3924-3925`, `:3895`, any book hash it states | `npm run --prefix node build:debug`; `node --test node/tests/fix.test.js` | those pins | the loader and declarations (`git diff --exit-code -- node/index.js node/index.d.ts`) |
| 6 - docs and skills | `docs/fix/lifecycle.md` (the `Follows` row `:13` gains the count; a section "Fills are counted once along a chain" after `:357`, **tabs Rust and Python only** over the capture of D45.11 - the request does not name Node, and AGENTS §4 says such a change adds no JavaScript tab, though Node carries `lifecycle` and its existing tabs stand), `docs/graph/market.md` (`:60-72` the standing table gains "a partial fill whose count reaches its order quantity reads `FILLED` along a walk"; a `quantity` row per kind, D45.6; `:303` Following and merging; the `fill_of` hook beside `follow_identity`), `docs/graph/execution.md`, `docs/graph/trade.md` (`quantity` is `lastqty`), `docs/graph/schemas.md` (the `quantity` column's sentence), `docs/graph/book.md` (a book's hash follows its executions' `quantity`), `docs/types/enum/state.md` (`:282` the bridge word through the code set; beside `:337` a "Filled by its fills" paragraph), `skills/yggdryl-fix/SKILL.md:21-24` and `references/{rust,python}.md`, `skills/yggdryl-market-data/SKILL.md` | `python -m mkdocs build --strict --config-file mkdocs.yml`; the three `python scripts/check_docs_examples.py --lang {rust,python,javascript}` as chain steps (JavaScript for the existing blocks) | - | - |
| 7 - inventories | `.api-inventory.txt` (the `EventIterator` row's sentence, `Operation::fill_of`, the `code_by_name` sentence) | `python scripts/check_api_inventory.py` | - | - |
| 8 - the chain | `$S/logs/chain.sh` adapted: `cargo test -p yggdryl-market --all-targets --all-features --no-fail-fast`, the same `-p yggdryl-fix`, `-p yggdryl-cli --all-targets`, clippy `-D warnings` both lanes on the two crates, `cargo doc -D warnings`, the rustdoc examples, `pytest python/tests`, `npm test --prefix node`, `node scripts/build_docs_fix.js --check`, `build_docs_playground.js --check`, mkdocs, the inventories, the three example runners | one background script, one log, read once | the pins of phases 1-3 only | everything else |

Then the commit (one, its message ending with the two attribution lines the session's reminder
names and no other `Co-Authored-By`), one push, the CI run read to `CI result`, and the results
commit (`DESIGN.md` gains D45's row and section; `$S/user_decisions.md` 26-27 marked implemented;
`MARKET_SPLIT_NEXT.md`'s `State`/`Checks`/`Next`).

## Put to the user (interpretations taken; say if another was meant)

1. **Anchor plus count** (D45.2): the chain's first stated `CumQty` is adopted as what traded
   before the walk saw the chain (order 557 opens at 340 beside a 21 fill), and from there the
   count of distinct execids is the order's `cumqty`; a later stated total that disagrees is
   warned and does not move the ledger. **On the capture this ends order 557 at 472 / 128
   `PARTIALLY_FILLED`** - the exchange-side frame of fill 468 states `151=0` with no `14` (`:73`),
   one frame of fill 467 states 0 and the other 203 for the same fill (`:35`, `:56`), so the
   derived 600 / 0 `FILLED` is that level's and not the order's. The first draft kept the stated
   number, under which the rule changed no capture row; the review read that as doing the
   opposite of the instruction ("updates the real consumed"). **Say if the venue's stated total
   should win after all** - then a per-tag derived mark on `FixMsg` is needed so the count beats
   only a `14` the parse derived from `151`, and order 557 would end `FILLED` at 600 with the
   warning.
2. **The rule is the walk's; `with_previous` is unchanged** (D45.4): the chain's memory lives where
   the chain does, a bare follower holds none. The alternative - a one-step compare inside
   `chain_operation` (the user's literal) - catches an adjacent duplicate only and would count a
   non-adjacent one twice; it could sit beside the walk's rule, with the walk correcting it, at
   the cost of two passes per step.
3. **An ended chain's fills are remembered for the dedup window** (60 s of event time, D45.8), so
   a late copy of one restates it instead of starting a live phantom order from a count; past the
   window a copy starts afresh as today. The first draft released the ledger with the chain.
4. **P12b lands in P12** (D45.8): an `EXEC` whose execid's chain ended within the window is a
   restatement of that execution, yielded once - 41 / 23 / 9 become 39 / 21 / 7 on the capture,
   the books' events deduplicated, the snapshot's `[009]`/`[010]` and `[019]`/`[020]` the
   restatements'. It falls out of the same tombstone; the alternative is its own commit after
   P12's, with the counts standing until then.
5. **Busts and corrections move the ledger** (`ExecType` `H`/`G` with `ExecRefID(19)`, D45.1): a
   bust subtracts the fill it names, a correction replaces it, neither counts itself or promotes.
   `19` and `MultiLegReportingType(442)` are read by tag inside `FixMsg::fill_of`, so the
   dictionary does not move. **Say if `ExecRefID` should also map into `identifiers` as
   `execrefid`** - a `FIX:idmap` entry the generator writes, moving the dictionary hash, the dump
   and the census - so the row carries it as an identifier.
6. **The disagreement, the overfill, the unidentified fill and the bust naming nothing are
   warnings, not `FixAnomaly`s**, each keyed by the kind with the chain and the values in the
   detail: a `FixAnomaly` is message content and would move the digests of every row the walk
   disagrees with. Say if an anomaly under `cumqty` is wanted for the medallion.
7. **Order 9623's `ORDSTATUS=partfilled` is read in P12** (D45.7), as an alias of the `OrdStatus`
   code set's `PartiallyFilled` in the set's own reading by name (where `filled` already resolves),
   never as a fallback of `from_status` to the state's spellings; the dictionary does not move.
   The alternative leaves it to a code-set change in the dictionary.
8. **Decision 27 lands in P12** (D45.6): an execution's and a trade's `quantity` is its `lastqty`,
   moving every `EXEC`/`TRAD` row's `quantity`, `hashcode`, `uuid`, every book hash over a book that
   records an execution, and the Rust, Python and Node pins named. The alternative is its own
   commit before P12's.
9. **A fill stating no `ExecID`, or `ExecID` `0`, counts nothing** and is warned once per kind; it
   takes the chain's `cumqty` as a follower reporting no fill does. `0` is FIX's own value for a
   status reply and never an identity.
10. **Orders only** (D45.1): the ledger and the promotion apply to `MarketDataKind::Order`. A quote's
    two legs share one unsided chain, so a ledger over it would end the quote from one leg. **Say
    if quotes are wanted** - then one ledger per leg keyed by the fill's side, and never ending the
    quote from one leg.
11. **A fill is identified by the ids on its own row** (D45.4): `execid`/`secondaryexecid` are read
    before `with_previous`, since the generic followed set carries every type but `mdentryrefid`
    onto an `OrderEvent` follower. Say if `execid` should leave that generic set instead
    (`idtype.rs:343` calls it "a per-report reference"), which would change what a bare
    `OrderEvent` follower carries.
12. **Not in P12**: the one-step rule in a bare `with_previous`; a book's event deduplication
    beyond what the restatements give; the `ExecRefID` idmap (5); quotes (10).
