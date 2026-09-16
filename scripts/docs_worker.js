// Run documentation examples inside one long-lived Node process.
//
// A JavaScript example spends milliseconds on what it demonstrates and most of
// a second loading the native addon and Apache Arrow. Running one process per
// example paid that load once per block; this worker pays it once per core,
// then compiles block after block as its own CommonJS module - a fresh scope
// over a warm require cache.
//
// Protocol, one JSON object per line in each direction:
//
//   in  - {"path": "/tmp/.../uri_index_0.js"}
//   out - {"ok": true} | {"ok": false, "detail": "..."}
//
// Requests arrive on stdin and answers leave on stdout, whose `write` is
// captured for the duration of a block so an example's own output cannot
// corrupt the protocol.

'use strict'

const fs = require('node:fs')
const path = require('node:path')
const readline = require('node:readline')
const Module = require('node:module')

const warm = process.argv.slice(2)

// The protocol writes to the descriptor itself, so it is unaffected by the
// capture installed over `process.stdout.write` below - and needs no name for
// a stream that Windows does not give one.
const emit = (answer) => fs.writeSync(1, `${JSON.stringify(answer)}\n`)

let captured = []
const record = (chunk) => {
  captured.push(typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString('utf8'))
  return true
}
process.stdout.write = record
process.stderr.write = record

// The requires every block pays for, paid once.
for (const name of warm) {
  try {
    require(name)
  } catch {
    // Left to the first block to report against its own code.
  }
}

const home = process.cwd()

function runBlock(file) {
  captured = []
  try {
    const code = fs.readFileSync(file, 'utf8')
    const block = new Module(file, null)
    block.filename = file
    block.paths = Module._nodeModulePaths(path.dirname(file))
    block._compile(code, file)
    return { ok: true }
  } catch (error) {
    const printed = captured.join('').trim()
    const detail = (error && error.stack) || String(error)
    return { ok: false, detail: printed ? `${printed}\n${detail}` : detail }
  } finally {
    // A block that wandered leaves the next one where it started.
    process.chdir(home)
  }
}

;(async () => {
  const lines = readline.createInterface({ input: process.stdin, crlfDelay: Infinity })
  for await (const line of lines) {
    const trimmed = line.trim()
    if (!trimmed) continue
    emit(runBlock(JSON.parse(trimmed).path))
  }
})()
