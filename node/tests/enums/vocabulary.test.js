'use strict'

// The static enum vocabularies: canonical spellings, unpacked from the core.

const assert = require('node:assert/strict')
const test = require('node:test')

const { DataType, Field, enums } = require('yggdryl')

// Walking the export rather than a list written beside it: a vocabulary added
// to the core reaches this test by being exported, which is the only way it
// could reach a caller either.
const vocabularies = Object.entries(enums).filter(([name]) => name !== 'levels')

test('every vocabulary is a frozen non-empty array of strings', () => {
  assert.ok(vocabularies.length > 0)
  for (const [name, listing] of vocabularies) {
    assert.ok(Array.isArray(listing) && listing.length > 0, name)
    assert.ok(Object.isFrozen(listing), name)
    assert.ok(
      listing.every((value) => typeof value === 'string' && value.length > 0),
      name,
    )
  }
  assert.ok(Object.isFrozen(enums))
})

test('the vocabularies the core lists are the ones exported', () => {
  assert.deepEqual(
    vocabularies.map(([name]) => name).sort(),
    [
      'charsets',
      'codecs',
      'compatibilitySchemes',
      'dataTypeIds',
      'dataTypeKinds',
      'digestAlgorithms',
      'eventColumns',
      'ioKinds',
      'ioModes',
      'marketColumns',
      'marketKinds',
      'marketViews',
      'mdUpdateActions',
      'operationColumns',
      'pythonKinds',
      'timeUnits',
      'unionModes',
    ],
  )
})

test('the spellings are the ones the parsers accept', () => {
  assert.ok(enums.dataTypeIds.includes('int64'))
  assert.equal(DataType.from('int64').id, 'int64')
  assert.ok(enums.dataTypeKinds.includes('integer'))
  assert.deepEqual([...enums.unionModes], ['sparse', 'dense'])
  assert.deepEqual([...enums.ioModes], ['overwrite', 'append', 'merge', 'readonly', 'random'])
  assert.ok(enums.timeUnits.includes('us'))
  assert.ok(enums.codecs.includes('gzip'))
  assert.ok(enums.charsets.includes('windows-1252'))
  assert.ok(enums.ioKinds.includes('file'))
  assert.ok(enums.digestAlgorithms.includes('xxh3-128'))
  assert.ok(enums.compatibilitySchemes.includes('arrow'))
  // The typed `PYTHON:` vocabulary is Rust and Python only, so this listing is
  // the whole of what JavaScript knows about the spellings `PYTHON:kind` takes.
  assert.ok(enums.pythonKinds.includes('dataclass'))
  assert.throws(
    () => new Field('Quote', 'int64', false).set('PYTHON:kind', 'record'),
    /PYTHON:kind/,
  )
})

test('the level scale names its points', () => {
  assert.equal(enums.levels.none, 0)
  assert.equal(enums.levels.fast, 1)
  assert.equal(enums.levels.default, 6)
  assert.equal(enums.levels.best, 9)
})
