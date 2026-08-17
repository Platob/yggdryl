'use strict'

const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { performance } = require('node:perf_hooks')
const { Readable, Writable } = require('node:stream')
const { Value, codec, json, toml, yaml } = require('yggdryl')

const value = {
  trades: Array.from({ length: 1_000 }, (_, index) => ({
    id: BigInt(index),
    price: index / 100,
    symbol: `SYM${index % 20}`,
  })),
}
const exotic = {
  bytes: Buffer.from([0, 1, 127, 255]),
  map: new Map([[{ venue: 'XPAR' }, new Set([1n, 2n])]]),
  nan: Number.NaN,
  regexp: /a\/b/giu,
  typed: new Uint16Array([0, 65535]),
}
// The temporal and decimal boundary is its own cost: each value crosses as
// parts rather than as one number, and a Date is rebuilt on the way back.
const temporal = {
  at: Value.timestamp(1700000000000000n, 'us', 'UTC'),
  date: new Date('2026-08-15T12:30:00.000Z'),
  on: Value.date(19723),
  price: Value.decimal(-1050n, 2),
  sinceMidnight: Value.time(45296000000n, 'us'),
  took: Value.duration(90n, 's'),
}
let deep = { leaf: true }
for (let depth = 0; depth < 24; depth += 1) deep = { nested: deep }

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
      measure(`${name}/string`, encoded.length, 100, () => format.loads(text))
      measure(`${name}/offset_view`, encoded.length, 100, () => format.loads(view))
      measure(`${name}/reader_path`, encoded.length, 100, () => format.load(file))
      measure(`${name}/vector_emit`, encoded.length, 100, () => format.dumps(value))
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
    measure('pivot/from_js', pivotBytes, 100, () => Value.fromJs(value))
    const pivot = Value.fromJs(value)
    measure('pivot/as_js', pivotBytes, 100, () => pivot.asJs())

    const inferred = json.dumps(value)
    measure('generic/content_sniff', inferred.length, 100, () => codec.from(inferred))
    measure('generic/explicit_json', inferred.length, 100, () =>
      codec.from(inferred, { format: 'json' }),
    )
    const inferredToml = toml.dumps(value)
    measure('generic/content_infer_toml', inferredToml.length, 50, () =>
      codec.from(inferredToml),
    )
  } finally {
    fs.rmSync(directory, { force: true, recursive: true })
  }
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
