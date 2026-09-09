# FIX

FIX field definitions are ordinary fields: a `fix:` vocabulary on a [`Field`](../types/field.md), a [registry](registry.md) resolving them, [shards](store.md) persisting them, a [message](message.md) typed against one, an [Arrow boundary](arrow.md) streaming a whole capture through it, a [capture](capture.md) landing in one fixed row, and a [tool](cli.md) to manage all of it.

The dictionary is also open in the browser: [explore](explorer.md) it, [decode](decode.md) a frame against it, or inspect its native [wire emission](encode.md).

## Pages

| Page | Purpose |
| --- | --- |
| [FIX](index.md) | This page: vocabulary, `FixBranch`, `FixId`, nesting |
| [Explorer](explorer.md) | The whole dictionary live: counts, field search, message layouts, provenance |
| [Decode](decode.md) | A frame in, an explanation out; every shape a capture holds, read by the package |
| [Encode](encode.md) | Native wire emission from captured message entries |
| [Registry](registry.md) | `FixRegistry`: tiered resolution, `FixKey`, mutation, protocol inference, the process-wide default |
| [Store](store.md) | Shard trees and the branch manifest under one `IOBase` folder, `from_handle`, `write_into`, the tracked seed |
| [Message](message.md) | `FixMsg`: root Struct plus row and registry, derived branch, accessors, JSON |
| [Arrow](arrow.md) | `FixBatchReader`, `FixOptions`, `classify_arrow_array`: a capture already in Arrow, streamed through a dictionary |
| [Capture](capture.md) | `FixCodec`, `fix_schema`, `FixMsg::into_row`: a day of session log as one table |
| [CLI](cli.md) | `ygg`: dictionary CRUD, `.cfb` ingest, schema dump, quality and drift, from a terminal |

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `FixField` / `FixFieldMut` (`as_fix()` / `as_fix_mut()`), `FixBranch`, `FixId`; no second field class |
| Keys | `fix:branch`, `fix:tag`, `fix:tags`, `fix:aliases`; name, datatype, `display` and `description` stay the field's own |
| Branch | ASCII letter first, then letters, digits, `-`, `.`, `_`; at most `FixBranch::MAX_LENGTH` (23) bytes; case folded once on parse |
| Standard branch | Empty name, digest zero, `Version::default()`, empty sender and target component IDs; an absent key means it, and setting it removes the key |
| Named branch | Any non-empty spelling, `std` and `standard` included; `FixBranch::from_parts` fills name, digest and `Version`, and the registry stores that value |
| Identity | `FixId` is the tag and the branch's cached XXH32 digest, two `i32` halves - `tag()` and `branch()`, the same pair and the same signed reading an arrival entry stores; `Copy`, eight bytes, ordered tag-major then by branch |
| Spelling | Parsed as `tag:branch`; displayed `35:` for the standard branch and `5001:#7f3a1c02` for another; a field keeps its branch text, so `field.fix.id` reads `5001:cme` |
| Derived | The identifier is computed on every read from `fix:branch` and `fix:tag`, never stored; `None` without a tag |
| User tag range | A non-standard branch may claim only `FixId::USER_TAG_MIN..FixId::USER_TAG_MAX`, currently `[5000, 40000)`; the standard branch holds every non-negative tag |
| Tag gate | `FixId::from_parts` is the one gate, for canonical and alternate tags; a refusal names both bounds |
| List properties | Comma-separated text; `aliases()` lazy slices, `tags()` a parsed `Vec`; empty list removes the key |
| Errors | `InvalidMetadataValue` naming the full key; the field stays unchanged |
| Categories | `fields/` stores tagged scalar fields; `components/` reusable Structs; `groups/` Lists of components; `messages/` required Struct definitions |
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
    // Two, not three: a description is a fact about the column rather than a
    // FIX fact, so it lives on the generic key beside `display`.
    assert_eq!(field.get_metadata("description"), Some("Quantity ordered."));
    assert_eq!(field.as_fix().len(), 2);

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
    # Two, not three: a description is a fact about the column rather than a
    # FIX fact, so it lives on the generic key beside `display`.
    assert field.metadata["description"] == "Quantity ordered."
    assert len(field.fix) == 2

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
    // Two, not three: a description is a fact about the column rather than a
    // FIX fact, so it lives on the generic key beside `display`.
    assert.equal(field.get('description'), 'Quantity ordered.')
    assert.equal(field.fix.size, 2)

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
| `description` | `description` | text | the specification's wording, on the generic key every catalog reads |
| `codes` | `fix:codes` | canonical JSON, by wire value | the inline enum values declared by this field; see [Registry](registry.md#a-field-carries-its-code-set) |
| `counter` | `fix:counter` | `i32` | the scalar count field's tag, on a group definition |
| `component` | `fix:component` | name | component reference, including a group's occurrence |
| `field_ref` / `fieldRef` | `fix:field` | name | scalar field reference in a definition |
| `group` | `fix:group` | name | group reference in a definition |
| `msgtype` | `fix:msgtype` | text | complete case-sensitive wire code on a message Struct |
| `lineage` | `fix:lineage` | canonical JSON, oldest first | what this field was called and typed at each FIX version; see [Registry](registry.md#versions-are-a-filter-on-the-read) |

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
    # while `FixBranch` also has a native Python value wrapper.
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

`NoPartyIDs` is an `int32` field at tag 453. `Parties` is a separate List of the
`Party` Struct, linked to that count through `fix:counter`. Fields, components,
groups and messages are independently addressable registry categories; a group
member is also a scalar field in the field catalog.

The published FIX component names guide the catalog: [FIX message structures](https://fixtrading.org/concepts-part1-messagestructures/)
and [FIX Orchestra](https://github.com/FIXTradingCommunity/fix-orchestra-spec/blob/master/v1-0-STANDARD/orchestra_spec.md)
distinguish fields, components, messages and repeating groups. The generator
uses a standard name when it fits the role, then a deterministic descriptive
name checked against existing field and definition names. The native canonical
names are folded; `display` keeps the specification's spelling.

=== "Rust"

    ```rust
    use yggdryl::holder::local::Folder;
    use yggdryl::{DataType, FixCategory, FixRegistry};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = FixRegistry::from_handle(&Folder::new(root)?)?;
    assert_eq!(registry.field_by_tag(453)?.dtype(), &DataType::Int32);
    let parties = registry.definition(FixCategory::Groups, "Parties", None)?;
    assert_eq!(parties.as_fix().counter()?, Some(453));
    assert!(!registry.definition(FixCategory::Components, "Party", None)?.fields().is_empty());
    assert_eq!(registry.field_by_path("Parties.PartyID", None)?.as_fix().tag()?, Some(448));
    assert_eq!(registry.field_by_name("PartyID", None)?.as_fix().tag()?, Some(448));
    ```

=== "Python"

    ```python
    from pathlib import Path
    from yggdryl.fix import FixRegistry

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    assert str(registry.field_by_tag(453).dtype) == "int32"
    assert registry.definition("groups", "Parties").fix.counter == 453
    assert registry.definition("components", "Party").is_struct
    assert registry.field_by_path("Parties.PartyID").fix.tag == 448
    assert registry.field_by_name("PartyID").fix.tag == 448
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config/fix'))
    assert.equal(registry.fieldByTag(453).dtype.toString(), 'int32')
    assert.equal(registry.definition('groups', 'Parties').fix.counter, 453)
    assert.ok(registry.definition('components', 'Party').fieldLen > 0)
    assert.equal(registry.fieldByPath('Parties.PartyID').fix.tag, 448)
    assert.equal(registry.fieldByName('PartyID').fix.tag, 448)
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
- `get_field_by_path` traverses a declared group without an occurrence index; a message value uses an index, for example `Parties.0.PartyID`.
- A shared count tag can describe different group layouts. A message singleton selects its own group context; an ambiguous registry-wide counter lookup fails.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --lib fix::tests
    cargo test -p yggdryl --lib -- fix::tests::name_indexes_fold_ascii fix::tests::a_branch_folds fix::tests::an_identifier_is_two_halves fix::tests::properties_round_trip fix::tests::a_property_write fix::tests::the_branch_round_trips fix::tests::a_specification_tag fix::tests::set_id_moves fix::tests::a_corrupt_stored fix::tests::a_path_reaches
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
