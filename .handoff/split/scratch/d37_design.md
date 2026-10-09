### D37 - `MarketMessage`, the market side's own message, and the FIX codec's doors onto it

**Decision.** `MarketMessage` is a concrete public struct in
`rust/src/graph/message.rs` - the one message every protocol lands in and the
one `MarketData` holds whole. S3's `pub trait MarketMessage`,
`MarketData::Fix(Box<dyn MarketMessage>)`, `as_message::<T>()`, `into_any`,
`clone_box` and `dyn_eq` are deleted; nothing in `graph/` names a FIX type.

**The type.**

```rust
pub struct MarketMessage {
    facts: Box<OperationEventFacts>, // every element, event, market and operation fact, typed - the one owner
    stated: StatedFacts,             // the facts the source stated, one bit each (FixMsg's 28 bits, generic)
    root: Arc<Field>,                // the protocol's entry root - one Field per dictionary, shared
    row: Scalar,                     // the entries: the ordered run under root, what was stated beyond the facts
    children: Vec<MarketMessage>,    // the elements the message states - a book message's levels - each a message
    metadata: Metadata,              // the keys no dictionary resolved
    anomalies: Vec<Anomaly>,         // what a parse or a walk could not honour
    instrument: InstrumentStatement, // countrycode, underlyingisin, eusipacode stated of its instrument
}
```

- `Element`, `Event`, `Market` and `Operation` are implemented once, on
  `MarketMessage`, through `delegate_event!`, `delegate_market!` and
  `delegate_operation!` over `facts`; `with_previous`, `merge_with` and
  `restating` are concrete over `Element`'s combinators
  (`following_operation`, `merging_operation_event`, `fold_anomalies`, all
  generic); `note_conflict` pushes an `Anomaly`; `follow_identity` is the
  trait's provided one. `OperationEventFacts` stays crate-private.
- The entries are the one row shape: `record() -> FieldRecord<'_>`,
  `entry(name) -> Option<FieldScalar<'_>>` by exact name through
  `Field::index_of`, `with_entries(root, row)` canonicalizing once under the
  root. No second schema class, no per-read parse: a cell read is one buffer
  read. A tag index is FIX's, on its handle.
- `children` are full messages - facts filled by the codec at parse, the
  record under the group's root (one `Arc<Field>` per group per dictionary);
  the parent's record holds no cell for them. `into_market_data(self)`: with
  children, one leaf per child the book records
  (`MarketDataKind::is_recorded`), the parent's facts carried into what the
  child states nothing of (`carry`, as `expand_message` does today), each
  child's facts box moved; without, one leaf by `marketdatakind`, the facts
  box moved, zero allocations; a book message with no children one scoped
  snapshot control. `into_market_leaf` as today. `expand_message`,
  `operations`, `direct_of`, `is_book_message`, `carry`, `direct_unmapped`
  and `FixMarketIterator` move from `fix/market.rs` to `graph/message.rs`
  (`MessageIterator`, the lazy projection of sorted messages into leaves) and
  read facts alone - every FIX reading they made (tag 268, `MDEntryType`,
  `MsgType`) is a fact the parse filled.
- `Anomaly { field, reason }` in `graph/anomaly.rs` is `FixAnomaly` moved and
  renamed - no row holds it today and none will.
- `InstrumentStatement { countrycode: Option<Country>, underlyingisin:
  Option<Isin>, eusipacode: Option<Eusipa> }`: what a message states of its
  instrument that is a registry fact and no element fact - set by a codec at
  parse (FIX from `UnderlyingSecurityID(309)`, `UnderlyingSymbol(311)` and the
  bridge keys exactly as `stated_underlying_isin`/`stated_eusipa` read them
  today), read by `IsinRegistry::learn_stating` through `instrument()`,
  lifted into no column.
- `StatedFacts`: the bits of the facts a source stated; `FixMsg`'s `stated`
  and `row_stated` move here, its `stale` bits vanish - the row holds no fact
  cell to go stale.
- Size: about 170 bytes inline in `MarketData::Message(MarketMessage)`, the
  facts boxed as `FixMsg` boxes them today; `size_of::<MarketData>() == 912`
  holds, `SnapshotEvent` the widest leaf.
- `MarketData::Message(MarketMessage)` replaces `Fix(..)`;
  `MarketKind::Message`, spelled `message`, replaces `Fix`/`fix` - last in
  the order as `fix` was, the `kinds` pins in both bindings and
  `rust/tests/graph/kind.rs` re-pinned with the sentence; `From<MarketMessage>
  for MarketData`, `TryFrom<MarketData> for MarketMessage`, `as_message()` and
  `as_message_mut()` borrow; `From<FixMsg> for MarketData` is `into_message`
  then hold; the `marketdata` writer and the book fold split a held message
  through `into_market_data` as they split a FIX message today.
- The message's own row form: `MarketMessage::field(root: &Field) -> Field` -
  the fact columns in `marketdata` order (`ElementColumn`, `EventColumn`,
  `MarketColumn`, `OperationColumn`), then the entry root's children, then
  `children` (`serie<struct<..>>` of the child form, one level), `metadata`,
  `anomalies` (`serie<struct<field: utf8, reason: utf8>>`) -
  `into_scalar(&self)` and `from_scalar(root, &Scalar)`; pickle and the value
  stream ride it. It is not the FIX fixed row, which is the codec's.

