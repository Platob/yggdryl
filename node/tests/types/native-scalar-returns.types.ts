import { Readable } from 'node:stream'

import {
  Field,
  IOBase,
  Scalar,
  codec,
  json,
  toml,
  yaml,
  type ScalarReadOptions,
} from '../..'

const field = new Field('value', 'float16', false)
const jsonValue: Scalar = json.loads('1.5', { field, scalar: true })
const yamlValue: Scalar = yaml.loads('1.5', { field, scalar: true })
const tomlValue: Scalar = toml.loads('value = 1.5', {
  field: new Field('row', 'struct<value: float16 not null>', false),
  scalar: true,
})
const jsonValues: Scalar[] = json.loadsAll('1.5\n', { field, scalar: true })
const inferredValue: Scalar = codec.from('1.5', { field, scalar: true })
const inferredValues: Scalar[] = codec.from('1.5\n', {
  field,
  format: 'json_lines',
  scalar: true,
})

const streamedValue: Promise<Scalar> = json.load(Readable.from(['1.5']), {
  field,
  scalar: true,
})
const streamedValues: AsyncIterable<Scalar> = json.loadAll(
  Readable.from(['1.5\n']),
  { field, scalar: true },
)

const handle = IOBase.fromBytes(Buffer.from('1.5'))
const readOptions: ScalarReadOptions & { scalar: true } = { field, scalar: true }
const ioValue: Scalar = handle.readScalar(readOptions)
const naturalValue: number = handle.readScalar<number>(field)

// @ts-expect-error the selector is boolean, never a target spelling
json.loads('1.5', { scalar: 'Scalar' })
// @ts-expect-error the I/O selector is boolean, never a target spelling
handle.readScalar({ scalar: 'Scalar' })

void [
  jsonValue,
  yamlValue,
  tomlValue,
  jsonValues,
  inferredValue,
  inferredValues,
  streamedValue,
  streamedValues,
  ioValue,
  naturalValue,
]
