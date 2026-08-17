'use strict'

// Apache Arrow JS materialization for values the native side hands back as
// IPC. This module owns no schema behavior: it turns bytes into an Arrow
// value and nothing else.

let arrowRuntime

function arrow() {
  // Keep startup cheap. Apache Arrow is loaded only when a caller actually
  // asks for an Arrow value, and its absence is reported as the missing
  // package it is rather than as whatever the failing `require` said.
  if (arrowRuntime === undefined) {
    try {
      arrowRuntime = require('apache-arrow')
    } catch (cause) {
      throw new Error(
        'materializing Arrow values needs apache-arrow installed, and loading it failed',
        { cause },
      )
    }
  }
  return arrowRuntime
}

function ipcBytes(value, label = 'Arrow IPC input') {
  if (Buffer.isBuffer(value)) return value
  if (value instanceof ArrayBuffer) return Buffer.from(value)
  if (
    typeof SharedArrayBuffer !== 'undefined' &&
    value instanceof SharedArrayBuffer
  ) {
    return Buffer.from(value)
  }
  if (ArrayBuffer.isView(value)) {
    return Buffer.from(value.buffer, value.byteOffset, value.byteLength)
  }
  throw new TypeError(
    `${label} must be a Buffer, ArrayBuffer, or ArrayBuffer view`,
  )
}

function arrowScalarFromIPC(input, label) {
  const runtime = arrow()
  let table
  try {
    table = runtime.tableFromIPC(ipcBytes(input, label))
  } catch (cause) {
    throw new TypeError(
      `Apache Arrow JS cannot materialize ${label}; the physical layout is unsupported`,
      { cause },
    )
  }
  if (table.numCols !== 1 || table.numRows !== 1) {
    throw new TypeError(
      `${label} IPC must contain exactly one column and one row`,
    )
  }
  const vector = table.getChildAt(0)
  if (vector === null) throw new TypeError(`${label} IPC has no scalar column`)
  return vector.get(0)
}

module.exports = { arrow, arrowScalarFromIPC, ipcBytes }
