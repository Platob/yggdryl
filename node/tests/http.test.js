'use strict'

const assert = require('node:assert/strict')
const fs = require('node:fs')
const nodeHttp = require('node:http')
const os = require('node:os')
const path = require('node:path')
const { after, before, test } = require('node:test')
const { Worker } = require('node:worker_threads')
const arrow = require('apache-arrow')

const { Field, IOBase, Scalar, Url, http } = require('yggdryl')

// The outside implementation every client test talks to: `node:http` in a
// worker thread. The binding's exchanges are synchronous, so a server on this
// thread's own event loop could never answer them; a worker has its own loop.
const FIXTURE = String.raw`
const { parentPort, workerData } = require('node:worker_threads')
const http = require('node:http')

let origin = ''
const blob = Buffer.alloc(100000)
for (let index = 0; index < blob.length; index += 1) blob[index] = index % 251
const parquet = Buffer.from(workerData.parquet)

function send(res, status, headers, body) {
  const bytes = body === undefined ? Buffer.alloc(0) : Buffer.from(body)
  res.writeHead(status, { 'content-length': bytes.length, ...headers })
  res.end(bytes)
}

function json(res, status, value, headers = {}) {
  send(res, status, { 'content-type': 'application/json', ...headers }, JSON.stringify(value))
}

// Bytes with ranges, as a static file server answers them.
function ranged(req, res, bytes, type) {
  const headers = { 'content-type': type, 'accept-ranges': 'bytes' }
  const range = /^bytes=(\d+)-(\d*)$/.exec(req.headers.range ?? '')
  if (range === null) return send(res, 200, headers, bytes)
  const start = Number(range[1])
  const end = range[2] === '' ? bytes.length - 1 : Math.min(Number(range[2]), bytes.length - 1)
  if (start >= bytes.length) {
    return send(res, 416, { 'content-range': 'bytes */' + bytes.length })
  }
  send(res, 206, { ...headers, 'content-range': 'bytes ' + start + '-' + end + '/' + bytes.length },
    bytes.subarray(start, end + 1))
}

function route(req, res, url, body) {
  switch (url.pathname) {
    case '/echo':
      return json(res, 200, {
        method: req.method,
        query: [...url.searchParams],
        headers: req.headers,
        body: body.toString('utf8'),
      })
    case '/text':
      return send(res, 200, { 'content-type': 'text/plain; charset=utf-8' }, 'hello')
    case '/redirect':
      return send(res, 302, { location: '/text' })
    case '/missing':
      return send(res, 404, { 'content-type': 'text/plain' }, 'no such thing\nsecond line')
    case '/blob':
      return ranged(req, res, blob, 'application/octet-stream')
    case '/rows.json':
      return json(res, 200, [
        { symbol: 'AAPL', price: 1.5 },
        { symbol: 'MSFT', price: 2.5 },
      ])
    case '/trades.parquet':
      return ranged(req, res, parquet, 'application/vnd.apache.parquet')
    case '/linked': {
      const page = Number(url.searchParams.get('page') ?? '1')
      const headers = page < 3 ? { link: '<' + origin + '/linked?page=' + (page + 1) + '>; rel="next"' } : {}
      return json(res, 200, { data: [{ id: page * 10 + 1 }, { id: page * 10 + 2 }] }, headers)
    }
    case '/cursor': {
      const cursor = url.searchParams.get('cursor')
      if (cursor === null) return json(res, 200, { data: [{ id: 1 }, { id: 2 }], next_cursor: 'b' })
      if (cursor === 'b') return json(res, 200, { data: [{ id: 3 }] })
      return json(res, 400, { error: 'unknown cursor ' + cursor })
    }
    default:
      return send(res, 404, {}, 'unrouted')
  }
}

const server = http.createServer((req, res) => {
  const chunks = []
  req.on('data', (chunk) => chunks.push(chunk))
  req.on('end', () => route(req, res, new URL(req.url, origin), Buffer.concat(chunks)))
})
server.listen(0, '127.0.0.1', () => {
  origin = 'http://127.0.0.1:' + server.address().port
  parentPort.postMessage(origin)
})
`

