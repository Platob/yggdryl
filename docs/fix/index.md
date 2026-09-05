# FIX

FIX field definitions are ordinary fields: a `fix:` vocabulary on a [`Field`](../types/field.md), a [registry](registry.md) resolving them, [shards](store.md) persisting them, and a [message](message.md) typed against one.

## Pages

| Page | Purpose |
| --- | --- |
| [FIX](index.md) | This page: vocabulary, `FixBranch`, `FixId`, nesting |
| [Registry](registry.md) | `FixRegistry`: tiered resolution, index halves, `FixKey`, mutation, the process-wide default |
| [Store](store.md) | Shard trees under one `IOBase` folder, `from_handle`, `write_into`, the tracked seed |
| [Message](message.md) | `FixMsg`: root Struct plus row and registry, derived branch, accessors, JSON |

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `FixField` / `FixFieldMut` (`as_fix()` / `as_fix_mut()`), `FixBranch`, `FixId`; no second field class |
| Keys | `fix:branch`, `fix:tag`, `fix:tags`, `fix:aliases`, `fix:description`; name, datatype and `display` stay the field's own |
| Branch | ASCII letter first, then letters, digits, `-`, `.`, `_`; at most `FixBranch::MAX_LENGTH` (23) bytes; case folded once on parse |
| Standard branch | `standard`; an absent key means it, and setting it removes the key |
| Identity | `FixId` = `branch:tag`, derived on every read, never stored; `None` without a tag; ordered branch-major, then tag |
| Standard-tag rule | Tag below `FixId::STANDARD_TAG_LIMIT` (5000) forces the standard branch, one-way; `FixId::from_parts` refuses at every door |
| List properties | Comma-separated text; `aliases()` lazy slices, `tags()` a parsed `Vec`; empty list removes the key |
| Errors | `InvalidMetadataValue` naming the full key; the field stays unchanged |
| Nesting | Struct = component, List of that Struct = group; `dtype().is_nested()` picks the [registry](registry.md) half and the [store](store.md) tree |
| Bindings | Python `field.fix` and [`yggdryl.fix`](../extensions/python.md); JavaScript `field.fix` and the [`fix` namespace](../extensions/javascript.md); branch and id cross as text |

## Use

