# Explorer

Search the native FIX catalog and inspect the fields, components and groups it stores; a message is a component carrying `fix:msgtype`.

## Contract

| Surface | Contract |
| --- | --- |
| Source | `scripts/build_docs_fix.js` runs the native package over `config/fix`, retaining compact stored documents and adding the crate's own definitions, which the shipped seed does not state. |
| Catalog | Three categories of native `Field` documents: `fields`, `components`, `groups`; messages are components carrying `fix:msgtype`, and enum codes stay inline on fields. |
| Search | Filters names, tags, `fix:names`, `fix:identifiers` and descriptions; it does not invoke registry lookup or parse FIX input. |
| References | A member button selects a search in its target category. The browser does not resolve or merge schemas. |
| Samples | [Decode](decode.md) and [Encode](encode.md) display recorded native codec results and emitted bytes. |

## Use

A List group and its scalar count have separate definitions: `NoPartyIDs` is the integer field at tag 453; `Parties` is a group containing `Party` components. The built-in `identifiers` and `metadata` Map groups instead own tag and counter together - 65020 and 65049 - with no scalar counter column.

| Collection | Shipped documents | Live registry |
| --- | ---: | ---: |
| Scalar fields | 6,241 | 6,259 |
| Groups | 580 | 582 |
| Components, including messages | 928 | 928 |
| Messages, a subset of components | 181 | 181 |

The live additions are the crate's 18 fields - `parentuuids` among them, a list of the identities a message descends from, which is one field under one name rather than a repeating group - and its two Map groups; the shipped dictionary already defines `SendingTime` and `TransactTime`, so no standard clock is seeded beside them. The native fixed capture schema has 115 columns.

=== "Rust"

    ```rust
    use yggdryl::{DataType, FixId, FixRegistry, CURRUNIX_TAG_NAME};
    use yggdryl::holder::local::Folder;

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = FixRegistry::from_handle(&Folder::new(root)?)?;
    // Every category is in the one length: the fields, the components and
    // the groups.
    assert_eq!(registry.len(), 7_769);
    // The walk is the same listing: the fields, then the definitions.
    assert_eq!(registry.iter().count(), 7_769);
    assert_eq!(registry.field_by_tag(453)?.dtype(), &DataType::Int32);
    let parties = registry.field_by_name("parties")?;
    assert_eq!(parties.as_fix().counter()?, Some(453));
    assert_eq!(parties.as_fix().component(), Some("party"));
    let identifiers = registry.field_by_counter(65_020)?;
    assert_eq!(identifiers.name(), "identifiers");
    assert_eq!(identifiers.as_fix().tag()?, Some(65_020));
    assert!(matches!(identifiers.dtype(), DataType::Map(map) if map.keys_sorted()));
    assert!(registry.get_field_by_tag(65_020).is_none());
    assert_eq!(registry.msgtype("D")?.as_str(), "D");
    // The crate's own columns are fields from tag 65003, held by every registry;
    // an identity is the tag and the name together.
    let (tag, name) = CURRUNIX_TAG_NAME;
    let currunix = registry.field_by_id(FixId::of(tag, name)?)?;
    assert_eq!(currunix.name(), "currunix");
    assert_eq!(currunix.display(), Some("CurrUnix"));
    assert_eq!(currunix.as_fix().id()?, Some(FixId::of(65_003, "CurrUnix")?));
    ```

=== "Python"

    ```python
    from pathlib import Path
    from yggdryl.fix import FixRegistry

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    # Every category is in the one length: the fields, the components and the
    # groups; iterating a Python registry walks the fields alone.
    assert len(registry) == 7_769
    assert sum(1 for _ in registry) == 6_259
    assert str(registry.field_by_tag(453).dtype) == "int32"
    parties = registry.field_by_name("parties")
    assert parties.fix.counter == 453
    assert parties.fix.component == "party"
    identifiers = registry.field_by_counter(65_020)
    assert identifiers.name == "identifiers" and identifiers.fix.tag == 65_020
    assert identifiers.into_arrow().type.keys_sorted
    assert registry.get_field_by_tag(65_020) is None
    assert registry.msgtype("D").value == "D"
    # The crate's own columns are fields from tag 65003, held by every registry;
    # an identity is the tag and the name together, an int derived on every read.
    currunix = registry.field_by_tag(65_003)
    assert currunix.name == "currunix"
    assert currunix.display == "CurrUnix"
    assert registry.field_by_id(currunix.fix.id) == currunix
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    // Every category is in the one size: the fields, the components and the
    // groups, which is what a Node registry iterates too.
    assert.equal(registry.size, 7769)
    assert.equal([...registry].length, registry.size)
    assert.equal(registry.fieldByTag(453).dtype.toString(), 'int32')
    const parties = registry.fieldByName('parties')
    assert.equal(parties.fix.counter, 453)
    assert.equal(parties.fix.component, 'party')
    const identifiers = registry.fieldByCounter(65020)
    assert.equal(identifiers.name, 'identifiers')
    assert.equal(identifiers.fix.tag, 65020)
    assert.match(identifiers.dtype.toString(), /keys_sorted=true/)
    assert.equal(registry.getFieldByTag(65020), null)
    assert.equal(registry.msgtype('D').asStr(), 'D')
    // The crate's own columns are fields from tag 65003, held by every registry;
    // an identity is the tag and the name together, a number derived on every read.
    const currunix = registry.fieldByTag(65_003)
    assert.equal(currunix.name, 'currunix')
    assert.equal(currunix.display, 'CurrUnix')
    assert.ok(registry.fieldById(currunix.fix.id).equals(currunix))
    ```

## What the registry holds

<div class="ygg-fx" data-fix="kpi" markdown="1">
This section displays the generated native registry counts and needs JavaScript.
</div>

## Search all three categories

Search `453` to see the scalar counter and group definitions that reference it. Select `groups` and search `Parties`, then open its declared occurrence to navigate to the `Party` component. Each result also exposes its exact native `Field` JSON.

<div class="ygg-fx" data-fix="catalog" markdown="1">
This section searches the generated native catalog and needs JavaScript.
</div>

Codes and `fix:identifiers` appear inside their owning field's detail panel. List groups carry a name-derived `fix:tag` beside their scalar `fix:counter`; the built-in `identifiers` and `metadata` Maps use their own reserved tag as their counter, and their entries Field is displayed directly from the native document. Search `identifiers` for that group, or `clordid` for declarations selecting that direct identifier; no browser-side reference expansion is involved.

## The capture row

The [Capture](capture.md#find-a-column) page searches the 118 fixed columns projected by the native schema, in the [nine bands](capture.md#the-columns-are-the-folded-names) they are ordered in. The [decoded samples](decode.md) also expose each message's native `Field`, `Scalar` and entries.

## Where it came from

<div class="ygg-fx" data-fix="sources" markdown="1">
This section displays the pinned source documents and needs JavaScript.
</div>

## Edges

- Results show at most sixty definitions initially; **Show more** adds the next sixty.
- Search is case-insensitive text filtering. Exact registry resolution - a tag's canonical holder before an alternate, a canonical name before an alias, an id exact - remains native API behavior.
- Stored references are displayed without expanding them in the browser. Native sample schemas show the codec's resolved result.
- Input and emitted sample bytes are displayed with visible escapes. **Copy displayed text** copies that representation.
- Every typed value and digest shown for a sample comes from the generated native result.

## Commands

```bash
node scripts/build_docs_fix.js
node scripts/build_docs_fix.js --check
/usr/bin/python3 -m mkdocs build --strict
```
