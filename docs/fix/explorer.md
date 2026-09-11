# Explorer

Search the native FIX catalog and inspect the fields, messages, components and groups it stores.

## Contract

| Surface | Contract |
| --- | --- |
| Source | `scripts/build_docs_fix.js` runs the native package over `config/fix` and adds the crate's capture fields. |
| Catalog | Four categories of native `Field` documents, with enum codes inline on scalar fields. |
| Search | Filters stored names, tags, aliases and descriptions; it does not invoke registry lookup or parse FIX input. |
| References | A member button selects a search in its target category. The browser does not resolve or merge schemas. |
| Samples | [Decode](decode.md) and [Encode](encode.md) display recorded native codec results and emitted bytes. |

## Use

A count and its logical collection have separate definitions. `NoPartyIDs` is the integer field at tag 453; `Parties` is a group containing `Party` components. The count below is the dictionary this repository ships plus the twenty fields of the crate's own that every registry holds.

=== "Rust"

    ```rust
    use yggdryl::{DataType, FixCategory, FixId, FixRegistry, TIMESTAMP_TAG_NAME};
    use yggdryl::holder::local::Folder;

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = FixRegistry::from_handle(&Folder::new(root)?)?;
    assert_eq!(registry.len(), 6_261);
    assert_eq!(registry.field_by_tag(453)?.dtype(), &DataType::Int32);
    let parties = registry.definition(FixCategory::Groups, "parties")?;
    assert_eq!(parties.as_fix().counter()?, Some(453));
    assert_eq!(parties.as_fix().component(), Some("party"));
    assert_eq!(registry.msgtype("D")?.as_str(), "D");
    // The crate's own columns are fields from tag 65000, held by every registry;
    // an identity is the tag and the name together.
    let (tag, name) = TIMESTAMP_TAG_NAME;
    let timestamp = registry.field_by_id(FixId::of(tag, name)?)?;
    assert_eq!(timestamp.name(), "timestamp");
    assert_eq!(timestamp.display(), Some("Timestamp"));
    assert_eq!(timestamp.as_fix().id()?, Some(FixId::of(65_003, "Timestamp")?));
    ```

=== "Python"

    ```python
    from pathlib import Path
    from yggdryl.fix import FixRegistry

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    assert len(registry) == 6_261
    assert str(registry.field_by_tag(453).dtype) == "int32"
    parties = registry.definition("groups", "parties")
    assert parties.fix.counter == 453
    assert parties.fix.component == "party"
    assert registry.msgtype("D").value == "D"
    # The crate's own columns are fields from tag 65000, held by every registry;
    # an identity is the tag and the name together, an int derived on every read.
    timestamp = registry.field_by_tag(65_003)
    assert timestamp.name == "timestamp"
    assert timestamp.display == "Timestamp"
    assert registry.field_by_id(timestamp.fix.id) == timestamp
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    assert.equal(registry.size, 6_261)
    assert.equal(registry.fieldByTag(453).dtype.toString(), 'int32')
    const parties = registry.definition('groups', 'parties')
    assert.equal(parties.fix.counter, 453)
    assert.equal(parties.fix.component, 'party')
    assert.equal(registry.msgtype('D').asStr(), 'D')
    // The crate's own columns are fields from tag 65000, held by every registry;
    // an identity is the tag and the name together, a number derived on every read.
    const timestamp = registry.fieldByTag(65_003)
    assert.equal(timestamp.name, 'timestamp')
    assert.equal(timestamp.display, 'Timestamp')
    assert.ok(registry.fieldById(timestamp.fix.id).equals(timestamp))
    ```

## What the registry holds

<div class="ygg-fx" data-fix="kpi" markdown="1">
This section displays the generated native registry counts and needs JavaScript.
</div>

## Search all four categories

Search `453` to see the scalar counter and group definitions that reference it. Select `groups` and search `Parties`, then open its declared occurrence to navigate to the `Party` component. Each result also exposes its exact native `Field` JSON.

<div class="ygg-fx" data-fix="catalog" markdown="1">
This section searches the generated native catalog and needs JavaScript.
</div>

Codes appear inside their owning field's detail panel. A group carries the `fix:tag` derived from its own name, beside a `fix:counter` reference; it does not take the counter's tag or scalar datatype. Different message contexts remain separate definitions.

## The capture row

The [Capture](capture.md#find-a-column) page searches the ninety-three fixed columns projected by the native schema. The [decoded samples](decode.md) also expose each message's native `Field`, `Scalar`, raw arrivals, facets and anomalies.

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
python -m mkdocs build --strict
```
