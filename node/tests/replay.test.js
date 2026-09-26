'use strict'

// `yggdryl/replay`, the entry and its command line: `node/replay.js`.

const assert = require('node:assert/strict')
const { spawn } = require('node:child_process')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { after, test } = require('node:test')

const replay = require('yggdryl/replay')

const ENTRY = path.join(__dirname, '..', 'replay.js')

const made = []
after(() => {
  for (const folder of made) fs.rmSync(folder, { recursive: true, force: true })
})

/** A fresh temporary folder, removed when the file's tests are done. */
function scratch(prefix) {
  const folder = fs.mkdtempSync(path.join(os.tmpdir(), prefix))
  made.push(folder)
  return folder
}

test('the package exports the entry as yggdryl/replay', () => {
  assert.equal(require.resolve('yggdryl/replay'), ENTRY)
  assert.deepEqual(Object.keys(replay).sort(), [
    'bookAt',
    'bookJson',
    'booksBetween',
    'booksJson',
    'createReplayServer',
    'indexBooks',
    'loadSource',
    'main',
    'parseArgs',
    'refusalText',
    'rowsOf',
    'synthetic',
    'toJson',
    'walk',
  ])
  assert.equal(replay.createReplayServer, require('../replay/server.js').createReplayServer)
  assert.equal(replay.synthetic.books, require('../replay/synthetic.js').books)
})

test('parseArgs: one source, every option read once, the defaults in one place', () => {
  assert.deepEqual(replay.parseArgs(['synthetic']), {
    source: 'synthetic',
    port: 0,
    snapshotMillis: 0,
    global: false,
    rowheader: undefined,
    sendingTime: undefined,
    registry: undefined,
    web: undefined,
  })
  assert.deepEqual(
    replay.parseArgs([
      '--port', '8080', 'capture.log', '--snapshot-millis', '250', '--global', '--rowheader', '^(?P<x>\\d+) ',
      '--sending-time', '2024-01-02T10:15:30Z', '--registry', 'config/fix', '--web', 'w',
    ]),
    {
      source: 'capture.log',
      port: 8080,
      snapshotMillis: 250,
      global: true,
      rowheader: '^(?P<x>\\d+) ',
      sendingTime: '2024-01-02T10:15:30Z',
      registry: 'config/fix',
      web: 'w',
    },
  )
})

test('parseArgs refuses what it cannot read, by name', () => {
  assert.throws(() => replay.parseArgs([]), /expected a source/)
  assert.throws(() => replay.parseArgs(['a', 'b']), { message: 'expected one source, got "a" and "b"' })
  assert.throws(() => replay.parseArgs(['a', '--port']), { message: '--port: expected a value' })
  assert.throws(() => replay.parseArgs(['a', '--port', '-1']), { message: '--port: expected a non-negative whole number, got "-1"' })
  assert.throws(() => replay.parseArgs(['a', '--snapshot-millis', '1.5']), /--snapshot-millis: expected a non-negative whole number/)
  assert.throws(() => replay.parseArgs(['a', '--verbose']), /unknown option --verbose; the options are --global, --port/)
  assert.throws(() => replay.parseArgs(['a', '--state', 'st']), /unknown option --state/)
})

test('the command line serves a source until interrupted', async () => {
  const child = spawn(process.execPath, [ENTRY, 'synthetic', '--port', '0'], {
    stdio: ['ignore', 'pipe', 'pipe'],
    timeout: 60_000,
  })
  const exited = new Promise((resolve) => child.once('exit', resolve))
  try {
    let stderr = ''
    child.stderr.on('data', (chunk) => {
      stderr += chunk
    })
    const url = await new Promise((resolve, reject) => {
      let stdout = ''
      child.stdout.on('data', (chunk) => {
        stdout += chunk
        if (stdout.includes('\n')) resolve(stdout.split('\n')[0])
      })
      child.once('exit', (code) => reject(new Error(`the command line exited with ${code}: ${stderr}`)))
    })
    assert.match(url, /^http:\/\/127\.0\.0\.1:\d+\/$/)
    const answer = await fetch(new URL('/api/sources', url))
    assert.equal(answer.status, 200)
    const { sources } = await answer.json()
    assert.deepEqual(sources.map((source) => [source.id, source.operations, source.symbols]), [
      ['synthetic', 23, ['ALPHA', 'BETA', 'GLOBAL']],
    ])
    assert.match(stderr, /^source synthetic: 23 operations, symbols ALPHA, BETA\n/)
  } finally {
    child.kill('SIGINT')
    await exited
  }
})

test('the command line refuses a source it cannot load, verbatim, and exits 1', async () => {
  const file = path.join(scratch('yggdryl-replay-cli-'), 'corrupt.parquet')
  fs.writeFileSync(file, 'not parquet')
  let expected
  try {
    replay.loadSource(file)
  } catch (error) {
    expected = error.message
  }
  assert.match(expected, /Invalid Parquet file/)
  const child = spawn(process.execPath, [ENTRY, file], { stdio: ['ignore', 'pipe', 'pipe'], timeout: 60_000 })
  let stderr = ''
  child.stderr.on('data', (chunk) => {
    stderr += chunk
  })
  const code = await new Promise((resolve) => child.once('exit', resolve))
  assert.equal(code, 1)
  assert.equal(stderr, `${expected}
`)
})

test('main answers the URL and a close that stops serving', async () => {
  // It prints the URL and a line about the source, as the command line does.
  const served = await replay.main(['synthetic'])
  try {
    assert.match(served.url, /^http:\/\/127\.0\.0\.1:\d+\/$/)
    const answer = await fetch(new URL('/api/sources/synthetic/books?symbol=BETA', served.url))
    assert.equal(answer.status, 200)
  } finally {
    await served.close()
  }
  assert.equal(process.listenerCount('SIGINT'), 0)
})
