'use strict'

// Keep language protocols here: Node-API owns values and validation, while
// this loader only adapts JavaScript symbols and built-in collection inputs.
const fs = require('node:fs')
const { fileURLToPath, URL } = require('node:url')
const { types: utilTypes } = require('node:util')
const binding = require('./index.js')

// Native methods taking serde_json::Value recurse through caller-owned JS
// objects before Rust can enforce its own schema/value depth limits. Keep that
// FFI traversal on a detached, bounded JSON tree. The 256 raw-container limit
// admits Yggdryl's maximum valid 62-level nested-union Record wire (depth 254)
// while staying below the depth at which V8/NAPI recursive conversion can
// exhaust the central stack. This is a host-value safety boundary, not another
// schema or Record wire implementation.
const NATIVE_JSON_MAX_DEPTH = 256
const NATIVE_JSON_MAX_NODES = 1_000_000

function snapshotNativeJSON(value, label = 'JSON value') {
  const holder = Object.create(null)
  const pending = [{ source: value, target: holder, key: 'value', depth: 0 }]
  const seen = new WeakSet()
  let nodes = 0

  const assign = (target, key, item) => {
    Object.defineProperty(target, key, {
      configurable: true,
      enumerable: true,
      value: item,
      writable: true,
    })
  }

  while (pending.length !== 0) {
    const { source, target, key, depth } = pending.pop()
    nodes += 1
    if (nodes > NATIVE_JSON_MAX_NODES) {
      throw new RangeError(
        `${label} exceeds the hard limit of ${NATIVE_JSON_MAX_NODES} nodes`,
      )
    }
    if (depth > NATIVE_JSON_MAX_DEPTH) {
      throw new RangeError(
        `${label} exceeds the hard depth limit of ${NATIVE_JSON_MAX_DEPTH}`,
      )
    }

    if (
      source === null ||
      typeof source === 'string' ||
      typeof source === 'boolean'
    ) {
      assign(target, key, source)
      continue
    }
    if (typeof source === 'number') {
      if (!Number.isFinite(source)) {
        throw new TypeError(`${label} contains a non-finite JSON number`)
      }
      assign(target, key, source)
      continue
    }
    if (typeof source !== 'object') {
      throw new TypeError(`${label} contains a non-JSON ${typeof source} value`)
    }
    if (utilTypes.isProxy(source)) {
      throw new TypeError(`${label} must not contain Proxy objects`)
    }
    if (seen.has(source)) {
      throw new TypeError(
        `${label} must be an acyclic tree without shared object references`,
      )
    }
    seen.add(source)

    if (Array.isArray(source)) {
      const length = source.length
      if (length > NATIVE_JSON_MAX_NODES - nodes - pending.length) {
        throw new RangeError(
          `${label} exceeds the hard limit of ${NATIVE_JSON_MAX_NODES} nodes`,
        )
      }
      const keys = Reflect.ownKeys(source)
      if (keys.length !== length + 1 || !keys.includes('length')) {
        throw new TypeError(`${label} arrays must be dense and contain no extra keys`)
      }
      const clone = new Array(length)
      assign(target, key, clone)
      for (let index = length - 1; index >= 0; index -= 1) {
        const itemKey = String(index)
        const descriptor = Object.getOwnPropertyDescriptor(source, itemKey)
        if (
          descriptor === undefined ||
          !descriptor.enumerable ||
          !Object.hasOwn(descriptor, 'value')
        ) {
          throw new TypeError(
            `${label} arrays must use enumerable own data elements`,
          )
        }
        pending.push({
          depth: depth + 1,
          key: itemKey,
          source: descriptor.value,
          target: clone,
        })
      }
      continue
    }

    const prototype = Object.getPrototypeOf(source)
    if (
      prototype !== null &&
      (utilTypes.isProxy(prototype) || Object.getPrototypeOf(prototype) !== null)
    ) {
      throw new TypeError(`${label} must contain only plain objects`)
    }
    const keys = Reflect.ownKeys(source)
    if (keys.length > NATIVE_JSON_MAX_NODES - nodes - pending.length) {
      throw new RangeError(
        `${label} exceeds the hard limit of ${NATIVE_JSON_MAX_NODES} nodes`,
      )
    }
    const clone = Object.create(null)
    assign(target, key, clone)
    for (let index = keys.length - 1; index >= 0; index -= 1) {
      const itemKey = keys[index]
      if (typeof itemKey !== 'string') {
        throw new TypeError(`${label} must not contain symbol keys`)
      }
      const descriptor = Object.getOwnPropertyDescriptor(source, itemKey)
      if (
        descriptor === undefined ||
        !descriptor.enumerable ||
        !Object.hasOwn(descriptor, 'value')
      ) {
        throw new TypeError(
          `${label} objects must use enumerable own data properties`,
        )
      }
      pending.push({
        depth: depth + 1,
        key: itemKey,
        source: descriptor.value,
        target: clone,
      })
    }
  }

  return holder.value
}

const {
  DataType: NativeDataType,
  Field: NativeField,
  MediaType: NativeMediaType,
  MimeType: NativeMimeType,
  Uri: NativeUri,
  Url: NativeUrl,
  Urn: NativeUrn,
  Value: NativeValue,
} = binding

// The pivot keeps its private conversion handles inside this loader: `fromJs`
// needs the intrinsic tables assembled below and `asJs` needs the transport
// reader, and neither of those belongs on the published class.
const nativeValueFromJs = NativeValue._fromJsNative.bind(NativeValue)
const nativeValueAsJs = NativeValue.prototype._asJsNative
delete NativeValue.prototype._asJsNative

function publicNativeClass(NativeClass, name, hiddenStatics) {
  const PublicClass = function (...args) {
    if (new.target === undefined) {
      throw new TypeError(`Class constructor ${name} cannot be invoked without 'new'`)
    }
    const target = new.target === PublicClass ? NativeClass : new.target
    return Reflect.construct(NativeClass, args, target)
  }
  Object.defineProperty(PublicClass, 'name', { value: name })
  PublicClass.prototype = NativeClass.prototype
  Object.defineProperty(PublicClass.prototype, 'constructor', {
    configurable: true,
    value: PublicClass,
    writable: true,
  })
  for (const key of Reflect.ownKeys(NativeClass)) {
    if (
      key === 'name' ||
      key === 'length' ||
      key === 'prototype' ||
      hiddenStatics.has(key)
    ) {
      continue
    }
    Object.defineProperty(
      PublicClass,
      key,
      Object.getOwnPropertyDescriptor(NativeClass, key),
    )
  }
  return PublicClass
}

const internalDataTypeNames = new Set([
  'fromJSON',
  '_simple',
  '_temporal',
  '_fixedSizeBinary',
  '_decimal',
  '_list',
  '_fromFields',
  '_union',
  '_variant',
  '_dictionary',
  '_map',
  '_mapOf',
  '_runEndEncoded',
  'fromArrowString',
])
const DataType = publicNativeClass(
  NativeDataType,
  'DataType',
  internalDataTypeNames,
)
const Field = publicNativeClass(
  NativeField,
  'Field',
  new Set(['fromArrowString', 'fromJSON']),
)
const MimeType = publicNativeClass(
  NativeMimeType,
  'MimeType',
  new Set(['_known', 'fromJSON']),
)
const MediaType = publicNativeClass(
  NativeMediaType,
  'MediaType',
  new Set(['_fromExtensions', '_fromParts', 'fromJSON']),
)
const Uri = publicNativeClass(NativeUri, 'Uri', new Set(['fromJSON']))
const Url = publicNativeClass(NativeUrl, 'Url', new Set(['fromJSON']))
const Urn = publicNativeClass(NativeUrn, 'Urn', new Set(['fromJSON']))
const Value = publicNativeClass(NativeValue, 'Value', new Set(['_fromJsNative']))
binding.DataType = DataType
binding.Field = Field
binding.MimeType = MimeType
binding.MediaType = MediaType
binding.Uri = Uri
binding.Url = Url
binding.Urn = Urn
binding.Value = Value

for (const [PublicClass, NativeClass, name] of [
  [DataType, NativeDataType, 'DataType'],
  [Field, NativeField, 'Field'],
  [MimeType, NativeMimeType, 'MimeType'],
  [MediaType, NativeMediaType, 'MediaType'],
  [Uri, NativeUri, 'Uri'],
  [Url, NativeUrl, 'Url'],
  [Urn, NativeUrn, 'Urn'],
]) {
  const nativeFromJSON = NativeClass.fromJSON.bind(NativeClass)
  Object.defineProperty(PublicClass, 'fromJSON', {
    value(value) {
      return nativeFromJSON(snapshotNativeJSON(value, `${name}.fromJSON input`))
    },
  })
}

