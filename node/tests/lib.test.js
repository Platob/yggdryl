'use strict'

// `node/src/lib.rs`: what the addon registers when it loads - the logger the
// core's warnings reach standard error through.

const assert = require('node:assert/strict')
const { spawnSync } = require('node:child_process')
const path = require('node:path')
const test = require('node:test')

test('a data warning reaches standard error once and then is counted', () => {
  // What a parse passes over it says as a warning, deduplicated for the whole
  // process: the first occurrence in full, then a count at each tenfold. A
  // fresh process reads the table from its first entry.
  const script = `
    const { fix } = require(process.argv[1])
    const codec = new fix.FixCodec(new fix.FixRegistry())
    for (let at = 0; at < 10; at += 1) {
      const message = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|52=bad|10=0|'))
      if (message.anomalies.map((held) => held.field).join() !== 'sendingtime') {
        throw new Error('expected the clock as an anomaly')
      }
    }
  `
  const child = spawnSync(process.execPath, ['-e', script, path.join(__dirname, '..')], {
    encoding: 'utf8',
    timeout: 60_000,
  })
  assert.equal(child.status, 0, `child exited ${child.status}:\n${child.stdout}\n${child.stderr}`)
  const warned = child.stderr
    .split('\n')
    .filter((line) => line.includes('FIX clock left unstated'))
  assert.equal(warned.length, 2, child.stderr)
  assert.ok(warned.every((line) => line.startsWith('yggdryl: ')), child.stderr)
  assert.match(warned[0], /\(sendingtime\).*"bad".*later occurrences are counted rather than repeated/)
  assert.match(warned[1], /seen 10 times$/)
})
