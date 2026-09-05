# Playground

Every ASCII datatype, every registered code, every refusal, and a declared vocabulary, as the package itself answered them.

## Contract

| | |
| --- | --- |
| Source | `scripts/build_docs_playground.js` runs the real Node package over a fixed corpus |
| Manifest | `docs/assets/playground.json`, committed and checked for drift by the addon build job |
| Browser | Renders the manifest only; nothing is computed client-side (the addon is native, so no WebAssembly target exists) |
| Contract proven | The [ASCII page](ascii.md): widths, registered codes, packed integers, declared vocabularies |

To try a value of your own, add it to the generator's corpus and regenerate:

```bash
node scripts/build_docs_playground.js
```

## The ASCII datatypes and the codes

<div class="ygg-pg" data-playground="widths" markdown="1">
This section renders `assets/playground.json` and needs JavaScript.
</div>

## Encode: text into storage

<div class="ygg-pg" data-playground="encode" markdown="1">
This section renders `assets/playground.json` and needs JavaScript.
</div>

## Decode: storage into text

<div class="ygg-pg" data-playground="decode" markdown="1">
This section renders `assets/playground.json` and needs JavaScript.
</div>

## A declared vocabulary

<div class="ygg-pg" data-playground="vocabulary" markdown="1">
This section renders `assets/playground.json` and needs JavaScript.
</div>

A member is the integer its ASCII value packs into (the storage bytes, big-endian), so the code is the same in every process. The declaration rides on the field under one reserved metadata key, so it crosses Arrow, a file, and another runtime.

## Look up a value

<div class="ygg-pg" data-playground="lookup" markdown="1">
This section renders `assets/playground.json` and needs JavaScript.
</div>
