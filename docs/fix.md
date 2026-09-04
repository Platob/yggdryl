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
| `branch` | `fix:branch` | `FixBranch` | the dictionary this field belongs to; **absent means the standard one**, and setting the standard one removes the key |
| `tag` | `fix:tag` | `i32` | canonical FIX tag, never negative |
| `tags` | `fix:tags` | ordered `i32` list | alternate tags, highest priority first |
| `aliases` | `fix:aliases` | ordered name list | alternate names, highest priority first |
| `description` | `fix:description` | text | the specification's own wording |

List-valued properties store as comma-separated text and parse on read. A write rejects an empty
element, a duplicate (aliases compared with ASCII case folded), an alias containing a comma, and a
negative tag; an empty list removes the property. `aliases()` is a lazy iterator over slices of the
stored text, so reading aliases allocates nothing; `tags()` parses integers and answers a `Vec`.
`branch()` answers `FixBranch::STANDARD` when the key is absent, which is why every
specification field - and the whole tracked seed - carries no branch line at all.

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

## Identity is a branch and a tag

`FixBranch` names the dictionary a field belongs to: `FixBranch::STANDARD` is the FIX
specification's own, spelled `standard`, and any other spelling is a venue's. It parses from text
with ASCII case folded once on the way in - `CME` and `cme` are one branch - and refuses what a
branch may not be: a first byte that is not an ASCII letter, a byte outside letters, digits,
`-`, `.` and `_`, or more than `FixBranch::MAX_LENGTH` (23) bytes. That bound is
`smol_str`'s inline capacity, which is what keeps every registry probe carrying a branch
allocation-free.

`FixId` is that branch plus a tag, rendered and parsed as `branch:tag` - `standard:35`,
`cme:5001`. It is **derived on every read** from `fix:branch` and `fix:tag`, never stored: there
is no `fix:id` key, on disk or in the map, so the identity cannot drift from the two facts it is
computed from. `field.as_fix().id()` answers `None` exactly when `fix:tag` is absent, and orders
branch-major then by tag, which is the order a registry iterates and a store writes in.

`FixId::from_parts` is the one place the standard-tag rule lives: **a tag below
`FixId::STANDARD_TAG_LIMIT` (5000) forces the standard branch**, because 0-4999 is what the FIX
specification assigns itself; 5000-9999 is its user-defined range and everything above is vendor
space. The rule is one-way - the standard branch holds any tag, and the seed and these examples
already use 10000. Because the constructor carries it, an inadmissible identity is unconstructible
rather than refused in several places, and every door reaches the same refusal: `set_branch`
(checking the canonical tag *and* every alternate tag first), `set_tag`, `set_tags`, `FixField::id`
on read, and the registry's insert, update and shard loader. The refusal is an
`InvalidMetadataValue` naming `fix:branch`, the limit and both sides, and it leaves the field or
the registry unchanged.

`set_id` moves both halves at once. Without it, `standard:35` → `cme:5001` works only in the order
set-tag-then-set-branch and the reverse move only in the opposite order, because each single
setter holds the field to the rule as it stands; `set_id` writes the branch, then the tag, and
puts the prior branch entry back if the tag write fails.

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

A component is a Struct field whose children are its members; a repeating group is a List field
whose item is that Struct; the group's counter tag is the group field's own `fix:tag`. Every member
carries its own tag, and the one path resolver every [`Field`](field.md) has reaches them:
`NoPartyIDs.PartyID` descends through the list's item because a list is transparent to a dotted
path, and `NoPartyIDs.item.PartyID` spells the same route.