=== "Rust"

    ```rust
    use yggdryl::DataType;

    let mut field = DataType::decimal128(20, 8)?.nullable_field("OrderQty");
    field.as_fix_mut().set_tag(38)?;
    field.as_fix_mut().set_aliases(["Qty", "Quantity"])?;
    field.as_fix_mut().set_description("Quantity ordered.")?;
    field.set_display("Order quantity")?;

    assert_eq!(field.as_fix().tag()?, Some(38));
    assert_eq!(field.as_fix().tags()?, Vec::<i32>::new());
    assert_eq!(field.as_fix().aliases().collect::<Vec<_>>(), ["Qty", "Quantity"]);
    assert_eq!(field.as_fix().description(), Some("Quantity ordered."));
    // Stored as ordinary namespaced text, in the one metadata map.
    assert_eq!(field.get_metadata("fix:aliases"), Some("Qty,Quantity"));
    assert_eq!(field.as_fix().len(), 3);

    // A refusal names the full key and leaves the field unchanged.
    let error = field.as_fix_mut().set_tags(&[152, 152]).unwrap_err();
    assert!(error.to_string().contains("fix:tags"), "{error}");
    assert!(!field.has_metadata("fix:tags"));
    let error = field.as_fix_mut().set_tag(-1).unwrap_err();
    assert!(error.to_string().contains("fix:tag"), "{error}");
    assert_eq!(field.as_fix().tag()?, Some(38));
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import Field

    field = Field("OrderQty", "decimal128(20, 8)")
    field.fix.tag = 38
    field.fix.aliases = ["Qty", "Quantity"]
    field.fix.description = "Quantity ordered."
    field.set_display("Order quantity")

    assert field.fix.tag == 38
    assert field.fix.tags == []
    assert field.fix.aliases == ["Qty", "Quantity"]
    assert field.fix.description == "Quantity ordered."
    # Stored as ordinary namespaced text, in the one metadata map.
    assert field.metadata["fix:aliases"] == "Qty,Quantity"
    assert len(field.fix) == 3

    # A refusal names the full key and leaves the field unchanged.
    with pytest.raises(ValueError, match="fix:tags"):
        field.fix.tags = [152, 152]
    assert "fix:tags" not in field.metadata
    with pytest.raises(ValueError, match="fix:tag"):
        field.fix.tag = -1
    assert field.fix.tag == 38

    # A bool is never a tag, and one outside i32 is never narrowed.
    with pytest.raises(TypeError, match="not bool"):
        field.fix.tag = True
    with pytest.raises(OverflowError):
        field.fix.tag = 2**31

    # An empty list removes a list property; `del` removes any of them.
    field.fix.aliases = []
    assert field.fix.aliases == []
    del field.fix["tag"]
    assert field.fix.tag is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field } = require('yggdryl')

    const field = Field.from('OrderQty: decimal128(20, 8)')
    field.fix.tag = 38
    field.fix.aliases = ['Qty', 'Quantity']
    field.fix.description = 'Quantity ordered.'
    field.setDisplay('Order quantity')

    assert.equal(field.fix.tag, 38)
    assert.deepEqual(field.fix.tags, [])
    assert.deepEqual(field.fix.aliases, ['Qty', 'Quantity'])
    assert.equal(field.fix.description, 'Quantity ordered.')
    // Stored as ordinary namespaced text, in the one metadata map.
    assert.equal(field.get('fix:aliases'), 'Qty,Quantity')
    assert.equal(field.fix.size, 3)

    // A refusal names the full key and leaves the field unchanged.
    assert.throws(() => {
      field.fix.tags = [152, 152]
    }, /fix:tags/)
    assert.equal(field.has('fix:tags'), false)
    assert.throws(() => {
      field.fix.tag = -1
    }, /fix:tag/)
    assert.equal(field.fix.tag, 38)

    // A tag crosses as a number and is never narrowed, and the vocabulary is
    // answered only by the fix view.
    assert.throws(() => {
      field.fix.tag = 2 ** 31
    }, /signed 32-bit integer/)
    assert.throws(() => field.iceberg.tag, TypeError)

    // An empty array removes a list property; `delete` removes any of them.
    field.fix.aliases = []
    assert.deepEqual(field.fix.aliases, [])
    field.fix.delete('tag')
    assert.equal(field.fix.tag, null)
    ```

## The vocabulary is metadata

The namespace adds only what FIX states beyond a field, and a caller never spells `fix:`.

| Property | Key | Type | Meaning |
| --- | --- | --- | --- |
| `branch` | `fix:branch` | `FixBranch` | owning dictionary; absent means standard |
| `tag` | `fix:tag` | `i32` | canonical tag, never negative |
| `tags` | `fix:tags` | ordered `i32` list | alternate tags, highest priority first |
| `aliases` | `fix:aliases` | ordered name list | alternate names, highest priority first |
| `description` | `fix:description` | text | the specification's wording |

## Identity is a branch and a tag

`standard:35` to `cme:5001` needs set-tag-then-set-branch, and the reverse move the opposite order. `set_id` writes both halves at once and restores the prior branch when the tag write fails.

