'use strict'

// `node/src/state.rs`: `State`, the lifecycle-sorted enum a `state` column
// stores as the `uint16` code of its member.

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
  // A thing acknowledged and awaiting its next step works; an approval ends
  // one.
  assert.equal(State.PENDING_VERIFICATION, 4006)
  assert.equal(State.PENDING_ALLOCATION, 4007)
  assert.equal(State.PENDING_APPROVAL, 4008)
  assert.equal(State.APPROVED, 8013)
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

test('a spelling reads by the words it is made of', () => {
  for (const [spelling, state] of [
    ['order fill', 'FILLED'],
    ['Part-Filled', 'PARTIALLY_FILLED'],
    ['partial fill order', 'PARTIALLY_FILLED'],
    ['pending cxl', 'PENDING_CANCEL'],
  ]) {
    assert.equal(new DataType('state').scalar(spelling).asJs(), state, spelling)
  }
  assert.throws(() => new DataType('state').scalar('partially frobnicated'), /partially frobnicated/)
})
