'use strict'

// The Apache Arrow JS half of the record boundary.
//
// Arrow JS has no C Data consumer, so a batch crosses as Arrow IPC in both
// directions: the native reader hands over one self-contained stream per batch,
// and a write is handed one stream. This module owns that translation and the
// argument coercion around it. Every schema decision, projection, and cast
// stays native; nothing here reads a datatype.
//
// `readArrow`, `writeArrow`, and `appendArrow` are the same three calls with
// the argument widened to whatever a JavaScript caller is holding: an Arrow JS
// `Table`, `RecordBatch`, or `RecordBatchReader`, a `Vector`, an object of
// named columns, an array or iterable of any of those, plain records, or an
// async iterable. Each one becomes the single native reader and is handed to
// the same native method, so widening the argument never adds a second way to
// write.

const { arrow, ipcBytes } = require('./values.js')

function isBytes(value) {
  return (
    Buffer.isBuffer(value) ||
    value instanceof ArrayBuffer ||
    ArrayBuffer.isView(value) ||
    (typeof SharedArrayBuffer !== 'undefined' &&
      value instanceof SharedArrayBuffer)
  )
}

function isPlainRecord(value) {
  if (value === null || typeof value !== 'object') return false
  const prototype = Object.getPrototypeOf(value)
  return prototype === Object.prototype || prototype === null
}

// A class from another copy of apache-arrow fails `instanceof` against the copy
// this package loaded, so the constructor chain is walked by name as well. The
// name alone would accept any class that happens to be called `Table`, so each
// name is paired with the shape it promises.
const ARROW_SHAPES = Object.freeze({
  Table: (value) => Array.isArray(value.batches) && value.schema !== undefined,
  RecordBatch: (value) =>
    value.schema !== undefined && typeof value.numRows === 'number',
  RecordBatchReader: (value) => typeof value.readAll === 'function',
  Vector: (value) => typeof value.length === 'number' && value.type !== undefined,
})

function arrowKind(value) {
  if (value === null || typeof value !== 'object') return null
  const runtime = arrow()
  if (value instanceof runtime.Table) return 'Table'
  if (value instanceof runtime.RecordBatch) return 'RecordBatch'
  if (value instanceof runtime.RecordBatchReader) return 'RecordBatchReader'
  if (value instanceof runtime.Vector) return 'Vector'
  for (
    let type = value.constructor;
    typeof type === 'function';
    type = Object.getPrototypeOf(type)
  ) {
    const shape = ARROW_SHAPES[type.name]
    if (shape !== undefined && shape(value)) return type.name
  }
  return null
}