let worker
let origin

before(async () => {
  // The parquet body is written by the binding itself, from an Arrow JS table.
  const sink = IOBase.fromBytes()
  sink.mediaType = 'application/vnd.apache.parquet'
  sink.overwriteArrowTable(
    arrow.tableFromArrays({
      symbol: ['AAPL', 'MSFT', 'NVDA'],
      qty: Int32Array.from([1, 2, 3]),
    }),
  )
  worker = new Worker(FIXTURE, { eval: true, workerData: { parquet: sink.readBytes() } })
  origin = await new Promise((resolve, reject) => {
    worker.once('message', resolve)
    worker.once('error', reject)
  })
})

after(async () => {
  await worker.terminate()
})

// An outside client for the core's `Server`: `node:http` on this thread's
// loop, while the server answers from its own threads.
function call(method, url, { headers, body } = {}) {
  return new Promise((resolve, reject) => {
    const request = nodeHttp.request(url, { method, headers, agent: false }, (response) => {
      const chunks = []
      response.on('data', (chunk) => chunks.push(chunk))
      response.on('error', reject)
      response.on('end', () =>
        resolve({
          status: response.statusCode,
          headers: response.headers,
          body: Buffer.concat(chunks),
        }),
      )
    })
    request.on('error', reject)
    request.end(body)
  })
}

function scratch() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-http-'))
}

test('refusals name what they refuse before anything goes out', () => {
  assert.throws(() => new http.Session(42), /baseUrl must be a string, a Url or a URL/)
  assert.throws(() => new http.Session(undefined, { retries: 3 }), /unknown session option "retries"/)
  assert.throws(
    () => new http.Session(undefined, { options: { timeout: 'soon' } }),
    /timeout/,
  )
  assert.throws(() => new http.Session(undefined, { timeout: -1 }), /timeout must be a finite, non-negative/)
  assert.throws(() => new http.Headers(5), /headers must be a Headers, a Map/)
  assert.throws(() => new http.Headers([['only-a-name']]), /\[name, value\] pairs/)
  assert.throws(() => new http.Headers({ 'x-a': {} }), /must be a string, a finite number/)
  assert.throws(() => new http.Headers({ 'bad name': 'v' }), /header/)
  assert.throws(() => new http.Request('BREW', 'http://127.0.0.1/'), /BREW/)
  assert.throws(() => new http.Request('GET', 'ftp://example.com/x'), /http url/)
  assert.throws(() => http.Response.fromBytes(Buffer.from('not a response')), /http message/)

  const session = new http.Session()
  assert.throws(() => session.get('/relative'), /http url/)
  assert.throws(() => session.get('http://127.0.0.1/', { bogus: 1 }), /unknown request option "bogus"/)
  assert.throws(
    () => session.post('http://127.0.0.1/', { data: 'a', json: { b: 1 } }),
    /one body/,
  )
  assert.throws(
    () => session.get('http://127.0.0.1/', { auth: { bearer: 'x', username: 'y' } }),
    /auth must be exactly one of/,
  )
  assert.throws(() => session.get('http://127.0.0.1/', { auth: ['one'] }), /\[username, password\] pair/)
  assert.throws(() => session.get('http://127.0.0.1/', { pagination: 'sideways' }), /pagination/)
  assert.throws(() => session.sendAll([], -1), /concurrency must be a non-negative whole number/)
  // Nothing above sent a request.
  assert.equal(session.stats.requests, 0)
})

