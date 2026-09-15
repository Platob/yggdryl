# Explorer

Search the native FIX catalog and inspect the fields, components and groups it stores; a message is a component carrying `fix:msgtype`.

## Contract

| Surface | Contract |
| --- | --- |
| Source | `scripts/build_docs_fix.js` runs the native package over `config/fix`, retaining compact stored documents and adding native built-ins omitted by persistence. |
| Catalog | Three categories of native `Field` documents: `fields`, `components`, `groups`; messages are components carrying `fix:msgtype`, and enum codes stay inline on fields. |
| Search | Filters names, tags, aliases, `fix:identifiers` and descriptions; it does not invoke registry lookup or parse FIX input. |
| References | A member button selects a search in its target category. The browser does not resolve or merge schemas. |
| Samples | [Decode](decode.md) and [Encode](encode.md) display recorded native codec results and emitted bytes. |

## Use

A List group and its scalar count have separate definitions: `NoPartyIDs` is the integer field at tag 453; `Parties` is a group containing `Party` components. The built-in `altids` Map group instead owns tag and counter 65020 together, with no scalar counter column.

| Collection | Shipped documents | Live registry |
| --- | ---: | ---: |
| Scalar fields | 6,241 | 6,265 |
| Groups | 580 | 581 |
| Components, including messages | 928 | 929 |
| Messages, a subset of components | 181 | 182 |

The live additions are the crate's 25 scalar fields, the `altids` group and the `pluginconfig` message component; the shipped dictionary already defines `SendingTime` and `TransactTime`, so no standard clock is seeded beside them. The native fixed capture schema has 110 columns.

=== "Rust"

    ```rust
    use yggdryl::{DataType, FixCategory, FixId, FixRegistry, UPDATEDAT_TAG_NAME};
    use yggdryl::holder::local::Folder;

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = FixRegistry::from_handle(&Folder::new(root)?)?;
    assert_eq!(registry.len(), 6_265);
    assert_eq!(registry.field_by_tag(453)?.dtype(), &DataType::Int32);
    let parties = registry.definition(FixCategory::Groups, "parties")?;
    assert_eq!(parties.as_fix().counter()?, Some(453));
    assert_eq!(parties.as_fix().component(), Some("party"));
    let altids = registry.group_by_tag(65_020)?;
    assert_eq!(altids.name(), "altids");
    assert_eq!(altids.as_fix().tag()?, Some(65_020));
    assert_eq!(altids.as_fix().counter()?, Some(65_020));
    assert!(matches!(altids.dtype(), DataType::Map(map) if map.keys_sorted()));
    assert!(registry.get_field_by_tag(65_020).is_none());
    assert_eq!(registry.msgtype("D")?.as_str(), "D");
    // The crate's own columns are fields from tag 65001, held by every registry;
    // an identity is the tag and the name together.
    let (tag, name) = UPDATEDAT_TAG_NAME;
    let updatedat = registry.field_by_id(FixId::of(tag, name)?)?;
    assert_eq!(updatedat.name(), "updatedat");
    assert_eq!(updatedat.display(), Some("UpdatedAt"));
    assert_eq!(updatedat.as_fix().id()?, Some(FixId::of(65_003, "UpdatedAt")?));
    ```

=== "Python"

    ```python
    from pathlib import Path
    from yggdryl.fix import FixRegistry

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    assert len(registry) == 6_265
    assert str(registry.field_by_tag(453).dtype) == "int32"
    parties = registry.definition("groups", "parties")
    assert parties.fix.counter == 453
    assert parties.fix.component == "party"
    altids = registry.group_by_tag(65_020)
    assert altids.name == "altids" and altids.fix.tag == 65_020
    assert altids.fix.counter == 65_020
    assert altids.into_arrow().type.keys_sorted
    assert registry.get_field_by_tag(65_020) is None
    assert registry.msgtype("D").value == "D"
    # The crate's own columns are fields from tag 65001, held by every registry;
    # an identity is the tag and the name together, an int derived on every read.
    updatedat = registry.field_by_tag(65_003)
    assert updatedat.name == "updatedat"
    assert updatedat.display == "UpdatedAt"
    assert registry.field_by_id(updatedat.fix.id) == updatedat
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    assert.equal(registry.size, 6_265)
    assert.equal(registry.fieldByTag(453).dtype.toString(), 'int32')
    const parties = registry.definition('groups', 'parties')
    assert.equal(parties.fix.counter, 453)
    assert.equal(parties.fix.component, 'party')
    const altids = registry.groupByTag(65020)
    assert.equal(altids.name, 'altids')
    assert.equal(altids.fix.tag, 65020)
    assert.equal(altids.fix.counter, 65020)
    assert.match(altids.dtype.toString(), /keys_sorted=true/)
    assert.equal(registry.getFieldByTag(65020), null)
    assert.equal(registry.msgtype('D').asStr(), 'D')
    // The crate's own columns are fields from tag 65001, held by every registry;
    // an identity is the tag and the name together, a number derived on every read.
    const updatedat = registry.fieldByTag(65_003)
    assert.equal(updatedat.name, 'updatedat')
    assert.equal(updatedat.display, 'UpdatedAt')
    assert.ok(registry.fieldById(updatedat.fix.id).equals(updatedat))
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

Codes and `fix:identifiers` appear inside their owning field's detail panel. List groups carry a name-derived `fix:tag` beside their scalar `fix:counter`; the built-in `altids` Map uses its own reserved tag as its counter, and its entries Field is displayed directly from the native document. Search `altids` for that group, or `clordid` for declarations selecting that direct identifier; no browser-side reference expansion is involved.

## The capture row

The [Capture](capture.md#find-a-column) page searches the 110 fixed columns projected by the native schema. The [decoded samples](decode.md) also expose each message's native `Field`, `Scalar`, raw arrivals, facets and anomalies.

## Where it came from

<div class="ygg-fx" data-fix="sources" markdown="1">
This section displays the pinned source documents and needs JavaScript.
</div>

## Edges

- Results show at most sixty definitions initially; **Show more** adds the next sixty.
- Search is case-insensitive text filtering. Exact registry resolution - a tag's canonical holder before an alternate, a canonical name before an alias, an id exact - remains native API behavior.
- Stored references are displayed without expanding them in the browser. Native sample schemas show the codec's resolved result.
- Input and emitted sample bytes are displayed with visible escapes. **Copy displayed text** copies that representation.
- Every typed value, digest, facet and anomaly shown for a sample comes from the generated native result.

## Commands

```bash
node scripts/build_docs_fix.js
node scripts/build_docs_fix.js --check
/usr/bin/python3 -m mkdocs build --strict
```
