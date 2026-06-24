// Parity tests: BytesIO and LocalPath behave the same for the `stream` flag and
// `open`. The same assertions run against both handles.
// Build first with `npm run build`, then run `node --test`.

const { test } = require('node:test')
const assert = require('node:assert')
const os = require('node:os')
const path = require('node:path')
const { BytesIO, LocalPath } = require('..')

let counter = 0
const kinds = [
  { id: 'bytesio', make: (data) => new BytesIO(Buffer.from(data)) },
  {
    id: 'localpath',
    make: (data) => {
      const p = path.join(os.tmpdir(), `yggdryl_parity_${process.pid}_${counter++}`)
      new LocalPath(p).write(Buffer.from(data))
      return new LocalPath(p)
    },
  },
]

for (const { id, make } of kinds) {
  test(`stream parity (${id})`, () => {
    // Streaming (the default): each read advances the cursor.
    let io = make('abcdef')
    assert.strictEqual(io.stream, true)
    assert.deepStrictEqual(io.read(3), Buffer.from('abc'))
    assert.strictEqual(io.tell(), 3)
    assert.deepStrictEqual(io.read(), Buffer.from('def'))
    assert.strictEqual(io.tell(), 6)

    // Non-streaming: the cursor stays put, so reads repeat.
    io = make('abcdef')
    io.stream = false
    assert.strictEqual(io.stream, false)
    assert.deepStrictEqual(io.read(3), Buffer.from('abc'))
    assert.deepStrictEqual(io.read(3), Buffer.from('abc'))
    assert.strictEqual(io.tell(), 0)
  })

  test(`close parity (${id})`, () => {
    // close() is a no-op (the Node analog of Python's `with` cleanup).
    const io = make('abcdef')
    io.close()
    const child = make('abcdef').open('r')
    child.close()
  })

  test(`open parity (${id})`, () => {
    // Read open keeps the bytes, carries the stream flag and the mode.
    let child = make('abcdef').open('r', false)
    assert.strictEqual(child.mode, 'r')
    assert.strictEqual(child.stream, false)
    assert.deepStrictEqual(child.getValue(), Buffer.from('abcdef'))

    // Write open truncates.
    child = make('abcdef').open('w')
    assert.strictEqual(child.mode, 'w')
    assert.deepStrictEqual(child.getValue(), Buffer.from(''))

    // Append open keeps the bytes with the cursor at the end.
    child = make('abcdef').open('a')
    assert.strictEqual(child.mode, 'a')
    assert.strictEqual(child.tell(), 6)
    assert.deepStrictEqual(child.getValue(), Buffer.from('abcdef'))
  })
}
