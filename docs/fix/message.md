# Message

`FixMsg` is a market event with a FIX body around it: the [event](../graph.md) the message is, the standard header typed, what the capture said about the line, the free text and a bridge's namespaced keys - each held once, as a typed fact - and the content row, every other field the message states, typed by the registry's field for it. One message is one frame, one bridge row or one document - never the line, which can carry [several of them or none](decode.md#a-line-yields-none-one-or-many-messages).

## Contract

| item | contract |
| --- | --- |
| Holders | `event() -> &MarketEventData`, what the four [graph traits](../graph.md) answer - the identities, the codes, the instants, the state, the place in the chain, the price, the quantity, the side, the lanes and the instrument codes; `header() -> &FixHeader`, tags 8, 35, 49, 56, 34, 52, 43 and 385 typed; `capture() -> &FixCapture`, `sourceurl`, `recordedat`, `pluginid`, `msgctxid` and `msgsessionid`; `text() -> Option<&str>`, `Text(58)`; `metadata() -> &BTreeMap<SmolStr, SmolStr>`, what a bridge stated under its own namespaces - `TECH.CLIENTID`, `firm.acronym` - each under the key as the bridge spelled it, folded |
| Row | `as_field()` and `as_value()`: a root Struct [`Field`](../types/field.md) and the `Scalar::Sequence` it declares, holding only what no holder owns - the dictionary's fields, a group as a List of Struct occurrences beside its `int32` counter, a component as a Struct, a key no dictionary explains under its own spelling; a [typed tag](#typed-tags) is never in it |
| Entries | `entries() -> &[FixEntry]`, the row read as a tree, derived on the first ask and dropped by every write: one entry per non-null child, each carrying the tag the dictionary resolved - `0` for a key it does not explain - the canonical name and the value as the wire spells it; a group is one entry under its counter valued the count, with one valueless entry per occurrence heading the members; a component a valueless entry heading its members; the typed facts are not entries |
| Wire | `into_bytes(separator)` and `into_text(separator)` re-emit the message as it now stands, derived values included: the header tags 8, 35, 49, 56, 34, 43 and 52 - the last only where the message stated it - then the event's own tags 15, 54, 461, 132, 133, 134 and 135, then 58, then the entries pre-order; a coded fact spells as its wire code, `54=1`, and a lane number at the decimal's full scale. `digest() -> u128` is the XXH3-128 of what `into_bytes` emits, whatever separator, and is what [`FixDedup`](arrow.md#a-pin-is-on-the-codec-a-stage-is-a-call) keys on |
| Constructors | `FixMsg::new` links `FixRegistry::global()`; `FixMsg::with_registry` keeps the `Arc` it is given, lifts every typed fact out of the children that state it, settles the clocks and derives the identity, and runs no derivation - a [parse](capture.md#a-reader-is-the-whole-parse-surface) does; `FixMsg::from_row` reads a [fixed row](#a-row-is-a-message-again) back, entries included |
| Lookups | every one answers an owned `Scalar`: a typed tag its holder's fact, or nothing where the holder states none; any other key the row child it reaches |
| Writes | `set`, `set_many`, `with_value` and `remove`: a key reaching a typed fact writes its holder, a `Null` clearing it; any other key [writes the row](#written-into-the-row), typed through the registry's field; every write settles the identity again; a refusal leaves the message unchanged. `set_many` and `with_value` are Rust-only |
| Settled | `unix` is a stated `unix`, else `TransactTime(60)` stating a clock, else `SendingTime(52)`; `SendingTime` is the message's own, else the codec's `default_sending_time`, else one UTC-now read at intake, and only a stated one is a fact of the message - it goes back on the wire and into a row, while a stand-in intake supplied does neither; `creatunix` is a stated one, else `OrigSendingTime(122)`, else `unix`; the [identity](../hashing.md) - `crosscode`, `crosshashcode`, `hashcode`, `curruuid`, `crossuuid` - is derived from what the message *states*: the event's facts, the text, the metadata, the header fields it stated, then the entry tree, so two readings that lay one message out differently answer one code |
| Identity | a field is its tag and its name; a message speaks no dialect and carries no membership, so a bare tag or name resolves in the registry's [one namespace](#one-namespace) |
| Graph | `FixMsg` implements `Element`, `Event` and `MarketElement` - so `MarketEvent` - through its event: `is_after` is the instant, `finalize` settles the identity again, `with_previous` is `Event::following` with the predecessor adopted as a parent, `merge_with` is `MarketEvent::merging_market_event`; import the traits to call them |
| Serialization | inherited: `as_field().clone().into_json()` renders the row's schema, [`into_json_scalar`](../media/json/index.md) its value, `from_json_scalar_with_field` reads it back typed, ordered and canonicalized against the same root; `into_row` is the whole message as one fixed row |
| Equality | over the holders, the row and the registry - the same `Arc`, or registries holding the same fields; `Hash` over the hashcode, the row's schema and its value |
| Bindings | Rust, Python, JavaScript |

## Use

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::{Element, Event, MarketElement};
    use yggdryl::{DataType, FixMsg, FixRegistry, Scalar, FieldPath, from_json_scalar_with_field, into_json_scalar};

    let mut msgtype = DataType::utf8().nullable_field("MsgType");
    msgtype.as_fix_mut().set_tag(35)?;
    let mut side = DataType::utf8().nullable_field("Side");
    side.as_fix_mut().set_tag(54)?;
    let mut symbol = DataType::utf8().required_field("Symbol");
    symbol.as_fix_mut().set_tag(55)?;
    symbol.as_fix_mut().set_names(["Ticker"])?;
    let mut qty = DataType::Int64.required_field("OrderQty");
    qty.as_fix_mut().set_tag(38)?;
    let mut party_id = DataType::utf8().nullable_field("PartyID");
    party_id.as_fix_mut().set_tag(448)?;
    let mut count = DataType::Int32.required_field("NoPartyIDs");
    count.as_fix_mut().set_tag(453)?;
    let party = DataType::from_fields([party_id.clone()])?.required_field("Party");
    let mut parties = DataType::list(party.clone()).nullable_field("Parties");
    parties.as_fix_mut().set_counter(453)?;
    parties.as_fix_mut().set_component("Party")?;
    let mut registry = FixRegistry::from_fields([msgtype.clone(), side.clone(), symbol.clone(), qty.clone(), count.clone(), party_id])?;
    registry.insert(party)?;
    registry.insert(parties.clone())?;
    let registry = Arc::new(registry);

    // The root carries two typed tags and a tag no dictionary explains.
    let root = DataType::from_fields([msgtype, side, qty, symbol, count, parties, DataType::utf8().nullable_field("9999")])?
        .required_field("NewOrderSingle");
    let value = Scalar::from_record([
        ("MsgType", Scalar::from("D")),
        ("Side", Scalar::from("1")),
        ("Symbol", Scalar::from("AAPL")),
        ("OrderQty", Scalar::from(100_i64)),
        ("NoPartyIDs", Scalar::from(1_i32)),
        ("Parties", Scalar::from_sequence([
            Scalar::from_record([("PartyID", Scalar::from("BROKER"))])?,
        ])),
        ("9999", Scalar::from("custom")),
    ])?;
    let msg = FixMsg::with_registry(Arc::clone(&registry), root, value)?;

    // The typed facts left the row for their holders: the type is the
    // header's, the side the event's, and the row holds the five others.
    assert_eq!(msg.header().msgtype(), "D");
    assert_eq!(msg.get_side().as_str(), "BUY");
    let children: Vec<&str> = msg.as_field().fields().iter().map(yggdryl::Field::name).collect();
    assert_eq!(children, ["OrderQty", "Symbol", "NoPartyIDs", "Parties", "9999"]);
    // A lookup answers the holder for a typed tag and the row for the rest.
    assert_eq!(msg.by_tag(35)?, Scalar::from("D"));
    assert_eq!(msg.by_tag(54)?, Scalar::from("BUY"));
    assert_eq!(msg.by_tag(38)?, Scalar::from(100_i64));
    assert_eq!(msg.by_name("ticker")?, Scalar::from("AAPL"));
    assert_eq!(msg.by_path(&FieldPath::from_str("Parties[0].PartyID")?)?, Scalar::from("BROKER"));
    assert_eq!(msg.by_tag(9999)?, Scalar::from("custom"), "an unknown tag is retained");
    assert_eq!(msg.get(55), msg.get_by_tag(55));
    assert!(msg.value("Parties.PartyID").is_err(), "a group member needs its index");

    // The identity is settled from what the message states.
    assert_ne!(msg.get_hashcode(), 0);
    assert_eq!(msg.get_curruuid(), msg.time_uuid()?);
    assert_eq!(msg.get_unix(), msg.header().sendingtime(), "undated, so the sending clock stands in");
    assert_eq!(msg.get_crosscode(), "", "no OrderID or ClOrdID names a chain");
    assert_eq!(msg.get_crossuuid(), msg.get_curruuid(), "so the message is a chain of one");

    // An identifier is the tag and the name together, under the one fold, and exact.
    let id = registry.field_by_tag(38)?.as_fix().id()?.expect("a tagged field");
    assert_eq!(id, yggdryl::FixId::of(38, "order_qty")?);
    assert_eq!(msg.by_id(id)?, Scalar::from(100_i64));
    assert!(msg.get_by_id(yggdryl::FixId::of(38, "Quantity")?).is_none(), "another name is another field");

    // The row serializes through the paths every field and value share, and
    // a message rebuilt from them holds the same content; its typed facts
    // are its own to state again.
    let root = msg.as_field().clone();
    let schema = root.clone().into_json()?;
    assert!(schema.contains("\"fix:tag\":\"55\""), "{schema}");
    let text = into_json_scalar(msg.as_value())?;
    let read = from_json_scalar_with_field(&text, &root)?;
    assert_eq!(&read, msg.as_value());
    let again = FixMsg::with_registry(registry, root, read)?;
    assert_eq!(again.entries(), msg.entries());
    assert_eq!(again.header().msgtype(), "", "the type was the header's, not the row's");
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType, Field, types
    from yggdryl.fix import FixMsg, FixRegistry

    symbol = Field("Symbol", "utf8", nullable=False)
    symbol.fix.tag = 55
    symbol.fix.names = ["Ticker"]
    qty = Field("OrderQty", "int64", nullable=False)
    qty.fix.tag = 38
    party_id = Field("PartyID", "utf8")
    party_id.fix.tag = 448
    count = Field("NoPartyIDs", "int32", nullable=False)
    count.fix.tag = 453
    item = Field("Party", DataType.from_fields([party_id]), nullable=False)
    parties = types.list("Parties", item)
    parties.fix.counter = 453
    parties.fix.component = "Party"
    registry = FixRegistry.from_fields([symbol, qty, count, party_id])
    registry.create_definition("components", item)
    registry.create_definition("groups", parties)

    # The root carries a tag no dictionary explains, under its rendered name.
    root = Field(
        "NewOrderSingle",
        DataType.from_fields([qty, symbol, count, parties, Field("9999", "utf8")]),
        nullable=False,
    )
    message = FixMsg(
        root,
        {
            "Symbol": "AAPL",
            "OrderQty": 100,
            "NoPartyIDs": 1,
            "Parties": [{"PartyID": "BROKER"}],
            "9999": "custom",
        },
        registry,
    )

    # The mapping became the ordered row the root declares, with the seven
    # settled values appended behind the five it stated.
    assert len(message) == 12
    assert message.updatedat() == message.by_tag(52), "an undated message is dated once"
    assert message.msgphash() == message.by_name("msgphash")
    assert message.by_tag(38).as_py() == 100
    assert message.by_name("ticker").as_py() == "AAPL"
    assert message.by_path("Parties[0].PartyID").as_py() == "BROKER"
    assert message.by_tag(9999).as_py() == "custom", "an unknown tag is retained"
    assert message[55] == message.get_by_tag(55)
    with pytest.raises(KeyError):
        message.by_path("Parties.PartyID")  # a group member needs its index
    assert [name for name, _ in message] == [
        "OrderQty", "Symbol", "NoPartyIDs", "Parties", "9999",
        "updatedat", "createdat", "msghash", "msgphash", "code", "snapshotat", "sendingtime",
    ]

    # An identifier is the tag and the name together, under the one fold, and exact.
    folded = Field("order_qty", "int64")
    folded.fix.tag = 38
    assert folded.fix.id == qty.fix.id
    assert message.by_id(qty.fix.id).as_py() == 100
    renamed = Field("Quantity", "int64")
    renamed.fix.tag = 38
    assert message.get_by_id(renamed.fix.id) is None, "another name is another field"
    retagged = Field("OrderQty", "int64")
    retagged.fix.tag = 5001
    assert message.get_by_id(retagged.fix.id) is None, "another tag is another field"

    # The schema serializes through the path every field already has, and the
    # value the message holds names the same row, settled values included.
    assert '"fix:tag":"55"' in message.field.into_json()
    assert FixMsg(message.field, message.value, registry) == message
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, Scalar, fields, fix } = require('yggdryl')

    const symbol = Field.from('Symbol: utf8 not null')
    symbol.fix.tag = 55
    symbol.fix.names = ['Ticker']
    const qty = Field.from('OrderQty: int64 not null')
    qty.fix.tag = 38
    const partyId = Field.from('PartyID: utf8')
    partyId.fix.tag = 448
    const count = fields.int32('NoPartyIDs', { nullable: false })
    count.fix.tag = 453
    const item = fields.struct('Party', [partyId], { nullable: false })
    const parties = fields.list('Parties', item)
    parties.fix.counter = 453
    parties.fix.component = 'Party'
    const registry = fix.FixRegistry.fromFields([symbol, qty, count, partyId])
    registry.createDefinition('components', item)
    registry.createDefinition('groups', parties)

    // The root carries a tag no dictionary explains, under its rendered name.
    const root = fields.struct(
      'NewOrderSingle',
      [qty, symbol, count, parties, Field.from('9999: utf8')],
      { nullable: false },
    )
    const message = new fix.FixMsg(
      root,
      {
        Symbol: 'AAPL',
        OrderQty: 100n,
        NoPartyIDs: 1,
        Parties: [{ PartyID: 'BROKER' }],
        9999: 'custom',
      },
      registry,
    )

    // The plain object became the ordered row the root declares, with the
    // seven settled values appended behind the five it stated.
    assert.equal(message.value.kind, 'sequence')
    assert.equal(message.size, 12)
    assert.ok(message.updatedat().equals(message.byTag(52)), 'an undated message is dated once')
    assert.ok(message.msgphash().equals(message.byName('msgphash')))
    assert.equal(message.byTag(38).asJs(), 100)
    assert.equal(message.byName('ticker').asJs(), 'AAPL')
    assert.equal(message.byPath('Parties[0].PartyID').asJs(), 'BROKER')
    assert.equal(message.byTag(9999).asJs(), 'custom', 'an unknown tag is retained')
    assert.ok(message.get(55).equals(message.getByTag(55)))
    assert.throws(() => message.at('Parties.PartyID'), /a fix value/)
    assert.deepEqual(
      [...message].map(([name]) => name),
      [
        'OrderQty', 'Symbol', 'NoPartyIDs', 'Parties', '9999',
        'updatedat', 'createdat', 'msghash', 'msgphash', 'code', 'snapshotat', 'sendingtime',
      ],
    )

    // An identifier is the tag and the name together, under the one fold, and exact.
    const folded = Field.from('order_qty: int64')
    folded.fix.tag = 38
    assert.equal(folded.fix.id, qty.fix.id)
    assert.equal(message.byId(qty.fix.id).asJs(), 100)
    const renamed = Field.from('Quantity: int64')
    renamed.fix.tag = 38
    assert.equal(message.getById(renamed.fix.id), null, 'another name is another field')
    const retagged = Field.from('OrderQty: int64')
    retagged.fix.tag = 5001
    assert.equal(message.getById(retagged.fix.id), null, 'another tag is another field')

    // Schema and value serialize through the paths every field and value
    // share, and the settled values come back as they were.
    const document = message.toJSON()
    assert.equal(document.field.dtype.fields[1].metadata['fix:tag'], '55')
    assert.deepEqual(document.value[1], 'AAPL')
    assert.ok(new fix.FixMsg(message.field, message.value, registry).equals(message))
    ```

## Typed tags

A message holds each fact once. The tags below are the holders' and are never in the row: a child stating one at construction fills its holder and leaves the row, a lookup by one of them answers the holder, a write to one of them writes the holder. Everything else - `ClOrdID(11)`, `OrderQty(38)`, a `Parties` group, a `9999` no dictionary explains - is the row's.

| tags | holder | facts |
| --- | --- | --- |
| 8, 35, 49, 56, 34, 52, 43, 385 | `header()` | `beginstring`, `msgtype`, `sendercompid`, `targetcompid`, `msgseqnum`, `sendingtime` with `stated_sendingtime`, `possdupflag`, `msgdirection` |
| 15, 54, 461, 132, 133, 134, 135 | `event()` | the currency, the side, the CFI classification, and the bid and ask lanes' prices and sizes - FIX's own spellings of what a [`MarketElement`](../graph.md) states |
| every [crate tag](capture.md#the-crates-own-columns), 65000 to 65099 | `event()` and `capture()` | the identities, the codes, the instants, the state, the place in the chain, the price, the quantity, the unit, the lane currencies and units, the instrument codes and the market on the event; `sourceurl`, `recordedat`, `pluginid`, `msgctxid` and `msgsessionid` on the capture; `identifiers` and `metadata` are the event's names and the bridge's namespaced keys |
| 58 | `text()` | the free text |

A typed fact answers as its column types it: `by_tag(35)` is text, `by_tag(52)` a `datetime64(ns, UTC)`, `by_tag(54)` the side's name - `BUY`, `SELL` - `by_tag(HASHCODE_TAG_NAME.0)` a `UInt64`, `by_tag(CURRUUID_TAG_NAME.0)` a `Uuid`, `by_tag(IDENTIFIERS_TAG_NAME.0)` a sorted map; a holder stating nothing - a side of `UNKNOWN`, a state of `00UNKNOWN`, a place of zero in no chain - answers nothing.

## One namespace

A message speaks no dialect of its own: the registry is one namespace, and a bare tag or name resolves in it directly, the way the [registry](registry.md) resolves it.

| key | answers |
| --- | --- |
| a tag | the canonical holder of the tag, then a field holding it as an alternate |
| a name | the canonical fold, then an alias fold - `ticker` reaches `Symbol` through its alias |
| an id | exactly one field: `FixId::of(tag, name)`, the signed XXH32 of the tag's bytes and the folded name, so `OrderQty`, `order_qty` and `orderqty` under 38 are one id and `Quantity` under 38 is another |

`get_by_tag(5001)` finds a venue's own field and `get_by_tag(35)` finds the message type from the same message, because both live in the one namespace. Which dictionaries a field belongs to is the field's own `fix:branches`, a membership a reader may ask about and nothing here resolves through; a message root the codec builds carries none.

## Accessors

| accessor | resolution |
| --- | --- |
| `get_by_tag` / `by_tag` | a [typed tag](#typed-tags) answers its holder; else the root child carrying the tag, else the tag through the registry to its canonical name and the root child of that name, else a root child named by the tag's decimal text |
| `get_by_id` / `by_id` | takes a `FixId` (an `int` in Python, a `number` in JavaScript) and names one field exactly: a typed field's holder, else the child under that field's name, and a miss for any other tag or name |
| `get_by_name` / `by_name` | folds through the registry to the canonical spelling - a typed field answers its holder - then matches a root child exactly |
| `get_by_path` / `by_path` | the first segment as a name, then segment by segment: into a Struct child by name, into a List entry by a decimal index |
| `get` / `value` | takes a `FixKey` and redirects; a name that reaches nothing and spells more than one segment is read as a path |
| `event`, `header`, `capture`, `text`, `metadata` | the holders themselves, borrowed without a lookup |

Every lookup answers an owned `Scalar`: a holder's fact is rendered into the column's type on the way out, and a row child cloned.

## Written into the row

A message is read once and then written to: a [walk](lifecycle.md) stamps what the stream implied, a caller corrects a value. All of it goes through one door. `set` writes one value, `set_many` lands several with one rebuild, `with_value` is the consuming twin, and `remove` takes a value out and answers what it held. A key reaching a [typed tag](#typed-tags) writes its holder - `set(34, ..)` is the header's sequence number, `set(PX_TAG_NAME.0, ..)` the event's price - and a `Null` clears it; any other key writes the row, typed by the field the key reaches. Every write settles the [identity](../hashing.md) again, so a content write moves `hashcode` and `curruuid` while `crossuuid` stays with the cross code; and every write drops the derived entries, so `into_bytes` re-emits the message as it now stands.

| Key | Reaches |
| --- | --- |
| a typed tag | the holder that owns it; a value the fact's type refuses is silence, a `Null` clears the fact |
| a tag the dictionary knows | the child carrying it, replaced where it stands; else appended under the dictionary's field - its canonical name, its datatype, its `fix:tag` - so a written child is indistinguishable from a stated one |
| a name the dictionary knows | the same field, through the [one namespace](#one-namespace) every lookup resolves in |
| a name it does not know | the child spelled that way, exactly or under the fold every name resolves by, keeping that child's own field; nothing reached is refused, and the message stands |
| a tag it does not know | the child named by its decimal, else a nullable `utf8` child appended under it - what the builder does with an unknown tag |
| a `Null` value | a stated null: the child stays, nullable, holding nothing |

A written child keeps its position, so every reader already holding the row addresses it as before, and the tag index follows the one child that changed rather than being reread. A value the field refuses - text into `OrderQty(38)` - is refused whole, and with `set_many` one refused write refuses them all.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::Element;
    use yggdryl::holder::local::Folder;
    use yggdryl::{FixCodec, FixMsg, FixRegistry, Scalar, fix_schema};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    let reader = FixCodec::new(Arc::clone(&registry));

    let line = b"8=FIX.4.4|35=D|52=20260102-10:15:30|11=A1|55=AAPL|54=1|9999=x|10=0|";
    let mut message = reader.parse_fix_line(line)?;
    let children = message.as_field().fields().len();
    let at = message.as_field().index_of("symbol").expect("the symbol child");
    let (hashcode, crossuuid) = (message.get_hashcode(), message.get_crossuuid());

    // A typed tag lands on its holder and never in the row.
    message.set(34, Scalar::from(7_i32))?;
    assert_eq!(message.header().msgseqnum(), Some(7));
    assert_eq!(message.by_name("MsgSeqNum")?.as_u64(), Some(7));
    assert_eq!(message.as_field().fields().len(), children);
    // The content identity moved with the content; the chain's is the cross code's alone.
    assert_ne!(message.get_hashcode(), hashcode);
    assert_eq!(message.get_crossuuid(), crossuuid);

    // Replaced where it stands: the position is kept, the value changes.
    message.set("Symbol", Scalar::from("MSFT"))?;
    assert_eq!(message.as_field().index_of("symbol"), Some(at));
    assert_eq!(message.by_tag(55)?.as_str(), Some("MSFT"));

    // A tag no dictionary explains is kept under its decimal spelling, and
    // appended to the row.
    message.set(7777, Scalar::from("custom"))?;
    assert_eq!(message.by_tag(7777)?.as_str(), Some("custom"));
    assert_eq!(message.as_field().fields().len(), children + 1);

    // A name nothing reaches is refused, and the row stands as it was.
    assert!(message.set("nosuchfield", Scalar::from("x")).is_err());
    assert_eq!(message.as_field().fields().len(), children + 1);

    // Removed, and the value answered: a typed fact is cleared on its
    // holder, a row child taken out, and the other tags still reach theirs.
    assert_eq!(message.remove(54)?.as_ref().and_then(Scalar::as_str), Some("BUY"));
    assert_eq!(message.get_by_tag(54), None);
    assert_eq!(message.remove("nosuchfield")?, None);
    assert_eq!(message.by_tag(11)?.as_str(), Some("A1"));

    // The wire is the message as it now stands: the header from its holder,
    // then the row, the written pairs where they landed.
    assert_eq!(
        message.into_text('|')?,
        "8=FIX.4.4|35=D|34=7|52=20260102-10:15:30|11=A1|55=MSFT|9999=x|10=0|59=0|7777=custom|",
    );

    // The written message is a fixed row, and the row a message again:
    // the same entries, the same code, re-emitting the same wire.
    let schema = fix_schema(&registry, "fix")?;
    let row = message.into_row(&schema)?;
    let held = FixMsg::from_row(Arc::clone(&registry), &schema, &row)?;
    assert_eq!(held.by_tag(55)?, message.by_tag(55)?);
    assert_eq!(held.entries(), message.entries());
    assert_eq!(held.get_hashcode(), message.get_hashcode());
    assert_eq!(held.into_bytes(b'|'), message.into_bytes(b'|'));
    assert_eq!(held.into_row(&schema)?, row);
    ```

=== "Python"

    ```python
    from pathlib import Path

    import pytest

    from yggdryl.fix import FixCodec, FixMsg, FixRegistry, fix_schema

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    reader = FixCodec(registry)

    line = b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|9999=x|10=0|"
    message = reader.parse_fix_line(line)
    children = len(message)
    names = [name for name, _ in message]
    msghash, msgphash = message.msghash(), message.msgphash()

    # Appended under the dictionary's field, typed by it, reached by tag or name.
    message.set(34, 7)
    assert message.by_tag(34).as_py() == 7
    assert message.by_name("MsgSeqNum").as_py() == 7
    assert [name for name, _ in message][-1] == "msgseqnum"

    # Replaced where it stands: the position is kept, the value changes.
    message.set("Symbol", "MSFT")
    assert [name for name, _ in message].index("symbol") == names.index("symbol")
    assert message.by_tag(55).as_py() == "MSFT"
    # The content identity moved with the content; the chain's is `code`'s alone.
    assert message.msghash() != msghash
    assert message.msgphash() == msgphash

    # A tag no dictionary explains is kept under its decimal spelling.
    message.set(7777, "custom")
    assert message.by_tag(7777).as_py() == "custom"

    # A name nothing reaches is refused, and the row stands as it was.
    with pytest.raises((KeyError, ValueError)):
        message.set("nosuchfield", "x")
    assert len(message) == children + 2

    # Removed, and the value answered; a mandatory field refuses removal and
    # a null; the other tags still reach their children.
    assert message.remove(54).as_py() == "BUY"
    assert message.get_by_tag(54) is None
    assert message.remove("nosuchfield") is None
    with pytest.raises(ValueError):
        message.remove("updatedat")
    with pytest.raises((TypeError, ValueError)):
        message.set("updatedat", None)
    assert message.by_tag(11).as_py() == "A1"

    # Only the row changed: the wire comes back byte for byte.
    assert message.into_bytes(ord("|")) == line
    assert message.entries() == reader.parse_fix_line(line).entries()

    # The written message is a fixed row, and the row a message again: the
    # root is the schema, reached the same way, re-emitting the same wire.
    schema = fix_schema(registry, "fix")
    row = message.into_row(schema)
    held = FixMsg.from_row(schema, row, registry)
    assert held.field == schema
    assert held.by_tag(55) == message.by_tag(55)
    assert held.entries() == message.entries()
    assert held.digest() == message.digest()
    assert held.into_bytes(ord("|")) == line
    assert held.into_row(schema) == row
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const reader = new fix.FixCodec(registry)

    const line = '8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|9999=x|10=0|'
    const message = reader.parseFixLine(Buffer.from(line))
    const children = message.size
    const names = [...message].map(([name]) => name)
    const [msghash, msgphash] = [message.msghash(), message.msgphash()]

    // Appended under the dictionary's field, typed by it, reached by tag or name.
    message.set(34, 7)
    assert.equal(message.byTag(34).asJs(), 7)
    assert.equal(message.byName('MsgSeqNum').asJs(), 7)
    assert.equal([...message].map(([name]) => name).at(-1), 'msgseqnum')

    // Replaced where it stands: the position is kept, the value changes.
    message.set('Symbol', 'MSFT')
    assert.equal([...message].map(([name]) => name).indexOf('symbol'), names.indexOf('symbol'))
    assert.equal(message.byTag(55).asJs(), 'MSFT')
    // The content identity moved with the content; the chain's is `code`'s alone.
    assert.ok(!message.msghash().equals(msghash))
    assert.ok(message.msgphash().equals(msgphash))

    // A tag no dictionary explains is kept under its decimal spelling.
    message.set(7777, 'custom')
    assert.equal(message.byTag(7777).asJs(), 'custom')

    // A name nothing reaches is refused, and the row stands as it was.
    assert.throws(() => message.set('nosuchfield', 'x'))
    assert.equal(message.size, children + 2)

    // Removed, and the value answered; a mandatory field refuses removal and
    // a null; the other tags still reach their children.
    assert.equal(message.remove(54).asJs(), 'BUY')
    assert.equal(message.getByTag(54), null)
    assert.equal(message.remove('nosuchfield'), null)
    assert.throws(() => message.remove('updatedat'))
    assert.throws(() => message.set('updatedat', null))
    assert.equal(message.byTag(11).asJs(), 'A1')

    // Only the row changed: the wire comes back byte for byte.
    assert.equal(Buffer.from(message.intoBytes(124)).toString(), line)

    // The written message is a fixed row, and the row a message again: the
    // root is the schema, reached the same way, re-emitting the same wire.
    const schema = fix.schema(registry, 'fix')
    const row = message.intoRow(schema)
    const held = fix.FixMsg.fromRow(schema, row, registry)
    assert.ok(held.field.equals(schema))
    assert.ok(held.byTag(55).equals(message.byTag(55)))
    assert.deepEqual(held.digest(), message.digest())
    assert.equal(Buffer.from(held.intoBytes(124)).toString(), line)
    assert.ok(held.intoRow(schema).equals(row))
    ```

## A row is a message again

`from_row` is the inverse of [`into_row`](capture.md#a-column-is-filled-by-the-tag-its-field-carries): the typed facts are read off the columns that hold them, and the content is rebuilt from the `fixentries` column - every level the row materialized, and the leaf the deepest level folded into decoded through the crate's own JSON reader - each entry typed through the dictionary exactly as the builder types a pair, so every lookup reaches the rebuilt message as it reaches a parsed one; the row's `beginstring`, `unix`, `creatunix`, `hashcode`, `crosshashcode`, `curruuid` and `crossuuid` columns must be stated, and every other one may be null - `sendingtime` among them, because a row states tag 52 only where the message did. A body column is the same content read once more, so it is read past; a column no tag names is a capture's own and stays a child of the content row, which is how a row read back through `from_row` returns to its schema whole. Nothing is parsed again, which is what makes a [batch of rows a stream of messages](arrow.md#rows-are-messages-again-and-messages-rows) at the cost of the values it already holds. A row without the entries column rebuilds a message with the typed facts and no content.

The round trip is exact for an entry whose bytes are text - which is every entry a log wrote. It cannot be for one whose bytes are not: the row spells a value as `utf8` because a column a reader can read is what a row is for, and a `data` field carrying bytes no text holds reaches that column as the decode of them. One shape is not exact yet, and `FixMsg::from_row`'s own documentation names it: a repeating group whose occurrences nest a second group that only some of them state - the row holds each occurrence on the union of the members any of them stated, so rebuilding lays the nested level out in the order the entries met it rather than the order the parse did. Over `rust/tests/fix/ulbridge.log`, a bridge capture of 94 messages, 83 rebuild exactly and the 11 that do not are all parties nesting `PtysSubGrp`. A group no dictionary declares - a bridge packing `NOTRADINGSESSIONS[0]=...` under a counter's own name - rebuilds from the row as the list it is. The [example above](#written-into-the-row) ends with the round trip.

## Restated under the dictionary

A capture holds what each session spoke: a FIX 4.2 report states its fill as `LastShares`, its broker as `ExecBroker(76)`, its capacity as `Rule80A(47)` and a partial fill as `ExecType(150)` `1` - four things the newest specification spells as `LastQty`, a `Parties` occurrence, `OrderCapacity(528)` and `Trade`. A [parse](capture.md#a-reader-is-the-whole-parse-surface) restates the message once as it builds it, from what the dictionary itself says: the registry's field for every tag, the aliases that reach it, and the [`fix:replacements`](registry.md#a-field-carries-what-replaced-it) each retired field or value carries - and then [fills what the restated row implies](capture.md#what-a-message-implied-is-filled-in) by the [`fix:derivation`](registry.md#a-field-carries-how-it-is-derived) each derived field carries. Nothing in Rust holds a table of rules, so a registry edit is a rule edit, and there is no door of its own: a parsed message is a restated message.

| item | contract |
| --- | --- |
| Canonicalizes | every child the registry knows - by its `fix:tag`, else its name or alias, else the decimal tag its name spells - is re-expressed under the registry's field: canonical name, datatype, tag, in the position it held; a child no dictionary knows stays exactly as it is |
| Merges | children reaching one field become one: the canonical-named child's value when stated, else the first stated among the rest; a child whose stated value disagrees with the kept one is left in place, so nothing that arrived is lost |
| Restates | each child whose field carries `fix:replacements`, in ascending tag order, by the first entry whose plan's `where` holds over the level, `:msgtype` and `:group` supplied; a group occurrence in the `select` makes or completes one occurrence and sets its counter |
| Deprecated | a field the dictionary marks [`fix:deprecated`](registry.md#the-dictionary-holds-one-reading-and-filters-by-no-version) - one FIX Latest removed, `MaxFloor(111)` among them - is restated to the field that replaced it and then nulled, so the row holds the fact once under its latest name while the entries keep the pair as it arrived |
| All or nothing | every target an entry fills is computed and checked before any is written; one target that cannot take its value blocks the whole entry, and no later entry fills in for it |
| Never overwrites | a value the message stated: a target takes a value only when it is absent, null or already equal; the source field itself is the one exception, because it is what is being restated |
| Keeps | a removed field the specification named no replacement for, and the source of every rule that did not write it - the row says what was sent and what it means |
| Levels | the root, then every occurrence of every repeating group to any depth, each canonicalized and restated in turn; `:group` is the occurrence's group, `:msgtype` the root's `MsgType(35)` |
| Wire | `into_bytes` emits the restated row, so a FIX 4.2 report re-emits with the `OrderCapacity`, the `Parties` and the derived pairs behind what it stated; the entries carry both the source and its restatement |
| Idempotent | a message read back from its row is the same message: what one parse wrote is what the row states |
| Batch | inside the parse, so `parse_text_arrow_reader` lands restated rows and the [lifecycle](lifecycle.md) walks them |
| Bindings | reached through every `parse_*` door in all three languages |

A FIX 4.2 execution report, read as it was sent, which restates it as it builds it.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::holder::local::Folder;
    use yggdryl::{FixCodec, FixRegistry, Scalar, FieldPath};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    let reader = FixCodec::new(Arc::clone(&registry));

    // A cancelled partial fill (20=1, 150=1) of an agency order (47=A),
    // naming its broker (76) and client (109), the fill as LastShares (32).
    let line: &[u8] = b"8=FIX.4.2|35=8|37=O1|17=E1|20=1|150=1|39=1|55=AAPL|54=1|32=100|31=10.5|14=100|151=0|47=A|109=CLIENT1|76=BRKR|10=0|";
    let latest = reader.parse_line(line)?.next().expect("one frame")?;

    // ExecTransType Cancel states TradeCancel, but ExecType already stated
    // PartiallyFilled and a stated value stands - so the rule that answers
    // 150 is ExecType's own, folding the partial fill into Trade. The source
    // stays as it arrived.
    assert_eq!(latest.by_tag(150)?.as_str(), Some("F"));
    assert_eq!(latest.by_tag(20)?.as_str(), Some("1"));
    // Rule80A A is an agency order.
    assert_eq!(latest.by_tag(528)?.as_str(), Some("A"));
    assert_eq!(latest.by_tag(47)?.as_str(), Some("A"), "the source stays as read");
    // ExecBroker and ClientID are two parties, in tag order, and the
    // counter states the count.
    assert_eq!(latest.by_tag(453)?.as_i128(), Some(2));
    assert_eq!(latest.by_path(&FieldPath::from_str("parties[0].partyid")?)?, Scalar::from("BRKR"));
    assert_eq!(latest.by_path(&FieldPath::from_str("parties[0].partyrole")?)?, Scalar::from(1));
    assert_eq!(latest.by_path(&FieldPath::from_str("parties[1].partyid")?)?, Scalar::from("CLIENT1"));
    assert_eq!(latest.by_path(&FieldPath::from_str("parties[1].partyrole")?)?, Scalar::from(3));
    // LastShares is LastQty, reachable by either spelling.
    assert_eq!(latest.by_tag(32)?, Scalar::from(100.0_f64));
    assert_eq!(latest.by_name("LastShares")?, latest.by_tag(32)?);

    // What the message said of itself is its header's; the wire re-emits
    // the restated row behind what arrived.
    assert_eq!(latest.header().beginstring(), "FIX.4.2");
    let wire = latest.into_text('|')?;
    assert!(wire.starts_with("8=FIX.4.2|35=8|54=1|37=O1|17=E1|20=1|150=F|"), "{wire}");
    assert!(wire.contains("|528=A|453=2|448=BRKR|452=1|448=CLIENT1|452=3|"), "{wire}");
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    reader = FixCodec(registry)

    # A cancelled partial fill (20=1, 150=1) of an agency order (47=A),
    # naming its broker (76) and client (109), the fill as LastShares (32).
    line = b"8=FIX.4.2|35=8|37=O1|17=E1|20=1|150=1|39=1|55=AAPL|54=1|32=100|31=10.5|14=100|151=0|47=A|109=CLIENT1|76=BRKR|10=0|"
    read = next(reader.parse_line(line))
    assert read.by_name("version").as_py() == "4.2"
    assert read.by_tag(150).as_py() == "40PARTFILL", "read at 4.2"

    latest = reader.enrich_message(read)

    # ExecTransType Cancel states TradeCancel, but ExecType already stated
    # PartiallyFilled and a stated value stands, so ExecType's own rule folds
    # the partial fill into Trade. The source stays as it arrived.
    assert latest.by_tag(150).as_py() == "40TRADE"
    assert latest.by_tag(20).as_py() == "1"
    # Rule80A A is an agency order.
    assert latest.by_tag(528).as_py() == "A"
    # ExecBroker and ClientID are two parties, in tag order, and the
    # counter states the count.
    assert latest.by_tag(453).as_py() == 2
    assert latest.by_path("parties[0].partyid").as_py() == "BRKR"
    assert latest.by_path("parties[0].partyrole").as_py() == 1
    assert latest.by_path("parties[1].partyid").as_py() == "CLIENT1"
    assert latest.by_path("parties[1].partyrole").as_py() == 3
    # LastShares is LastQty, reachable by either spelling.
    assert latest.by_tag(32).as_py() == 100.0
    assert latest.by_name("LastShares") == latest.by_tag(32)

    # The version is what the line said; what the message said of itself,
    # the entries and the wire are untouched.
    assert latest.by_name("version").as_py() == "4.2"
    assert latest.by_tag(8).as_py() == "FIX.4.2"
    assert latest.entries() == read.entries()
    assert latest.into_bytes(ord("|")) == line
    assert reader.enrich_message(latest) == latest, "a second pass changes nothing"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const reader = new fix.FixCodec(registry)

    // A cancelled partial fill (20=1, 150=1) of an agency order (47=A),
    // naming its broker (76) and client (109), the fill as LastShares (32).
    const line = '8=FIX.4.2|35=8|37=O1|17=E1|20=1|150=1|39=1|55=AAPL|54=1|32=100|31=10.5|14=100|151=0|47=A|109=CLIENT1|76=BRKR|10=0|'
    const read = reader.parseLine(Buffer.from(line)).next().value
    assert.equal(read.byName('version').toJSON(), '4.2')
    assert.equal(read.byTag(150).toJSON(), '40PARTFILL', 'read at 4.2')

    const latest = reader.enrichMessage(read)

    // ExecTransType Cancel states TradeCancel, but ExecType already stated
    // PartiallyFilled and a stated value stands, so ExecType's own rule folds
    // the partial fill into Trade. The source stays as it arrived.
    assert.equal(latest.byTag(150).toJSON(), '40TRADE')
    assert.equal(latest.byTag(20).toJSON(), '1')
    // Rule80A A is an agency order.
    assert.equal(latest.byTag(528).toJSON(), 'A')
    // ExecBroker and ClientID are two parties, in tag order, and the
    // counter states the count.
    assert.equal(latest.byTag(453).toJSON(), 2)
    assert.equal(latest.byPath('parties[0].partyid').toJSON(), 'BRKR')
    assert.equal(latest.byPath('parties[0].partyrole').toJSON(), 1)
    assert.equal(latest.byPath('parties[1].partyid').toJSON(), 'CLIENT1')
    assert.equal(latest.byPath('parties[1].partyrole').toJSON(), 3)
    // LastShares is LastQty, reachable by either spelling.
    assert.equal(latest.byTag(32).toJSON(), 100)
    assert.ok(latest.byName('LastShares').equals(latest.byTag(32)))

    // The version is what the line said; what the message said of itself
    // and the wire are untouched.
    assert.equal(latest.byName('version').toJSON(), '4.2')
    assert.equal(latest.byTag(8).toJSON(), 'FIX.4.2')
    assert.equal(latest.intoBytes('|'.charCodeAt(0)).toString(), line)
    assert.ok(reader.enrichMessage(latest).equals(latest), 'a second pass changes nothing')
    ```

### What a held value is, to a rule

A rule's plan reads the row's typed values through the [expression grammar](../expression/grammar.md) and writes wire text, because that is what the specification's appendices are written in. The two meet at the condition and at the target.

| held value | in the `where` | so |
| --- | --- | --- |
| text, ASCII | compared as text | `rule80a = 'A'`; a `MultipleCharValue` holds several codes in one text, so `ExecInst` `G T` is asked with `contains(execinst, 'T')` |
| boolean | `'Y'` coerces to it | `OddLot` `Y` meets `oddlot = 'Y'` |
| integer, float, decimal | a literal coerces to the column's type | `repurchaseterm = '1'`; a join spells an integer with two digits through `substring(concat('0', cast(maturityday as utf8)), -2)`, which is how a day completes a month-year |
| temporal | itself | `onbehalfofsendingtime as hopsendingtime` lands the instant it is |
| a coded value | compared by the code's spelling | `exectype = '1'` meets the `1` a partial fill was read as, as `exectype in ('1', '2')` meets either |

A value written into a target is re-typed for the target's field through the codec's own text-to-typed reading: a constant `'1'` lands in `PartyRole(452)` as an integer, `'F'` in `ExecType(150)` as the text it is, `'A'` in `OrderCapacity(528)` likewise. A constant written over a multi-valued source replaces the token the condition named - `G T` restated at `T` is `G R`. A value the target cannot hold blocks the entry rather than landing as null.

## Edges

- `get_by_tag(9999)`, an unknown tag -> the root child named `9999` exactly, never `09999`; the miss allocates nothing.
- A tag two fields hold under different names (`OrderQty` and `Quantity`, both 38) -> `by_id` tells them apart, each id reaching its own child; a bare tag reaches one child, the one named by the registry's first holder where the row itself does not carry the tag.
- `by_id` with another name or another tag than the field's -> a miss, because an identifier names the pair exactly and never folds a tag onto a name it does not carry.
- `FixId::of(0, ..)` or `FixId::of(-1, ..)` -> refused, because a definition's tag is positive and 0 marks only an unresolved arrival entry; a message root the codec builds carries no `fix:branches`, because a message is not a dictionary member.
- `by_path("Parties.PartyID")` -> an error; a repeating group is a List of Structs, so a member needs the occurrence (`Parties[0].PartyID`), which is the spelling the registry takes too.
- A typed tag stated null at construction, or a holder stating nothing -> the lookup answers nothing: `get_by_tag(54)` on a report stating no side is `None`, `get_by_tag(SEQNUM_TAG_NAME.0)` on a message in no chain is `None`, and `get_by_tag(STATE_TAG_NAME.0)` on an order that reached no state is `None`.
- `set` with a name nothing reaches -> a typed absence naming the key, and the message unchanged; with a value the field refuses -> the value contract's refusal, and the message unchanged; `set_many` refuses all of its writes on the first refusal.
- `set` on a typed tag with a value its type refuses - text into `OrderQty`'s twin `qty`, a spelling outside the side's set - is silence: the holder keeps what it held. `set` with a `Null` on a row child -> a stated null, the child kept and made nullable; on a typed tag -> the fact cleared. `remove` -> the child gone or the fact cleared and its value answered, `None` for a key that reaches nothing; a cleared `crosscode` keeps the settled one, because the identity is re-settled from what the message states and the code, once named, stands.
- `FixMsg::new` / `with_registry` on a root stating `SendingTime(52)` -> the header's clock, marked stated; on one stating none -> one UTC-now read, marked not stated, so the wire omits it; a stated clock that is not an instant -> a located refusal.
- `set` twice under one key -> one child, the later value; a bare unknown tag written twice -> one decimal-named child.
- `from_row` on a row whose entries column holds something that is not an arrival entry - a folded entry without its members, a negative tag, a name or value that is not text - or a leaf the JSON reader cannot decode -> refused at the arrival path; on a schema without the column -> a message with the typed facts, no content and a wire of the header alone; on a row leaving `unix`, `creatunix`, `hashcode`, `crosshashcode`, `curruuid` or `crossuuid` null -> the schema's refusal, since the fixed row declares them required.
- Two children reaching one field, both stated and different (`lastqty` `50` beside `LastShares` `100`) -> both kept as they arrived; equal once re-typed, or one null -> one child.
- A child named by a tag's digits (`"32"`) that the registry knows -> re-expressed under the registry's field like any other; one it does not know (`"9999"`) -> kept exactly, name, datatype and value.
- A List no `fix:counter` heads, and any nested value that is not a repeating group -> kept exactly; only group occurrences are levels.
- A rule whose target holds a stated current code (`40=A|59=0`, a `TimeInForce` the message chose) -> blocked whole: `OrdType` stays `A` and no later entry answers for it; `59=7`, the rule's own value, is no obstacle.
- A rule that rewrote the source's own value (`ExecInst` `T` -> `R`) -> the new value is restated in turn (`R` is a `PegPriceType`), so one parse reaches what a second would find; a chain is bounded by the rules the field states.
- A `join` with a part unstated (`205=5` and no `200`) or a `from` whose tag is absent -> the entry fills nothing.
- A group fill whose constants match an existing occurrence (`ClearingFirm` made the role-4 party, `ClearingAccount` adds its sub-identifier) -> merged into it and the counter unchanged; a stated occurrence of the same role with another identifier -> the fill is blocked, the occurrence stands.
- A rule scoped by `:msgtype` on a message stating no `MsgType(35)` -> does not apply; one scoped by `:group` at the root -> does not apply.
- A removed field the specification replaced with nothing (`SendingDate(51)`) -> kept as read, and the registry still holds it under its own tag.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --lib fix::tests::a_message
    cargo test -p yggdryl --test fix codec::a_message_re_emits_from_its_entries_and_reads_back_equal
    cargo test -p yggdryl --test fix message::
    cargo test -p yggdryl --test fix latest::
    cargo test -p yggdryl --test fix latest::a_fix_42_execution_report_restates_at_the_dictionarys_newest_version
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/fix
    python/.venv/bin/python -m pytest python/tests/fix -k "message or scalar_value_and_field"
    python/.venv/bin/python -m pytest python/tests/fix -k latest
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/fix/fix.test.js
    node --test --test-name-pattern="message" node/tests/fix/fix.test.js
    node --test --test-name-pattern="latest" node/tests/fix/fix.test.js
    ```
