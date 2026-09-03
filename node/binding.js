'use strict'

// Keep language protocols here: Node-API owns values and validation, while
// this loader only adapts JavaScript symbols and built-in collection inputs.
const fs = require('node:fs')
const { fileURLToPath, URL } = require('node:url')
const { types: utilTypes } = require('node:util')
const binding = require('./index.js')
const {
  arrow,
  arrowBatchFromIPC,
  arrowBatchIntoIPC,
  arrowScalarFromIPC,
  arrowScalarIntoIPC,
  arrowTableFromIPC,
  arrowTableIntoIPC,
  arrowVectorFromIPC,
  arrowVectorIntoIPC,
} = require('./values.js')

// Native methods taking serde_json::Scalar recurse through caller-owned JS
// objects before Rust can enforce its own schema/value depth limits. Keep that
// FFI traversal on a detached, bounded JSON tree. The 256 raw-container limit
// admits Yggdryl's maximum valid 62-level nested structured wire (depth 254)
// while staying below the depth at which V8/NAPI recursive conversion can
// exhaust the central stack. This is a host-value safety boundary, not another
// schema or structured-value wire implementation.
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
  AsciiDictionary: NativeAsciiDictionary,
  AvroBlock: NativeAvroBlock,
  AvroBlocks: NativeAvroBlocks,
  AvroSchema: NativeAvroSchema,
  DataType: NativeDataType,
  Expression: NativeExpression,
  Field: NativeField,
  MediaType: NativeMediaType,
  MimeType: NativeMimeType,
  PartitionSpec: NativePartitionSpec,
  Uri: NativeUri,
  Url: NativeUrl,
  Urn: NativeUrn,
  Scalar: NativeScalar,
} = binding

// The pivot keeps its private conversion handles inside this loader: `fromJs`
// needs the intrinsic tables assembled below and `asJs` needs the transport
// reader, and neither of those belongs on the published class.
const nativeScalarFromJs = NativeScalar._fromJsNative.bind(NativeScalar)
const nativeScalarFromDecimalParts =
  NativeScalar._fromDecimalPartsNative.bind(NativeScalar)
const nativeScalarFromTemporalParts =
  NativeScalar._fromTemporalPartsNative.bind(NativeScalar)
const nativeScalarAsJs = NativeScalar.prototype._asJsNative
const nativeScalarIter = NativeScalar.prototype._iterNative
const nativeScalarGet = NativeScalar.prototype._getNative
const nativeScalarSet = NativeScalar.prototype._setNative
const nativeScalarRemove = NativeScalar.prototype._removeNative
const nativeScalarArithmetic = Object.freeze({
  add: NativeScalar.prototype._addNative,
  subtract: NativeScalar.prototype._subtractNative,
  multiply: NativeScalar.prototype._multiplyNative,
  divide: NativeScalar.prototype._divideNative,
  remainder: NativeScalar.prototype._remainderNative,
  negate: NativeScalar.prototype._negateNative,
  absolute: NativeScalar.prototype._absoluteNative,
})
const nativeExpressionArithmetic = Object.freeze({
  add: NativeExpression.prototype._addNative,
  subtract: NativeExpression.prototype._subtractNative,
  multiply: NativeExpression.prototype._multiplyNative,
  divide: NativeExpression.prototype._divideNative,
  remainder: NativeExpression.prototype._remainderNative,
})
const nativeAsciiDictionaryIntoEnum =
  NativeAsciiDictionary.prototype._intoEnumNative
const nativeAsciiDictionaryIntoArrowArray =
  NativeAsciiDictionary.prototype._intoArrowArrayIpcNative
const nativeAsciiDictionaryFromArrowArray =
  NativeAsciiDictionary._fromArrowArrayIpcNative.bind(NativeAsciiDictionary)
const nativeScalarFromArrowScalar =
  NativeScalar._fromArrowScalarIpcNative.bind(NativeScalar)
const nativeScalarFromArrowArray =
  NativeScalar._fromArrowArrayIpcNative.bind(NativeScalar)
const nativeScalarFromArrowBatch =
  NativeScalar._fromArrowBatchIpcNative.bind(NativeScalar)
const nativeScalarFromArrowTable =
  NativeScalar._fromArrowTableIpcNative.bind(NativeScalar)
const nativeScalarIntoArrowScalar =
  NativeScalar.prototype._intoArrowScalarIpcNative
const nativeScalarIntoArrowArray = NativeScalar.prototype._intoArrowArrayIpcNative
const nativeScalarIntoArrowBatch =
  NativeScalar.prototype._intoArrowBatchIpcNative
const nativeScalarIntoArrowTable = NativeScalar.prototype._intoArrowTableIpcNative
const nativeAvroSchemaFromValue =
  NativeAvroSchema._fromScalarNative.bind(NativeAvroSchema)
const nativeAvroSchemaFromUtf8 =
  NativeAvroSchema._fromUtf8Native.bind(NativeAvroSchema)
const nativeAvroSchemaFromBytes =
  NativeAvroSchema._fromBytesNative.bind(NativeAvroSchema)
const nativeAvroSchemaIntoValue = NativeAvroSchema.prototype._intoScalarNative
const nativeAvroIntoSingleObject =
  NativeAvroSchema.prototype._intoSingleObjectNative
const nativeAvroFromSingleObject =
  NativeAvroSchema.prototype._fromSingleObjectNative
const nativeAvroBlockRows = NativeAvroBlock.prototype._rowsNative
const nativeAvroBlocksMetadata = NativeAvroBlocks.prototype._metadataNative
const nativePartitionSpecFromValue =
  NativePartitionSpec._fromScalarNative.bind(NativePartitionSpec)
const nativePartitionSpecIntoValue =
  NativePartitionSpec.prototype._intoScalarNative
const nativeAvroBlocksGet = NativeAvroBlocks.prototype.get
const nativeAvroBlocksNext = NativeAvroBlocks.prototype.next
const nativeAvroBlocks = binding.avroBlocksNative
const nativeAvroLoads = binding.avroLoadsNative
const nativeAvroDumps = binding.avroDumpsNative
delete NativeScalar.prototype._asJsNative
delete NativeScalar.prototype._iterNative
delete NativeScalar.prototype._getNative
delete NativeScalar.prototype._setNative
delete NativeScalar.prototype._removeNative
for (const nativeName of [
  '_addNative',
  '_subtractNative',
  '_multiplyNative',
  '_divideNative',
  '_remainderNative',
  '_negateNative',
  '_absoluteNative',
]) {
  delete NativeScalar.prototype[nativeName]
}
for (const nativeName of [
  '_addNative',
  '_subtractNative',
  '_multiplyNative',
  '_divideNative',
  '_remainderNative',
]) {
  delete NativeExpression.prototype[nativeName]
}
delete NativeAsciiDictionary.prototype._intoEnumNative
delete NativeAsciiDictionary.prototype._intoArrowArrayIpcNative
delete NativeScalar.prototype._intoArrowScalarIpcNative
delete NativeScalar.prototype._intoArrowArrayIpcNative
delete NativeScalar.prototype._intoArrowBatchIpcNative
delete NativeScalar.prototype._intoArrowTableIpcNative
delete NativeAvroSchema.prototype._intoScalarNative
delete NativeAvroSchema.prototype._intoSingleObjectNative
delete NativeAvroSchema.prototype._fromSingleObjectNative
delete NativeAvroBlock.prototype._rowsNative
delete NativeAvroBlocks.prototype._metadataNative
delete NativePartitionSpec.prototype._intoScalarNative
delete binding.avroBlocksNative
delete binding.avroLoadsNative
delete binding.avroDumpsNative
delete binding.ScalarIterator

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

