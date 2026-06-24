// Tests for the yggdryl Node.js extension.
// Build first with `npm run build`, then run `node --test`.

const { test } = require('node:test')
const assert = require('node:assert')
const { Uri, Url } = require('..')

test('uri components', () => {
  const uri = new Uri('https://example.com/docs?page=2#intro')
  assert.strictEqual(uri.scheme, 'https')
  assert.strictEqual(uri.authority, 'example.com')
  assert.strictEqual(uri.path, '/docs')
  assert.strictEqual(uri.query, 'page=2')
  assert.strictEqual(uri.fragment, 'intro')
})

test('uri without authority', () => {
  const uri = Uri.parse('mailto:alice@example.com')
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
