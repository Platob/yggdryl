'use strict'

// The Apache Arrow JS half of the record boundary.
//
// Arrow JS has no C Data consumer, so a batch crosses as Arrow IPC in both
// directions: the native reader hands over one self-contained stream per batch,
// and a write is handed one stream. This module owns that translation and the
// argument coercion around it. Every schema decision, projection, and cast
// stays native; nothing here reads a datatype.
//
// Write intent and representation are both explicit. Each of ArrowReader,
// ArrowTable, ArrowBatch, and Records has overwrite/append/merge entry
// points. The representation-specific adapter widens to one native reader and
// the intent-specific call redirects to the matching Rust primitive.

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

function byteView(value) {
  if (value === null || value === undefined) return value
  if (Buffer.isBuffer(value) || value instanceof Uint8Array) return value
  if (value instanceof ArrayBuffer) return new Uint8Array(value)
  if (ArrayBuffer.isView(value)) {
    return new Uint8Array(value.buffer, value.byteOffset, value.byteLength)
  }
  return new Uint8Array(value)
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
  RecordOptions,
  Table,
  Tables,
}) {
  const classFields = new WeakMap()
  const nextIpc = BatchReader.prototype._nextIpcNative
  if (typeof nextIpc !== 'function') {
    throw new TypeError('native binding is missing BatchReader._nextIpcNative')
  }
  delete BatchReader.prototype._nextIpcNative
  const chainIpcPull = BatchReader.prototype._chainIpcPullNative
  if (typeof chainIpcPull !== 'function') {
    throw new TypeError('native binding is missing BatchReader._chainIpcPullNative')
  }
  delete BatchReader.prototype._chainIpcPullNative
  const emptyFromField = Field.prototype._emptyArrowReaderNative
  if (typeof emptyFromField !== 'function') {
    throw new TypeError('native binding is missing Field._emptyArrowReaderNative')
  }
  delete Field.prototype._emptyArrowReaderNative
  const requireWritePreflight = RecordOptions.prototype._requireWritePreflightNative
  if (typeof requireWritePreflight !== 'function') {
    throw new TypeError('native binding is missing RecordOptions._requireWritePreflightNative')
  }
  delete RecordOptions.prototype._requireWritePreflightNative
  const beginWriteSession = IOBase.prototype._beginArrowWriteSessionNative
  if (typeof beginWriteSession !== 'function') {
    throw new TypeError(
      'native binding is missing IOBase._beginArrowWriteSessionNative',
    )
  }
  delete IOBase.prototype._beginArrowWriteSessionNative
  const pushWriteSession = IOBase.prototype._pushArrowWriteSessionNative
  const finishWriteSession = IOBase.prototype._finishArrowWriteSessionNative
  const abortWriteSession = IOBase.prototype._abortArrowWriteSessionNative
  for (const [name, method] of [
    ['_pushArrowWriteSessionNative', pushWriteSession],
    ['_finishArrowWriteSessionNative', finishWriteSession],
    ['_abortArrowWriteSessionNative', abortWriteSession],
  ]) {
    if (typeof method !== 'function') {
      throw new TypeError(`native binding is missing IOBase.${name}`)
    }
    delete IOBase.prototype[name]
  }

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
  // encoded by Arrow JS itself. This is the explicit `BatchReader.from`
  // conversion contract; write methods do not silently accept a different
  // representation than their names declare.
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

  function nativeArrowReader(source) {
    if (source instanceof BatchReader) return source
    throw new TypeError(
      'reader must be a native BatchReader; use BatchReader.from(value) to convert another Arrow representation',
    )
  }

  // The representation-specific paths deliberately skip the generic source
  // classifier. Arrow JS has no C Data consumer, so each already-materialized
  // holder is encoded once into the native streaming reader boundary.
  function arrowTableReader(source, rootName) {
    if (arrowKind(source) !== 'Table') {
      throw new TypeError('table must be an Apache Arrow JS Table')
    }
    return BatchReader.fromIpc(arrow().tableToIPC(source), rootName)
  }

  function arrowRecordBatchReader(source, rootName) {
    if (arrowKind(source) !== 'RecordBatch') {
      throw new TypeError('batch must be an Apache Arrow JS RecordBatch')
    }
    const runtime = arrow()
    return BatchReader.fromIpc(
      runtime.tableToIPC(new runtime.Table(source)),
      rootName,
    )
  }

  function isStructRecord(value) {
    if (isPlainRecord(value)) return true
    if (value === null || typeof value !== 'object') return false
    const owner = value.constructor
    return (
      typeof owner === 'function' &&
      'intoStructField' in owner
    )
  }

  function plainStructRecord(value) {
    if (!isStructRecord(value)) {
      throw new TypeError(
        'records must be plain JavaScript objects or instances whose class exposes a static intoStructField getter',
      )
    }
    return {
      field: isPlainRecord(value) ? undefined : intoField(value),
      record: Object.fromEntries(Object.entries(value)),
    }
  }

  // One record stream becomes bounded Arrow IPC chunks. The first chunk fixes
  // the Arrow JS physical schema; every later chunk builds vectors under those
  // exact types so one native BatchReader remains a valid stream. The native
  // pull bridge asks for a chunk only as the core drains it, preserving one
  // logical write and one publication without holding the incoming iterable.
  function recordChunker(settings, defaultBatchRowSize) {
    const rowSize = settings.batchRowSize ?? defaultBatchRowSize
    const cadence = settings.commitRowSize
    let rowsToCommit = cadence
    let remainingRows = settings.maxRowSize
    let arrowSchema
    let inferred
    let recordKind

    function convert(value) {
      const item = plainStructRecord(value)
      const kind = item.field === undefined ? 'plain' : 'field-class'
      if (recordKind === undefined) {
        recordKind = kind
      } else if (recordKind !== kind) {
        throw new TypeError(
          'one record write cannot mix plain objects with field-class instances',
        )
      }
      if (item.field !== undefined) {
        if (inferred === undefined) {
          inferred = item.field
        } else if (inferred !== item.field && !inferred.equals(item.field)) {
          throw new TypeError(
            'record instances in one write must expose the same intoStructField getter',
          )
        }
      }
      return item.record
    }

    function encode(records) {
      const runtime = arrow()
      let table
      if (arrowSchema === undefined) {
        table = runtime.tableFromJSON(records)
        const columns = Object.create(null)
        let replaced = false
        for (const field of table.schema.fields) {
          const values = records.map((record) => record[field.name])
          const present = values.filter((value) => value !== null && value !== undefined)
          if (present.length > 0 && present.every(isBytes)) {
            columns[field.name] = runtime.vectorFromArray(
              values.map(byteView),
              new runtime.Binary(),
            )
            replaced = true
          } else {
            columns[field.name] = table.getChild(field.name)
          }
        }
        if (replaced) table = new runtime.Table(columns)
        arrowSchema = table.schema
      } else {
        const columns = Object.create(null)
        for (const field of arrowSchema.fields) {
          columns[field.name] = runtime.vectorFromArray(
            records.map((record) => record[field.name]),
            field.type,
          )
        }
        table = new runtime.Table(columns)
      }
      return runtime.tableToIPC(table)
    }

    function nextRowSize() {
      let size = rowSize
      if (rowsToCommit !== null) size = Math.min(size, rowsToCommit)
      if (remainingRows !== null) size = Math.min(size, remainingRows)
      return size
    }

    function accepted(rows) {
      if (remainingRows !== null) remainingRows -= rows
      if (rowsToCommit !== null) {
        rowsToCommit -= rows
        if (rowsToCommit === 0) rowsToCommit = cadence
      }
    }

    function sync(iterator) {
      const size = nextRowSize()
      if (size === 0) return undefined
      const records = []
      while (records.length < size) {
        const item = iterator.next()
        if (item.done) break
        records.push(convert(item.value))
      }
      if (records.length === 0) return undefined
      const bytes = encode(records)
      accepted(records.length)
      return bytes
    }

    async function asynchronous(iterator) {
      const size = nextRowSize()
      if (size === 0) return undefined
      const records = []
      while (records.length < size) {
        const item = await iterator.next()
        if (item.done) break
        records.push(convert(item.value))
      }
      if (records.length === 0) return undefined
      const bytes = encode(records)
      accepted(records.length)
      return bytes
    }

    return {
      async: asynchronous,
      inferred: () => inferred,
      sync,
    }
  }

  function emptyRecordsReader(settings) {
    if (settings.field === null) {
      throw new TypeError('an empty record sequence requires options.field')
    }
    return {
      reader: Reflect.apply(emptyFromField, settings.field, []),
      settings,
    }
  }

  function syncRecordIterator(source) {
    if (isStructRecord(source)) return [source][Symbol.iterator]()
    if (
      source !== null &&
      source !== undefined &&
      typeof source[Symbol.iterator] === 'function'
    ) {
      return source[Symbol.iterator]()
    }
    throw new TypeError(
      'records must be a JavaScript struct or an iterable of JavaScript structs',
    )
  }

  function recordsReader(source, settings, defaultBatchRowSize) {
    const chunks = recordChunker(settings, defaultBatchRowSize)
    const iterator = syncRecordIterator(source)
    const first = chunks.sync(iterator)
    if (first === undefined) return emptyRecordsReader(settings)

    const inferred = chunks.inferred()
    let reader = BatchReader.fromIpc(first, settings.name)
    // An explicit field wins. Otherwise a field class supplies its cached root;
    // plain objects take the native field inferred from the bounded first
    // chunk. The core remains the one place that applies the declared cast.
    if (settings.field === null) {
      settings = settings.withField(inferred ?? reader.field)
    }
    reader = Reflect.apply(chainIpcPull, reader, [() => chunks.sync(iterator)])
    return { reader, settings }
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

  // An async iterator cannot be pulled from a synchronous core call. Its
  // bounded IPC chunks are therefore spooled to one private temporary file,
  // then replayed through the same native pull reader. RAM stays bounded and
  // the resource still sees one core write/publication; cleanup runs on every
  // success and failure path.
  async function awaitedRecordsReader(source, settings, defaultBatchRowSize) {
    const iterator = source[Symbol.asyncIterator]()
    const chunks = recordChunker(settings, defaultBatchRowSize)
    const first = await chunks.async(iterator)
    if (first === undefined) return emptyRecordsReader(settings)

    if (settings.field === null) {
      const inferred = BatchReader.fromIpc(first, settings.name)
      settings = settings.withField(chunks.inferred() ?? inferred.field)
    }

    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-records-'))
    const location = path.join(directory, 'chunks.ipc')
    let descriptor
    let writePosition = 0
    let readPosition = 0

    function close() {
      if (descriptor !== undefined) {
        fs.closeSync(descriptor)
        descriptor = undefined
      }
      fs.rmSync(location, { force: true })
      fs.rmdirSync(directory)
    }

    function write(buffer) {
      if (writePosition + buffer.length > Number.MAX_SAFE_INTEGER) {
        throw new RangeError('the private record spool exceeds JavaScript safe file offsets')
      }
      let offset = 0
      while (offset < buffer.length) {
        const size = fs.writeSync(
          descriptor,
          buffer,
          offset,
          buffer.length - offset,
          writePosition + offset,
        )
        if (size === 0) {
          throw new Error('the private record spool accepted no bytes')
        }
        offset += size
      }
      writePosition += buffer.length
    }

    try {
      descriptor = fs.openSync(location, 'wx+')
      for (;;) {
        const bytes = await chunks.async(iterator)
        if (bytes === undefined) break
        const header = Buffer.allocUnsafe(8)
        header.writeBigUInt64LE(BigInt(bytes.byteLength))
        write(header)
        write(Buffer.from(bytes.buffer, bytes.byteOffset, bytes.byteLength))
      }

      function read(buffer) {
        let offset = 0
        while (offset < buffer.length) {
          const size = fs.readSync(
            descriptor,
            buffer,
            offset,
            buffer.length - offset,
            readPosition + offset,
          )
          if (size === 0) {
            throw new Error('the private record spool ended inside an IPC chunk')
          }
          offset += size
        }
        readPosition += buffer.length
      }

      const pull = () => {
        if (readPosition === writePosition) return undefined
        const header = Buffer.allocUnsafe(8)
        read(header)
        const length = Number(header.readBigUInt64LE())
        if (!Number.isSafeInteger(length)) {
          throw new RangeError('a spooled Arrow IPC chunk exceeds JavaScript safe length')
        }
        const bytes = Buffer.allocUnsafe(length)
        read(bytes)
        return bytes
      }
      let reader = BatchReader.fromIpc(first, settings.name)
      reader = Reflect.apply(chainIpcPull, reader, [pull])
      return { close, reader, settings }
    } catch (error) {
      close()
      throw error
    }
  }

  // A bounded async source alternates exactly one await with one synchronous
  // core push. The Rust session retains the operation-wide cast, byte/row
  // limits, cadence remainder, and destination routing plan, so a later
  // source/conversion failure leaves every earlier complete prefix visible.
  async function awaitedCommittedRecordsWrite(
    handle,
    source,
    settings,
    defaultBatchRowSize,
    intent,
    publish,
  ) {
    const iterator = source[Symbol.asyncIterator]()
    const chunks = recordChunker(settings, defaultBatchRowSize)
    let session
    let finished = false
    try {
      for (;;) {
        const bytes = await chunks.async(iterator)
        if (bytes === undefined) break
        const reader = BatchReader.fromIpc(bytes, settings.name)
        if (settings.field === null) {
          settings = settings.withField(chunks.inferred() ?? reader.field)
        }
        if (session === undefined) {
          session = Reflect.apply(beginWriteSession, handle, [intent, settings])
        }
        const more = Reflect.apply(pushWriteSession, handle, [session, reader])
        if (!more) {
          Reflect.apply(finishWriteSession, handle, [session])
          finished = true
          if (typeof iterator.return === 'function') await iterator.return()
          return
        }
      }

      if (session === undefined) {
        const converted = emptyRecordsReader(settings)
        return publish(converted.reader, converted.settings)
      }
      Reflect.apply(finishWriteSession, handle, [session])
      finished = true
    } catch (error) {
      if (session !== undefined && !finished) {
        try {
          Reflect.apply(abortWriteSession, handle, [session])
        } catch {
          // Preserve the source, conversion, or publication error that caused
          // the abort; completed cadences are already visible.
        }
      }
      if (typeof iterator.return === 'function') {
        try {
          await iterator.return()
        } catch {
          // As above, cleanup cannot mask the operation's original failure.
        }
      }
      throw error
    }
  }

  function recordOptions(options) {
    if (options === undefined || options === null) return options
    if (options instanceof RecordOptions) return options
    return RecordOptions.from(options)
  }

  // Every write crosses with one concrete options value. This resolves the
  // handle's encoding at the JavaScript boundary, where representation
  // inference can attach a Field without mutating caller-owned options.
  function resolvedRecordOptions(handle, options) {
    return recordOptions(options) ?? handle.recordOptions()
  }

  function inferredRecordOptions(settings, reader) {
    return settings.field === null ? settings.withField(reader.field) : settings
  }

  function preflightWriteIntent(settings, intent) {
    return Reflect.apply(requireWritePreflight, settings, [intent])
  }

  function writeMode(mode) {
    if (typeof mode !== 'string') {
      throw new TypeError('mode must be overwrite, append, or merge')
    }
    const canonical = mode.trim().toLowerCase()
    if (!['overwrite', 'append', 'merge'].includes(canonical)) {
      throw new TypeError(`unknown write mode ${JSON.stringify(mode)}`)
    }
    return canonical
  }

  function writeLimitIsZero(settings) {
    return settings.maxRowSize === 0 || settings.maxByteSize === 0
  }

  // Metadata must be an accessor, not a stored value or a method. Looking up
  // the descriptor before reading the property prevents a method/static Field
  // path from silently reappearing. An inherited getter remains
  // a getter; its result is still memoized for the concrete owner below.
  function structFieldGetter(owner) {
    for (
      let current = owner;
      typeof current === 'function';
      current = Object.getPrototypeOf(current)
    ) {
      const descriptor = Object.getOwnPropertyDescriptor(
        current,
        'intoStructField',
      )
      if (descriptor === undefined) continue
      if (typeof descriptor.get !== 'function') {
        throw new TypeError(
          'intoStructField must be a static getter returning a native Field',
        )
      }
      return descriptor.get
    }
    return undefined
  }

  function intoField(value, name) {
    if (name !== undefined && name !== null && typeof name !== 'string') {
      throw new TypeError('name must be a string, null, or undefined')
    }
    if (value === undefined || value === null) {
      throw new TypeError(
        'value must be a Field, field expression, or class with a static intoStructField getter',
      )
    }

    let converted
    if (value instanceof Field) {
      converted = value
    } else {
      const owner =
        typeof value === 'function'
          ? value
          : typeof value === 'object'
            ? value.constructor
            : undefined
      converted = owner === undefined ? undefined : classFields.get(owner)
      if (converted === undefined && owner !== undefined) {
        const getter = structFieldGetter(owner)
        if (getter !== undefined) {
          converted = Reflect.apply(getter, owner, [])
          if (
            !(converted instanceof Field) ||
            converted.dtype.kind !== 'struct' ||
            converted.nullable
          ) {
            throw new TypeError(
              'intoStructField must return a non-null native struct Field',
            )
          }
          classFields.set(owner, converted)
        }
      }
      if (converted === undefined) {
        if (typeof value === 'string') {
          converted = Field.from(value)
        } else {
          throw new TypeError(
            'value must be a Field, field expression, or class with a static intoStructField getter',
          )
        }
      }
    }

    if (name === undefined || name === null || name === converted.name) {
      return converted
    }
    const renamed = new Field(converted)
    renamed.setName(name)
    return renamed
  }

  // Native projection arguments are optional even though the public converter
  // is not. Keep that distinction here so `intoField(null)` has the same error
  // contract as Python while a scan that omits its projection still passes
  // `null` through to the core.
  function optionalField(value) {
    return value === undefined || value === null ? value : intoField(value)
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
    intoTable: {
      configurable: true,
      value() {
        return arrow().tableFromIPC(this.intoIpc())
      },
    },
  })

  Object.defineProperty(BatchReader, 'from', {
    configurable: true,
    value(source, name) {
      return batchReader(source, name)
    },
  })

  const intents = ['overwrite', 'append', 'merge']
  const nativeWrite = IOBase.prototype.writeArrowReader
  if (typeof nativeWrite !== 'function') {
    throw new TypeError('native binding is missing IOBase.writeArrowReader')
  }
  const nativeWrites = Object.fromEntries(
    intents.map((intent) => {
      const name = `${intent}ArrowReader`
      const native = IOBase.prototype[name]
      if (typeof native !== 'function') {
        throw new TypeError(`native binding is missing IOBase.${name}`)
      }
      return [intent, native]
    }),
  )

  // Intent and representation stay visible in every method name. Arrow JS has
  // no C Data consumer, so Table and RecordBatch take one IPC bridge into a
  // native BatchReader; the reader method itself accepts only that native
  // stream. Each path infers a Field at the boundary when none was declared,
  // then redirects to the matching Rust primitive.
  const representations = [
    ['ArrowReader', nativeArrowReader],
    ['ArrowTable', arrowTableReader],
    ['ArrowBatch', arrowRecordBatchReader],
  ]
  for (const intent of intents) {
    const native = nativeWrites[intent]
    for (const [suffix, convert] of representations) {
      const name = `${intent}${suffix}`
      Object.defineProperty(IOBase.prototype, name, {
        configurable: true,
        value(source, options) {
          let settings = resolvedRecordOptions(this, options)
          preflightWriteIntent(settings, intent)
          if (writeLimitIsZero(settings)) {
            if (intent === 'append') return undefined
            const converted = emptyRecordsReader(settings)
            return native.call(this, converted.reader, converted.settings)
          }
          const reader = convert(source, settings.name)
          settings = inferredRecordOptions(settings, reader)
          return native.call(this, reader, settings)
        },
      })
    }
  }

  // Generic entry points keep the same representation-specific conversion,
  // then pass the required mode into the core's one dispatcher. Input, mode,
  // options is the canonical order in every language.
  for (const [suffix, convert] of representations) {
    Object.defineProperty(IOBase.prototype, `write${suffix}`, {
      configurable: true,
      value(source, mode, options) {
        const intent = writeMode(mode)
        let settings = resolvedRecordOptions(this, options)
        preflightWriteIntent(settings, intent)
        if (writeLimitIsZero(settings)) {
          if (intent === 'append') return undefined
          const converted = emptyRecordsReader(settings)
          return nativeWrite.call(
            this,
            converted.reader,
            intent,
            converted.settings,
          )
        }
        const reader = convert(source, settings.name)
        settings = inferredRecordOptions(settings, reader)
        return nativeWrite.call(this, reader, intent, settings)
      },
    })
  }

  const readBatches = IOBase.prototype.readArrowReader
  Object.defineProperty(IOBase.prototype, 'readArrowReader', {
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

  // The generic cast: whatever Arrow JS holds - a Table, a RecordBatch, a
  // BatchReader, IPC bytes - casts to this exact Field batch by batch and
  // comes back a Table. `cast` is the same call under the generic name.
  for (const name of ['castArrow', 'cast']) {
    Object.defineProperty(Field.prototype, name, {
      configurable: true,
      value(rows, options) {
        const safe = options?.safe ?? true
        const bytes = batchReader(rows, this.name).intoIpc()
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
      if (typeof cls !== 'function') {
        if (options !== undefined) {
          throw new TypeError(
            'readRecords accepts one options value, or a record class followed by one options value',
          )
        }
        options = cls
        cls = undefined
      }
      let settings = recordOptions(options)
      if (
        cls !== undefined &&
        'intoStructField' in cls &&
        (settings === undefined || settings === null || settings.field === null)
      ) {
        settings = (settings ?? this.recordOptions()).withField(intoField(cls))
      }
      const reader = this.readArrowReader(settings)
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

  // Plain objects and field-class instances are inferred by a bounded first
  // chunk, then streamed through the chosen core primitive. Async records
  // return a Promise; synchronous records stay lazy.
  function writeRecordSource(handle, rows, options, intent, publish) {
    const settings = resolvedRecordOptions(handle, options)
    const defaultBatchRowSize = preflightWriteIntent(settings, intent)
    if (writeLimitIsZero(settings)) {
      if (intent === 'append') return undefined
      // A limited merge was rejected by preflight. Overwrite still publishes
      // the explicitly typed empty value without inspecting the input.
      const converted = emptyRecordsReader(settings)
      return publish(converted.reader, converted.settings)
    }
    const asynchronous = needsAwait(rows)
    if (asynchronous && settings.commitRowSize !== null) {
      return awaitedCommittedRecordsWrite(
        handle,
        rows,
        settings,
        defaultBatchRowSize,
        intent,
        publish,
      )
    }
    if (asynchronous) {
      return awaitedRecordsReader(rows, settings, defaultBatchRowSize).then((converted) => {
        try {
          return publish(converted.reader, converted.settings)
        } finally {
          converted.close?.()
        }
      })
    }
    const converted = recordsReader(rows, settings, defaultBatchRowSize)
    return publish(converted.reader, converted.settings)
  }

  for (const intent of intents) {
    const native = nativeWrites[intent]
    Object.defineProperty(IOBase.prototype, `${intent}Records`, {
      configurable: true,
      value(rows, options) {
        return writeRecordSource(
          this,
          rows,
          options,
          intent,
          (reader, settings) => native.call(this, reader, settings),
        )
      },
    })
  }

  Object.defineProperty(IOBase.prototype, 'writeRecords', {
    configurable: true,
    value(rows, mode, options) {
      const intent = writeMode(mode)
      return writeRecordSource(
        this,
        rows,
        options,
        intent,
        (reader, settings) =>
          nativeWrite.call(this, reader, intent, settings),
      )
    },
  })


  // An Iceberg write takes what every other write here takes. Anything
  // Arrow-shaped is the reader it already names; everything else is rows, and
  // those are typed against the table's stored schema so a plain object does
  // not have to guess one. A table that does not exist yet names no schema,
  // and the rows are then what declare it.
  const ICEBERG_DATA_MIME_TYPE = 'application/vnd.apache.parquet'

  function isArrowShaped(source) {
    if (source instanceof BatchReader || isBytes(source)) return true
    if (arrowKind(source) !== null) return true
    return Array.isArray(source) && source.length > 0 && arrowKind(source[0]) !== null
  }

  function icebergBatchReader(table, source) {
    if (source === undefined || source === null || isArrowShaped(source)) {
      return batchReader(source)
    }
    let settings = new RecordOptions(ICEBERG_DATA_MIME_TYPE)
    const stored = table == null ? null : table.schema
    if (stored != null) settings = settings.withField(stored)
    const converted = recordsReader(
      source,
      settings,
      preflightWriteIntent(settings, 'append'),
    )
    if (stored != null) return converted.reader
    // Nothing declared a schema, so the rows named one - and the rows were
    // encoded by Arrow JS, which dictionary-encodes a string, a datatype
    // Iceberg does not express. The comparison is against the reader's own
    // field rather than the declared one: a field class declares `utf8` and
    // still arrives as `dictionary(int32, utf8)`, so comparing declarations
    // would skip the cast that is exactly what the create needs. Casting
    // materializes, which these rows already were.
    const declared = converted.settings.field
    if (declared === null || declared === undefined) return converted.reader
    const widened = declared.intoSchemeCompat('iceberg')
    if (widened.equals(converted.reader.field)) return converted.reader
    return batchReader(widened.castArrow(converted.reader), widened.name)
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
        return native.call(this, icebergBatchReader(this, batches), options)
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
        return overwriteWhere.call(this, filters, icebergBatchReader(this, batches), options)
      },
    })
  }

  const merge = Table.prototype.merge
  if (merge) {
    Object.defineProperty(Table.prototype, 'merge', {
      configurable: true,
      value(batches, mergeByNames, safe, options) {
        return merge.call(this, icebergBatchReader(this, batches), mergeByNames, safe, options)
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
          icebergBatchReader(this, batches),
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
        if (args.length > at) args[at] = optionalField(args[at])
        return native.apply(this, args)
      },
    })
  }

  const scanAt = Table.prototype.scanAt
  Object.defineProperty(Table.prototype, 'scanAt', {
    configurable: true,
    value(snapshotId, filters, field, options) {
      return scanAt.call(this, snapshotId, filters, optionalField(field), options)
    },
  })

  const evolveSchema = Table.prototype.evolveSchema
  Object.defineProperty(Table.prototype, 'evolveSchema', {
    configurable: true,
    value(schema) {
      return evolveSchema.call(this, intoField(schema))
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
          // An existing table declares the schema its rows are typed against;
          // a create-on-write names none yet, and the rows declare it.
          const stored = this.has(table) ? this.get(table) : null
          return native.call(this, table, icebergBatchReader(stored, batches), options)
        },
      })
    }
    for (const name of ['create', 'openOrCreate']) {
      const native = Tables.prototype[name]
      if (!native) continue
      Object.defineProperty(Tables.prototype, name, {
        configurable: true,
        value(table, schema) {
          // An array of child Fields is a shape the native call assembles
          // itself, under a root named `row`; only scalar spellings coerce.
          return native.call(
            this,
            table,
            Array.isArray(schema) ? schema : intoField(schema),
          )
        },
      })
    }
  }

  return Object.freeze({ icebergBatchReader, intoField })
}

module.exports = { installRecords }
