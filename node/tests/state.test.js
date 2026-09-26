'use strict'

// `node/src/state.rs`: `State`, the lifecycle-sorted enum a `state` column
// stores as the `int32` code of its member.

const test = require('node:test')
const assert = require('node:assert/strict')

const { DataType, Field, State, fields } = require('yggdryl')

test('State is the core enum, member for member, in code order', () => {
  assert.ok(Object.isFrozen(State))
  const codes = Object.values(State)
  assert.deepEqual(codes, [...codes].sort((left, right) => left - right))
  assert.equal(new Set(codes).size, codes.length)
  assert.equal(State.UNKNOWN, 0)
  assert.equal(State.NEW, 2001)
  assert.equal(State.PARTIALLY_FILLED, 4001)
  assert.equal(State.EXPIRED, 9500)
  assert.equal('_stateMembersNative' in require('yggdryl'), false)
})

test('a state column stores the code and reads back the member name', () => {
  const field = fields.state('state', { nullable: false })
  assert.equal(field.dtype.id, 'state')
  assert.equal(field.dtype.kind, 'enum')
  assert.equal(field.dtype.codeWidth, null)
  for (const given of ['NEW', '0', 'New']) {
    assert.equal(new DataType('state').scalar(given).asJs(), 'NEW', given)
  }
  assert.throws(() => new DataType('state').scalar('not a state'))
  assert.equal(Field.from('state: state').dtype.id, 'state')
})
