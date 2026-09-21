'use strict'

// `binding.js` deletes the native codec entry points from the loader as it
// wires the public ones, so they are read here, before any suite below
// requires `yggdryl`.
const nativeJsonDumps = require('../../index.js').jsonDumpsNative
const nativeYamlDumpAll = require('../../index.js').yamlDumpAllNative

// The `placeholder` suite, in its own block: it brings its own
// fixtures, and `const` is block-scoped.
{
  // Jinja-style `{{ }}` placeholders: a YAML and TOML feature only. JSON is a
  // data interchange format, and both the JS boundary and the core refuse the
  // pair for it by name - see the dedicated test at the bottom.

  const assert = require('node:assert/strict')
  const fs = require('node:fs')
  const os = require('node:os')
  const path = require('node:path')
  const test = require('node:test')
  const { pathToFileURL } = require('node:url')

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
    assert.deepEqual(yaml.load(pathToFileURL(target), options), { a: 'app' })
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
}

// The `toml` suite, in its own block: it brings its own
// fixtures, and `const` is block-scoped.
{
  const assert = require('node:assert/strict')
  const fs = require('node:fs')
  const os = require('node:os')
  const path = require('node:path')
  const { Writable } = require('node:stream')
  const { ReadableStream, WritableStream } = require('node:stream/web')
  const test = require('node:test')
  const { pathToFileURL } = require('node:url')

  const { DataType, Scalar, codec, fields, toml } = require('yggdryl')

  function nestedRecord(count) {
    let value = { leaf: 0 }
    for (let index = 0; index < count; index += 1) {
      value = { nested: value }
    }
    return value
  }

  test('TOML is a byte-first single-document facade', () => {
    const value = toml.loads('title = "Yggdryl"\n[owner]\nname = "Ada"\n')
    assert.deepEqual(value, { title: 'Yggdryl', owner: { name: 'Ada' } })

    const encoded = toml.dumps(value)
    assert.ok(Buffer.isBuffer(encoded))
    assert.deepEqual(toml.loads(encoded), value)
    assert.ok(Object.isFrozen(toml))

    for (const name of [
      'loadsAll',
      'loadAll',
      'dumpAll',
      'loadAllStream',
      'dumpAllStream',
    ]) {
      assert.equal(toml[name], undefined)
    }
  })

  test('TOML uses natural shapes and refuses values TOML cannot spell', () => {
    const value = {
      bigint: 2n ** 62n,
      bytes: Buffer.from([0, 1, 127, 255]),
      infinity: Infinity,
      map: new Map([['venues', new Set(['XPAR', 'XNAS'])]]),
      nan: NaN,
      negativeZero: -0,
      regexp: new RegExp('a/b(?<name>c)', 'giu'),
      schema: DataType.fromString('struct<id:int64 not null>'),
      typed: new Uint16Array([0, 65535]),
    }

    const encoded = toml.dumps(value)
    const decoded = toml.loads(encoded)
    assert.equal(decoded.bigint, value.bigint)
    assert.equal(decoded.bytes, value.bytes.toString('base64'))
    assert.equal(decoded.infinity, Infinity)
    assert.ok(Number.isNaN(decoded.nan))
    assert.ok(Object.is(decoded.negativeZero, -0))
    assert.equal(decoded.regexp, '/a\\/b(?<name>c)/giu')
    assert.deepEqual(decoded.map, { venues: ['XPAR', 'XNAS'] })
    assert.deepEqual(decoded.schema, value.schema.toJSON())
    assert.deepEqual(decoded.typed, [0, 65535])

    assert.throws(() => toml.dumps({ bigint: 2n ** 100n }), /exceeds i64/i)
    assert.throws(() => toml.dumps({ map: new Map([[1, 2]]) }), /keys must be strings/i)
    assert.throws(() => toml.dumps({ missing: undefined }), /cannot represent null/i)
    for (const root of [null, 'scalar root', [1, 2], Buffer.from([1, 2])]) {
      assert.throws(() => toml.dumps(root), /root must be a record/i)
    }
  })

  test('TOML writes natural temporals and field-directed exact decimals', () => {
    const written = toml.dumps({
      at: new Date('2026-08-15T12:30:00.000Z'),
      on: new DataType('date32').scalar(19723),
      since: new DataType('time32(s)').scalar(27120),
      price: Scalar.decimal(-1050n, 2),
      wide: Scalar.decimal(123456789012345678901234567890n, 4),
    })

    const text = written.toString('utf8')
    assert.match(text, /"at" = 2026-08-15T12:30:00/)
    assert.match(text, /"on" = 2024-01-01/)
    assert.match(text, /"since" = 07:32:00/)
    assert.match(text, /"price" = "-10\.50"/)

    const decoded = toml.loads(written)
    assert.ok(decoded.at instanceof Date)
    assert.equal(decoded.at.toISOString(), '2026-08-15T12:30:00.000Z')
    assert.ok(decoded.on.equals(new DataType('date32').scalar(19723)))
    assert.ok(decoded.since.equals(new DataType('time32(s)').scalar(27120)))
    assert.equal(decoded.price, '-10.50')

    const field = fields.struct('root', [
      fields.datetime64('at', 's', 'UTC', { nullable: false }),
      fields.date32('on', { nullable: false }),
      fields.decimal128('price', 10, 2, { nullable: false }),
      fields.time32('since', 's', { nullable: false }),
      fields.decimal256('wide', 40, 4, { nullable: false }),
    ], { nullable: false })
    const typed = toml.loads(written, { field })
    assert.ok(typed.at instanceof Date)
    assert.ok(typed.on.equals(new DataType('date32').scalar(19723)))
    assert.ok(typed.price.equals(Scalar.decimal(-1050n, 2)))
    assert.ok(typed.since.equals(new DataType('time32(s)').scalar(27120)))
    assert.ok(typed.wide.equals(Scalar.decimal(123456789012345678901234567890n, 4)))
  })

  test('TOML emission applies requested depth to the natural value', () => {
    const defaultBoundary = nestedRecord(12)
    const defaultBytes = toml.dumps(defaultBoundary)
    assert.deepEqual(toml.loads(defaultBytes), defaultBoundary)
    assert.deepEqual(
      codec.from(codec.into(defaultBoundary, { format: 'toml' }), {
        format: 'toml',
      }),
      defaultBoundary,
    )
    assert.throws(() => toml.dumps(nestedRecord(49)), /depth/i)
    assert.throws(() => codec.into(nestedRecord(49), { format: 'toml' }), /depth/i)
    assert.throws(() => toml.dumps(nestedRecord(3), { maxDepth: 3 }), /depth/i)

    const customBoundary = nestedRecord(6)
    const customBytes = toml.dumps(customBoundary, { maxDepth: 7 })
    assert.deepEqual(toml.loads(customBytes, { maxDepth: 7 }), customBoundary)
    assert.throws(() => toml.dumps(customBoundary, { maxDepth: 6 }), /depth/i)
  })

  test('native TOML date-times arrive as temporal values and write back exactly', () => {
    const source =
      'offset = 1979-05-27T07:32:00Z\nlocal = 1979-05-27T07:32:00\ndate = 1979-05-27\ntime = 07:32:00\n'
    const decoded = toml.loads(source)

    assert.ok(decoded.offset instanceof Date)
    assert.equal(decoded.offset.toISOString(), '1979-05-27T07:32:00.000Z')
    assert.ok(decoded.local.equals(new DataType('datetime64(s)').scalar(296638320n)))
    assert.ok(decoded.date.equals(new DataType('date32').scalar(3433)))
    assert.ok(decoded.time.equals(new DataType('time32(s)').scalar(27120)))

    assert.equal(
      toml.dumps(decoded).toString('utf8'),
      '"date" = 1979-05-27\n"local" = 1979-05-27T07:32:00\n"offset" = 1979-05-27T07:32:00Z\n"time" = 07:32:00\n',
    )
  })

  test('a class instance crosses TOML as its own properties', () => {
    let constructorCalls = 0
    class Order {
      static yggdrylType = 'orders.Order'

      constructor(id) {
        constructorCalls += 1
        this.id = id
      }
    }

    const encoded = toml.dumps(new Order(42))
    constructorCalls = 0
    assert.doesNotMatch(encoded.toString('utf8'), /orders\.Order/)
    assert.deepEqual(toml.loads(encoded), { id: 42 })
    assert.equal(constructorCalls, 0)
  })

  test('TOML content accepts strings and exact byte views', () => {
    assert.deepEqual(toml.loads('id = 42\n'), { id: 42 })
    assert.deepEqual(toml.load('id = 43\n'), { id: 43 })

    const framed = Buffer.from('xxid = 44\nyy')
    const view = new DataView(framed.buffer, framed.byteOffset + 2, 8)
    assert.deepEqual(toml.loads(view), { id: 44 })

    const shared = new SharedArrayBuffer(8)
    new Uint8Array(shared).set(Buffer.from('id = 45\n'))
    assert.deepEqual(toml.loads(shared), { id: 45 })
  })

  test('generic content inference delegates ambiguous syntax to the core', () => {
    assert.deepEqual(codec.from('{"id":1}'), { id: 1 })
    assert.deepEqual(codec.from('[1,2]'), [1, 2])
    assert.deepEqual(codec.from('id: 2\n'), { id: 2 })
    assert.deepEqual(codec.from('id = 3\n'), { id: 3 })
    assert.equal(codec.from('missing.toml'), 'missing.toml')
    assert.throws(() => codec.from(''), /yaml/i)
    assert.throws(() => codec.from('# shared comment syntax\n'), /yaml/i)

    assert.deepEqual(codec.from('id = 4\n', { format: 'toml' }), { id: 4 })
    assert.deepEqual(codec.from('', { format: 'toml' }), {})
    assert.deepEqual(codec.from('# TOML comment\n', { format: 'toml' }), {})
  })

  test('TOML file URLs use native readers while string destinations stay paths', () => {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-toml-path-'))
    const source = path.join(directory, 'source.toml')
    const destination = path.join(directory, 'destination.toml')
    const misleading = path.join(directory, 'explicit.json')
    fs.writeFileSync(source, 'id = 46\n')

    const readSync = fs.readSync
    const writeFileSync = fs.writeFileSync
    try {
      fs.readSync = () => {
        throw new Error('real TOML paths must bypass JavaScript read staging')
      }
      assert.deepEqual(toml.load(pathToFileURL(source)), { id: 46 })
      assert.deepEqual(codec.from(pathToFileURL(source)), { id: 46 })

      fs.writeFileSync = () => {
        throw new Error('real TOML paths must bypass JavaScript write staging')
      }
      toml.dump({ id: 47 }, destination)
      codec.into({ id: 48 }, misleading, { format: 'toml' })
    } finally {
      fs.readSync = readSync
      fs.writeFileSync = writeFileSync
    }

    try {
      assert.deepEqual(toml.load(pathToFileURL(destination)), { id: 47 })
      assert.deepEqual(
        codec.from(pathToFileURL(misleading), { format: 'toml' }),
        { id: 48 },
      )
      assert.deepEqual(
        codec.from(Buffer.from('id = 49\n'), { format: 'toml' }),
        { id: 49 },
      )
      assert.deepEqual(
        toml.loads(codec.into({ id: 50 }, { format: 'toml' })),
        { id: 50 },
      )
    } finally {
      fs.rmSync(directory, { force: true, recursive: true })
    }
  })

  test('TOML file descriptors stay caller-owned', () => {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-toml-fd-'))
    const source = path.join(directory, 'source.toml')
    const destination = path.join(directory, 'destination.toml')
    fs.writeFileSync(source, 'id = 51\n')
    const sourceFd = fs.openSync(source, 'r')
    const destinationFd = fs.openSync(destination, 'w+')
    try {
      assert.deepEqual(toml.load(sourceFd), { id: 51 })
      assert.ok(fs.fstatSync(sourceFd).isFile())
      toml.dump({ id: 52 }, destinationFd)
      assert.ok(fs.fstatSync(destinationFd).isFile())
    } finally {
      fs.closeSync(sourceFd)
      fs.closeSync(destinationFd)
    }
    try {
      assert.deepEqual(toml.load(pathToFileURL(destination)), { id: 52 })
    } finally {
      fs.rmSync(directory, { force: true, recursive: true })
    }
  })

  test('TOML async readers preserve split strings and bounded single-document semantics', async () => {
    async function* splitUnicode() {
      yield 'label = "\ud83d'
      yield '\ude42"\n[nested]\n'
      yield 'value = 53\n'
    }

    const decoded = await toml.load(splitUnicode())
    assert.deepEqual(decoded, {
      label: '\u{1f642}',
      nested: { value: 53 },
    })
    assert.deepEqual(
      [...decoded.label].map((character) => character.codePointAt(0)),
      [0x1f642],
    )
    let pulls = 0
    async function* inferredToml() {
      pulls += 1
      yield 'id = '
      pulls += 1
      yield '54\n'
    }
    assert.deepEqual(await codec.from(inferredToml()), { id: 54 })
    assert.equal(pulls, 2)

    const webInput = new ReadableStream({
      start(controller) {
        controller.enqueue(Buffer.from('id = 55\n'))
        controller.close()
      },
    })
    assert.deepEqual(await toml.load(webInput), { id: 55 })
  })

  test('TOML Node and WHATWG writers honor errors, backpressure, and no-close ownership', async () => {
    const chunks = []
    const nodeOutput = new Writable({
      write(chunk, _encoding, done) {
        setImmediate(() => {
          chunks.push(Buffer.from(chunk))
          done()
        })
      },
    })
    await toml.dump({ id: 56 }, nodeOutput)
    assert.deepEqual(toml.loads(Buffer.concat(chunks)), { id: 56 })
    assert.equal(nodeOutput.writableEnded, false)

    const webChunks = []
    let webClosed = false
    const webOutput = new WritableStream({
      write(chunk) {
        webChunks.push(Buffer.from(chunk))
      },
      close() {
        webClosed = true
      },
    })
    await codec.into({ id: 57 }, webOutput, { format: 'toml' })
    assert.deepEqual(toml.loads(Buffer.concat(webChunks)), { id: 57 })
    assert.equal(webClosed, false)

    const expected = new Error('TOML sink rejected bytes')
    const failing = new Writable({
      write(_chunk, _encoding, done) {
        done(expected)
      },
    })
    await assert.rejects(toml.dump({ id: 58 }, failing), (error) => error === expected)
  })

  test('TOML errors and limits stay native and path conversion is non-destructive', () => {
    assert.throws(() => toml.loads('id = 1\nid = 2\n'), /toml/i)

    let nested = { leaf: true }
    for (let index = 0; index < 24; index += 1) nested = { nested }
    const nestedBytes = toml.dumps(nested)
    assert.deepEqual(toml.loads(nestedBytes), nested)
    assert.throws(() => toml.loads(nestedBytes, { maxDepth: 8 }), /depth/i)

    let deep = { value: 1 }
    for (let index = 0; index < 49; index += 1) deep = { nested: deep }
    assert.throws(() => toml.dumps(deep), /maxDepth 48/)

    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-toml-safe-'))
    const destination = path.join(directory, 'existing.toml')
    fs.writeFileSync(destination, 'keep = "me"\n')
    const cyclic = {}
    cyclic.self = cyclic
    try {
      assert.throws(() => toml.dump(cyclic, destination), /cyclic/)
      assert.equal(fs.readFileSync(destination, 'utf8'), 'keep = "me"\n')
      assert.throws(
        () => toml.dump(nestedRecord(12), destination, { maxDepth: 12 }),
        /depth/i,
      )
      assert.equal(fs.readFileSync(destination, 'utf8'), 'keep = "me"\n')
      assert.throws(
        () =>
          codec.into(nestedRecord(4), destination, {
            format: 'toml',
            maxDepth: 4,
          }),
        /depth/i,
      )
      assert.equal(fs.readFileSync(destination, 'utf8'), 'keep = "me"\n')
    } finally {
      fs.rmSync(directory, { force: true, recursive: true })
    }
  })
}

