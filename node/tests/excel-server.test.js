'use strict'

const assert = require('node:assert/strict')
const fs = require('node:fs')
const http = require('node:http')
const os = require('node:os')
const path = require('node:path')
const test = require('node:test')

const arrow = require('apache-arrow')
const { Sheet, Workbook } = require('yggdryl')

test('a Node HTTP server imports records and serves the saved workbook', async (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-workbook-server-'))
  const file = path.join(root, 'inventory.xlsx')
  const server = http.createServer(async (request, response) => {
    try {
      if (request.method === 'POST' && request.url === '/import') {
        let body = ''
        for await (const chunk of request) body += chunk
        const rows = JSON.parse(body)
        const table = new arrow.Table({
          sku: arrow.vectorFromArray(rows.map((row) => row.sku), new arrow.Utf8()),
          quantity: arrow.vectorFromArray(rows.map((row) => row.quantity), new arrow.Int32()),
          price: arrow.vectorFromArray(rows.map((row) => row.price), new arrow.Float64()),
        })
        const workbook = new Workbook()
        workbook.insertSheet(Sheet.fromSerie('Inventory', table))
        workbook.writeInto(file)
        response.writeHead(201, { 'content-type': 'application/json' })
        response.end(JSON.stringify({ bytes: fs.statSync(file).size }))
        return
      }
      if (request.method === 'GET' && request.url === '/workbook.xlsx') {
        response.writeHead(200, {
          'content-type': 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
        })
        response.end(fs.readFileSync(file))
        return
      }
      response.writeHead(404).end()
    } catch (error) {
      response.writeHead(500, { 'content-type': 'application/json' })
      response.end(JSON.stringify({ error: error.message }))
    }
  })

  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
  t.after(async () => {
    await new Promise((resolve, reject) => server.close((error) => (error ? reject(error) : resolve())))
    fs.rmSync(root, { recursive: true, force: true })
  })

  const address = server.address()
  const origin = `http://127.0.0.1:${address.port}`
  const imported = await fetch(`${origin}/import`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify([
      { sku: 'A-100', quantity: 4, price: 2.5 },
      { sku: 'B-200', quantity: 0, price: 7.25 },
    ]),
  })
  assert.equal(imported.status, 201)
  const receipt = await imported.json()
  assert.equal(receipt.bytes, fs.statSync(file).size)
  assert.ok(receipt.bytes > 0)

  const downloaded = await fetch(`${origin}/workbook.xlsx`)
  assert.equal(downloaded.status, 200)
  const workbook = Workbook.fromBytes(Buffer.from(await downloaded.arrayBuffer()))
  assert.deepEqual(workbook.sheetNames, ['Inventory'])
  const sheet = workbook.sheet('Inventory')
  assert.deepEqual(sheet.row(0).cells().map((cell) => cell.value.asJs()), ['sku', 'quantity', 'price'])
  assert.equal(sheet.cell('A2').value.asJs(), 'A-100')
  assert.equal(sheet.cell('B3').value.asJs(), 0)
  assert.equal(sheet.cell('C3').value.asJs(), 7.25)
})
