'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { Headers } = yggdryl.headers
const { MimeType } = yggdryl.mimetype
const { MediaType } = yggdryl.mediatype

// -------------------------------------------------------------------------------------
// Namespace + construction
// -------------------------------------------------------------------------------------

test('the headers namespace exposes Headers', () => {
  assert.equal(typeof Headers, 'function')
})

test('construction: empty, withCapacity, parseHttp', () => {
  const empty = new Headers()
  assert.equal(empty.len(), 0)
  assert.ok(empty.isEmpty())

  const sized = Headers.withCapacity(8)
  assert.ok(sized.isEmpty())

  const parsed = Headers.parseHttp(Buffer.from('Host: example.com\r\nAccept: */*\r\n'))
  assert.equal(parsed.len(), 2)
  assert.equal(parsed.get('host'), 'example.com')
  assert.equal(parsed.get('accept'), '*/*')
})

test('parseHttp is lenient: blank line ends the block, colon-less lines are skipped', () => {
  const parsed = Headers.parseHttp(Buffer.from('A: 1\nnocolon\nB: 2\r\n\r\nC: 3\r\n'))
  assert.equal(parsed.len(), 2)
  assert.equal(parsed.get('A'), '1')
  assert.equal(parsed.get('B'), '2')
  assert.ok(!parsed.contains('C')) // after the blank line — body territory
})

// -------------------------------------------------------------------------------------
// Read: case-insensitive get, multi-value, byte twins
// -------------------------------------------------------------------------------------

test('get is case-insensitive; getAll returns every value in order', () => {
  const h = new Headers()
  h.insert('Content-Type', 'application/json')
  assert.equal(h.get('content-type'), 'application/json')
  assert.equal(h.get('CONTENT-TYPE'), 'application/json')
  assert.ok(h.contains('CoNtEnT-tYpE'))
  assert.equal(h.get('absent'), null)

  h.append('Set-Cookie', 'a=1')
  h.append('set-cookie', 'b=2')
  assert.deepEqual(h.getAll('SET-COOKIE'), ['a=1', 'b=2'])
  assert.equal(h.get('Set-Cookie'), 'a=1') // first match wins
})

test('byte accessors reach raw non-UTF-8 values; string accessors skip them', () => {
  const h = new Headers()
  h.appendBytes(Buffer.from('X-Bin'), Buffer.from([0xff, 0xfe]))
  assert.equal(h.get('X-Bin'), null) // not valid UTF-8
  assert.deepEqual(h.getBytes(Buffer.from('x-bin')), Buffer.from([0xff, 0xfe]))

  h.appendBytes(Buffer.from('X-Bin'), Buffer.from([0x41]))
  assert.equal(h.getAllBytes(Buffer.from('X-BIN')).length, 2)
  assert.deepEqual(h.getAll('X-Bin'), ['A']) // only the UTF-8 value survives
  assert.equal(h.getBytes(Buffer.from('absent')), null)
})

// -------------------------------------------------------------------------------------
// Write: append vs insert, with, remove, clear
// -------------------------------------------------------------------------------------

test('append keeps existing entries; insert replaces every occurrence', () => {
  const h = new Headers()
  h.append('Set-Cookie', 'a=1')
  h.append('Set-Cookie', 'b=2')
  assert.equal(h.len(), 2)

  h.insert('Set-Cookie', 'c=3') // HTTP "replace" semantics
  assert.deepEqual(h.getAll('Set-Cookie'), ['c=3'])
  assert.equal(h.len(), 1)

  h.insertBytes(Buffer.from('Set-Cookie'), Buffer.from('d=4'))
  assert.deepEqual(h.getAll('set-cookie'), ['d=4'])
})

test('with is the chainable, non-mutating builder', () => {
  const base = new Headers()
  const built = base.with('A', '1').with('B', '2')
  assert.equal(built.get('A'), '1')
  assert.equal(built.get('B'), '2')
  assert.ok(base.isEmpty()) // the base is untouched
})

test('remove returns the number removed; clear empties the map', () => {
  const h = new Headers()
  h.append('Dup', 'a')
  h.append('DUP', 'b')
  h.append('Keep', 'k')
  assert.equal(h.remove('dup'), 2) // case-insensitive, every occurrence
  assert.ok(!h.contains('Dup'))
  assert.equal(h.remove('dup'), 0)
  assert.equal(h.get('Keep'), 'k')

  h.clear()
  assert.ok(h.isEmpty())
  assert.equal(h.len(), 0)
})

