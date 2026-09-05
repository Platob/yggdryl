'use strict'

const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { performance } = require('node:perf_hooks')
const { Readable, Writable } = require('node:stream')
const { pathToFileURL } = require('node:url')
const arrow = require('apache-arrow')
const { Field, Scalar, avro, codec, json, toml, yaml } = require('yggdryl')

const value = {
  trades: Array.from({ length: 1_000 }, (_, index) => ({
    id: BigInt(index),
    price: index / 100,
    symbol: `SYM${index % 20}`,
  })),
}
const exotic = {
  bytes: Buffer.from([0, 1, 127, 255]),
  map: new Map([['venues', new Set(['XPAR', 'XNAS'])]]),
  ratio: Math.PI,
  regexp: /a\/b/giu,
  typed: new Uint16Array([0, 65535]),
}
// The temporal and decimal boundary is its own cost: each value crosses as
// parts rather than as one number, and a Date is rebuilt on the way back.
const temporal = {
  at: Scalar.datetime(1700000000000000n, 'us', 'UTC'),
  date: new Date('2026-08-15T12:30:00.000Z'),
  on: Scalar.date(19723),
  price: Scalar.decimal(-(2n ** 160n), 2),
  sinceMidnight: Scalar.time(45296000000n, 'us'),
  took: Scalar.duration(90, 's'),
}
let deep = { leaf: true }
for (let depth = 0; depth < 24; depth += 1) deep = { nested: deep }

// Raw Avro fixtures are built once. Timed operations cross only the public
// JavaScript/native boundary; setup and fixture generation stay outside every
// measured loop.
const avroSchemaDocument = {
  type: 'record',
  name: 'trade',
  fields: [
    { name: 'id', type: 'long' },
    { name: 'price', type: 'double' },
    { name: 'symbol', type: 'string' },
  ],
}
const avroReaderDocument = {
  type: 'record',
  name: 'trade',
  fields: [
    { name: 'id', type: 'long' },
    { name: 'ticker', aliases: ['symbol'], type: 'string' },
  ],
}
const avroSchema = new avro.Schema(avroSchemaDocument)
const avroReader = new avro.Schema(avroReaderDocument)
const avroRows = value.trades.map((row, index) => ({
  ...row,
  // Every value stays physically double; an integral JavaScript number is
  // deliberately inferred as an integer by the shared Scalar pivot.
  price: index + 0.5,
}))
const avroContainer = avro.dumps(avroRows, avroSchema, { source: 'benchmark' })
const avroSingle = avro.dumpsSingle(avroRows[0], avroSchema)
const avroSchemaBytes = Buffer.byteLength(JSON.stringify(avroSchemaDocument))
const avroBlock = avro.blocks(avroContainer).next().value
const avroResolvedBlock = avro.blocks(avroContainer, {
  readerSchema: avroReader,
}).next().value

function measure(name, bytes, iterations, operation) {
  for (let index = 0; index < 5; index += 1) operation()
  const started = performance.now()
  for (let index = 0; index < iterations; index += 1) operation()
  const elapsed = performance.now() - started
  const throughput = (bytes * iterations) / (elapsed / 1_000) / (1024 * 1024)
  console.log(
    `${name}: ${(elapsed / iterations).toFixed(3)} ms/op, ${throughput.toFixed(1)} MiB/s`,
  )
}

async function measureAsync(name, bytes, iterations, operation) {
  for (let index = 0; index < 2; index += 1) await operation()
  const started = performance.now()
  for (let index = 0; index < iterations; index += 1) await operation()
  const elapsed = performance.now() - started
  const throughput = (bytes * iterations) / (elapsed / 1_000) / (1024 * 1024)
  console.log(
    `${name}: ${(elapsed / iterations).toFixed(3)} ms/op, ${throughput.toFixed(1)} MiB/s`,
  )
}

