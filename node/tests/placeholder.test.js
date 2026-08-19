'use strict'

// Jinja-style `{{ }}` placeholders: a YAML and TOML feature only. JSON is a
// data interchange format, and both the JS boundary and the core refuse the
// pair for it by name - see the dedicated test at the bottom.

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const test = require('node:test')

const { json, toml, yaml } = require('yggdryl')

// The same document, written the way each format spells it. YAML *requires*
// the quotes: a bare `{{ X }}` is a flow mapping.
const DOCUMENTS = [
  [yaml, (scalar) => `value: ${JSON.stringify(scalar)}\n`],
  [toml, (scalar) => `value = ${JSON.stringify(scalar)}\n`],
]

function resolved(scalar, options) {
  return DOCUMENTS.map(([codec, document]) => codec.loads(document(scalar), options).value)
}

test('a whole-scalar placeholder adopts the resolved value type', () => {
  const placeholders = { PORT: 8080, DEBUG: true, HOSTS: ['a', 'b'], NOTHING: null }
  assert.deepEqual(resolved('{{ PORT }}', { placeholders }), [8080, 8080])
  assert.deepEqual(resolved('{{ DEBUG }}', { placeholders }), [true, true])
  assert.deepEqual(resolved('{{ HOSTS }}', { placeholders }), [
    ['a', 'b'],
    ['a', 'b'],
  ])
})

test('an embedded placeholder is textual and stays a string', () => {
  const placeholders = { ROOT: '/var/log', PORT: 8080 }
  assert.deepEqual(resolved('{{ ROOT }}/app', { placeholders }), [
    '/var/log/app',
    '/var/log/app',
  ])
  assert.deepEqual(resolved('h:{{ PORT }}/x', { placeholders }), ['h:8080/x', 'h:8080/x'])
  // A container has no text form inside a larger string.
  assert.throws(
    () => yaml.loads('a: "x{{ HOSTS }}"\n', { placeholders: { HOSTS: ['a'] } }),
    /resolve to a scalar/,
  )
})

test('a missing variable names itself rather than resolving to nothing', () => {
  assert.throws(
    () => yaml.loads('a:\n  b: "{{ MISSING }}"\n', { placeholders: {} }),
    /MISSING[\s\S]*\$\.a\.b|\$\.a\.b[\s\S]*MISSING/,
  )
})

test('a default makes a variable optional and carries its own type', () => {
  assert.deepEqual(resolved('{{ PORT | default(8080) }}', { placeholders: {} }), [8080, 8080])
  assert.deepEqual(resolved('{{ R | default("/tmp") }}', { placeholders: {} }), [
    '/tmp',
    '/tmp',
  ])
  // A supplied value wins over the default.
  assert.deepEqual(resolved('{{ P | default(1) }}', { placeholders: { P: 2 } }), [2, 2])
  // `default` is the only filter there is.
  assert.throws(
    () => yaml.loads('a: "{{ R | upper }}"\n', { placeholders: { R: 'x' } }),
    /default\(LITERAL\)/,
  )
})

test('a doubled opener is a literal one', () => {
  assert.deepEqual(resolved('{{{{ NAME }}', { placeholders: {} }), [
    '{{ NAME }}',
    '{{ NAME }}',
  ])
  assert.throws(() => yaml.loads('a: "{{ NAME"\n', { placeholders: {} }), /unterminated/)
})

test('substitution is off unless asked for, and the environment is its own switch', () => {
  // No options at all: the braces are ordinary text.
  assert.equal(yaml.loads('a: "{{ MISSING }}"\n').a, '{{ MISSING }}')

  const name = 'YGGDRYL_PLACEHOLDER_NODE_VALUE'
  process.env[name] = 'from-environment'
  try {
    const scalar = `{{ ${name} }}`
    // Set, and still not resolved: the environment was not consulted.
    assert.throws(() => resolved(scalar, { placeholders: {} }), /not consulted/)

    assert.deepEqual(resolved(scalar, { environment: true }), [
      'from-environment',
      'from-environment',
    ])
    // The supplied mapping wins.
    assert.deepEqual(
      resolved(scalar, { placeholders: { [name]: 'from-mapping' }, environment: true }),
      ['from-mapping', 'from-mapping'],
    )
  } finally {
    delete process.env[name]
  }
})

test('a document without placeholders parses identically either way', (t) => {
  const document = 'a: plain\nb:\n  - 1\n  - 2\nc:\n  d: null\n'
  assert.deepEqual(yaml.loads(document, { placeholders: { X: 1 } }), yaml.loads(document))

  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-placeholder-'))
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const target = path.join(root, 'config.yaml')
  fs.writeFileSync(target, 'a: "{{ NAME }}"\n')
  const options = { placeholders: { NAME: 'app' } }
  assert.deepEqual(yaml.load(target, options), { a: 'app' })
  assert.deepEqual(yaml.loads(fs.readFileSync(target), options), { a: 'app' })

  // And nothing about the options is guessed for the caller.
  assert.throws(() => yaml.loads(document, { placeholders: ['a'] }), TypeError)
  assert.throws(() => yaml.loads(document, { environment: 'yes' }), TypeError)
})

test('JSON refuses placeholders by name at the call site', async () => {
  assert.throws(
    () => json.loads('{"a": "{{ NAME }}"}', { placeholders: { NAME: 'app' } }),
    /yaml\/toml feature/,
  )
  assert.throws(() => json.loads('{"a": 1}', { environment: true }), /yaml\/toml feature/)
  // The multi-document spellings refuse the same way, the streaming one as a
  // clean TypeError on the first pull - even over an empty stream.
  assert.throws(
    () => json.loadsAll('{"a": 1}\n', { placeholders: { NAME: 'app' } }),
    /yaml\/toml feature/,
  )
  async function* empty() {}
  await assert.rejects(
    json.loadAllStream(empty(), { placeholders: { NAME: 'app' } }).next(),
    /yaml\/toml feature/,
  )
  // And a plain JSON load reads braces as the text they are.
  assert.equal(json.loads('{"a": "{{ NAME }}"}').a, '{{ NAME }}')
})

test('an unquoted YAML placeholder is what YAML says it is', () => {
  const options = { placeholders: { PORT: 8080 } }
  assert.equal(yaml.loads('port: "{{ PORT }}"\n', options).port, 8080)

  // Unquoted, YAML read a flow mapping before anything here ran.
  const bare = yaml.loads('port: {{ PORT }}\n', options).port
  assert.equal(typeof bare, 'object')
})

test('dumping never reintroduces a placeholder', () => {
  const value = yaml.loads('path: "{{ ROOT }}/x"\n', { placeholders: { ROOT: '/srv' } })
  assert.equal(yaml.dumps(value).toString(), 'path: /srv/x\n')
})