test('removeBytes reaches entries whose name is not valid UTF-8', () => {
  const h = new Headers()
  h.appendBytes(Buffer.from([0xff]), Buffer.from('v1'))
  h.appendBytes(Buffer.from([0xff]), Buffer.from('v2'))
  h.append('Keep', 'k')
  assert.equal(h.len(), 3)

  assert.equal(h.removeBytes(Buffer.from([0xff])), 2) // every occurrence, by raw bytes
  assert.equal(h.len(), 1)
  assert.equal(h.removeBytes(Buffer.from([0xff])), 0)
  assert.equal(h.get('Keep'), 'k')

  // The string form works through it too (same core method underneath).
  h.append('Set-Cookie', 'a=1')
  assert.equal(h.removeBytes(Buffer.from('SET-COOKIE')), 1) // still case-insensitive
})

test('items returns every [name, value] byte pair in insertion order', () => {
  const h = new Headers()
  h.append('Set-Cookie', 'a=1')
  h.appendBytes(Buffer.from([0xff]), Buffer.from([0x01, 0x02])) // non-UTF-8 name
  h.append('Set-Cookie', 'b=2')

  const items = h.items()
  assert.equal(items.length, 3) // the non-UTF-8 entry appears here…
  assert.deepEqual(h.keys(), ['Set-Cookie', 'Set-Cookie']) // …though keys skips it
  assert.deepEqual(items[0], [Buffer.from('Set-Cookie'), Buffer.from('a=1')])
  assert.deepEqual(items[1], [Buffer.from([0xff]), Buffer.from([0x01, 0x02])])
  assert.deepEqual(items[2], [Buffer.from('Set-Cookie'), Buffer.from('b=2')])

  assert.deepEqual(new Headers().items(), [])
})

// -------------------------------------------------------------------------------------
// mergeWith + typed conveniences
// -------------------------------------------------------------------------------------

test('mergeWith overlays other: its names replace, names only here are kept', () => {
  const base = new Headers().with('Keep', 'k')
  base.append('Dup', 'a')
  base.append('Dup', 'b')
  const other = new Headers().with('Dup', 'z').with('New', 'n')

  const merged = base.mergeWith(other)
  assert.equal(merged.get('Keep'), 'k')
  assert.deepEqual(merged.getAll('Dup'), ['z']) // both occurrences replaced
  assert.equal(merged.get('New'), 'n')

  // Both originals are untouched.
  assert.deepEqual(base.getAll('Dup'), ['a', 'b'])
  assert.ok(!other.contains('Keep'))
})

test('contentType / contentLength read the common headers', () => {
  const h = new Headers()
    .with('Content-Type', 'application/json')
    .with('Content-Length', ' 1024 ')
  assert.equal(h.contentType(), 'application/json')
  assert.equal(h.contentLength(), 1024) // trimmed and parsed

  assert.equal(new Headers().contentType(), null)
  assert.equal(new Headers().contentLength(), null)
  assert.equal(new Headers().with('Content-Length', 'abc').contentLength(), null)
})

// -------------------------------------------------------------------------------------
// HTTP text form + binary codec
// -------------------------------------------------------------------------------------

test('toHttpBytes renders the wire form and parseHttp round-trips it', () => {
  const h = new Headers()
  h.insert('Host', 'example.com')
  h.append('Set-Cookie', 'a=1')
  h.append('Set-Cookie', 'b=2')

  const wire = h.toHttpBytes()
  assert.equal(wire.toString(), 'Host: example.com\r\nSet-Cookie: a=1\r\nSet-Cookie: b=2\r\n')
  assert.ok(Headers.parseHttp(wire).equals(h))
})

test('serializeBytes / deserializeBytes round-trip arbitrary bytes; truncated throws', () => {
  const h = new Headers()
  h.append('Set-Cookie', 'a=1')
  h.append('Set-Cookie', 'b=2')
  h.appendBytes(Buffer.from('X-Bin'), Buffer.from([0x00, 0xff, 13, 10])) // not HTTP-safe

  const frame = h.serializeBytes()
  const back = Headers.deserializeBytes(frame)
  assert.ok(back.equals(h))
  assert.equal(back.len(), 3)
  assert.deepEqual(back.getBytes(Buffer.from('X-Bin')), Buffer.from([0x00, 0xff, 13, 10]))

  assert.throws(
    () => Headers.deserializeBytes(frame.subarray(0, frame.length - 1)),
    /unexpected end of data/
  )
})

// -------------------------------------------------------------------------------------
// keys, equals, copy, toString
// -------------------------------------------------------------------------------------

