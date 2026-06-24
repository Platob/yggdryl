// Tests for the yggdryl Node.js extension.
// Build first with `npm run build`, then run `node --test`.

const { test } = require('node:test')
const assert = require('node:assert')
const { Uri, Url, Version, percentEncode, percentDecode } = require('..')

test('uri components', () => {
  const uri = new Uri('https://example.com/docs?page=2#intro')
  assert.strictEqual(uri.scheme, 'https')
  assert.strictEqual(uri.authority, 'example.com')
  assert.strictEqual(uri.path, '/docs')
  assert.strictEqual(uri.query, 'page=2')
  assert.strictEqual(uri.fragment, 'intro')
})

test('uri without authority', () => {
  const uri = Uri.fromStr('mailto:alice@example.com')
  assert.strictEqual(uri.scheme, 'mailto')
  assert.strictEqual(uri.authority, null)
  assert.strictEqual(uri.path, 'alice@example.com')
})

test('uri toString round-trip', () => {
  const text = 'file:///etc/hosts'
  assert.strictEqual(new Uri(text).toString(), text)
})

test('uri invalid throws', () => {
  assert.throws(() => new Uri('no-scheme/path'))
})

test('url components', () => {
  const url = new Url('https://user:pw@example.com:8443/api?v=1#top')
  assert.strictEqual(url.scheme, 'https')
  assert.strictEqual(url.username, 'user')
  assert.strictEqual(url.password, 'pw')
  assert.strictEqual(url.host, 'example.com')
  assert.strictEqual(url.port, 8443)
  assert.strictEqual(url.path, '/api')
  assert.strictEqual(url.query, 'v=1')
  assert.strictEqual(url.fragment, 'top')
  assert.strictEqual(url.authority, 'user:pw@example.com:8443')
})

test('url ipv6', () => {
  const url = new Url('http://[::1]:8080/status')
  assert.strictEqual(url.host, '::1')
  assert.strictEqual(url.port, 8080)
  assert.strictEqual(url.toString(), 'http://[::1]:8080/status')
})

test('url requires authority', () => {
  assert.throws(() => new Url('mailto:alice@example.com'))
})

test('version components', () => {
  const v = Version.fromStr('1.4.2')
  assert.strictEqual(v.major, 1)
  assert.strictEqual(v.minor, 4)
  assert.strictEqual(v.patch, 2)
  assert.strictEqual(v.toString(), '1.4.2')
})

test('version constructor and defaults', () => {
  assert.strictEqual(new Version(2, 0, 0).toString(), '2.0.0')
  assert.strictEqual(Version.fromStr('2').toString(), '2.0.0')
})

test('version compare', () => {
  assert.strictEqual(new Version(1, 4, 2).compare(new Version(1, 10, 0)), -1)
  assert.strictEqual(new Version(2, 0, 0).compare(new Version(1, 9, 9)), 1)
  assert.ok(new Version(1, 2, 3).equals(Version.fromStr('1.2.3')))
})

test('version invalid throws', () => {
  assert.throws(() => Version.fromStr('1.x.0'))
})

test('safe flag', () => {
  assert.throws(() => new Uri('1http:x'))
  assert.strictEqual(new Uri('1http:x', false).scheme, '1http')
  assert.strictEqual(Version.fromStr('1.2.3.4', false).toString(), '1.2.3')
})

test('from mapping', () => {
  const uri = Uri.fromMapping({ scheme: 'https', authority: 'example.com', path: '/x' })
  assert.strictEqual(uri.toString(), 'https://example.com/x')
  const url = Url.fromMapping({ scheme: 'https', host: 'h', port: '8443' })
  assert.strictEqual(url.host, 'h')
  assert.strictEqual(url.port, 8443)
})

test('from parts (no string building)', () => {
  const url = Url.fromParts('https', 'example.com', 8443, 'user', 'pw', '/api')
  assert.strictEqual(url.toString(), 'https://user:pw@example.com:8443/api')
  const uri = Uri.fromParts('mailto', 'alice@example.com')
  assert.strictEqual(uri.toString(), 'mailto:alice@example.com')
})

test('functional copy / with_', () => {
  const base = new Url('https://example.com/api')
  const secured = base.withPort(8443).withUsername('user')
  assert.strictEqual(secured.toString(), 'https://user@example.com:8443/api')
  // original untouched
  assert.strictEqual(base.toString(), 'https://example.com/api')
  assert.strictEqual(Version.fromStr('1.0.0').withMinor(4).toString(), '1.4.0')
})

test('percent encoding', () => {
  assert.strictEqual(percentEncode('a b/c'), 'a%20b%2Fc')
  assert.strictEqual(percentDecode('a%20b%2Fc'), 'a b/c')
  assert.throws(() => percentDecode('%zz'))
})

test('params and addParam (multi-value)', () => {
  const url = new Url('https://h/p?a=1&a=2&b=hi')
  assert.deepStrictEqual(url.params(), { a: ['1', '2'], b: ['hi'] })
  const updated = url.addParam('a', ['x']).addParam('c', ['1', '2'])
  assert.deepStrictEqual(updated.params().a, ['x'])
  assert.deepStrictEqual(updated.params().c, ['1', '2'])
  const built = new Uri('https://h/p').withParams({ q: ['a b'] })
  assert.strictEqual(built.query, 'q=a%20b')
})

test('copy overrides', () => {
  const url = new Url('https://example.com/api')
  assert.strictEqual(url.copy(null, null, null, null, 8443).toString(), 'https://example.com:8443/api')
  assert.strictEqual(url.copy().toString(), 'https://example.com/api')
})

test('url toUri', () => {
  const uri = new Url('https://user@h:8443/p?x=1').toUri()
  assert.strictEqual(uri.authority, 'user@h:8443')
})
