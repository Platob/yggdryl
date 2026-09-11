'use strict'

// The character encodings, checked against the runtime's own decoder.

const assert = require('node:assert/strict')
const test = require('node:test')

const { MediaType, TextOptions, charset, enums } = require('yggdryl')

// The charsets whose WHATWG label decodes the same bytes the same way.
//
// Three are deliberately absent. `TextDecoder` has no label for IBM 437 at
// all. `TextDecoder` never refuses, so US-ASCII and UTF-8 replace where these
// refuse - the lossy door is what matches there. And `iso-8859-1` is the
// interesting one: WHATWG maps that label to windows-1252, while this package
// decodes ISO 8859-1 itself, which is asserted on its own below.
const SHARED_LABELS = [
  'utf-16le',
  'iso-8859-2',
  'iso-8859-15',
  'windows-1250',
  'windows-1251',
  'windows-1252',
  'macintosh',
]

test('the listing is the vocabulary the names resolve to', () => {
  assert.deepEqual([...charset.CHARSETS], [...enums.charsets])
  for (const name of charset.CHARSETS) {
    assert.equal(charset.canonicalName(name), name)
  }
})

test('customary aliases resolve to one name', () => {
  assert.equal(charset.canonicalName('cp1252'), 'windows-1252')
  assert.equal(charset.canonicalName('latin1'), 'iso-8859-1')
  assert.equal(charset.canonicalName('UTF8'), 'utf-8')
  // Two standards disagree over bare 'utf-16', so it is refused rather than
  // defaulted to either order.
  assert.throws(() => charset.canonicalName('utf-16'), /charset/)
})

test('decoding agrees with TextDecoder over the same names', () => {
  const wire = Buffer.from([0x63, 0x61, 0x66, 0xe9, 0x20, 0x80])
  for (const label of SHARED_LABELS) {
    const expected = new TextDecoder(label).decode(wire)
    assert.equal(charset.decode(label, wire), expected, label)
  }
  // Where the runtime replaces, the lossy door is the one that matches.
  assert.equal(charset.decodeLossy('utf-8', wire), new TextDecoder().decode(wire))
})

test('iso-8859-1 is not a spelling of windows-1252 here', () => {
  // The two differ exactly over the C1 range, and WHATWG resolves the label
  // to windows-1252. This package decodes ISO 8859-1 itself.
  const c1 = Buffer.from([0x80])
  assert.equal(charset.decode('iso-8859-1', c1), '\u0080')
  assert.equal(charset.decode('windows-1252', c1), '\u20AC')
  assert.equal(new TextDecoder('iso-8859-1').decode(c1), '\u20AC')
})

test('encoding is the direction TextEncoder does not have', () => {
  assert.deepEqual(
    charset.encode('windows-1252', 'prix 12€'),
    Buffer.from('prix 12\x80', 'latin1'),
  )
  assert.equal(charset.decode('windows-1252', charset.encode('windows-1252', 'désk')), 'désk')
  assert.throws(() => charset.encode('iso-8859-1', 'prix 12€'), /U\+20AC/)
})

test('a refusal names the charset and what it found', () => {
  assert.throws(() => charset.decode('us-ascii', Buffer.from([0x63, 0xe9])), /us-ascii/)
  assert.equal(charset.decodeLossy('us-ascii', Buffer.from([0x63, 0xe9])), 'c�')
})

test('a byte-order mark names its charset and its length', () => {
  assert.deepEqual(charset.fromBom(Buffer.from([0xff, 0xfe, 0x41, 0x00])), {
    charset: 'utf-16le',
    length: 2,
  })
  assert.equal(charset.fromBom(Buffer.from('id,name')), null)
  assert.deepEqual(charset.bom('utf-8'), Buffer.from([0xef, 0xbb, 0xbf]))
  assert.equal(charset.bom('windows-1252'), null)
})

test('a media type carries the charset it declares', () => {
  const declared = MediaType.fromString('text/csv;charset=windows-1252')
  assert.equal(declared.charset, 'windows-1252')
  assert.equal(declared.toString(), 'text/csv;charset=windows-1252')
  assert.ok(MediaType.fromString(declared.toString()).equals(declared))
  assert.equal(MediaType.fromString('text/csv').charset, null)

  const mutable = MediaType.fromString('text/csv')
  mutable.setCharset('latin1')
  assert.equal(mutable.charset, 'iso-8859-1')
  mutable.setCharset(null)
  assert.equal(mutable.charset, null)
})

test('text options declare the capture charset', () => {
  const options = new TextOptions()
  assert.equal(options.charset, 'utf-8')
  options.charset = 'cp1252'
  assert.equal(options.charset, 'windows-1252')
  assert.throws(() => {
    options.charset = 'nope'
  }, /charset/)
})