const internalDtypeNames = new Set([
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
  internalDtypeNames,
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
const Scalar = publicNativeClass(
  NativeScalar,
  'Scalar',
  new Set([
    '_fromJsNative',
    '_fromDecimalPartsNative',
    '_fromTemporalPartsNative',
    '_fromArrowScalarIpcNative',
    '_fromArrowArrayIpcNative',
    '_fromArrowBatchIpcNative',
    '_fromArrowTableIpcNative',
  ]),
)
const AsciiDictionary = publicNativeClass(
  NativeAsciiDictionary,
  'AsciiDictionary',
  new Set(['_fromArrowArrayIpcNative']),
)
const PartitionSpec = publicNativeClass(
  NativePartitionSpec,
  'PartitionSpec',
  new Set(['_fromScalarNative']),
)

// Avro schemas accept several JavaScript shapes, while the NAPI class itself
// deliberately accepts only the already-normalized native forms. This public
// constructor is the one inference gate and returns that same native class.
function AvroSchema(value, options) {
  if (new.target === undefined) {
    throw new TypeError('Class constructor AvroSchema cannot be invoked without new')
  }
  return avroSchemaFrom(value, true, options)
}
AvroSchema.prototype = NativeAvroSchema.prototype
Object.defineProperty(AvroSchema.prototype, 'constructor', {
  configurable: true,
  value: AvroSchema,
  writable: true,
})

binding.AsciiDictionary = AsciiDictionary
binding.DataType = DataType
binding.Field = Field
binding.MimeType = MimeType
binding.MediaType = MediaType
binding.Uri = Uri
binding.Url = Url
binding.Urn = Urn
binding.Scalar = Scalar
binding.PartitionSpec = PartitionSpec
binding.AvroSchema = AvroSchema
delete binding.AvroBlock
delete binding.AvroBlocks
delete binding.JsAvroSchema

// Structural expression values already produce canonical JSON in Rust. The
// language hook returns the parsed document so JSON.stringify composes them
// naturally instead of double-encoding that native JSON string.
for (const StructuralValue of [binding.Expression, binding.Statement]) {
  Object.defineProperty(StructuralValue.prototype, 'toJSON', {
    configurable: true,
    value() {
      return JSON.parse(this.intoJson())
    },
  })
}

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
  'PUFFIN',
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
const internalDtype = Object.freeze({
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
    return internalDtype.fromFields(collectFields(values))
  },
})

Object.defineProperty(DataType, 'variant', {
  value(values) {
    // The parenthesis disambiguates, exactly as it does in the grammar: a
    // bare call is the self-describing Variant datatype, and a member list
    // keeps building the dense-union sugar.
    if (values === undefined) return internalDtype.variant()
    return internalDtype.variant(collectFields(values))
  },
})

const fields = createFields(DataType, Field, internalDtype)
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
  Scalar.prototype,
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
  const allowed = new Set([
    'environment',
    'field',
    'format',
    'indent',
    'maxDepth',
    'maxDocuments',
    'maxInputBytes',
    'maxNodes',
    'placeholders',
    'scalar',
  ])
  for (const name of Object.keys(options)) {
    if (!allowed.has(name)) throw new TypeError(`unknown codec option ${name}`)
  }
  if (
    options.maxDepth !== undefined &&
    options.maxDepth !== null &&
    (!Number.isSafeInteger(options.maxDepth) ||
      options.maxDepth <= 0 ||
      options.maxDepth > MAX_DEPTH)
  ) {
    throw new RangeError(`maxDepth must be an integer between 1 and ${MAX_DEPTH}`)
  }
  for (const name of ['maxInputBytes', 'maxNodes', 'maxDocuments']) {
    const value = options[name]
    if (
      value !== undefined &&
      value !== null &&
      (!Number.isSafeInteger(value) || value < 0)
    ) {
      throw new RangeError(`${name} must be a non-negative safe integer or null`)
    }
  }
  if (
    options.indent !== undefined &&
    options.indent !== null &&
    options.indent !== '\t' &&
    (!Number.isSafeInteger(options.indent) ||
      options.indent < 0 ||
      options.indent > 255)
  ) {
    throw new RangeError('indent must be an integer between 0 and 255, "\\t", or null')
  }
  if (options.format !== undefined && typeof options.format !== 'string') {
    throw new TypeError('codec format must be a string')
  }
  if (
    options.scalar !== undefined &&
    options.scalar !== null &&
    typeof options.scalar !== 'boolean'
  ) {
    throw new TypeError('scalar must be a boolean')
  }
  const field =
    options.field === undefined || options.field === null
      ? options.field
      : intoField(options.field)
  if (
    options.placeholders !== undefined &&
    options.placeholders !== null &&
    (typeof options.placeholders !== 'object' || Array.isArray(options.placeholders))
  ) {
    throw new TypeError('placeholders must be a mapping of variable names to values')
  }
  if (options.environment !== undefined && typeof options.environment !== 'boolean') {
    throw new TypeError('environment must be a boolean')
  }
  return field === options.field ? options : { ...options, field }
}

function codecLimits(options) {
  const optional = (value) => value === null ? undefined : value
  return {
    maxDepth: optional(options.maxDepth),
    maxInputBytes: optional(options.maxInputBytes),
    maxNodes: optional(options.maxNodes),
    maxDocuments: optional(options.maxDocuments),
  }
}

function codecIndent(options) {
  if (options.indent === undefined) return 'default'
  if (options.indent === null) return 'none'
  if (options.indent === '\t') return 'tabs'
  return `spaces:${options.indent}`
}

function codecInputLimit(options) {
  return options.maxInputBytes === undefined || options.maxInputBytes === null
    ? DEFAULT_MAX_STREAM_BYTES
    : options.maxInputBytes
}

function codecDocumentLimit(options) {
  return options.maxDocuments === undefined || options.maxDocuments === null
    ? DEFAULT_MAX_STREAM_DOCUMENTS
    : options.maxDocuments
}

// The two `{{ }}` switches, crossing as one native `Scalar` and one boolean.
// Both stay `undefined` unless the caller set them, so a plain load is exactly
// the plain load: no substitution pass, and no environment access at all.
function fillingArguments(options) {
  const { placeholders, environment } = options
  return [
    placeholders === undefined || placeholders === null ? null : Scalar.fromJs(placeholders),
    environment === undefined ? null : environment,
  ]
}

