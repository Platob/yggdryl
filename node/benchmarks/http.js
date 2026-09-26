'use strict'

// The HTTP boundary: the core's synchronous client against Node's own
// `fetch`, both asking the crate's `Server` on the loopback, which answers on
// native threads so neither client waits on this event loop.
const { performance } = require('node:perf_hooks')
const { IOBase, MimeType, http } = require('yggdryl')

const smoke = process.env.YGGDRYL_BENCH_SMOKE === '1'
const iterations = Number.parseInt(
  smoke ? '1' : (process.env.YGGDRYL_BENCH_ITERATIONS ?? '2000'),
  10,
)
if (!Number.isSafeInteger(iterations) || iterations <= 0) {
  throw new RangeError('YGGDRYL_BENCH_ITERATIONS must be a positive safe integer')
}

async function benchmark(name, operation, scale = 1) {
  const rounds = Math.max(1, Math.floor(iterations / scale))
  const warmups = smoke ? 0 : Math.min(rounds, 100)
  for (let index = 0; index < warmups; index += 1) await operation()
  const started = performance.now()
  for (let index = 0; index < rounds; index += 1) await operation()
  const elapsed = performance.now() - started
  const each = (elapsed * 1_000) / rounds
  console.log(`${name}: ${each.toFixed(1)} us/operation`)
}

const FAN_OUT = 64
const small = IOBase.fromBytes(Buffer.from('{"symbol":"AAPL","price":1}'))
small.mediaType = MimeType.JSON
const server = http.Server.bind()
server.setRecording(false)
server.mount('/small.json', small)
server.mount('/blob', IOBase.fromBytes(Buffer.alloc(1 << 20, 7)))
const base = server.url.toString()
const session = new http.Session(base)
const fanOut = Array.from({ length: FAN_OUT }, () => 'small.json')
const fanOutUrls = fanOut.map((path) => base + path)

async function main() {
  await benchmark('http/get_small_text', () => session.get('small.json').text())
  await benchmark('http/get_small_text_fetch', async () =>
    (await fetch(base + 'small.json')).text(),
  )
  await benchmark('http/get_small_json', () => session.get('small.json').json())
  await benchmark('http/get_small_json_fetch', async () =>
    (await fetch(base + 'small.json')).json(),
  )
  await benchmark('http/get_1mib_bytes', () => session.get('blob').content(), 20)
  await benchmark(
    'http/get_1mib_bytes_fetch',
    async () => Buffer.from(await (await fetch(base + 'blob')).arrayBuffer()),
    20,
  )
  await benchmark(
    `http/send_all_${FAN_OUT}`,
    () => {
      for (const answer of session.sendAll(fanOut, 8)) answer.text()
    },
    FAN_OUT,
  )
  await benchmark(
    `http/fetch_all_${FAN_OUT}`,
    () => Promise.all(fanOutUrls.map(async (url) => (await fetch(url)).text())),
    FAN_OUT,
  )
  await benchmark('http/headers_from_object', () =>
    new http.Headers({ accept: 'application/json', 'x-request-id': 'abc' }),
  )
}

main().finally(() => server.shutdown())
