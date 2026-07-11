/* Hand-written namespace map over the NAPI-generated `index.js`.
 *
 * The native addon (auto-generated `index.js`) is regenerated on every `napi build`, so
 * cross-namespace aliases that can't be expressed with `#[napi(namespace = ...)]` live
 * here instead. Today that is the one merged type: the `u8` buffer *is* the byte store,
 * so `yggdryl.buffer.U8Buffer` is `yggdryl.io.ByteBuffer` (see `CLAUDE.md` — `ByteBuffer`
 * and `U8Buffer` are one type).
 */

'use strict'

const native = require('./index.js')

// `U8Buffer` ≡ `ByteBuffer`: expose the io byte store under the buffer namespace too.
native.buffer.U8Buffer = native.io.ByteBuffer

module.exports = native