async function main() {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-node-bench-'))
  try {
    for (const [name, format, extension] of [
      ['json', json, 'json'],
      ['toml', toml, 'toml'],
      ['yaml', yaml, 'yaml'],
    ]) {
      const encoded = format.dumps(value)
      const text = encoded.toString('utf8')
      const padded = Buffer.concat([Buffer.from('xx'), encoded, Buffer.from('yy')])
      const view = new DataView(
        padded.buffer,
        padded.byteOffset + 2,
        encoded.byteLength,
      )
      const file = path.join(directory, `value.${extension}`)
      fs.writeFileSync(file, encoded)

      // Keep these group spellings stable so before/after runs remain comparable.
      measure(`${name}/slice`, encoded.length, 100, () => format.loads(encoded))
      measure(`${name}/exact_value`, encoded.length, 100, () =>
        format.loads(encoded, { scalar: true }),
      )
      measure(`${name}/string`, encoded.length, 100, () => format.loads(text))
      measure(`${name}/offset_view`, encoded.length, 100, () => format.loads(view))
      measure(`${name}/reader_path`, encoded.length, 100, () =>
        format.load(pathToFileURL(file)),
      )
      measure(`${name}/vector_emit`, encoded.length, 100, () => format.dumps(value))
      measure(`${name}/formatted_emit`, encoded.length, 100, () =>
        format.dumps(value, { indent: 2 }),
      )
      measure(`${name}/explicit_limits`, encoded.length, 100, () =>
        format.loads(encoded, {
          maxDepth: 48,
          maxInputBytes: encoded.length,
          maxNodes: 10_000,
          maxDocuments: 1,
        }),
      )
      measure(`${name}/writer_path`, encoded.length, 100, () => format.dump(value, file))
      await measureAsync(`${name}/reader_stream`, encoded.length, 25, () =>
        format.load(Readable.from([encoded])),
      )
      await measureAsync(`${name}/writer_stream`, encoded.length, 25, () =>
        format.dump(
          value,
          new Writable({ write(_chunk, _encoding, done) { done() } }),
        ),
      )

      const exoticEncoded = format.dumps(exotic)
      measure(`${name}/exotic_values`, exoticEncoded.length, 50, () =>
        format.loads(exoticEncoded),
      )
      const temporalEncoded = format.dumps(temporal)
      measure(`${name}/temporal_values`, temporalEncoded.length, 50, () =>
        format.loads(temporalEncoded),
      )
      const deepEncoded = format.dumps(deep)
      measure(`${name}/deep_nested`, deepEncoded.length, 50, () =>
        format.loads(deepEncoded),
      )
    }

    // The pivot is the conversion every load and dump crosses, measured on its
    // own against the same payload so a codec number can be read against it.
    const pivotBytes = json.dumps(value).length
    measure('pivot/from_js', pivotBytes, 100, () => Scalar.fromJs(value))
    const pivot = Scalar.fromJs(value)
    measure('pivot/as_js', pivotBytes, 100, () => pivot.asJs())
    measure('pivot/equals_native', pivotBytes, 1_000, () => pivot.equals(pivot))
    measure('pivot/compare_native', pivotBytes, 1_000, () => pivot.compare(pivot))
    measure('pivot/stable_hash', pivotBytes, 1_000, () => pivot.stableHash())
    measure('pivot/clone_native', pivotBytes, 1_000, () => pivot.clone())
    measure('pivot/id', 1, 10_000, () => pivot.id)
    measure('pivot/family', 1, 10_000, () => pivot.family)
    const enumScalar = Scalar.fromEnum('io_mode', 'append')
    measure('pivot/enum_from', 1, 10_000, () => Scalar.fromEnum('io_mode', 'append'))
    measure('pivot/enum_kind', 1, 10_000, () => enumScalar.enumKind)
    measure('pivot/enum_value', 1, 10_000, () => enumScalar.enumValue)
    measure('pivot/enum_ordinal', 1, 10_000, () => enumScalar.enumOrdinal)
    measure('pivot/float_family', 8, 10_000, () => Scalar.float(1.5, 32))
    measure('pivot/decimal_family', 16, 10_000, () => Scalar.decimal(123456n, 2))
    measure('pivot/date_family', 8, 10_000, () => Scalar.date(19723))
    measure('pivot/time_family', 8, 10_000, () => Scalar.time(45296, 's'))
    measure('pivot/datetime_family', 8, 10_000, () =>
      Scalar.datetime(1700000000000n, 'ms', 'UTC'),
    )
    measure('pivot/duration_family', 8, 10_000, () => Scalar.duration(90, 's'))
    const traversed = Scalar.fromJs({ trades: value.trades })
    measure('pivot/native_path', 1, 10_000, () => traversed.path('trades.500.price'))
    measure('pivot/native_get', 1, 10_000, () => traversed.get('trades').at(500))
    measure('pivot/persistent_set', 1, 2_000, () => traversed.set('version', 1))
    const scalarValue = Scalar.fromJs(42)
    const arrayValue = Scalar.fromJs([1, null])
    const rowValue = Scalar.fromJs([{ id: 1, symbol: 'AAPL' }])
    measure('pivot/infer_scalar_field', 1, 1_000, () => scalarValue.intoField())
    measure('pivot/infer_array_field', 2, 1_000, () => arrayValue.intoArrayField())
    measure('pivot/infer_struct_field', 2, 1_000, () => rowValue.intoStructField())
    const arithmeticLeft = Scalar.decimal(123456n, 2)
    const arithmeticRight = Scalar.decimal(25n, 2)
    measure('pivot/add_native', 1, 10_000, () => arithmeticLeft.add(arithmeticRight))
    measure('pivot/add_inferred_js', 1, 10_000, () => scalarValue.add(1))
    measure('pivot/subtract_native', 1, 10_000, () =>
      arithmeticLeft.subtract(arithmeticRight),
    )
    measure('pivot/multiply_native', 1, 10_000, () =>
      arithmeticLeft.multiply(arithmeticRight),
    )
    measure('pivot/divide_native', 1, 10_000, () =>
      arithmeticLeft.divide(arithmeticRight),
    )
    measure('pivot/remainder_native', 1, 10_000, () =>
      arithmeticLeft.remainder(arithmeticRight),
    )
    measure('pivot/negate_native', 1, 10_000, () => arithmeticLeft.negate())
    const arithmeticNegative = arithmeticLeft.negate()
    measure('pivot/absolute_native', 1, 10_000, () => arithmeticNegative.absolute())

    const decimalField = new Field('price', 'decimal256(40,2)', false)
    const decimalJson = Buffer.from('"123456789012345678901234567890.50"')
    measure('json/field_d256', decimalJson.length, 100, () =>
      json.loads(decimalJson, { field: decimalField }),
    )

    const arrowVector = arrow.vectorFromArray(Int32Array.from({ length: 1_000 }, (_, i) => i))
    const arrowValues = Scalar.fromArrowArray(arrowVector)
    measure('arrow/from_array_ipc', arrowVector.length * 4, 50, () =>
      Scalar.fromArrowArray(arrowVector),
    )
    measure('arrow/into_array_ipc', arrowVector.length * 4, 50, () =>
      arrowValues.intoArrowArray(),
    )

    const inferred = json.dumps(value)
    measure('generic/content_sniff', inferred.length, 100, () => codec.from(inferred))
    measure('generic/explicit_json', inferred.length, 100, () =>
      codec.from(inferred, { format: 'json' }),
    )
    const inferredToml = toml.dumps(value)
    measure('generic/content_infer_toml', inferredToml.length, 50, () =>
      codec.from(inferredToml),
    )

    measure('avro/schema_parse', avroSchemaBytes, 100, () =>
      new avro.Schema(avroSchemaDocument),
    )
    measure('avro/schema_canonical', avroSchemaBytes, 100, () =>
      avroSchema.intoCanonicalForm(),
    )
    measure('avro/schema_equals', avroSchemaBytes, 1_000, () =>
      avroSchema.equals(avroSchema),
    )
    measure('avro/schema_compare', avroSchemaBytes, 1_000, () =>
      avroSchema.compare(avroSchema),
    )
    measure('avro/schema_stable_hash', avroSchemaBytes, 1_000, () =>
      avroSchema.stableHash(),
    )
    measure('avro/schema_clone', avroSchemaBytes, 1_000, () => avroSchema.clone())
    measure('avro/container_decode', avroContainer.length, 50, () =>
      avro.loads(avroContainer),
    )
    measure('avro/container_resolve', avroContainer.length, 50, () =>
      avro.loads(avroContainer, { readerSchema: avroReader }),
    )
    // Measure the lazy contract at its useful boundary: header parse plus
    // time to the first still-compressed block, without decoding its rows.
    measure('avro/blocks_first', avroContainer.length, 100, () =>
      avro.blocks(avroContainer).next(),
    )
    measure('avro/block_decode', avroContainer.length, 50, () =>
      avroBlock.rows(),
    )
    measure('avro/block_resolve', avroContainer.length, 50, () =>
      avroResolvedBlock.rows(),
    )
    measure('avro/container_encode', avroContainer.length, 50, () =>
      avro.dumps(avroRows, avroSchema, { source: 'benchmark' }),
    )
    measure('avro/single_decode', avroSingle.length, 100, () =>
      avro.loadsSingle(avroSingle, avroSchema),
    )
    measure('avro/single_encode', avroSingle.length, 100, () =>
      avro.dumpsSingle(avroRows[0], avroSchema),
    )
  } finally {
    fs.rmSync(directory, { force: true, recursive: true })
  }
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
