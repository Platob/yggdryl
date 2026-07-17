'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')

const yggdryl = require('..')
const { Uri, Url, Authority, defaultPort } = yggdryl.uri

test('the uri namespace exposes Uri, Url, and Authority', () => {
  for (const cls of [Uri, Url, Authority]) {
    assert.equal(typeof cls, 'function')
  }
})

test('parse splits a full URI into its RFC 3986 components', () => {
  const uri = Uri.parse('https://user:pw@example.com:8080/a/b.txt?q=1#frag')
  assert.equal(uri.scheme, 'https')
  assert.equal(uri.user, 'user')
  assert.equal(uri.password, 'pw')
  assert.equal(uri.host, 'example.com')
  assert.equal(uri.port, 8080)
  assert.equal(uri.path, '/a/b.txt')
  assert.equal(uri.query, 'q=1')
  assert.equal(uri.fragment, 'frag')
  assert.equal(uri.name, 'b.txt')
  assert.equal(uri.stem, 'b')
  assert.equal(uri.extension, 'txt')
  assert.equal(uri.toString(), 'https://user:pw@example.com:8080/a/b.txt?q=1#frag')

  // A parity sanity-check against the platform's WHATWG URL for a comparable input.
  const whatwg = new URL('https://user:pw@example.com:8080/a/b.txt?q=1#frag')
  assert.equal(uri.host, whatwg.hostname)
  assert.equal(String(uri.port), whatwg.port)
  assert.equal(uri.path, whatwg.pathname)
})

test('absent components read as null; a bare path is a valid Uri', () => {
  const uri = Uri.parse('/a/b/c')
  assert.equal(uri.scheme, null)
  assert.equal(uri.authority, null)
  assert.equal(uri.host, null)
  assert.equal(uri.port, null)
  assert.equal(uri.query, null)
  assert.equal(uri.fragment, null)
  assert.equal(uri.path, '/a/b/c')
})

test('the authority is exposed as its own value type', () => {
  const auth = Uri.parse('sc://user:pw@host:99/p').authority
  assert.ok(auth instanceof Authority)
  assert.equal(auth.user, 'user')
  assert.equal(auth.password, 'pw')
  assert.equal(auth.host, 'host')
  assert.equal(auth.port, 99)
  assert.equal(auth.toString(), 'user:pw@host:99')
  assert.ok(auth.equals(new Authority('host', 'user', 'pw', 99)))
  assert.ok(Authority.fromHost('host').equals(new Authority('host')))
})

test('extensions: multi-dot, dotfile, directory-like', () => {
  assert.deepEqual(Uri.fromPath('/x/archive.tar.gz').extensions, ['tar', 'gz'])
  assert.equal(Uri.fromPath('/x/archive.tar.gz').stem, 'archive.tar')
  assert.equal(Uri.fromPath('/x/archive.tar.gz').extension, 'gz')

  // A leading dot is not an extension separator (hidden dotfile).
  const dot = Uri.fromPath('/x/.bashrc')
  assert.equal(dot.name, '.bashrc')
  assert.equal(dot.stem, '.bashrc')
  assert.equal(dot.extension, null)
  assert.deepEqual(dot.extensions, [])

  // A directory-like path (trailing slash) has no filename.
  assert.equal(Uri.fromPath('/a/b/').name, null)
})

test('Windows paths are normalized to POSIX slashes with no scheme', () => {
  const drive = Uri.parse('C:\\Users\\x\\a.tar.gz')
  assert.equal(drive.scheme, null) // drive letter, not a one-letter scheme
  assert.equal(drive.path, 'C:/Users/x/a.tar.gz')
  assert.deepEqual(drive.extensions, ['tar', 'gz'])

  assert.equal(Uri.fromPath('a\\b\\c').path, 'a/b/c')
  // UNC path.
  assert.equal(Uri.parse('\\\\server\\share\\f').path, '//server/share/f')
})

test('IPv6 hosts stay bracketed', () => {
  const uri = Uri.parse('http://[::1]:8080/p')
  assert.equal(uri.host, '[::1]')
  assert.equal(uri.port, 8080)
})