const knownMimeNames = Object.freeze([
  'OCTET_STREAM',
  'JSON',
  'JSON_LINES',
  'YAML',
  'TOML',
  'CSV',
  'TSV',
  'PARQUET',
  'ARROW_FILE',
  'ARROW_STREAM',
  'AVRO',
  'ORC',
  'PLAIN_TEXT',
  'MARKDOWN',
  'HTML',
  'CSS',
  'JAVASCRIPT',
  'XML',
  'PDF',
  'CBOR',
  'MESSAGE_PACK',
  'PROTOBUF',
  'SQLITE3',
  'PNG',
  'JPEG',
  'GIF',
  'WEBP',
  'SVG',
  'MP3',
  'WAV',
  'OGG',
  'FLAC',
  'MP4',
  'WEBM',
  'WOFF',
  'WOFF2',
  'TTF',
  'OTF',
  'XLS',
  'XLSX',
  'ODS',
  'DOC',
  'DOCX',
  'GZIP',
  'ZSTD',
  'BROTLI',
  'ZLIB',
  'COMPRESS',
  'BZIP2',
  'XZ',
  'LZ4',
  'SNAPPY',
  'ZIP',
  'SEVEN_ZIP',
  'RAR',
  'TAR',
])
const nativeKnownMime = NativeMimeType._known.bind(NativeMimeType)
for (const name of knownMimeNames) {
  Object.defineProperty(MimeType, name, {
    enumerable: true,
    value: Object.freeze(nativeKnownMime(name)),
    writable: false,
  })
}

function collectMediaIterable(values, label) {
  if (
    values === null ||
    typeof values === 'string' ||
    typeof values?.[Symbol.iterator] !== 'function'
  ) {
    throw new TypeError(`${label} must be a non-string iterable`)
  }
  return Array.from(values)
}

const mediaFromParts = NativeMediaType._fromParts.bind(NativeMediaType)
Object.defineProperty(MediaType, 'fromParts', {
  value(base, encodings) {
    return mediaFromParts(base, collectMediaIterable(encodings, 'encodings'))
  },
})
const mediaFromExtensions = NativeMediaType._fromExtensions.bind(NativeMediaType)
Object.defineProperty(MediaType, 'fromExtensions', {
  value(extensions) {
    return mediaFromExtensions(collectMediaIterable(extensions, 'extensions'))
  },
})
const mediaSetEncodings = NativeMediaType.prototype._setEncodings
delete NativeMediaType.prototype._setEncodings
Object.defineProperty(MediaType.prototype, 'setEncodings', {
  configurable: true,
  value(encodings) {
    return mediaSetEncodings.call(
      this,
      collectMediaIterable(encodings, 'encodings'),
    )
  },
})
Object.defineProperty(MediaType.prototype, Symbol.iterator, {
  configurable: true,
  value: function encodings() {
    return this.encodings[Symbol.iterator]()
  },
})
const { createFields, normalizeMetadata } = require('./fields.js')
const internalDataType = Object.freeze({
  simple: NativeDataType._simple.bind(NativeDataType),
  temporal: NativeDataType._temporal.bind(NativeDataType),
  fixedSizeBinary: NativeDataType._fixedSizeBinary.bind(NativeDataType),
  decimal: NativeDataType._decimal.bind(NativeDataType),
  list: NativeDataType._list.bind(NativeDataType),
  fromFields: NativeDataType._fromFields.bind(NativeDataType),
  union: NativeDataType._union.bind(NativeDataType),
  variant: NativeDataType._variant.bind(NativeDataType),
  dictionary: NativeDataType._dictionary.bind(NativeDataType),
  map: NativeDataType._map.bind(NativeDataType),
  mapOf: NativeDataType._mapOf.bind(NativeDataType),
  runEndEncoded: NativeDataType._runEndEncoded.bind(NativeDataType),
})

function collectFields(values) {
  if (
    values === null ||
    typeof values === 'string' ||
    typeof values?.[Symbol.iterator] !== 'function'
  ) {
    throw new TypeError('fields must be an iterable of native Field values')
  }
  return Array.from(values)
}

Object.defineProperty(DataType, 'fromFields', {
  value(values) {
    return internalDataType.fromFields(collectFields(values))
  },
})

Object.defineProperty(DataType, 'variant', {
  value(values) {
    return internalDataType.variant(collectFields(values))
  },
})

const fields = createFields(DataType, Field, internalDataType)
const { installDefaults } = require('./defaults.js')
installDefaults({ DataType, Field, NativeDataType, NativeField })
const nativeCodec = Object.freeze({
  inferFormat: binding.codecInferFormat,
  loadsInferred: binding.codecLoadsInferredNative,
  normalizeFormat: binding.codecNormalizeFormat,
  json: Object.freeze({
    dumpAll: binding.jsonLinesDumpAllNative,
    dumpAllPath: binding.jsonLinesDumpPathNative,
    dumpPath: binding.jsonDumpPathNative,
    dumps: binding.jsonDumpsNative,
    loadAllPath: binding.jsonLinesLoadPathNative,
    loadPath: binding.jsonLoadPathNative,
    loads: binding.jsonLoadsNative,
    loadsAll: binding.jsonLinesLoadsNative,
  }),
  toml: Object.freeze({
    dumpPath: binding.tomlDumpPathNative,
    dumps: binding.tomlDumpsNative,
    loadPath: binding.tomlLoadPathNative,
    loads: binding.tomlLoadsNative,
  }),
  yaml: Object.freeze({
    dumpAll: binding.yamlDumpAllNative,
    dumpAllPath: binding.yamlDumpAllPathNative,
    dumpPath: binding.yamlDumpPathNative,
    dumps: binding.yamlDumpsNative,
    loadAllPath: binding.yamlLoadAllPathNative,
    loadPath: binding.yamlLoadPathNative,
    loads: binding.yamlLoadsNative,
    loadsAll: binding.yamlLoadsAllNative,
  }),
})
for (const name of [
  'codecInferFormat',
  'codecLoadsInferredNative',
  'codecNormalizeFormat',
  'jsonDumpPathNative',
  'jsonDumpsNative',
  'jsonLinesDumpAllNative',
  'jsonLinesDumpPathNative',
  'jsonLinesLoadPathNative',
  'jsonLinesLoadsNative',
  'jsonLoadPathNative',
  'jsonLoadsNative',
  'tomlDumpPathNative',
  'tomlDumpsNative',
  'tomlLoadPathNative',
  'tomlLoadsNative',
  'yamlDumpAllNative',
  'yamlDumpAllPathNative',
  'yamlDumpPathNative',
  'yamlDumpsNative',
  'yamlLoadAllPathNative',
  'yamlLoadPathNative',
  'yamlLoadsAllNative',
  'yamlLoadsNative',
]) {
  delete binding[name]
}

const TRANSPORT_KEY = '__yggdryl_codec__'
const FORMAT_JSON = 'json'
const FORMAT_JSON_LINES = 'json_lines'
const FORMAT_TOML = 'toml'
const FORMAT_YAML = 'yaml'
const MAX_DEPTH = 48
const DEFAULT_MAX_STREAM_BYTES = 64 * 1024 * 1024
const DEFAULT_MAX_STREAM_DOCUMENTS = 1024
const UTF8_REPLACEMENT_BYTE_LENGTH = 3
const YAML_EXPLICIT_END = Buffer.from('...\n')
const nativeWrapperPrototypes = Object.freeze([
  Value.prototype,
  DataType.prototype,
  Field.prototype,
  Uri.prototype,
  Url.prototype,
  Urn.prototype,
])
const regexpSourceGetter = Object.getOwnPropertyDescriptor(
  RegExp.prototype,
  'source',
).get
const regexpFlagsGetter = Object.getOwnPropertyDescriptor(
  RegExp.prototype,
  'flags',
).get
const nativeIntrinsics = Object.freeze([
  utilTypes.isMap,
  utilTypes.isSet,
  utilTypes.isRegExp,
  (value) => Reflect.apply(regexpSourceGetter, value, []),
  (value) => Reflect.apply(regexpFlagsGetter, value, []),
])