test('get sends params and headers and reads json, text and content', () => {
  const session = new http.Session(origin + '/', {
    headers: { 'X-Default': 'session' },
    auth: { bearer: 'k-123' },
  })
  const response = session.get('echo', {
    params: { limit: 10, tag: ['a', 'b'] },
    headers: new http.Headers([['X-Call', 'one']]),
  })
  assert.equal(response.statusCode, 200)
  assert.equal(response.reason, 'OK')
  assert.equal(response.ok, true)
  assert.ok(response.url instanceof Url)
  assert.equal(response.url.toString(), `${origin}/echo?limit=10&tag=a&tag=b`)
  assert.equal(response.headers.contentType, 'application/json')
  assert.equal(response.mediaType.toString(), 'application/json')
  assert.ok(response.elapsed >= 0)

  const document = response.json()
  assert.equal(document.method, 'GET')
  assert.deepEqual(document.query, [['limit', '10'], ['tag', 'a'], ['tag', 'b']])
  assert.equal(document.headers['x-default'], 'session')
  assert.equal(document.headers['x-call'], 'one')
  assert.equal(document.headers.authorization, 'Bearer k-123')
  assert.ok(response.scalar() instanceof Scalar)
  assert.equal(JSON.parse(response.text()).method, 'GET')
  assert.ok(Buffer.isBuffer(response.content()))
  assert.equal(response.content().toString('utf8'), response.text())

  const text = http.get(`${origin}/text`)
  assert.equal(text.text(), 'hello')
  assert.equal(text.encoding, 'utf-8')
  assert.equal(text.contentLength, 5)

  assert.equal(session.stats.gets, 1)
})

test('bodies go out as data, json and form, the caller headers winning', () => {
  const session = new http.Session()
  const posted = session.post(`${origin}/echo`, { json: { a: 1, b: [true, null] } }).json()
  assert.equal(posted.method, 'POST')
  assert.deepEqual(JSON.parse(posted.body), { a: 1, b: [true, null] })
  assert.equal(posted.headers['content-type'], 'application/json')

  const put = session.put(`${origin}/echo`, {
    data: 'raw text',
    headers: { 'Content-Type': 'text/plain' },
  }).json()
  assert.equal(put.body, 'raw text')
  assert.equal(put.headers['content-type'], 'text/plain')

  const form = session.patch(`${origin}/echo`, { form: { q: 'a b', n: 2 } }).json()
  assert.equal(form.body, 'q=a+b&n=2')
  assert.equal(form.headers['content-type'], 'application/x-www-form-urlencoded')

  const request = new http.Request('POST', new URL(`${origin}/echo`))
    .withJson({ ok: true })
    .withHeader('X-Built', 'yes')
  assert.equal(request.method, 'POST')
  assert.equal(request.body.toString('utf8'), '{"ok":true}')
  const answered = request.send().json()
  assert.equal(answered.headers['x-built'], 'yes')
  assert.equal(answered.body, '{"ok":true}')
})

test('redirects are followed and kept as history', () => {
  const response = http.get(`${origin}/redirect`)
  assert.equal(response.statusCode, 200)
  assert.equal(response.url.toString(), `${origin}/text`)
  assert.equal(response.history.length, 1)
  const [hop] = response.history
  assert.equal(hop.statusCode, 302)
  assert.equal(hop.url, `${origin}/redirect`)
  assert.equal(hop.headers.location, '/text')

  const held = new http.Session().get(`${origin}/redirect`, { followRedirects: false })
  assert.equal(held.statusCode, 302)
  assert.equal(held.isRedirect, true)
  assert.equal(held.history.length, 0)
})

test('a 404 is an answer until raiseForStatus refuses it', () => {
  const response = http.get(`${origin}/missing`)
  assert.equal(response.statusCode, 404)
  assert.equal(response.ok, false)
  assert.equal(response.text(), 'no such thing\nsecond line')
  assert.throws(() => response.raiseForStatus(), (error) => {
    assert.match(error.message, /404/)
    assert.match(error.message, /no such thing/)
    assert.doesNotMatch(error.message, /second line/)
    return true
  })
  const fine = http.get(`${origin}/text`)
  assert.equal(fine.raiseForStatus(), fine)
})

