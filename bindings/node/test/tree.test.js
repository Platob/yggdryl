// Tests for the yggdryl Node.js extension.
// Build first with `npm run build`, then run `node --test`.

const { test } = require('node:test')
const assert = require('node:assert')
const { Tree } = require('..')

function sample() {
  const tree = new Tree()
  tree.insert('roots/urdr', 1.0)
  tree.insert('roots/verdandi', 2.0)
  tree.insert('roots/skuld', 3.0)
  return tree
}

test('insert and get', () => {
  const tree = sample()
  assert.strictEqual(tree.get('roots/urdr'), 1.0)
  assert.strictEqual(tree.get('roots/missing'), null)
})

test('insert returns previous value', () => {
  const tree = new Tree()
  assert.strictEqual(tree.insert('a', 1.0), null)
  assert.strictEqual(tree.insert('a', 2.0), 1.0)
})

test('count, sum, depth', () => {
  const tree = sample()
  assert.strictEqual(tree.count(), 4)
  assert.strictEqual(tree.sum(), 6.0)
  assert.strictEqual(tree.depth(), 2)
})

test('leaves are sorted by path', () => {
  assert.deepStrictEqual(sample().leaves(), [
    { path: 'roots/skuld', value: 3.0 },
    { path: 'roots/urdr', value: 1.0 },
    { path: 'roots/verdandi', value: 2.0 },
  ])
})

test('remove', () => {
  const tree = sample()
  assert.strictEqual(tree.remove('roots/urdr'), 1.0)
  assert.strictEqual(tree.get('roots/urdr'), null)
  assert.throws(() => tree.remove('roots/urdr'))
})

test('empty path throws', () => {
  const tree = new Tree()
  assert.throws(() => tree.insert('', 1.0))
})

test('arrow IPC round-trip', () => {
  const tree = sample()
  const buf = tree.toArrowIpc()
  assert.ok(Buffer.isBuffer(buf) && buf.length > 0)
  const restored = Tree.fromArrowIpc(buf)
  assert.deepStrictEqual(restored.leaves(), tree.leaves())
})

test('fromArrowIpc rejects garbage', () => {
  assert.throws(() => Tree.fromArrowIpc(Buffer.from('not arrow')))
})