// `{{ }}` substitution is a YAML and TOML feature: JSON is a data
// interchange format, and the core refuses the pair for it by name. The
// refusal happens here too, so a JS caller learns at the call site rather
// than from a native error.
function refuseFillingForJson(format, options) {
  if (options.placeholders !== undefined && options.placeholders !== null) {
    throw new TypeError(`placeholders are a yaml/toml feature, not a ${format} one`)
  }
  if (options.environment !== undefined && options.environment !== false) {
    throw new TypeError(`environment resolution is a yaml/toml feature, not a ${format} one`)
  }
}

function loadArguments(format, options) {
  if (format === FORMAT_JSON || format === FORMAT_JSON_LINES) {
    refuseFillingForJson(format, options)
    return [options.field]
  }
  return [options.field, ...fillingArguments(options)]
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

async function* streamByteChunks(stream, maxBytes = DEFAULT_MAX_STREAM_BYTES) {
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
        if (size > maxBytes) {
          throw new RangeError(
            `stream exceeds the ${maxBytes}-byte input limit`,
          )
        }
        yield Buffer.from(text)
      }
      continue
    }
    if (pendingHighSurrogate !== '') {
      size += UTF8_REPLACEMENT_BYTE_LENGTH
      if (size > maxBytes) {
        throw new RangeError(
          `stream exceeds the ${maxBytes}-byte input limit`,
        )
      }
      yield Buffer.from(pendingHighSurrogate)
      pendingHighSurrogate = ''
    }
    const bytes = toBytes(chunk)
    size += bytes.length
    if (size > maxBytes) {
      throw new RangeError(
        `stream exceeds the ${maxBytes}-byte input limit`,
      )
    }
    yield bytes
  }
  if (pendingHighSurrogate !== '') {
    if (size + UTF8_REPLACEMENT_BYTE_LENGTH > maxBytes) {
      throw new RangeError(
        `stream exceeds the ${maxBytes}-byte input limit`,
      )
    }
    yield Buffer.from(pendingHighSurrogate)
  }
}

function sourceFilePath(value) {
  if (!(value instanceof URL)) return null
  if (value.protocol !== 'file:') {
    throw new TypeError('URL codec sources must use the file: protocol')
  }
  return fileURLToPath(value)
}

function readDescriptorBounded(descriptor, maximum = DEFAULT_MAX_STREAM_BYTES) {
  const chunks = []
  let total = 0
  while (true) {
    const capacity = Math.min(
      64 * 1024,
      maximum + 1 - total,
    )
    if (capacity <= 0) {
      throw new RangeError(
        `input exceeds the ${maximum}-byte input limit`,
      )
    }
    const chunk = Buffer.allocUnsafe(capacity)
    const length = fs.readSync(descriptor, chunk, 0, capacity, null)
    if (length === 0) break
    total += length
    if (total > maximum) {
      throw new RangeError(
        `input exceeds the ${maximum}-byte input limit`,
      )
    }
    chunks.push(chunk.subarray(0, length))
  }
  if (chunks.length === 0) return Buffer.alloc(0)
  if (chunks.length === 1) return chunks[0]
  return Buffer.concat(chunks, total)
}

