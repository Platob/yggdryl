# Playground

Every ASCII datatype, every registered code, every refusal, and a declared vocabulary, as the
package itself answered them.

Nothing on this page is computed in your browser. The JavaScript extension is a native Node addon,
so a browser cannot load it, and a documentation page may not reimplement what the package does. So
the outputs are generated: `scripts/build_docs_playground.js` runs the real package over a fixed
corpus and writes `assets/playground.json`, which is committed and checked for drift in CI; this
page fetches that manifest and renders it. A browser build would need a WebAssembly target of the
`node` crate that does not exist, and could not carry Parquet or Iceberg, so it is not offered here.

The contract these values prove is on the [datatype
page](datatype.md#ascii-widths-and-the-registered-codes); this page is the same contract with
every case laid out. To try a value of your own, add it to the corpus in the generator and rerun it:

```console
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

A member *is* the integer its ASCII value packs into - the value's own storage bytes, big-endian -
so the code is the same in every process and in every declaration, never a position in some
column's vocabulary. The declaration itself rides on the field under one reserved metadata key, so
it crosses Arrow, a file, and another runtime beside the datatype's own identity.

## Look up a value

<div class="ygg-pg" data-playground="lookup" markdown="1">
This section renders `assets/playground.json` and needs JavaScript.
</div>
