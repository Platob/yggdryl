'use strict'

// The static enum vocabularies: canonical spellings, unpacked from the core.

const assert = require('node:assert/strict')
const test = require('node:test')

const { DataType, enums } = require('yggdryl')

test('every vocabulary is a frozen non-empty array of strings', () => {
  for (const listing of [
    enums.dataTypeIds,
    enums.dataTypeKinds,
    enums.timeUnits,
    enums.unionModes,
    enums.ioModes,
    enums.codecs,
    enums.ioKinds,
    enums.compatibilitySchemes,
  ]) {
    assert.ok(Array.isArray(listing) && listing.length > 0)
    assert.ok(Object.isFrozen(listing))
    assert.ok(listing.every((value) => typeof value === 'string' && value.length > 0))
  }
  assert.ok(Object.isFrozen(enums))
})

test('the spellings are the ones the parsers accept', () => {
  assert.ok(enums.dataTypeIds.includes('int64'))
  assert.equal(DataType.from('int64').id, 'int64')
  assert.ok(enums.dataTypeKinds.includes('integer'))
  assert.deepEqual([...enums.unionModes], ['sparse', 'dense'])
  assert.deepEqual([...enums.ioModes], ['overwrite', 'append', 'merge', 'readonly', 'random'])
  assert.ok(enums.timeUnits.includes('us'))
  assert.ok(enums.codecs.includes('gzip'))
  assert.ok(enums.ioKinds.includes('file'))
  assert.ok(enums.compatibilitySchemes.includes('arrow'))
})

test('the level scale names its points', () => {
  assert.equal(enums.levels.none, 0)
  assert.equal(enums.levels.fast, 1)
  assert.equal(enums.levels.default, 6)
  assert.equal(enums.levels.best, 9)
})