function readSource(source, maximum = DEFAULT_MAX_STREAM_BYTES) {
  if (typeof source === 'number') {
    return { content: readDescriptorBounded(source, maximum), path: null }
  }
  const path = sourceFilePath(source)
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
function fromTypedMarker(value) {
  const decimalKeys = [TRANSPORT_KEY, 'scale', 'value'].sort()
  for (const id of ['decimal128', 'decimal256']) {
    if (markerShape(value, id, decimalKeys)) {
      return nativeScalarFromDecimalParts(id, BigInt(value.value), value.scale)
    }
  }
  const temporalKeys = [TRANSPORT_KEY, 'date', 'unit', 'value', 'zone'].sort()
  const temporalKinds = new Set([
    'date32',
    'date64',
    'time32',
    'time64',
    'timestamp',
    'duration32',
    'duration64',
  ])
  const kind = value[TRANSPORT_KEY]
  if (!temporalKinds.has(kind) || !markerShape(value, kind, temporalKeys)) {
    return undefined
  }
  if (value.date !== null && kind === 'timestamp') {
    return new Date(value.date)
  }
  return nativeScalarFromTemporalParts(
    kind,
    BigInt(value.value),
    value.unit,
    value.zone,
  )
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
  if (markerShape(value, 'record', [TRANSPORT_KEY, 'value'].sort())) {
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
  const typed = fromTypedMarker(value)
  if (typed !== undefined) return typed
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
Object.defineProperty(Scalar, 'fromJs', {
  value(value, options) {
    options = checkedOptions(options)
    return nativeScalarFromJs(
      value,
      options.maxDepth,
      nativeWrapperPrototypes,
      nativeIntrinsics,
    )
  },
})

Object.defineProperties(
  NativeExpression.prototype,
  Object.fromEntries(
    Object.entries(nativeExpressionArithmetic).map(([name, native]) => [
      name,
      {
        configurable: true,
        value(other) {
          const operand =
            other instanceof NativeExpression
              ? other
              : typeof other === 'string'
                ? new NativeExpression(other)
                : NativeExpression.literal(Scalar.fromJs(other))
          return Reflect.apply(native, this, [operand])
        },
      },
    ]),
  ),
)

Object.defineProperty(Scalar.prototype, 'asJs', {
  configurable: true,
  value(options) {
    options = checkedOptions(options)
    return fromTransport(Reflect.apply(nativeScalarAsJs, this, [options.maxDepth]))
  },
})

Object.defineProperty(PartitionSpec, 'fromJSON', {
  configurable: true,
  value(value) {
    return nativePartitionSpecFromValue(Scalar.fromJs(value))
  },
})

Object.defineProperties(PartitionSpec.prototype, {
  intoJSON: {
    configurable: true,
    value() {
      return JSON.parse(
        Reflect.apply(nativePartitionSpecIntoValue, this, []).asJsonUtf8(),
      )
    },
  },
  toJSON: {
    configurable: true,
    value() {
      return this.intoJSON()
    },
  },
})

Object.defineProperties(Scalar, {
  fromArrowScalar: {
    value(value, field) {
      return nativeScalarFromArrowScalar(arrowScalarIntoIPC(value), field)
    },
  },
  fromArrowArray: {
    value(value, field) {
      return nativeScalarFromArrowArray(
        arrowVectorIntoIPC(value, 'Scalar.fromArrowArray input'),
        field,
      )
    },
  },
  fromArrowBatch: {
    value(value, field) {
      return nativeScalarFromArrowBatch(
        arrowBatchIntoIPC(value, 'Scalar.fromArrowBatch input'),
        field,
      )
    },
  },
  fromArrowTable: {
    value(value, field) {
      return nativeScalarFromArrowTable(
        arrowTableIntoIPC(value, 'Scalar.fromArrowTable input'),
        field,
      )
    },
  },
})

Object.defineProperties(Scalar.prototype, {
  ...Object.fromEntries(
    Object.entries(nativeScalarArithmetic).map(([name, native]) => [
      name,
      {
        configurable: true,
        value(other) {
          if (name === 'negate' || name === 'absolute') {
            return Reflect.apply(native, this, [])
          }
          const operand = other instanceof NativeScalar ? other : Scalar.fromJs(other)
          return Reflect.apply(native, this, [operand])
        },
      },
    ]),
  ),
  [Symbol.iterator]: {
    configurable: true,
    value() {
      return Reflect.apply(nativeScalarIter, this, [])
    },
  },
  get: {
    configurable: true,
    value(key) {
      if (this.kind === 'sequence') {
        if (!Number.isSafeInteger(key) || key < 0) {
          throw new TypeError('sequence keys must be non-negative safe integers')
        }
        return this.at(key)
      }
      if (this.kind === 'record' && typeof key !== 'string') {
        throw new TypeError('record field names must be strings')
      }
      const nativeKey = key instanceof Scalar ? key : Scalar.fromJs(key)
      return Reflect.apply(nativeScalarGet, this, [nativeKey])
    },
  },
  has: {
    configurable: true,
    value(key) {
      return this.get(key) !== null
    },
  },
  set: {
    configurable: true,
    value(key, value) {
      if (this.kind === 'record' && typeof key !== 'string') {
        throw new TypeError('record field names must be strings')
      }
      const nativeKey = key instanceof Scalar ? key : Scalar.fromJs(key)
      const nativeItem = value instanceof Scalar ? value : Scalar.fromJs(value)
      return Reflect.apply(nativeScalarSet, this, [nativeKey, nativeItem])
    },
  },
  remove: {
    configurable: true,
    value(key) {
      if (typeof key !== 'string') {
        throw new TypeError('remove requires a string key')
      }
      return Reflect.apply(nativeScalarRemove, this, [Scalar.fromJs(key)])
    },
  },
  intoArrowScalar: {
    value(field) {
      return arrowScalarFromIPC(
        Reflect.apply(nativeScalarIntoArrowScalar, this, [field]),
        'Scalar.intoArrowScalar output',
      )
    },
  },
  intoArrowArray: {
    value(field) {
      return arrowVectorFromIPC(
        Reflect.apply(nativeScalarIntoArrowArray, this, [field]),
        'Scalar.intoArrowArray output',
      )
    },
  },
  intoArrowBatch: {
    value(field) {
      return arrowBatchFromIPC(
        Reflect.apply(nativeScalarIntoArrowBatch, this, [field]),
        'Scalar.intoArrowBatch output',
      )
    },
  },
  intoArrowTable: {
    value(field) {
      return arrowTableFromIPC(
        Reflect.apply(nativeScalarIntoArrowTable, this, [field]),
        'Scalar.intoArrowTable output',
      )
    },
  },
  toJSON: {
    value() {
      return JSON.parse(this.asJsonUtf8())
    },
  },
})

// The generated enum is name to code only: a numeric reverse map would
// collide with values that render as digits, and `values()` already answers
// the code to value direction.
Object.defineProperties(AsciiDictionary.prototype, {
  intoEnum: {
    configurable: true,
    value(name) {
      if (typeof name !== 'string' || name.trim() === '') {
        throw new TypeError('AsciiDictionary.intoEnum needs a non-empty enum name')
      }
      const members = Reflect.apply(nativeAsciiDictionaryIntoEnum, this, [])
      Object.defineProperty(members, Symbol.toStringTag, { value: name })
      return Object.freeze(members)
    },
  },
  intoArrowArray: {
    configurable: true,
    value(values) {
      return arrowVectorFromIPC(
        Reflect.apply(nativeAsciiDictionaryIntoArrowArray, this, [values]),
        'AsciiDictionary.intoArrowArray output',
      )
    },
  },
})

Object.defineProperties(AsciiDictionary, {
  fromArrowArray: {
    configurable: true,
    value(value) {
      return nativeAsciiDictionaryFromArrowArray(
        arrowVectorIntoIPC(value, 'AsciiDictionary.fromArrowArray input'),
      )
    },
  },
})

const AVRO_DECODE_OPTION_NAMES = new Set([
  'maxDepth',
  'maxInputBytes',
  'maxNodes',
  'readerSchema',
])

function checkedAvroDecodeOptions(options, allowReaderSchema) {
  if (options == null) return { limits: undefined, readerSchema: undefined }
  if (
    typeof options !== 'object' ||
    Array.isArray(options) ||
    ![Object.prototype, null].includes(Object.getPrototypeOf(options))
  ) {
    throw new TypeError('Avro decode options must be a plain object')
  }
  for (const key of Reflect.ownKeys(options)) {
    if (typeof key !== 'string' || !AVRO_DECODE_OPTION_NAMES.has(key)) {
      throw new TypeError(`unknown Avro decode option ${String(key)}`)
    }
  }
  const hasReaderSchema = Object.hasOwn(options, 'readerSchema')
  if (!allowReaderSchema && hasReaderSchema) {
    throw new TypeError('readerSchema is only valid for Avro container and block decoding')
  }
  const limits = {}
  for (const name of ['maxDepth', 'maxInputBytes', 'maxNodes']) {
    if (!Object.hasOwn(options, name)) continue
    const value = options[name]
    if (value === undefined || value === null) continue
    if (!Number.isSafeInteger(value) || value < 0) {
      throw new RangeError(`${name} must be a non-negative safe integer`)
    }
    limits[name] = value
  }
  return {
    limits: Object.keys(limits).length === 0 ? undefined : limits,
    readerSchema:
      !hasReaderSchema || options.readerSchema == null
        ? undefined
        : avroSchemaFrom(options.readerSchema, false, limits),
  }
}

function avroSchemaFrom(value, clone = false, options) {
  const { limits } = checkedAvroDecodeOptions(options, false)
  if (value instanceof NativeAvroSchema) {
    return clone ? value.clone() : value
  }
  if (typeof value === 'string') return nativeAvroSchemaFromUtf8(value, limits)
  if (
    Buffer.isBuffer(value) ||
    utilTypes.isAnyArrayBuffer(value) ||
    ArrayBuffer.isView(value)
  ) {
    return nativeAvroSchemaFromBytes(toBytes(value), limits)
  }
  return nativeAvroSchemaFromValue(Scalar.fromJs(value), limits)
}

function avroBytes(value, label) {
  if (typeof value === 'string') {
    throw new TypeError(`${label} must be bytes, not a string`)
  }
  try {
    return toBytes(value)
  } catch (cause) {
    throw new TypeError(
      `${label} must be a Buffer, ArrayBuffer, SharedArrayBuffer, or array-buffer view`,
      { cause },
    )
  }
}

Object.defineProperty(AvroSchema, 'from', {
  value(value, options) {
    return avroSchemaFrom(value, true, options)
  },
})

Object.defineProperties(AvroSchema.prototype, {
  intoJSON: {
    value() {
      return Reflect.apply(nativeAvroSchemaIntoValue, this, []).asJs()
    },
  },
  intoCanonicalForm: {
    value() {
      return this.canonicalForm
    },
  },
  intoSingleObject: {
    value(value) {
      return Reflect.apply(nativeAvroIntoSingleObject, this, [Scalar.fromJs(value)])
    },
  },
  fromSingleObject: {
    value(input, options) {
      const { limits } = checkedAvroDecodeOptions(options, false)
      return Reflect.apply(nativeAvroFromSingleObject, this, [
        avroBytes(input, 'Avro single-object input'),
        limits,
      ]).asJs()
    },
  },
  toJSON: {
    value() {
      return this.intoJSON()
    },
  },
  toString: {
    value() {
      return this.canonicalForm
    },
  },
})

Object.defineProperty(NativeAvroBlock.prototype, 'rows', {
  value() {
    return Reflect.apply(nativeAvroBlockRows, this, []).asJs()
  },
})

Object.defineProperty(NativeAvroBlocks.prototype, 'metadata', {
  get() {
    return Object.fromEntries(
      Reflect.apply(nativeAvroBlocksMetadata, this, []).map(({ key, value }) => [key, value]),
    )
  },
})

NativeAvroBlocks.prototype.get = function get(key) {
  const value = Reflect.apply(nativeAvroBlocksGet, this, [key])
  return value === null ? undefined : value
}

NativeAvroBlocks.prototype.next = function next() {
  const block = Reflect.apply(nativeAvroBlocksNext, this, [])
  return block === null
    ? { value: undefined, done: true }
    : { value: block, done: false }
}
Object.defineProperty(NativeAvroBlocks.prototype, Symbol.iterator, {
  value() {
    return this
  },
})

function avroRows(values) {
  if (
    values === null ||
    typeof values === 'string' ||
    typeof values?.[Symbol.iterator] !== 'function'
  ) {
    throw new TypeError('Avro rows must be a non-string iterable')
  }
  return Array.from(values)
}

function avroMetadata(values) {
  if (values === undefined || values === null) return []
  // The accepted shapes are the same string-pair shapes field metadata uses;
  // only the label differs at this language boundary.
  try {
    return normalizeMetadata(values)
  } catch (cause) {
    throw new TypeError(
      String(cause.message).replaceAll('field metadata', 'Avro metadata'),
      { cause },
    )
  }
}

function avroLoads(input, options) {
  const { limits, readerSchema } = checkedAvroDecodeOptions(options, true)
  const decoded = nativeAvroLoads(
    avroBytes(input, 'Avro container input'),
    readerSchema,
    limits,
  ).asJs()
  const metadata = Object.fromEntries(
    decoded.metadata.map(({ key, value }) => [key, value]),
  )
  return {
    metadata,
    rows: decoded.rows,
    // The container already decoded this schema into the shared natural
    // Scalar shape. A primitive schema is therefore `"long"`, not the JSON
    // source text `'"long"'`; keep it on the native-Scalar path so those two
    // intentionally distinct inputs cannot be confused.
    schema: nativeAvroSchemaFromValue(Scalar.fromJs(decoded.schema)),
  }
}

function avroBlocks(input, options) {
  const { limits, readerSchema } = checkedAvroDecodeOptions(options, true)
  return nativeAvroBlocks(
    avroBytes(input, 'Avro container input'),
    readerSchema,
    limits,
  )
}

function avroDumps(rows, schema, metadata) {
  return nativeAvroDumps(
    avroSchemaFrom(schema),
    Scalar.fromJs(avroRows(rows)),
    avroMetadata(metadata),
  )
}

const avro = Object.freeze({
  Schema: AvroSchema,
  blocks: avroBlocks,
  dumps: avroDumps,
  dumpsSingle(value, schema) {
    return avroSchemaFrom(schema).intoSingleObject(value)
  },
  loads: avroLoads,
  loadsSingle(input, schema, options) {
    return avroSchemaFrom(schema, false, options).fromSingleObject(input, options)
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
  const decoded = nativeFormat(format).loads(
    toNativeContent(content),
    codecLimits(options),
    ...loadArguments(format, options),
    options.scalar === true,
  )
  return options.scalar === true ? decoded : fromTransport(decoded)
}

function nativeLoadsInferred(content, options) {
  options = checkedOptions(options)
  const decoded = nativeCodec.loadsInferred(
    toNativeContent(content),
    codecLimits(options),
    options.field,
    options.scalar === true,
  )
  return options.scalar === true ? decoded : fromTransport(decoded)
}

function nativeLoadsAll(content, format, options) {
  options = checkedOptions(options)
  if (format === FORMAT_JSON || format === FORMAT_JSON_LINES) {
    refuseFillingForJson(format, options)
  }
  const decoded = nativeFormat(format).loadsAll(
    toNativeContent(content),
    codecLimits(options),
    options.field,
    options.scalar === true,
  )
  return options.scalar === true
    ? decoded
    : decoded.map((value) => fromTransport(value))
}

function nativeLoadPath(path, format, options) {
  options = checkedOptions(options)
  const decoded = nativeFormat(format).loadPath(
    path,
    codecLimits(options),
    ...loadArguments(format, options),
    options.scalar === true,
  )
  return options.scalar === true ? decoded : fromTransport(decoded)
}

function nativeLoadAllPath(path, format, options) {
  options = checkedOptions(options)
  if (format === FORMAT_JSON || format === FORMAT_JSON_LINES) {
    refuseFillingForJson(format, options)
  }
  const decoded = nativeFormat(format).loadAllPath(
    path,
    codecLimits(options),
    options.field,
    options.scalar === true,
  )
  return options.scalar === true
    ? decoded
    : decoded.map((value) => fromTransport(value))
}

function nativeDumps(value, format, options) {
  options = checkedOptions(options)
  return nativeFormat(format).dumps(
    value,
    options.maxDepth,
    codecIndent(options),
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
    codecIndent(options),
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
    codecIndent(options),
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
    codecIndent(options),
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

async function readStream(stream, options) {
  const chunks = []
  let size = 0
  const maximum = codecInputLimit(options)
  for await (const bytes of streamByteChunks(stream, maximum)) {
    size += bytes.length
    if (size > maximum) {
      throw new RangeError(
        `stream exceeds the ${maximum}-byte input limit`,
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
  // A JavaScript async iterable cannot implement Rust's synchronous `Read`,
  // and the core deliberately has no second push-decoder abstraction. This
  // boundary therefore finds complete lines (including the four JSON framing
  // whitespace bytes used to skip blank rows), while the native codec remains
  // authoritative for syntax, values, and byte positions within each row. The
  // stream tests compare this path with buffered core decoding across
  // arbitrary chunk boundaries.
  // Refused here, before the first chunk: deferring to the per-line load
  // would surface the misconfiguration as a mid-stream parse error - or not
  // at all on an empty stream.
  options = checkedOptions(options)
  refuseFillingForJson(FORMAT_JSON_LINES, options)
  const maximum = codecInputLimit(options)
  const documentLimit = codecDocumentLimit(options)
  let lineParts = []
  let lineLength = 0
  let lineOffset = 0
  let total = 0
  let documents = 0
  for await (const bytes of streamByteChunks(stream, maximum)) {
    const chunkOffset = total
    total += bytes.length
    if (total > maximum) {
      throw new RangeError(
        `stream exceeds the ${maximum}-byte input limit`,
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
        if (documents > documentLimit) {
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
    if (++documents > documentLimit) {
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

async function* yamlLineFrames(stream, maximum) {
  let lineParts = []
  let lineLength = 0
  let lineStart = 0
  let total = 0
  let pendingCarriageReturn = false

  for await (const bytes of streamByteChunks(stream, maximum)) {
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
  // As above, only the async-protocol framing stays in JavaScript. Each
  // completed document is parsed by the native YAML codec, and parity tests
  // cover marker spelling, directives, line endings, block scalars, and
  // malformed preambles against the buffered core reader.
  let documentParts = []
  let documentLength = 0
  let documentHasContent = false
  let documentHasDirective = false
  let explicitStart = false
  let documentOffset = 0
  let documents = 0
  options = checkedOptions(options)
  const documentLimit = codecDocumentLimit(options)

  const decode = (bytes, byteOffset) => {
    documents += 1
    if (documents > documentLimit) {
      throw new RangeError('YAML document limit exceeded')
    }
    try {
      return nativeLoads(bytes, FORMAT_YAML, options)
    } catch (error) {
      throw streamError('YAML', byteOffset, error)
    }
  }

  for await (const { framedLine, line, lineStart } of yamlLineFrames(
    stream,
    codecInputLimit(options),
  )) {
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
  // Pulling an async iterable and honoring Node/WHATWG backpressure are
  // language-runtime duties. Scalar conversion and document encoding still
  // cross the native codec once per item; holding all values merely to call a
  // buffered core collection writer would violate this method's stream shape.
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
        return (async () => nativeLoads(await readStream(source, options), format, options))()
      }
      const input = readSource(source, codecInputLimit(options))
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
      return nativeLoads(await readStream(stream, options), format, options)
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
      options = checkedOptions(options)
      if (isReadable(source)) {
        return multiFormat === FORMAT_JSON_LINES
          ? jsonLinesStream(source, options)
          : yamlDocumentStream(source, options)
      }
      const input = readSource(source, codecInputLimit(options))
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
    const input = readSource(source, codecInputLimit(options))
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
      return (async () => nativeLoads(await readStream(stream, options), format, options))()
    }
    return (async () => {
      const bytes = await readStream(stream, options)
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

const dtypeFromArrowString = NativeDataType.fromArrowString.bind(NativeDataType)
Object.defineProperty(DataType, 'fromArrow', {
  value(value) {
    const inferred = arrowString(value, DataType, 'DataType')
    return inferred instanceof DataType
      ? DataType.from(inferred)
      : dtypeFromArrowString(inferred)
  },
})

Object.defineProperty(DataType.prototype, Symbol.iterator, {
  configurable: true,
  value: function* fields() {
    for (let index = 0; index < this.length; index += 1) {
      const field = this.getFieldAt(index)
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
const nativeIOReadValue = IOBase.prototype._readScalarNative
const nativeIOWriteValue = IOBase.prototype._writeScalarNative
const nativeIOBuffered = IOBase.prototype._bufferedNative
const nativeIOReadRangeBytes = IOBase.prototype.readRangeBytes
const nativeIOAppendBytes = IOBase.prototype.appendBytes
const nativeIOReadRangeText = IOBase.prototype._readRangeTextNative
delete IOBase.prototype._readRangeTextNative
delete IOBase.prototype._readScalarNative
delete IOBase.prototype._writeScalarNative
delete IOBase.prototype._bufferedNative

function checkedBufferedOptions(options) {
  if (options === undefined || options === null) return {}
  if (!isPlainObject(options)) {
    throw new TypeError('buffered options must be an object')
  }
  for (const name of ['pageSize', 'maxBytes', 'ttlMs']) {
    const value = options[name]
    if (
      value !== undefined &&
      value !== null &&
      (!Number.isSafeInteger(value) || value < 0)
    ) {
      throw new RangeError(`${name} must be a non-negative safe integer or null`)
    }
  }
  return options
}

function rangeReadArguments(options) {
  if (options === undefined || options === null) return false
  if (!isPlainObject(options)) {
    throw new TypeError('readRange options must be an object')
  }
  for (const name of Object.keys(options)) {
    if (name !== 'text') {
      throw new TypeError(`unknown readRange option ${name}`)
    }
  }
  if (
    options.text !== undefined &&
    options.text !== null &&
    typeof options.text !== 'boolean'
  ) {
    throw new TypeError('text must be a boolean')
  }
  return options.text === true
}

// The one place a JavaScript byte source becomes the `Uint8Array` the native
// append borrows. A string is UTF-8, matching `writeText`; any typed array or
// `DataView` is read over its own window, which is what Python's `memoryview`
// reaches on that side.
function appendedBytes(data) {
  if (typeof data === 'string') return Buffer.from(data, 'utf8')
  if (data instanceof Uint8Array) return data
  if (ArrayBuffer.isView(data)) {
    return new Uint8Array(data.buffer, data.byteOffset, data.byteLength)
  }
  if (data instanceof ArrayBuffer) return new Uint8Array(data)
  throw new TypeError(
    'appended data must be a typed array, DataView, ArrayBuffer, or string',
  )
}

function readScalarArguments(input) {
  if (isPlainObject(input)) {
    for (const name of Object.keys(input)) {
      if (name !== 'field' && name !== 'scalar') {
        throw new TypeError(`unknown readScalar option ${name}`)
      }
    }
    if (
      input.scalar !== undefined &&
      input.scalar !== null &&
      typeof input.scalar !== 'boolean'
    ) {
      throw new TypeError('scalar must be a boolean')
    }
    return {
      field:
        input.field === undefined || input.field === null
          ? null
          : intoField(input.field),
      nativeScalar: input.scalar === true,
    }
  }
  return {
    field: input === undefined || input === null ? null : intoField(input),
    nativeScalar: false,
  }
}

Object.defineProperties(IOBase.prototype, {
  buffered: {
    configurable: true,
    value(options) {
      options = checkedBufferedOptions(options)
      nativeIOBuffered.call(
        this,
        options.pageSize,
        options.maxBytes,
        options.ttlMs,
      )
      return this
    },
  },
  readRange: {
    configurable: true,
    value(offset, length, options) {
      // Settled before the read, so a rejected option costs no fetch. Text
      // decodes in the core, which refuses an invalid sequence rather than
      // substituting replacement characters the way `toString` would.
      return rangeReadArguments(options)
        ? nativeIOReadRangeText.call(this, offset, length)
        : nativeIOReadRangeBytes.call(this, offset, length)
    },
  },
  append: {
    configurable: true,
    value(data) {
      return nativeIOAppendBytes.call(this, appendedBytes(data))
    },
  },
  readScalar: {
    configurable: true,
    value(options) {
      const { field, nativeScalar } = readScalarArguments(options)
      const decoded = nativeIOReadValue.call(this, field, nativeScalar)
      return nativeScalar ? decoded : fromTransport(decoded)
    },
  },
  writeScalar: {
    configurable: true,
    value(value) {
      return nativeIOWriteValue.call(
        this,
        value instanceof Scalar ? value : Scalar.fromJs(value),
      )
    },
  },
})

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

// A generic URI is not necessarily a filesystem URL, but its path component
// has the same core join semantics. JavaScript has no object `/` operator, so
// `joinPath` is the explicit variadic spelling at this boundary.
const uriJoinPath = Uri.prototype.joinPath
Object.defineProperty(Uri.prototype, 'joinPath', {
  configurable: true,
  value(...others) {
    return uriJoinPath.call(this, pathParts(others))
  },
})

const { BatchReader, RecordOptions } = binding
// Runtime-only state used by the async record bridge. It is intentionally not
// a fourth public write abstraction.
delete binding.ArrowWriteSession

// The declared root metadata takes the same inputs `Field.prototype.update`
// does: entries, a plain object, a Map, or a Field's own pairs.
const recordMetadata = Object.getOwnPropertyDescriptor(RecordOptions.prototype, 'metadata')
Object.defineProperty(RecordOptions.prototype, 'metadata', {
  configurable: true,
  enumerable: recordMetadata.enumerable,
  get: recordMetadata.get,
  set(values) {
    recordMetadata.set.call(this, normalizeMetadata(values))
  },
})
const recordWithMetadata = RecordOptions.prototype.withMetadata
RecordOptions.prototype.withMetadata = function withMetadata(values) {
  return recordWithMetadata.call(this, normalizeMetadata(values))
}

// The record surface is one shape in both directions: a read returns a
// `BatchReader` and a write consumes one. This installs the Apache Arrow JS
// translation and the argument coercion around it.
const { installRecords } = require('./records.js')
const { icebergBatchReader, intoField } = installRecords({
  BatchReader,
  Field,
  IOBase,
  RecordOptions,
  Table: binding.Table,
  Tables: binding.Tables,
})
binding.intoField = intoField

// A Statement binds once in the native core. JavaScript widens only the two
// inputs it can spell more conveniently: any FieldLike becomes one native
// Field, and an ordinary parameter object becomes the shared Scalar::Record.
// Batch execution then keeps the caller's Arrow holder: readers stay lazy,
// tables remain tables, and a one-batch sort remains a RecordBatch operation.
const NativeStatement = binding.Statement
const BoundStatement = binding.BoundStatement
const nativeStatementBind = NativeStatement.prototype._bindNative
const nativeStatementProjectReader =
  BoundStatement.prototype._projectArrowReaderNative
const nativeStatementProjectBatch =
  BoundStatement.prototype._projectArrowBatchNative
const nativeStatementSortBatch =
  BoundStatement.prototype._sortArrowBatchNative
for (const [owner, name, method] of [
  [NativeStatement.prototype, '_bindNative', nativeStatementBind],
  [BoundStatement.prototype, '_projectArrowReaderNative', nativeStatementProjectReader],
  [BoundStatement.prototype, '_projectArrowBatchNative', nativeStatementProjectBatch],
  [BoundStatement.prototype, '_sortArrowBatchNative', nativeStatementSortBatch],
]) {
  if (typeof method !== 'function') {
    throw new TypeError(`native binding is missing ${owner.constructor.name}.${name}`)
  }
  delete owner[name]
}

Object.defineProperty(NativeStatement.prototype, 'bind', {
  configurable: true,
  value(schema, parameters) {
    const supplied =
      parameters === undefined || parameters === null
        ? undefined
        : parameters instanceof Scalar
          ? parameters
          : Scalar.fromJs(parameters)
    return nativeStatementBind.call(this, intoField(schema), supplied)
  },
})

Object.defineProperties(BoundStatement.prototype, {
  projectArrowReader: {
    configurable: true,
    value(reader) {
      if (!(reader instanceof BatchReader)) {
        throw new TypeError(
          'reader must be a native BatchReader; use projectArrow for inferred Arrow input',
        )
      }
      return nativeStatementProjectReader.call(this, reader)
    },
  },
  projectArrowBatch: {
    configurable: true,
    value(batch) {
      const reader = BatchReader.fromIpc(
        arrowBatchIntoIPC(batch, 'BoundStatement.projectArrowBatch input'),
      )
      return arrowBatchFromIPC(
        nativeStatementProjectBatch.call(this, reader).intoIpc(),
        'BoundStatement.projectArrowBatch output',
      )
    },
  },
  projectArrowTable: {
    configurable: true,
    value(table) {
      const reader = BatchReader.fromIpc(
        arrowTableIntoIPC(table, 'BoundStatement.projectArrowTable input'),
      )
      return nativeStatementProjectReader.call(this, reader).intoTable()
    },
  },
  projectArrow: {
    configurable: true,
    value(value) {
      if (value instanceof BatchReader) return this.projectArrowReader(value)
      const runtime = arrow()
      if (runtime.isArrowRecordBatch(value)) {
        return this.projectArrowBatch(value)
      }
      if (runtime.isArrowTable(value)) return this.projectArrowTable(value)
      throw new TypeError(
        'value must be a native BatchReader, Apache Arrow RecordBatch, or Apache Arrow Table',
      )
    },
  },
  sortArrowBatch: {
    configurable: true,
    value(batch) {
      const reader = BatchReader.fromIpc(
        arrowBatchIntoIPC(batch, 'BoundStatement.sortArrowBatch input'),
      )
      return arrowBatchFromIPC(
        nativeStatementSortBatch.call(this, reader).intoIpc(),
        'BoundStatement.sortArrowBatch output',
      )
    },
  },
})

// Parquet's DTOs cross through the shared Scalar transport, never through a
// second JavaScript metadata model.
const nativeIOReadParquetStatistics =
  IOBase.prototype._readParquetStatisticsNative
const nativeIOReadParquetGeospatialStatistics =
  IOBase.prototype._readParquetGeospatialStatisticsNative
delete IOBase.prototype._readParquetStatisticsNative
delete IOBase.prototype._readParquetGeospatialStatisticsNative
Object.defineProperties(IOBase.prototype, {
  readParquetStatistics: {
    configurable: true,
    value() {
      return fromTransport(nativeIOReadParquetStatistics.call(this))
    },
  },
  readParquetGeospatialStatistics: {
    configurable: true,
    value(column) {
      return fromTransport(
        nativeIOReadParquetGeospatialStatistics.call(this, column),
      )
    },
  },
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
// Every level of the hierarchy that carries properties widens the same way.
for (const Owner of [binding.Table, binding.Catalog, binding.Namespace]) {
  const nativeUpdateProperties = Owner.prototype.updateProperties
  Object.defineProperty(Owner.prototype, 'updateProperties', {
    configurable: true,
    value(updates, removes) {
      return nativeUpdateProperties.call(
        this,
        updates == null ? undefined : normalizeMetadata(updates),
        removes,
      )
    },
  })
}

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
      updateType(path, dtype) {
        recorder.updateType(path, dtype)
        return builder
      },
      commit() {
        return nativeCommitSchemaUpdate.call(table, recorder)
      },
    }
    return builder
  },
})

// A catalog write takes exactly what a table write takes, through the one
// Iceberg inference point: Arrow shapes and IPC bytes name their own reader,
// and rows are typed by the table the write lands in - which a create-on-write
// does not have yet, so the rows declare it.
for (const name of ['append', 'overwrite']) {
  const native = binding.Catalog.prototype[name]
  Object.defineProperty(binding.Catalog.prototype, name, {
    configurable: true,
    value(tableName, data, options) {
      const tables = this.tables
      const stored = tables.has(tableName) ? tables.get(tableName) : null
      return native.call(this, tableName, icebergBatchReader(stored, data), options)
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
  Compaction: binding.Compaction,
  IcebergOptions: binding.IcebergOptions,
  ManifestFile: binding.ManifestFile,
  PartitionField: binding.PartitionField,
  PartitionSpec: binding.PartitionSpec,
  DataFile: binding.DataFile,
  ScanPlan: binding.ScanPlan,
  Snapshot: binding.Snapshot,
  SnapshotRef: binding.SnapshotRef,
  assignFieldIds: binding.icebergAssignFieldIdsNative,
  canPromote: binding.icebergCanPromoteNative,
  // A metadata document is whatever the JSON facade decoded, so a plain object
  // crosses through the one conversion `Scalar.fromJs` already owns.
  schemaFromJson(name, document) {
    return nativeSchemaFromJson(
      name,
      document instanceof Scalar ? document : Scalar.fromJs(document),
    )
  },
  schemaIntoJson: binding.icebergSchemaIntoJsonNative,
})

// The Iceberg values are reached through the namespace and nowhere else, so a
// table format has exactly one spelling here, as it does in the core.
for (const name of [
  'Catalog',
  'Compaction',
  'DataFile',
  'DifferenceIterator',
  'IcebergOptions',
  'JsCatalog',
  'JsCompaction',
  'JsDataFile',
  'JsDifferenceIterator',
  'JsIcebergOptions',
  'JsManifestFile',
  'JsNamespace',
  'JsNamespaces',
  'JsPartitionSpec',
  'JsPartitionField',
  'JsScanPlan',
  'JsSchemaUpdate',
  'JsSnapshot',
  'JsSnapshotRef',
  'JsTable',
  'JsTables',
  'Namespace',
  'Namespaces',
  'ManifestFile',
  'PartitionField',
  'PartitionSpec',
  'ScanPlan',
  'SchemaUpdate',
  'Snapshot',
  'SnapshotRef',
  'Table',
  'Tables',
  'icebergAssignFieldIdsNative',
  'icebergCanPromoteNative',
  'icebergSchemaFromJsonNative',
  'icebergSchemaIntoJsonNative',
]) {
  delete binding[name]
}
// A byte stream is a JS iterable and iterator at once. The native half returns
// one Buffer or null and retains its source reference; this wrapper only maps
// that result onto the standard protocol, without prefetching or collecting.
if (binding.ByteIterator) {
  const nativeNext = binding.ByteIterator.prototype.next
  binding.ByteIterator.prototype.next = function next() {
    const bytes = nativeNext.call(this)
    return bytes === null ? { value: undefined, done: true } : { value: bytes, done: false }
  }
  Object.defineProperty(binding.ByteIterator.prototype, Symbol.iterator, {
    configurable: true,
    value: function bytes() {
      return this
    },
  })
}

// A listing is a JS iterable and iterator at once: `next()` is native, the
// protocol wrappers live here, so `for...of handle.ls(true)` works and so does
// spreading. Nothing is collected on the way across - the walk runs as the
// iterator is drained. The native `next` returns null at the end; the protocol
// spells that as done.
if (binding.Listing) {
  const nativeNext = binding.Listing.prototype.next
  binding.Listing.prototype.next = function next() {
    const entry = nativeNext.call(this)
    return entry === null ? { value: undefined, done: true } : { value: entry, done: false }
  }
  Object.defineProperty(binding.Listing.prototype, Symbol.iterator, {
    configurable: true,
    value: function entries() {
      return this
    },
  })
}

// The catalog collections are Map-like: `keys()` is a lazy native iterator,
// and the loader supplies the protocol plus `values()` and `entries()` so
// `for...of namespaces.keys()` and spreading both work. `values` and
// `entries` open each named resource through `get`, one at a time.
if (binding.IcebergNames) {
  const nativeNext = binding.IcebergNames.prototype.next
  binding.IcebergNames.prototype.next = function next() {
    const name = nativeNext.call(this)
    return name === null ? { value: undefined, done: true } : { value: name, done: false }
  }
  Object.defineProperty(binding.IcebergNames.prototype, Symbol.iterator, {
    configurable: true,
    value: function names() {
      return this
    },
  })
}
// The classes live under the frozen `iceberg` namespace by the time this
// runs - the raw exports above were deleted - so the wiring reaches them
// through the namespace, which holds the same prototypes.
for (const collection of [iceberg.Namespaces, iceberg.Tables]) {
  if (!collection) continue
  Object.defineProperty(collection.prototype, Symbol.iterator, {
    configurable: true,
    value: function keys() {
      return this.keys()[Symbol.iterator]()
    },
  })
  Object.defineProperty(collection.prototype, 'values', {
    configurable: true,
    value: function* values() {
      for (const name of this.keys()) yield this.get(name)
    },
  })
  Object.defineProperty(collection.prototype, 'entries', {
    configurable: true,
    value: function* entries() {
      for (const name of this.keys()) yield [name, this.get(name)]
    },
  })
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

// The text-line surface. The whole extractor crosses as one native `Scalar` -
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
  ['batchRowSize', 'batch_row_size'],
  ['batch_row_size', 'batch_row_size'],
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
    document.batch_row_size !== undefined &&
    (!Number.isInteger(document.batch_row_size) || document.batch_row_size <= 0)
  ) {
    throw new TypeError(
      `batchRowSize must be a positive integer, got ${document.batch_row_size}`,
    )
  }
  if (
    document.byte_size !== undefined &&
    (!Number.isInteger(document.byte_size) || document.byte_size <= 0)
  ) {
    throw new TypeError(`byteSize must be a positive integer, got ${document.byte_size}`)
  }
  return Object.keys(document).length === 0 ? null : Scalar.fromJs(document)
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
  const nativeFieldFromPattern = binding._fieldFromPatternNative
  delete binding._fieldFromPatternNative
  binding.fieldFromPattern = function fieldFromPattern(patternOrOptions, options) {
    return nativeFieldFromPattern(lineArguments(patternOrOptions, options))
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
binding.avro = avro
binding.fields = fields
binding.iceberg = iceberg
binding.json = json
binding.toml = toml
binding.yaml = yaml

// The core's static enum vocabularies, frozen: pure enums cross the boundary
// as strings by convention, and this is the enumeration of what those strings
// can be, unpacked from one native listing so it can never drift.
{
  const listing = binding._enumValuesNative()
  const levels = binding._levelValuesNative()
  delete binding._enumValuesNative
  delete binding._levelValuesNative
  binding.enums = Object.freeze({
    dataTypeIds: Object.freeze(listing.dataTypeIds),
    dataTypeKinds: Object.freeze(listing.dataTypeKinds),
    timeUnits: Object.freeze(listing.timeUnits),
    unionModes: Object.freeze(listing.unionModes),
    ioModes: Object.freeze(listing.ioModes),
    codecs: Object.freeze(listing.codecs),
    ioKinds: Object.freeze(listing.ioKinds),
    compatibilitySchemes: Object.freeze(listing.compatibilitySchemes),
    levels: Object.freeze(levels),
  })
}

module.exports = binding
