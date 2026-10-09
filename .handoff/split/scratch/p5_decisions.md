# P5 (D37) decisions after the define - foreground, 2026-10-09 14:05 UTC

Binding on the P45 lane manager; normative over `p5_define_report.md` where the two differ.
D37 holds as written in `d37_design.md`; these decide what the define left open.

## C1 - the idmap rule holds; arrival order is the handle's
Option (a). A tag the dictionary's idmap maps onto a fact is never an entry: read into the fact at
parse, rendered from the fact. The message's record holds no fact cell, the row form stays flat
(fact columns, then the entry root's children), `MarketMessage::from_scalar` reads every FIX-derived
message, the pickle round trip holds, W4's and W6's idmap pins stand as written.
W2's objection - arrival order - is answered on the codec's handle, never in the message: `FixMsg`
keeps, beside its tag and name indexes, the crate-private arrival sequence - one slot per wire field
in wire order: the tag, and for a field the idmap consumed, its wire text as it arrived (a `Str`, inline
for the short values FIX writes). `into_bytes`/`into_text` of a parsed message walk that sequence: an
entry tag renders its record cell, a consumed tag renders its arrival text unless its fact was set
through the handle after the parse (`set_side` - the slot marked dirty, rendered from the fact
through the inverse idmap), so the wire, the FIX digest and `rust/tests/fix/equivalence.snapshot`
stay byte-identical with no edit. A message no parse built renders by D37's native rule (the inverse
idmap table, then the entries by tag in pre-order). `into_message` drops the handle's caches, the
arrival sequence among them, and allocates nothing.
The children, the book control (`W`/`X`: action, scope), the acknowledgement flag and
`InstrumentStatement` are filled at parse in `fix/build.rs`, as D37 says and W1 asked; `into_message`
builds nothing. `contributes_to_market(&m)` becomes `m.message().is_market_data()`.

## C2 - a fact read off a wire field is stated
`StatedFacts` is "the facts the source stated" (D37). The parse marks the bit of every fact it reads
off any field, a standard tag or a crate tag alike; `laid_out` unmarks nothing; a setter marks. So
`FixCodec::parse(render(m)).stated() == m.stated()` for every message - the round-trip pin in
`rust/tests/fix/codec.rs` `mod round_trip` and the `docs/fix/message.md` example hold.
The fixed row's crate band is rendered from the handle's own wire word (`FixMsg::stated_band`: which
facts arrived under a crate tag), a fact of the wire and not of the message, so the dump, the snapshot
and the dictionary hash do not move. A second word on the handle is not a second owner: it answers
"arrived under 650xx", the message's bits answer "stated".

## C3 - one spelling: the FixMsg market_data wrappers retire
`FixMsg::{market_data, into_market_data, into_market_leaf}` are deleted; the spelling is
`fix.into_message().into_market_data()` / `.into_market_leaf()?`, `MessageIterator::new(..)` over
`Result<MarketMessage>` items. The `$.NoMDEntries(268)` refusal is the parse's, raised and located
there since C1 builds the children at parse (`rust/tests/fix/market.rs` re-pins it at the parse door);
`into_market_leaf` on a message with children refuses generically by its kind. The ~45 W6 sites in
`rust/tests/fix/{market,crated,idmap}.rs`, `.api-inventory.txt:1068-1070` and the Rust docs tabs are
re-spelled. Node's existing `FixMsg.marketData()` stays as a door and redirects through
`into_message()` - no new door, no removed one, Node unnamed by the request.

## C4 - a fill through the handle settles the handle
`finalize` on a `MarketMessage` keeps a held `hashcode` - the content digest is the source's: the
FIX digest of the wire for a parsed message, XXH3 of the body for a line, the generic fact digest
only where none is held - and derives `uuid`, `crossuuid` and `crosshashcode` from it and the
identity as every event does (a walk's rekey moves the cross identity and the uuid, never the
hashcode). So a FIX-derived message walked as `MarketData::Message` keeps its FIX identity.
A registry fill or enrich of a `FixMsg` goes through a handle door (`FixMsg::fill(&registry)`,
`enrich`) that borrows `message_mut()`, lets the registry fill, then `settle`s - indexes and the
derived overlay refreshed; the Python and Node `fill`/`enrich` on a `FixMsg` call it and reach
`message_mut()` for nothing; on a bare `MarketMessage` they call the registry's generic door.

## C5 - what the into_message/from_message round trip does not carry, by design
`into_message` is the projection to the generic message; what is handle state - the capture it was
cut from and its carried cells, the header cache and an unstated header clock, the plugin side a
registry source stated without a `msgpluginside` cell, the arrival sequence, the arrival/idmap
split of anomalies (one `anomalies` list on the message) - is the codec's and does not cross; a
bridge key lifted into an identifier comes back as that identifier, the lift being the parse's
reading. `from_message` adopts a message as a new FIX statement under the registry: identity
(`uuid`, `hashcode`) travels as facts and is not recomputed from the re-rendered wire; the render
order of a re-adopted message is the native rule's. `docs/fix/message.md` states this in one
"What crosses" table; no re-render pin claims byte identity through the round trip.

## Recording
The P45 manager writes these five under DESIGN.md "## P5: design" as "### Decisions after the
define" in the P5 commit, and `d37_design.md` carries the same text (appended below the design).