// The `codec` suite, in its own block: it brings its own
// fixtures, and `const` is block-scoped.
{
  const assert = require('node:assert/strict')
  const fs = require('node:fs')
  const os = require('node:os')
  const path = require('node:path')
  const { Readable, Writable } = require('node:stream')
  const { WritableStream } = require('node:stream/web')
  const test = require('node:test')
  const { pathToFileURL } = require('node:url')
  const { types: utilTypes } = require('node:util')
  const vm = require('node:vm')
  const arrow = require('apache-arrow')
  const nativeBinding = require('../../index.js')
  const rawNativeWrapperPrototypes = [
    nativeBinding.Scalar.prototype,
    nativeBinding.DataType.prototype,
    nativeBinding.Field.prototype,
    nativeBinding.Uri.prototype,
    nativeBinding.Url.prototype,
    nativeBinding.Urn.prototype,
    nativeBinding.Version.prototype,
  ]
  const rawRegExpSourceGetter = Object.getOwnPropertyDescriptor(
    RegExp.prototype,
    'source',
  ).get
  const rawRegExpFlagsGetter = Object.getOwnPropertyDescriptor(
    RegExp.prototype,
    'flags',
  ).get
  const rawNativeIntrinsics = [
    utilTypes.isMap,
    utilTypes.isSet,
    utilTypes.isRegExp,
    (value) => Reflect.apply(rawRegExpSourceGetter, value, []),
    (value) => Reflect.apply(rawRegExpFlagsGetter, value, []),
  ]

  const {
    DataType,
    Field,
    Uri,
    Url,
    Urn,
    Version,
    Timezone,
    Scalar,
    codec,
    json,
    toml,
    yaml,
  } = require('yggdryl')

  test('YAML lowers extended JavaScript values to natural shapes', () => {
    const value = {
      bigint: 2n ** 100n,
      bytes: Buffer.from([0, 1, 127, 255]),
      date: new Date('2026-08-15T12:30:00.000Z'),
      infinity: Infinity,
      map: new Map([[{ key: true }, new Set(['XPAR', 'XNAS'])]]),
      nan: NaN,
      negativeZero: -0,
      regexp: new RegExp('a/b(?<name>c)', 'giu'),
      typed: new Uint16Array([0, 65535]),
      undefined,
    }

    for (const format of [yaml]) {
      const bytes = format.dumps(value)
      const decoded = format.loads(bytes)

      assert.ok(Buffer.isBuffer(bytes))
      // An exact integer, bytes, and every float stay themselves; an instant
      // is spelled as its classic ISO string, the loosely typed deal every
      // schemaless wire now makes.
      assert.equal(decoded.bigint, value.bigint)
      assert.deepEqual(decoded.bytes, value.bytes)
      assert.equal(decoded.date, '2026-08-15T12:30:00.000Z')
      assert.equal(decoded.infinity, Infinity)
      assert.ok(Number.isNaN(decoded.nan))
      assert.ok(Object.is(decoded.negativeZero, -0))

      // These are the documented losses: no wrapper carries the class back.
      assert.equal(decoded.regexp, '/a\\/b(?<name>c)/giu')
      assert.ok(decoded.map instanceof Map)
      assert.deepEqual([...decoded.map.keys()], [{ key: true }])
      assert.deepEqual([...decoded.map.values()], [['XPAR', 'XNAS']])
      assert.deepEqual(decoded.typed, [0, 65535])
      assert.ok(Object.hasOwn(decoded, 'undefined'))
      assert.equal(decoded.undefined, null)
    }
    assert.throws(() => json.dumps(value), /non-finite float/)
  })

  test('a bigint beyond the exact 128-bit range is refused, not rounded', () => {
    assert.equal(json.loads(json.dumps(2n ** 127n - 1n)), 2n ** 127n - 1n)
    assert.equal(json.loads(json.dumps(2n ** 128n - 1n)), 2n ** 128n - 1n)
    assert.throws(
      () => json.dumps(2n ** 128n),
      /exceeds the exact 128-bit integer range/,
    )
    // A bigint small enough to be an ordinary number comes back as one.
    assert.equal(json.loads(json.dumps(42n)), 42)
  })

  test('Map and Set carry explicit undefined entries as null', () => {
    const value = {
      map: new Map([[undefined, undefined]]),
      set: new Set([undefined]),
    }

    for (const format of [yaml]) {
      const decoded = format.loads(format.dumps(value))
      assert.equal(decoded.map.size, 1)
      assert.equal(decoded.map.get(null), null)
      assert.deepEqual([...decoded.set], [null])
    }
  })

  test('two JavaScript Map keys that are one native key are refused', () => {
    assert.throws(
      () => json.dumps(new Map([[1, 'number'], [1n, 'bigint']])),
      /duplicate/i,
    )
  })

  test('schema wrappers cross structurally and locations cross as text', () => {
    const type = DataType.fromString('struct<id:int64 not null>')
    const field = new Field('id', type, false)

    for (const format of [json, yaml]) {
      assert.deepEqual(format.loads(format.dumps(type)), type.toJSON())
      assert.deepEqual(format.loads(format.dumps(field)), field.toJSON())
    }
    assert.equal(Scalar.from(type).kind, 'mapping')
    assert.equal(Scalar.from(field).kind, 'mapping')

    for (const value of [
      Uri.fromString('https://example.com/value'),
      Url.fromString('https://example.com/value'),
      Urn.fromString('urn:example:value'),
      new Version(5, 0, 300),
    ]) {
      assert.equal(json.loads(json.dumps(value)), value.toString())
    }
  })

  test('native wrapper encoding reads native state instead of replaceable methods', () => {
    const values = [
      DataType.fromString('int64'),
      new Field('id', 'int64', false),
      Uri.fromString('https://example.com/value'),
      Url.fromString('https://example.com/value'),
      Urn.fromString('urn:example:value'),
      new Version(5, 0, 300),
    ]

    for (const value of values) {
      const expected = value instanceof DataType || value instanceof Field
        ? value.toJSON()
        : value.toString()
      Object.defineProperty(value, 'toString', {
        value() {
          throw new Error('replaceable JavaScript method must not run')
        },
      })
      assert.deepEqual(json.loads(json.dumps(value)), expected)
    }
  })

  test('a URL and a Date read their state from the prototype, not the instance', () => {
    const url = new URL('https://example.com/value?q=1')
    Object.defineProperty(url, 'toString', {
      value() {
        throw new Error('replaceable JavaScript method must not run')
      },
    })
    assert.equal(json.loads(json.dumps(url)), 'https://example.com/value?q=1')

    const date = new Date('2026-08-15T12:30:00.000Z')
    Object.defineProperty(date, 'getTime', {
      value: () => 0,
    })
    assert.equal(json.loads(json.dumps(date)), '2026-08-15T12:30:00.000Z')
    assert.throws(() => json.dumps(new Date(NaN)), /invalid Date/)
  })

  test('temporal values cross as classic ISO strings; a decimal stays typed', () => {
    const values = {
      at: new DataType('datetime64(us,"UTC")').scalar(1700000000000000n),
      naive: new DataType('datetime64(us)').scalar(1700000000123456n),
      on: new DataType('date32').scalar(19723),
      sinceMidnight: new DataType('time64(us)').scalar(45296000000n),
      took: Scalar.duration(90, 's'),
      price: Scalar.decimal(-1050n, 2),
    }

    for (const format of [json, yaml]) {
      const decoded = format.loads(format.dumps(values))
      // The fraction width is the unit, so nothing about the reading is lost -
      // it is just spelled the way every other tool spells it.
      assert.equal(decoded.at, '2023-11-14T22:13:20.000000Z')
      assert.equal(decoded.naive, '2023-11-14T22:13:20.123456')
      assert.equal(decoded.on, '2024-01-01')
      assert.equal(decoded.sinceMidnight, '12:34:56.000000')
      assert.equal(decoded.took, 'PT90S')
      assert.equal(decoded.price, '-10.50')
    }

    assert.equal(
      json.dumps({ price: values.price }).toString(),
      '{"price":"-10.50"}',
    )
    // One instant in two resolutions is one value, and so is one number in two
    // spellings, because the core compares what a value names.
    assert.ok(Scalar.duration(1, 's').equals(Scalar.duration(1000n, 'ms')))
    assert.ok(Scalar.decimal(150n, 2).equals(Scalar.decimal(15n, 1)))
  })

  test('temporal families select one exact width and non-null timezone', () => {
    const naive = Timezone.from('naive')
    const values = [
      [new DataType('date32').scalar(7), 'date32', 7n, 'd'],
      // A `date64` counts whole days in milliseconds, and the type checks it.
      [new DataType('date64').scalar(86_400_000n), 'date64', 86_400_000n, 'ms'],
      [new DataType('time32(s)').scalar(7), 'time32', 7n, 's'],
      [new DataType('time64(us)').scalar(7n), 'time64', 7n, 'us'],
      [new DataType('datetime64(ns)').scalar(7n), 'datetime64', 7n, 'ns'],
      [Scalar.duration(7, 'ms'), 'duration32', 7n, 'ms'],
      [Scalar.duration(2147483648n, 'us'), 'duration64', 2147483648n, 'us'],
    ]

    for (const [value, kind, count, unit] of values) {
      assert.equal(value.kind, kind)
      assert.equal(value.count, count)
      assert.equal(value.unit, unit)
      assert.equal(value.zone, 'NAIVE')
    }

    // A zone is part of the type now, so it is spelled once, where the width is.
    // These used to pass a `timezone` argument the core could only accept as
    // naive; the type simply has nowhere to put one for a date or a time of day.
    assert.ok(
      new DataType('datetime64(ns,"UTC")')
        .scalar(7n)
        .equals(new DataType('datetime64(ns,"UTC")').scalar(7n)),
    )
    assert.ok(
      !new DataType('datetime64(ns)').scalar(7n).equals(
        new DataType('datetime64(ns,"UTC")').scalar(7n),
      ),
      'a naive instant is not the same value as a zoned one',
    )
    assert.equal(new DataType('datetime64(ns,"UTC")').scalar(7n).zone, 'UTC')
    assert.ok(Scalar.duration(7, 'ms').equals(Scalar.duration(7, 'ms')))

    // A unit the width cannot hold is refused by the type, not by a factory.
    assert.throws(() => new DataType('date32(s)'), /unexpected/)
    assert.throws(() => new DataType('time32(d)'), /expected/)
    // A date and a time of day carry no zone, so the type cannot spell one.
    assert.throws(() => new DataType('date32(d,"UTC")'), /unexpected/)
    assert.throws(() => new DataType('time64(us,"UTC")'), /expected/)
    // A duration keeps its factory, and still refuses a zone.
    assert.throws(() => new DataType('duration32(s,"UTC")'), /expected/)
  })

  test('Scalar family factories keep selected widths, hashes, and natural accessors', () => {
    const wide = Scalar.decimal(-(2n ** 200n), 7)
    const longDuration = Scalar.duration(2147483648n, 'us')
    assert.equal(Scalar.float(1.5, 16).kind, 'f16')
    assert.equal(Scalar.float(1.5, 32).kind, 'f32')
    assert.equal(Scalar.float(1.5).kind, 'f64')
    assert.equal(Scalar.decimal(1n).scale, 0)
    assert.throws(() => new DataType('float24'), /unknown datatype/)
    assert.throws(() => Scalar.decimal(1n, 128), /scale/)
    assert.equal(wide.kind, 'd256')
    assert.equal(wide.unscaled, -(2n ** 200n))
    assert.equal(wide.scale, 7)
    assert.match(wide.dtype.toString(), /^decimal256/)
    assert.equal(typeof wide.stableHash(), 'bigint')
    assert.equal(wide.asJs().kind, 'd256')
    assert.equal(wide.asJs().unscaled, wide.unscaled)
    assert.equal(longDuration.kind, 'duration64')
    assert.equal(longDuration.asJs().kind, 'duration64')
    assert.equal(longDuration.asJs().count, longDuration.count)

    // An enum member's datatype is `string`, so the value is its canonical
    // name and the vocabulary is what `fromEnum` validates against.
    const mode = Scalar.fromEnum('IOMode', 'append')
    assert.equal(mode.kind, 'string')
    assert.equal(mode.asJs(), 'append')
    assert.equal(mode.asStr(), 'append')
    assert.deepEqual(mode, Scalar.from('append'))
    assert.throws(() => Scalar.fromEnum('IOMode', 'missing'), /unknown/)

    assert.deepEqual(Scalar.from(Buffer.from([0, 255])).asBytes(), Buffer.from([0, 255]))
    assert.equal(Scalar.from('AAPL').asStr(), 'AAPL')
    assert.equal(Scalar.from(1).asStr(), null)
    const record = Scalar.from({ z: 2, a: 1 })
    assert.equal(record.intoJson(), '{"a":1,"z":2}')
    assert.deepEqual(record.intoJsonBytes(), Buffer.from(record.intoJson()))
    assert.equal(record.toString(), record.intoJson())
    // `toJSON` goes through binding.js, which called a method the rename had
    // already removed - nothing covered that path, so the break shipped.
    assert.deepEqual(JSON.parse(JSON.stringify(record)), { a: 1, z: 2 })

    // Truthiness is a coercion: absence, zero, empty text or bytes, and a
    // container with nothing set in it all read false.
    assert.equal(Scalar.from(5).isTruthy(), true)
    assert.equal(Scalar.from(0).isTruthy(), false)
    assert.equal(Scalar.from('').isTruthy(), false)
    assert.equal(Scalar.from('off').isTruthy(), false)
    assert.equal(Scalar.from('anything').isTruthy(), true)
    assert.equal(record.isTruthy(), true)

    // The typed door for a value: the width, unit, scale and zone are named on
    // the type. Node had no `scalar` on either prototype, so the only spelling
    // was the options bag - which is the same conversion, said awkwardly.
    const instant = new Field('at', new DataType('datetime64(ns,"UTC")'), true)
    assert.equal(instant.scalar(1700000000123456789n).kind, 'datetime64')
    assert.equal(instant.scalar(1n).unit, 'ns')
    assert.equal(instant.scalar(1n).zone, 'UTC')
    assert.ok(instant.scalar(1n).equals(Scalar.from(1n, { field: instant })))
    // A width the statics cannot reach at all.
    assert.equal(new DataType('decimal32(9,2)').scalar(125).kind, 'd32')
    assert.ok(Scalar.float(1.5, 16).equals(Scalar.float(1.5, 16)))
    assert.deepEqual(record.toJSON(), { a: 1, z: 2 })
    const clone = record.clone()
    assert.notEqual(clone, record)
    assert.ok(clone.equals(record))
    assert.equal(clone.compare(record), 0)
    assert.equal(clone.stableHash(), record.stableHash())
    assert.ok(Scalar.from(1).compare(Scalar.from(2)) < 0)
    assert.equal(Scalar.decimal(150n, 2).compare(Scalar.decimal(15n, 1)), 0)
  })

  test('Scalar identity accessors name the exact leaf and family', () => {
    const values = [
      [Scalar.from(null), 'null', 'null'],
      [Scalar.from(true), 'boolean', 'boolean'],
      [Scalar.from(1n), 'int64', 'integer'],
      [Scalar.float(1.5, 32), 'float32', 'floating'],
      [Scalar.decimal(150n, 2), 'decimal128', 'decimal'],
      [new DataType('date32').scalar(1), 'date32', 'temporal'],
      [Scalar.from('AAPL'), 'utf8', 'text'],
      [
        json.loads('"USD"', {
          field: new Field('value', 'currency', false),
          scalar: true,
        }),
        'currency',
        'code',
      ],
      [
        json.loads('"00112233-4455-6677-8899-aabbccddeeff"', {
          field: new Field('value', 'uuid', false),
          scalar: true,
        }),
        'uuid',
        'uuid',
      ],
      [
        json.loads('"5.0.1"', {
          field: new Field('value', 'version', false),
          scalar: true,
        }),
        'version',
        'text',
      ],
      [Scalar.from(Buffer.from('bytes')), 'binary', 'bytes'],
      [Scalar.from({ id: 1 }), 'struct', 'nested'],
    ]
    for (const [value, expectedId, expectedFamily] of values) {
      assert.equal(value.id, expectedId)
      assert.equal(value.family, expectedFamily)
      assert.equal(value.id, value.dtype.id)
      assert.equal(value.family, value.dtype.kind)
    }
  })

  test('exact intervals retain their flat JavaScript layouts', () => {
    const typed = (document, dtype) => json.loads(document, {
      field: new Field('span', dtype, false),
      scalar: true,
    })

    assert.equal(typed('12', 'interval(year_month)').asJs(), 12)
    assert.deepEqual(typed('[2,3]', 'interval(day_time)').asJs(), [2, 3])
    assert.deepEqual(
      typed('[1,2,3]', 'interval(month_day_nano)').asJs(),
      [1, 2, 3],
    )
  })

  test('Scalar traversal and persistent updates stay entirely native', () => {
    const instant = new DataType('datetime64(ns,"Europe/Paris")').scalar(1700000000123456789n)
    const record = Scalar.from({ z: 2, legs: [{ at: instant }] })

    assert.equal(record.length, 2)
    assert.equal(record.isEmpty(), false)
    assert.equal(Scalar.from({}).isEmpty(), true)
    assert.equal(Scalar.from(1).isEmpty(), false)
    assert.equal(record.get('missing'), null)
    assert.equal(record.has('legs'), true)
    assert.equal(record.has('missing'), false)

    const legs = record.get('legs')
    assert.ok(legs instanceof Scalar)
    assert.equal(legs.length, 1)
    assert.ok(legs.get(0).equals(legs.at(0)))
    assert.equal(legs.at(1), null)
    assert.throws(() => legs.at(-1), /non-negative/)

    const nested = record.path('legs.0.at')
    assert.ok(nested instanceof Scalar)
    assert.equal(nested.kind, 'datetime64')
    assert.equal(nested.count, 1700000000123456789n)
    assert.equal(nested.unit, 'ns')
    assert.equal(nested.zone, 'Europe/Paris')
    assert.equal(record.path('legs.9.at'), null)

    // Record iteration is deterministic field-name order and yields values.
    assert.deepEqual([...record].map((value) => value.kind), ['sequence', 'i64'])
    // Sequence iteration yields its exact children.
    assert.equal([...legs][0].path('at').count, instant.count)

    const changed = record.set('z', instant).set('a', 1)
    assert.equal(record.get('z').kind, 'i64')
    assert.equal(changed.get('z').kind, 'datetime64')
    assert.deepEqual([...changed].map((value) => value.kind), [
      'i64',
      'sequence',
      'datetime64',
    ])
    const removed = changed.remove('legs')
    assert.equal(removed.has('legs'), false)
    assert.equal(changed.has('legs'), true)
    assert.ok(removed.remove('missing').equals(removed))

    const decimalKey = Scalar.decimal(150n, 2)
    const mapping = Scalar.from(new Map([[decimalKey, instant]]))
    assert.equal(mapping.length, 1)
    assert.equal(mapping.get(Scalar.decimal(15n, 1)).count, instant.count)
    assert.equal([...mapping][0].kind, 'd128')
    const added = mapping.set('venue', 'XNAS')
    assert.equal(added.get('venue').asStr(), 'XNAS')
    assert.equal(mapping.get('venue'), null)
    assert.equal(added.remove('venue').length, 1)

    assert.throws(() => record.get(0), /field names must be strings/)
    assert.throws(() => record.set(0, 1), /field names must be strings/)
    assert.throws(() => mapping.remove(decimalKey), /string key/)
    assert.throws(() => legs.set(0, 1), /mapping or record/)
  })

  test('Scalar arithmetic infers JavaScript operands once and stays native', () => {
    const forty = Scalar.from(40)
    assert.ok(forty.add(2).equals(Scalar.from(42)))
    assert.ok(forty.subtract(Scalar.from(2)).equals(Scalar.from(38)))
    assert.ok(Scalar.from(6).multiply(7).equals(Scalar.from(42)))
    assert.ok(Scalar.from(84).divide(2).equals(Scalar.from(42)))
    assert.ok(Scalar.from(5).remainder(2).equals(Scalar.from(1)))
    assert.ok(Scalar.from(5).negate().equals(Scalar.from(-5)))
    assert.ok(Scalar.from(-5).absolute().equals(Scalar.from(5)))

    assert.ok(
      Scalar.decimal(105n, 2)
        .add(Scalar.decimal(2n, 1))
        .equals(Scalar.decimal(125n, 2)),
    )
    assert.ok(
      Scalar.decimal(1n)
        .divide(Scalar.decimal(2n))
        .equals(Scalar.decimal(5n, 1)),
    )
    assert.ok(
      Scalar.decimal(1n)
        .divide(Scalar.decimal(128n))
        .equals(Scalar.decimal(78125n, 7)),
    )
    assert.throws(
      () => Scalar.decimal(1n).divide(Scalar.decimal(3n)),
      (error) =>
        error instanceof RangeError &&
        error.code === 'ERR_YGGDRYL_INEXACT_ARITHMETIC',
    )
    assert.equal(Scalar.float(1.5, 16).multiply(Scalar.float(2, 32)).kind, 'f32')

    const instant = new DataType('datetime64(ms,"UTC")').scalar(1000n)
    assert.ok(
      instant
        .add(Scalar.duration(2n, 's'))
        .equals(new DataType('datetime64(ms,"UTC")').scalar(3000n)),
    )
    assert.ok(
      instant
        .subtract(new DataType('datetime64(ms,"UTC")').scalar(500n))
        .equals(Scalar.duration(500n, 'ms')),
    )

    assert.throws(
      () => Scalar.from(1).divide(0),
      (error) =>
        error instanceof RangeError &&
        error.code === 'ERR_YGGDRYL_DIVISION_BY_ZERO' &&
        /division by zero/i.test(error.message),
    )
    assert.throws(
      () => Scalar.from(9223372036854775807n).add(1n),
      (error) =>
        error instanceof RangeError && error.code === 'ERR_YGGDRYL_ARITHMETIC_OVERFLOW',
    )
    // Text, bytes and sequences have no sum, so `add` joins them.
    assert.equal(Scalar.from('AA').add('PL').asJs(), 'AAPL')
    assert.deepEqual(
      Scalar.from(Buffer.from([1])).add(Buffer.from([2])).asBytes(),
      Buffer.from([1, 2]),
    )
    assert.deepEqual(Scalar.from([1]).add([2]).asJs(), [1, 2])
    // Two repertoires still do not join, and only `add` joins at all.
    assert.throws(
      () => Scalar.from('a').add(1),
      (error) =>
        error instanceof TypeError &&
        error.code === 'ERR_YGGDRYL_INVALID_ARITHMETIC' &&
        /addition/i.test(error.message),
    )
    assert.throws(
      () => Scalar.from('a').subtract('b'),
      (error) =>
        error instanceof TypeError &&
        error.code === 'ERR_YGGDRYL_INVALID_ARITHMETIC' &&
        /subtraction/i.test(error.message),
    )
    for (const hidden of [
      '_addNative',
      '_subtractNative',
      '_multiplyNative',
      '_divideNative',
      '_remainderNative',
      '_negateNative',
      '_absoluteNative',
    ]) {
      assert.equal(forty[hidden], undefined, hidden)
    }
  })

  test('Field-directed natural JSON keeps exact typed values', () => {
    const narrow = json.loads('7', {
      field: new Field('value', 'int16', false),
      scalar: true,
    })
    assert.ok(narrow instanceof Scalar)
    assert.equal(narrow.kind, 'i16')

    const decimal = new Field('price', 'decimal256(40,2)', false)
    const decoded = json.loads('"123456789012345678901234567890.50"', {
      field: decimal,
    })
    assert.equal(decoded.kind, 'd256')
    assert.equal(decoded.scale, 2)
    assert.equal(decoded.unscaled, 12345678901234567890123456789050n)

    const row = new Field(
      'trade',
      'struct<quantity: int32 not null, symbol: utf8 not null>',
      false,
    )
    assert.deepEqual(
      json.loads('{"quantity":2,"symbol":"AAPL"}', { field: row }),
      { quantity: 2, symbol: 'AAPL' },
    )
    class Trade {
      static get intoStructField() {
        return row
      }
    }
    assert.deepEqual(
      json.loads('{"quantity":2,"symbol":"AAPL"}', { field: Trade }),
      { quantity: 2, symbol: 'AAPL' },
    )
    assert.deepEqual(
      json.loads('{"quantity":2,"symbol":"AAPL"}', { field: new Trade() }),
      { quantity: 2, symbol: 'AAPL' },
    )
    assert.equal(json.loads('1', { field: 'value: int64 not null' }), 1)
    assert.throws(() => json.loads('1', { field: {} }), /static intoStructField getter/)
  })

  test('Scalar Arrow scalar and array interop uses standard IPC', () => {
    const vector = arrow.vectorFromArray(Int32Array.from([1, 2, 3]))
    const values = Scalar.fromArrowArray(vector)
    assert.equal(values.kind, 'sequence')
    assert.deepEqual(values.asJs(), [1, 2, 3])
    assert.deepEqual([...values.intoArrowArray()], [1, 2, 3])

    const scalarVector = arrow.vectorFromArray(Int32Array.of(42))
    const scalar = Scalar.fromArrowScalar(scalarVector)
    assert.equal(scalar.kind, 'i32')
    assert.equal(scalar.intoArrowScalar(), 42)

    const coefficient = new Uint32Array(8)
    coefficient[0] = 1
    const decimal = Scalar.fromArrowScalar(
      arrow.vectorFromArray([coefficient], new arrow.Decimal(2, 40, 256)),
    )
    const duration = Scalar.fromArrowScalar(
      arrow.vectorFromArray([1n], new arrow.DurationMicrosecond()),
    )
    assert.equal(decimal.kind, 'd256')
    assert.equal(decimal.asJs().kind, 'd256')
    assert.equal(duration.kind, 'duration64')
    assert.equal(duration.asJs().kind, 'duration64')

    assert.throws(
      () => Scalar.fromArrowScalar(vector),
      /one-item Arrow Vector/,
    )

    const empty = Scalar.from([])
    assert.throws(() => empty.intoArrowArray(), /empty.*pass a Field/i)
    const emptyVector = empty.intoArrowArray(new Field('value', 'int32', true))
    assert.equal(emptyVector.length, 0)

    const overflowing = arrow.vectorFromArray(Int32Array.of(200))
    assert.deepEqual(
      Scalar.fromArrowArray(overflowing, new Field('value', 'int8', false)).asJs(),
      [0],
    )
  })

  test('Scalar Field accessors redirect to core inference', () => {
    const scalar = Scalar.from(42).intoField()
    assert.equal(scalar.name, 'value')
    assert.equal(scalar.dtype.toString(), 'int64')
    assert.equal(scalar.nullable, false)

    const item = Scalar.from([1, null]).intoArrayField()
    assert.equal(item.name, 'item')
    assert.equal(item.dtype.toString(), 'int64')
    assert.equal(item.nullable, true)

    const root = Scalar.from([{ id: 1, venue: null }, { id: 2, venue: 'XNAS' }])
      .intoStructField()
    assert.equal(root.name, 'row')
    assert.equal(root.nullable, false)
    const children = [...root.dtype]
    assert.deepEqual(children.map((child) => child.name), ['id', 'venue'])
    assert.equal(children[1].nullable, true)

    assert.throws(() => Scalar.from([]).intoArrayField(), /empty Sequence/)
    assert.throws(() => Scalar.from([[1]]).intoStructField(), /field names/)
  })

  test('Scalar Arrow record and table interop uses the native schema engine', () => {
    const table = arrow.tableFromArrays({
      id: Int32Array.from([1, 2]),
      symbol: ['AAPL', 'MSFT'],
    })
    const root = new Field(
      'row',
      DataType.fromFields([
        new Field('id', 'int32', false),
        new Field('symbol', 'utf8', false),
      ]),
      false,
    )
    const rows = Scalar.fromArrowTable(table, root)
    assert.deepEqual(rows.asJs(), [[1, 'AAPL'], [2, 'MSFT']])
    const restored = rows.intoArrowTable(root)
    assert.equal(restored.numRows, 2)
    assert.deepEqual([...restored.getChild('id')], [1, 2])

    const inferred = Scalar.from([{ id: 1 }, { id: 2 }]).intoArrowBatch()
    assert.deepEqual([...inferred.getChild('id')], [1n, 2n])
    assert.throws(
      () => Scalar.from([]).intoArrowTable(),
      /cannot infer a Struct Field from empty rows; pass a Struct Field/i,
    )
  })

  test('a Date is the JavaScript spelling of a UTC millisecond datetime64', () => {
    const date = new Date('2026-08-15T12:30:00.000Z')
    assert.ok(Scalar.from(date).equals(new DataType('datetime64(ms,"UTC")').scalar(1786797000000n)))
    assert.ok(Scalar.from(date).asJs() instanceof Date)

    // On the wire every temporal is its classic ISO string; the typed reading
    // comes back wherever a schema names the column's datatype.
    assert.equal(
      json.loads(json.dumps(new DataType('datetime64(s)').scalar(1786797000n))),
      '2026-08-15T12:30:00',
    )
    assert.equal(
      json.loads(json.dumps(new DataType('datetime64(ms,"Europe/Paris")').scalar(1786797000000n))),
      '2026-08-15T14:30:00.000+02:00[Europe/Paris]',
    )
  })

  test('null crosses everywhere a value goes', () => {
    // Null is a value, not a trap: alone, inside arrays, as an object value,
    // and through both codecs, it stays null - and undefined lowers to it.
    assert.equal(Scalar.from(null).kind, 'null')
    assert.equal(Scalar.from(null).asJs(), null)
    assert.deepEqual(json.loads(json.dumps({ gap: null, list: [null, 1] })), {
      gap: null,
      list: [null, 1],
    })
    assert.deepEqual(yaml.loads(yaml.dumps([null])), [null])
  })

  test('fromJs and asJs are the conversion every codec entry point crosses', () => {
    // The pivot answers what a JavaScript value becomes, losses included.
    assert.equal(Scalar.from(new Set([1, 2])).kind, 'sequence')
    assert.deepEqual(Scalar.from(new Set([1, 2])).asJs(), [1, 2])
    assert.equal(Scalar.from(new Map([['id', 1]])).kind, 'mapping')
    assert.deepEqual(Scalar.from(new Map([['id', 1]])).asJs(), new Map([['id', 1]]))
    assert.equal(Scalar.from({ id: 1 }).kind, 'struct')
    assert.equal(Scalar.from(undefined).kind, 'null')

    // dumps is fromJs with bytes on the far side, and loads is asJs - except
    // the instant, which the wire spells as its classic string.
    const value = { id: 1, tags: new Set(['a']) }
    assert.deepEqual(json.loads(json.dumps(value)), Scalar.from(value).asJs())
    assert.equal(json.loads(json.dumps({ at: new Date(0) })).at, '1970-01-01T00:00:00.000Z')
    assert.throws(() => Scalar.from({}, { maxDepth: 0 }), /between 1 and 48/)
  })

  test('Scalar.from applies a declared core Field exactly at intake', () => {
    const count = new Field('count', 'int8', false)
    const value = Scalar.from(127, { field: count })
    assert.equal(value.kind, 'i8')
    assert.equal(value.asJs(), 127)
    assert.throws(() => Scalar.from(128, { field: count }), /int8|count/)
    assert.throws(() => Scalar.from(null, { field: count }), /null|count/)
    assert.equal(Scalar.from(null, { field: new Field('count', 'int8', true) }).kind, 'null')
    const row = new Field('row', 'struct<id: int8 not null, release: version not null>', false)
    const resolved = Scalar.from({ release: '5.0.300', id: 7 }, { field: row })
    assert.equal(resolved.kind, 'sequence')
    assert.equal(resolved.get(0).kind, 'i8')
    assert.ok(resolved.get(1).asJs().equals(new Version(5, 0, 300)))
  })

  test('same-named native wrapper subclasses cannot lose application state', () => {
    const NativeDataType = DataType
    const NativeField = Field
    const NativeUri = Uri
    const NativeUrl = Url
    const NativeUrn = Urn
    const NativeVersion = Version
    const SubDataType = class DataType extends NativeDataType {
      constructor() {
        super('int64')
        this.applicationState = true
      }
    }
    const SubField = class Field extends NativeField {
      constructor() {
        super('id', 'int64', false)
        this.applicationState = true
      }
    }
    const SubUri = class Uri extends NativeUri {
      constructor() {
        super('https://example.com/value')
        this.applicationState = true
      }
    }
    const SubUrl = class Url extends NativeUrl {
      constructor() {
        super('https://example.com/value')
        this.applicationState = true
      }
    }
    const SubUrn = class Urn extends NativeUrn {
      constructor() {
        super('urn:example:value')
        this.applicationState = true
      }
    }
    const SubVersion = class Version extends NativeVersion {
      constructor() {
        super(5, 0, 300)
        this.applicationState = true
      }
    }

    for (const [value, name] of [
      [new SubDataType(), 'DataType'],
      [new SubField(), 'Field'],
      [new SubUri(), 'Uri'],
      [new SubUrl(), 'Url'],
      [new SubUrn(), 'Urn'],
      [new SubVersion(), 'Version'],
    ]) {
      assert.throws(() => json.dumps(value), new RegExp(`${name} subclasses`))
    }
  })

  test('a class instance crosses as its own properties and nothing else', () => {
    let constructorCalls = 0
    class Order {
      static yggdrylType = 'orders.Order'

      constructor(id) {
        constructorCalls += 1
        this.id = id
      }
    }

    const bytes = yaml.dumps(new Order(42))
    const text = bytes.toString()
    constructorCalls = 0
    const decoded = yaml.loads(bytes)

    // No name travels beside the data, so nothing on the read side can be asked
    // to look one up, import a module, or run a constructor.
    assert.doesNotMatch(text, /yggdryl/)
    assert.doesNotMatch(text, /orders\.Order/)
    assert.equal(Object.getPrototypeOf(decoded), Object.prototype)
    assert.deepEqual(decoded, { id: 42 })
    assert.equal(constructorCalls, 0)
  })

  test('decoded object keys never reach the prototype chain', () => {
    const value = {}
    Object.defineProperty(value, '__proto__', {
      enumerable: true,
      value: { polluted: true },
    })
    Object.defineProperty(value, 'constructor', {
      enumerable: true,
      value: 'payload data',
    })

    const decoded = yaml.loads(yaml.dumps(value))
    assert.deepEqual(decoded.__proto__, { polluted: true })
    assert.equal(decoded.constructor, 'payload data')
    assert.equal(Object.prototype.polluted, undefined)
    assert.equal(Object.getPrototypeOf(decoded), Object.prototype)
  })

  test('built-in brands cannot be spoofed by constructor names', () => {
    const NamedMap = class Map {
      constructor() {
        this.id = 1
      }
    }
    const NamedDate = class Date {
      constructor() {
        this.id = 2
      }
    }
    const NamedDataType = class DataType {
      constructor() {
        this.id = 3
      }
    }

    for (const Constructor of [NamedMap, NamedDate, NamedDataType]) {
      const decoded = yaml.loads(yaml.dumps(new Constructor()))
      assert.equal(Object.getPrototypeOf(decoded), Object.prototype)
      assert.deepEqual(decoded, { id: new Constructor().id })
    }
  })

  test('built-in subclasses fail explicitly instead of losing application state', () => {
    class MyMap extends Map {
      constructor() {
        super([['id', 1]])
        this.applicationState = true
      }
    }
    class MyDate extends Date {
      constructor() {
        super(0)
        this.applicationState = true
      }
    }
    class MyBytes extends Uint8Array {
      constructor() {
        super([1, 2])
        this.applicationState = true
      }
    }

    assert.throws(() => json.dumps(new MyMap()), /Map subclasses/)
    assert.throws(() => yaml.dumps(new MyDate()), /Date subclasses/)
    assert.throws(() => json.dumps(new MyBytes()), /Uint8Array subclasses/)
  })

  test('cross-realm slot-based built-ins fail instead of losing their contents', () => {
    for (const [value, name] of [
      [vm.runInNewContext('new Map([["id", 1]])'), 'Map'],
      [vm.runInNewContext('new Set([1, 2])'), 'Set'],
      [vm.runInNewContext('new RegExp("a/b", "giu")'), 'RegExp'],
    ]) {
      assert.throws(() => json.dumps(value), new RegExp(`cross-realm ${name}`))
    }
  })

  test('reserved transport keys and non-string map keys do not collide', () => {
    const publicBinding = require('yggdryl')
    const nativeHelpers = [
      'codecLoadsInferredNative',
      'jsonDumpPathNative',
      'jsonDumpsNative',
      'jsonLinesDumpAllNative',
      'jsonLinesDumpPathNative',
      'jsonLinesLoadPathNative',
      'jsonLinesLoadsNative',
      'jsonLoadPathNative',
      'jsonLoadsNative',
      'tomlDumpPathNative',
      'tomlDumpsNative',
      'tomlLoadPathNative',
      'tomlLoadsNative',
      'yamlDumpAllNative',
      'yamlDumpAllPathNative',
      'yamlDumpPathNative',
      'yamlDumpsNative',
      'yamlLoadAllPathNative',
      'yamlLoadPathNative',
      'yamlLoadsAllNative',
      'yamlLoadsNative',
    ]
    assert.equal(publicBinding.Scalar._fromJsNative, undefined)
    assert.equal(publicBinding.Scalar._fromDecimalPartsNative, undefined)
    assert.equal(publicBinding.Scalar._fromTemporalPartsNative, undefined)
    assert.equal(publicBinding.Scalar.prototype._asJsNative, undefined)
    for (const name of nativeHelpers) assert.equal(publicBinding[name], undefined)

    const generatedTypes = fs.readFileSync(
      path.join(__dirname, '..', '..', 'index.d.ts'),
      'utf8',
    )
    for (const name of nativeHelpers) assert.doesNotMatch(generatedTypes, new RegExp(name))
    const value = {}
    Object.defineProperty(value, '__yggdryl_codec__', {
      enumerable: true,
      value: 'datetime64',
    })
    Object.defineProperty(value, '__proto__', {
      enumerable: true,
      value: 'data',
    })
    Object.defineProperties(value, {
      unit: { enumerable: true, value: 'ms' },
      value: { enumerable: true, value: '0' },
      zone: { enumerable: true, value: null },
    })

    const decoded = json.loads(json.dumps(value))
    assert.equal(Object.getPrototypeOf(decoded), Object.prototype)
    assert.equal(decoded.__yggdryl_codec__, 'datetime64')
    assert.equal(decoded.__proto__, 'data')
    assert.equal(decoded.unit, 'ms')
    assert.equal(decoded.value, '0')
    assert.equal(decoded instanceof Date, false)

    const mapped = yaml.loads(yaml.dumps(new Map([[42, 'answer']])))
    assert.ok(mapped instanceof Map)
    assert.equal(mapped.get(42), 'answer')

    assert.throws(
      () => yaml.loads('1: integer\n1.0: floating\n'),
      /collide under JavaScript Map equality/,
    )
  })

  test('source intent is type-driven without existence probes', () => {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-codec-'))
    const file = path.join(directory, 'value.yaml')
    try {
      yaml.dump({ id: 42 }, file)
      // A string is content even when an existing file has that exact name.
      const statSync = fs.statSync
      let probed = false
      try {
        fs.statSync = (...arguments_) => {
          probed = true
          return statSync(...arguments_)
        }
        assert.equal(yaml.load(file), file)
      } finally {
        fs.statSync = statSync
      }
      assert.equal(probed, false)
      assert.equal(yaml.load('id: 43\n').id, 43)

      const yamlUrl = pathToFileURL(file)
      assert.equal(yaml.load(yamlUrl).id, 42)
      assert.equal(codec.from(yamlUrl).id, 42)

      const jsonFile = path.join(directory, 'value.json')
      const jsonUrl = pathToFileURL(jsonFile)
      codec.into({ id: 44 }, jsonUrl)
      assert.equal(codec.from(jsonUrl).id, 44)

      const jsonLinesFile = path.join(directory, 'rows.jsonl')
      const rows = [[1, 2], { id: 45 }]
      codec.into(rows, jsonLinesFile)
      assert.deepEqual(codec.from(pathToFileURL(jsonLinesFile)), rows)

      const misleadingJson = path.join(directory, 'actually-yaml.json')
      fs.writeFileSync(misleadingJson, 'id: 46\n')
      assert.deepEqual(
        codec.from(pathToFileURL(misleadingJson), { format: 'yaml' }),
        { id: 46 },
      )

      const misleadingYaml = path.join(directory, 'actually-json.yaml')
      codec.into({ id: 47 }, misleadingYaml, { format: 'json' })
      assert.deepEqual(json.load(pathToFileURL(misleadingYaml)), { id: 47 })

      const missingJsonLinesFile = path.join(directory, 'missing.jsonl')
      assert.equal(codec.from(missingJsonLinesFile), missingJsonLinesFile)
      assert.equal(codec.from(path.join(directory, 'missing.json')), path.join(directory, 'missing.json'))
      assert.equal(codec.from(path.join(directory, 'missing.yaml')), path.join(directory, 'missing.yaml'))
    } finally {
      fs.rmSync(directory, { force: true, recursive: true })
    }
  })

  test('string and byte-view content preserve exact source boundaries', () => {
    assert.deepEqual(json.loads('{"id":42}'), { id: 42 })
    assert.deepEqual(yaml.loads('id: 43\n'), { id: 43 })

    const framed = Buffer.from('xx{"id":44}yy')
    const view = new DataView(framed.buffer, framed.byteOffset + 2, 9)
    assert.deepEqual(json.loads(view), { id: 44 })

    const shared = new SharedArrayBuffer(7)
    new Uint8Array(shared).set(Buffer.from('[45,46]'))
    assert.deepEqual(json.loads(shared), [45, 46])
  })

  test('native path readers and writers bypass JavaScript whole-file buffers', () => {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-codec-native-path-'))
    const source = path.join(directory, 'source.yaml')
    const destination = path.join(directory, 'destination.json')
    fs.writeFileSync(source, 'id: 47\n')
    const readSync = fs.readSync
    const writeFileSync = fs.writeFileSync
    try {
      fs.readSync = () => {
        throw new Error('JavaScript read buffer must not be used for a real path')
      }
      assert.deepEqual(yaml.load(pathToFileURL(source)), { id: 47 })

      fs.writeFileSync = () => {
        throw new Error('JavaScript output buffer must not be used for a path')
      }
      json.dump({ id: 48 }, destination)
    } finally {
      fs.readSync = readSync
      fs.writeFileSync = writeFileSync
    }
    try {
      assert.deepEqual(JSON.parse(fs.readFileSync(destination, 'utf8')), { id: 48 })
    } finally {
      fs.rmSync(directory, { force: true, recursive: true })
    }
  })

  test('file descriptors remain caller-owned after synchronous codec I/O', () => {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-codec-fd-'))
    const source = path.join(directory, 'source.json')
    const destination = path.join(directory, 'destination.yaml')
    fs.writeFileSync(source, '{"id":49}')
    const sourceFd = fs.openSync(source, 'r')
    const destinationFd = fs.openSync(destination, 'w+')
    try {
      const fstatSync = fs.fstatSync
      try {
        fs.fstatSync = () => {
          throw new Error('descriptor reads must act without a metadata probe')
        }
        assert.deepEqual(json.load(sourceFd), { id: 49 })
      } finally {
        fs.fstatSync = fstatSync
      }
      assert.ok(fs.fstatSync(sourceFd).isFile())
      yaml.dump({ id: 50 }, destinationFd)
      assert.ok(fs.fstatSync(destinationFd).isFile())
    } finally {
      fs.closeSync(sourceFd)
      fs.closeSync(destinationFd)
      fs.rmSync(directory, { force: true, recursive: true })
    }
  })

  test('load and dump redirect Node and WHATWG streams with sound async behavior', async () => {
    assert.deepEqual(
      await json.load(Readable.from(['{"id":', '51}'])),
      { id: 51 },
    )

    const rows = []
    for await (const row of json.loadAll(
      Readable.from(['{"id":52}\n', '{"id":53}\n']),
    )) {
      rows.push(row)
    }
    assert.deepEqual(rows, [{ id: 52 }, { id: 53 }])

    const nodeChunks = []
    const nodeOutput = new Writable({
      write(chunk, _encoding, done) {
        nodeChunks.push(Buffer.from(chunk))
        done()
      },
    })
    await yaml.dump({ id: 54 }, nodeOutput)
    assert.deepEqual(yaml.loads(Buffer.concat(nodeChunks)), { id: 54 })

    const webChunks = []
    let webClosed = false
    const webOutput = new WritableStream({
      write(chunk) {
        webChunks.push(Buffer.from(chunk))
      },
      close() {
        webClosed = true
      },
    })
    await json.dump({ id: 55 }, webOutput)
    assert.deepEqual(json.loads(Buffer.concat(webChunks)), { id: 55 })
    assert.equal(webClosed, false)
  })

  test('redirected stream writes preserve errors and document backpressure', async () => {
    const expected = new Error('sink rejected frame')
    const failing = new Writable({
      write(_chunk, _encoding, done) {
        done(expected)
      },
    })
    await assert.rejects(yaml.dump({ id: 56 }, failing), (error) => error === expected)

    const cyclic = {}
    cyclic.self = cyclic
    const partialChunks = []
    const partial = new Writable({
      write(chunk, _encoding, done) {
        partialChunks.push(Buffer.from(chunk))
        done()
      },
    })
    await assert.rejects(yaml.dumpAll([{ id: 57 }, cyclic], partial), /cyclic/)
    const partialBytes = Buffer.concat(partialChunks)
    assert.doesNotMatch(partialBytes.toString(), /^---|\n---/)
    assert.deepEqual(yaml.loads(partialBytes), { id: 57 })

    let produced = 0
    let completedFrames = 0
    async function* values() {
      for (let id = 0; id < 3; id += 1) {
        assert.equal(produced, completedFrames)
        produced += 1
        yield { id }
      }
    }
    const chunks = []
    const slow = new Writable({
      write(chunk, _encoding, done) {
        chunks.push(Buffer.from(chunk))
        setImmediate(() => {
          if (chunk.length !== 0 && chunk[chunk.length - 1] === 0x0a) {
            completedFrames += 1
          }
          done()
        })
      },
    })
    await json.dumpAll(values(), slow)
    assert.equal(produced, 3)
    assert.equal(completedFrames, 3)
    assert.deepEqual(json.loadsAll(Buffer.concat(chunks)), [
      { id: 0 },
      { id: 1 },
      { id: 2 },
    ])
  })

  test('generic stream inference consumes anonymous sources exactly once', async () => {
    let pulls = 0
    async function* source() {
      pulls += 1
      yield '{"id":'
      pulls += 1
      yield '57}'
    }
    assert.deepEqual(await codec.from(source()), { id: 57 })
    assert.equal(pulls, 2)

    const rows = []
    for await (const row of codec.from(
      Readable.from(['{"id":58}\n', '{"id":59}\n']),
      { format: 'jsonl' },
    )) {
      rows.push(row)
    }
    assert.deepEqual(rows, [{ id: 58 }, { id: 59 }])

    let coerced = false
    assert.throws(
      () => codec.from('{}', {
        format: {
          toString() {
            coerced = true
            return 'json'
          },
        },
      }),
      /format must be a string/,
    )
    assert.equal(coerced, false)
    assert.equal(codec.from('\u00a0{"id":60}'), '{"id":60}')
  })

  test('string stream chunks preserve split Unicode surrogate pairs centrally', async () => {
    async function* jsonChunks() {
      yield '{"value":"\ud83d'
      yield '\ude42"}'
    }
    assert.deepEqual(await json.load(jsonChunks()), { value: '🙂' })
    assert.deepEqual(await codec.from(jsonChunks()), { value: '🙂' })

    const jsonLinesValues = []
    for await (const value of json.loadAllStream(jsonChunksWithNewline())) {
      jsonLinesValues.push(value)
    }
    assert.deepEqual(jsonLinesValues, [{ value: '🙂' }])

    async function* jsonChunksWithNewline() {
      yield '{"value":"\ud83d'
      yield '\ude42"}\n'
    }

    const yamlValues = []
    for await (const value of yaml.loadAllStream(yamlChunks())) {
      yamlValues.push(value)
    }
    assert.deepEqual(yamlValues, [{ value: '🙂' }])

    async function* yamlChunks() {
      yield '---\nvalue: "\ud83d'
      yield '\ude42"\n'
    }

    async function* danglingHighSurrogate() {
      yield 'value: \ud83d'
    }
    async function* unpairedLowSurrogate() {
      yield 'value: \ude42'
    }
    assert.deepEqual(await yaml.load(danglingHighSurrogate()), { value: '�' })
    assert.deepEqual(await yaml.load(unpairedLowSurrogate()), { value: '�' })
  })

  test('path and single-value stream reads enforce limits before unbounded allocation', async () => {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-codec-limit-'))
    const file = path.join(directory, 'oversized.json')
    try {
      fs.writeFileSync(file, '')
      fs.truncateSync(file, 64 * 1024 * 1024 + 1)
      assert.throws(() => json.load(pathToFileURL(file)), /input limit/)
    } finally {
      fs.rmSync(directory, { force: true, recursive: true })
    }

    const chunk = Buffer.alloc(1024 * 1024)
    let yielded = 0
    async function* oversized() {
      for (let index = 0; index < 65; index += 1) {
        yielded += 1
        yield chunk
      }
    }
    await assert.rejects(() => json.loadStream(oversized()), /input limit/)
    assert.equal(yielded, 65)

    let pulledInvalid = false
    async function* invalidOptionsSource() {
      pulledInvalid = true
      yield '{}'
    }
    await assert.rejects(
      () => json.loadStream(invalidOptionsSource(), { maxDepth: 0 }),
      /between 1 and 48/,
    )
    assert.equal(pulledInvalid, false)
  })

  test('buffered JSON Lines and YAML document collections round-trip', () => {
    const values = [{ id: 1 }, { id: 2 }, { id: 3 }]

    assert.deepEqual(json.loadsAll(json.dumpAll(values)), values)
    assert.deepEqual(yaml.loadsAll(yaml.dumpAll(values)), values)
  })

  test('buffered collection encoders stop before unbounded materialization', () => {
    let pulled = 0
    let closed = false
    function* tooMany() {
      try {
        while (true) {
          pulled += 1
          yield { id: pulled }
        }
      } finally {
        closed = true
      }
    }

    assert.throws(() => json.dumpAll(tooMany()), /1024-document limit/)
    assert.equal(pulled, 1025)
    assert.equal(closed, true)
    assert.throws(
      () =>
        nativeYamlDumpAll(
          Array(1025).fill(null),
          undefined,
          'default',
          rawNativeWrapperPrototypes,
          rawNativeIntrinsics,
        ),
      /1024-document limit/,
    )
  })

  test('generic JSON Lines operations treat the value as a row collection', async () => {
    const rows = [[1, 2], { id: 2 }]
    const bytes = codec.into(rows, { format: 'jsonl' })
    assert.deepEqual(codec.from(bytes, { format: 'ndjson' }), rows)
    assert.deepEqual(codec.from(bytes, { format: 'json-lines' }), rows)

    const decoded = codec.fromStream(
      Readable.from([Buffer.from('[1,2]\n'), Buffer.from('{"id":2}\n')]),
      { format: 'json_lines' },
    )
    assert.equal(typeof decoded[Symbol.asyncIterator], 'function')
    const streamedRows = []
    for await (const row of decoded) streamedRows.push(row)
    assert.deepEqual(streamedRows, rows)

    const chunks = []
    const output = new Writable({
      write(chunk, _encoding, done) {
        chunks.push(Buffer.from(chunk))
        done()
      },
    })
    await codec.intoStream(rows, output, { format: 'jsonl' })
    assert.deepEqual(codec.from(Buffer.concat(chunks), { format: 'jsonl' }), rows)
  })

  test('JSON Lines streams decode incrementally across arbitrary chunks', async () => {
    let pulls = 0
    async function* chunks() {
      pulls += 1
      yield Buffer.from('{"id":1}\n{"i')
      pulls += 1
      yield Buffer.from('d":2}\r\n')
    }

    const iterator = json.loadAllStream(chunks())[Symbol.asyncIterator]()
    assert.deepEqual((await iterator.next()).value, { id: 1 })
    assert.equal(pulls, 1)
    assert.deepEqual((await iterator.next()).value, { id: 2 })
    assert.equal(pulls, 2)
    assert.equal((await iterator.next()).done, true)
  })

  test('one-byte stream chunks do not require repeated whole-line copies', async () => {
    const payload = 'x'.repeat(32 * 1024)
    const encoded = Buffer.from(JSON.stringify({ payload }) + '\n')
    async function* bytes() {
      for (const byte of encoded) yield Buffer.of(byte)
    }

    const values = []
    for await (const value of json.loadAllStream(bytes())) values.push(value)
    assert.equal(values.length, 1)
    assert.equal(values[0].payload, payload)
  })

  test('YAML streams preserve block content and decode one document at a time', async () => {
    const input = Readable.from([
      Buffer.from('---\nid: 1\nnote: |\n  keep\n  ---\n'),
      Buffer.from('---\nid: 2\n'),
    ])
    const values = []
    for await (const value of yaml.loadAllStream(input)) values.push(value)

    assert.deepEqual(values, [
      { id: 1, note: 'keep\n---\n' },
      { id: 2 },
    ])
  })

  test('YAML streams preserve explicit null documents at every position', async () => {
    const input = Readable.from([
      Buffer.from('---\nnull\n---\n'),
      Buffer.from('---\nid: 1\n---'),
    ])
    const values = []
    for await (const value of yaml.loadAllStream(input)) values.push(value)

    assert.deepEqual(values, [null, null, { id: 1 }, null])
  })

  test('YAML stream framing matches core marker separation rules', async () => {
    const content = Buffer.from('---#not-a-marker\n...#not-an-end\n')
    const buffered = yaml.loadsAll(content)
    const streamed = []
    for await (const value of yaml.loadAllStream(Readable.from([content]))) {
      streamed.push(value)
    }
    assert.deepEqual(streamed, buffered)
  })

  test('YAML streams frame LF, CRLF, and lone CR across chunk boundaries', async () => {
    async function collect(chunks) {
      async function* source() {
        yield* chunks
      }
      const values = []
      for await (const value of yaml.loadAllStream(source())) values.push(value)
      return values
    }

    const loneCarriageReturn = Buffer.from('---\rid: 1\r---\rid: 2\r')
    assert.deepEqual(await collect([loneCarriageReturn]), [
      { id: 1 },
      { id: 2 },
    ])
    assert.deepEqual(
      await collect([Buffer.from('---\r')]),
      yaml.loadsAll(Buffer.from('---\r')),
    )

    const crlf = Buffer.from('---\r\nid: 1\r\n---\r\nid: 2\r\n')
    assert.deepEqual(
      await collect(['---\r', '\nid: 1\r', '\n---\r', '\nid: 2\r', '\n']),
      yaml.loadsAll(crlf),
    )
  })

  test('YAML stream preamble classification uses exact YAML bytes', async () => {
    async function collect(chunks) {
      async function* source() {
        yield* chunks
      }
      const values = []
      for await (const value of yaml.loadAllStream(source())) values.push(value)
      return values
    }

    async function assertParity(content, chunks = [content]) {
      let expected
      try {
        expected = yaml.loadsAll(content)
      } catch (bufferedError) {
        await assert.rejects(
          () => collect(chunks),
          (streamError) => {
            assert.match(streamError.message, /YAML stream error/)
            assert.match(streamError.message, /cumulative byte/)
            assert.equal(streamError.cause?.name, bufferedError.name)
            return true
          },
        )
        return
      }
      assert.deepEqual(await collect(chunks), expected)
    }

    assert.deepEqual(await collect(['...\n']), [])
    assert.deepEqual(await collect(['# comment\n', '...\n']), [])
    await assertParity('  %YAML 1.2\n', ['  %Y', 'AML 1.2\n'])
    await assertParity('%YAML 1.2\n...\n', ['%YAML 1.2\n', '...\n'])
    await assertParity('%YAML 1.2\n---\nid: 1\n')

    const nonBreakingSpaceScalar = '\u00a0# scalar, not a comment\n'
    assert.deepEqual(yaml.loadsAll(nonBreakingSpaceScalar), [
      '# scalar, not a comment',
    ])
    await assertParity(nonBreakingSpaceScalar, ['\u00a0', '# scalar, not a comment\n'])

    // JavaScript trimStart() treats these controls as whitespace. YAML framing
    // must pass them to the native parser instead of silently discarding them.
    await assertParity('\u000b\n', ['\u000b', '\n'])
    await assertParity('\u000c\n', ['\u000c', '\n'])
  })

  test('YAML stream document splits preserve block-scalar end context', async () => {
    async function collectOneByteChunks(content) {
      async function* chunks() {
        for (const byte of Buffer.from(content)) yield Buffer.of(byte)
      }
      const values = []
      for await (const value of yaml.loadAllStream(chunks())) values.push(value)
      return values
    }

    for (const indicator of ['|', '|-', '|+', '>', '>-', '>+']) {
      const empty = `text: ${indicator}\n---\nid: 2\n`
      assert.deepEqual(await collectOneByteChunks(empty), yaml.loadsAll(empty))

      const populated = `text: ${indicator}\n  first\n  second\n---\nid: 2\n`
      assert.deepEqual(
        await collectOneByteChunks(populated),
        yaml.loadsAll(populated),
      )
    }

    const loneCarriageReturn = 'text: |\r---\rid: 2\r'
    assert.deepEqual(
      await collectOneByteChunks(loneCarriageReturn),
      yaml.loadsAll(loneCarriageReturn),
    )

    await assert.rejects(
      () => collectOneByteChunks('items: [1\n---\nid: 2\n'),
      /cumulative byte 10/,
    )
  })

  test('streamed writers frame and flush each value separately', async () => {
    const chunks = []
    const output = new Writable({
      write(chunk, _encoding, done) {
        chunks.push(Buffer.from(chunk))
        done()
      },
    })

    async function* values() {
      yield [1, 2]
      yield { id: 2 }
    }

    await json.dumpAllStream(values(), output)
    assert.ok(chunks.length >= 2)
    assert.deepEqual(json.loadsAll(Buffer.concat(chunks)), [[1, 2], { id: 2 }])
  })

  test('stream errors report cumulative byte offsets', async () => {
    const input = Readable.from([
      Buffer.from('{"id":1}\n'),
      Buffer.from('{bad}\n'),
    ])

    await assert.rejects(
      async () => {
        for await (const _value of json.loadAllStream(input)) {
          // Drain the iterator to surface its second-line failure.
        }
      },
      /cumulative byte 10 \(frame byte 1\)/,
    )

    await assert.rejects(
      async () => {
        for await (const _value of json.loadAllStream(
          Readable.from([Buffer.from([0xc2, 0xa0, 0x0a])]),
        )) {
          // A non-breaking space is not legal JSON whitespace.
        }
      },
      /JSON Lines stream error/,
    )
  })

  test('structured codec formatting redirects to the core for every destination', () => {
    const value = { outer: { enabled: true, values: [1, 2] } }
    assert.equal(
      json.dumps(value, { indent: 2 }).toString(),
      '{\n  "outer": {\n    "enabled": true,\n    "values": [\n      1,\n      2\n    ]\n  }\n}',
    )
    assert.deepEqual(json.loads(json.dumps(value, { indent: null })), value)
    assert.match(yaml.dumps(value, { indent: null }).toString(), /^\{.*\}\n?$/s)

    for (const [name, format] of [['json', json], ['yaml', yaml], ['toml', toml]]) {
      for (const indent of [null, 0, 2, 255, '\t']) {
        const encoded = format.dumps(value, { indent })
        assert.deepEqual(format.loads(encoded), value, `${name} indent ${String(indent)}`)
        assert.deepEqual(
          codec.into(value, { format: name, indent }),
          encoded,
          `${name} generic redirect`,
        )
      }
    }

    assert.throws(() => json.dumps(value, { indent: -1 }), /indent/)
    assert.throws(() => json.dumps(value, { indent: '  ' }), /indent/)
    assert.throws(() => json.dumps(value, { indent: 256 }), /indent/)
  })

  test('nullable parser limits reach byte, inferred, collection, path, and stream decoders', async (t) => {
    assert.deepEqual(json.loads('{"id":1}', { maxDepth: null }), { id: 1 })
    assert.throws(() => json.loads('{"id":1}', { maxInputBytes: 3 }), /input byte limit/i)
    assert.throws(() => json.loads('[1,2]', { maxNodes: 2 }), /node limit/i)
    assert.throws(() => json.loadsAll('1\n2\n', { maxDocuments: 1 }), /document limit/i)
    assert.throws(() => yaml.loadsAll('---\n1\n---\n2\n', { maxDocuments: 1 }), /document limit/i)

    const field = new Field('row', 'struct<id:int64 not null>', false)
    assert.throws(
      () => codec.from('{"id":1}', { field, maxNodes: 1 }),
      /node limit/i,
    )

    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-codec-options-'))
    t.after(() => fs.rmSync(directory, { recursive: true, force: true }))
    const file = path.join(directory, 'value.json')
    fs.writeFileSync(file, '{"id":1}')
    assert.throws(
      () => json.load(pathToFileURL(file), { maxInputBytes: 3 }),
      /input limit/i,
    )

    await assert.rejects(
      () => json.load(Readable.from(['{"id":', '1}']), { maxInputBytes: 4 }),
      /4-byte input limit/,
    )
    const rows = []
    await assert.rejects(
      async () => {
        for await (const row of json.loadAllStream(
          Readable.from(['1\n', '2\n']),
          { maxDocuments: 1 },
        )) rows.push(row)
      },
      /document limit/i,
    )
    assert.deepEqual(rows, [1])

    assert.throws(() => json.loads('1', { maxNodes: -1 }), /maxNodes/)
    assert.throws(() => json.loads('1', { maxInputBytes: 1.5 }), /maxInputBytes/)
    assert.throws(() => json.loads('1', { maxDocuments: '1' }), /maxDocuments/)
  })

  test('depth limits reject adversarial nested input without recursion overflow', () => {
    let value = 0
    for (let index = 0; index < 32; index += 1) value = [value]

    assert.throws(() => json.dumps(value, { maxDepth: 16 }), /maxDepth 16/)
    const bytes = json.dumps(value)
    assert.throws(() => json.loads(bytes, { maxDepth: 16 }), /depth/i)

    let adversarial = 0
    for (let index = 0; index < 5000; index += 1) adversarial = [adversarial]
    assert.throws(
      () => json.dumps(adversarial, { maxDepth: 5000 }),
      /between 1 and 48/,
    )
    assert.throws(() => json.dumps(adversarial), /maxDepth 48/)
    assert.throws(
      () =>
        nativeJsonDumps(
          {},
          5000,
          'default',
          rawNativeWrapperPrototypes,
          rawNativeIntrinsics,
        ),
      /between 1 and 48/,
    )
  })

  test('malformed streams and cyclic objects fail deterministically', () => {
    const cyclic = {}
    cyclic.self = cyclic

    assert.throws(() => yaml.dumps(cyclic), /cyclic/)
    assert.throws(() => json.loads('{"missing":]'), /JSON/i)
    assert.throws(() => yaml.loads('value: [unterminated'), /YAML/i)
  })

  test('the byte codings round-trip and read node:zlib output', () => {
    const zlibNative = require('node:zlib')
    const { gzip, zlib, zstd } = require('yggdryl')
    const payload = Buffer.from('{"id":1}\n'.repeat(512))

    assert.deepEqual(gzip.loads(gzip.dumps(payload)), payload)
    assert.deepEqual(gzip.loads(zlibNative.gzipSync(payload)), payload)
    assert.deepEqual(zlibNative.gunzipSync(gzip.dumps(payload, 9)), payload)
    assert.deepEqual(zlib.loads(zlib.dumps(payload)), payload)
    assert.deepEqual(zstd.loads(zstd.dumps(payload, 9)), payload)
  })

  test('raw DEFLATE round-trips, reads node:zlib, and shares no framing with zlib', () => {
    const zlibNative = require('node:zlib')
    const { zlib } = require('yggdryl')
    const payload = Buffer.from('{"id":1}\n'.repeat(512))

    assert.deepEqual(zlib.loadsRaw(zlib.dumpsRaw(payload)), payload)
    assert.deepEqual(zlib.loadsRaw(zlibNative.deflateRawSync(payload)), payload)
    assert.deepEqual(zlibNative.inflateRawSync(zlib.dumpsRaw(payload, 9)), payload)
    // The raw output is the framed output without the two-byte header and the
    // four-byte checksum, which is the whole of the difference.
    assert.equal(zlib.dumpsRaw(payload).length + 6, zlib.dumps(payload).length)

    // Nothing in unframed bytes says which framing they are, so the pair is
    // named rather than sniffed - and each half refuses the other's output
    // instead of decoding it into something plausible.
    assert.throws(() => zlib.loads(zlib.dumpsRaw(payload)), /deflate/)
    assert.throws(() => zlib.loadsRaw(zlib.dumps(payload)), /deflate/)
  })

  test('value bytes carry any value and read back as itself', () => {
    const value = new DataType('int32').scalar(7)
    const data = value.intoValueBytes()
    assert.ok(Buffer.isBuffer(data))
    // The version, the identifier, then four little-endian bytes.
    assert.equal(data[0], 0)
    assert.equal(data.length, 6)
    assert.deepEqual([...data.subarray(2)], [7, 0, 0, 0])
    assert.ok(Scalar.fromValueBytes(data).equals(value))
    assert.ok(Scalar.fromValueBytes(new Uint8Array(data)).equals(value))

    const quote = Scalar.from({ symbol: 'AAPL', sizes: [100, null] })
    assert.ok(Scalar.fromValueBytes(quote.intoValueBytes()).equals(quote))
    assert.equal(Scalar.fromValueBytes(quote.intoValueBytes()).kind, 'struct')

    for (const [raw, expected] of [
      [{ symbol: 'AAPL', sizes: [100, null] }, { symbol: 'AAPL', sizes: [100, null] }],
      [7, 7],
      [null, null],
    ]) {
      const held = new DataType('variant').scalar(raw)
      assert.equal(held.kind, 'variant')
      assert.deepEqual(held.asJs(), expected)
      const restored = Scalar.fromValueBytes(held.intoValueBytes())
      assert.equal(restored.kind, 'variant')
      assert.ok(restored.equals(held))
    }
    const optional = new Field('payload', new DataType('variant'), true)
    const required = new Field('payload', new DataType('variant'), false)
    const encodedNull = new DataType('variant').scalar(null)
    assert.equal(optional.scalar(null).kind, 'null')
    assert.throws(() => required.scalar(null), /non-nullable/)
    assert.ok(required.scalar(encodedNull).equals(encodedNull))

    const long = Scalar.from('x'.repeat(4 * 1024 + 1))
    const packed = long.intoValueBytes()
    assert.equal(packed[2], 1)
    assert.ok(packed.length < 64)
    assert.ok(Scalar.fromValueBytes(packed).equals(long))

    assert.throws(() => Scalar.fromValueBytes(Buffer.from([1, 0])), /version 1/)
    assert.throws(() => Scalar.fromValueBytes(Buffer.from([0, 0, 0])), /bytes left/)
  })
}