test('byte codec round-trips and is the exact inverse', () => {
  const uri = Uri.parse('sc://h/p?q#f')
  const raw = uri.serializeBytes()
  assert.ok(Buffer.isBuffer(raw))
  assert.equal(raw.toString('utf8'), 'sc://h/p?q#f')
  assert.ok(Uri.deserializeBytes(raw).equals(uri))

  // Non-UTF-8 bytes are rejected with a guided error.
  assert.throws(() => Uri.deserializeBytes(Buffer.from([0xff, 0xfe])), /not valid UTF-8/)
})

test('value semantics: equality and hashing agree with the bytes', () => {
  const a = Uri.parse('sc://h/p')
  const b = Uri.parse('sc://h/p')
  assert.ok(a.equals(b))
  assert.equal(a.hashCode(), b.hashCode())
  assert.ok(!a.equals(Uri.parse('sc://h/other')))
})

test('builder mutators return a new Uri; setters mutate in place', () => {
  const base = Uri.fromPath('/p')
  const built = base.withScheme('https').withHost('example.com').withPort(443)
  assert.equal(built.toString(), 'https://example.com:443/p')
  assert.equal(base.scheme, null) // original untouched (builder returns a copy)

  const uri = Uri.fromPath('/p')
  uri.setScheme('https')
  uri.setHost('example.com')
  uri.setPort(443)
  uri.setQuery('a=1')
  uri.setFragment('top')
  assert.equal(uri.toString(), 'https://example.com:443/p?a=1#top')
})

test('Url requires a scheme; authority stays optional', () => {
  const url = Url.parse('https://example.com/a/b.txt')
  assert.equal(url.scheme, 'https')
  assert.equal(url.host, 'example.com')
  assert.equal(url.name, 'b.txt')

  // A scheme-present, authority-less input is still a valid Url (host is null).
  const mailto = Url.parse('mailto:person@example.com')
  assert.equal(mailto.scheme, 'mailto')
  assert.equal(mailto.host, '') // total: empty when there is no authority
  assert.equal(mailto.hasAuthority, false)
  assert.equal(mailto.path, 'person@example.com')

  // A scheme-less input is not an absolute URL.
  assert.throws(() => Url.parse('/relative/path'), /requires a scheme/)
})

test('Uri <-> Url interchange', () => {
  const url = Uri.parse('sc://h/p').toUrl()
  assert.ok(url instanceof Url)
  assert.equal(url.scheme, 'sc')

  // A scheme-less Uri cannot become a Url.
  assert.throws(() => Uri.parse('/relative').toUrl(), /requires a scheme/)

  // Round back to a Uri.
  const uri = url.toUri()
  assert.ok(uri instanceof Uri)
  assert.equal(uri.scheme, 'sc')
  assert.equal(uri.toString(), 'sc://h/p')
})

test('Url byte codec and value semantics', () => {
  const url = Url.parse('https://h/p')
  assert.ok(Url.deserializeBytes(url.serializeBytes()).equals(url))
  assert.equal(url.hashCode(), Url.parse('https://h/p').hashCode())

  // Decoding scheme-less bytes as a Url fails.
  assert.throws(() => Url.deserializeBytes(Buffer.from('/relative')), /requires a scheme/)
})

test('defaultPort maps well-known schemes (case-insensitive)', () => {
  assert.equal(defaultPort('https'), 443)
  assert.equal(defaultPort('HTTPS'), 443) // scheme is case-insensitive
  assert.equal(defaultPort('ws'), 80)
  assert.equal(defaultPort('postgres'), 5432)
  assert.equal(defaultPort('s3'), null) // no registered default
})

test('portOrDefault falls back to the scheme default without mutating the Uri', () => {
  const implicit = Uri.parse('https://example.com/p')
  assert.equal(implicit.port, null)
  assert.equal(implicit.defaultPort, 443)
  assert.equal(implicit.portOrDefault, 443)

  const explicit = Uri.parse('https://example.com:8443/p')
  assert.equal(explicit.portOrDefault, 8443) // explicit wins over the default

  // Scheme-less / no-default schemes report null.
  assert.equal(Uri.parse('//h/p').portOrDefault, null)
  assert.equal(Uri.parse('s3://bucket/key').portOrDefault, null)

  // The fallback is read-only: nothing is written into the canonical form.
  assert.equal(implicit.toString(), 'https://example.com/p') // no ":443"
  assert.ok(!implicit.equals(Uri.parse('https://example.com:443/p')))

  // Url mirrors it.
  assert.equal(Url.parse('wss://h/socket').portOrDefault, 443)

  // A parity check against the platform WHATWG URL, which fills the default port as ''.
  const whatwg = new URL('https://example.com/p')
  assert.equal(whatwg.port, '') // WHATWG blanks the implicit port; we surface it explicitly
})