Those two shapes are exactly what `field.dtype().is_nested()` answers `true` for, and that one
core predicate is what puts a field in the registry's nested index half and in the `nested/`
storage tree - see [the registry](#the-registry-resolves-in-tiers) and
[storage](#storage-is-two-trees-of-shards-under-one-handle).

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
    from yggdryl import DataType, Field, fields
    from yggdryl.fix import STANDARD_BRANCH, FixRegistry

    party_id = Field("PartyID", "utf8")
    party_id.fix.tag = 448
    role = Field("PartyRole", "int32")
    role.fix.tag = 452
    item = Field("item", DataType.from_fields([party_id, role]), nullable=False)
    group = fields.list("NoPartyIDs", item)
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

## The registry resolves in tiers

`FixRegistry` holds its fields in one vector and four indexes of positions over it: canonical and
alternate `FixId`s in two ordered maps, canonical names and aliases in two maps keyed by a
branch beside ASCII-case-folded text. Each of the four is kept in two halves, one for the
primitive fields and one for the nested ones. A lookup consults a later tier only when every
earlier one missed, **inside one branch**:

1. canonical identifier, then alternate identifiers;
2. canonical name folded, then aliases folded.

Each of those four indexes is **split into a primitive and a nested half**, by the same
`field.dtype().is_nested()` that decides [which tree a field is stored in](#storage-is-two-trees-of-shards-under-one-handle),
and each tier reads the primitive half before the nested one:

1. primitive canonical identifier, then nested canonical identifier;
2. primitive alternate identifier, then nested alternate identifier;
3. primitive canonical name folded, then nested canonical name folded;
4. primitive alias folded, then nested alias folded.

**The split is a locality optimization and cannot change which field a key resolves to.** The
identity space is not split: a nested field can never claim a primitive field's identifier, name,
alternate identifier or alias, because every insert and merge checks *both* halves before anything
is written, and the conflict it raises names both fields exactly as a conflict between two
primitives does. The split is also a partition of each index rather than a fifth tier above them,
so a canonical key of the nested half still beats an alternate key of the primitive one. What
changes is only how many entries the hot probe walks: components and repeating groups are a small
minority of a dictionary, so the primitive half a transcriber probes per wire tag stays nearly the
whole of it and the cold half stays small.

A tag query never consults names and a name query never consults tags. Either answers the canonical
field - its own `name()`, never the spelling the query used - and an alias can never take a name
away from a field that claims it canonically. Folding happens once, at insert; a probe hashes the
caller's text folded as it reads it and carries an inline branch beside it, so a hit allocates
nothing, in either half.

No lookup ever crosses a branch. **A bare tag and a bare name are the standard branch** -
never whichever dictionaries happen to be loaded, which would make an answer depend on a process's
configuration. Below `FixId::STANDARD_TAG_LIMIT` no other branch may hold a tag at all; at or
above it, a vendor field is reached by its `FixId` or through the branch-qualified name
accessors. **A colon-bearing string is a name, not an identifier**: `From<&str>` cannot fail, so
parsing there would need a silent fallback to a name lookup. An identifier is parsed explicitly -
`registry.field(&FixId::from_str("cme:5001")?)`.

Every lookup has a specialized form for a key the caller already holds and a failing twin that
raises a typed absence naming the key (`tag 35`, `identifier cme:5001`, `name "MsgType"`,
`path "a.b"`):

| optional | failing | key |
| --- | --- | --- |
| `get_field_by_id(&FixId)` | `field_by_id` | canonical or alternate identifier, in any branch; carries the implementation |
| `get_field_by_tag(i32)` | `field_by_tag` | canonical or alternate tag in the standard branch, which is `get_field_by_id(&FixId::standard(tag))` |
| `get_field_by_name(&FixBranch, &str)` | `field_by_name` | canonical name or alias, folded, inside one branch |
| `get_field_by_path(&FixBranch, &str)` | `field_by_path` | the whole string as a name first, else the first segment here and the rest through `Field::get_field_by_path` |
| `get_field(impl Into<FixKey>)` | `field` | matches `FixKey::Tag` / `FixKey::Id` / `FixKey::Name` once and redirects to the rows above, a bare key meaning the standard branch |

`FixKey` is built from an `i32`, a `&FixId`, a `&str` or a `&String`, exactly as `FieldKey` is, so
`registry.field(35)` and `registry.field("MsgType")` are one call. `contains` takes the same key,
`iter` walks the fields in ascending identifier order - branch-major, then by tag - merging the two
halves as it goes, so a nested field takes its place among the primitives rather than after them
and the order is exactly what one undivided index would give. `next_field_after`, the cursor a
binding advances with, walks the same merge, and `len` / `is_empty` count both halves.

Identity is the `FixId` and, separately, the pair of branch and folded canonical name. Two
fields may share neither, nor an alternate identifier, nor an alias. **Two branches may define
the same name and the same tag**, because a venue dictionary reusing `Symbol` or `TradeID` is the
normal case; a conflict is only ever within one branch, and every conflict message names it.
`insert` answers `Ok(None)` for a fresh field, `Ok(Some(prior))` when both halves of the identity
match one stored field (a wholesale replacement), and a typed conflict naming both fields and the
key otherwise; it never silently replaces a different field. Overlap *across* tiers, and any
overlap across branches, is legal. `update` merges a definition into the stored field with the
same identifier - a branch disagreement is simply an absence, because the branch is half of
the identity: the incoming field wins the name spelling, nullability and every metadata key both
declare; the stored field keeps the keys only it declares; `tags` and `aliases` concatenate,
incoming first, deduplicated, order kept; a datatype disagreement is a typed error naming both,
never a silent widening. Both build the result first and check every key it would claim, so a
refusal leaves the vector and both halves of all four indexes untouched. `remove` takes a tag, an identifier or a
name and answers the field.

=== "Rust"

    ```rust
    use yggdryl::{DataType, FixId, FixKey, FixBranch, FixRegistry};

    let standard = FixBranch::STANDARD;
    let cme = FixBranch::from_str("cme")?;

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

    // A bare tag and a bare name are the standard branch; the venue field
    // is reached by its identifier or by name inside its own dictionary.
    let venue_id = FixId::from_str("cme:5055")?;
    assert_eq!(registry.field_by_id(&venue_id)?.as_fix().branch()?, cme);
    assert_eq!(registry.field(&venue_id)?.as_fix().tag()?, Some(5055));
    assert_eq!(registry.field_by_name(&cme, "SYMBOL")?.as_fix().tag()?, Some(5055));
    assert_eq!(registry.field_by_name(&standard, "symbol")?.as_fix().tag()?, Some(55));
    assert!(registry.get_field_by_tag(5055).is_none(), "never crosses a branch");
    assert!(registry.get_field("cme:5055").is_none(), "a string key is a name");

    // A key another field holds *in the same branch* is a conflict naming
    // both, and the branch; nothing changes.
    let mut clash = DataType::Utf8.nullable_field("SymbolSfx");
    clash.as_fix_mut().set_tag(65)?;
    clash.as_fix_mut().set_aliases(["ticker"])?;
    let error = registry.insert(clash).unwrap_err();
    assert!(error.is_conflict(), "{error}");
    assert!(error.to_string().contains("in branch"), "{error}");
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

    // Iteration is branch-major, then by tag.
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
    from yggdryl.fix import STANDARD_BRANCH, FixRegistry


    def fix_field(name: str, dtype: str, identifier: str, *aliases: str) -> Field:
        field = Field(name, dtype)
        field.fix.id = identifier
        if aliases:
            field.fix.aliases = aliases
        return field


    registry = FixRegistry.from_fields(
        [
            fix_field("Symbol", "utf8", "standard:55", "Ticker"),
            fix_field("Price", "decimal128(20, 8)", "standard:44", "Px"),
            # The venue dictionary reuses the name `Symbol`, the normal case.
            fix_field("Symbol", "utf8", "cme:5055"),
        ]
    )

    # Any spelling of a name or alias answers the canonical field.
    assert registry.field_by_name(STANDARD_BRANCH, "TICKER").name == "Symbol"
    assert registry.field("px").name == "Price"
    assert registry.get_field(55) == registry.get_field("symbol")
    assert 44 in registry
    assert "44" not in registry, "a tag query never consults names"
    with pytest.raises(KeyError, match="tag 35"):
        registry.field_by_tag(35)

    # A bare tag and a bare name are the standard branch; the venue field is
    # reached by its identifier or by name inside its own dictionary.
    assert registry.field_by_id("cme:5055").fix.branch == "cme"
    assert registry.field_by_name("cme", "SYMBOL").fix.tag == 5055
    assert registry.field_by_name("standard", "symbol").fix.tag == 55
    assert registry.get_field_by_tag(5055) is None, "never crosses a branch"
    assert registry.get_field("cme:5055") is None, "a string key is a name"

    # A key another field holds *in the same branch* is a conflict naming
    # both, and the branch; nothing changes.
    with pytest.raises(ValueError, match="held by Symbol") as conflict:
        registry.insert(fix_field("SymbolSfx", "utf8", "standard:65", "ticker"))
    assert 'branch \\"standard\\"' in str(conflict.value)
    assert len(registry) == 3

    # A merge keeps what only the stored field declared and adds the rest.
    incoming = fix_field("SYMBOL", "utf8", "standard:55", "Sym")
    incoming.fix.tags = [65]
    registry.update(incoming)
    merged = registry.field_by_tag(65)
    assert merged.name == "SYMBOL"
    assert merged.fix.aliases == ["Sym", "Ticker"]
    # A datatype disagreement is refused, never widened.
    with pytest.raises(ValueError):
        registry.update(fix_field("Symbol", "large_utf8", "standard:55"))
    assert registry.field_by_tag(55).dtype == DataType("utf8")

    # Iteration is branch-major, then by tag.
    assert [field.fix.id for field in registry] == [
        "cme:5055",
        "standard:44",
        "standard:55",
    ]
    assert registry.remove("sym").name == "SYMBOL"
    assert registry.get_field_by_tag(65) is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Field, fix } = require('yggdryl')

    function fixField(name, dtype, identifier, ...aliases) {
      const field = Field.from(`${name}: ${dtype}`)
      field.fix.id = identifier
      if (aliases.length !== 0) field.fix.aliases = aliases
      return field
    }

    const registry = fix.FixRegistry.fromFields([
      fixField('Symbol', 'utf8', 'standard:55', 'Ticker'),
      fixField('Price', 'decimal128(20, 8)', 'standard:44', 'Px'),
      // The venue dictionary reuses the name `Symbol`, the normal case.
      fixField('Symbol', 'utf8', 'cme:5055'),
    ])

    // Any spelling of a name or alias answers the canonical field.
    assert.equal(registry.fieldByName(fix.STANDARD_BRANCH, 'TICKER').name, 'Symbol')
    assert.equal(registry.field('px').name, 'Price')
    assert.ok(registry.getField(55).equals(registry.getField('symbol')))
    assert.equal(registry.has(44), true)
    assert.equal(registry.has('44'), false, 'a tag query never consults names')
    assert.throws(() => registry.fieldByTag(35), /tag 35/)

    // A bare tag and a bare name are the standard branch; the venue field is
    // reached by its identifier or by name inside its own dictionary.
    assert.equal(registry.fieldById('cme:5055').fix.branch, 'cme')
    assert.equal(registry.fieldByName('cme', 'SYMBOL').fix.tag, 5055)
    assert.equal(registry.fieldByName('standard', 'symbol').fix.tag, 55)
    assert.equal(registry.getFieldByTag(5055), null, 'never crosses a branch')
    assert.equal(registry.getField('cme:5055'), null, 'a string key is a name')

    // A key another field holds *in the same branch* is a conflict naming
    // both, and the branch; nothing changes.
    assert.throws(
      () => registry.insert(fixField('SymbolSfx', 'utf8', 'standard:65', 'ticker')),
      /held by Symbol/,
    )
    assert.equal(registry.size, 3)

    // A merge keeps what only the stored field declared and adds the rest.
    const incoming = fixField('SYMBOL', 'utf8', 'standard:55', 'Sym')
    incoming.fix.tags = [65]
    registry.update(incoming)
    const merged = registry.fieldByTag(65)
    assert.equal(merged.name, 'SYMBOL')
    assert.deepEqual(merged.fix.aliases, ['Sym', 'Ticker'])
    // A datatype disagreement is refused, never widened.
    assert.throws(() => registry.update(fixField('Symbol', 'large_utf8', 'standard:55')))
    assert.ok(registry.fieldByTag(55).dtype.equals(DataType.from('utf8')))

    // Iteration is branch-major, then by tag.
    assert.deepEqual(
      [...registry].map((field) => field.fix.id),
      ['cme:5055', 'standard:44', 'standard:55'],
    )
    assert.equal(registry.remove('sym').name, 'SYMBOL')
    assert.equal(registry.getFieldByTag(65), null)
    // `remove` reads a string as a standard name, so a vendor field leaves by
    // its identifier.
    assert.equal(registry.removeById('cme:5055').name, 'Symbol')
    assert.equal(registry.size, 1)
    ```

## Storage is two trees of shards under one handle

A registry reads and writes through one [`IOBase`](io.md) folder handle and nothing else, into two
trees:

```text
<root>/primitive/<branch>/<shard>.json
<root>/nested/<branch>/<shard>.json
```

**`primitive` holds the fields whose datatype is one scalar value and `nested` the ones whose
datatype carries a subtree.** What counts as nested is `field.dtype().is_nested()`, the core
predicate and the only one: it already unwraps a dictionary to its value type and a run-end
encoding to its values, so a dictionary-encoded Struct is nested and a dictionary-encoded Utf8 is
not. In FIX terms the nested fields are exactly the components - a Struct whose children are its
members - and the repeating groups - a List of that Struct; every price, quantity, timestamp and
character code is primitive. Components and groups are a small minority of any dictionary but they
carry a whole subtree each, so isolating them means the lookup a transcriber performs thousands of
times per message touches only the small, hot half, and a dictionary's nested definitions can be
read, written and skipped as a unit.

`shard = tag / 100` is unchanged inside each tree, so `standard:55` is
`primitive/standard/0.json` and `cme:5001` is `primitive/cme/50.json`: the branch level sits above
the shard level because a shard index is only unique inside one dictionary, a tag then reaches
exactly one shard by arithmetic, and an alternate tag is an index entry that never fans a field
across shards. Each shard is a JSON array of the core field document - what `Field::into_value`
projects - ordered by canonical identifier and rendered indented, so the whole `fix:` namespace
persists through the path every field already has and the tracked seed reads in a diff. Nothing
else is composed: no envelope, no version marker. The branch segment is the canonical lowercase
text, whose grammar - a leading letter, no separators, no `.` or `..` - makes it a safe single path
segment.

**Both trees are optional.** A dictionary of only scalars writes no `nested/` folder at all, a
dictionary of only components and groups writes no `primitive/`, and a root holding neither loads
as the empty registry.

**The record is authoritative and the folder is layout.** A field's own `fix:branch` decides
which dictionary it belongs to and its own datatype decides which tree it belongs to; a field whose
branch contradicts the folder, or whose datatype contradicts the tree, is a typed error naming
both. A standard field states nothing, so it costs no key.

`from_handle` is the one loader. It lists each tree and expects **branch folders only**: a leaf
directly under `primitive/` or `nested/` is a typed error naming it, and a folder whose name is not
a branch keeps `FixBranch::from_str`'s typed parse failure and its byte position with the folder
URL attached. Inside a branch folder it reads every `<n>.json` leaf and inserts its fields, leaving
anything else alone; every shard of both trees is loaded on open, because a name has no numeric
structure to pick a shard with and a dictionary is small enough that loading it whole costs less
than lazy machinery. A folder that does not exist lists nothing and answers the empty registry, as
every handle's laziness contract says; a shard that exists but does not parse, holds a field
without a tag or with a tag another shard owns, or holds a field the registry refuses, is a typed
error naming the shard's URL. `write_into` routes each field to its tree and writes every populated
shard whole - creation is a write consequence - then removes any `<n>.json` no field populates, any
branch folder the registry holds no field for, and any tree that ends up empty, so a reload cannot
resurrect a removed field, a dropped dictionary, or a definition that moved trees when its datatype
changed.

Nothing reads a `records/` folder any more, and nothing reads the flat
`<root>/records/<shard>.json` that came before it. The project keeps no backward compatibility, so
neither layout is migrated. Neither is silently tolerated either: a root that still holds `records/`
is refused, naming the folder, because it is a dictionary this loader cannot read rather than the
absence an untouched machine has. Answering it with an empty registry would turn every later lookup
into a wrong answer instead of a failure, which is the one thing loading must never do. Delete the
folder, or point at a root written by this version.

=== "Rust"

    ```rust
    use yggdryl::io::IOBase;
    use yggdryl::local::Folder;
    use yggdryl::{DataType, FixId, FixBranch, FixRegistry};

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
    // One venue field, which lands in its own branch folder.
    let cme = FixBranch::from_str("cme")?;
    let mut trade = DataType::Utf8.nullable_field("TradeID");
    trade.as_fix_mut().set_id(&FixId::from_parts(cme.clone(), 5001)?)?;
    fields.push(trade);
    // One repeating group, which is the only field of the nested tree.
    let item = DataType::from_fields([DataType::Utf8.nullable_field("PartyID")])?
        .required_field("item");
    let mut parties = DataType::list(item).nullable_field("NoPartyIDs");
    parties.as_fix_mut().set_tag(453)?;
    fields.push(parties);
    let mut registry = FixRegistry::from_fields(fields)?;
    registry.write_into(&mut folder)?;

    let shards = |tree: &str, branch: &str| -> yggdryl::Result<Vec<String>> {
        let mut names: Vec<String> = std::fs::read_dir(root.join(tree).join(branch))?
            .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
            .collect::<Result<_, _>>()?;
        names.sort();
        Ok(names)
    };
    // The alternate tag 20 wrote nothing into shard 0 beyond MsgType and StopPx.
    assert_eq!(shards("primitive", "standard")?, ["0.json", "1.json"]);
    // Each branch owns its own shard arithmetic: 5001 / 100 is 50.
    assert_eq!(shards("primitive", "cme")?, ["50.json"]);
    // The group is nested, so it is the nested tree's only shard: 453 / 100.
    assert_eq!(shards("nested", "standard")?, ["4.json"]);

    let reloaded = FixRegistry::from_handle(&folder)?;
    assert_eq!(reloaded, registry);
    assert_eq!(reloaded.field_by_tag(20)?.name(), "ExecType");
    assert_eq!(reloaded.field_by_tag(453)?.name(), "NoPartyIDs");
    assert_eq!(reloaded.field_by_id(&FixId::from_str("cme:5001")?)?.name(), "TradeID");

    // Removing the only field of a shard removes the shard on the next write,
    // emptying a branch removes its folder whole, and emptying a tree removes
    // the tree.
    registry.remove(100);
    registry.remove(150);
    registry.remove(453);
    registry.remove(&FixId::from_str("cme:5001")?);
    registry.write_into(&mut folder)?;
    assert!(!root.join("primitive").join("standard").join("1.json").exists());
    assert!(!root.join("primitive").join("cme").exists());
    assert!(!root.join("nested").exists());
    assert_eq!(FixRegistry::from_handle(&folder)?.len(), 2);

    // A leaf directly under a tree root is a typed error, never a silent
    // empty load.
    std::fs::write(root.join("primitive").join("0.json"), b"[]")?;
    let error = FixRegistry::from_handle(&folder).unwrap_err();
    assert!(error.to_string().contains("only branch folders"), "{error}");
    std::fs::remove_file(root.join("primitive").join("0.json"))?;

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

    import pytest

    from yggdryl import DataType, Field, fields as field_builders
    from yggdryl.fix import FixRegistry

    workspace = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-doc-fix-"))
    root = workspace / "dictionary"

    declared = []
    for tag, name in ((35, "MsgType"), (99, "StopPx"), (100, "NoAllocs"), (150, "ExecType")):
        field = Field(name, "utf8")
        field.fix.tag = tag
        declared.append(field)
    declared[3].fix.tags = [20]
    # One venue field, which lands in its own branch folder.
    trade = Field("TradeID", "utf8")
    trade.fix.id = "cme:5001"
    declared.append(trade)
    # One repeating group, which is the only field of the nested tree.
    item = Field("item", DataType.from_fields([Field("PartyID", "utf8")]), nullable=False)
    parties = field_builders.list("NoPartyIDs", item)
    parties.fix.tag = 453
    declared.append(parties)
    registry = FixRegistry.from_fields(declared)
    registry.write_into(root)


    def shards(tree: str, branch: str) -> list[str]:
        return sorted(path.name for path in (root / tree / branch).iterdir())


    # The alternate tag 20 wrote nothing into shard 0 beyond MsgType and StopPx.
    assert shards("primitive", "standard") == ["0.json", "1.json"]
    # Each branch owns its own shard arithmetic: 5001 / 100 is 50.
    assert shards("primitive", "cme") == ["50.json"]
    # The group is nested, so it is the nested tree's only shard: 453 / 100.
    assert shards("nested", "standard") == ["4.json"]

    reloaded = FixRegistry.from_handle(root)
    assert reloaded == registry
    assert reloaded.field_by_tag(20).name == "ExecType"
    assert reloaded.field_by_tag(453).name == "NoPartyIDs"
    assert reloaded.field_by_id("cme:5001").name == "TradeID"

    # Removing the only field of a shard removes the shard on the next write,
    # emptying a branch removes its folder whole, and emptying a tree removes
    # the tree. `remove` reads a str key as a standard name, so the venue
    # field leaves by rebuilding the dictionary without it.
    registry.remove(100)
    registry.remove(150)
    registry.remove(453)
    kept = FixRegistry.from_fields(
        [field for field in registry if field.fix.branch == "standard"]
    )
    kept.write_into(root)
    assert not (root / "primitive" / "standard" / "1.json").exists()
    assert not (root / "primitive" / "cme").exists()
    assert not (root / "nested").exists()
    assert len(FixRegistry.from_handle(root)) == 2

    # A leaf directly under a tree root is a typed error, never a silent
    # empty load.
    stray = root / "primitive" / "0.json"
    stray.write_bytes(b"[]")
    with pytest.raises(ValueError, match="only branch folders"):
        FixRegistry.from_handle(root)
    stray.unlink()

    # A root left in the retired `records/` layout is refused, not read empty.
    retired = workspace / "retired"
    (retired / "records" / "standard").mkdir(parents=True)
    with pytest.raises(ValueError, match="records"):
        FixRegistry.from_handle(retired)

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
    const { Field, fields, fix } = require('yggdryl')

    const workspace = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-doc-fix-'))
    const root = path.join(workspace, 'dictionary')

    const declared = []
    for (const [tag, name] of [[35, 'MsgType'], [99, 'StopPx'], [100, 'NoAllocs'], [150, 'ExecType']]) {
      const field = Field.from(`${name}: utf8`)
      field.fix.tag = tag
      declared.push(field)
    }
    declared[3].fix.tags = [20]
    // One venue field, which lands in its own branch folder.
    const trade = Field.from('TradeID: utf8')
    trade.fix.id = 'cme:5001'
    declared.push(trade)
    // One repeating group, which is the only field of the nested tree.
    const item = fields.struct('item', [Field.from('PartyID: utf8')], { nullable: false })
    const parties = fields.list('NoPartyIDs', item)
    parties.fix.tag = 453
    declared.push(parties)
    const registry = fix.FixRegistry.fromFields(declared)
    registry.writeInto(root)

    const shards = (tree, branch) => fs.readdirSync(path.join(root, tree, branch)).sort()
    // The alternate tag 20 wrote nothing into shard 0 beyond MsgType and StopPx.
    assert.deepEqual(shards('primitive', 'standard'), ['0.json', '1.json'])
    // Each branch owns its own shard arithmetic: 5001 / 100 is 50.
    assert.deepEqual(shards('primitive', 'cme'), ['50.json'])
    // The group is nested, so it is the nested tree's only shard: 453 / 100.
    assert.deepEqual(shards('nested', 'standard'), ['4.json'])

    const reloaded = fix.FixRegistry.fromHandle(root)
    assert.ok(reloaded.equals(registry))
    assert.equal(reloaded.fieldByTag(20).name, 'ExecType')
    assert.equal(reloaded.fieldByTag(453).name, 'NoPartyIDs')
    assert.equal(reloaded.fieldById('cme:5001').name, 'TradeID')

    // Removing the only field of a shard removes the shard on the next write,
    // emptying a branch removes its folder whole, and emptying a tree removes
    // the tree. A vendor field leaves by its identifier, because `remove`
    // reads a string as a standard name.
    registry.remove(100)
    registry.remove(150)
    registry.remove(453)
    registry.removeById('cme:5001')
    registry.writeInto(root)
    assert.equal(fs.existsSync(path.join(root, 'primitive', 'standard', '1.json')), false)
    assert.equal(fs.existsSync(path.join(root, 'primitive', 'cme')), false)
    assert.equal(fs.existsSync(path.join(root, 'nested')), false)
    assert.equal(fix.FixRegistry.fromHandle(root).size, 2)

    // A leaf directly under a tree root is a typed error, never a silent
    // empty load.
    const stray = path.join(root, 'primitive', '0.json')
    fs.writeFileSync(stray, '[]')
    assert.throws(() => fix.FixRegistry.fromHandle(root), /only branch folders/)
    fs.rmSync(stray)

    // A root left in the retired `records/` layout is refused, not read empty.
    const retired = path.join(workspace, 'retired')
    fs.mkdirSync(path.join(retired, 'records', 'standard'), { recursive: true })
    assert.throws(() => fix.FixRegistry.fromHandle(retired), /records/)

    // A folder that is not there loads as empty and is not created.
    const absent = path.join(root, 'absent')
    assert.equal(fix.FixRegistry.fromHandle(absent).size, 0)
    assert.equal(fs.existsSync(absent), false)

    fs.rmSync(workspace, { recursive: true, force: true })
    ```

The repository ships a seed dictionary at `config/fix/primitive/standard/<shard>.json` and
`config/fix/nested/standard/4.json`, written by `write_into` itself: a small FIX 4.4 subset - the
standard header and trailer, the order and execution fields,
the `Parties` component as a repeating group - with the specification's wording as each
description, a display name where FIX has one, declared aliases such as `Ticker` for `Symbol`, and
one alternate tag (`20` for `ExecType`, the pre-4.3 `ExecTransType` whose role it absorbed). It is
what the tests, benchmarks and this page resolve against; it is *not* in the default registry's
resolution order.

=== "Rust"

    ```rust
    use yggdryl::local::Folder;
    use yggdryl::{FixBranch, FixRegistry};

    let standard = FixBranch::STANDARD;
    let seed = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("config").join("fix");
    let registry = FixRegistry::from_handle(&Folder::new(seed)?)?;

    assert_eq!(registry.field_by_tag(55)?.name(), "Symbol");
    assert_eq!(registry.field_by_name(&standard, "ticker")?.name(), "Symbol");
    assert_eq!(registry.field_by_tag(20)?.name(), "ExecType");
    assert_eq!(registry.field_by_path(&standard, "NoPartyIDs.PartyID")?.as_fix().tag()?, Some(448));
    assert_eq!(registry.field_by_name(&standard, "ClOrdID")?.display(), Some("Client order ID"));
    // Every seed field is a specification field, so none states a branch.
    assert!(registry.iter().all(|field| !field.has_metadata("fix:branch")));
    assert!(registry.len() < 40);
    ```

=== "Python"

    ```python
    import pathlib

    from yggdryl.fix import STANDARD_BRANCH, FixRegistry

    # The seed this repository tracks, named from the repository root.
    seed = pathlib.Path("config/fix").resolve()
    registry = FixRegistry.from_handle(seed)

    assert registry.field_by_tag(55).name == "Symbol"
    assert registry.field_by_id("standard:55").name == "Symbol"
    assert registry.field_by_name(STANDARD_BRANCH, "ticker").name == "Symbol"
    assert registry.field_by_tag(20).name == "ExecType"
    assert registry.field_by_path(STANDARD_BRANCH, "NoPartyIDs.PartyID").fix.tag == 448
    assert registry.field_by_name(STANDARD_BRANCH, "ClOrdID").display == "Client order ID"
    # Every seed field is a specification field, so none states a branch.
    assert all("fix:branch" not in field.metadata for field in registry)
    assert len(registry) < 40
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    // The seed this repository tracks, named from the repository root.
    const standard = fix.STANDARD_BRANCH
    const registry = fix.FixRegistry.fromHandle(path.resolve('config/fix'))

    assert.equal(registry.fieldByTag(55).name, 'Symbol')
    assert.equal(registry.fieldById('standard:55').name, 'Symbol')
    assert.equal(registry.fieldByName(standard, 'ticker').name, 'Symbol')
    assert.equal(registry.fieldByTag(20).name, 'ExecType')
    assert.equal(registry.fieldByPath(standard, 'NoPartyIDs.PartyID').fix.tag, 448)
    assert.equal(registry.fieldByName(standard, 'ClOrdID').display, 'Client order ID')
    // Every seed field is a specification field, so none states a branch.
    assert.ok([...registry].every((field) => field.has('fix:branch') === false))
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

A message has a branch, and it is **derived, not declared**: `branch()` answers the root
field's own `fix:branch`, resolved once at construction - so a malformed one is a typed error
there rather than a silent miss later - and there is no constructor argument that could disagree
with it.

A bare tag or name then resolves in a fixed **two-step tier and no further**:

1. this message's own branch, when the identifier that would name is legal at all - that is,
   the tag is at or above `FixId::STANDARD_TAG_LIMIT`, or the message is already standard;
2. the standard branch.

That is what makes both halves of a real venue message reachable: `get_by_tag(5001)` finds the
venue's own field, and `get_by_tag(35)` still finds `MsgType`, which every FIX message carries.

The value accessors mirror the registry's and resolve through the linked registry, never a private
copy of its rules: `get_by_tag` / `by_tag` resolve the tag through that tier to its canonical name
and pick the root child of that name, falling back to a root child named by the tag's decimal text
- an unknown tag a transcriber retained is reachable, never dropped; `get_by_id` / `by_id` name a
dictionary exactly and do not tier, so a foreign branch simply misses; `get_by_name` / `by_name`
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

    // The message's branch is the root's own, and an identifier is exact.
    assert_eq!(msg.branch(), &yggdryl::FixBranch::STANDARD);
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
    from yggdryl.fix import STANDARD_BRANCH, FixMsg, FixRegistry

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

    # The message's branch is the root's own, and an identifier is exact.
    assert message.branch == STANDARD_BRANCH
    assert message.by_id("standard:38").as_py() == 100
    assert message.get_by_id("cme:5001") is None

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
    assert.equal(message.byId('standard:38').asJs(), 100)
    assert.equal(message.getById('cme:5001'), null)

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
- A tag below `FixId::STANDARD_TAG_LIMIT` is the FIX specification's own, so no other branch may
  claim it - as a canonical tag or as an alternate one, since an alternate tag resolves with the
  same power. The refusal names `fix:branch`, the limit and both sides, and it comes from
  `FixId::from_parts` wherever an identity is built: a setter, a read, an insert, or a shard load.
- `remove` takes a tag, an identifier or a name, never a path: a component's member is not a
  registry entry. A bare tag or name means the standard branch here too.
- `from_handle` admits only branch folders directly under `primitive/` and `nested/`; a leaf there
  is a typed error, never a folder skipped into an empty load. Inside a branch
  folder it reads only leaves named `<n>.json` with a decimal `n`; a README beside them is ignored
  and left alone by `write_into`'s cleanup. A field stored in the wrong shard, in a folder its
  own `fix:branch` contradicts, or in the tree its own datatype contradicts, is refused with both
  sides named.
- The primitive and nested halves partition every index but not the identity space. A conflict is
  looked for in both halves before anything is written, so a repeating group can no more take a
  scalar's tag, name, alternate tag or alias than another scalar can.
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
| `get_field_by_tag` primitive hit | 32.3 ns |
| `get_field_by_tag` nested hit (`NoPartyIDs`, tag 453) | 93.1 ns |
| `get_field_by_tag` alternate-tag hit | 65.8 ns |
| `get_field_by_tag` miss | 72.2 ns |
| `get_field_by_id` vendor hit, over 1034 fields in two branches | 128.1 ns |
| `get_field_by_name` hit | 81.8 ns |
| `get_field_by_name` hit, query differently cased | 81.2 ns |
| `get_field_by_name` alias hit | 191.5 ns |
| `get_field_by_name` miss | 175.4 ns |
| `get_field_by_name` vendor-branch hit, over 1034 fields | 89.7 ns |
| `get_field_by_name` vendor-branch alias hit, over 1034 fields | 175.2 ns |
| `get_field_by_tag` cross-branch miss, over 1034 fields | 222.1 ns |
| `get_field_by_tag` standard hit, over 1034 fields in two branches | 137.8 ns |
| `get_field(FixKey::Tag)` generic tag hit | 32.4 ns |
| `get_field("Symbol")` generic name hit | 89.2 ns |
| `field(55)` failing-half tag hit | 31.4 ns |
| `get_field_by_path`, one segment | 142.6 ns |
| `get_field_by_path`, two segments (`NoPartyIDs.PartyID`) | 389.4 ns |
| `get_field_by_path`, three segments (`NoPartyIDs.item.PartyRole`) | 389.2 ns |
| `FixId::to_string` | 197.6 ns |
| `FixId::from_str("cme:5001")` | 143.1 ns |
| baseline `HashMap<FixId, Field>` hit | 39.4 ns |
| baseline `HashMap<i32, Field>` tag hit | 19.3 ns |
| baseline `HashMap<String, Field>` hit after lowercasing the query | 97.0 ns |
| tag hit over 4034 fields, all primitive | 179.5 ns |
| name hit over 4034 fields, all primitive | 106.4 ns |
| alias hit over 4034 fields, all primitive | 188.0 ns |
| primitive tag hit over 4034 fields, one in fifty nested | 201.3 ns |
| nested tag hit over 4034 fields, one in fifty nested | 268.2 ns |

Mutation clones the dictionary in the batch setup and hands it back as the
routine's output, so neither the clone nor the drop is inside the timer:

| mutation | estimate |
| --- | ---: |
| `insert` into the seed | 5.13 us |
| `insert` into 4034 fields | 115 us |
| `from_fields` over 4034 fields | 14.1 ms |
| `update` merging an alias and an alternate tag into the seed | 9.93 us |
| the same `update` over 4034 fields | 17.9 us |
| `remove` from 4034 fields | 10.6 us |
| `set_branch` on a field whose tags allow it | 816 ns |
| `set_id` moving a field into a vendor branch | 1.05 us |
| `set_id` back to the standard branch | 569 ns |
| `set_branch` refused for a specification tag | 654 ns |

| storage | estimate |
| --- | ---: |
| `from_handle`, 1 shard of 10 fields | 962 us |
| `from_handle`, 10 shards of 10 fields | 5.55 ms |
| `from_handle`, 100 shards of 10 fields | 62.1 ms |
| `from_handle`, the seed (4 shards in two trees, 34 fields) | 2.87 ms |
| `from_handle`, two branches (1034 fields, 14 shards) | 19.8 ms |
| `write_into`, 100 shards | 324 ms |
| `write_into`, two branches (1034 fields, 14 shards) | 150 ms |
| explicit-location autoload of the seed (URL parse, folder, load) | 2.82 ms |

The generic accessor costs the specialized one plus its dispatch, which on a tag is within the
noise of the lookup itself: 32.4 ns against 32.3 ns. The specialized pair still exists so a caller
that already knows which key it holds pays no dispatch, but the dispatch is no longer most of the
call the way it was when a tag probe was four nanoseconds.

A folded name hit costs what the plain `HashMap<String, Field>` baseline costs *before* that
baseline lowercases its query - the fold happens inside the hash, so no folded copy is built.

**What the primitive/nested split cost and did not buy.** The point of the split is locality: the
hot half a transcriber probes per wire tag holds only the scalars. On this dictionary shape the
numbers do not show that as a win, and the honest reading is that they show a small loss:

- the primitive tag hit over the seed is 32.3 ns, where the single index measured 27.4 ns. The
  seed's nested half holds one field of 34, so the primitive map is one entry smaller than the
  undivided one was - far too little to change a B-tree's depth, and the extra structure costs
  more than the one entry saves;
- the primitive tag hit over 4034 fields with one in fifty nested is 201.3 ns, against 179.5 ns for
  the same hit over 4034 all-primitive fields in the undivided shape. Again: 2% fewer entries in
  the hot map buys nothing measurable;
- what clearly did get slower is every probe that misses its first map. A tag miss went from
  48.4 ns to 72.2 ns, an alias hit from 136.5 ns to 191.5 ns, and a one-segment path from 86.5 ns
  to 142.6 ns, because each tier now reads two maps instead of one. A nested hit pays that too:
  93.1 ns over the seed, against 32.3 ns for a primitive one;
- `from_handle` of the seed went from 2.06 ms to 2.87 ms, because a load now lists two trees where
  it listed one, and reads four shards where it read three.

So the split earns its place on the layout, not on the lookup: the nested definitions are a
contiguous half that can be read, written and skipped as a unit, and a field's tree is decided by
the same predicate that decides its index half. **A dictionary whose nested share is a minority
does not get a faster primitive hit out of it, and this page does not claim one.** A dictionary
whose nested half were a large fraction would be the case that pays, and none is measured here.

The tag hit itself is still the number the `FixId` key moved: it was 4.5 ns against an 18.1 ns
`HashMap<i32, Field>`, and it is 32.3 ns against 19.3 ns. Every level of the identifier index
compares an inline branch string before an `i32`, and a `FixId` key is 32 bytes where an `i32`
was 4, so a node spans six cache lines rather than one. Against the baseline that answers the same
question - `HashMap<FixId, Field>`, 39.4 ns - the ordered index is still faster, and it is the
only one of the two that can answer `next_field_after`, `iter` and the store's branch-major
grouping at all. `HashMap<i32, Field>` is faster only because it cannot hold two branches: it
answers the ambiguous question the identity carries.

`from_handle` scales with the number of shards rather than with the fields in them: a shard costs
roughly half a millisecond to open, read and parse, so the seed's four shards cost about four
times one shard and a hundred shards cost about a hundred times one. The storage rows are
filesystem-bound and the wider ones move by tens of percent between runs; read them as orders of
magnitude rather than as a ranking.

### The Python boundary

```console
cd python && .venv/Scripts/python benchmarks/fix.py --iterations 2000
```

One local Windows x86_64 run of the release wheel (`maturin build --release`) under CPython 3.12,
median time per call over the same tracked seed of 34 fields, except the vendor rows, which run
over the seed beside a generated `cme` dictionary of 1000 fields. The sub-microsecond rows move by
a third between runs on this machine, so read them as one order of magnitude rather than as a
ranking of one against another:

| Python operation | estimate |
| --- | ---: |
| `get_field_by_tag(55)` hit | 196 ns |
| `get_field_by_tag(20)` alternate-tag hit | 235 ns |
| `get_field_by_id("standard:55")` hit | 408 ns |
| `get_field_by_name("standard", "Symbol")` hit | 303 ns |
| `get_field_by_name("standard", "symbol")` hit, folded query | 298 ns |
| `get_field_by_name("standard", "ticker")` alias hit | 404 ns |
| `get_field_by_tag(9999)` miss | 186 ns |
| `get_field_by_name("standard", "Nope")` miss | 264 ns |
| `get_field_by_id("cme:5001")` miss | 308 ns |
| `get_field(55)` generic tag hit | 205 ns |
| `field_by_path`, one segment | 357 ns |
| `field_by_path`, two segments (`NoPartyIDs.PartyID`) | 568 ns |
| `get_field_by_id("cme:5001")` vendor hit, two branches | 519 ns |
| `get_field_by_name("cme", ...)` vendor hit, two branches | 304 ns |
| `get_field_by_name("cme", ...)` vendor alias hit, two branches | 493 ns |
| `get_field_by_tag(5001)` cross-branch miss, two branches | 288 ns |
| `get_field_by_tag(55)` standard hit, two branches | 304 ns |
| `field.fix.branch` | 549 ns |
| `field.fix.id` | 625 ns |
| `FixMsg.get_by_tag(55)` | 251 ns |
| `FixMsg.get_by_id("standard:55")` | 386 ns |
| `FixMsg.get_by_name("ticker")` | 367 ns |
| `FixMsg.get_by_path("NoPartyIDs.0.PartyID")` | 711 ns |
| `FixMsg.branch` | 148 ns |
| `from_handle`, the seed (4 shards in two trees, 34 fields) | 2.99 ms |
| `from_handle`, 1000 generated fields (11 shards) | 16.7 ms |

A hit costs the native lookup plus one crossing: the key is read once at the boundary and the
answer is wrapped as a `Field` or a `Scalar`, which clones the stored value rather than borrowing
it. That wrapping is what the numbers are almost entirely made of - the native tag hit is 32.3 ns,
an order of magnitude below the crossing - which is why the tiers the Rust table separates are
indistinguishable here, and why a caller resolving the same field repeatedly should hold the
answer rather than ask again. A miss is the one case that is reliably cheaper, because nothing is
wrapped.

**What a branch and an identifier cost at the boundary.** Both cross as text and are parsed on
every call, which is the price of having no class for either: `get_field_by_id("standard:55")` is
408 ns against 196 ns for the same field by tag, and the 212 ns between them is
`FixId::from_str` - a branch validated and folded, then a decimal tag - not a slower lookup, since
the native identifier probe is what the tag probe redirects to anyway. A branch-qualified name is
303 ns against 264 ns for the same call's miss, so the extra argument costs about what one short
string coercion costs. `field.fix.branch` and `field.fix.id` are the dearest rows on the table
because each is a metadata read plus a fresh Python `str`, and `id` renders one. A caller in a
loop should hold the answer, exactly as it should hold a resolved `Field`.

`from_handle` stays within a few percent of the native load - 2.99 ms against 2.87 ms - because
the shards are listed, read and parsed natively and only the finished registry crosses; it also
moved with the core, which now lists two trees and reads four shards where it listed one and read
three.

### The JavaScript boundary

```console
npm run --prefix node bench:fix
```

One local Windows x86_64 run (AMD Ryzen 5 150) of the release addon
(`npm run --prefix node build`) under Node.js v24.18.0, whole-loop rate over the same tracked seed
of 34 fields, except the vendor rows, which run over the seed beside a generated `cme` dictionary
of 1000 fields; the two loads run a thousandth of the hit count. The sub-microsecond rows move by a
third between runs on this machine, so read them as one order of magnitude rather than as a ranking
of one against another:

| JavaScript operation | rate | per call |
| --- | ---: | ---: |
| `getFieldByTag(55)` hit | 320k/s | 3.12 us |
| `getFieldByTag(20)` alternate-tag hit | 308k/s | 3.25 us |
| `getFieldById('standard:55')` hit | 268k/s | 3.73 us |
| `getFieldByName('standard', 'Symbol')` hit | 287k/s | 3.48 us |
| `getFieldByName('standard', 'symbol')` hit, folded query | 265k/s | 3.78 us |
| `getFieldByName('standard', 'ticker')` alias hit | 242k/s | 4.14 us |
| `getFieldByTag(9999)` miss | 1.23M/s | 815 ns |
| `getFieldByName('standard', 'Nope')` miss | 884k/s | 1.13 us |
| `getFieldById('cme:5001')` miss | 929k/s | 1.08 us |
| `getField(55)` generic tag hit | 319k/s | 3.14 us |
| `getField('Symbol')` generic name hit | 283k/s | 3.53 us |
| `fieldByPath`, one segment | 273k/s | 3.66 us |
| `fieldByPath`, two segments (`NoPartyIDs.PartyID`) | 223k/s | 4.48 us |
| `getFieldById('cme:5001')` vendor hit, two branches | 219k/s | 4.56 us |
| `getFieldByName('cme', ...)` vendor hit, two branches | 269k/s | 3.72 us |
| `getFieldByName('cme', ...)` vendor alias hit, two branches | 264k/s | 3.79 us |
| `getFieldByTag(5001)` cross-branch miss, two branches | 1.13M/s | 886 ns |
| `getFieldByTag(55)` standard hit, two branches | 302k/s | 3.31 us |
| `removeById('cme:9999')` miss, two branches | 848k/s | 1.18 us |
| `field.fix.branch` | 238k/s | 4.19 us |
| `field.fix.id` | 223k/s | 4.48 us |
| `FixMsg.getByTag(55)` | 311k/s | 3.21 us |
| `FixMsg.getById('standard:55')` | 261k/s | 3.83 us |
| `FixMsg.getByName('ticker')` | 211k/s | 4.73 us |
| `FixMsg.getByPath('NoPartyIDs.0.PartyID')` | 249k/s | 4.01 us |
| `FixMsg.branch` | 1.20M/s | 834 ns |
| `fromHandle`, the seed (4 shards in two trees, 34 fields) | 281/s | 3.56 ms |
| `fromHandle`, 1000 generated fields (11 shards) | 38/s | 26.3 ms |

A miss is the honest price of the crossing itself: 815 ns for the key coercion, the native probe,
and `null` back - the same order as the 641 ns a bare `registry.size` costs on this machine, which
is the crossing with no lookup in it at all. Every hit adds the wrapper the answer is put in, and
that wrapper is what the rest of the numbers are made of: `field.clone()` on an already-held native
`Field` costs 2.99 us here, so a 3.12 us tag hit is very nearly one `Field` materialization and
nothing else. The native tag hit is 32.3 ns, two orders of magnitude below the crossing, which is
why the tiers the Rust table separates are indistinguishable here and why a caller resolving the
same field repeatedly should hold the answer rather than ask again. `FixMsg`'s accessors wrap a
`Scalar` instead and cost the same shape.

**What a branch and an identifier cost at the boundary.** Both cross as text and are parsed on
every call, which is the price of having no class for either. The misses isolate that price,
because nothing is wrapped in them: `getFieldById('cme:5001')` is 1.08 us against 815 ns for a tag
miss, so `FixId::from_str` - a branch validated and folded, then a decimal tag - is a few hundred
nanoseconds, not a slower lookup, since the native identifier probe is what the tag probe redirects
to anyway. `removeById`'s miss is 1.18 us, the same parse plus the mutation's uniqueness check.
`field.fix.branch` and `field.fix.id` are the dearest rows on the table because `field.fix` builds
a fresh protocol view per access before the property is even read, and `id` then renders a new
JavaScript string; a caller in a loop should hold the view, exactly as it should hold a resolved
`Field`. `FixMsg.branch` is 834 ns because the branch was resolved once at construction and only
the string crosses.

`fromHandle` stays close to the native load - the shards are listed, read and parsed natively, and
only the finished registry crosses - and scales with shard count, not with the fields in them; it
also moved with the core, which now lists two trees and reads four shards where it listed one and
read three.
