import { Readable } from 'node:stream'

import {
  Field,
  IOBase,
  Value,
  codec,
  json,
  toml,
  yaml,
  type ValueReadOptions,
} from '..'

const field = new Field('value', 'float16', false)
const jsonValue: Value = json.loads('1.5', { field, value: true })
const yamlValue: Value = yaml.loads('1.5', { field, value: true })
const tomlValue: Value = toml.loads('value = 1.5', {
  field: new Field('row', 'struct<value: float16 not null>', false),
  value: true,
})
const jsonValues: Value[] = json.loadsAll('1.5\n', { field, value: true })
const inferredValue: Value = codec.from('1.5', { field, value: true })
const inferredValues: Value[] = codec.from('1.5\n', {
  field,
  format: 'json_lines',
  value: true,
})

const streamedValue: Promise<Value> = json.load(Readable.from(['1.5']), {
  field,
  value: true,
})
const streamedValues: AsyncIterable<Value> = json.loadAll(
  Readable.from(['1.5\n']),
  { field, value: true },
)

const handle = IOBase.fromBytes(Buffer.from('1.5'))
const readOptions: ValueReadOptions & { value: true } = { field, value: true }
const ioValue: Value = handle.readValue(readOptions)
const naturalValue: number = handle.readValue<number>(field)

// @ts-expect-error the selector is boolean, never a target spelling
json.loads('1.5', { value: 'Value' })
// @ts-expect-error the I/O selector is boolean, never a target spelling
handle.readValue({ value: 'Value' })

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