test('IPv6 host detection and unbracketing', () => {
  const uri = Uri.parse('http://[2001:db8::1]:8080/p')
  assert.equal(uri.host, '[2001:db8::1]') // stored bracketed
  assert.ok(uri.hostIsIpv6)
  assert.equal(uri.hostUnbracketed, '2001:db8::1') // bare address to dial
  assert.equal(uri.portOrDefault, 8080)

  const plain = Uri.parse('http://example.com/p')
  assert.ok(!plain.hostIsIpv6)
  assert.equal(plain.hostUnbracketed, 'example.com')

  // No authority -> null / false.
  const mailto = Uri.parse('mailto:a@b.com')
  assert.ok(!mailto.hostIsIpv6)
  assert.equal(mailto.hostUnbracketed, null)

  // Authority value type exposes the same pair.
  const auth = Authority.fromHost('[::1]')
  assert.ok(auth.hostIsIpv6)
  assert.equal(auth.hostUnbracketed, '::1')

  // Url mirrors it.
  const url = Url.parse('https://[::1]/status')
  assert.ok(url.hostIsIpv6)
  assert.equal(url.hostUnbracketed, '::1')
})

test('joinpath combines paths correctly', () => {
  const base = Uri.parse('https://api.example.com/v1')
  assert.equal(base.joinpath('users').toString(), 'https://api.example.com/v1/users')
  // Chains; a trailing slash on the base is not doubled.
  assert.equal(base.joinpath('users').joinpath('42').path, '/v1/users/42')
  assert.equal(Uri.fromPath('/v1/').joinpath('users').path, '/v1/users')
  // Multi-segment in one call.
  assert.equal(Uri.fromPath('/v1').joinpath('users/42').path, '/v1/users/42')
  // An absolute segment resets the path; query/fragment are kept.
  assert.equal(Uri.parse('https://h/a?x=1#f').joinpath('/b').toString(), 'https://h/b?x=1#f')
  // A relative segment under an authority stays rooted (does not fuse into the host).
  assert.equal(Uri.parse('https://h').joinpath('p').path, '/p')
  // Encoded like setPath.
  assert.equal(Uri.fromPath('/v1').joinpath('a b').path, '/v1/a%20b')
  // Url.joinpath keeps the scheme.
  assert.equal(Url.parse('https://h/v1').joinpath('x').scheme, 'https')
})

test('mergeWith overlays present components', () => {
  const base = Uri.parse('https://prod.example.com/v1?trace=1')
  // A patch with only an authority swaps the host, keeping scheme/path/query.
  assert.equal(
    base.mergeWith(Uri.parse('//staging.example.com')).toString(),
    'https://staging.example.com/v1?trace=1',
  )
  // Merging a default (empty) URI is an identity copy.
  assert.ok(base.mergeWith(Uri.parse('')).equals(base))
  // Authority merges at the component level.
  const a = new Authority('db', 'svc', 'secret', 5432)
  assert.equal(a.mergeWith(Authority.fromHost('replica')).toString(), 'svc:secret@replica:5432')
})

test('copy is an independent clone', () => {
  const base = Uri.parse('https://h/a?q#f')
  const dup = base.copy()
  assert.ok(dup.equals(base))
  dup.setPath('/b') // mutating the copy leaves the original untouched
  assert.equal(base.path, '/a')
  assert.equal(dup.path, '/b')
  assert.ok(Authority.fromHost('h').copy().equals(Authority.fromHost('h')))
})

test('withAuthority attaches a built Authority; Authority builders chain', () => {
  const authority = Authority.fromHost('db.internal').withUser('svc').withPort(5432)
  const built = Uri.fromPath('').withScheme('postgres').withAuthority(authority).withPath('/app')
  assert.equal(built.toString(), 'postgres://svc@db.internal:5432/app')
  // Dropping the authority.
  assert.equal(Uri.parse('https://user@h:8080/p').withAuthority(null).authority, null)
  // Authority builders chain and clear via null.
  const a = Authority.fromHost('h').withUser('u').withPassword('p').withPort(80)
  assert.equal(a.toString(), 'u:p@h:80')
  assert.equal(a.withUser(null).withPassword(null).toString(), 'h:80')
})