function checkedOptions(options) {
  if (options == null) return {}
  if (typeof options !== 'object' || Array.isArray(options)) {
    throw new TypeError('codec options must be an object')
  }
  if (
    options.maxDepth !== undefined &&
    (!Number.isSafeInteger(options.maxDepth) ||
      options.maxDepth <= 0 ||
      options.maxDepth > MAX_DEPTH)
  ) {
    throw new RangeError(`maxDepth must be an integer between 1 and ${MAX_DEPTH}`)
  }
  if (options.format !== undefined && typeof options.format !== 'string') {
    throw new TypeError('codec format must be a string')
  }
  return options
}

function toNativeContent(content) {
  if (typeof content === 'string' || Buffer.isBuffer(content)) return content
  if (utilTypes.isAnyArrayBuffer(content)) return Buffer.from(content)
  if (ArrayBuffer.isView(content)) {
    return Buffer.from(content.buffer, content.byteOffset, content.byteLength)
  }
  throw new TypeError(
    'content must be a string, Buffer, ArrayBuffer, SharedArrayBuffer, or array-buffer view',
  )
}

function toBytes(content) {
  const native = toNativeContent(content)
  return typeof native === 'string' ? Buffer.from(native) : native
}

function endsWithHighSurrogate(value) {
  if (value.length === 0) return false
  const codeUnit = value.charCodeAt(value.length - 1)
  return codeUnit >= 0xd800 && codeUnit <= 0xdbff
}

async function* streamByteChunks(stream) {
  if (stream == null || typeof stream[Symbol.asyncIterator] !== 'function') {
    throw new TypeError('stream must be an async iterable of string or byte chunks')
  }
  let pendingHighSurrogate = ''
  let size = 0
  for await (const chunk of stream) {
    if (typeof chunk === 'string') {
      let text = pendingHighSurrogate + chunk
      pendingHighSurrogate = ''
      if (endsWithHighSurrogate(text)) {
        pendingHighSurrogate = text.slice(-1)
        text = text.slice(0, -1)
      }
      if (text.length !== 0) {
        const byteLength = Buffer.byteLength(text)
        size += byteLength
        if (size > DEFAULT_MAX_STREAM_BYTES) {
          throw new RangeError(
            `stream exceeds the ${DEFAULT_MAX_STREAM_BYTES}-byte input limit`,
          )
        }
        yield Buffer.from(text)
      }
      continue
    }
    if (pendingHighSurrogate !== '') {
      size += UTF8_REPLACEMENT_BYTE_LENGTH
      if (size > DEFAULT_MAX_STREAM_BYTES) {
        throw new RangeError(
          `stream exceeds the ${DEFAULT_MAX_STREAM_BYTES}-byte input limit`,
        )
      }
      yield Buffer.from(pendingHighSurrogate)
      pendingHighSurrogate = ''
    }
    const bytes = toBytes(chunk)
    size += bytes.length
    if (size > DEFAULT_MAX_STREAM_BYTES) {
      throw new RangeError(
        `stream exceeds the ${DEFAULT_MAX_STREAM_BYTES}-byte input limit`,
      )
    }
    yield bytes
  }
  if (pendingHighSurrogate !== '') {
    if (size > DEFAULT_MAX_STREAM_BYTES - UTF8_REPLACEMENT_BYTE_LENGTH) {
      throw new RangeError(
        `stream exceeds the ${DEFAULT_MAX_STREAM_BYTES}-byte input limit`,
      )
    }
    yield Buffer.from(pendingHighSurrogate)
  }
}

function sourcePath(value) {
  if (value instanceof URL) {
    if (value.protocol !== 'file:') {
      throw new TypeError('URL codec sources must use the file: protocol')
    }
    return fileURLToPath(value)
  }
  if (typeof value !== 'string') return null
  try {
    return fs.statSync(value).isFile() ? value : null
  } catch {
    return null
  }
}

function readDescriptorBounded(descriptor) {
  const size = fs.fstatSync(descriptor).size
  if (size > DEFAULT_MAX_STREAM_BYTES) {
    throw new RangeError(
      `input exceeds the ${DEFAULT_MAX_STREAM_BYTES}-byte input limit`,
    )
  }
  const chunks = []
  let total = 0
  while (true) {
    const capacity = Math.min(
      64 * 1024,
      DEFAULT_MAX_STREAM_BYTES + 1 - total,
    )
    if (capacity <= 0) {
      throw new RangeError(
        `input exceeds the ${DEFAULT_MAX_STREAM_BYTES}-byte input limit`,
      )
    }
    const chunk = Buffer.allocUnsafe(capacity)
    const length = fs.readSync(descriptor, chunk, 0, capacity, null)
    if (length === 0) break
    total += length
    if (total > DEFAULT_MAX_STREAM_BYTES) {
      throw new RangeError(
        `input exceeds the ${DEFAULT_MAX_STREAM_BYTES}-byte input limit`,
      )
    }
    chunks.push(chunk.subarray(0, length))
  }
  if (chunks.length === 0) return Buffer.alloc(0)
  if (chunks.length === 1) return chunks[0]
  return Buffer.concat(chunks, total)
}

function readSource(source) {
  if (typeof source === 'number') {
    return { content: readDescriptorBounded(source), path: null }
  }
  const path = sourcePath(source)
  if (path !== null) return { content: null, path }
  return { content: toNativeContent(source), path: null }
}

function formatFor(source, options, fallback) {
  if (options.format !== undefined) {
    return nativeCodec.normalizeFormat(options.format)
  }
  if (source !== null) return nativeCodec.inferFormat(source)
  if (fallback !== undefined) return fallback
  throw new TypeError('codec format requires content, a supported path, or an explicit format')
}

function markerShape(value, kind, keys) {
  if (
    value === null ||
    typeof value !== 'object' ||
    Array.isArray(value) ||
    value[TRANSPORT_KEY] !== kind
  ) {
    return false
  }
  const actual = Object.keys(value).sort()
  return actual.length === keys.length && actual.every((key, index) => key === keys[index])
}

// A temporal and an exact decimal arrive as their parts rather than as one
// number: a nanosecond instant needs more than the 53 bits a JSON number keeps
// exactly, and a decimal fraction has no finite binary expansion at all.
function fromTemporalMarker(value) {
  if (markerShape(value, 'decimal', [TRANSPORT_KEY, 'scale', 'value'].sort())) {
    return Value.decimal(BigInt(value.value), value.scale)
  }
  if (markerShape(value, 'date', [TRANSPORT_KEY, 'value'].sort())) {
    return Value.date(value.value)
  }
  if (markerShape(value, 'time', [TRANSPORT_KEY, 'unit', 'value'].sort())) {
    return Value.time(BigInt(value.value), value.unit)
  }
  if (markerShape(value, 'duration', [TRANSPORT_KEY, 'unit', 'value'].sort())) {
    return Value.duration(BigInt(value.value), value.unit)
  }
  if (
    !markerShape(
      value,
      'timestamp',
      [TRANSPORT_KEY, 'date', 'unit', 'value', 'zone'].sort(),
    )
  ) {
    return undefined
  }
  // The core decides whether a Date holds this instant exactly; when it does,
  // `date` carries the millisecond count a Date would hold. Every other
  // resolution or zone stays a native Value, because rounding would change it.
  if (value.date !== null) return new Date(value.date)
  return Value.timestamp(BigInt(value.value), value.unit, value.zone)
}

function fromTransport(value) {
  if (Array.isArray(value)) {
    return value.map((item) => fromTransport(item))
  }
  if (value === null || typeof value !== 'object') return value
  if (markerShape(value, 'bytes', [TRANSPORT_KEY, 'value'].sort())) {
    return Buffer.from(value.value, 'base64')
  }
  if (markerShape(value, 'bigint', [TRANSPORT_KEY, 'value'].sort())) {
    return BigInt(value.value)
  }
  if (markerShape(value, 'float', [TRANSPORT_KEY, 'value'].sort())) {
    switch (value.value) {
      case 'nan':
        return Number.NaN
      case 'infinity':
        return Number.POSITIVE_INFINITY
      case '-infinity':
        return Number.NEGATIVE_INFINITY
      case '-0':
        return -0
      default:
        throw new TypeError(`invalid native float marker: ${value.value}`)
    }
  }
  if (markerShape(value, 'mapping', [TRANSPORT_KEY, 'value'].sort())) {
    const result = new Map()
    for (const [encodedKey, encodedItem] of value.value) {
      const key = fromTransport(encodedKey)
      if (result.has(key)) {
        throw new TypeError(
          'decoded mapping contains distinct native keys that collide under JavaScript Map equality',
        )
      }
      result.set(key, fromTransport(encodedItem))
    }
    return result
  }
  if (markerShape(value, 'object', [TRANSPORT_KEY, 'value'].sort())) {
    const result = {}
    for (const [key, item] of value.value) {
      Object.defineProperty(result, key, {
        configurable: true,
        enumerable: true,
        value: fromTransport(item),
        writable: true,
      })
    }
    return result
  }
  const temporal = fromTemporalMarker(value)
  if (temporal !== undefined) return temporal
  const result = {}
  for (const [key, item] of Object.entries(value)) {
    Object.defineProperty(result, key, {
      configurable: true,
      enumerable: true,
      value: fromTransport(item),
      writable: true,
    })
  }
  return result
}

