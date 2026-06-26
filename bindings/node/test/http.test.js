// Tests for HttpSession / HttpResponse against a localhost server.
// Hermetic: a node:http server runs in-process; the client's requests resolve as
// Promises (off the event loop) so there is no deadlock and no real network.
// Build first with `npm run build`, then run `node --test`.

const { test } = require('node:test')
const assert = require('node:assert')
const http = require('node:http')
const { HttpSession } = require('..')

function startServer() {
  return new Promise((resolve) => {
    const server = http.createServer((req, res) => {
      if (req.url === '/missing') {
        res.writeHead(404, { 'Content-Type': 'text/plain' })
        res.end('nope')
        return
      }
      if (req.url === '/brotli') {
        // A Brotli-compressed JSON body advertised via Content-Encoding: br.
        const { Compression } = require('..')
        const body = Buffer.from('{"msg":"brotli over the wire","n":7}')
        const packed = Compression.fromStr('br').compress(body)
        res.writeHead(200, { 'Content-Type': 'application/json', 'Content-Encoding': 'br' })
        res.end(Buffer.from(packed))
        return
      }
      const chunks = []
      req.on('data', (c) => chunks.push(c))
      req.on('end', () => {
        const body = Buffer.concat(chunks)
        if (req.method === 'GET') {
          res.writeHead(200, {
            'Content-Type': 'text/plain',
            'X-Echo-Back': req.headers['x-echo'] || '',
            'X-Auth-Back': req.headers['authorization'] || '',
          })
          res.end('hello world')
        } else if (req.method === 'DELETE') {
          res.writeHead(204)
          res.end()
        } else {
          res.writeHead(201, { 'Content-Type': 'application/octet-stream' })
          res.end(body) // echo the request body
        }
      })
    })
    server.listen(0, '127.0.0.1', () => resolve({ server, port: server.address().port }))
  })
}

test('http session against a localhost server', async () => {
  const { server, port } = await startServer()
  const base = `http://127.0.0.1:${port}`
  try {
    const session = new HttpSession('yggdryl-test')

    const r = await session.get(base + '/')
    assert.strictEqual(r.status, 200)
    assert.strictEqual(r.ok, true)
    assert.strictEqual(r.text(), 'hello world')
    assert.deepStrictEqual(r.content, Buffer.from('hello world'))
    assert.strictEqual(r.contentType, 'text/plain')
    assert.ok(r.url.startsWith('http://127.0.0.1'))
    // The buffered convenience API stamps both timestamps (dispatch, then EOF).
    assert.ok(r.sentAt > 0)
    assert.ok(r.receivedAt >= r.sentAt)

    const posted = await session.post(base + '/submit', Buffer.from('ping-payload'))
    assert.strictEqual(posted.status, 201)
    assert.deepStrictEqual(posted.content, Buffer.from('ping-payload'))

    // Default header, then a per-request override.
    const withDefault = new HttpSession(undefined, { 'X-Echo': 'from-default' })
    assert.strictEqual((await withDefault.get(base + '/')).header('x-echo-back'), 'from-default')
    const overridden = await withDefault.request('GET', base + '/', { 'X-Echo': 'from-request' })
    assert.strictEqual(overridden.header('x-echo-back'), 'from-request')

    // raiseError=false returns the 404 response; the verb helpers reject.
    // request args: method, url, headers, body, params, basicAuth, bearerAuth,
    // allowRedirect, keepAlive, httpVersion, raiseError, send.
    const notFound = await session.request(
      'GET', base + '/missing', undefined, undefined, undefined, undefined,
      undefined, undefined, undefined, undefined, false,
    )
    assert.strictEqual(notFound.status, 404)
    assert.strictEqual(notFound.ok, false)
    assert.throws(() => notFound.raiseForStatus())
    await assert.rejects(session.get(base + '/missing'))

    const deleted = await session.request('DELETE', base + '/thing')
    assert.strictEqual(deleted.status, 204)

    // Pass a LocalPath (an Io handle) as the body: streamed off disk, echoed.
    const { LocalPath } = require('..')
    const os = require('node:os')
    const path = require('node:path')
    const p = path.join(os.tmpdir(), `yggdryl_upload_${process.pid}.bin`)
    new LocalPath(p).write(Buffer.from('file-streamed-upload'))
    const uploaded = await session.put(base + '/up', new LocalPath(p))
    assert.deepStrictEqual(uploaded.content, Buffer.from('file-streamed-upload'))
  } finally {
    server.close()
  }
})

