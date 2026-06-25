// Tests for Io.json() — parsing a handle's bytes as JSON in Rust.
// Build first with `npm run build`, then run `node --test`.

const { test } = require('node:test')
const assert = require('node:assert')
const os = require('node:os')
const path = require('node:path')
const { BytesIO, LocalPath } = require('..')

test('bytesio json', () => {
  const io = new BytesIO(Buffer.from('{"n":42,"xs":[1,2],"ok":true,"nil":null}'))
  assert.deepStrictEqual(io.json(), { n: 42, xs: [1, 2], ok: true, nil: null })
})

test('localpath json', () => {
  const p = path.join(os.tmpdir(), `yggdryl_json_${process.pid}.json`)
  new LocalPath(p).write(Buffer.from('{"a":[1,2,3],"b":"x"}'))
  assert.deepStrictEqual(new LocalPath(p).json(), { a: [1, 2, 3], b: 'x' })
})

test('invalid json throws', () => {
  assert.throws(() => new BytesIO(Buffer.from('{not json')).json())
})
