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
      const chunks = []
      req.on('data', (c) => chunks.push(c))
      req.on('end', () => {
        const body = Buffer.concat(chunks)
        if (req.method === 'GET') {
          res.writeHead(200, {
            'Content-Type': 'text/plain',
            'X-Echo-Back': req.headers['x-echo'] || '',
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

    const posted = await session.post(base + '/submit', Buffer.from('ping-payload'))
    assert.strictEqual(posted.status, 201)
    assert.deepStrictEqual(posted.content, Buffer.from('ping-payload'))

    // Default header, then a per-request override.
    const withDefault = new HttpSession(undefined, { 'X-Echo': 'from-default' })
    assert.strictEqual((await withDefault.get(base + '/')).header('x-echo-back'), 'from-default')
    const overridden = await withDefault.request('GET', base + '/', { 'X-Echo': 'from-request' })
    assert.strictEqual(overridden.header('x-echo-back'), 'from-request')

    // raiseError=false returns the 404 response; the verb helpers reject.
    const notFound = await session.request('GET', base + '/missing', undefined, undefined, false)
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
