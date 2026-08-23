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

function arrowVectorFromIPC(input, label = 'Arrow array') {
  const table = arrowTableFromIPC(input, label)
  if (table.numCols !== 1) {
    throw new TypeError(`${label} IPC must contain exactly one column`)
  }
  const vector = table.getChildAt(0)
  if (vector === null) throw new TypeError(`${label} IPC has no value column`)
  return vector
}

function arrowBatchFromIPC(input, label = 'Arrow record batch') {
  const table = arrowTableFromIPC(input, label)
  if (table.batches.length !== 1) {
    throw new TypeError(
      `${label} IPC must contain exactly one record batch, got ${table.batches.length}`,
    )
  }
  return table.batches[0]
}

function arrowTableFromIPC(input, label = 'Arrow table') {
  try {
    return arrow().tableFromIPC(ipcBytes(input, label))
  } catch (cause) {
    throw new TypeError(`Apache Arrow JS cannot materialize ${label}`, { cause })
  }
}

function arrowScalarIntoIPC(value) {
  const runtime = arrow()
  if (!runtime.isArrowVector(value) || value.length !== 1) {
    throw new TypeError('Value.fromArrowScalar expects a one-item Arrow Vector')
  }
  return arrowVectorIntoIPC(value, 'Value.fromArrowScalar input')
}

function arrowVectorIntoIPC(value, label = 'Arrow array input') {
  const runtime = arrow()
  if (!runtime.isArrowVector(value)) {
    throw new TypeError(`${label} must be an Apache Arrow Vector`)
  }
  return Buffer.from(runtime.tableToIPC(runtime.makeTable({ value }), 'stream'))
}

function arrowBatchIntoIPC(value, label = 'Arrow record batch input') {
  const runtime = arrow()
  if (!runtime.isArrowRecordBatch(value)) {
    throw new TypeError(`${label} must be an Apache Arrow RecordBatch`)
  }
  return Buffer.from(runtime.tableToIPC(new runtime.Table(value), 'stream'))
}

function arrowTableIntoIPC(value, label = 'Arrow table input') {
  const runtime = arrow()
  if (!runtime.isArrowTable(value)) {
    throw new TypeError(`${label} must be an Apache Arrow Table`)
  }
  return Buffer.from(runtime.tableToIPC(value, 'stream'))
}

module.exports = {
  arrow,
  arrowBatchFromIPC,
  arrowBatchIntoIPC,
  arrowScalarFromIPC,
  arrowScalarIntoIPC,
  arrowTableFromIPC,
  arrowTableIntoIPC,
  arrowVectorFromIPC,
  arrowVectorIntoIPC,
  ipcBytes,
}