test('setCookie seeds the jar', () => {
  const session = new HttpSession()
  session.setCookie('http://example.com/', 'sid', 'abc123')
  assert.strictEqual(session.cookies.sid, 'abc123')
})

test('module-level verbs use the shared session', async () => {
  const yggdryl = require('..')
  const { server, port } = await startServer()
  const base = `http://127.0.0.1:${port}`
  try {
    const got = await yggdryl.get(base + '/')
    assert.strictEqual(got.status, 200)
    assert.strictEqual(got.text(), 'hello world')

    const posted = await yggdryl.post(base + '/submit', Buffer.from('ping'))
    assert.deepStrictEqual(posted.content, Buffer.from('ping'))

    // DELETE has no module-level verb (JS reserved word); use request().
    const deleted = await yggdryl.request('DELETE', base + '/thing')
    assert.strictEqual(deleted.status, 204)
  } finally {
    server.close()
  }
})

test('baseUrl resolves relative targets', async () => {
  const { server, port } = await startServer()
  const base = `http://127.0.0.1:${port}`
  try {
    const session = new HttpSession(undefined, undefined, undefined, base + '/')
    assert.strictEqual(session.baseUrl, base + '/')
    // A relative target reaches the server (the echo handler replies 200).
    assert.strictEqual((await session.get('some/path')).status, 200)
    // An absolute URL bypasses the base.
    assert.strictEqual((await session.get(base + '/')).status, 200)
  } finally {
    server.close()
  }
})

test('basicAuth and bearerAuth set a default Authorization header', async () => {
  const { server, port } = await startServer()
  const base = `http://127.0.0.1:${port}`
  const opts = [undefined, undefined, undefined, undefined, undefined, undefined, undefined, undefined, undefined]
  try {
    // basicAuth: a [username, password] pair.
    const basic = new HttpSession(...opts, ['Aladdin', 'open sesame'])
    assert.strictEqual(
      (await basic.get(base + '/')).header('x-auth-back'),
      'Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ==',
    )
    // bearerAuth: a token.
    const bearer = new HttpSession(...opts, undefined, 'tok-123')
    assert.strictEqual((await bearer.get(base + '/')).header('x-auth-back'), 'Bearer tok-123')
  } finally {
    server.close()
  }
})

test('setBaseUrl configures the shared singleton', async () => {
  const yggdryl = require('..')
  const { server, port } = await startServer()
  const base = `http://127.0.0.1:${port}`
  try {
    yggdryl.setBaseUrl(base + '/')
    assert.strictEqual((await yggdryl.get('/')).text(), 'hello world')
  } finally {
    // Reset so other tests' absolute-URL module verbs are unaffected.
    yggdryl.setBaseUrl('http://127.0.0.1:1')
    server.close()
  }
})

const CA_FIXTURE = `-----BEGIN CERTIFICATE-----
MIIBQjCB9aADAgECAhQuzAiSQcN9LmU+b23fQ4OnlJr4nzAFBgMrZXAwFzEVMBMG
A1UEAwwMeWdnZHJ5bC10ZXN0MB4XDTI2MDYyNTE4MDczOFoXDTM2MDYyMjE4MDcz
OFowFzEVMBMGA1UEAwwMeWdnZHJ5bC10ZXN0MCowBQYDK2VwAyEAxQDw21VJgXZq
oYc6cXjHtCyGS+Xhu4OzPcRqzez2t8yjUzBRMB0GA1UdDgQWBBS8VDtYTuBsTuVe
Cc9+2uF8BKgWHzAfBgNVHSMEGDAWgBS8VDtYTuBsTuVeCc9+2uF8BKgWHzAPBgNV
HRMBAf8EBTADAQH/MAUGAytlcANBAKXArPIcky5wHp+VgiKw954G3+1I1PQzmpfJ
r9/00T2PpD5GwhdzsrH/liNZug/eMW7w38c0zU0A05lLhgZEIAM=
-----END CERTIFICATE-----
`