test('an out-of-range port is a guided error naming the offending value', () => {
  assert.throws(() => Uri.parse('http://h:99999/'), /99999/)
  assert.throws(() => Uri.parse('http://h:99999/'), /0\.\.=65535/)
})

test('query parameter map access and CRUD', () => {
  const uri = Uri.parse('http://h/p?a=1&b=2&a=3')
  // Read
  assert.equal(uri.queryParam('a'), '1') // first occurrence wins
  assert.equal(uri.queryParam('missing'), null)
  assert.deepEqual(uri.queryParamAll('a'), ['1', '3'])
  assert.deepEqual(uri.queryParams(), [['a', '1'], ['b', '2'], ['a', '3']])
  assert.deepEqual(Object.fromEntries(uri.queryParams()), { a: '3', b: '2' }) // map view
  assert.ok(uri.hasQueryParam('b'))
  assert.ok(!uri.hasQueryParam('z'))

  // Update then create
  uri.setQueryParam('a', '9')
  assert.equal(uri.query, 'a=9&b=2')
  uri.setQueryParam('c', '7')
  assert.equal(uri.query, 'a=9&b=2&c=7')

  // Delete
  assert.equal(uri.removeQueryParam('a'), true)
  assert.equal(uri.query, 'b=2&c=7')
  assert.equal(uri.removeQueryParam('z'), false)

  // Builder variants return fresh values
  const built = Uri.parse('http://h/p').withQueryParam('x', '1').withQueryParam('y', '2')
  assert.equal(built.toString(), 'http://h/p?x=1&y=2')
  assert.equal(built.withoutQueryParam('x').toString(), 'http://h/p?y=2')
})

test('query parameters on Url and edge cases', () => {
  const url = Url.parse('https://h/p?flag&a=')
  assert.equal(url.queryParam('flag'), '') // bare key -> empty value
  assert.equal(url.queryParam('a'), '') // explicit empty value
  assert.ok(url.hasQueryParam('flag'))
  url.setQueryParam('flag', 'on')
  assert.equal(url.queryParam('flag'), 'on')
  assert.equal(Uri.parse('http://h/p?t=a=b').queryParam('t'), 'a=b') // value keeps inner '='
})

test('bulk query update and normalize', () => {
  const uri = Uri.parse('http://h/p?a=1&b=2&a=3')
  uri.setQueryParams([['a', '9'], ['c', '7']]) // bulk update in one pass
  assert.equal(uri.query, 'a=9&b=2&c=7')
  uri.setQueryParams(Object.entries({ z: '1' })) // apply an object via Object.entries
  assert.equal(uri.queryParam('z'), '1')

  const messy = Uri.parse('http://h/p?c=3&a=1&&b=2')
  messy.normalizeQuery() // sort by key + drop empty tokens
  assert.equal(messy.query, 'a=1&b=2&c=3')

  const built = Uri.parse('http://h/p?b=2').withQueryParams([['a', '1']]).withNormalizedQuery()
  assert.equal(built.toString(), 'http://h/p?a=1&b=2')
})

test('query parameter percent-encoding', () => {
  const uri = Uri.parse('http://h/p')
  uri.setQueryParam('q', 'a b&c')
  assert.equal(uri.query, 'q=a%20b%26c') // stored encoded
  assert.equal(uri.queryParam('q'), 'a b&c') // decoded by default
  assert.equal(uri.queryParam('q', true), 'a%20b%26c') // raw stored form

  for (const value of ['plain', 'a b', '100%', 'x&y=z', 'café', '']) {
    const u = Uri.parse('http://h/p').withQueryParam('k', value)
    assert.equal(u.queryParam('k'), value) // set -> get round-trips
  }

  const u = Uri.parse('http://h/p').withQueryParam('n', 'a b').withQueryParam('t', 'x&y')
  assert.deepEqual(Object.fromEntries(u.queryParams()), { n: 'a b', t: 'x&y' }) // decoded
  assert.deepEqual(Object.fromEntries(u.queryParams(true)), { n: 'a%20b', t: 'x%26y' })

  assert.equal(Uri.parse('http://h').withPath('/a b').path, '/a%20b') // component encoded
})
