# Explorer

The whole committed dictionary, live: what it holds, what each field is, which messages carry it, and the fixed row a capture lands in.

## Contract

| | |
| --- | --- |
| Source | `scripts/build_docs_fix.js` runs the real package over `config/fix` and the layouts manifest |
| Manifests | `docs/assets/fix.json` (index, layouts, corpus) and `docs/assets/fix-codes.json` (code sets, lineages), both committed and checked for drift by the addon build job |
| Browser | Renders those answers, and reads FIX text you type against them; it types no value and derives no facet |
| Pages | This one explores the dictionary, [Decode](decode.md) reads a frame, [Encode](encode.md) writes one |
| Contract proven | [Registry](registry.md) tiers and code sets, [FIX](index.md) vocabulary, [Capture](capture.md) projection |

## Use

The counts below are the dictionary this repository ships. The same numbers come out of the package.

=== "Rust"

    ```rust
    use yggdryl::FixRegistry;
    use yggdryl::holder::local::Folder;

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = FixRegistry::from_handle(&Folder::new(root)?)?.with_crate_fields()?;
    assert_eq!(registry.len(), 6_210);
    assert_eq!(registry.field_by_tag(35)?.name(), "msgtype");
    // A repeating group is reached through its counter, and the crate's own
    // columns are ordinary fields on their own branch.
    let standard = yggdryl::FixBranch::STANDARD;
    assert_eq!(registry.field_by_path("nopartyids.partyid", Some(&standard))?.as_fix().tag()?, Some(448));
    assert_eq!(registry.field_by_id("30004:yggdryl".parse()?)?.name(), "timestamp");
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import STANDARD_BRANCH, FixRegistry

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    registry.with_crate_fields()
    assert len(registry) == 6_210
    assert registry.field_by_tag(35).name == "msgtype"
    # A repeating group is reached through its counter, and the crate's own
    # columns are ordinary fields on their own branch.
    assert registry.field_by_path("nopartyids.partyid", STANDARD_BRANCH).fix.tag == 448
    assert registry.field_by_id("30004:yggdryl").name == "timestamp"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    registry.withCrateFields()
    assert.equal(registry.size, 6_210)
    assert.equal(registry.fieldByTag(35).name, 'msgtype')
    // A repeating group is reached through its counter, and the crate's own
    // columns are ordinary fields on their own branch.
    assert.equal(registry.fieldByPath('nopartyids.partyid', fix.STANDARD_BRANCH).fix.tag, 448)
    assert.equal(registry.fieldById('30004:yggdryl').name, 'timestamp')
    ```

## What the dictionary holds

<div class="ygg-fx" data-fix="kpi" markdown="1">
This section renders `assets/fix.json` and needs JavaScript.
</div>

## Find a field

Search matches the tag, the canonical name, the display spelling, every alias and the specification's own wording, all at once. Open a row for its datatype, its branch, its code set, its lineage and the message types that carry it.

<div class="ygg-fx" data-fix="fields" markdown="1">
This section renders `assets/fix.json` and needs JavaScript.
</div>

A field's identity is its branch and its tag, and a lookup resolves through [four tiers](registry.md#tiers): canonical identifier, alternate identifier, canonical name, alias. The explorer searches all of them at once because a reader with a tag in front of them does not know which tier will answer.

## Walk a message

A message type is a layout: fields, components, and repeating groups, nested as deep as the specification put them. Each block opens on demand, because a `NewOrderSingle` reaches over two hundred fields through its components.

<div class="ygg-fx" data-fix="messages" markdown="1">
This section renders `assets/fix.json` and needs JavaScript.
</div>

A `groupRef` names a group's own identifier, which is neither a tag nor a component identifier, so `config/fix/layouts.json` records all three tables and the tree resolves without guessing.

## The fixed row

A day of session log becomes one table with the same columns whatever arrived, resolved once by a projection. The eighty-nine columns, and the filter over them, are on the [Capture](capture.md#find-a-column) page.

## Where it came from

<div class="ygg-fx" data-fix="sources" markdown="1">
This section renders `assets/fix.json` and needs JavaScript.
</div>

## Edges

- The explorer states nothing the package did not answer. A value it cannot show — a typed row, a digest, a derived facet, an anomaly — is shown only for the frames in the generated corpus, where the package's own answer is carried in the manifest.
- The code sets and the lineages are a second manifest, fetched behind the first paint; a field panel says so while it waits.
- A group's own datatype spells its whole `item` Struct, so the index carries the shape and the member tags rather than the transcription.
- The search list renders sixty matches at a time. The count above it is the whole result, not what is drawn.

## Commands

```bash
node scripts/build_docs_fix.js
node scripts/build_docs_fix.js --check
python -m mkdocs build --strict
```
