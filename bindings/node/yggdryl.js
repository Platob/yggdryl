'use strict'

// The package entry: the per-crate namespaces over the generated native binding
// (`index.js`, produced by the napi CLI).
//
// napi-rs registers class constructors by JS class name in one addon-global
// registry, so a class named `Int64` in the `dtype` namespace would collide with
// the `field` and `scalar` ones. The native classes therefore carry a unique
// concern prefix (`DtypeInt64`, `FieldInt64`, `ScalarInt64`, ...), and this
// wrapper strips it into the per-crate namespaces — `yggdryl.dtype.Int64`,
// `yggdryl.field.Int64`, `yggdryl.scalar.Int64` — so the JS surface mirrors the
// crate tree with the same bare names as Rust and Python. `yggdryl.d.ts` mirrors
// this map for TypeScript.

const native = require('./index.js')

/** Every native export starting with `prefix`, re-keyed without it. */
function namespaceFrom(prefix) {
  const namespace = {}
  for (const [name, value] of Object.entries(native)) {
    if (name.startsWith(prefix)) {
      namespace[name.slice(prefix.length)] = value
    }
  }
  return namespace
}

module.exports = {
  core: native.core,
  dtype: namespaceFrom('Dtype'),
  field: namespaceFrom('Field'),
  scalar: namespaceFrom('Scalar'),
}