test('a streamed body reads through intoIOBase, which moves the answer', () => {
  const response = new http.Session().stream(`${origin}/blob`)
  assert.equal(response.statusCode, 200)
  assert.equal(response.headers.acceptRanges, true)
  const handle = response.intoIOBase()
  assert.ok(handle instanceof IOBase)
  const bytes = handle.readBytes()
  assert.equal(bytes.length, 100000)
  assert.equal(bytes[250], 250)
  assert.equal(bytes[251], 0)
  assert.throws(() => response.statusCode, /moved into an IOBase/)
  assert.equal(String(response), 'Response(moved)')

  // A held answer is an IOBase over its body too.
  const held = http.get(`${origin}/rows.json`).intoIOBase()
  assert.equal(held.mediaType.toString(), 'application/json')
  assert.deepEqual(JSON.parse(held.readText()), [
    { symbol: 'AAPL', price: 1.5 },
    { symbol: 'MSFT', price: 2.5 },
  ])
  const records = http.get(`${origin}/rows.json`).intoIOBase().readArrowReader().intoTable()
  assert.deepEqual(
    records.toArray().map((row) => row.symbol),
    ['AAPL', 'MSFT'],
  )
  // A record encoding reads from the body the same way.
  const parquet = http.get(`${origin}/trades.parquet`).intoIOBase()
  assert.equal(parquet.mediaType.toString(), 'application/vnd.apache.parquet')
  assert.deepEqual(parquet.readArrowReader().intoTable().getChild('symbol').toArray(), [
    'AAPL',
    'MSFT',
    'NVDA',
  ])
})

test('sendAll answers in the order given, a failure in its place', () => {
  const session = new http.Session(origin + '/')
  const requests = [0, 1, 2, 3].map((index) =>
    new http.Request('GET', `${origin}/echo?i=${index}`).withSession(session),
  )
  requests.push(`${origin}/text`, 'http://127.0.0.1:1/unreachable')
  const answers = [...session.sendAll(requests, 3)]
  assert.equal(answers.length, 6)
  answers.slice(0, 4).forEach((answer, index) => {
    assert.deepEqual(answer.json().query, [['i', String(index)]])
  })
  assert.equal(answers[4].text(), 'hello')
  assert.ok(answers[5] instanceof Error)
})

test('an in-memory handle mounts as a copy of its bytes under its media type', () => {
  const server = http.Server.bind()
  const handle = IOBase.fromBytes(Buffer.from('[1, 2, 3]'))
  server.mount('/rows.json', handle)
  const response = http.get(new URL('rows.json', server.url.toString()).toString())
  assert.equal(response.statusCode, 200)
  assert.equal(response.text(), '[1, 2, 3]')
  // The caller's handle stays its own.
  assert.equal(handle.readText(), '[1, 2, 3]')
  server.shutdown()
})

test('sendAll reads specs and is a walk pulled one answer at a time', () => {
  const session = new http.Session(origin + '/')
  const walk = session.sendAll(
    [
      `${origin}/text`,
      { method: 'POST', url: `${origin}/echo`, json: { n: 1 } },
      { url: `${origin}/echo`, params: { from: 'spec' }, headers: { 'x-probe': 'yes' } },
    ],
    2,
  )
  assert.equal(typeof walk.next, 'function')
  const first = walk.next()
  assert.equal(first.done, false)
  assert.equal(first.value.text(), 'hello')
  const [posted, specified] = [...walk]
  assert.equal(posted.json().method, 'POST')
  assert.deepEqual(JSON.parse(posted.json().body), { n: 1 })
  assert.deepEqual(specified.json().query, [['from', 'spec']])
  assert.equal(specified.json().headers['x-probe'], 'yes')
  assert.throws(() => session.sendAll([{ method: 'GET' }]), /names its url/)
})