test('keys lists names in insertion order (repeats kept, non-UTF-8 names skipped)', () => {
  const h = new Headers()
  h.append('Set-Cookie', 'a=1')
  h.append('Set-Cookie', 'b=2')
  h.append('Host', 'example.com')
  assert.deepEqual(h.keys(), ['Set-Cookie', 'Set-Cookie', 'Host'])

  h.appendBytes(Buffer.from([0xff]), Buffer.from('v')) // a non-UTF-8 name
  assert.equal(h.len(), 4)
  assert.deepEqual(h.keys(), ['Set-Cookie', 'Set-Cookie', 'Host']) // skipped, not garbled
})

test('equals is content equality over the exact entries', () => {
  const a = new Headers().with('A', '1')
  const b = new Headers().with('A', '1')
  assert.ok(a.equals(b))
  b.append('B', '2')
  assert.ok(!a.equals(b))
})

test('copy is an independent clone', () => {
  const h = new Headers().with('A', '1')
  const dup = h.copy()
  assert.ok(dup.equals(h))
  dup.insert('A', '9')
  assert.equal(h.get('A'), '1') // original untouched
  assert.equal(dup.get('A'), '9')
})

test('toString renders the readable map form', () => {
  const h = new Headers().with('A', '1')
  assert.equal(typeof h.toString(), 'string')
  assert.match(h.toString(), /"A": "1"/)
  assert.equal(new Headers().toString(), '{}')
})

// -------------------------------------------------------------------------------------
// media type: Content-Type / Content-Encoding accessors
// -------------------------------------------------------------------------------------

test('content type / encoding setters and readers', () => {
  const h = new Headers()
  h.setContentType('application/json')
  assert.equal(h.contentType(), 'application/json')
  h.setContentEncoding('gzip')
  assert.equal(h.contentEncoding(), 'gzip')
  assert.equal(new Headers().contentEncoding(), null)
})

test('mimeType / mediaType interpret Content-Type and Content-Encoding', () => {
  const h = new Headers()
  h.setContentType('application/json; charset=utf-8')
  assert.ok(h.mimeType() instanceof MimeType)
  assert.equal(h.mimeType().essence, 'application/json') // primary, parameters dropped

  // Content-Type + Content-Encoding compose into the layered media type.
  h.setContentType('application/x-tar')
  h.setContentEncoding('gzip')
  assert.ok(h.mediaType() instanceof MediaType)
  assert.deepEqual(h.mediaType().essences(), ['application/x-tar', 'application/gzip'])

  // No Content-Type -> null (the justified absence).
  assert.equal(new Headers().mimeType(), null)
  assert.equal(new Headers().mediaType(), null)
})

test('setMimeType / setMediaType write Content-Type from the value types', () => {
  const h = new Headers()
  h.setMimeType(MimeType.parse('image/png'))
  assert.equal(h.contentType(), 'image/png')

  h.setMediaType(MediaType.parse('application/x-tar, application/gzip'))
  assert.equal(h.contentType(), 'application/x-tar, application/gzip') // comma-joined essences
  assert.equal(h.mimeType().essence, 'application/x-tar') // primary of the list
})

// -------------------------------------------------------------------------------------
// modification time (epoch microseconds) + header-name constants
// -------------------------------------------------------------------------------------

test('mtime round-trips signed epoch microseconds', () => {
  const h = new Headers()
  assert.equal(h.mtime(), null)

  h.setMtime(1_600_000_000_000_000)
  assert.equal(h.mtime(), 1_600_000_000_000_000)
  assert.equal(h.get(yggdryl.headers.MTIME), '1600000000000000') // stored as a compact decimal

  // A signed value before the epoch stays negative (never wrapped to a u32).
  h.setMtime(-123_456)
  assert.equal(h.mtime(), -123_456)

  // A non-integer value reads back as null.
  assert.equal(new Headers().with(yggdryl.headers.MTIME, 'notanumber').mtime(), null)
})

test('the header-name constants are exposed on the namespace', () => {
  assert.equal(yggdryl.headers.MTIME, 'X-Mtime-Us')
  assert.equal(yggdryl.headers.LAST_MODIFIED, 'Last-Modified')
  // The constants address real entries.
  const h = new Headers().with(yggdryl.headers.LAST_MODIFIED, 'Wed, 21 Oct 2015 07:28:00 GMT')
  assert.equal(h.get('last-modified'), 'Wed, 21 Oct 2015 07:28:00 GMT')
})

test('nullable flag defaults false and round-trips', () => {
  const h = new Headers()
  assert.strictEqual(h.nullable(), false) // unset -> non-nullable default
  h.setNullable(true)
  assert.strictEqual(h.nullable(), true)
  assert.strictEqual(h.get(Headers.NULLABLE ?? 'X-Nullable'), 'true')
  h.setNullable(false)
  assert.strictEqual(h.nullable(), false)
})