// The conversion pair every codec entry point crosses. `dumps` is `fromJs`
// with bytes on the far side and `loads` is `asJs`; they run this exact code.
Object.defineProperty(Value, 'fromJs', {
  value(value, options) {
    options = checkedOptions(options)
    return nativeValueFromJs(
      value,
      options.maxDepth,
      nativeWrapperPrototypes,
      nativeIntrinsics,
    )
  },
})

Object.defineProperty(Value.prototype, 'asJs', {
  configurable: true,
  value(options) {
    options = checkedOptions(options)
    return fromTransport(Reflect.apply(nativeValueAsJs, this, [options.maxDepth]))
  },
})

function nativeFormat(format) {
  switch (format) {
    case FORMAT_JSON:
    case FORMAT_JSON_LINES:
      return nativeCodec.json
    case FORMAT_TOML:
      return nativeCodec.toml
    case FORMAT_YAML:
      return nativeCodec.yaml
    default:
      throw new TypeError(`unsupported normalized codec format ${format}`)
  }
}

function nativeLoads(content, format, options) {
  options = checkedOptions(options)
  return fromTransport(
    nativeFormat(format).loads(toNativeContent(content), options.maxDepth),
  )
}

function nativeLoadsInferred(content, options) {
  options = checkedOptions(options)
  const decoded = nativeCodec.loadsInferred(
    toNativeContent(content),
    options.maxDepth,
  )
  return fromTransport(decoded)
}

function nativeLoadsAll(content, format, options) {
  options = checkedOptions(options)
  return nativeFormat(format)
    .loadsAll(toNativeContent(content), options.maxDepth)
    .map((value) => fromTransport(value))
}

function nativeLoadPath(path, format, options) {
  options = checkedOptions(options)
  return fromTransport(nativeFormat(format).loadPath(path, options.maxDepth))
}

function nativeLoadAllPath(path, format, options) {
  options = checkedOptions(options)
  return nativeFormat(format)
    .loadAllPath(path, options.maxDepth)
    .map((value) => fromTransport(value))
}

function nativeDumps(value, format, options) {
  options = checkedOptions(options)
  return nativeFormat(format).dumps(
    value,
    options.maxDepth,
    nativeWrapperPrototypes,
    nativeIntrinsics,
  )
}

function boundedValues(values) {
  if (values == null || typeof values[Symbol.iterator] !== 'function') {
    throw new TypeError('values must be iterable')
  }
  const bounded = []
  for (const value of values) {
    if (bounded.length >= DEFAULT_MAX_STREAM_DOCUMENTS) {
      throw new RangeError(
        `codec collection exceeds the ${DEFAULT_MAX_STREAM_DOCUMENTS}-document limit`,
      )
    }
    bounded.push(value)
  }
  return bounded
}

function nativeDumpAll(values, format, options) {
  options = checkedOptions(options)
  return nativeFormat(format).dumpAll(
    boundedValues(values),
    options.maxDepth,
    nativeWrapperPrototypes,
    nativeIntrinsics,
  )
}

function nativeDumpPath(value, path, format, options) {
  options = checkedOptions(options)
  nativeFormat(format).dumpPath(
    value,
    path,
    options.maxDepth,
    nativeWrapperPrototypes,
    nativeIntrinsics,
  )
}

function nativeDumpAllPath(values, path, format, options) {
  options = checkedOptions(options)
  nativeFormat(format).dumpAllPath(
    boundedValues(values),
    path,
    options.maxDepth,
    nativeWrapperPrototypes,
    nativeIntrinsics,
  )
}

function isDestination(value) {
  return (
    typeof value === 'string' ||
    typeof value === 'number' ||
    value instanceof URL ||
    isWritable(value)
  )
}

function isReadable(value) {
  return value !== null && typeof value?.[Symbol.asyncIterator] === 'function'
}

function isNodeWritable(value) {
  return value !== null && typeof value === 'object' && typeof value.write === 'function'
}

function isWebWritable(value) {
  return value !== null && typeof value === 'object' && typeof value.getWriter === 'function'
}

function isWritable(value) {
  return isNodeWritable(value) || isWebWritable(value)
}

function destinationPath(destination) {
  if (typeof destination === 'string') return destination
  if (!(destination instanceof URL)) return null
  if (destination.protocol !== 'file:') {
    throw new TypeError('URL codec destinations must use the file: protocol')
  }
  return fileURLToPath(destination)
}

function writeDestination(destination, bytes) {
  if (destination === undefined || destination === null) return bytes
  if (typeof destination === 'string' || typeof destination === 'number') {
    fs.writeFileSync(destination, bytes)
    return undefined
  }
  if (destination instanceof URL) {
    destinationPath(destination)
    fs.writeFileSync(destination, bytes)
    return undefined
  }
  throw new TypeError('destination must be a path, file URL, or descriptor')
}

async function readStream(stream) {
  const chunks = []
  let size = 0
  for await (const bytes of streamByteChunks(stream)) {
    size += bytes.length
    if (size > DEFAULT_MAX_STREAM_BYTES) {
      throw new RangeError(
        `stream exceeds the ${DEFAULT_MAX_STREAM_BYTES}-byte input limit`,
      )
    }
    chunks.push(bytes)
  }
  if (chunks.length === 0) return Buffer.alloc(0)
  if (chunks.length === 1) return chunks[0]
  return Buffer.concat(chunks, size)
}

async function writeStream(stream, bytes) {
  if (isWebWritable(stream)) {
    const writer = stream.getWriter()
    try {
      await writer.ready
      await writer.write(bytes)
    } finally {
      writer.releaseLock()
    }
    return
  }
  if (!isNodeWritable(stream)) {
    throw new TypeError('stream must be a Node or WHATWG writable stream')
  }
  await new Promise((resolve, reject) => {
    let settled = false
    const cleanup = () => {
      if (typeof stream.off === 'function') stream.off('error', onError)
    }
    const finish = (error) => {
      if (settled) return
      settled = true
      if (error) {
        // Node emits the same write-callback error after invoking the callback.
        // Retain the listener through that emission so it cannot escape as an
        // uncaught exception, then remove it for caller-owned stream reuse.
        setImmediate(cleanup)
        reject(error)
      } else {
        cleanup()
        resolve()
      }
    }
    const onError = (error) => {
      cleanup()
      finish(error)
    }
    if (typeof stream.once === 'function') stream.once('error', onError)
    try {
      const result = stream.write(bytes, (error) => finish(error))
      if (result !== null && typeof result?.then === 'function') {
        result.then(() => finish(), finish)
      } else if (stream.write.length < 2) {
        finish()
      }
    } catch (error) {
      finish(error)
    }
  })
}

function parserByteOffset(error) {
  const match = /\bat byte (\d+)\b/i.exec(String(error?.message ?? error))
  if (match === null) return 0
  const offset = Number(match[1])
  return Number.isSafeInteger(offset) ? offset : 0
}

function streamError(format, frameOffset, error) {
  const localOffset = parserByteOffset(error)
  const byteOffset = frameOffset + localOffset
  const wrapped = new SyntaxError(
    `${format} stream error at cumulative byte ${byteOffset} (frame byte ${localOffset}): ${error.message}`,
  )
  wrapped.cause = error
  return wrapped
}

function joinParts(parts, length) {
  if (parts.length === 0) return Buffer.alloc(0)
  if (parts.length === 1) return parts[0]
  return Buffer.concat(parts, length)
}

function isJsonWhitespace(bytes) {
  for (const byte of bytes) {
    if (byte !== 0x09 && byte !== 0x0a && byte !== 0x0d && byte !== 0x20) {
      return false
    }
  }
  return true
}

