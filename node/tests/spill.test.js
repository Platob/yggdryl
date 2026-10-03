'use strict'

// Pins node/src/spill.rs: the bound a column stays resident under and the
// local folder it spills to, read once into the core's `SpillOptions`, and
// the process default the environment states.

const assert = require('node:assert/strict')
const { spawnSync } = require('node:child_process')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const test = require('node:test')

const binding = require('yggdryl')
const { DEFAULT_SPILL_BYTE_SIZE, Field, IOBase, Serie, SpillOptions, Url } = binding

const MiB = 1024n * 1024n

test('the default bounds a column at 64 MiB over the platform temporary folder', () => {
  assert.equal(DEFAULT_SPILL_BYTE_SIZE, 64n * MiB)
  const options = new SpillOptions()
  assert.equal(options.byteSize, DEFAULT_SPILL_BYTE_SIZE)
  assert.equal(options.folder, null)
  assert.equal(options.isNever(), false)
  assert.equal(options.toString(), `SpillOptions(byteSize=${64n * MiB})`)
  // An absent and a null slot, and a null object, are the default.
  for (const stated of [undefined, null, {}, { byteSize: null, folder: null }]) {
    assert.ok(new SpillOptions(stated).equals(options))
  }
})

test('the bound reads a whole number or a bigint, and NEVER spills nothing', () => {
  assert.equal(SpillOptions.NEVER, 2n ** 64n - 1n)
  assert.equal(new SpillOptions({ byteSize: 0 }).byteSize, 0n)
  assert.equal(new SpillOptions({ byteSize: 1_024 }).byteSize, 1_024n)
  assert.equal(new SpillOptions({ byteSize: 2n ** 60n }).byteSize, 2n ** 60n)
  const never = new SpillOptions({ byteSize: SpillOptions.NEVER })
  assert.equal(never.isNever(), true)
  assert.equal(never.toString(), 'SpillOptions(byteSize=never)')
  assert.throws(() => new SpillOptions({ byteSize: -1 }), /byteSize must be a non-negative whole number/)
  assert.throws(() => new SpillOptions({ byteSize: 1.5 }), /byteSize must be a non-negative whole number/)
  assert.throws(
    () => new SpillOptions({ byteSize: -1n }),
    /expected byteSize to fit an unsigned 64-bit integer/,
  )
  assert.throws(
    () => new SpillOptions({ byteSize: 2n ** 64n }),
    /expected byteSize to fit an unsigned 64-bit integer/,
  )
  assert.throws(() => new SpillOptions({ bytes: 0 }), /take byteSize and folder, got "bytes"/)
  assert.throws(() => new SpillOptions(0), /must be an object of byteSize and folder/)
})

test('the folder is any location naming a local folder, one folder however named', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-spill-'))
  try {
    const byPath = new SpillOptions({ byteSize: 0, folder: root })
    const folder = byPath.folder
    assert.ok(folder instanceof IOBase)
    assert.equal(String(folder.url), String(Url.fromPath(root)))
    assert.ok(new SpillOptions({ byteSize: 0, folder: Url.fromPath(root) }).equals(byPath))
    assert.ok(new SpillOptions({ byteSize: 0, folder }).equals(byPath))
    assert.ok(byPath.clone().equals(byPath))
    assert.equal(byPath.equals(new SpillOptions({ byteSize: 0 })), false)
    assert.match(byPath.toString(), /^SpillOptions\(byteSize=0, folder=file:\/\//)
    // A spill file is created under the folder and is gone from its listing
    // as soon as it is opened: the folder lists nothing while the rows are
    // mapped.
    const prices = Serie.fromScalars(
      new Field('price', 'int64', false),
      Array.from({ length: 1_024 }, (_, index) => index),
    )
    prices.spill(byPath)
    assert.equal(prices.isSpilled(), true)
    assert.deepEqual(fs.readdirSync(root), [])
    assert.equal(prices.scalar(1_023).asJs(), 1_023)
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
  assert.throws(
    () => new SpillOptions({ folder: 's3://bucket/spill/' }),
    /expected a local folder to spill to, got s3:\/\/bucket\/spill\//,
  )
})

// One child process per environment: the process default is read once.
function child(script, environment = {}) {
  const run = spawnSync(process.execPath, ['-e', script, path.join(__dirname, '..')], {
    encoding: 'utf8',
    env: { ...process.env, YGGDRYL_SPILL_BYTE_SIZE: '', YGGDRYL_SPILL_FOLDER: '', ...environment },
    timeout: 60_000,
  })
  assert.equal(run.status, 0, `child exited ${run.status}:\n${run.stdout}\n${run.stderr}`)
  return run.stdout.trim()
}

test('fromEnv reads the two variables once, and installEnv states them first', () => {
  const read = `
    const { SpillOptions } = require(process.argv[1])
    const options = SpillOptions.fromEnv()
    process.stdout.write(String(options))
  `
  assert.equal(child(read), `SpillOptions(byteSize=${64n * MiB})`)
  assert.equal(child(read, { YGGDRYL_SPILL_BYTE_SIZE: 'NEVER' }), 'SpillOptions(byteSize=never)')
  assert.equal(child(read, { YGGDRYL_SPILL_BYTE_SIZE: ' 4096 ' }), 'SpillOptions(byteSize=4096)')
  const refused = `
    const { SpillOptions } = require(process.argv[1])
    for (let attempt = 0; attempt < 2; attempt += 1) {
      try {
        SpillOptions.fromEnv()
        process.stdout.write('read')
      } catch (error) {
        process.stdout.write(error.message + '\\n')
      }
    }
  `
  const messages = child(refused, { YGGDRYL_SPILL_BYTE_SIZE: 'lots' }).split('\n')
  // A refused variable is named, and the next call reads it again.
  assert.equal(messages.length, 2)
  for (const message of messages) {
    assert.match(message, /YGGDRYL_SPILL_BYTE_SIZE/)
  }
  const installed = `
    const { Serie, SpillOptions } = require(process.argv[1])
    SpillOptions.installEnv(new SpillOptions({ byteSize: 0 }))
    const answers = [String(SpillOptions.fromEnv())]
    try {
      SpillOptions.installEnv(new SpillOptions())
    } catch (error) {
      answers.push('refused')
    }
    // The process default is what a verb given no options settles under.
    const prices = Serie.fromScalars('price: int64 not null', [1, 2, 3])
    prices.spill()
    answers.push(String(prices.isSpilled()))
    process.stdout.write(answers.join(' | '))
  `
  assert.equal(child(installed), 'SpillOptions(byteSize=0) | refused | true')
})
