# FIX

FIX field definitions as ordinary fields: the `fix:` vocabulary on a field's view, a registry that
resolves an identifier or a name to the canonical field, the shards it persists to through any
[`IOBase`](io.md) handle, the process-wide default, and the message value typed against one.

!!! note "Bindings"
    The `fix:` vocabulary reaches every runtime through the protocol view a field already
    answers - `field.as_fix()` in Rust, `field.fix` in Python and JavaScript - so there is no
    second field class anywhere. Python reaches the registry, the message and the process
    default through [`yggdryl.fix`](extensions/python.md#fix-registry-at-the-boundary), and
    JavaScript reaches the same surface under its own
    [`fix` namespace](extensions/javascript.md#fix-is-a-namespace).

## The vocabulary is metadata

A FIX field is a [`Field`](field.md) whose metadata carries the `fix:` namespace. The canonical
name is the field's own `name()`, the datatype its own `dtype()`, and the display name the generic
`display` key; the namespace adds only what FIX states beyond a field. It is read through
`field.as_fix()` (`FixField`) and written through `field.as_fix_mut()` (`FixFieldMut`), so a
caller never spells `fix:` - the property names live in one private place.

| Property | Key | Type | Meaning |
| --- | --- | --- | --- |
| `namespace` | `fix:namespace` | `FixNamespace` | the dictionary this field belongs to; **absent means the standard one**, and setting the standard one removes the key |
| `tag` | `fix:tag` | `i32` | canonical FIX tag, never negative |
| `tags` | `fix:tags` | ordered `i32` list | alternate tags, highest priority first |
| `aliases` | `fix:aliases` | ordered name list | alternate names, highest priority first |
| `description` | `fix:description` | text | the specification's own wording |

List-valued properties store as comma-separated text and parse on read. A write rejects an empty
element, a duplicate (aliases compared with ASCII case folded), an alias containing a comma, and a
negative tag; an empty list removes the property. `aliases()` is a lazy iterator over slices of the
stored text, so reading aliases allocates nothing; `tags()` parses integers and answers a `Vec`.
`namespace()` answers `FixNamespace::STANDARD` when the key is absent, which is why every
specification field - and the whole tracked seed - carries no namespace line at all.

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

## Identity is a namespace and a tag

`FixNamespace` names the dictionary a field belongs to: `FixNamespace::STANDARD` is the FIX
specification's own, spelled `standard`, and any other spelling is a venue's. It parses from text
with ASCII case folded once on the way in - `CME` and `cme` are one namespace - and refuses what a
namespace may not be: a first byte that is not an ASCII letter, a byte outside letters, digits,
`-`, `.` and `_`, or more than `FixNamespace::MAX_LENGTH` (23) bytes. That bound is
`smol_str`'s inline capacity, which is what keeps every registry probe carrying a namespace
allocation-free.

`FixId` is that namespace plus a tag, rendered and parsed as `namespace:tag` - `standard:35`,
`cme:5001`. It is **derived on every read** from `fix:namespace` and `fix:tag`, never stored: there
is no `fix:id` key, on disk or in the map, so the identity cannot drift from the two facts it is
computed from. `field.as_fix().id()` answers `None` exactly when `fix:tag` is absent, and orders
namespace-major then by tag, which is the order a registry iterates and a store writes in.

`FixId::from_parts` is the one place the standard-tag rule lives: **a tag below
`FixId::STANDARD_TAG_LIMIT` (5000) forces the standard namespace**, because 0-4999 is what the FIX
specification assigns itself; 5000-9999 is its user-defined range and everything above is vendor
space. The rule is one-way - the standard namespace holds any tag, and the seed and these examples
already use 10000. Because the constructor carries it, an inadmissible identity is unconstructible
rather than refused in several places, and every door reaches the same refusal: `set_namespace`
(checking the canonical tag *and* every alternate tag first), `set_tag`, `set_tags`, `FixField::id`
on read, and the registry's insert, update and shard loader. The refusal is an
`InvalidMetadataValue` naming `fix:namespace`, the limit and both sides, and it leaves the field or
the registry unchanged.

`set_id` moves both halves at once. Without it, `standard:35` → `cme:5001` works only in the order
set-tag-then-set-namespace and the reverse move only in the opposite order, because each single
setter holds the field to the rule as it stands; `set_id` writes the namespace, then the tag, and
puts the prior namespace entry back if the tag write fails.

=== "Rust"

    ```rust
    use yggdryl::{DataType, FixId, FixNamespace};

    let cme = FixNamespace::from_str("CME")?;
    assert_eq!(cme.as_str(), "cme", "folded once, on the way in");
    assert!(FixNamespace::from_str("2cme").is_err());

    let mut trade = DataType::Utf8.nullable_field("TradeID");
    // Absent means standard, and there is no identity without a tag.
    assert_eq!(trade.as_fix().namespace()?, FixNamespace::STANDARD);
    assert_eq!(trade.as_fix().id()?, None);

    trade.as_fix_mut().set_id(&FixId::from_parts(cme.clone(), 5001)?)?;
    assert_eq!(trade.as_fix().id()?.map(|id| id.to_string()), Some("cme:5001".into()));
    assert_eq!(trade.get_metadata("fix:namespace"), Some("cme"));
    assert_eq!(trade.as_fix().id()?, Some(FixId::from_str("cme:5001")?));

    // A tag the FIX specification assigns belongs to the standard namespace,
    // at every door, and a refusal leaves the field unchanged.
    let error = FixId::from_parts(cme.clone(), 35).unwrap_err();
    assert!(error.to_string().contains("fix:namespace"), "{error}");
    assert!(trade.as_fix_mut().set_tag(35).is_err());
    assert_eq!(trade.as_fix().id()?, Some(FixId::from_str("cme:5001")?));
    let mut msg_type = DataType::Utf8.nullable_field("MsgType");
    msg_type.as_fix_mut().set_tag(35)?;
    assert!(msg_type.as_fix_mut().set_namespace(&cme).is_err());
    // The rule is one-way: the standard namespace holds any tag.
    assert!(FixId::from_parts(FixNamespace::STANDARD, 10_000).is_ok());

    // Setting the standard namespace removes the key rather than storing it.
    trade.as_fix_mut().set_id(&FixId::standard(9001))?;
    assert!(!trade.has_metadata("fix:namespace"));
    assert_eq!(trade.as_fix().id()?, Some(FixId::standard(9001)));
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import Field
    from yggdryl.fix import STANDARD_NAMESPACE, STANDARD_TAG_LIMIT

    trade = Field("TradeID", "utf8")
    # Absent means standard, and there is no identity without a tag.
    assert trade.fix.namespace == STANDARD_NAMESPACE == "standard"
    assert trade.fix.id is None

    trade.fix.id = "CME:5001"
    assert trade.fix.id == "cme:5001", "folded once, on the way in"
    assert trade.fix.namespace == "cme"
    assert trade.metadata["fix:namespace"] == "cme"

    # A tag the FIX specification assigns belongs to the standard namespace.
    assert STANDARD_TAG_LIMIT == 5000
    with pytest.raises(ValueError, match="fix:namespace"):
        trade.fix.tag = 35
    assert trade.fix.id == "cme:5001"
    with pytest.raises(ValueError, match="fix:namespace"):
        Field("MsgType", "utf8", metadata={"fix:tag": "35"}).fix.namespace = "cme"

    # Setting the standard namespace removes the key rather than storing it.
    trade.fix.id = "standard:9001"
    assert "fix:namespace" not in trade.metadata
    assert trade.fix.namespace == "standard"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fix } = require('yggdryl')

    const trade = Field.from('TradeID: utf8')
    // Absent means standard, and there is no identity without a tag.
    assert.equal(trade.fix.namespace, fix.STANDARD_NAMESPACE)
    assert.equal(trade.fix.namespace, 'standard')
    assert.equal(trade.fix.id, null)

    trade.fix.id = 'CME:5001'
    assert.equal(trade.fix.id, 'cme:5001', 'folded once, on the way in')
    assert.equal(trade.fix.namespace, 'cme')
    assert.equal(trade.get('fix:namespace'), 'cme')

    // A tag the FIX specification assigns belongs to the standard namespace.
    assert.equal(fix.STANDARD_TAG_LIMIT, 5000)
    assert.throws(() => {
      trade.fix.tag = 35
    }, /fix:namespace/)
    assert.equal(trade.fix.id, 'cme:5001')

    // Setting the standard namespace removes the key rather than storing it.
    trade.fix.id = 'standard:9001'
    assert.equal(trade.has('fix:namespace'), false)
    assert.equal(trade.fix.namespace, 'standard')
    ```

## Nesting needs no second type

A component is a Struct field whose children are its members; a repeating group is a List field
whose item is that Struct; the group's counter tag is the group field's own `fix:tag`. Every member
carries its own tag, and the one path resolver every [`Field`](field.md) has reaches them:
`NoPartyIDs.PartyID` descends through the list's item because a list is transparent to a dotted
path, and `NoPartyIDs.item.PartyID` spells the same route.

=== "Rust"

    ```rust
    use yggdryl::{DataType, FixNamespace, FixRegistry};

    let standard = FixNamespace::STANDARD;
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
    from yggdryl import DataType, Field, fields
    from yggdryl.fix import FixRegistry

    party_id = Field("PartyID", "utf8")
    party_id.fix.tag = 448
    role = Field("PartyRole", "int32")
    role.fix.tag = 452
    item = Field("item", DataType.from_fields([party_id, role]), nullable=False)
    group = fields.list("NoPartyIDs", item)
    group.fix.tag = 453

    registry = FixRegistry.from_fields([group])
    assert registry.field_by_path("NoPartyIDs").fix.tag == 453
    assert registry.field_by_path("NoPartyIDs.PartyID").fix.tag == 448
    assert registry.field_by_path("NoPartyIDs.item.PartyRole").name == "PartyRole"
    # A member is reached through its group, not registered on its own.
    assert registry.get_field_by_name("PartyID") is None
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

    const registry = fix.FixRegistry.fromFields([group])
    assert.equal(registry.fieldByPath('NoPartyIDs').fix.tag, 453)
    assert.equal(registry.fieldByPath('NoPartyIDs.PartyID').fix.tag, 448)
    assert.equal(registry.fieldByPath('NoPartyIDs.item.PartyRole').name, 'PartyRole')
    // A member is reached through its group, not registered on its own.
    assert.equal(registry.getFieldByName('PartyID'), null)
    ```

## The registry resolves in tiers

`FixRegistry` holds its fields in one vector and four indexes of positions over it: canonical and
alternate `FixId`s in two ordered maps, canonical names and aliases in two maps keyed by a
namespace beside ASCII-case-folded text. A lookup consults a later tier only when every earlier one
missed, **inside one namespace**:

1. canonical identifier, then alternate identifiers;
2. canonical name folded, then aliases folded.

A tag query never consults names and a name query never consults tags. Either answers the canonical
field - its own `name()`, never the spelling the query used - and an alias can never take a name
away from a field that claims it canonically. Folding happens once, at insert; a probe hashes the
caller's text folded as it reads it and carries an inline namespace beside it, so a hit allocates
nothing.

No lookup ever crosses a namespace. **A bare tag and a bare name are the standard namespace** -
never whichever dictionaries happen to be loaded, which would make an answer depend on a process's
configuration. Below `FixId::STANDARD_TAG_LIMIT` no other namespace may hold a tag at all; at or
above it, a vendor field is reached by its `FixId` or through the namespace-qualified name
accessors. **A colon-bearing string is a name, not an identifier**: `From<&str>` cannot fail, so
parsing there would need a silent fallback to a name lookup. An identifier is parsed explicitly -
`registry.field(&FixId::from_str("cme:5001")?)`.

Every lookup has a specialized form for a key the caller already holds and a failing twin that
raises a typed absence naming the key (`tag 35`, `identifier cme:5001`, `name "MsgType"`,
`path "a.b"`):

| optional | failing | key |
| --- | --- | --- |
| `get_field_by_id(&FixId)` | `field_by_id` | canonical or alternate identifier, in any namespace; carries the implementation |
| `get_field_by_tag(i32)` | `field_by_tag` | canonical or alternate tag in the standard namespace, which is `get_field_by_id(&FixId::standard(tag))` |
| `get_field_by_name(&FixNamespace, &str)` | `field_by_name` | canonical name or alias, folded, inside one namespace |
| `get_field_by_path(&FixNamespace, &str)` | `field_by_path` | the whole string as a name first, else the first segment here and the rest through `Field::get_field_by_path` |
| `get_field(impl Into<FixKey>)` | `field` | matches `FixKey::Tag` / `FixKey::Id` / `FixKey::Name` once and redirects to the rows above, a bare key meaning the standard namespace |

`FixKey` is built from an `i32`, a `&FixId`, a `&str` or a `&String`, exactly as `FieldKey` is, so
`registry.field(35)` and `registry.field("MsgType")` are one call. `contains` takes the same key,
`iter` walks the fields in ascending identifier order - namespace-major, then by tag - and
`len` / `is_empty` count them.

Identity is the `FixId` and, separately, the pair of namespace and folded canonical name. Two
fields may share neither, nor an alternate identifier, nor an alias. **Two namespaces may define
the same name and the same tag**, because a venue dictionary reusing `Symbol` or `TradeID` is the
normal case; a conflict is only ever within one namespace, and every conflict message names it.
`insert` answers `Ok(None)` for a fresh field, `Ok(Some(prior))` when both halves of the identity
match one stored field (a wholesale replacement), and a typed conflict naming both fields and the
key otherwise; it never silently replaces a different field. Overlap *across* tiers, and any
overlap across namespaces, is legal. `update` merges a definition into the stored field with the
same identifier - a namespace disagreement is simply an absence, because the namespace is half of
the identity: the incoming field wins the name spelling, nullability and every metadata key both
declare; the stored field keeps the keys only it declares; `tags` and `aliases` concatenate,
incoming first, deduplicated, order kept; a datatype disagreement is a typed error naming both,
never a silent widening. Both build the result first and check every key it would claim, so a
refusal leaves the vector and all four indexes untouched. `remove` takes a tag, an identifier or a
name and answers the field.

=== "Rust"

    ```rust
    use yggdryl::{DataType, FixId, FixKey, FixNamespace, FixRegistry};

    let standard = FixNamespace::STANDARD;
    let cme = FixNamespace::from_str("cme")?;

    let mut symbol = DataType::Utf8.nullable_field("Symbol");
    symbol.as_fix_mut().set_tag(55)?;
    symbol.as_fix_mut().set_aliases(["Ticker"])?;
    let mut price = DataType::decimal128(20, 8)?.nullable_field("Price");
    price.as_fix_mut().set_tag(44)?;
    price.as_fix_mut().set_aliases(["Px"])?;
    // The venue dictionary reuses the name `Symbol`, which is the normal case.
    let mut venue = DataType::Utf8.nullable_field("Symbol");
    venue.as_fix_mut().set_id(&FixId::from_parts(cme.clone(), 5055)?)?;
    let mut registry = FixRegistry::from_fields([symbol, price, venue])?;

    // Any spelling of a name or alias answers the canonical field.
    assert_eq!(registry.field_by_name(&standard, "TICKER")?.name(), "Symbol");
    assert_eq!(registry.field("px")?.name(), "Price");
    assert_eq!(registry.get_field(55), registry.get_field("symbol"));
    assert!(registry.contains(FixKey::Tag(44)));
    assert!(!registry.contains("44"), "a tag query never consults names");
    let error = registry.field_by_tag(35).unwrap_err();
    assert!(error.is_absent());

    // A bare tag and a bare name are the standard namespace; the venue field
    // is reached by its identifier or by name inside its own dictionary.
    let venue_id = FixId::from_str("cme:5055")?;
    assert_eq!(registry.field_by_id(&venue_id)?.as_fix().namespace()?, cme);
    assert_eq!(registry.field(&venue_id)?.as_fix().tag()?, Some(5055));
    assert_eq!(registry.field_by_name(&cme, "SYMBOL")?.as_fix().tag()?, Some(5055));
    assert_eq!(registry.field_by_name(&standard, "symbol")?.as_fix().tag()?, Some(55));
    assert!(registry.get_field_by_tag(5055).is_none(), "never crosses a namespace");
    assert!(registry.get_field("cme:5055").is_none(), "a string key is a name");

    // A key another field holds *in the same namespace* is a conflict naming
    // both, and the namespace; nothing changes.
    let mut clash = DataType::Utf8.nullable_field("SymbolSfx");
    clash.as_fix_mut().set_tag(65)?;
    clash.as_fix_mut().set_aliases(["ticker"])?;
    let error = registry.insert(clash).unwrap_err();
    assert!(error.is_conflict(), "{error}");
    assert!(error.to_string().contains("in namespace"), "{error}");
    assert!(error.to_string().contains("held by Symbol"), "{error}");
    assert_eq!(registry.len(), 3);

    // A merge keeps what only the stored field declared and adds the rest.
    let mut incoming = DataType::Utf8.nullable_field("SYMBOL");
    incoming.as_fix_mut().set_tag(55)?;
    incoming.as_fix_mut().set_tags(&[65])?;
    incoming.as_fix_mut().set_aliases(["Sym"])?;
    registry.update(incoming)?;
    let merged = registry.field_by_tag(65)?;
    assert_eq!(merged.name(), "SYMBOL");
    assert_eq!(merged.as_fix().aliases().collect::<Vec<_>>(), ["Sym", "Ticker"]);
    // A datatype disagreement is refused, never widened.
    let mut widened = DataType::LargeUtf8.nullable_field("Symbol");
    widened.as_fix_mut().set_tag(55)?;
    assert!(registry.update(widened).is_err());
    assert_eq!(registry.field_by_tag(55)?.dtype(), &DataType::Utf8);

    // Iteration is namespace-major, then by tag.
    assert_eq!(
        registry.iter().map(|field| field.name()).collect::<Vec<_>>(),
        ["Symbol", "Price", "SYMBOL"],
    );
    assert_eq!(registry.remove("sym").map(|field| field.name().to_owned()), Some("SYMBOL".into()));
    assert!(registry.get_field_by_tag(65).is_none());
    assert_eq!(registry.remove(&venue_id).map(|field| field.name().to_owned()), Some("Symbol".into()));
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType, Field
    from yggdryl.fix import FixRegistry


    def fix_field(name: str, dtype: str, tag: int, *aliases: str) -> Field:
        field = Field(name, dtype)
        field.fix.tag = tag
        if aliases:
            field.fix.aliases = aliases
        return field


    registry = FixRegistry.from_fields(
        [
            fix_field("Symbol", "utf8", 55, "Ticker"),
            fix_field("Price", "decimal128(20, 8)", 44, "Px"),
        ]
    )

    # Any spelling of a name or alias answers the canonical field.
    assert registry.field_by_name("TICKER").name == "Symbol"
    assert registry.field("px").name == "Price"
    assert registry.get_field(55) == registry.get_field("symbol")
    assert 44 in registry
    assert "44" not in registry, "a tag query never consults names"
    with pytest.raises(KeyError, match="tag 35"):
        registry.field_by_tag(35)

    # A key another field holds is a conflict naming both; nothing changes.
    with pytest.raises(ValueError, match="held by Symbol"):
        registry.insert(fix_field("SymbolSfx", "utf8", 65, "ticker"))
    assert len(registry) == 2

    # A merge keeps what only the stored field declared and adds the rest.
    incoming = fix_field("SYMBOL", "utf8", 55, "Sym")
    incoming.fix.tags = [65]
    registry.update(incoming)
    merged = registry.field_by_tag(65)
    assert merged.name == "SYMBOL"
    assert merged.fix.aliases == ["Sym", "Ticker"]
    # A datatype disagreement is refused, never widened.
    with pytest.raises(ValueError):
        registry.update(fix_field("Symbol", "large_utf8", 55))
    assert registry.field_by_tag(55).dtype == DataType("utf8")

    assert [field.name for field in registry] == ["Price", "SYMBOL"]
    assert registry.remove("sym").name == "SYMBOL"
    assert registry.get_field_by_tag(65) is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Field, fix } = require('yggdryl')

    function fixField(name, dtype, tag, ...aliases) {
      const field = Field.from(`${name}: ${dtype}`)
      field.fix.tag = tag
      if (aliases.length !== 0) field.fix.aliases = aliases
      return field
    }

    const registry = fix.FixRegistry.fromFields([
      fixField('Symbol', 'utf8', 55, 'Ticker'),
      fixField('Price', 'decimal128(20, 8)', 44, 'Px'),
    ])

    // Any spelling of a name or alias answers the canonical field.
    assert.equal(registry.fieldByName('TICKER').name, 'Symbol')
    assert.equal(registry.field('px').name, 'Price')
    assert.ok(registry.getField(55).equals(registry.getField('symbol')))
    assert.equal(registry.has(44), true)
    assert.equal(registry.has('44'), false, 'a tag query never consults names')
    assert.throws(() => registry.fieldByTag(35), /tag 35/)

    // A key another field holds is a conflict naming both; nothing changes.
    assert.throws(
      () => registry.insert(fixField('SymbolSfx', 'utf8', 65, 'ticker')),
      /held by Symbol/,
    )
    assert.equal(registry.size, 2)

    // A merge keeps what only the stored field declared and adds the rest.
    const incoming = fixField('SYMBOL', 'utf8', 55, 'Sym')
    incoming.fix.tags = [65]
    registry.update(incoming)
    const merged = registry.fieldByTag(65)
    assert.equal(merged.name, 'SYMBOL')
    assert.deepEqual(merged.fix.aliases, ['Sym', 'Ticker'])
    // A datatype disagreement is refused, never widened.
    assert.throws(() => registry.update(fixField('Symbol', 'large_utf8', 55)))
    assert.ok(registry.fieldByTag(55).dtype.equals(DataType.from('utf8')))

    assert.deepEqual([...registry].map((field) => field.name), ['Price', 'SYMBOL'])
    assert.equal(registry.remove('sym').name, 'SYMBOL')
    assert.equal(registry.getFieldByTag(65), null)
    ```

## Storage is shards under one handle

A registry reads and writes through one [`IOBase`](io.md) folder handle and nothing else. Shards
live at `<root>/records/<namespace>/<shard>.json` with `shard = tag / 100`, so `standard:55` is
`records/standard/0.json` and `cme:5001` is `records/cme/50.json`: the namespace level sits above
the shard level because a shard index is only unique inside one dictionary, a tag then reaches
exactly one shard by arithmetic, and an alternate tag is an index entry that never fans a field
across shards. Each shard is a JSON array of the core field document - what `Field::into_value`
projects - ordered by canonical identifier and rendered indented, so the whole `fix:` namespace
persists through the path every field already has and the tracked seed reads in a diff. Nothing
else is composed: no envelope, no version marker. The namespace segment is the canonical lowercase
text, whose grammar - a leading letter, no separators, no `.` or `..` - makes it a safe single path
segment.

**The record is authoritative and the folder is layout.** A field's own `fix:namespace` decides
which dictionary it belongs to; a field whose namespace contradicts the folder it was read from is
a typed error naming both. A standard field states nothing, so it costs no key.

`from_handle` is the one loader. It lists `records/` and expects **namespace folders only**: a leaf
directly under `records/` is a typed error naming it, and a folder whose name is not a namespace
keeps `FixNamespace::from_str`'s typed parse failure and its byte position with the folder URL
attached. Inside a namespace folder it reads every `<n>.json` leaf and inserts its fields, leaving
anything else alone; every shard is loaded on open, because a name has no numeric structure to pick
a shard with and a dictionary is small enough that loading it whole costs less than lazy machinery.
A folder that does not exist lists nothing and answers the empty registry, as every handle's
laziness contract says; a shard that exists but does not parse, holds a field without a tag or with
a tag another shard owns, or holds a field the registry refuses, is a typed error naming the
shard's URL. `write_into` writes every populated shard whole - creation is a write consequence -
then removes any `<n>.json` no field populates and any namespace folder the registry holds no field
for, so a reload cannot resurrect a removed field or a dropped dictionary.

Nothing recognizes the flat `<root>/records/<shard>.json` layout: a dictionary written by an
earlier version is a load error rather than a silently empty registry, because a default that
quietly loaded nothing would turn every later lookup into a wrong answer.

=== "Rust"

    ```rust
    use yggdryl::io::IOBase;
    use yggdryl::local::Folder;
    use yggdryl::{DataType, FixId, FixNamespace, FixRegistry};

    let root = Folder::temporary()?.path()?.join(format!("yggdryl-doc-fix-store-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let mut folder = Folder::new(&root)?;

    let mut fields = Vec::new();
    for (tag, name) in [(35, "MsgType"), (99, "StopPx"), (100, "NoAllocs"), (150, "ExecType")] {
        let mut field = DataType::Utf8.nullable_field(name);
        field.as_fix_mut().set_tag(tag)?;
        fields.push(field);
    }
    fields[3].as_fix_mut().set_tags(&[20])?;
    // One venue field, which lands in its own namespace folder.
    let cme = FixNamespace::from_str("cme")?;
    let mut trade = DataType::Utf8.nullable_field("TradeID");
    trade.as_fix_mut().set_id(&FixId::from_parts(cme.clone(), 5001)?)?;
    fields.push(trade);
    let mut registry = FixRegistry::from_fields(fields)?;
    registry.write_into(&mut folder)?;

    let shards = |namespace: &str| -> yggdryl::Result<Vec<String>> {
        let mut names: Vec<String> = std::fs::read_dir(root.join("records").join(namespace))?
            .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
            .collect::<Result<_, _>>()?;
        names.sort();
        Ok(names)
    };
    // The alternate tag 20 wrote nothing into shard 0 beyond MsgType and StopPx.
    assert_eq!(shards("standard")?, ["0.json", "1.json"]);
    // Each namespace owns its own shard arithmetic: 5001 / 100 is 50.
    assert_eq!(shards("cme")?, ["50.json"]);

    let reloaded = FixRegistry::from_handle(&folder)?;
    assert_eq!(reloaded, registry);
    assert_eq!(reloaded.field_by_tag(20)?.name(), "ExecType");
    assert_eq!(reloaded.field_by_id(&FixId::from_str("cme:5001")?)?.name(), "TradeID");

    // Removing the only field of a shard removes the shard on the next write,
    // and emptying a namespace removes its folder whole.
    registry.remove(100);
    registry.remove(150);
    registry.remove(&FixId::from_str("cme:5001")?);
    registry.write_into(&mut folder)?;
    assert!(!root.join("records").join("standard").join("1.json").exists());
    assert!(!root.join("records").join("cme").exists());
    assert_eq!(FixRegistry::from_handle(&folder)?.len(), 2);

    // The flat pre-namespace layout is a typed error, never a silent empty load.
    std::fs::write(root.join("records").join("0.json"), b"[]")?;
    let error = FixRegistry::from_handle(&folder).unwrap_err();
    assert!(error.to_string().contains("only namespace folders"), "{error}");
    std::fs::remove_file(root.join("records").join("0.json"))?;

    // A folder that is not there loads as empty and is not created.
    let absent = Folder::new(root.join("absent"))?;
    assert!(FixRegistry::from_handle(&absent)?.is_empty());
    assert!(!absent.exists());
    let _ = std::fs::remove_dir_all(&root);
    ```

=== "Python"

    ```python
    import pathlib
    import shutil
    import tempfile

    from yggdryl import Field
    from yggdryl.fix import FixRegistry

    workspace = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-doc-fix-"))
    root = workspace / "dictionary"

    fields = []
    for tag, name in ((35, "MsgType"), (99, "StopPx"), (100, "NoAllocs"), (150, "ExecType")):
        field = Field(name, "utf8")
        field.fix.tag = tag
        fields.append(field)
    fields[3].fix.tags = [20]
    registry = FixRegistry.from_fields(fields)
    registry.write_into(root)

    # The alternate tag 20 wrote nothing into shard 0 beyond MsgType and StopPx.
    shards = sorted(path.name for path in (root / "records" / "standard").iterdir())
    assert shards == ["0.json", "1.json"]

    reloaded = FixRegistry.from_handle(root)
    assert reloaded == registry
    assert reloaded.field_by_tag(20).name == "ExecType"

    # Removing the only fields of a shard removes the shard on the next write.
    registry.remove(100)
    registry.remove(150)
    registry.write_into(root)
    assert not (root / "records" / "standard" / "1.json").exists()
    assert len(FixRegistry.from_handle(root)) == 2

    # A folder that is not there loads as empty and is not created.
    absent = root / "absent"
    assert not FixRegistry.from_handle(absent)
    assert not absent.exists()

    shutil.rmtree(workspace)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { Field, fix } = require('yggdryl')

    const workspace = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-doc-fix-'))
    const root = path.join(workspace, 'dictionary')

    const declared = []
    for (const [tag, name] of [[35, 'MsgType'], [99, 'StopPx'], [100, 'NoAllocs'], [150, 'ExecType']]) {
      const field = Field.from(`${name}: utf8`)
      field.fix.tag = tag
      declared.push(field)
    }
    declared[3].fix.tags = [20]
    const registry = fix.FixRegistry.fromFields(declared)
    registry.writeInto(root)

    // The alternate tag 20 wrote nothing into shard 0 beyond MsgType and StopPx.
    const shards = fs.readdirSync(path.join(root, 'records', 'standard')).sort()
    assert.deepEqual(shards, ['0.json', '1.json'])

    const reloaded = fix.FixRegistry.fromHandle(root)
    assert.ok(reloaded.equals(registry))
    assert.equal(reloaded.fieldByTag(20).name, 'ExecType')

    // Removing the only fields of a shard removes the shard on the next write.
    registry.remove(100)
    registry.remove(150)
    registry.writeInto(root)
    assert.equal(fs.existsSync(path.join(root, 'records', 'standard', '1.json')), false)
    assert.equal(fix.FixRegistry.fromHandle(root).size, 2)

    // A folder that is not there loads as empty and is not created.
    const absent = path.join(root, 'absent')
    assert.equal(fix.FixRegistry.fromHandle(absent).size, 0)
    assert.equal(fs.existsSync(absent), false)

    fs.rmSync(workspace, { recursive: true, force: true })
    ```

The repository ships a seed dictionary at `config/fix/records/standard/<shard>.json`, written by
`write_into` itself: a small FIX 4.4 subset - the standard header and trailer, the order and execution fields,
the `Parties` component as a repeating group - with the specification's wording as each
description, a display name where FIX has one, declared aliases such as `Ticker` for `Symbol`, and
one alternate tag (`20` for `ExecType`, the pre-4.3 `ExecTransType` whose role it absorbed). It is
what the tests, benchmarks and this page resolve against; it is *not* in the default registry's
resolution order.

=== "Rust"

    ```rust
    use yggdryl::local::Folder;
    use yggdryl::{FixNamespace, FixRegistry};

    let standard = FixNamespace::STANDARD;
    let seed = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("config").join("fix");
    let registry = FixRegistry::from_handle(&Folder::new(seed)?)?;

    assert_eq!(registry.field_by_tag(55)?.name(), "Symbol");
    assert_eq!(registry.field_by_name(&standard, "ticker")?.name(), "Symbol");
    assert_eq!(registry.field_by_tag(20)?.name(), "ExecType");
    assert_eq!(registry.field_by_path(&standard, "NoPartyIDs.PartyID")?.as_fix().tag()?, Some(448));
    assert_eq!(registry.field_by_name(&standard, "ClOrdID")?.display(), Some("Client order ID"));
    // Every seed field is a specification field, so none states a namespace.
    assert!(registry.iter().all(|field| !field.has_metadata("fix:namespace")));
    assert!(registry.len() < 40);
    ```

=== "Python"

    ```python
    import pathlib

    from yggdryl.fix import FixRegistry

    # The seed this repository tracks, named from the repository root.
    seed = pathlib.Path("config/fix").resolve()
    registry = FixRegistry.from_handle(seed)

    assert registry.field_by_tag(55).name == "Symbol"
    assert registry.field_by_name("ticker").name == "Symbol"
    assert registry.field_by_tag(20).name == "ExecType"
    assert registry.field_by_path("NoPartyIDs.PartyID").fix.tag == 448
    assert registry.field_by_name("ClOrdID").display == "Client order ID"
    assert len(registry) < 40
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    // The seed this repository tracks, named from the repository root.
    const registry = fix.FixRegistry.fromHandle(path.resolve('config/fix'))

    assert.equal(registry.fieldByTag(55).name, 'Symbol')
    assert.equal(registry.fieldByName('ticker').name, 'Symbol')
    assert.equal(registry.fieldByTag(20).name, 'ExecType')
    assert.equal(registry.fieldByPath('NoPartyIDs.PartyID').fix.tag, 448)
    assert.equal(registry.fieldByName('ClOrdID').display, 'Client order ID')
    assert.ok(registry.size < 40)
    ```

## One default registry per process

`FixRegistry::global()` answers the `Arc<FixRegistry>` every caller gets when it names none. It
resolves on the first call, on the calling thread, reading the environment exactly once - nothing
loads at module init and no thread is spawned - and every later call answers the same `Arc`. First
match wins:

1. a registry installed by `FixRegistry::install_global`, which fails with a typed conflict once
   the default has resolved so the value every caller saw cannot change underneath them;
2. the folder `YGGDRYL_FIX_REGISTRY` names - a URL, or a bare path - through the local backend;
3. `~/.config/fix`, the production default, reached through [`Folder::config`](local.md) when that
   folder exists; skipped when the machine has no `HOME` or `USERPROFILE`;
4. the empty registry.

Step 3 is the one place absence is not a failure: a machine with no dictionary installed is an
ordinary first-run state. A `YGGDRYL_FIX_REGISTRY` that is set but names no directory, a scheme
this crate has no backend for, or a malformed shard under either folder is an error from
`global()`, never the empty registry - and the default stays unresolved, so the next call retries
the load. The repository's own `config/fix` is not in this order: nothing walks up from the working
directory, because behaviour must not depend on where a process was started.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::local::Folder;
    use yggdryl::{DataType, FixMsg, FixRegistry, Scalar};

    // Install the tracked seed as this process's default before anything asks for it.
    let seed = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("config").join("fix");
    let registry = FixRegistry::from_handle(&Folder::new(seed)?)?;
    FixRegistry::install_global(registry)?;

    let global = FixRegistry::global()?;
    assert_eq!(global.field_by_tag(55)?.name(), "Symbol");
    assert!(Arc::ptr_eq(FixRegistry::global()?, global), "resolved once");

    // A message built without a registry links that same `Arc`.
    let root = DataType::from_fields([global.field_by_tag(55)?.clone()])?.required_field("row");
    let msg = FixMsg::new(root, Scalar::from_record([("Symbol", Scalar::from("AAPL"))])?)?;
    assert!(Arc::ptr_eq(msg.registry(), global));

    // Once resolved, the default is fixed.
    assert!(FixRegistry::install_global(FixRegistry::new()).unwrap_err().is_conflict());
    ```

=== "Python"

    ```python
    import pathlib

    import pytest

    from yggdryl import DataType, Field
    from yggdryl.fix import FixMsg, FixRegistry, global_registry, install_global_registry

    # Install the tracked seed as this process's default before anything asks for it.
    seed = FixRegistry.from_handle(pathlib.Path("config/fix").resolve())
    install_global_registry(seed)

    default = global_registry()
    assert default.field_by_tag(55).name == "Symbol"
    assert default == global_registry(), "resolved once"

    # A message built without a registry links that same dictionary.
    root = Field("row", DataType.from_fields([default.field_by_tag(55)]), nullable=False)
    assert FixMsg(root, {"Symbol": "AAPL"}).registry == default

    # Once resolved, the default is fixed.
    with pytest.raises(ValueError, match="already resolved"):
        install_global_registry(FixRegistry())
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fields, fix } = require('yggdryl')

    // Install the tracked seed as this process's default before anything asks for it.
    const seed = fix.FixRegistry.fromHandle(path.resolve('config/fix'))
    fix.installGlobalRegistry(seed)

    const global = fix.globalRegistry()
    assert.equal(global.fieldByTag(55).name, 'Symbol')
    assert.ok(global.equals(fix.globalRegistry()), 'resolved once')

    // A message built without a registry links that same dictionary.
    const root = fields.struct('row', [global.fieldByTag(55)], { nullable: false })
    assert.ok(new fix.FixMsg(root, { Symbol: 'AAPL' }).registry.equals(global))

    // Once resolved, the default is fixed.
    assert.throws(() => fix.installGlobalRegistry(new fix.FixRegistry()), /already resolved/)
    ```

## A message carries its registry

`FixMsg` is a value plus the registry that types it: a root Struct [`Field`](field.md) - the only
row schema - and the row as the ordered `Scalar::Sequence` the root declares, validated and
canonicalized through `Field::validate_value` and `Field::canonicalize_value` like every row, so a
`Scalar::Record` input becomes that sequence. `FixMsg::new` links `FixRegistry::global()`;
`FixMsg::with_registry` keeps the `Arc` it is given. `registry()`, `as_field()` and `as_value()`
borrow the three parts.

A message has a namespace, and it is **derived, not declared**: `namespace()` answers the root
field's own `fix:namespace`, resolved once at construction - so a malformed one is a typed error
there rather than a silent miss later - and there is no constructor argument that could disagree
with it.

A bare tag or name then resolves in a fixed **two-step tier and no further**:

1. this message's own namespace, when the identifier that would name is legal at all - that is,
   the tag is at or above `FixId::STANDARD_TAG_LIMIT`, or the message is already standard;
2. the standard namespace.

That is what makes both halves of a real venue message reachable: `get_by_tag(5001)` finds the
venue's own field, and `get_by_tag(35)` still finds `MsgType`, which every FIX message carries.

The value accessors mirror the registry's and resolve through the linked registry, never a private
copy of its rules: `get_by_tag` / `by_tag` resolve the tag through that tier to its canonical name
and pick the root child of that name, falling back to a root child named by the tag's decimal text
- an unknown tag a transcriber retained is reachable, never dropped; `get_by_id` / `by_id` name a
dictionary exactly and do not tier, so a foreign namespace simply misses; `get_by_name` / `by_name`
fold through the same tier to the registry's canonical spelling, then match a root child exactly;
`get_by_path` / `by_path` try the whole string as a name, then descend segment by segment - into a
Struct child by name, or into a List entry by a decimal index; `get` / `value` take a `FixKey` and
redirect. A repeating group is a List of Structs, so one of its members needs the entry's index:
`NoPartyIDs.0.PartyID`.

Serialization is inherited, not written: `field.clone().into_json()` renders the schema,
[`into_json_scalar`](json.md) the value, and `from_json_scalar_with_field` reads it back typed,
ordered and canonicalized against the same root.

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
        ("OrderQty", Scalar::I64(100)),
        ("NoPartyIDs", Scalar::from_sequence([
            Scalar::from_record([("PartyID", Scalar::from("BROKER"))])?,
        ])),
        ("9999", Scalar::from("custom")),
    ])?;
    let msg = FixMsg::with_registry(Arc::clone(&registry), root.clone(), value)?;

    // The record became the ordered row the root declares.
    assert_eq!(msg.as_value().as_sequence().map(|row| row.len()), Some(4));
    assert_eq!(msg.by_tag(38)?, &Scalar::I64(100));
    assert_eq!(msg.by_name("ticker")?, &Scalar::from("AAPL"));
    assert_eq!(msg.by_path("NoPartyIDs.0.PartyID")?, &Scalar::from("BROKER"));
    assert_eq!(msg.by_tag(9999)?, &Scalar::from("custom"), "an unknown tag is retained");
    assert_eq!(msg.get(55), msg.get_by_tag(55));
    assert!(msg.value("NoPartyIDs.PartyID").is_err(), "a group member needs its index");

    // The message's namespace is the root's own, and an identifier is exact.
    assert_eq!(msg.namespace(), &yggdryl::FixNamespace::STANDARD);
    assert_eq!(msg.by_id(&yggdryl::FixId::standard(38))?, &Scalar::I64(100));
    assert!(msg.get_by_id(&yggdryl::FixId::from_str("cme:5001")?).is_none());

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

    from yggdryl import DataType, Field, fields
    from yggdryl.fix import FixMsg, FixRegistry

    symbol = Field("Symbol", "utf8", nullable=False)
    symbol.fix.tag = 55
    symbol.fix.aliases = ["Ticker"]
    qty = Field("OrderQty", "int64", nullable=False)
    qty.fix.tag = 38
    party_id = Field("PartyID", "utf8")
    party_id.fix.tag = 448
    item = Field("item", DataType.from_fields([party_id]), nullable=False)
    parties = fields.list("NoPartyIDs", item)
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

    // Schema and value serialize through the paths every field and value share.
    const document = message.toJSON()
    assert.equal(document.field.dtype.fields[1].metadata['fix:tag'], '55')
    assert.deepEqual(document.value[1], 'AAPL')
    assert.ok(new fix.FixMsg(root, message.value, registry).equals(message))
    ```

## Edge cases

- Folding is ASCII only: `Größe` and `GRÖSSE` are two names. FIX spellings are ASCII, and a
  non-ASCII byte compares as it is.
- A tag is a decimal integer from `0` to `i32::MAX`. The setters refuse a negative one and the
  readers refuse stored text that is not exactly decimal digits (`+35`, `-35`, `3x`), so the shard
  arithmetic is total.
- A path tries the whole string as a name first, so a field whose name contains a dot stays
  reachable; only then is the first segment resolved here and the rest handed to
  `Field::get_field_by_path`, where the remainder matches child names exactly.
- An alternate tag equal to another field's canonical tag, or an alias equal to another field's
  canonical name, is legal and simply never wins. The same key twice in the *same* tier across two
  fields is a conflict.
- A tag below `FixId::STANDARD_TAG_LIMIT` is the FIX specification's own, so no other namespace may
  claim it - as a canonical tag or as an alternate one, since an alternate tag resolves with the
  same power. The refusal names `fix:namespace`, the limit and both sides, and it comes from
  `FixId::from_parts` wherever an identity is built: a setter, a read, an insert, or a shard load.
- `remove` takes a tag, an identifier or a name, never a path: a component's member is not a
  registry entry. A bare tag or name means the standard namespace here too.
- `from_handle` admits only namespace folders directly under `records/`; a leaf there is the stale
  flat layout and a typed error, never a folder skipped into an empty load. Inside a namespace
  folder it reads only leaves named `<n>.json` with a decimal `n`; a README beside them is ignored
  and left alone by `write_into`'s cleanup. A field stored in the wrong shard, or in a folder its
  own `fix:namespace` contradicts, is refused with both sides named.
- `YGGDRYL_FIX_REGISTRY` must name an existing directory: a configured location that is not there
  is a misconfiguration, not a first run, so it is an error where `~/.config/fix` would be empty.
- `FixMsg::get_by_tag` renders an unknown tag on the stack, so a miss allocates nothing. The
  fallback matches the root child's name exactly - `9999`, never `09999`.
- `Field::get_field_by_path` is transparent to a list on a read; a write through
  `set_field_by_path` or `remove_field_by_path` still spells the item (`NoPartyIDs.item.PartyID`).

## Measured resolution cost

One local Windows x86_64 release run of the Criterion target (point estimates), over the tracked
seed of 34 fields unless a name says otherwise:

```console
cargo bench -p yggdryl --bench fix -- --warm-up-time 0.2 --measurement-time 0.5 --sample-size 10
```

| resolution | estimate |
| --- | ---: |
| `get_field_by_tag` hit | 27.4 ns |
| `get_field_by_tag` alternate-tag hit | 54.2 ns |
| `get_field_by_tag` miss | 48.4 ns |
| `get_field_by_id` vendor hit, over 1034 fields in two namespaces | 127.0 ns |
| `get_field_by_name` hit | 79.4 ns |
| `get_field_by_name` hit, query differently cased | 80.7 ns |
| `get_field_by_name` alias hit | 136.5 ns |
| `get_field_by_name` miss | 121.2 ns |
| `get_field_by_name` vendor-namespace hit, over 1034 fields | 84.2 ns |
| `get_field_by_name` vendor-namespace alias hit, over 1034 fields | 123.4 ns |
| `get_field_by_tag` cross-namespace miss, over 1034 fields | 175.6 ns |
| `get_field_by_tag` standard hit, over 1034 fields in two namespaces | 118.3 ns |
| `get_field(FixKey::Tag)` generic tag hit | 28.2 ns |
| `get_field("Symbol")` generic name hit | 90.6 ns |
| `field(55)` failing-half tag hit | 27.7 ns |
| `get_field_by_path`, one segment | 86.5 ns |
| `get_field_by_path`, two segments (`NoPartyIDs.PartyID`) | 255.3 ns |
| `get_field_by_path`, three segments (`NoPartyIDs.item.PartyRole`) | 280.1 ns |
| `FixId::to_string` | 205.6 ns |
| `FixId::from_str("cme:5001")` | 142.8 ns |
| baseline `HashMap<FixId, Field>` hit | 39.0 ns |
| baseline `HashMap<i32, Field>` tag hit | 19.5 ns |
| baseline `HashMap<String, Field>` hit after lowercasing the query | 96.6 ns |
| tag hit over 4034 fields | 175.3 ns |
| name hit over 4034 fields | 104.8 ns |
| alias hit over 4034 fields | 139.4 ns |

Mutation clones the dictionary in the batch setup and hands it back as the
routine's output, so neither the clone nor the drop is inside the timer:

| mutation | estimate |
| --- | ---: |
| `insert` into the seed | 5.30 us |
| `insert` into 4034 fields | 117 us |
| `from_fields` over 4034 fields | 14.0 ms |
| `update` merging an alias and an alternate tag into the seed | 9.51 us |
| the same `update` over 4034 fields | 15.6 us |
| `remove` from 4034 fields | 11.6 us |
| `set_namespace` on a field whose tags allow it | 822 ns |
| `set_id` moving a field into a vendor namespace | 1.00 us |
| `set_id` back to the standard namespace | 569 ns |
| `set_namespace` refused for a specification tag | 667 ns |

| storage | estimate |
| --- | ---: |
| `from_handle`, 1 shard of 10 fields | 861 us |
| `from_handle`, 10 shards of 10 fields | 5.24 ms |
| `from_handle`, 100 shards of 10 fields | 48.2 ms |
| `from_handle`, the seed (3 shards, 34 fields) | 2.06 ms |
| `from_handle`, two namespaces (1034 fields, 14 shards) | 17.8 ms |
| `write_into`, 100 shards | 324 ms |
| `write_into`, two namespaces (1034 fields, 14 shards) | 169 ms |
| explicit-location autoload of the seed (URL parse, folder, load) | 2.12 ms |

The generic accessor costs the specialized one plus its dispatch, which on a tag is now within the
noise of the lookup itself: 28.2 ns against 27.4 ns. The specialized pair still exists so a caller
that already knows which key it holds pays no dispatch, but the dispatch is no longer most of the
call the way it was when a tag probe was four nanoseconds.

A folded name hit costs what the plain `HashMap<String, Field>` baseline costs *before* that
baseline lowercases its query - the fold happens inside the hash, so no folded copy is built.

The tag hit is the one number this change moved: it was 4.5 ns against an 18.1 ns
`HashMap<i32, Field>`, and it is now 27.4 ns against 19.5 ns. Every level of the identifier index
compares an inline namespace string before an `i32`, and a `FixId` key is 32 bytes where an `i32`
was 4, so a node spans six cache lines rather than one. Against the baseline that answers the same
question - `HashMap<FixId, Field>`, 39.0 ns - the ordered index is still 1.4x faster, and it is the
only one of the two that can answer `next_field_after`, `iter` and the store's namespace-major
grouping at all. `HashMap<i32, Field>` is faster only because it cannot hold two namespaces: it
answers the ambiguous question this change exists to remove.

`from_handle` scales with the number of shards rather than with the fields in them: a shard costs
roughly half a millisecond to open, read and parse, so the seed's three shards cost about three
times one shard and a hundred shards cost about a hundred times one.

### The Python boundary

```console
cd python && .venv/Scripts/python benchmarks/fix.py --iterations 2000
```

One local Windows x86_64 run of the release wheel (`maturin build --release`) under CPython 3.12,
median time per call over the same tracked seed of 34 fields. The sub-microsecond rows move by a
third between runs on this machine, so read them as one order of magnitude rather than as a
ranking of one against another:

| Python operation | estimate |
| --- | ---: |
| `get_field_by_tag(55)` hit | 254 ns |
| `get_field_by_tag(20)` alternate-tag hit | 256 ns |
| `get_field_by_name("Symbol")` hit | 239 ns |
| `get_field_by_name("symbol")` hit, folded query | 240 ns |
| `get_field_by_name("ticker")` alias hit | 282 ns |
| `get_field_by_tag(9999)` miss | 171 ns |
| `get_field_by_name("Nope")` miss | 183 ns |
| `get_field(55)` generic tag hit | 183 ns |
| `field_by_path`, one segment | 239 ns |
| `field_by_path`, two segments (`NoPartyIDs.PartyID`) | 375 ns |
| `FixMsg.get_by_tag(55)` | 188 ns |
| `FixMsg.get_by_name("ticker")` | 271 ns |
| `FixMsg.get_by_path("NoPartyIDs.0.PartyID")` | 462 ns |
| `from_handle`, the seed (3 shards, 34 fields) | 1.70 ms |
| `from_handle`, 1000 generated fields (11 shards) | 13.8 ms |

A hit costs the native lookup plus one crossing: the key is read once at the boundary and the
answer is wrapped as a `Field` or a `Scalar`, which clones the stored value rather than borrowing
it. That wrapping is what the numbers are almost entirely made of - the native tag hit is 27.4 ns,
an order of magnitude below the crossing - which is why the tiers the Rust table separates are
indistinguishable here, and why a caller resolving the same field repeatedly should hold the
answer rather than ask again. A miss is the one case that is reliably cheaper, because nothing is
wrapped. `from_handle` stays within a fifth of the native load: the shards are listed, read and
parsed natively, and only the finished registry crosses.

### The JavaScript boundary

```console
npm run --prefix node bench:fix
```

One local Windows x86_64 run (AMD Ryzen 5 150) of the release addon
(`npm run --prefix node build`) under Node.js v24.18.0, whole-loop rate over the same tracked seed
of 34 fields; the two loads run a thousandth of the hit count:

| JavaScript operation | rate | per call |
| --- | ---: | ---: |
| `getFieldByTag(55)` hit | 295k/s | 3.39 us |
| `getFieldByTag(20)` alternate-tag hit | 341k/s | 2.94 us |
| `getFieldByName('Symbol')` hit | 273k/s | 3.66 us |
| `getFieldByName('symbol')` hit, folded query | 311k/s | 3.22 us |
| `getFieldByName('ticker')` alias hit | 265k/s | 3.77 us |
| `getFieldByTag(9999)` miss | 1.54M/s | 649 ns |
| `getFieldByName('Nope')` miss | 1.16M/s | 859 ns |
| `getField(55)` generic tag hit | 279k/s | 3.59 us |
| `getField('Symbol')` generic name hit | 280k/s | 3.57 us |
| `fieldByPath`, one segment | 295k/s | 3.39 us |
| `fieldByPath`, two segments (`NoPartyIDs.PartyID`) | 270k/s | 3.70 us |
| `FixMsg.getByTag(55)` | 271k/s | 3.69 us |
| `FixMsg.getByName('ticker')` | 233k/s | 4.30 us |
| `FixMsg.getByPath('NoPartyIDs.0.PartyID')` | 276k/s | 3.62 us |
| `fromHandle`, the seed (3 shards, 34 fields) | 534/s | 1.87 ms |
| `fromHandle`, 1000 generated fields (11 shards) | 43/s | 23.3 ms |

A miss is the honest price of the crossing itself: 649 ns for the key coercion, the native probe,
and `null` back. Every hit adds the wrapper the answer is put in, and that wrapper is what the rest
of the numbers are made of - `field.clone()` on an already-held native `Field` costs 3.0 us on this
machine, so a hit at 3.4 us is a 649 ns lookup plus one `Field` materialization. The native tag hit
is 27.4 ns, two orders of magnitude below the crossing, which is why the tiers the Rust table
separates are indistinguishable here and why a caller resolving the same field repeatedly should
hold the answer rather than ask again. `FixMsg`'s accessors wrap a `Scalar` instead and cost the
same shape. `fromHandle` stays close to the native load - the shards are listed, read and parsed
natively, and only the finished registry crosses - and scales with shard count, not with the fields
in them.