async function* jsonLinesStream(stream, options) {
  let lineParts = []
  let lineLength = 0
  let lineOffset = 0
  let total = 0
  let documents = 0
  for await (const bytes of streamByteChunks(stream)) {
    const chunkOffset = total
    total += bytes.length
    if (total > DEFAULT_MAX_STREAM_BYTES) {
      throw new RangeError(
        `stream exceeds the ${DEFAULT_MAX_STREAM_BYTES}-byte input limit`,
      )
    }
    let start = 0
    let newline = bytes.indexOf(0x0a, start)
    while (newline !== -1) {
      const part = bytes.subarray(start, newline)
      if (part.length !== 0) lineParts.push(part)
      lineLength += part.length
      let line = joinParts(lineParts, lineLength)
      if (line.length > 0 && line[line.length - 1] === 0x0d) {
        line = line.subarray(0, line.length - 1)
      }
      if (!isJsonWhitespace(line)) {
        documents += 1
        if (documents > DEFAULT_MAX_STREAM_DOCUMENTS) {
          throw new RangeError('JSON Lines document limit exceeded')
        }
        try {
          yield nativeLoads(line, FORMAT_JSON, options)
        } catch (error) {
          throw streamError('JSON Lines', lineOffset, error)
        }
      }
      lineParts = []
      lineLength = 0
      start = newline + 1
      lineOffset = chunkOffset + start
      newline = bytes.indexOf(0x0a, start)
    }
    const tail = bytes.subarray(start)
    if (tail.length !== 0) lineParts.push(tail)
    lineLength += tail.length
  }
  let pending = joinParts(lineParts, lineLength)
  if (!isJsonWhitespace(pending)) {
    if (++documents > DEFAULT_MAX_STREAM_DOCUMENTS) {
      throw new RangeError('JSON Lines document limit exceeded')
    }
    try {
      yield nativeLoads(pending, FORMAT_JSON, options)
    } catch (error) {
      throw streamError('JSON Lines', lineOffset, error)
    }
  }
}

function yamlMarker(line, marker) {
  const markerByte = marker === '---' ? 0x2d : 0x2e
  if (
    line.length < 3 ||
    line[0] !== markerByte ||
    line[1] !== markerByte ||
    line[2] !== markerByte
  ) {
    return false
  }
  if (line.length === 3) return true
  const next = line[3]
  return next === 0x20 || next === 0x09
}

function yamlPreambleLine(line) {
  let end = line.length
  if (end !== 0 && line[end - 1] === 0x0d) end -= 1
  let index = 0
  while (index < end && (line[index] === 0x20 || line[index] === 0x09)) {
    index += 1
  }
  if (index === end || line[index] === 0x23) return true
  return index === 0 && line[index] === 0x25
}

function yamlDirectiveLine(line) {
  return line.length !== 0 && line[0] === 0x25
}

async function* yamlLineFrames(stream) {
  let lineParts = []
  let lineLength = 0
  let lineStart = 0
  let total = 0
  let pendingCarriageReturn = false

  for await (const bytes of streamByteChunks(stream)) {
    if (bytes.length === 0) continue
    const chunkStart = total
    total += bytes.length
    let start = 0

    if (pendingCarriageReturn) {
      let terminatorLength = 1
      if (bytes[0] === 0x0a) {
        lineParts.push(bytes.subarray(0, 1))
        lineLength += 1
        terminatorLength = 2
        start = 1
      }
      const framedLine = joinParts(lineParts, lineLength)
      yield {
        framedLine,
        line: framedLine.subarray(0, framedLine.length - terminatorLength),
        lineStart,
      }
      lineParts = []
      lineLength = 0
      lineStart = chunkStart + start
      pendingCarriageReturn = false
    }

    while (start < bytes.length) {
      let delimiter = start
      while (
        delimiter < bytes.length &&
        bytes[delimiter] !== 0x0a &&
        bytes[delimiter] !== 0x0d
      ) {
        delimiter += 1
      }
      if (delimiter === bytes.length) {
        const tail = bytes.subarray(start)
        if (tail.length !== 0) lineParts.push(tail)
        lineLength += tail.length
        break
      }

      let terminatorLength = 1
      if (bytes[delimiter] === 0x0d) {
        if (delimiter + 1 === bytes.length) {
          const part = bytes.subarray(start)
          lineParts.push(part)
          lineLength += part.length
          pendingCarriageReturn = true
          break
        }
        if (bytes[delimiter + 1] === 0x0a) terminatorLength = 2
      }
      const end = delimiter + terminatorLength
      const part = bytes.subarray(start, end)
      lineParts.push(part)
      lineLength += part.length
      const framedLine = joinParts(lineParts, lineLength)
      yield {
        framedLine,
        line: framedLine.subarray(0, framedLine.length - terminatorLength),
        lineStart,
      }
      lineParts = []
      lineLength = 0
      start = end
      lineStart = chunkStart + start
    }
  }

  if (pendingCarriageReturn) {
    const framedLine = joinParts(lineParts, lineLength)
    yield {
      framedLine,
      line: framedLine.subarray(0, framedLine.length - 1),
      lineStart,
    }
  } else if (lineLength !== 0) {
    const framedLine = joinParts(lineParts, lineLength)
    yield { framedLine, line: framedLine, lineStart }
  }
}

async function* yamlDocumentStream(stream, options) {
  let documentParts = []
  let documentLength = 0
  let documentHasContent = false
  let documentHasDirective = false
  let explicitStart = false
  let documentOffset = 0
  let documents = 0

  const decode = (bytes, byteOffset) => {
    documents += 1
    if (documents > DEFAULT_MAX_STREAM_DOCUMENTS) {
      throw new RangeError('YAML document limit exceeded')
    }
    try {
      return nativeLoads(bytes, FORMAT_YAML, options)
    } catch (error) {
      throw streamError('YAML', byteOffset, error)
    }
  }

  for await (const { framedLine, line, lineStart } of yamlLineFrames(stream)) {
    const isStart = yamlMarker(line, '---')
    const isEnd = yamlMarker(line, '...')
    if (isStart && (explicitStart || documentHasContent)) {
      // A following start marker supplies document-end context to YAML block
      // scalars. Decoding the preceding bytes as though they ended at EOF
      // changes the chomping result for an empty `|`, `|+`, `>`, or `>+`
      // scalar. Append an equivalent explicit end marker; its first byte maps
      // to the real following start marker, so parser offsets remain anchored
      // to the original stream.
      yield decode(
        Buffer.concat(
          [...documentParts, YAML_EXPLICIT_END],
          documentLength + YAML_EXPLICIT_END.length,
        ),
        documentOffset,
      )
      documentOffset = lineStart
      documentParts = [framedLine]
      documentLength = framedLine.length
      documentHasContent = false
      documentHasDirective = false
      explicitStart = true
    } else {
      documentParts.push(framedLine)
      documentLength += framedLine.length
      if (isStart) explicitStart = true
      else if (yamlDirectiveLine(line)) documentHasDirective = true
      else if (!isEnd && !yamlPreambleLine(line)) {
        documentHasContent = true
      }
    }
    if (isEnd) {
      if (explicitStart || documentHasContent || documentHasDirective) {
        yield decode(joinParts(documentParts, documentLength), documentOffset)
      }
      documentOffset = lineStart + framedLine.length
      documentParts = []
      documentLength = 0
      documentHasContent = false
      documentHasDirective = false
      explicitStart = false
    }
  }
  if (explicitStart || documentHasContent || documentHasDirective) {
    yield decode(joinParts(documentParts, documentLength), documentOffset)
  }
}

async function dumpAllStream(values, stream, format, options) {
  if (
    values == null ||
    (typeof values[Symbol.iterator] !== 'function' &&
      typeof values[Symbol.asyncIterator] !== 'function')
  ) {
    throw new TypeError('values must be iterable or async iterable')
  }
  let index = 0
  for await (const value of values) {
    if (index >= DEFAULT_MAX_STREAM_DOCUMENTS) {
      throw new RangeError('stream document limit exceeded')
    }
    // JSON Lines frames one logical iterable item per line. Encoding an Array
    // with the core JSON Lines writer would instead treat its children as
    // separate records, so each item is deliberately encoded as plain JSON.
    const itemFormat = format === FORMAT_JSON_LINES ? FORMAT_JSON : format
    const bytes = nativeDumps(value, itemFormat, options)
    if (format === FORMAT_YAML && index !== 0) {
      await writeStream(stream, Buffer.from('---\n'))
    }
    await writeStream(stream, bytes)
    if (bytes.length === 0 || bytes[bytes.length - 1] !== 0x0a) {
      await writeStream(stream, Buffer.from('\n'))
    }
    index += 1
  }
}

