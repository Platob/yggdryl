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
// the intent-specific call redirects to the matching Rust primitive. The
// Serie verbs are the one generic door: rows in any shape the crate holds
// them - a Serie, a ChunkedSerie, a StreamChunkedSerie - or any columnar value a
// BatchReader is built from, read back as a StreamChunkedSerie.

const { arrow, ipcBytes } = require('./values.js')
const optionProperties = require('./properties.js')

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
  ChunkedSerie,
  Field,
  IcebergOptions,
  IOBase,
  IOResult,
  RecordOptions,
  Serie,
  StreamChunkedSerie,
  StreamSerie,
  KeySerie,
  KeySeries,
  StreamKeySerie,
  WindowSerie,
  TextOptions,
  Table,
  nativeWriteMode,
}) {
  if (typeof IOResult !== 'function') {
    throw new TypeError('native binding is missing IOResult')
  }
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
  if (typeof nativeWriteMode !== 'function') {
    throw new TypeError('native binding is missing RecordOptions._writeModeNative')
  }
  const textRecordOptions = TextOptions.prototype._recordOptionsNative
  if (typeof textRecordOptions !== 'function') {
    throw new TypeError('native binding is missing TextOptions._recordOptionsNative')
  }
  delete TextOptions.prototype._recordOptionsNative
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
  const writeSerieNative = IOBase.prototype._writeSerieNative
  if (typeof writeSerieNative !== 'function') {
    throw new TypeError('native binding is missing IOBase._writeSerieNative')
  }
  delete IOBase.prototype._writeSerieNative

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
  function nativeSourceReader(source) {
    if ([Serie, ChunkedSerie, StreamChunkedSerie, StreamSerie, KeySerie, KeySeries, StreamKeySerie, WindowSerie].some(Owner => source instanceof Owner)) {
      return source.intoChunkedStream().intoArrowReader()
    }
    return null
  }

  function batchReader(source, rootName) {
    const native = nativeSourceReader(source)
    if (native !== null) return native
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
    const native = nativeSourceReader(source)
    if (native !== null) return native
    if (source instanceof BatchReader) return source
    throw new TypeError(
      'reader must be a native BatchReader; use BatchReader.from(value) to convert another Arrow representation',
    )
  }

  // The representation-specific paths deliberately skip the generic source
  // classifier. Arrow JS has no C Data consumer, so each already-materialized
  // holder is encoded once into the native streaming reader boundary.
  function arrowTableReader(source, rootName) {
    const native = nativeSourceReader(source)
    if (native !== null) return native
    if (arrowKind(source) !== 'Table') {
      throw new TypeError('table must be an Apache Arrow JS Table')
    }
    return BatchReader.fromIpc(arrow().tableToIPC(source), rootName)
  }

  function arrowRecordBatchReader(source, rootName) {
    const native = nativeSourceReader(source)
    if (native !== null) return native
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
  // A column whose rows mix `number` and `bigint` is one integer column, as
  // Python's `int` is one type: Arrow JS would refuse it with the engine's own
  // conversion error, naming neither the column nor the value. An integral
  // number is read as the `bigint` beside it; anything else is refused by
  // column and value.
  function unifiedIntegers(records) {
    const kinds = new Map()
    const mixed = new Set()
    for (const record of records) {
      for (const name of Object.keys(record)) {
        const kind = typeof record[name]
        if (kind !== 'number' && kind !== 'bigint') continue
        const before = kinds.get(name)
        if (before === undefined) kinds.set(name, kind)
        else if (before !== kind) mixed.add(name)
      }
    }
    if (mixed.size === 0) return records
    return records.map((record) => {
      let copy
      for (const name of mixed) {
        if (typeof record[name] !== 'number') continue
        copy ??= { ...record }
        copy[name] = integralBigInt(name, record[name])
      }
      return copy ?? record
    })
  }

  function integralBigInt(name, value) {
    if (!Number.isInteger(value)) {
      throw new TypeError(
        `expected an integer beside the bigint values of column ${JSON.stringify(name)}, got ${value}`,
      )
    }
    return BigInt(value)
  }

  // A later chunk builds its vectors under the types the first one fixed, so
  // a `number` meets a 64-bit integer column and a `bigint` a `number` one:
  // each is read as the other where it is exactly that value.
  function conformedIntegers(runtime, field, values) {
    const wide = runtime.DataType.isInt(field.type) && field.type.bitWidth === 64
    const numeric = runtime.DataType.isFloat(field.type) || (runtime.DataType.isInt(field.type) && !wide)
    if (!wide && !numeric) return values
    return values.map((value) => {
      if (wide && typeof value === 'number') return integralBigInt(field.name, value)
      if (numeric && typeof value === 'bigint') {
        const number = Number(value)
        if (!Number.isSafeInteger(number)) {
          throw new TypeError(
            `expected a number column ${JSON.stringify(field.name)} to hold a safe integer, got ${value}n`,
          )
        }
        return number
      }
      return value
    })
  }

  // Each chunk is one batch of `batchRowSize` records; a commit cadence counts
  // these batches whole in the core and never cuts one.
  function recordChunker(settings, defaultBatchRowSize) {
    const rowSize = settings.batchRowSize ?? defaultBatchRowSize
    // The rows the limit seam keeps are the ones after its skip, so
    // conversion stops once both are covered.
    let remainingRows =
      settings.maxRowSize == null ? settings.maxRowSize : settings.maxRowSize + (settings.rowOffset ?? 0)
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
        table = runtime.tableFromJSON(unifiedIntegers(records))
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
            conformedIntegers(
              runtime,
              field,
              records.map((record) => record[field.name]),
            ),
            field.type,
          )
        }
        table = new runtime.Table(columns)
      }
      return runtime.tableToIPC(table)
    }

    function nextRowSize() {
      let size = rowSize
      if (remainingRows !== null) size = Math.min(size, remainingRows)
      return size
    }

    function accepted(rows) {
      if (remainingRows !== null) remainingRows -= rows
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
  // The session's finish answers the write's IOResult, counted in the core.
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
          const result = Reflect.apply(finishWriteSession, handle, [session])
          finished = true
          if (typeof iterator.return === 'function') await iterator.return()
          return result
        }
      }

      if (session === undefined) {
        const converted = emptyRecordsReader(settings)
        return publish(converted.reader, converted.settings)
      }
      const result = Reflect.apply(finishWriteSession, handle, [session])
      finished = true
      return result
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
    if (options instanceof TextOptions) return textRecordOptions.call(options)
    return RecordOptions.from(options)
  }

  // A plain object is a set of option properties rather than an options
  // value: `handle.readArrowReader({ rowheader })` reads with the handle's own
  // options carrying that property, and `handle.readArrowReader(options, {
  // rowheader })` with a copy of the given ones. Each property is set by its
  // own setter, so it is validated exactly as an assignment is, and an
  // `undefined` value is skipped: the project's spelling for an argument that
  // was not given. A name no setter of the options' class owns - a typo, a
  // getter, a method - is skipped with an `UnknownPropertyWarning` naming the
  // closest property, whatever its value, and never lands on the copy.
  const { isPropertyBag } = optionProperties

  function withProperties(settings, bag) {
    return settings instanceof TextOptions
      ? optionProperties.withProperties(settings, TextOptions, 'TextOptions', bag)
      : optionProperties.withProperties(settings, RecordOptions, 'RecordOptions', bag)
  }

  // The options a property bag lands on: the ones given, or the handle's own.
  // A plain-text handle answers its own `TextOptions`, so a text setting in
  // the bag lands on the setter it names over the row header it retained.
  function propertyBase(handle, options) {
    if (options instanceof RecordOptions || options instanceof TextOptions) return options
    if (options !== undefined && options !== null) return RecordOptions.from(options)
    return handle.recordOptions()
  }

  // Every write crosses with one concrete options value. This resolves the
  // handle's encoding at the JavaScript boundary, where representation
  // inference can attach a Field without mutating caller-owned options.
  function resolvedRecordOptions(handle, options, properties) {
    if (isPropertyBag(options)) {
      properties = options
      options = undefined
    }
    if (properties === undefined || properties === null) {
      return recordOptions(options == null ? handle.recordOptions() : options)
    }
    return recordOptions(withProperties(propertyBase(handle, options), properties))
  }

  // The read side keeps an absent options value absent, so the native reader
  // resolves the handle's encoding itself; a property bag resolves here.
  function readRecordOptions(handle, options, properties) {
    if (isPropertyBag(options) || (properties !== undefined && properties !== null)) {
      return resolvedRecordOptions(handle, options, properties)
    }
    return recordOptions(options)
  }

  function inferredRecordOptions(settings, reader) {
    return settings.field === null ? settings.withField(reader.field) : settings
  }

  // The destination, where one is given, answers a merge naming no key with
  // its own: an Iceberg table's identity partition columns, then its
  // identifier columns.
  function preflightWriteIntent(settings, intent, handle) {
    return Reflect.apply(requireWritePreflight, settings, [intent, handle])
  }

  // The mode is read by the core's `IOMode` vocabulary, before any input is
  // touched; only its type is checked here.
  function writeMode(mode) {
    if (typeof mode !== 'string') {
      throw new TypeError('mode must be overwrite, append, or merge')
    }
    return nativeWriteMode(mode)
  }

  function writeLimitIsZero(settings) {
    return settings.maxRowSize === 0 || settings.maxByteSize === 0
  }

  // An append bounded to no row reads no source and writes nothing, which is
  // the core's own answer to that bound: its empty result, built by its
  // constructor, so the source is never converted only to be handed over.
  function emptyAppendResult() {
    return new IOResult()
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
            converted.dtype.id !== 'struct' ||
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
        value(source, options, properties) {
          let settings = resolvedRecordOptions(this, options, properties)
          preflightWriteIntent(settings, intent, this)
          if (writeLimitIsZero(settings)) {
            if (intent === 'append') return emptyAppendResult()
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
      value(source, mode, options, properties) {
        const intent = writeMode(mode)
        let settings = resolvedRecordOptions(this, options, properties)
        preflightWriteIntent(settings, intent, this)
        if (writeLimitIsZero(settings)) {
          if (intent === 'append') return emptyAppendResult()
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
    value(options, properties) {
      return readBatches.call(this, readRecordOptions(this, options, properties))
    },
  })

  const readArrowField = IOBase.prototype.readArrowField
  Object.defineProperty(IOBase.prototype, 'readArrowField', {
    configurable: true,
    value(options, properties) {
      return readArrowField.call(this, readRecordOptions(this, options, properties))
    },
  })

  // The line decode takes what a record read takes: absent options are the
  // handle's own, a property bag lands on a copy of them, and the native
  // half refuses options of another encoding by name.
  const readTextLines = IOBase.prototype.readTextLines
  Object.defineProperty(IOBase.prototype, 'readTextLines', {
    configurable: true,
    value(options, properties) {
      return readTextLines.call(this, readRecordOptions(this, options, properties))
    },
  })

  // The Serie verbs keep an absent options value absent, so the core resolves
  // the handle's own - a container's the table beneath it, a structured text
  // document the record column its rows parse into. A property bag lands on
  // a copy of the options given, else of the handle's own, else - for a
  // handle naming no record encoding, as a structured document does, which
  // reads the declared field alone - of Arrow stream options.
  const ARROW_STREAM_MIME_TYPE = 'application/vnd.apache.arrow.stream'

  function serieOptionsBase(handle, options) {
    if (options !== undefined && options !== null) return propertyBase(handle, options)
    try {
      return handle.recordOptions()
    } catch {
      return new RecordOptions(ARROW_STREAM_MIME_TYPE)
    }
  }

  function serieRecordOptions(handle, options, properties) {
    if (isPropertyBag(options)) {
      properties = options
      options = undefined
    }
    if (properties === undefined || properties === null) return recordOptions(options)
    return recordOptions(withProperties(serieOptionsBase(handle, options), properties))
  }

  // Rows in any shape the crate holds them cross as they are; every other
  // columnar value is the stream of its batches - a native BatchReader as it
  // is, an Arrow JS table, batch or batches and IPC bytes through
  // `BatchReader.from` - read under one plan.
  function serieSource(source, rootName) {
    if (
      source instanceof Serie ||
      source instanceof ChunkedSerie ||
      source instanceof StreamChunkedSerie ||
      source instanceof StreamSerie ||
      source instanceof KeySerie ||
      source instanceof KeySeries ||
      source instanceof StreamKeySerie ||
      source instanceof WindowSerie
    ) {
      return source
    }
    if (!isArrowShaped(source)) {
      throw new TypeError(
        'value must be a Serie, a ChunkedSerie, a StreamChunkedSerie, a BatchReader, an Apache Arrow JS Table or RecordBatch, or Arrow IPC bytes',
      )
    }
    return StreamChunkedSerie.fromArrowReader(batchReader(source, rootName))
  }

  // The one generic write: preflighted, bounded and typed exactly as the
  // other record writes are, the rows' own root standing in for a field the
  // options do not declare. An absent options value crosses absent.
  function writeSerie(handle, source, intent, options, properties) {
    const settings = serieRecordOptions(handle, options, properties)
    if (settings === undefined || settings === null) {
      return Reflect.apply(writeSerieNative, handle, [serieSource(source), intent, undefined])
    }
    preflightWriteIntent(settings, intent, handle)
    if (writeLimitIsZero(settings)) {
      if (intent === 'append') return emptyAppendResult()
      // A limited merge was rejected by preflight. An overwrite bounded to no
      // row publishes the declared field's empty value without reading the
      // source; with no field declared, the source's own root names it.
      if (settings.field !== null) {
        const converted = emptyRecordsReader(settings)
        return Reflect.apply(writeSerieNative, handle, [
          StreamChunkedSerie.fromArrowReader(converted.reader),
          intent,
          converted.settings,
        ])
      }
    }
    return Reflect.apply(writeSerieNative, handle, [
      serieSource(source, settings.name),
      intent,
      settings,
    ])
  }

  const readSerie = IOBase.prototype.readSerie
  if (typeof readSerie !== 'function') {
    throw new TypeError('native binding is missing IOBase.readSerie')
  }
  Object.defineProperty(IOBase.prototype, 'readSerie', {
    configurable: true,
    value(options, properties) {
      return readSerie.call(this, serieRecordOptions(this, options, properties))
    },
  })

  for (const intent of intents) {
    Object.defineProperty(IOBase.prototype, `${intent}Serie`, {
      configurable: true,
      value(value, options, properties) {
        return writeSerie(this, value, intent, options, properties)
      },
    })
  }

  Object.defineProperty(IOBase.prototype, 'writeSerie', {
    configurable: true,
    value(value, mode = 'overwrite', options, properties) {
      return writeSerie(this, value, writeMode(mode), options, properties)
    },
  })

  // Rows as records: each stored row as one plain object, or as one instance
  // of the class you pass - `new cls(row)` receives the plain row, so any
  // constructor that takes named fields is a runtime record class. Rows come
  // batch by batch off the same native reader every other read uses, so
  // nothing is collected, and a resource that does not exist yields no rows.
  Object.defineProperty(IOBase.prototype, 'readRecords', {
    configurable: true,
    value(cls, options, properties) {
      if (typeof cls !== 'function') {
        if (properties !== undefined) {
          throw new TypeError(
            'readRecords accepts one options value and its properties, or a record class first',
          )
        }
        properties = options
        options = cls
        cls = undefined
      }
      let settings = readRecordOptions(this, options, properties)
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
  function writeRecordSource(handle, rows, options, properties, intent, publish) {
    const settings = resolvedRecordOptions(handle, options, properties)
    const defaultBatchRowSize = preflightWriteIntent(settings, intent, handle)
    if (writeLimitIsZero(settings)) {
      if (intent === 'append') return emptyAppendResult()
      // A limited merge was rejected by preflight. Overwrite still publishes
      // the explicitly typed empty value without inspecting the input.
      const converted = emptyRecordsReader(settings)
      return publish(converted.reader, converted.settings)
    }
    const asynchronous = needsAwait(rows)
    if (asynchronous && settings.commitBatchNum !== null) {
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
      value(rows, options, properties) {
        return writeRecordSource(
          this,
          rows,
          options,
          properties,
          intent,
          (reader, settings) => native.call(this, reader, settings),
        )
      },
    })
  }

  Object.defineProperty(IOBase.prototype, 'writeRecords', {
    configurable: true,
    value(rows, mode, options, properties) {
      const intent = writeMode(mode)
      return writeRecordSource(
        this,
        rows,
        options,
        properties,
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
    // The table's stored schema types the rows. A schema the rows would have
    // to name is a catalog's business: `Tables.append` creates there, over
    // the schema as Iceberg expresses it.
    const settings = new RecordOptions(ICEBERG_DATA_MIME_TYPE).withField(table.schema)
    return recordsReader(source, settings, preflightWriteIntent(settings, 'append')).reader
  }

  const explicitOptions = Table.prototype._explicitOptionsNative
  delete Table.prototype._explicitOptionsNative

  // The per-call options an Iceberg call runs under. A property bag - beside
  // the options, or alone in their place - is set on a copy of the options
  // given, else of the table's own override, else of nothing set, by each
  // field's own setter; a name no field owns is skipped with a warning.
  function icebergCallOptions(table, options, bag) {
    if (optionProperties.isPropertyBag(options)) {
      bag = options
      options = undefined
    }
    if (bag === undefined || bag === null) return options
    const base =
      options ?? (table ? explicitOptions.call(table) : null) ?? new IcebergOptions()
    return optionProperties.withProperties(base, IcebergOptions, 'IcebergOptions', bag)
  }

  // The writes that take rows widen them the way every other write here does,
  // and pass the trailing per-call options through - with their properties
  // set on a copy. Forwarding it is not optional bookkeeping: a wrapper that
  // drops the argument leaves a documented option silently doing nothing,
  // which is worse than not having it at all.
  for (const name of ['append', 'overwrite']) {
    const native = Table.prototype[name]
    Object.defineProperty(Table.prototype, name, {
      configurable: true,
      value(batches, options, properties) {
        const settings = icebergCallOptions(this, options, properties)
        return native.call(this, icebergBatchReader(this, batches), settings)
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
      value(filters, batches, options, properties) {
        const settings = icebergCallOptions(this, options, properties)
        return overwriteWhere.call(this, filters, icebergBatchReader(this, batches), settings)
      },
    })
  }

  const merge = Table.prototype.merge
  if (merge) {
    Object.defineProperty(Table.prototype, 'merge', {
      configurable: true,
      value(batches, mergeBy, safe, options, properties) {
        const settings = icebergCallOptions(this, options, properties)
        return merge.call(this, icebergBatchReader(this, batches), mergeBy, safe, settings)
      },
    })
  }

  const mergeWhere = Table.prototype.mergeWhere
  if (mergeWhere) {
    Object.defineProperty(Table.prototype, 'mergeWhere', {
      configurable: true,
      value(filters, batches, mergeBy, safe, options, properties) {
        const settings = icebergCallOptions(this, options, properties)
        return mergeWhere.call(
          this,
          filters,
          icebergBatchReader(this, batches),
          mergeBy,
          safe,
          settings,
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
        // The options follow the projection; a property bag beside them.
        if (args.length > at + 1) {
          args[at + 1] = icebergCallOptions(this, args[at + 1], args[at + 2])
          args.length = at + 2
        }
        return native.apply(this, args)
      },
    })
  }

  const scanAt = Table.prototype.scanAt
  Object.defineProperty(Table.prototype, 'scanAt', {
    configurable: true,
    value(snapshotId, filters, field, options, properties) {
      const settings = icebergCallOptions(this, options, properties)
      return scanAt.call(this, snapshotId, filters, optionalField(field), settings)
    },
  })

  const evolveSchema = Table.prototype.evolveSchema
  Object.defineProperty(Table.prototype, 'evolveSchema', {
    configurable: true,
    value(schema) {
      return evolveSchema.call(this, intoField(schema))
    },
  })

  return Object.freeze({ intoField })
}

module.exports = { installRecords }