**FIX.**

- `FixMsg` stays as the codec's handle over a message: `registry:
  Arc<FixRegistry>`, `message: MarketMessage`, `header: Box<FixHeader>` (the
  typed header cache), the tag and name indexes into the record, the capture
  it was cut from where one exists. `FixLifted` is deleted: the record's cells
  are the typed storage and `LIFTED_TAGS` render from them. `into_message(self)
  -> MarketMessage` moves, zero allocations; `from_message(registry, message)
  -> Result<FixMsg>`: a message under this registry's entry root adopted as
  it is, its indexes rebuilt once, O(entries), no text parsed; one under
  another root re-rooted - an entry whose name the registry resolves lands
  under that field, one it does not in `metadata`, the facts untouched.
  `FixMsg`'s hand-written `Element`/`Event`/`Market`/`Operation` impls are
  deleted; the codec reads facts through `message()`/`message_mut()`, and a
  view that must be an event (the lifecycle's `LifecycleMessage`) delegates
  to the message as it does today.
- The FIX entry root: one `Arc<Field>` per registry built beside
  `fix_schema` - the fixed row's columns less the fact columns (the crate tags
  `Typed::fact` maps onto facts), less `metadata`, less the elements group
  the children render; `fixentries` among them. `into_row` interleaves the
  fact columns (from `facts`) and the entry columns (from `row`, shared) by a
  per-registry column index; `from_row` is the inverse, the entries' run
  shared; the `NoMDEntries(268)` occurrences render into `fixentries` from
  the children, byte for byte as today. The fixed row's definition and column
  order, the crate dump, `fixmsg` (65_053) and the dictionary hash are
  untouched; the equivalence snapshot is unmoved.
- A tag the dictionary's idmap maps onto a fact - `Side(54)`, `Price(44)`,
  `OrderQty(38)`, every one of them - is never an entry: it is read into the
  fact at parse and rendered from the fact at `into_row` and on the wire. The
  entries are the tags the idmap does not consume. So "a FIX message writing
  the side as `Side(54)`" is the render rule, and `follow_identity` needs no
  FIX override.
- Wire: `into_bytes`/`into_text` as today for a parsed message. For a message
  no FIX parse built - its root another dictionary's or empty - the renderer
  writes each stated fact under the standard tag the idmap names for it (one
  tag per fact, the inverse of the enrichment, the table listed in the
  contract from `Typed`/`identity::record`), else under its crate tag
  (65_0xx), so no stated fact is lost; `MsgType(35)` from `marketdatakind`
  and `state` where unstated - `8` for an order or an execution event, `S` for
  a quote, `W` for a book, `AE` for a trade, the table beside
  `fix::state::from_msgtype`; the header from the entries; the entries by
  tag in pre-order; the trailer stated only. `FixCodec::parse(render(m))`
  restates `m` - facts, stated bits, entries, metadata, children equal - the
  round-trip pin in `rust/tests/fix/`.
- `stated_underlying_isin` and `stated_eusipa` become the parse's
  `state_instrument` fill; `refill_instrument_ids` stays the parse's.

**Bindings.** Python: `MarketMessage` (frozen, hashable, pickled through its
row form), `FixMsg.into_message()`, `FixMsg.from_message(registry, message)`,
`MarketData(message)` intake, `MarketData.as_message()` replacing `as_fix()`,
`Anomaly` replacing `FixAnomaly`. Node: `asFix()` re-spelled `asMessage()`
answering a `MarketMessage` object - the class Node must carry to keep the
door it has - `MarketData`'s intake of a `FixMsg` converting inside,
`FixAnomaly` re-spelled; nothing else added.

**Cost.** `into_message` and `into_market_leaf` zero allocations (the facts
box moves); the rekey rows (`allocations.rs` @8595) unchanged; bytes per
parsed message within the @8715 pin - the caches that leave `FixMsg` pay for
the message's vectors; children exist for book messages alone, and a new
row at a `W` fixture states their cost; a new `allocations` row for
`into_message`/`from_message`; the `fix` bench gains the render of a native
message.

**Pages.** `docs/graph/market-data.md` (the message, its doors, its row
form), the FIX pages' from/into, skills `yggdryl-market-data` and
`yggdryl-fix`; AGENTS.md: the `graph/` row (`message.rs`, `anomaly.rs`,
`market_data.rs`), the `fix/` row (`FixMsg` the handle; the idmap rule), the
S4 note (the 31 market-owned items FIX reached mostly gone: a public message
with public trait doors needs no `from_facts`).

Slice: P5, in place, after P4 (it speaks `transunix`/`sendunix`) and before
S4; one commit.

## Decisions after the define (2026-10-09)
See `p5_decisions.md` (C1 idmap rule held, arrival order on the handle; C2 wire-read facts stated, the handle keeps `stated_band`; C3 the FixMsg market_data wrappers retire, Node redirects; C4 finalize keeps a held hashcode, the handle fill door settles; C5 the projection losses stated in one table).