function singleDocumentMethods(format) {
  return {
    loads(content, options) {
      return nativeLoads(content, format, options)
    },
    load(source, options) {
      options = checkedOptions(options)
      if (isReadable(source)) {
        return (async () => nativeLoads(await readStream(source), format, options))()
      }
      const input = readSource(source)
      return input.path === null
        ? nativeLoads(input.content, format, options)
        : nativeLoadPath(input.path, format, options)
    },
    dumps(value, options) {
      return nativeDumps(value, format, options)
    },
    dump(value, destination, options) {
      if (!isDestination(destination) && destination !== undefined) {
        options = destination
        destination = undefined
      }
      if (isWritable(destination)) {
        return writeStream(destination, nativeDumps(value, format, options))
      }
      const path = destinationPath(destination)
      if (path !== null) {
        nativeDumpPath(value, path, format, options)
        return undefined
      }
      return writeDestination(destination, nativeDumps(value, format, options))
    },
    async loadStream(stream, options) {
      options = checkedOptions(options)
      return nativeLoads(await readStream(stream), format, options)
    },
    async dumpStream(value, stream, options) {
      await writeStream(stream, nativeDumps(value, format, options))
    },
  }
}

function fixedCodec(format, multiFormat) {
  return Object.freeze({
    ...singleDocumentMethods(format),
    loadsAll(content, options) {
      return nativeLoadsAll(content, multiFormat, options)
    },
    loadAll(source, options) {
      if (isReadable(source)) {
        options = checkedOptions(options)
        return multiFormat === FORMAT_JSON_LINES
          ? jsonLinesStream(source, options)
          : yamlDocumentStream(source, options)
      }
      const input = readSource(source)
      return input.path === null
        ? nativeLoadsAll(input.content, multiFormat, options)
        : nativeLoadAllPath(input.path, multiFormat, options)
    },
    dumpAll(values, destination, options) {
      if (!isDestination(destination) && destination !== undefined) {
        options = destination
        destination = undefined
      }
      if (isWritable(destination)) {
        return dumpAllStream(
          values,
          destination,
          multiFormat,
          checkedOptions(options),
        )
      }
      const path = destinationPath(destination)
      if (path !== null) {
        nativeDumpAllPath(values, path, multiFormat, options)
        return undefined
      }
      return writeDestination(destination, nativeDumpAll(values, multiFormat, options))
    },
    loadAllStream(stream, options) {
      options = checkedOptions(options)
      return multiFormat === FORMAT_JSON_LINES
        ? jsonLinesStream(stream, options)
        : yamlDocumentStream(stream, options)
    },
    async dumpAllStream(values, stream, options) {
      await dumpAllStream(values, stream, multiFormat, checkedOptions(options))
    },
  })
}

const json = fixedCodec(FORMAT_JSON, FORMAT_JSON_LINES)
const toml = Object.freeze(singleDocumentMethods(FORMAT_TOML))
const yaml = fixedCodec(FORMAT_YAML, FORMAT_YAML)

const codec = Object.freeze({
  from(source, options) {
    if (isReadable(source)) return codec.fromStream(source, options)
    options = checkedOptions(options)
    const input = readSource(source)
    if (input.path === null && options.format === undefined) {
      return nativeLoadsInferred(input.content, options)
    }
    const format = formatFor(input.path, options)
    if (input.path !== null) {
      return format === FORMAT_JSON_LINES
        ? nativeLoadAllPath(input.path, format, options)
        : nativeLoadPath(input.path, format, options)
    }
    return format === FORMAT_JSON_LINES
      ? nativeLoadsAll(input.content, format, options)
      : nativeLoads(input.content, format, options)
  },
  into(value, destination, options) {
    if (!isDestination(destination) && destination !== undefined) {
      options = destination
      destination = undefined
    }
    options = checkedOptions(options)
    if (isWritable(destination)) {
      return codec.intoStream(value, destination, options)
    }
    const path = destinationPath(destination)
    const format = formatFor(path, options, FORMAT_JSON)
    if (path !== null) {
      if (format === FORMAT_JSON_LINES) {
        nativeDumpAllPath(value, path, format, options)
      } else {
        nativeDumpPath(value, path, format, options)
      }
      return undefined
    }
    const bytes =
      format === FORMAT_JSON_LINES
        ? nativeDumpAll(value, format, options)
        : nativeDumps(value, format, options)
    return writeDestination(destination, bytes)
  },
  fromStream(stream, options) {
    options = checkedOptions(options)
    if (options.format !== undefined) {
      const format = formatFor(null, options, FORMAT_JSON)
      if (format === FORMAT_JSON_LINES) return jsonLinesStream(stream, options)
      return (async () => nativeLoads(await readStream(stream), format, options))()
    }
    return (async () => {
      const bytes = await readStream(stream)
      return nativeLoadsInferred(bytes, options)
    })()
  },
  async intoStream(value, stream, options) {
    options = checkedOptions(options)
    const format = formatFor(null, options, FORMAT_JSON)
    if (format === FORMAT_JSON_LINES) {
      await dumpAllStream(value, stream, format, options)
    } else {
      await writeStream(stream, nativeDumps(value, format, options))
    }
  },
})

function arrowString(value, NativeType, name) {
  if (value instanceof NativeType) return value
  if (typeof value === 'string') return value
  if (value === null || (typeof value !== 'object' && typeof value !== 'function')) {
    throw new TypeError(`${name}.fromArrow expects a string or Arrow-compatible object`)
  }
  const toString = value.toString
  if (typeof toString !== 'function' || toString === Object.prototype.toString) {
    throw new TypeError(`${name}.fromArrow expects an object with its own textual representation`)
  }
  const text = Reflect.apply(toString, value, [])
  if (typeof text !== 'string') {
    throw new TypeError(`${name}.fromArrow toString() must return a string`)
  }
  return text
}

const dataTypeFromArrowString = NativeDataType.fromArrowString.bind(NativeDataType)
Object.defineProperty(DataType, 'fromArrow', {
  value(value) {
    const inferred = arrowString(value, DataType, 'DataType')
    return inferred instanceof DataType
      ? DataType.from(inferred)
      : dataTypeFromArrowString(inferred)
  },
})

Object.defineProperty(DataType.prototype, Symbol.iterator, {
  configurable: true,
  value: function* fields() {
    for (let index = 0; index < this.length; index += 1) {
      const field = this.at(index)
      if (field !== null) yield field
    }
  },
})

for (const SchemaValue of [DataType, Field]) {
  const nativeShowDiffs = SchemaValue.prototype._showDiffs
  delete SchemaValue.prototype._showDiffs
  Object.defineProperty(SchemaValue.prototype, 'showDiffs', {
    configurable: true,
    value(other, withMetadata = true) {
      return nativeShowDiffs.call(this, other, withMetadata)[Symbol.iterator]()
    },
  })
}

const fieldFromArrowString = NativeField.fromArrowString.bind(NativeField)
Object.defineProperty(Field, 'fromArrow', {
  value(value) {
    const inferred = arrowString(value, Field, 'Field')
    return inferred instanceof Field
      ? Field.from(inferred)
      : fieldFromArrowString(inferred)
  },
})

const fieldUpdate = Field.prototype.update
Field.prototype.update = function update(values) {
  return fieldUpdate.call(this, normalizeMetadata(values))
}

Object.defineProperty(Field.prototype, Symbol.iterator, {
  configurable: true,
  value: function* metadataEntries() {
    for (const { key, value } of this.entries()) yield [key, value]
  },
})

// One protocol's properties are a Map keyed by bare names. The native view
// holds the Field rather than a copy of its metadata, so this loader only adds
// the two language protocols Node-API cannot spell: the built-in collection
// inputs `update` accepts, and iteration.
const { ProtocolMetadata } = binding
const protocolUpdate = ProtocolMetadata.prototype.update
ProtocolMetadata.prototype.update = function update(values) {
  return protocolUpdate.call(this, normalizeMetadata(values))
}