test('a closed streamed response stands alone and reopens at its cursor', () => {
  const session = new http.Session()
  const response = session.stream(`${origin}/blob`)
  assert.equal(response.opened, true)
  response.close()
  assert.equal(response.opened, false)
  // The whole body, from the first byte: one ranged GET re-opens it.
  const bytes = response.content()
  assert.equal(bytes.length, 100000)
  assert.equal(bytes[251], 0)
  assert.equal(session.stats.resumes, 1)
})

test('pages walk a Link header and a cursor, and read as Arrow', () => {
  const session = new http.Session()
  const first = session.get(`${origin}/linked`)
  assert.equal(first.links[0].rel[0], 'next')
  assert.equal(first.headers.nextLink, `${origin}/linked?page=2`)
  assert.equal(first.next.url.toString(), `${origin}/linked?page=2`)
  assert.equal(first.next.send().next.url.toString(), `${origin}/linked?page=3`)

  const pages = [...session.pages(`${origin}/linked`)]
  assert.deepEqual(
    pages.map((page) => page.json().data.map((row) => row.id)),
    [[11, 12], [21, 22], [31, 32]],
  )

  const table = session
    .pages(`${origin}/cursor`, { pagination: 'cursor:next_cursor:cursor', records: 'data' })
    .intoArrowReader()
    .intoTable()
  assert.deepEqual(
    table.toArray().map((row) => Number(row.id)),
    [1, 2, 3],
  )

  const declared = new http.Request('GET', `${origin}/cursor`)
    .withPagination('cursor:next_cursor:cursor')
    .withRecords('data')
    .pages()
  const reader = declared.intoArrowReader(Field.from('rows struct<id: int64 not null> not null'))
  assert.equal(reader.field.name, 'rows')
  assert.equal(reader.intoTable().numRows, 3)
  assert.throws(() => declared.next(), /consumed by intoArrowReader/)
})

test('an http URL is an IOBase reading records and ranges', () => {
  const rows = new IOBase(`${origin}/rows.json`)
  assert.equal(rows.url.scheme, 'http')
  assert.equal(rows.mediaType.toString(), 'application/json')
  const table = rows.readArrowReader().intoTable()
  assert.deepEqual(
    table.toArray().map((row) => [row.symbol, row.price]),
    [
      ['AAPL', 1.5],
      ['MSFT', 2.5],
    ],
  )

  // Parquet reads its footer with ranged GETs.
  const trades = IOBase.from(`${origin}/trades.parquet`)
  assert.equal(trades.mediaType.toString(), 'application/vnd.apache.parquet')
  const parquet = trades.readArrowReader().intoTable()
  assert.deepEqual(parquet.getChild('symbol').toArray(), ['AAPL', 'MSFT', 'NVDA'])
  assert.deepEqual([...parquet.getChild('qty').toArray()], [1, 2, 3])

  const blob = new http.Request('GET', `${origin}/blob`).intoIOBase()
  assert.deepEqual([...blob.readRangeBytes(250, 3)], [250, 0, 1])
  assert.equal(blob.size, 100000)

  // A request's IOBase carries its session and headers onto every read.
  const echoed = new http.Session(undefined, { headers: { 'X-Session': 's' } })
  const viaSession = new http.Request('GET', `${origin}/echo`)
    .withSession(echoed)
    .withHeader('X-Request', 'r')
    .intoIOBase()
  const seen = JSON.parse(viaSession.readText()).headers
  assert.equal(seen['x-session'], 's')
  assert.equal(seen['x-request'], 'r')

  // A session is the container over its base URL: a child is the GET of it.
  const container = new http.Session(`${origin}/`).intoIOBase()
  assert.equal(container.kind, 'directory')
  assert.equal(container.joinpath('text').readText(), 'hello')
})