test('CA certificate installer', () => {
  assert.strictEqual(new HttpSession().caCertCount, 0)
  const args = [undefined, undefined, undefined, undefined, undefined, undefined, undefined]
  const trusted = new HttpSession(...args, Buffer.from(CA_FIXTURE))
  assert.strictEqual(trusted.caCertCount, 1)
  // Undecodable PEM is rejected at install time.
  assert.throws(() =>
    new HttpSession(...args, Buffer.from('-----BEGIN CERTIFICATE-----\nnot-base64!\n-----END CERTIFICATE-----')),
  )
})

test('brotli response auto-decodes with json and accessors', async () => {
  const { server, port } = await startServer()
  const base = `http://127.0.0.1:${port}`
  try {
    const r = await new HttpSession().get(base + '/brotli')
    assert.strictEqual(r.contentEncoding, 'br')
    assert.strictEqual(r.compression, 'brotli')
    assert.strictEqual(r.mimeType, 'application/json')
    // mediaType combines Content-Type + Content-Encoding (inner → outer).
    assert.deepStrictEqual(r.mediaType, ['application/json', 'application/x-brotli'])
    assert.deepStrictEqual(r.json(), { msg: 'brotli over the wire', n: 7 })

    // The performant byte result is a yggdryl BytesIO handle — parse it in Rust with
    // no native copy; `content` gives a native Buffer when needed.
    const handle = r.io
    assert.deepStrictEqual(handle.json(), { msg: 'brotli over the wire', n: 7 })
    assert.deepStrictEqual(handle.getValue(), r.content)
  } finally {
    server.close()
  }
})

test('readTimeout, keepAlive seconds and copy', async () => {
  // readTimeout defaults to 120s and is the 12th constructor option.
  assert.strictEqual(new HttpSession().readTimeout, 120)
  const opts = Array(11).fill(undefined)
  assert.strictEqual(new HttpSession(...opts, 5).readTimeout, 5)
  // copy() carries configuration into an independent session.
  assert.strictEqual(new HttpSession(...opts, 9).copy().readTimeout, 9)

  // keepAlive is a TTL in seconds; a request succeeds whatever the value.
  const { server, port } = await startServer()
  const base = `http://127.0.0.1:${port}`
  try {
    const session = new HttpSession()
    // request args: method, url, headers, body, params, basicAuth, bearerAuth,
    // allowRedirect, keepAlive, httpVersion, raiseError, send.
    const closed = await session.request(
      'GET', base + '/', undefined, undefined, undefined, undefined, undefined, undefined, 0,
    )
    assert.strictEqual(closed.status, 200)
    const pooled = await session.request(
      'GET', base + '/', undefined, undefined, undefined, undefined, undefined, undefined, 30,
    )
    assert.strictEqual(pooled.status, 200)
  } finally {
    server.close()
  }
})

test('verify and proxy options', () => {
  assert.strictEqual(new HttpSession().verify, true)
  const insecure = new HttpSession(undefined, undefined, undefined, undefined, undefined, false)
  assert.strictEqual(insecure.verify, false)
  const proxied = new HttpSession(
    undefined, undefined, undefined, undefined, undefined, undefined, 'http://127.0.0.1:8080',
  )
  assert.ok(proxied.proxy.includes('127.0.0.1:8080'))
  assert.throws(
    () => new HttpSession(undefined, undefined, undefined, undefined, undefined, undefined, 'not a url'),
  )
})