Object.defineProperty(ProtocolMetadata.prototype, Symbol.iterator, {
  configurable: true,
  value: function* propertyEntries() {
    for (const { key, value } of this.entries()) yield [key, value]
  },
})

for (const PathValue of [Uri, Url, Urn]) {
  Object.defineProperty(PathValue.prototype, Symbol.iterator, {
    configurable: true,
    value: function pathSegments() {
      return this.pathSegments[Symbol.iterator]()
    },
  })
}

const { IOBase, Timezone } = binding

// UTC is the one zone that is always registered, so the canonical value is
// materialized once here rather than parsed by every caller.
Object.defineProperty(Timezone, 'UTC', {
  enumerable: true,
  value: Object.freeze(Timezone.fromString('UTC')),
  writable: false,
})

function isPlainObject(value) {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    return false
  }
  const prototype = Object.getPrototypeOf(value)
  return prototype === Object.prototype || prototype === null
}

function partitionEntry(column, value) {
  if (typeof column !== 'string' || typeof value !== 'string') {
    throw new TypeError('partition filters must name a column and a value as strings')
  }
  return { column, value }
}

// The native call takes one array of entries; a Map, an entry array, and a
// plain object are the three ways JavaScript spells the same set of pairs.
function partitionFilters(filters) {
  if (filters === undefined || filters === null) return []
  if (filters instanceof Map) {
    return Array.from(filters, ([column, value]) => partitionEntry(column, value))
  }
  if (Array.isArray(filters)) {
    return filters.map((entry) => {
      if (Array.isArray(entry)) {
        if (entry.length !== 2) {
          throw new TypeError('partition filter tuples must contain two items')
        }
        return partitionEntry(entry[0], entry[1])
      }
      if (entry === null || typeof entry !== 'object') {
        throw new TypeError('partition filters must be objects or tuples')
      }
      return partitionEntry(entry.column, entry.value)
    })
  }
  if (isPlainObject(filters)) {
    return Object.entries(filters).map(([column, value]) =>
      partitionEntry(column, value),
    )
  }
  throw new TypeError('partition filters must be an object, Map, or entry array')
}

const childrenWhere = IOBase.prototype.childrenWhere
Object.defineProperty(IOBase.prototype, 'childrenWhere', {
  configurable: true,
  value(filters, includePrivate) {
    return childrenWhere.call(this, partitionFilters(filters), includePrivate)
  },
})

// Iterating a handle lists its children, which is what iterating a directory
// means everywhere else.
Object.defineProperty(IOBase.prototype, Symbol.iterator, {
  configurable: true,
  value: function children() {
    return this.iterdir()[Symbol.iterator]()
  },
})

// `using` is the scope construct the binding contract binds to open/close, so
// a handle leaving scope publishes what was written through it. That matters
// most on a backend that replaces whole files - any Arrow file system - where
// the bytes are staged until the handle closes. Node exposes the symbol only
// from 20.11, so the binding is conditional rather than assumed.
if (typeof Symbol.dispose === 'symbol') {
  Object.defineProperty(IOBase.prototype, Symbol.dispose, {
    configurable: true,
    value: function dispose() {
      this.close()
    },
  })
}

function pathParts(values) {
  const parts = []
  for (const value of values) {
    if (typeof value === 'string') {
      parts.push(value)
      continue
    }
    if (Array.isArray(value)) {
      parts.push(...pathParts(value))
      continue
    }
    throw new TypeError('path components must be strings')
  }
  return parts
}

// `joinpath` is variadic the way `path.join` is; the native call takes the one
// array those arguments collect into.
for (const Location of [IOBase, Url]) {
  const joinpath = Location.prototype.joinpath
  Object.defineProperty(Location.prototype, 'joinpath', {
    configurable: true,
    value(...others) {
      return joinpath.call(this, pathParts(others))
    },
  })
}

const { BatchReader, RecordOptions } = binding

// The record surface is one shape in both directions: a read returns a
// `BatchReader` and a write consumes one. This installs the Apache Arrow JS
// translation and the argument coercion around it.
const { installRecords } = require('./records.js')
installRecords({
  BatchReader,
  Field,
  IOBase,
  Namespace: binding.Namespace,
  RecordOptions,
  Table: binding.Table,
  Tables: binding.Tables,
})

// A retained snapshot is read with the vocabulary the rest of the package
// already speaks: filters are the pairs `childrenWhere` takes, in any of the
// three ways JavaScript spells a set of them.
const nativeScanAt = binding.Table.prototype.scanAt
Object.defineProperty(binding.Table.prototype, 'scanAt', {
  configurable: true,
  value(snapshotId, filters, schema, options) {
    return nativeScanAt.call(this, snapshotId, partitionFilters(filters), schema, options)
  },
})

// Property updates arrive as whatever spells string pairs - an object, a Map,
// or entries - through the same normalization Field metadata updates use.
const nativeUpdateProperties = binding.Table.prototype.updateProperties
Object.defineProperty(binding.Table.prototype, 'updateProperties', {
  configurable: true,
  value(updates, removes) {
    return nativeUpdateProperties.call(
      this,
      updates == null ? undefined : normalizeMetadata(updates),
      removes,
    )
  },
})

// A schema evolution reads as one chained sentence. The native recorder holds
// the operations and the native commit replays them, so this wrapper only adds
// the chaining Node-API cannot spell: each call returns the builder, and
// `commit()` carries the table the chain started from.
const NativeSchemaUpdate = binding.SchemaUpdate
const nativeCommitSchemaUpdate = binding.Table.prototype._commitSchemaUpdateNative
delete binding.Table.prototype._commitSchemaUpdateNative
Object.defineProperty(binding.Table.prototype, 'updateSchema', {
  configurable: true,
  value() {
    const table = this
    const recorder = new NativeSchemaUpdate()
    const builder = {
      addColumn(parent, field) {
        recorder.addColumn(parent, field)
        return builder
      },
      dropColumn(path) {
        recorder.dropColumn(path)
        return builder
      },
      renameColumn(path, name) {
        recorder.renameColumn(path, name)
        return builder
      },
      updateDoc(path, doc) {
        recorder.updateDoc(path, doc)
        return builder
      },
      makeNullable(path) {
        recorder.makeNullable(path)
        return builder
      },
      updateType(path, dataType) {
        recorder.updateType(path, dataType)
        return builder
      },
      commit() {
        return nativeCommitSchemaUpdate.call(table, recorder)
      },
    }
    return builder
  },
})

// A catalog write takes exactly what a table write takes: `BatchReader.from`
// is the one inference point for anything that names a stream of batches.
for (const name of ['append', 'overwrite']) {
  const native = binding.Catalog.prototype[name]
  Object.defineProperty(binding.Catalog.prototype, name, {
    configurable: true,
    value(tableName, data, options) {
      return native.call(this, tableName, BatchReader.from(data), options)
    },
  })
}

// `yggdryl::iceberg` is a module in the core, so it is one here too: a table
// format sits on top of the record encodings rather than beside them.
const nativeSchemaFromJson = binding.icebergSchemaFromJsonNative
const iceberg = Object.freeze({
  Catalog: binding.Catalog,
  Namespace: binding.Namespace,
  Namespaces: binding.Namespaces,
  Tables: binding.Tables,
  Table: binding.Table,
  IcebergOptions: binding.IcebergOptions,
  PartitionSpec: binding.PartitionSpec,
  DataFile: binding.DataFile,
  ScanPlan: binding.ScanPlan,
  assignFieldIds: binding.icebergAssignFieldIdsNative,
  canPromote: binding.icebergCanPromoteNative,
  // A metadata document is whatever the JSON facade decoded, so a plain object
  // crosses through the one conversion `Value.fromJs` already owns.
  schemaFromJson(name, document) {
    return nativeSchemaFromJson(
      name,
      document instanceof Value ? document : Value.fromJs(document),
    )
  },
  schemaToJson: binding.icebergSchemaToJsonNative,
})