test('Headers read case-insensitively and parse their typed headers', () => {
  const headers = new http.Headers(
    new Map([
      ['Content-Type', 'text/csv; charset=windows-1252'],
      ['Content-Encoding', 'gzip'],
      ['Content-Length', '42'],
      ['Content-Range', 'bytes 0-9/42'],
      ['ETag', 'W/"v1"'],
      ['Last-Modified', 'Sun, 06 Nov 1994 08:49:37 GMT'],
      ['Link', '<https://example.com/p2>; rel="next"; title="two"'],
      ['Accept', 'a'],
    ]),
  )
  assert.equal(headers.get('content-type'), 'text/csv; charset=windows-1252')
  assert.equal(headers.get('CONTENT-TYPE'), headers.get('content-type'))
  assert.equal(headers.get('absent'), null)
  assert.equal(headers.has('etag'), true)
  assert.equal(headers.length, 8)
  assert.deepEqual(headers.keys().slice(0, 2), ['accept', 'content-encoding'])
  assert.deepEqual([...headers][0], ['accept', 'a'])
  assert.equal(JSON.parse(JSON.stringify(headers))['content-length'], '42')
  assert.equal(headers.contentLength, 42)
  assert.equal(headers.charset, 'windows-1252')
  assert.deepEqual(headers.contentEncoding, ['gzip'])
  assert.deepEqual(headers.contentRange, { start: 0, end: 9, total: 42, satisfied: true })
  assert.deepEqual(headers.etag, { opaque: 'v1', weak: true })
  assert.equal(headers.lastModified, 784111777000000000n)
  assert.equal(headers.nextLink, 'https://example.com/p2')
  assert.deepEqual(headers.links[0].parameters, [['title', 'two']])
  assert.equal(headers.acceptRanges, false)
  assert.ok(headers.equals(new http.Headers(headers)))

  const joined = new http.Headers([
    ['Accept', 'a'],
    ['accept', 'b'],
  ])
  assert.equal(joined.get('accept'), 'a, b')
  assert.deepEqual(joined.getAll('accept'), ['a', 'b'])
  assert.throws(() => new http.Headers({ 'Content-Length': 'many' }).contentLength, /content-length/i)
})

test('requests and responses round-trip their wire messages', () => {
  const request = http.Request.fromBytes(
    Buffer.from('POST /rows?limit=2 HTTP/1.1\r\nHost: example.com\r\nContent-Length: 5\r\n\r\nhello'),
  )
  assert.equal(request.method, 'POST')
  assert.equal(request.url.toString(), 'http://example.com/rows?limit=2')
  assert.equal(request.body.toString(), 'hello')
  assert.equal(
    request.intoBytes().toString(),
    'POST /rows?limit=2 HTTP/1.1\r\ncontent-length: 5\r\nhost: example.com\r\n\r\nhello',
  )

  const wire = Buffer.from(
    'HTTP/1.1 200 OK\r\ncontent-length: 16\r\ncontent-type: application/json\r\n\r\n{"orders":[1,2]}',
  )
  const response = http.Response.fromBytes(wire)
  assert.deepEqual(response.json(), { orders: [1, 2] })
  assert.deepEqual(response.intoBytes(), wire)
  assert.equal(response.intoScalar().asJs().status, 200)
})

