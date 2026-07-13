'use strict'

// The Node extension mirrors the core `version()` — the minimal end-to-end example.
const test = require('node:test')
const assert = require('node:assert')
const yggdryl = require('..')

test('version is a non-empty string', () => {
  const v = yggdryl.version()
  assert.strictEqual(typeof v, 'string')
  assert.notStrictEqual(v, '')
})

test('version matches the package 0.1.x line', () => {
  assert.ok(yggdryl.version().startsWith('0.1.'))
})
