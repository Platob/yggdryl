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
| Standard branch | Empty name, digest zero, `Version::default()`, empty sender and target component IDs; an absent key means it, and setting it removes the key |
| Named branch | Any non-empty spelling, `std` and `standard` included; `FixBranch::from_parts` fills name, digest, `Version`, `target_comp_id`, `sender_comp_id`, and the registry stores that value |
| Identity | `FixId` packs the tag in the high 32 bits and the branch's cached XXH32 digest in the low 32 bits of one positive `i64`; `Copy`, eight bytes, tag-major |
| Spelling | Parsed as `tag:branch`; displayed `35:` for the standard branch and `5001:#7f3a1c02` for another; a field keeps its branch text, so `field.fix.id` reads `5001:cme` |
| Derived | The identifier is computed on every read from `fix:branch` and `fix:tag`, never stored; `None` without a tag |
| User tag range | A non-standard branch may claim only `FixId::USER_TAG_MIN..FixId::USER_TAG_MAX`, currently `[5000, 40000)`; the standard branch holds every non-negative tag |
| Tag gate | `FixId::from_parts` is the one gate, for canonical and alternate tags; a refusal names both bounds |
| List properties | Comma-separated text; `aliases()` lazy slices, `tags()` a parsed `Vec`; empty list removes the key |
| Errors | `InvalidMetadataValue` naming the full key; the field stays unchanged |
| Nesting | Struct = component, List of that Struct = group; `dtype().is_nested()` routes a field into the [store](store.md)'s `nested/` tree |
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

`35:` to `5001:cme` needs set-tag-then-set-branch, and the reverse move the opposite order. `set_id` writes both halves at once and restores the prior branch when the tag write fails.

=== "Rust"

    ```rust
    use yggdryl::{DataType, FixId, FixBranch};

    let cme = FixBranch::from_str("CME")?;
    assert_eq!(cme.name(), "cme", "folded once, on the way in");
    assert!(FixBranch::from_str("2cme").is_err());

    let mut trade = DataType::Utf8.nullable_field("TradeID");
    // Absent means standard, and there is no identity without a tag.
    assert_eq!(trade.as_fix().branch()?, FixBranch::STANDARD);
    assert_eq!(trade.as_fix().id()?, None);

    trade.as_fix_mut().set_id(&cme, 5001)?;
    assert_eq!(trade.get_metadata("fix:branch"), Some("cme"));
    assert_eq!(trade.as_fix().id()?, Some(FixId::from_parts(&cme, 5001)?));
    assert_eq!(std::mem::size_of::<FixId>(), 8);

    assert_eq!(FixId::USER_TAG_MIN, 5_000);
    assert_eq!(FixId::USER_TAG_MAX, 40_000);
    assert!(FixId::from_parts(&cme, 4_999).is_err());
    assert!(FixId::from_parts(&cme, 5_000).is_ok());
    assert!(FixId::from_parts(&cme, 39_999).is_ok());
    assert!(FixId::from_parts(&cme, 40_000).is_err());
    assert!(!FixBranch::from_str("standard")?.is_standard());

    // Setting the standard branch removes the key rather than storing it.
    trade.as_fix_mut().set_id(&FixBranch::STANDARD, 9_001)?;
    assert!(!trade.has_metadata("fix:branch"));
    assert_eq!(trade.as_fix().id()?, Some(FixId::standard(9001)));
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import Field
    from yggdryl.fix import STANDARD_BRANCH, USER_TAG_MAX, USER_TAG_MIN

    trade = Field("TradeID", "utf8")
    # Absent means standard, and there is no identity without a tag.
    assert trade.fix.branch == STANDARD_BRANCH == ""
    assert trade.fix.id is None

    # A branch and an identifier cross as text, parsed once at the boundary,
    # so there is no class for either in Python.
    trade.fix.id = "5001:CME"
    assert trade.fix.id == "5001:cme", "folded once, on the way in"
    assert trade.fix.branch == "cme"
    assert trade.metadata["fix:branch"] == "cme"
    with pytest.raises(ValueError, match="fix branch"):
        trade.fix.branch = "2cme"
    with pytest.raises(ValueError, match="fix identifier"):
        trade.fix.id = "5001"

    assert (USER_TAG_MIN, USER_TAG_MAX) == (5_000, 40_000)
    for tag in (4_999, 40_000):
        with pytest.raises(ValueError, match="5000.*40000"):
            trade.fix.id = f"{tag}:cme"
    assert trade.fix.id == "5001:cme"
    # Setting the standard branch removes the key rather than storing it.
    trade.fix.id = "9001:"
    assert "fix:branch" not in trade.metadata
    assert trade.fix.branch == ""
    assert trade.fix.id == "9001:"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fix } = require('yggdryl')

    const trade = Field.from('TradeID: utf8')
    // Absent means standard, and there is no identity without a tag.
    assert.equal(trade.fix.branch, fix.STANDARD_BRANCH)
    assert.equal(fix.STANDARD_BRANCH, '')
    assert.equal(trade.fix.id, null)

    // A branch and an identifier cross as text, parsed once at the boundary,
    // so there is no class for either in JavaScript.
    trade.fix.id = '5001:CME'
    assert.equal(trade.fix.id, '5001:cme', 'folded once, on the way in')
    assert.equal(trade.fix.branch, 'cme')
    assert.equal(trade.get('fix:branch'), 'cme')
    assert.throws(() => {
      trade.fix.branch = '2cme'
    }, /fix branch/)
    assert.throws(() => {
      trade.fix.id = '5001'
    }, /fix identifier/)

    assert.deepEqual([fix.USER_TAG_MIN, fix.USER_TAG_MAX], [5_000, 40_000])
    assert.throws(() => {
      trade.fix.id = '40000:cme'
    }, /5000.*40000/)
    assert.equal(trade.fix.id, '5001:cme')
    // Setting the standard branch removes the key rather than storing it.
    trade.fix.id = '9001:'
    assert.equal(trade.has('fix:branch'), false)
    assert.equal(trade.fix.branch, '')
    assert.equal(trade.fix.id, '9001:')
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
    assert_eq!(registry.field_by_path("NoPartyIDs", Some(&standard))?.as_fix().tag()?, Some(453));
    assert_eq!(registry.field_by_path("NoPartyIDs.PartyID", Some(&standard))?.as_fix().tag()?, Some(448));
    assert_eq!(registry.field_by_path("NoPartyIDs.item.PartyRole", Some(&standard))?.name(), "PartyRole");
    // A member is reached through its group, not registered on its own.
    assert!(registry.get_field_by_name("PartyID", Some(&standard)).is_none());
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
    assert registry.field_by_path("NoPartyIDs", STANDARD_BRANCH).fix.tag == 453
    assert registry.field_by_path("NoPartyIDs.PartyID", STANDARD_BRANCH).fix.tag == 448
    assert (
        registry.field_by_path("NoPartyIDs.item.PartyRole", STANDARD_BRANCH).name
        == "PartyRole"
    )
    # A member is reached through its group, not registered on its own.
    assert registry.get_field_by_name("PartyID", STANDARD_BRANCH) is None
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
    assert.equal(registry.fieldByPath('NoPartyIDs', standard).fix.tag, 453)
    assert.equal(registry.fieldByPath('NoPartyIDs.PartyID', standard).fix.tag, 448)
    assert.equal(registry.fieldByPath('NoPartyIDs.item.PartyRole', standard).name, 'PartyRole')
    // A member is reached through its group, not registered on its own.
    assert.equal(registry.getFieldByName(standard, 'PartyID'), null)
    ```