test('http version negotiation', async () => {
  const { server, port } = await startServer()
  const base = `http://127.0.0.1:${port}`
  try {
    // The session default is "auto"; a response reports HTTP/1.1 (the only wired
    // transport today).
    const session = new HttpSession()
    assert.strictEqual(session.httpVersion, 'auto')
    const r = await session.get(base + '/')
    assert.strictEqual(r.httpVersion, 'HTTP/1.1')

    // A session can default to a version…
    const pinned = new HttpSession(undefined, undefined, undefined, undefined, '2')
    assert.strictEqual(pinned.httpVersion, 'HTTP/2')
    // …but pinning HTTP/2 (no transport yet) rejects rather than downgrading.
    await assert.rejects(pinned.get(base + '/'))
    // The per-request override rejects the same way.
    await assert.rejects(
      // httpVersion is the 10th arg (…, allowRedirect, keepAlive, httpVersion).
      session.request(
        'GET', base + '/', undefined, undefined, undefined, undefined, undefined, undefined, undefined, '3',
      ),
    )
  } finally {
    server.close()
  }
})

test('send=false returns an unsent response holding the request', async () => {
  const { HttpRequest } = require('..')
  const { server, port } = await startServer()
  const base = `http://127.0.0.1:${port}`
  try {
    const session = new HttpSession('yggdryl-test')
    // send is the 10th positional arg of get(); pass it false to build only.
    const unsent = await session.get(
      base + '/', { 'X-Echo': 'tweak-me' }, undefined, undefined, undefined,
      undefined, undefined, undefined, undefined, false,
    )
    assert.strictEqual(unsent.isSent, false)
    assert.strictEqual(unsent.status, 0)
    const request = unsent.request
    assert.ok(request instanceof HttpRequest)
    assert.strictEqual(request.method, 'GET')
    assert.strictEqual(request.url, base + '/')
    // prepare merged the session default header into the embedded request.
    assert.strictEqual(request.header('user-agent'), 'yggdryl-test')
    assert.strictEqual(request.header('x-echo'), 'tweak-me')

    // The unsent response can be dispatched later.
    const sent = await unsent.send()
    assert.strictEqual(sent.status, 200)
    assert.strictEqual(sent.text(), 'hello world')
  } finally {
    server.close()
  }
})

test('a sent response reports its request', async () => {
  const { server, port } = await startServer()
  const base = `http://127.0.0.1:${port}`
  try {
    const session = new HttpSession()
    const r = await session.post(base + '/submit', Buffer.from('hi'))
    assert.strictEqual(r.isSent, true)
    assert.strictEqual(r.request.method, 'POST')
    assert.strictEqual(r.request.url, base + '/submit')
  } finally {
    server.close()
  }
})

test('HttpRequest builds and sends, and a session dispatches a prebuilt request', async () => {
  const { HttpRequest } = require('..')
  const { server, port } = await startServer()
  const base = `http://127.0.0.1:${port}`
  try {
    const request = new HttpRequest('GET', base + '/', { 'X-Echo': 'via-request' })
    assert.strictEqual(request.method, 'GET')
    const r = await request.send()
    assert.strictEqual(r.status, 200)
    assert.strictEqual(r.header('x-echo-back'), 'via-request')

    // A session can dispatch a prebuilt request too (the centralised send path).
    const echoed = await new HttpSession().send(request)
    assert.strictEqual(echoed.header('x-echo-back'), 'via-request')
  } finally {
    server.close()
  }
})

test('verb config args build the request (headers + bearer auth)', async () => {
  const { server, port } = await startServer()
  const base = `http://127.0.0.1:${port}`
  try {
    const session = new HttpSession()
    // headers (2nd) + bearerAuth (5th) configured straight from the signature.
    const r = await session.get(base + '/', { 'X-Echo': 'kw' }, undefined, undefined, 'tok-xyz')
    assert.strictEqual(r.header('x-echo-back'), 'kw')
    assert.strictEqual(r.header('x-auth-back'), 'Bearer tok-xyz')
  } finally {
    server.close()
  }
})