=== "Rust"

    ```rust
    use yggdryl::{DataType, FixId, FixBranch};

    let cme = FixBranch::from_str("CME")?;
    assert_eq!(cme.as_str(), "cme", "folded once, on the way in");
    assert!(FixBranch::from_str("2cme").is_err());

    let mut trade = DataType::Utf8.nullable_field("TradeID");
    // Absent means standard, and there is no identity without a tag.
    assert_eq!(trade.as_fix().branch()?, FixBranch::STANDARD);
    assert_eq!(trade.as_fix().id()?, None);

    trade.as_fix_mut().set_id(&FixId::from_parts(cme.clone(), 5001)?)?;
    assert_eq!(trade.as_fix().id()?.map(|id| id.to_string()), Some("cme:5001".into()));
    assert_eq!(trade.get_metadata("fix:branch"), Some("cme"));
    assert_eq!(trade.as_fix().id()?, Some(FixId::from_str("cme:5001")?));

    // A tag the FIX specification assigns belongs to the standard branch,
    // at every door, and a refusal leaves the field unchanged.
    let error = FixId::from_parts(cme.clone(), 35).unwrap_err();
    assert!(error.to_string().contains("fix:branch"), "{error}");
    assert!(trade.as_fix_mut().set_tag(35).is_err());
    assert_eq!(trade.as_fix().id()?, Some(FixId::from_str("cme:5001")?));
    let mut msg_type = DataType::Utf8.nullable_field("MsgType");
    msg_type.as_fix_mut().set_tag(35)?;
    assert!(msg_type.as_fix_mut().set_branch(&cme).is_err());
    // The rule is one-way: the standard branch holds any tag.
    assert!(FixId::from_parts(FixBranch::STANDARD, 10_000).is_ok());

    // Setting the standard branch removes the key rather than storing it.
    trade.as_fix_mut().set_id(&FixId::standard(9001))?;
    assert!(!trade.has_metadata("fix:branch"));
    assert_eq!(trade.as_fix().id()?, Some(FixId::standard(9001)));
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import Field
    from yggdryl.fix import STANDARD_BRANCH, STANDARD_TAG_LIMIT

    trade = Field("TradeID", "utf8")
    # Absent means standard, and there is no identity without a tag.
    assert trade.fix.branch == STANDARD_BRANCH == "standard"
    assert trade.fix.id is None

    # A branch and an identifier cross as text, parsed once at the boundary,
    # so there is no class for either in Python.
    trade.fix.id = "CME:5001"
    assert trade.fix.id == "cme:5001", "folded once, on the way in"
    assert trade.fix.branch == "cme"
    assert trade.metadata["fix:branch"] == "cme"
    with pytest.raises(ValueError, match="fix branch"):
        trade.fix.branch = "2cme"
    with pytest.raises(ValueError, match="fix identifier"):
        trade.fix.id = "5001"

    # A tag the FIX specification assigns belongs to the standard branch, at
    # every door, and a refusal leaves the field unchanged.
    assert STANDARD_TAG_LIMIT == 5000
    with pytest.raises(ValueError, match="fix:branch"):
        trade.fix.tag = 35
    with pytest.raises(ValueError, match="fix:branch"):
        trade.fix.tags = [35]
    assert trade.fix.id == "cme:5001"
    msg_type = Field("MsgType", "utf8")
    msg_type.fix.tag = 35
    with pytest.raises(ValueError, match="fix:branch"):
        msg_type.fix.branch = "cme"
    # The rule is one-way: the standard branch holds any tag.
    high = Field("Vendorish", "utf8")
    high.fix.tag = 10_000
    assert high.fix.id == "standard:10000"

    # Setting the standard branch removes the key rather than storing it.
    trade.fix.id = "standard:9001"
    assert "fix:branch" not in trade.metadata
    assert trade.fix.branch == "standard"
    assert trade.fix.id == "standard:9001"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fix } = require('yggdryl')

    const trade = Field.from('TradeID: utf8')
    // Absent means standard, and there is no identity without a tag.
    assert.equal(trade.fix.branch, fix.STANDARD_BRANCH)
    assert.equal(fix.STANDARD_BRANCH, 'standard')
    assert.equal(trade.fix.id, null)

    // A branch and an identifier cross as text, parsed once at the boundary,
    // so there is no class for either in JavaScript.
    trade.fix.id = 'CME:5001'
    assert.equal(trade.fix.id, 'cme:5001', 'folded once, on the way in')
    assert.equal(trade.fix.branch, 'cme')
    assert.equal(trade.get('fix:branch'), 'cme')
    assert.throws(() => {
      trade.fix.branch = '2cme'
    }, /fix branch/)
    assert.throws(() => {
      trade.fix.id = '5001'
    }, /fix identifier/)

    // A tag the FIX specification assigns belongs to the standard branch, at
    // every door, and a refusal leaves the field unchanged.
    assert.equal(fix.STANDARD_TAG_LIMIT, 5000)
    assert.throws(() => {
      trade.fix.tag = 35
    }, /fix:branch/)
    assert.throws(() => {
      trade.fix.tags = [35]
    }, /fix:branch/)
    assert.equal(trade.fix.id, 'cme:5001')
    const msgType = Field.from('MsgType: utf8')
    msgType.fix.tag = 35
    assert.throws(() => {
      msgType.fix.branch = 'cme'
    }, /fix:branch/)
    // The rule is one-way: the standard branch holds any tag.
    const high = Field.from('Vendorish: utf8')
    high.fix.tag = 10_000
    assert.equal(high.fix.id, 'standard:10000')

    // Setting the standard branch removes the key rather than storing it.
    trade.fix.id = 'standard:9001'
    assert.equal(trade.has('fix:branch'), false)
    assert.equal(trade.fix.branch, 'standard')
    assert.equal(trade.fix.id, 'standard:9001')
    ```