test('Server serves a mount, fixed answers and faults to an outside client', async (t) => {
  const root = scratch()
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  fs.writeFileSync(path.join(root, 'a.txt'), 'hello world')

  const server = http.Server.bind()
  t.after(() => server.shutdown())
  assert.ok(server.port > 0)
  assert.equal(server.url.toString(), `http://127.0.0.1:${server.port}/`)
  const folder = new IOBase(root)
  server.mount('/files', folder)
  const base = `http://127.0.0.1:${server.port}`

  const got = await call('GET', `${base}/files/a.txt`)
  assert.equal(got.status, 200)
  assert.equal(got.body.toString(), 'hello world')
  assert.equal(got.headers['accept-ranges'], 'bytes')
  assert.match(got.headers['content-type'], /^text\/plain/)

  const ranged = await call('GET', `${base}/files/a.txt`, { headers: { Range: 'bytes=0-4' } })
  assert.equal(ranged.status, 206)
  assert.equal(ranged.body.toString(), 'hello')
  assert.equal(ranged.headers['content-range'], 'bytes 0-4/11')

  const created = await call('PUT', `${base}/files/b.json`, {
    headers: { 'Content-Type': 'application/json' },
    body: '{"b":2}',
  })
  assert.equal(created.status, 201)
  assert.equal(fs.readFileSync(path.join(root, 'b.json'), 'utf8'), '{"b":2}')
  const read = await call('GET', `${base}/files/b.json`)
  assert.equal(read.body.toString(), '{"b":2}')

  // The binding's own client reads the server too: it answers from its own
  // threads, so a synchronous request from this thread is fine.
  assert.equal(http.get(`${base}/files/a.txt`).text(), 'hello world')
  assert.equal(new IOBase(`${base}/files/b.json`).readText(), '{"b":2}')

  // A write through an http IOBase is one PUT, a removal one DELETE.
  const written = new IOBase(`${base}/files/c.txt`)
  written.writeText('via put')
  assert.equal(fs.readFileSync(path.join(root, 'c.txt'), 'utf8'), 'via put')
  written.remove()
  assert.equal(fs.existsSync(path.join(root, 'c.txt')), false)

  const removed = await call('DELETE', `${base}/files/b.json`)
  assert.equal(removed.status, 204)
  assert.equal((await call('GET', `${base}/files/b.json`)).status, 404)

  server.respond('/fixed', 418, { 'X-Kind': 'teapot' }, 'short and stout')
  const fixed = await call('GET', `${base}/fixed`)
  assert.equal(fixed.status, 418)
  assert.equal(fixed.headers['x-kind'], 'teapot')
  assert.equal(fixed.body.toString(), 'short and stout')
  server.respond('/posted', 201, null, null, 'POST')
  assert.equal((await call('POST', `${base}/posted`)).status, 201)
  assert.equal((await call('GET', `${base}/posted`)).status, 404)
  assert.equal(server.unroute('/posted', 'POST'), true)

  server.inject('/files/a.txt', { refuse: 503, retryAfter: 2000 })
  const refused = await call('GET', `${base}/files/a.txt`)
  assert.equal(refused.status, 503)
  assert.equal(refused.headers['retry-after'], '2')
  assert.equal((await call('GET', `${base}/files/a.txt`)).status, 200)

  server.inject('/files/a.txt', 'closeBeforeAnswer')
  await assert.rejects(call('GET', `${base}/files/a.txt`))
  server.inject('/files/a.txt', { cutBodyAt: 3 })
  await assert.rejects(call('GET', `${base}/files/a.txt`))
  assert.throws(() => server.inject('/x', 'explode'), /closeBeforeAnswer/)
  assert.throws(() => server.inject('/x', { delay: 1, refuse: 500 }), /exactly one/)

  const recorded = server.requests
  const [first] = recorded
  assert.equal(first.method, 'GET')
  assert.equal(first.path, '/files/a.txt')
  assert.equal(first.statusCode, 200)
  assert.ok(first.headers instanceof http.Headers)
  const put = recorded.find((entry) => entry.method === 'PUT')
  assert.equal(put.bodyLength, 7)
  assert.equal(put.statusCode, 201)
  assert.ok(recorded.some((entry) => entry.closed))
  assert.equal(server.requestCount, recorded.length)
  server.clearRequests()
  assert.equal(server.requestCount, 0)

  // The mounted handle is a second one on the same location.
  assert.equal(folder.joinpath('a.txt').readText(), 'hello world')

  server.shutdown()
  assert.equal(server.closed, true)
  assert.throws(() => server.url, /shut down/)
  server.shutdown()
})
