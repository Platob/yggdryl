'use strict'

// The browser modules the service shares: `node/replay/web.js`.

const assert = require('node:assert/strict')
const test = require('node:test')

const web = require('../../replay/web.js')

test('the diff and the scenario format load once, as the page has them', async () => {
  const first = web()
  assert.equal(web(), first)
  const modules = await first
  for (const name of ['diffStreams', 'diffBooks', 'createScenario', 'insertEvent', 'removeEvent', 'earliestAffected']) {
    assert.equal(typeof modules[name], 'function', name)
  }
  assert.ok(Object.isFrozen(modules))
  const diff = await import('../../web/diff.js')
  assert.equal(modules.diffStreams, diff.diffStreams)
})

test('the diff compares a map served as [key, value] pairs pair by pair, in order', async () => {
  const { diffFacts, sameValue } = await web()
  const pairs = [['31027', 'a'], ['7117', 'b'], ['__proto__', 'p']]
  assert.equal(sameValue(pairs, pairs.map((pair) => [...pair])), true)
  assert.equal(sameValue(pairs, [['31027', 'a'], ['7117', 'b'], ['__proto__', 'q']]), false)
  assert.equal(sameValue(pairs, pairs.slice(0, 2)), false)
  assert.deepEqual(diffFacts({ metadata: pairs }, { metadata: pairs.map((pair) => [...pair]) }), [])
  assert.deepEqual(diffFacts({ metadata: pairs }, { metadata: [] }), [{ name: 'metadata', base: pairs, scenario: [] }])
})