function installRecords({
  BatchReader,
  Field,
  IOBase,
  Namespace,
  RecordOptions,
  Table,
  Tables,
}) {
  const nextIpc = BatchReader.prototype._nextIpcNative
  if (typeof nextIpc !== 'function') {
    throw new TypeError('native binding is missing BatchReader._nextIpcNative')
  }
  delete BatchReader.prototype._nextIpcNative

  // One batch arrives as its own IPC stream, so its schema travels with it and
  // Arrow JS needs no separate handshake. That per-batch header is what a
  // copied boundary costs, and it is stated rather than hidden.
  function recordBatchFromIPC(bytes) {
    const runtime = arrow()
    const table = runtime.tableFromIPC(bytes)
    const [batch] = table.batches
    return batch ?? new runtime.RecordBatch(table.schema)
  }

  function arrowTable(source) {
    const runtime = arrow()
    if (source instanceof runtime.Table) return source
    if (source instanceof runtime.RecordBatch) return new runtime.Table(source)
    if (Array.isArray(source)) {
      if (source.length === 0) {
        throw new TypeError(
          'an empty array names no schema; build a BatchReader from an Arrow Table instead',
        )
      }
      return new runtime.Table(source)
    }
    return null
  }

  // Whatever a caller already holds becomes the one native reader shape: a
  // reader passes through, bytes already are a stream, and an Arrow JS value is
  // encoded by Arrow JS itself. This is the reader contract `BatchReader.from`
  // and the two `*ArrowBatchReader` writes publish; `rowsReader` below is the
  // wider one the generic entry points infer with.
  function batchReader(source, rootName) {
    if (source instanceof BatchReader) return source
    if (isBytes(source)) {
      return BatchReader.fromIpc(ipcBytes(source, 'Arrow IPC batches'), rootName)
    }
    const table = source === undefined || source === null ? null : arrowTable(source)
    if (table === null) {
      throw new TypeError(
        'batches must be a BatchReader, an Apache Arrow JS Table or RecordBatch, or Arrow IPC bytes',
      )
    }
    return BatchReader.fromIpc(arrow().tableToIPC(table), rootName)
  }

  // The batches one source contributes, appended to `into` so a sequence of
  // sources becomes one table rather than one table each. `column` is the name
  // a bare `Vector` fills, which only a one-column declared schema can supply.
  function collectBatches(source, into, column) {
    const runtime = arrow()
    switch (arrowKind(source)) {
      case 'Table':
        into.push(...source.batches)
        return
      case 'RecordBatch':
        into.push(source)
        return
      case 'RecordBatchReader':
        into.push(...source.readAll())
        return
      case 'Vector': {
        if (column === undefined) {
          throw new TypeError(
            'a bare Vector names no column; pass an object of named columns, or declare a one-column schema on the options',
          )
        }
        into.push(...new runtime.Table({ [column]: source }).batches)
        return
      }
      default:
        break
    }
    if (source instanceof BatchReader) {
      into.push(...runtime.tableFromIPC(source.toIpc()).batches)
      return
    }
    if (isBytes(source)) {
      into.push(...runtime.tableFromIPC(ipcBytes(source, 'Arrow IPC batches')).batches)
      return
    }
    if (isPlainRecord(source)) {
      // An object is either one row or a set of named columns, and a column is
      // the only one of the two whose values are sequences of their own.
      const columns = Object.values(source)
      const columnar =
        columns.length !== 0 &&
        columns.every(
          (column) =>
            arrowKind(column) === 'Vector' ||
            Array.isArray(column) ||
            ArrayBuffer.isView(column),
        )
      const table = columnar
        ? runtime.tableFromArrays(source)
        : runtime.tableFromJSON([source])
      into.push(...table.batches)
      return
    }
    if (typeof source[Symbol.iterator] === 'function') {
      const items = [...source]
      if (items.length === 0) {
        throw new TypeError(
          'an empty sequence names no schema; write an Arrow Table instead',
        )
      }
      // A sequence of plain records is one table, not one table per row: Arrow
      // JS infers the schema from all of them at once.
      if (items.every(isPlainRecord)) {
        into.push(...runtime.tableFromJSON(items).batches)
        return
      }
      for (const item of items) collectBatches(item, into, column)
      return
    }
    throw new TypeError(
      'rows must be a BatchReader, an Apache Arrow JS Table, RecordBatch, RecordBatchReader or Vector, an object of named columns, plain records, Arrow IPC bytes, or an iterable of those',
    )
  }

  // The one column a bare `Vector` fills, when the declared schema names
  // exactly one. Nothing else in a vector says which column it is.
  function soleColumn(schema) {
    if (schema === undefined || schema === null) return undefined
    const children = schema.dataType
    return children.length === 1 ? children.at(0)?.name : undefined
  }

  // Whatever a caller is holding becomes the one native reader shape. The
  // batches are concatenated into a single Arrow JS table first, because Arrow
  // IPC is what this boundary copies and one stream is one schema.
  function rowsReader(source, settings) {
    const name = rootName(settings?.schema)
    if (source instanceof BatchReader) return source
    if (isBytes(source)) {
      return BatchReader.fromIpc(ipcBytes(source, 'Arrow IPC batches'), name)
    }
    if (source === undefined || source === null) {
      throw new TypeError('rows must be given; got nothing to write')
    }
    const batches = []
    collectBatches(source, batches, soleColumn(settings?.schema))
    const runtime = arrow()
    const table =
      batches.length === 1 ? new runtime.Table(batches[0]) : new runtime.Table(batches)
    return BatchReader.fromIpc(runtime.tableToIPC(table), name)
  }

  // A value that implements both iteration protocols is treated as the
  // synchronous one: an Arrow JS reader implements both, and awaiting a source
  // whose rows are already here would make the call async for no reason.
  function needsAwait(value) {
    return (
      value !== null &&
      typeof value === 'object' &&
      typeof value[Symbol.asyncIterator] === 'function' &&
      typeof value[Symbol.iterator] !== 'function'
    )
  }

  // An async source cannot be drained by a synchronous native call, so the
  // items are awaited first. Nothing is lost by holding them: this boundary
  // already encodes one Arrow IPC stream, so the batches were going to be
  // materialized either way.
  async function awaitedRowsReader(source, settings) {
    const items = []
    for await (const item of source) items.push(item)
    return rowsReader(items, settings)
  }

  function recordOptions(options) {
    if (options === undefined || options === null) return options
    if (options instanceof RecordOptions) return options
    return RecordOptions.from(options)
  }

  function schemaField(field) {
    if (field === undefined || field === null) return field
    if (field instanceof Field) return field
    return Field.from(field)
  }

  function rootName(field) {
    return field === undefined || field === null ? undefined : field.name
  }

  Object.defineProperties(BatchReader.prototype, {
    // Iterating a reader is what consuming a stream means everywhere else.
    [Symbol.iterator]: {
      configurable: true,
      value: function* batches() {
        for (;;) {
          const encoded = Reflect.apply(nextIpc, this, [])
          if (encoded === null) return
          yield recordBatchFromIPC(encoded)
        }
      },
    },
    toTable: {
      configurable: true,
      value() {
        return arrow().tableFromIPC(this.toIpc())
      },
    },
  })

  Object.defineProperty(BatchReader, 'from', {
    configurable: true,
    value(source, name) {
      return batchReader(source, name)
    },
  })

  // Both writes take the batches in the same position, so the coercion is one
  // wrapper applied by name rather than one per method.
  for (const name of ['writeArrowBatchReader', 'appendArrowBatchReader']) {
    const native = IOBase.prototype[name]
    Object.defineProperty(IOBase.prototype, name, {
      configurable: true,
      value(batches, options) {
        const settings = recordOptions(options)
        return native.call(this, batchReader(batches, rootName(settings?.schema)), settings)
      },
    })
  }

  const readBatches = IOBase.prototype.readArrowBatchReader
  Object.defineProperty(IOBase.prototype, 'readArrowBatchReader', {
    configurable: true,
    value(options) {
      return readBatches.call(this, recordOptions(options))
    },
  })

  const readArrowField = IOBase.prototype.readArrowField
  Object.defineProperty(IOBase.prototype, 'readArrowField', {
    configurable: true,
    value(options) {
      return readArrowField.call(this, recordOptions(options))
    },
  })

  // `readArrow` is the short name for the same read: a `BatchReader` is the
  // record shape here, so a generic read has nothing to infer.
  Object.defineProperty(IOBase.prototype, 'readArrow', {
    configurable: true,
    value(options) {
      return this.readArrowBatchReader(options)
    },
  })

  // The two generic writes differ only in which native method they end at, so
  // the inference is installed once and named twice.
  for (const [name, target] of [
    ['writeArrow', 'writeArrowBatchReader'],
    ['appendArrow', 'appendArrowBatchReader'],
  ]) {
    const native = IOBase.prototype[target]
    Object.defineProperty(IOBase.prototype, name, {
      configurable: true,
      value(source, options) {
        const settings = recordOptions(options)
        // An async source makes the call async, because its rows do not exist
        // until they are awaited. A synchronous source stays synchronous.
        if (needsAwait(source)) {
          return awaitedRowsReader(source, settings).then((batches) =>
            native.call(this, batches, settings),
          )
        }
        return native.call(this, rowsReader(source, settings), settings)
      },
    })
  }

  // The generic cast: whatever Arrow JS holds - a Table, a RecordBatch, a
  // BatchReader, IPC bytes - casts to this exact Field batch by batch and
  // comes back a Table. `cast` is the same call under the generic name.
  for (const name of ['castArrow', 'cast']) {
    Object.defineProperty(Field.prototype, name, {
      configurable: true,
      value(rows, options) {
        const safe = options?.safe ?? true
        const bytes = batchReader(rows, this.name).toIpc()
        return arrow().tableFromIPC(this._castArrowIpc(bytes, safe))
      },
    })
  }

  // Rows as records: each stored row as one plain object, or as one instance
  // of the class you pass - `new cls(row)` receives the plain row, so any
  // constructor that takes named fields is a runtime record class. Rows come
  // batch by batch off the same native reader every other read uses, so
  // nothing is collected, and a resource that does not exist yields no rows.
  Object.defineProperty(IOBase.prototype, 'readRecords', {
    configurable: true,
    value(cls, options) {
      if (typeof cls !== 'function' && options === undefined) {
        options = cls
        cls = undefined
      }
      const reader = this.readArrowBatchReader(options)
      return (function* records() {
        for (const batch of reader) {
          for (const row of batch) {
            const record = row.toJSON()
            yield cls ? new cls(record) : record
          }
        }
      })()
    },
  })

  // The record writes are the generic writes under the names the read pairs
  // with: `writeArrow` already widens to plain records and record instances,
  // so these add vocabulary, never a second path.
  for (const [name, target] of [
    ['writeRecords', 'writeArrow'],
    ['appendRecords', 'appendArrow'],
  ]) {
    Object.defineProperty(IOBase.prototype, name, {
      configurable: true,
      value(rows, options) {
        return this[target](rows, options)
      },
    })
  }

  // The setter half of the map-like catalog: a schema opens the table,
  // creating it when absent; anything rows-like replaces the table's rows,
  // creating it from their own schema on first write.
  if (Namespace) {
    Object.defineProperty(Namespace.prototype, 'set', {
      configurable: true,
      value(name, value) {
        if (value instanceof Field || typeof value === 'string') {
          return this.openOrCreateTable(name, Field.from(value))
        }
        return this._setIpc(name, batchReader(value).toIpc())
      },
    })
  }

  // The writes that take rows widen them the way every other write here does,
  // and pass the trailing per-call options through untouched. Forwarding it
  // is not optional bookkeeping: a wrapper that drops the argument leaves a
  // documented option silently doing nothing, which is worse than not having
  // it at all.
  for (const name of ['append', 'overwrite']) {
    const native = Table.prototype[name]
    Object.defineProperty(Table.prototype, name, {
      configurable: true,
      value(batches, options) {
        return native.call(this, batchReader(batches), options)
      },
    })
  }

  // `overwriteWhere`, `merge`, and `mergeWhere` take the filters or the match
  // key first, so each one names where its rows sit rather than sharing one
  // positional rule.
  const overwriteWhere = Table.prototype.overwriteWhere
  if (overwriteWhere) {
    Object.defineProperty(Table.prototype, 'overwriteWhere', {
      configurable: true,
      value(filters, batches, options) {
        return overwriteWhere.call(this, filters, batchReader(batches), options)
      },
    })
  }

  const merge = Table.prototype.merge
  if (merge) {
    Object.defineProperty(Table.prototype, 'merge', {
      configurable: true,
      value(batches, mergeByNames, safe, options) {
        return merge.call(this, batchReader(batches), mergeByNames, safe, options)
      },
    })
  }

  const mergeWhere = Table.prototype.mergeWhere
  if (mergeWhere) {
    Object.defineProperty(Table.prototype, 'mergeWhere', {
      configurable: true,
      value(filters, batches, mergeByNames, safe, options) {
        return mergeWhere.call(
          this,
          filters,
          batchReader(batches),
          mergeByNames,
          safe,
          options,
        )
      },
    })
  }

  for (const name of ['scan', 'scanWhere', 'scanRef']) {
    const native = Table.prototype[name]
    if (!native) continue
    Object.defineProperty(Table.prototype, name, {
      configurable: true,
      // `scan` takes the projection first; the filtered pair takes what it
      // filters on first and the projection after, so the projection is
      // coerced wherever it sits.
      value(...args) {
        const at = name === 'scan' ? 0 : name === 'scanWhere' ? 1 : 2
        if (args.length > at) args[at] = schemaField(args[at])
        return native.apply(this, args)
      },
    })
  }

  const scanAt = Table.prototype.scanAt
  Object.defineProperty(Table.prototype, 'scanAt', {
    configurable: true,
    value(snapshotId, filters, field, options) {
      return scanAt.call(this, snapshotId, filters, schemaField(field), options)
    },
  })

  const evolveSchema = Table.prototype.evolveSchema
  Object.defineProperty(Table.prototype, 'evolveSchema', {
    configurable: true,
    value(schema) {
      return evolveSchema.call(this, schemaField(schema))
    },
  })

  // The tables view's own writes take a name first and then the rows, so they
  // widen the rows the same way the table's do and pass the options along.
  if (Tables) {
    for (const name of ['append', 'overwrite']) {
      const native = Tables.prototype[name]
      if (!native) continue
      Object.defineProperty(Tables.prototype, name, {
        configurable: true,
        value(table, batches, options) {
          return native.call(this, table, batchReader(batches), options)
        },
      })
    }
    for (const name of ['create', 'openOrCreate']) {
      const native = Tables.prototype[name]
      if (!native) continue
      Object.defineProperty(Tables.prototype, name, {
        configurable: true,
        value(table, schema) {
          return native.call(this, table, schemaField(schema))
        },
      })
    }
  }
}

module.exports = { installRecords }
