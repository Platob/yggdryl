'use strict'

const assert = require('node:assert/strict')
const fs = require('node:fs')
const path = require('node:path')
const { test } = require('node:test')

const root = path.join(__dirname, '..', '..')

test('browser UI subpaths resolve without loading the native addon', async () => {
  const before = new Set(Object.keys(require.cache))
  const [core, adapters, fix] = await Promise.all([
    import('yggdryl/ui'),
    import('yggdryl/ui/yggdryl'),
    import('yggdryl/ui/fix'),
  ])
  assert.equal(typeof core.make, 'function')
  assert.equal(typeof adapters.fixManifest, 'function')
  assert.equal(typeof fix.fixRegistryEditor, 'function')
  const newlyLoadedNative = Object.keys(require.cache).filter(
    (file) => !before.has(file) && (file.endsWith('.node') || file.endsWith('binding.js')),
  )
  assert.deepEqual(newlyLoadedNative, [])
})

test('the separately exported stylesheet is present in the published source tree', () => {
  const manifest = JSON.parse(fs.readFileSync(path.join(root, 'package.json'), 'utf8'))
  assert.equal(manifest.exports['./ui.css'], './ui/styles.css')
  assert.equal(fs.existsSync(path.join(root, 'ui', 'styles.css')), true)
})