## Nesting needs no second type

A component is a Struct field; a repeating group is a List of that Struct, its counter tag the group's own `fix:tag`. A list is transparent to a dotted path, so `NoPartyIDs.PartyID` and `NoPartyIDs.item.PartyID` spell one route.

=== "Rust"

    ```rust
    use yggdryl::{DataType, FixBranch, FixRegistry};

    let standard = FixBranch::STANDARD;
    let mut party_id = DataType::Utf8.nullable_field("PartyID");
    party_id.as_fix_mut().set_tag(448)?;
    let mut role = DataType::Int32.nullable_field("PartyRole");
    role.as_fix_mut().set_tag(452)?;
    let item = DataType::from_fields([party_id, role])?.required_field("item");
    let mut group = DataType::list(item).nullable_field("NoPartyIDs");
    group.as_fix_mut().set_tag(453)?;

    let registry = FixRegistry::from_fields([group])?;
    assert_eq!(registry.field_by_path(&standard, "NoPartyIDs")?.as_fix().tag()?, Some(453));
    assert_eq!(registry.field_by_path(&standard, "NoPartyIDs.PartyID")?.as_fix().tag()?, Some(448));
    assert_eq!(registry.field_by_path(&standard, "NoPartyIDs.item.PartyRole")?.name(), "PartyRole");
    // A member is reached through its group, not registered on its own.
    assert!(registry.get_field_by_name(&standard, "PartyID").is_none());
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field, types
    from yggdryl.fix import STANDARD_BRANCH, FixRegistry

    party_id = Field("PartyID", "utf8")
    party_id.fix.tag = 448
    role = Field("PartyRole", "int32")
    role.fix.tag = 452
    item = Field("item", DataType.from_fields([party_id, role]), nullable=False)
    group = types.list("NoPartyIDs", item)
    group.fix.tag = 453

    registry = FixRegistry.from_fields([group])
    assert registry.field_by_path(STANDARD_BRANCH, "NoPartyIDs").fix.tag == 453
    assert registry.field_by_path(STANDARD_BRANCH, "NoPartyIDs.PartyID").fix.tag == 448
    assert (
        registry.field_by_path(STANDARD_BRANCH, "NoPartyIDs.item.PartyRole").name
        == "PartyRole"
    )
    # A member is reached through its group, not registered on its own.
    assert registry.get_field_by_name(STANDARD_BRANCH, "PartyID") is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fields, fix } = require('yggdryl')

    const partyId = Field.from('PartyID: utf8')
    partyId.fix.tag = 448
    const role = Field.from('PartyRole: int32')
    role.fix.tag = 452
    const item = fields.struct('item', [partyId, role], { nullable: false })
    const group = fields.list('NoPartyIDs', item)
    group.fix.tag = 453

    const standard = fix.STANDARD_BRANCH
    const registry = fix.FixRegistry.fromFields([group])
    assert.equal(registry.fieldByPath(standard, 'NoPartyIDs').fix.tag, 453)
    assert.equal(registry.fieldByPath(standard, 'NoPartyIDs.PartyID').fix.tag, 448)
    assert.equal(registry.fieldByPath(standard, 'NoPartyIDs.item.PartyRole').name, 'PartyRole')
    // A member is reached through its group, not registered on its own.
    assert.equal(registry.getFieldByName(standard, 'PartyID'), null)
    ```

