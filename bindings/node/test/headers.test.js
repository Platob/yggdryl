'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { Headers } = yggdryl.io

test('the io namespace exposes Headers', () => {
  assert.equal(typeof Headers, 'function')
})

test('map-like, and insert replaces', () => {
  const h = new Headers()
  assert.equal(h.size, 0)
  assert.equal(h.get('x'), null) // absent -> undefined/null
  h.insert('a', '1')
  h.insert('b', '2')
  assert.equal(h.size, 2)
  assert.equal(h.get('a'), '1')
  assert.ok(h.has('b') && !h.has('c'))
  assert.deepEqual(h.keys(), ['a', 'b'])
  assert.deepEqual(h.values(), ['1', '2'])
  assert.deepEqual(h.toObject(), { a: '1', b: '2' })

  h.insert('a', '9') // replace
  assert.equal(h.get('a'), '9')
  assert.equal(h.size, 2)
  assert.equal(h.remove('a'), 1) // count removed
  assert.equal(h.remove('a'), 0)
  assert.equal(h.size, 1)
})

test('case-insensitive lookup, insertion-ordered equality', () => {
  const h = new Headers({ 'Content-Type': 'application/json' })
  assert.ok(h.has('content-type') && h.has('CONTENT-TYPE'))
  assert.equal(h.get('content-type'), 'application/json')

  // Equality is order-significant (unlike a plain object): insertion order matters.
  const a = new Headers()
  a.append('x', '1')
  a.append('y', '2')
  const b = new Headers()
  b.append('y', '2')
  b.append('x', '1')
  assert.ok(!a.equals(b))
  assert.ok(a.equals(new Headers({ x: '1', y: '2' })))
})

test('append is multi-value', () => {
  const h = new Headers()
  h.append('Set-Cookie', 'a=1')
  h.append('set-cookie', 'b=2') // case-insensitive name, second value
  assert.deepEqual(h.getAll('Set-Cookie'), ['a=1', 'b=2'])
  assert.equal(h.get('set-cookie'), 'a=1') // first value
  assert.equal(h.size, 2)

  h.insert('SET-COOKIE', 'c=3') // collapses to one
  assert.deepEqual(h.getAll('set-cookie'), ['c=3'])
  assert.equal(h.size, 1)
})

test('withEntry is non-mutating', () => {
  const base = new Headers({ a: '1' })
  const extended = base.withEntry('z', '9')
  assert.deepEqual(extended.keys(), ['a', 'z'])
  assert.deepEqual(base.keys(), ['a']) // base untouched
})

test('constructor from an object preserves key order; clear empties', () => {
  const h = new Headers({ z: '1', a: '2', m: '3' })
  assert.deepEqual(h.keys(), ['z', 'a', 'm']) // insertion order, not sorted
  h.clear()
  assert.equal(h.size, 0)
  assert.ok(h.isEmpty())
})

test('HTTP conveniences', () => {
  const h = new Headers()
  h.insert('Content-Type', 'text/html; charset=utf-8')
  h.insert('Content-Length', '2048')
  assert.equal(h.contentType, 'text/html; charset=utf-8')
  assert.equal(h.contentLength, 2048)

  h.append('Accept', 'text/html')
  h.append('Accept', 'application/json')
  const wire = h.toHttpBytes().toString('utf8')
  assert.ok(wire.includes('Accept: text/html\r\n'))
  assert.ok(wire.includes('Accept: application/json\r\n'))

  const parsed = Headers.parseHttp(Buffer.from('Host: example.com\r\nAccept: */*\r\n\r\nignored: body'))
  assert.equal(parsed.get('host'), 'example.com')
  assert.equal(parsed.get('accept'), '*/*')
  assert.ok(!parsed.has('ignored')) // stops at the blank line
})

test('byte codec round-trips multi-value', () => {
  const h = new Headers()
  h.append('Set-Cookie', 'a=1')
  h.append('Set-Cookie', 'b=2')
  h.insert('Host', 'example.com')
  const blob = h.serializeBytes()
  assert.ok(Headers.deserializeBytes(blob).equals(h)) // multi-value + order preserved

  assert.throws(() => Headers.deserializeBytes(Buffer.from([1, 0, 0, 0]))) // truncated frame
})

test('copy is an independent equal value', () => {
  const h = new Headers()
  h.append('Set-Cookie', 'a=1')
  h.append('Set-Cookie', 'b=2')
  const dup = h.copy()
  assert.ok(dup.equals(h))
  dup.insert('X', '1')
  assert.ok(!h.has('X')) // copy is independent
})
