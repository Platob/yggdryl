# Message

`FixMsg` is a value plus the registry that types it: a root Struct field, its ordered row, and the linked registry.

## Contract

| item | contract |
| --- | --- |
| Owns | root Struct [`Field`](../types/field.md), the row as the `Scalar::Sequence` that root declares, the linked registry |
| Constructors | `FixMsg::new` links `FixRegistry::global()`; `FixMsg::with_registry` keeps the `Arc` it is given |
| Validates | the row through `Field::validate_value` and `Field::canonicalize_value`, so a `Scalar::Record` input becomes that sequence |
| Borrows | `registry()`, `as_field()`, `as_value()` |
| Branch | derived, not declared: the root field's own `fix:branch`, resolved once at construction |
| Bare key tier | this message's branch, then the standard branch, and no further |
| Resolves through | the linked [registry](registry.md), never a private copy of its rules |
| Serialization | inherited: `into_json` renders the schema, [`into_json_scalar`](../text/json.md) the value, `from_json_scalar_with_field` reads it back typed, ordered and canonicalized against the same root |
| Bindings | Rust, Python, JavaScript |

## Use

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::{DataType, FixMsg, FixRegistry, Scalar, from_json_scalar_with_field, into_json_scalar};

    let mut symbol = DataType::Utf8.required_field("Symbol");
    symbol.as_fix_mut().set_tag(55)?;
    symbol.as_fix_mut().set_aliases(["Ticker"])?;
    let mut qty = DataType::Int64.required_field("OrderQty");
    qty.as_fix_mut().set_tag(38)?;
    let mut party_id = DataType::Utf8.nullable_field("PartyID");
    party_id.as_fix_mut().set_tag(448)?;
    let mut parties = DataType::list(DataType::from_fields([party_id])?.required_field("item"))
        .nullable_field("NoPartyIDs");
    parties.as_fix_mut().set_tag(453)?;
    let registry = Arc::new(FixRegistry::from_fields([symbol.clone(), qty.clone(), parties.clone()])?);

    // The root carries a tag no dictionary explains, under its rendered name.
    let root = DataType::from_fields([qty, symbol, parties, DataType::Utf8.nullable_field("9999")])?
        .required_field("NewOrderSingle");
    let value = Scalar::from_record([
        ("Symbol", Scalar::from("AAPL")),
        ("OrderQty", Scalar::from(100_i64)),
        ("NoPartyIDs", Scalar::from_sequence([
            Scalar::from_record([("PartyID", Scalar::from("BROKER"))])?,
        ])),
        ("9999", Scalar::from("custom")),
    ])?;
    let msg = FixMsg::with_registry(Arc::clone(&registry), root.clone(), value)?;

    // The record became the ordered row the root declares.
    assert_eq!(msg.as_value().as_sequence().map(|row| row.len()), Some(4));
    assert_eq!(msg.by_tag(38)?, &Scalar::from(100_i64));
    assert_eq!(msg.by_name("ticker")?, &Scalar::from("AAPL"));
    assert_eq!(msg.by_path("NoPartyIDs.0.PartyID")?, &Scalar::from("BROKER"));
    assert_eq!(msg.by_tag(9999)?, &Scalar::from("custom"), "an unknown tag is retained");
    assert_eq!(msg.get(55), msg.get_by_tag(55));
    assert!(msg.value("NoPartyIDs.PartyID").is_err(), "a group member needs its index");

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
    item = Field("item", DataType.from_fields([party_id]), nullable=False)
    parties = types.list("NoPartyIDs", item)
    parties.fix.tag = 453
    registry = FixRegistry.from_fields([symbol, qty, parties])

    # The root carries a tag no dictionary explains, under its rendered name.
    root = Field(
        "NewOrderSingle",
        DataType.from_fields([qty, symbol, parties, Field("9999", "utf8")]),
        nullable=False,
    )
    message = FixMsg(
        root,
        {
            "Symbol": "AAPL",
            "OrderQty": 100,
            "NoPartyIDs": [{"PartyID": "BROKER"}],
            "9999": "custom",
        },
        registry,
    )

    # The mapping became the ordered row the root declares.
    assert len(message) == 4
    assert message.by_tag(38).as_py() == 100
    assert message.by_name("ticker").as_py() == "AAPL"
    assert message.by_path("NoPartyIDs.0.PartyID").as_py() == "BROKER"
    assert message.by_tag(9999).as_py() == "custom", "an unknown tag is retained"
    assert message[55] == message.get_by_tag(55)
    with pytest.raises(KeyError):
        message.by_path("NoPartyIDs.PartyID")  # a group member needs its index
    assert [name for name, _ in message] == ["OrderQty", "Symbol", "NoPartyIDs", "9999"]

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
    const parties = fields.list('NoPartyIDs', fields.struct('item', [partyId], { nullable: false }))
    parties.fix.tag = 453
    const registry = fix.FixRegistry.fromFields([symbol, qty, parties])

    // The root carries a tag no dictionary explains, under its rendered name.
    const root = fields.struct(
      'NewOrderSingle',
      [qty, symbol, parties, Field.from('9999: utf8')],
      { nullable: false },
    )
    const message = new fix.FixMsg(
      root,
      {
        Symbol: 'AAPL',
        OrderQty: 100n,
        NoPartyIDs: [{ PartyID: 'BROKER' }],
        9999: 'custom',
      },
      registry,
    )

    // The plain object became the ordered row the root declares.
    assert.equal(message.value.kind, 'sequence')
    assert.equal(message.byTag(38).asJs(), 100)
    assert.equal(message.byName('ticker').asJs(), 'AAPL')
    assert.equal(message.byPath('NoPartyIDs.0.PartyID').asJs(), 'BROKER')
    assert.equal(message.byTag(9999).asJs(), 'custom', 'an unknown tag is retained')
    assert.ok(message.get(55).equals(message.getByTag(55)))
    assert.throws(() => message.at('NoPartyIDs.PartyID'), /a fix value/)
    assert.deepEqual(
      [...message].map(([name]) => name),
      ['OrderQty', 'Symbol', 'NoPartyIDs', '9999'],
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

## Edges

- A root whose `fix:branch` is malformed -> typed error at construction, never a silent miss later.
- `get_by_tag(9999)`, an unknown tag -> the root child named `9999` exactly, never `09999`; the miss allocates nothing.
- A bare tag outside `[FixId::USER_TAG_MIN, FixId::USER_TAG_MAX)` on a non-standard message -> only the standard branch is tried.
- `by_id` on a foreign branch -> a miss, because an identifier never tiers.
- `by_path("NoPartyIDs.PartyID")` -> an error; a repeating group is a List of Structs, so a member needs the entry's index (`NoPartyIDs.0.PartyID`).

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --lib fix::tests::a_message
    cargo test -p yggdryl --test fix serialization_is_inherited
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/fix
    python/.venv/bin/python -m pytest python/tests/fix -k "message or scalar_value_and_field"
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/fix/fix.test.js
    node --test --test-name-pattern="message" node/tests/fix/fix.test.js
    ```