## Edges

- Empty element, duplicate (aliases ASCII-folded), alias with a comma, or negative tag -> refused naming `fix:tags` / `fix:aliases` / `fix:tag`; field unchanged.
- Folding is ASCII only: `Größe` and `GRÖSSE` are two names.
- A tag is decimal `0` to `i32::MAX`; readers refuse stored `+35`, `-35`, `3x`.
- Python `tag = True` -> `TypeError`; `2**31` -> `OverflowError`. JavaScript `2 ** 31` -> "signed 32-bit integer"; `field.iceberg.tag` -> `TypeError`.
- `2cme` -> "fix branch"; `5001` as an identifier -> "fix identifier".
- A tag outside `[FixId::USER_TAG_MIN, FixId::USER_TAG_MAX)` on a named branch, canonical or alternate -> refused naming `fix:branch` and both bounds, from a setter, a read, an insert, or a shard load.
- `FixBranch::from_str("standard")` -> an ordinary named branch whose `is_standard()` is `false`; only the empty name is the standard branch.
- `FixId::from_parts` takes the branch by reference and `set_id` takes the branch and the tag, so neither clones a branch.
- `get_field_by_path` is transparent to a list on a read; `set_field_by_path` / `remove_field_by_path` spell the item (`NoPartyIDs.item.PartyID`).
- A group member is reached only through its group; `get_field_by_name("PartyID")` answers none.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --lib fix::tests
    cargo test -p yggdryl --lib -- fix::tests::name_indexes_fold_ascii fix::tests::a_branch_folds fix::tests::an_identifier_is_packed fix::tests::properties_round_trip fix::tests::a_property_write fix::tests::the_branch_round_trips fix::tests::a_specification_tag fix::tests::set_id_moves fix::tests::a_corrupt_stored fix::tests::a_path_reaches
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
