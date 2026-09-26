// `node/web/store.js`: one frozen state, one notification per change.
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { createStore } from '../../web/store.js'

test('set patches, freezes and notifies once per change', () => {
  const store = createStore({ symbol: 'ALPHA', at: 1n })
  const seen = []
  const stop = store.subscribe((state, previous) => seen.push([state.at, previous.at]))
  store.set({ at: 2n })
  store.set({ at: 2n }) // unchanged: no notification
  store.set((state) => ({ at: state.at + 1n }))
  assert.deepEqual(seen, [
    [2n, 1n],
    [3n, 2n],
  ])
  assert.ok(Object.isFrozen(store.get()))
  assert.equal(store.get().symbol, 'ALPHA')
  stop()
  store.set({ at: 4n })
  assert.equal(seen.length, 2)
})

test('select fires only when the selected value changes', () => {
  const store = createStore({ a: 1, b: 1 })
  const seen = []
  store.select((state) => state.a, (value) => seen.push(value))
  store.set({ b: 2 })
  store.set({ a: 2 })
  store.set({ a: 2, b: 3 })
  assert.deepEqual(seen, [2])
})

test('refusals name the shape', () => {
  const store = createStore()
  assert.throws(() => store.set(5), /patch object/)
  assert.throws(() => store.subscribe('x'), /listener function/)
})
