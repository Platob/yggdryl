# Message

`FixMsg` is a value plus the registry that types it: a root Struct field, its ordered row, and the linked registry.

## Contract

| item | contract |
| --- | --- |
| Owns | root Struct [`Field`](../types/field.md), the row as the `Scalar::Sequence` that root declares, the linked registry |
| Constructors | `FixMsg::new` links `FixRegistry::global()`; `FixMsg::with_registry` keeps the `Arc` it is given; `FixMsg::from_row` reads a [fixed row](#a-row-is-a-message-again) back, entries included |
| Writes | `set`, `set_many`, `with_value` and `remove` [change the row](#written-into-the-row) and never the entries; a key resolves as a lookup does, a value types through the registry's field, and a refusal leaves the message unchanged. `set_many` and `with_value` are Rust-only; Python and JavaScript expose `set` and `remove` |
| Validates | the row through `Field::scalar`, so a `Scalar::Record` input becomes that sequence |
| Borrows | `registry()`, `as_field()`, `as_value()` |
| Branch | derived, not declared: the root field's own `fix:branch`, resolved once at construction |
| Bare key tier | this message's branch, then the standard branch, and no further |
| Resolves through | the linked [registry](registry.md), never a private copy of its rules |
| Serialization | inherited: `into_json` renders the schema, [`into_json_scalar`](../text/json.md) the value, `from_json_scalar_with_field` reads it back typed, ordered and canonicalized against the same root |
| Restated | `into_latest` re-expresses the row at the [registry's newest version](#restated-at-the-dictionarys-newest-version) from the dictionary alone; the entries never change |
| Bindings | Rust, Python, JavaScript |

## Use

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::{DataType, FixCategory, FixMsg, FixRegistry, Scalar, from_json_scalar_with_field, into_json_scalar, FieldPath};

    let mut symbol = DataType::Utf8.required_field("Symbol");
    symbol.as_fix_mut().set_tag(55)?;
    symbol.as_fix_mut().set_aliases(["Ticker"])?;
    let mut qty = DataType::Int64.required_field("OrderQty");
    qty.as_fix_mut().set_tag(38)?;
    let mut party_id = DataType::Utf8.nullable_field("PartyID");
    party_id.as_fix_mut().set_tag(448)?;
    let mut count = DataType::Int32.required_field("NoPartyIDs");
    count.as_fix_mut().set_tag(453)?;
    let party = DataType::from_fields([party_id.clone()])?.required_field("Party");
    let mut parties = DataType::list(party.clone()).nullable_field("Parties");
    parties.as_fix_mut().set_counter(453)?;
    parties.as_fix_mut().set_component("Party")?;
    let mut registry = FixRegistry::from_fields([symbol.clone(), qty.clone(), count.clone(), party_id])?;
    registry.create_definition(FixCategory::Components, party)?;
    registry.create_definition(FixCategory::Groups, parties.clone())?;
    let registry = Arc::new(registry);

    // The root carries a tag no dictionary explains, under its rendered name.
    let root = DataType::from_fields([qty, symbol, count, parties, DataType::Utf8.nullable_field("9999")])?
        .required_field("NewOrderSingle");
    let value = Scalar::from_record([
        ("Symbol", Scalar::from("AAPL")),
        ("OrderQty", Scalar::from(100_i64)),
        ("NoPartyIDs", Scalar::from(1_i32)),
        ("Parties", Scalar::from_sequence([
            Scalar::from_record([("PartyID", Scalar::from("BROKER"))])?,
        ])),
        ("9999", Scalar::from("custom")),
    ])?;
    let msg = FixMsg::with_registry(Arc::clone(&registry), root.clone(), value)?;

    // The record became the ordered row the root declares.
    assert_eq!(msg.as_value().as_sequence().map(|row| row.len()), Some(5));
    assert_eq!(msg.by_tag(38)?, &Scalar::from(100_i64));
    assert_eq!(msg.by_name("ticker")?, &Scalar::from("AAPL"));
    assert_eq!(msg.by_path(&FieldPath::from_str("Parties[0].PartyID")?)?, &Scalar::from("BROKER"));
    assert_eq!(msg.by_tag(9999)?, &Scalar::from("custom"), "an unknown tag is retained");
    assert_eq!(msg.get(55), msg.get_by_tag(55));
    assert!(msg.value("Parties.PartyID").is_err(), "a group member needs its index");

    // The message's branch is the root's own, and an identifier is exact.
    assert_eq!(msg.branch(), &yggdryl::FixBranch::STANDARD);
    assert_eq!(
        msg.by_id(yggdryl::FixId::standard(38))?,
        &Scalar::from(100_i64)
    );
    assert!(msg.get_by_id(yggdryl::FixId::from_str("5001:cme")?).is_none());

    // Schema and value serialize through the paths every field and value share.
    let schema = root.clone().into_json()?;
    assert!(schema.contains("\"fix:tag\":\"55\""), "{schema}");
    let text = into_json_scalar(msg.as_value())?;
    let read = from_json_scalar_with_field(&text, &root)?;
    assert_eq!(&read, msg.as_value());
    assert_eq!(FixMsg::with_registry(registry, root, read)?, msg);
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType, Field, types
    from yggdryl.fix import STANDARD_BRANCH, FixMsg, FixRegistry

    symbol = Field("Symbol", "utf8", nullable=False)
    symbol.fix.tag = 55
    symbol.fix.aliases = ["Ticker"]
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

    # The mapping became the ordered row the root declares.
    assert len(message) == 5
    assert message.by_tag(38).as_py() == 100
    assert message.by_name("ticker").as_py() == "AAPL"
    assert message.by_path("Parties[0].PartyID").as_py() == "BROKER"
    assert message.by_tag(9999).as_py() == "custom", "an unknown tag is retained"
    assert message[55] == message.get_by_tag(55)
    with pytest.raises(KeyError):
        message.by_path("Parties.PartyID")  # a group member needs its index
    assert [name for name, _ in message] == ["OrderQty", "Symbol", "NoPartyIDs", "Parties", "9999"]

    # The message's branch is the root's own, and an identifier is exact.
    assert message.branch == STANDARD_BRANCH
    assert message.by_id("38:").as_py() == 100
    assert message.get_by_id("5001:cme") is None

    # The schema serializes through the path every field already has, and the
    # value the message holds names the same row.
    assert '"fix:tag":"55"' in root.into_json()
    assert FixMsg(root, message.value, registry) == message
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, Scalar, fields, fix } = require('yggdryl')

    const symbol = Field.from('Symbol: utf8 not null')
    symbol.fix.tag = 55
    symbol.fix.aliases = ['Ticker']
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

    // The plain object became the ordered row the root declares.
    assert.equal(message.value.kind, 'sequence')
    assert.equal(message.byTag(38).asJs(), 100)
    assert.equal(message.byName('ticker').asJs(), 'AAPL')
    assert.equal(message.byPath('Parties[0].PartyID').asJs(), 'BROKER')
    assert.equal(message.byTag(9999).asJs(), 'custom', 'an unknown tag is retained')
    assert.ok(message.get(55).equals(message.getByTag(55)))
    assert.throws(() => message.at('Parties.PartyID'), /a fix value/)
    assert.deepEqual(
      [...message].map(([name]) => name),
      ['OrderQty', 'Symbol', 'NoPartyIDs', 'Parties', '9999'],
    )

    // The message's branch is the root's own, and an identifier is exact.
    assert.equal(message.branch, fix.STANDARD_BRANCH)
    assert.equal(message.byId('38:').asJs(), 100)
    assert.equal(message.getById('5001:cme'), null)

    // Schema and value serialize through the paths every field and value share.
    const document = message.toJSON()
    assert.equal(document.field.dtype.fields[1].metadata['fix:tag'], '55')
    assert.deepEqual(document.value[1], 'AAPL')
    assert.ok(new fix.FixMsg(root, message.value, registry).equals(message))
    ```

## Resolution tier

A bare tag or name resolves in two steps and no further:

1. this message's own branch, when the tag is in `[FixId::USER_TAG_MIN, FixId::USER_TAG_MAX)`, or
   the message is already standard;
2. the standard branch.

`get_by_tag(5001)` finds the venue's own field, and `get_by_tag(35)` still finds `MsgType`.

## Accessors

| accessor | resolution |
| --- | --- |
| `get_by_tag` / `by_tag` | the tag through the tier to its canonical name, then the root child of that name; falls back to a root child named by the tag's decimal text |
| `get_by_id` / `by_id` | takes a `FixId` by value, names a dictionary exactly and does not tier, so a foreign branch misses |
| `get_by_name` / `by_name` | folds through the same tier to the registry's canonical spelling, then matches a root child exactly |
| `get_by_path` / `by_path` | the whole string as a name, then segment by segment: into a Struct child by name, into a List entry by a decimal index |
| `get` / `value` | takes a `FixKey` and redirects |

## Written into the row

A message is read once and then written to: enrichment fills what it implied, the lifecycle stamps what the stream implied, a restatement rewrites a retired spelling. All of them go through one door. `set` writes one value into the row, typed by the field the key reaches; `set_many` lands several with one rebuild, which is what a stream stamping three identities on every message wants; `with_value` is the consuming twin; `remove` takes a child out and answers what it held. Every one of them is row-only: the entries are what arrived, so `into_bytes` still re-emits the received line byte for byte and `digest` still answers the arrival record's.

| Key | Reaches |
| --- | --- |
| a tag the dictionary knows | the child carrying it, replaced where it stands; else appended under the dictionary's field - its canonical name, its datatype, its `fix:tag` - so a written child is indistinguishable from a stated one |
| a name the dictionary knows | the same field, through the [tier](#resolution-tier) every lookup resolves by |
| a name it does not know | the child spelled that way, exactly or under the fold every name resolves by, keeping that child's own field; nothing reached is refused, and the message stands |
| a tag it does not know | the child named by its decimal, else a nullable `utf8` child appended under it - what the builder does with an unknown tag |
| a `Null` value | a stated null: the child stays, nullable, holding nothing |

A written child keeps its position, so every reader already holding the row addresses it as before, and the tag index follows the one child that changed rather than being reread. A value the field refuses - text into `MsgSeqNum(34)` - is refused whole, and with `set_many` one refused write refuses them all.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::holder::local::Folder;
    use yggdryl::{FixCodec, FixRegistry, Scalar};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    let reader = FixCodec::new(Arc::clone(&registry));

    let line = b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|9999=x|10=0|";
    let mut message = reader.parse_fix_line(line)?;
    let children = message.as_field().fields().len();
    let at = message.as_field().index_of("symbol").expect("the symbol child");

    // Appended under the dictionary's field, typed by it, reached by tag or name.
    message.set(34, Scalar::from(7_i32))?;
    assert_eq!(message.by_tag(34)?.as_i128(), Some(7));
    assert_eq!(message.by_name("MsgSeqNum")?.as_i128(), Some(7));
    assert_eq!(message.as_field().fields().last().map(yggdryl::Field::name), Some("msgseqnum"));

    // Replaced where it stands: the position is kept, the value changes.
    message.set("Symbol", Scalar::from("MSFT"))?;
    assert_eq!(message.as_field().index_of("symbol"), Some(at));
    assert_eq!(message.by_tag(55)?.as_str(), Some("MSFT"));

    // A tag no dictionary explains is kept under its decimal spelling.
    message.set(7777, Scalar::from("custom"))?;
    assert_eq!(message.by_tag(7777)?.as_str(), Some("custom"));

    // A name nothing reaches is refused, and the row stands as it was.
    assert!(message.set("nosuchfield", Scalar::from("x")).is_err());
    assert_eq!(message.as_field().fields().len(), children + 2);

    // Removed, and the value answered; the other tags still reach their children.
    assert_eq!(message.remove(54).as_ref().and_then(Scalar::as_str), Some("1"));
    assert_eq!(message.get_by_tag(54), None);
    assert_eq!(message.remove("nosuchfield"), None);
    assert_eq!(message.by_tag(11)?.as_str(), Some("A1"));

    // Only the row changed: the wire comes back byte for byte.
    assert_eq!(message.into_bytes(b'|'), line);
    assert_eq!(message.entries(), reader.parse_fix_line(line)?.entries());
    ```

=== "Python"

    ```python
    from pathlib import Path

    import pytest

    from yggdryl.fix import FixCodec, FixRegistry

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    reader = FixCodec(registry)

    line = b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|9999=x|10=0|"
    message = reader.parse_fix_line(line)
    children = len(message)
    names = [name for name, _ in message]

    # Appended under the dictionary's field, typed by it, reached by tag or name.
    message.set(34, 7)
    assert message.by_tag(34).as_py() == 7
    assert message.by_name("MsgSeqNum").as_py() == 7
    assert [name for name, _ in message][-1] == "msgseqnum"

    # Replaced where it stands: the position is kept, the value changes.
    message.set("Symbol", "MSFT")
    assert [name for name, _ in message].index("symbol") == names.index("symbol")
    assert message.by_tag(55).as_py() == "MSFT"

    # A tag no dictionary explains is kept under its decimal spelling.
    message.set(7777, "custom")
    assert message.by_tag(7777).as_py() == "custom"

    # A name nothing reaches is refused, and the row stands as it was.
    with pytest.raises((KeyError, ValueError)):
        message.set("nosuchfield", "x")
    assert len(message) == children + 2

    # Removed, and the value answered; the other tags still reach their children.
    assert message.remove(54).as_py() == "1"
    assert message.get_by_tag(54) is None
    assert message.remove("nosuchfield") is None
    assert message.by_tag(11).as_py() == "A1"

    # Only the row changed: the wire comes back byte for byte.
    assert message.into_bytes(ord("|")) == line
    assert message.entries() == reader.parse_fix_line(line).entries()
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

    // Appended under the dictionary's field, typed by it, reached by tag or name.
    message.set(34, 7)
    assert.equal(message.byTag(34).asJs(), 7)
    assert.equal(message.byName('MsgSeqNum').asJs(), 7)
    assert.equal([...message].map(([name]) => name).at(-1), 'msgseqnum')

    // Replaced where it stands: the position is kept, the value changes.
    message.set('Symbol', 'MSFT')
    assert.equal([...message].map(([name]) => name).indexOf('symbol'), names.indexOf('symbol'))
    assert.equal(message.byTag(55).asJs(), 'MSFT')

    // A tag no dictionary explains is kept under its decimal spelling.
    message.set(7777, 'custom')
    assert.equal(message.byTag(7777).asJs(), 'custom')

    // A name nothing reaches is refused, and the row stands as it was.
    assert.throws(() => message.set('nosuchfield', 'x'))
    assert.equal(message.size, children + 2)

    // Removed, and the value answered; the other tags still reach their children.
    assert.equal(message.remove(54).asJs(), '1')
    assert.equal(message.getByTag(54), null)
    assert.equal(message.remove('nosuchfield'), null)
    assert.equal(message.byTag(11).asJs(), 'A1')

    // Only the row changed: the wire comes back byte for byte.
    assert.equal(Buffer.from(message.intoBytes(124)).toString(), line)
    ```

## A row is a message again

`from_row` is the inverse of [`into_row`](capture.md#a-column-is-filled-by-the-tag-its-field-carries): the message whose root is the schema and whose value is the row, checked and canonicalized as `with_registry` checks one, so every column is a child under the name the schema gave it and every lookup reaches it by tag as it reaches a parsed message's. The branch is the schema's own `fix:branch`. The entries are rebuilt from the `nofixentries` column - every level the row materialized, and the leaf the deepest level folded into decoded through the crate's own JSON reader - so `into_bytes` re-emits the line the row was read from and `digest` answers what it answered; a row without that column has no entries. Byte for byte over every capture this crate is tested against, and exact for any entry whose bytes are text - a `data` field carrying bytes no text holds reaches a `Utf8` column as the decode of them, so the message that row makes re-emits the decode and `anomalies` reports the `Lossy` that says so. Nothing is parsed again, which is what makes a [batch of rows a stream of messages](arrow.md#rows-are-messages-again-and-messages-rows) at the cost of the values it already holds.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::holder::local::Folder;
    use yggdryl::{FixCodec, FixMsg, FixRegistry, fix_schema};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    let reader = FixCodec::new(Arc::clone(&registry));
    let schema = fix_schema(&registry, "fix")?;

    let line = b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|9999=x|10=0|";
    let order = reader.parse_fix_line(line)?;
    let row = order.into_row(&schema)?;

    let held = FixMsg::from_row(Arc::clone(&registry), &schema, &row)?;
    // The root is the schema; the message is the same, reached the same way.
    assert_eq!(held.as_field(), &schema);
    assert_eq!(held.by_tag(55)?, order.by_tag(55)?);
    assert_eq!(held.version(), order.version());
    assert_eq!(held.entries(), order.entries());
    assert_eq!(held.digest(), order.digest());
    assert_eq!(held.into_bytes(b'|'), line);
    // And the row it came from is the row it makes.
    assert_eq!(held.into_row(&schema)?, row);
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixMsg, FixRegistry, fix_schema

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    reader = FixCodec(registry)
    schema = fix_schema(registry, "fix")

    line = b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|9999=x|10=0|"
    order = reader.parse_fix_line(line)
    row = order.into_row(schema)

    held = FixMsg.from_row(schema, row, registry)
    # The root is the schema; the message is the same, reached the same way.
    assert held.field == schema
    assert held.by_tag(55) == order.by_tag(55)
    assert held.entries() == order.entries()
    assert held.digest() == order.digest()
    assert held.into_bytes(ord("|")) == line
    # And the row it came from is the row it makes.
    assert held.into_row(schema) == row
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const reader = new fix.FixCodec(registry)
    const schema = fix.schema(registry, 'fix')

    const line = '8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|9999=x|10=0|'
    const order = reader.parseFixLine(Buffer.from(line))
    const row = order.intoRow(schema)

    const held = fix.FixMsg.fromRow(schema, row, registry)
    // The root is the schema; the message is the same, reached the same way.
    assert.ok(held.field.equals(schema))
    assert.ok(held.byTag(55).equals(order.byTag(55)))
    assert.deepEqual(held.digest(), order.digest())
    assert.equal(Buffer.from(held.intoBytes(124)).toString(), line)
    // And the row it came from is the row it makes.
    assert.ok(held.intoRow(schema).equals(row))
    ```

## Restated at the dictionary's newest version

A capture holds what each session spoke: a FIX 4.2 report states its fill as `LastShares`, its broker as `ExecBroker(76)`, its capacity as `Rule80A(47)` and a partial fill as `ExecType(150)` `1` - four things the newest specification spells as `LastQty`, a `Parties` occurrence, `OrderCapacity(528)` and `Trade`. `into_latest` restates the message once, from what the dictionary itself says: the registry's field for every tag, the aliases and lineage that reach it, and the [`fix:replacements`](registry.md#a-field-carries-what-replaced-it) each retired field or value carries. Nothing in Rust holds a table of rules, so a registry edit is a rule edit.

| item | contract |
| --- | --- |
| Row only | the entries are what arrived and are carried through untouched, so `into_bytes` re-emits the received line byte for byte and `anomalies` answers the same; `BeginString(8)` stays what the message said of itself |
| Canonicalizes | every child the registry knows - by its `fix:tag`, else its name or alias, else the decimal tag its name spells - is re-expressed under the registry's field: canonical name, datatype, tag, in the position it held; a child no dictionary knows stays exactly as it is |
| Merges | children reaching one field become one: the canonical-named child's value when stated, else the first stated among the rest; a child whose stated value disagrees with the kept one is left in place, so nothing that arrived is lost |
| Restates | each child whose field carries `fix:replacements`, in ascending tag order, by the first entry whose `msgtypes`, `in` and `when` hold; a group fill makes or completes one occurrence and sets its counter |
| All or nothing | every target an entry fills is computed and checked before any is written; one target that cannot take its value blocks the whole entry, and no later entry fills in for it |
| Never overwrites | a stated current value: a target takes a value only when it is absent, null, already equal, or holds a code its set no longer declares at the registry's newest version; the source field itself is the one exception, because it is what is being restated |
| Keeps | a removed field the specification named no replacement for, and the source of every rule that did not write it - the row says what was sent and what it means |
| Levels | the root, then every occurrence of every repeating group to any depth, each canonicalized and restated in turn; `in` is compared with the occurrence's group, `msgtypes` with the root's `MsgType(35)` |
| Stamps | the crate's `version` (65001) with `registry.newest()`, replacing a stated one or appending; a registry no field dates stamps nothing |
| Idempotent | a second pass answers an equal message: what one pass wrote is what the next finds stated |
| Batch | a [stage](arrow.md#a-pin-is-on-the-codec-a-stage-is-a-call) the caller composes over `messages` and `arrow_reader`, after enrichment and before the lifecycle stamp where both are wanted, so an enriched value is a stated one to the rules and the chain reads the restated row |
| Bindings | Rust `into_latest`, Python `into_latest`, JavaScript `intoLatest` |

A FIX 4.2 execution report, read as it was sent and then restated. The entries, the wire and the anomalies are the same before and after, and a second pass changes nothing.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::holder::local::Folder;
    use yggdryl::types::State;
    use yggdryl::{FixCodec, FixRegistry, Scalar, FieldPath};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    let reader = FixCodec::new(Arc::clone(&registry));

    // A cancelled partial fill (20=1, 150=1) of an agency order (47=A),
    // naming its broker (76) and client (109), the fill as LastShares (32).
    let line: &[u8] = b"8=FIX.4.2|35=8|37=O1|17=E1|20=1|150=1|39=1|55=AAPL|54=1|32=100|31=10.5|14=100|151=0|47=A|109=CLIENT1|76=BRKR|10=0|";
    let read = reader.parse_line(line)?.next().expect("one frame")?;
    assert_eq!(read.version().map(|version| version.to_string()), Some("4.2".to_owned()));
    let partial = State::from_spelling("1").expect("a lifecycle code");
    assert_eq!(read.by_tag(150)?.as_str(), Some(partial.as_str()), "read at 4.2");

    let latest = read.clone().into_latest()?;

    // ExecTransType Cancel wrote ExecType TradeCancel over the retired
    // PartiallyFilled before ExecType's own rule read it; the source stays.
    let cancel = State::from_spelling("H").expect("a lifecycle code");
    assert_eq!(latest.by_tag(150)?.as_str(), Some(cancel.as_str()));
    assert_eq!(latest.by_tag(20)?.as_str(), Some("1"));
    // Rule80A A is an agency order.
    assert_eq!(latest.by_tag(528)?.as_str(), Some("A"));
    // ExecBroker and ClientID are two parties, in tag order, and the
    // counter states the count.
    assert_eq!(latest.by_tag(453)?.as_i128(), Some(2));
    assert_eq!(latest.by_path(&FieldPath::from_str("parties[0].partyid")?)?, &Scalar::from("BRKR"));
    assert_eq!(latest.by_path(&FieldPath::from_str("parties[0].partyrole")?)?, &Scalar::from(1));
    assert_eq!(latest.by_path(&FieldPath::from_str("parties[1].partyid")?)?, &Scalar::from("CLIENT1"));
    assert_eq!(latest.by_path(&FieldPath::from_str("parties[1].partyrole")?)?, &Scalar::from(3));
    // LastShares is LastQty, reachable by either spelling.
    assert_eq!(latest.by_tag(32)?, &Scalar::from(100.0_f64));
    assert_eq!(latest.by_name("LastShares")?, latest.by_tag(32)?);

    // The version is the dictionary's newest; what the message said of
    // itself, the entries and the wire are untouched.
    let newest = registry.newest().expect("a dated dictionary").version();
    assert_eq!(latest.version(), Some(newest));
    assert_eq!(latest.by_tag(8)?.as_str(), Some("FIX.4.2"));
    assert_eq!(latest.entries(), read.entries());
    assert_eq!(latest.into_bytes(b'|'), line);
    assert_eq!(latest.clone().into_latest()?, latest, "a second pass changes nothing");
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

    latest = read.into_latest()

    # ExecTransType Cancel wrote ExecType TradeCancel over the retired
    # PartiallyFilled before ExecType's own rule read it; the source stays.
    assert latest.by_tag(150).as_py() == "40TRDCXL"
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

    # The version is the dictionary's newest; what the message said of
    # itself, the entries and the wire are untouched.
    assert latest.by_name("version").as_py() == "5.0.2"
    assert latest.by_tag(8).as_py() == "FIX.4.2"
    assert latest.entries() == read.entries()
    assert latest.into_bytes(ord("|")) == line
    assert latest.into_latest() == latest, "a second pass changes nothing"
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

    const latest = read.intoLatest()

    // ExecTransType Cancel wrote ExecType TradeCancel over the retired
    // PartiallyFilled before ExecType's own rule read it; the source stays.
    assert.equal(latest.byTag(150).toJSON(), '40TRDCXL')
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

    // The version is the dictionary's newest; what the message said of
    // itself and the wire are untouched.
    assert.equal(latest.byName('version').toJSON(), '5.0.2')
    assert.equal(latest.byTag(8).toJSON(), 'FIX.4.2')
    assert.equal(latest.intoBytes('|'.charCodeAt(0)).toString(), line)
    assert.ok(latest.intoLatest().equals(latest), 'a second pass changes nothing')
    ```

### What a held value is, to a rule

A rule reads and writes wire text, because that is what the specification's appendices are written in; the row holds typed values. One reading joins them.

| held value | reads as | so |
| --- | --- | --- |
| text, ASCII | itself | `when` equals it, or one of its space-separated tokens does - `ExecInst` `G T` meets a `when` of `T` |
| boolean | `Y` / `N` | `OddLot` `Y` meets a `when` of `Y` |
| integer, float, decimal | its decimal | an `int8` `MaturityDay` of `5` joins a month-year as `05` - a `join` spells an integer part with two digits |
| temporal | its canonical rendering | `OnBehalfOfSendingTime` lands in `HopSendingTime` as the instant it is |
| `state` | never rendered back to a code | `when` matches when `State::from_spelling(when)` is the state held, so `1` and `PartiallyFilled` both meet a `40PARTFILL` |

A value written into a target is re-typed for the target's field through the codec's own text-to-typed reading: a constant `1` lands in `PartyRole(452)` as an integer, `F` in `ExecType(150)` as the `Trade` state, `A` in `OrderCapacity(528)` as the text it is. A value the target cannot hold blocks the entry rather than landing as null.

## Edges

- A root whose `fix:branch` is malformed -> typed error at construction, never a silent miss later.
- `get_by_tag(9999)`, an unknown tag -> the root child named `9999` exactly, never `09999`; the miss allocates nothing.
- A bare tag outside `[FixId::USER_TAG_MIN, FixId::USER_TAG_MAX)` on a non-standard message -> only the standard branch is tried.
- `by_id` on a foreign branch -> a miss, because an identifier never tiers.
- `by_path("Parties.PartyID")` -> an error; a repeating group is a List of Structs, so a member needs the occurrence (`Parties[0].PartyID`), which is the spelling the registry takes too.
- `set` with a name nothing reaches -> a typed absence naming the key, and the message unchanged; with a value the field refuses -> the value contract's refusal, and the message unchanged; `set_many` refuses all of its writes on the first refusal.
- `set` with a `Null` -> a stated null, the child kept and made nullable; `remove` -> the child gone and its value answered, `None` for a key that reaches nothing.
- `set` twice under one key -> one child, the later value; a bare unknown tag written twice -> one decimal-named child.
- `from_row` on a row whose entries column holds something that is not an arrival entry, or a leaf the JSON reader cannot decode -> refused; on a schema without the column -> a message with no entries and an empty wire.
- `into_latest` on a message whose registry dates no field -> every rule still applies, nothing is stamped, and `version()` still reads `BeginString(8)`.
- Two children reaching one field, both stated and different (`lastqty` `50` beside `LastShares` `100`) -> both kept as they arrived; equal once re-typed, or one null -> one child.
- A child named by a tag's digits (`"32"`) that the registry knows -> re-expressed under the registry's field like any other; one it does not know (`"9999"`) -> kept exactly, name, datatype and value.
- A List no `fix:counter` heads, and any nested value that is not a repeating group -> kept exactly; only group occurrences are levels.
- A rule whose target holds a stated current code (`40=A|59=0`, a `TimeInForce` the message chose) -> blocked whole: `OrdType` stays `A` and no later entry answers for it; `59=7`, the rule's own value, is no obstacle.
- A rule that rewrote the source's own value (`ExecInst` `T` -> `R`) -> the new value is restated in turn (`R` is a `PegPriceType`), so one pass reaches what a second would find; a chain is bounded by the rules the field states.
- A `join` with a part unstated (`205=5` and no `200`) or a `from` whose tag is absent -> the entry fills nothing.
- A group fill whose constants match an existing occurrence (`ClearingFirm` made the role-4 party, `ClearingAccount` adds its sub-identifier) -> merged into it and the counter unchanged; a stated occurrence of the same role with another identifier -> the fill is blocked, the occurrence stands.
- A rule scoped by `msgtypes` on a message stating no `MsgType(35)` -> does not apply; one scoped by `in` at the root -> does not apply.
- A removed field the specification replaced with nothing (`SendingDate(51)`) -> kept as read, and `get_field_at(4.4, 51)` still answers none.
- Over a batch -> `into_latest` is a call between `messages` and `arrow_reader`, so where it runs relative to `enrich_messages` and `lifecycle` is where it is written; after enrichment an enriched value is a stated one to the rules, and before the lifecycle the chain reads the restated row.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --lib fix::tests::a_message
    cargo test -p yggdryl --test fix codec::a_message_re_emits_from_its_entries_and_reads_back_equal
    cargo test -p yggdryl --test fix message::
    cargo test -p yggdryl --test fix latest::
    cargo test -p yggdryl --test fix latest::a_fix_42_execution_report_restates_at_the_dictionarys_newest_version
    cargo bench -p yggdryl --bench fix -- fix/pipeline/into_latest
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