## Edges

- Empty element, duplicate (aliases ASCII-folded), alias with a comma, or negative tag -> refused naming `fix:tags` / `fix:aliases` / `fix:tag`; field unchanged.
- Folding is ASCII only: `Größe` and `GRÖSSE` are two names.
- A tag is decimal `0` to `i32::MAX`; readers refuse stored `+35`, `-35`, `3x`.
- Python `tag = True` -> `TypeError`; `2**31` -> `OverflowError`. JavaScript `2 ** 31` -> "signed 32-bit integer"; `field.iceberg.tag` -> `TypeError`.
- `2cme` -> "fix branch"; `5001` as an identifier -> "fix identifier".
- Tag below `STANDARD_TAG_LIMIT` on another branch, canonical or alternate -> refused naming `fix:branch`, from a setter, a read, an insert, or a shard load.
- `get_field_by_path` is transparent to a list on a read; `set_field_by_path` / `remove_field_by_path` spell the item (`NoPartyIDs.item.PartyID`).
- A group member is reached only through its group; `get_field_by_name("PartyID")` answers none.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --lib fix::tests
    cargo test -p yggdryl --lib -- fix::tests::folded_text fix::tests::a_branch_folds fix::tests::an_identifier_renders fix::tests::properties_round_trip fix::tests::a_property_write fix::tests::the_branch_round_trips fix::tests::a_specification_tag fix::tests::set_id_moves fix::tests::a_corrupt_stored fix::tests::a_path_reaches
    cargo bench -p yggdryl --bench fix -- fix/mutate/set_
    cargo bench -p yggdryl --bench fix -- fix/resolve/id_render
    cargo bench -p yggdryl --bench fix -- fix/resolve/id_parse
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/fix
    python/.venv/bin/python -m pytest python/tests/fix -k "vocabulary or tag_rejects or branch_and_id or specification_tag"
    python/.venv/bin/python python/benchmarks/fix.py --iterations 2000
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/fix/fix.test.js
    node --test --test-name-pattern="typed fix vocabulary|answers only on the fix view|never narrowed|round trip as text|malformed branch|specification tag" node/tests/fix/fix.test.js
    npm run --prefix node bench:fix
    ```

## Performance

=== "Rust"

    Field setters and the `FixId` codec: one local Windows x86_64 release run of the Criterion target, point estimates.

    | resolution | estimate |
    | --- | ---: |
    | `FixId::to_string` | 197.6 ns |
    | `FixId::from_str("cme:5001")` | 143.1 ns |

    | mutation | estimate |
    | --- | ---: |
    | `set_branch` on a field whose tags allow it | 816 ns |
    | `set_id` moving a field into a vendor branch | 1.05 us |
    | `set_id` back to the standard branch | 569 ns |
    | `set_branch` refused for a specification tag | 654 ns |

    ```bash
    cargo bench -p yggdryl --bench fix -- fix/mutate/set_
    cargo bench -p yggdryl --bench fix -- fix/resolve/id_render
    cargo bench -p yggdryl --bench fix -- fix/resolve/id_parse
    ```

=== "Python"

    Release wheel (`maturin build --release`) under CPython 3.12 on local Windows x86_64, median time per call.

    | Python operation | estimate |
    | --- | ---: |
    | `field.fix.branch` | 549 ns |
    | `field.fix.id` | 625 ns |

    Both cross as text and parse per call; each row is a metadata read plus a fresh `str`, so hold the answer in a loop.

    ```bash
    python/.venv/bin/python python/benchmarks/fix.py --iterations 2000
    ```

=== "JavaScript"

    Release addon (`npm run --prefix node build`) under Node.js v24.18.0 on local Windows x86_64 (AMD Ryzen 5 150), whole-loop rate.

    | JavaScript operation | rate | per call |
    | --- | ---: | ---: |
    | `field.fix.branch` | 238k/s | 4.19 us |
    | `field.fix.id` | 223k/s | 4.48 us |

    Both cross as text; `field.fix` builds a fresh protocol view per access and `id` renders a new string, so hold the view in a loop.

    ```bash
    npm run --prefix node bench:fix
    ```