// The Iceberg values are reached through the namespace and nowhere else, so a
// table format has exactly one spelling here, as it does in the core.
for (const name of [
  'Catalog',
  'DataFile',
  'DifferenceIterator',
  'IcebergOptions',
  'JsCatalog',
  'JsDataFile',
  'JsDifferenceIterator',
  'JsIcebergOptions',
  'JsNamespace',
  'JsNamespaces',
  'JsPartitionSpec',
  'JsScanPlan',
  'JsSchemaUpdate',
  'JsTable',
  'JsTables',
  'Namespace',
  'Namespaces',
  'PartitionSpec',
  'ScanPlan',
  'SchemaUpdate',
  'Table',
  'Tables',
  'icebergAssignFieldIdsNative',
  'icebergCanPromoteNative',
  'icebergSchemaFromJsonNative',
  'icebergSchemaToJsonNative',
]) {
  delete binding[name]
}
// A line iterator is a JS iterable and iterator at once: `next()` is native,
// the protocol wrappers live here, so `for...of handle.readLines()` works and
// so does spreading. The native `next` returns null at the end; the protocol
// spells that as done.
if (binding.LineIterator) {
  const nativeNext = binding.LineIterator.prototype.next
  binding.LineIterator.prototype.next = function next() {
    const line = nativeNext.call(this)
    return line === null ? { value: undefined, done: true } : { value: line, done: false }
  }
  Object.defineProperty(binding.LineIterator.prototype, Symbol.iterator, {
    configurable: true,
    value: function lines() {
      return this
    },
  })
}

// The text-line surface. The whole extractor crosses as one native `Value` -
// the same shape a YAML or TOML document parses into - so JavaScript and a
// configuration file configure the identical reader, and the core validates
// both through one conversion. This block is only the coercion: camelCase
// spellings become the document's names, a `DataType` becomes its canonical
// expression, and each constant crosses through the one JavaScript-to-core
// conversion.
const LINE_OPTION_NAMES = new Map([
  ['pattern', 'pattern'],
  ['opening', 'opening'],
  ['header', 'header'],
  ['linesep', 'linesep'],
  ['lineSep', 'linesep'],
  ['lstrip', 'lstrip'],
  ['rstrip', 'rstrip'],
  ['byteSize', 'byte_size'],
  ['byte_size', 'byte_size'],
  ['batchSize', 'batch_size'],
  ['batch_size', 'batch_size'],
  ['timestampCapture', 'timestamp_capture'],
  ['timestamp_capture', 'timestamp_capture'],
  ['timezone', 'timezone'],
  ['captureTypes', 'capture_types'],
  ['capture_types', 'capture_types'],
  ['customFields', 'custom_fields'],
  ['custom_fields', 'custom_fields'],
])

function lineColumnEntries(kind, source) {
  if (source === undefined || source === null) {
    return []
  }
  if (typeof source !== 'object') {
    // A string is iterable and would silently become per-character columns;
    // nothing non-object names columns.
    throw new TypeError(
      `${kind} must be a Map, an iterable of [name, value] pairs, or a plain object`,
    )
  }
  return Symbol.iterator in source ? [...source] : Object.entries(source)
}

// A declared type crosses as the canonical expression `DataType.fromString`
// reads back, so a native `DataType` and the string spelling of one are the
// same declaration.
function lineCaptureTypes(source) {
  const declared = {}
  for (const [name, type] of lineColumnEntries('captureTypes', source)) {
    declared[name] = typeof type === 'string' ? type : String(type)
  }
  return declared
}

function lineCustomFields(source) {
  const constants = {}
  for (const [name, value] of lineColumnEntries('customFields', source)) {
    constants[name] = value
  }
  return constants
}

// `logs: true` is the timestamp opening spelled for the common case; a
// pattern is the third opening. Whichever the caller names wins over the
// default, and naming two is the core's error to report, not this loader's.
function lineOptionsValue(options, pattern) {
  const document = {}
  const source = options ?? {}
  if (typeof source !== 'object') {
    throw new TypeError('line options must be an object')
  }
  for (const [key, value] of Object.entries(source)) {
    if (value === undefined || value === null) {
      continue
    }
    if (key === 'logs') {
      if (value) {
        document.opening = 'timestamp'
      }
      continue
    }
    const name = LINE_OPTION_NAMES.get(key)
    if (name === undefined) {
      throw new TypeError(
        `unknown line option ${JSON.stringify(key)}; expected one of ` +
          `${[...new Set(LINE_OPTION_NAMES.values())].join(', ')}, or logs`,
      )
    }
    if (name === 'capture_types') {
      document.capture_types = lineCaptureTypes(value)
    } else if (name === 'custom_fields') {
      document.custom_fields = lineCustomFields(value)
    } else {
      document[name] = value
    }
  }
  if (pattern !== undefined && pattern !== null) {
    document.pattern = pattern
  }
  if (
    document.batch_size !== undefined &&
    (!Number.isInteger(document.batch_size) || document.batch_size <= 0)
  ) {
    throw new TypeError(`batchSize must be a positive integer, got ${document.batch_size}`)
  }
  if (
    document.byte_size !== undefined &&
    (!Number.isInteger(document.byte_size) || document.byte_size <= 0)
  ) {
    throw new TypeError(`byteSize must be a positive integer, got ${document.byte_size}`)
  }
  return Object.keys(document).length === 0 ? null : Value.fromJs(document)
}

// The first argument is the pattern the common case names positionally, or
// the options object when there is no pattern to name.
function lineArguments(patternOrOptions, options) {
  if (typeof patternOrOptions === 'string') {
    return lineOptionsValue(options, patternOrOptions)
  }
  if (patternOrOptions !== undefined && patternOrOptions !== null && options !== undefined) {
    throw new TypeError('pass the options once: as the first argument or the second, not both')
  }
  return lineOptionsValue(patternOrOptions ?? options, null)
}

// Records stream: the iterable is pulled one record at a time, so neither
// side of the boundary ever holds the whole write.
function linePuller(lines, kind) {
  if (lines === undefined || lines === null) {
    throw new TypeError(`${kind} needs an iterable of records`)
  }
  if (typeof lines === 'string') {
    // A string is iterable and would silently become one record per character.
    throw new TypeError(`${kind} takes an iterable of records, not one string`)
  }
  const iterator = lines[Symbol.iterator]?.()
  if (iterator === undefined) {
    throw new TypeError(`${kind} needs an iterable of records`)
  }
  return function pull() {
    const step = iterator.next()
    return step.done ? null : step.value
  }
}

for (const [name, nativeName] of [
  ['readLines', '_readLinesNative'],
  ['readArrowLines', '_readArrowLinesNative'],
]) {
  const native = IOBase.prototype[nativeName]
  delete IOBase.prototype[nativeName]
  Object.defineProperty(IOBase.prototype, name, {
    configurable: true,
    value: function readText(patternOrOptions, options) {
      return native.call(this, lineArguments(patternOrOptions, options))
    },
  })
}

for (const [name, nativeName] of [
  ['writeLines', '_writeLinesNative'],
  ['appendLines', '_appendLinesNative'],
]) {
  const native = IOBase.prototype[nativeName]
  delete IOBase.prototype[nativeName]
  Object.defineProperty(IOBase.prototype, name, {
    configurable: true,
    value: function writeText(lines, options) {
      return native.call(this, linePuller(lines, name), lineOptionsValue(options, null))
    },
  })
}

// The standalone schema builder: the root Field the projection emits, off a
// pattern alone, so a table can exist before the first log line does.
{
  const nativeSchemaFromPattern = binding._schemaFromPatternNative
  delete binding._schemaFromPatternNative
  binding.schemaFromPattern = function schemaFromPattern(patternOrOptions, options) {
    return nativeSchemaFromPattern(lineArguments(patternOrOptions, options))
  }
}

// The three byte codings, grouped the way the documentation names them. The
// native halves carry a leading underscore so only these namespaces are the
// public spelling.
for (const name of ['gzip', 'zlib', 'zstd']) {
  const loads = binding[`_${name}Loads`]
  const dumps = binding[`_${name}Dumps`]
  delete binding[`_${name}Loads`]
  delete binding[`_${name}Dumps`]
  // zlib carries a second framing of the same algorithm - raw DEFLATE, with
  // no header and no checksum - so its namespace has four halves where the
  // others have two. The pair is named rather than inferred, because raw
  // bytes carry nothing to sniff a framing from.
  const raw = {}
  if (binding[`_${name}LoadsRaw`]) {
    raw.loadsRaw = binding[`_${name}LoadsRaw`]
    raw.dumpsRaw = binding[`_${name}DumpsRaw`]
    delete binding[`_${name}LoadsRaw`]
    delete binding[`_${name}DumpsRaw`]
  }
  binding[name] = Object.freeze({ loads, dumps, ...raw })
}

binding.codec = codec
binding.fields = fields
binding.iceberg = iceberg
binding.json = json
binding.toml = toml
binding.yaml = yaml

module.exports = binding
