# Playground

Every ASCII width, every refusal, and the dictionary vocabulary, as the package itself answered
them.

Nothing on this page is computed in your browser. The JavaScript extension is a native Node addon,
so a browser cannot load it, and a documentation page may not reimplement what the package does. So
the outputs are generated: `scripts/build_docs_playground.js` runs the real package over a fixed
corpus and writes `assets/playground.json`, which is committed and checked for drift in CI; this
page fetches that manifest and renders it. A browser build would need a WebAssembly target of the
`node` crate that does not exist, and could not carry Parquet or Iceberg, so it is not offered here.

The contract these values prove is on the [datatype
page](datatype.md#ascii-widths-and-the-registered-names); this page is the same contract with
every case laid out. To try a value of your own, add it to the corpus in the generator and rerun it:

```console
node scripts/build_docs_playground.js
```

## The widths

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

## The dictionary vocabulary

<div class="ygg-pg" data-playground="dictionary" markdown="1">
This section renders `assets/playground.json` and needs JavaScript.
</div>

The vocabulary is a value the caller carries, not a process-global registry: a code is stable
exactly as far as that value travels.

## Look up a value

<div class="ygg-pg" data-playground="lookup" markdown="1">
This section renders `assets/playground.json` and needs JavaScript.
</div>
