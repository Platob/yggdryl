// `node/book.d.ts`, reached the way a consumer reaches it: the package's own
// name, resolved through the `./book` entry of its `exports` map under the
// strict Node16 settings of `node/tsconfig.json`.
import book = require('yggdryl/book')
import type { ServeExit, ServeOptions, ServeTable, Serving } from 'yggdryl/book'
import type { Readable } from 'node:stream'

const folder: string = book.assets
const files: readonly string[] = book.assetFiles
// @ts-expect-error the embedded file list is frozen
book.assetFiles.push('extra.js')

const bare: string[] = book.serveArguments()
const tables: ServeTable[] = ['books=/data/books', { name: 'ref', location: 's3://bucket/ref' }, { location: '/data/other' }]
const spelled: string[] = book.serveArguments({
  tables,
  bind: '0.0.0.0:8080',
  path: '/book',
  capture: ['a.log', 'b.log'],
  args: ['--snapshot-millis', '250'],
})
// A single table, log or argument stands for a list of one, and null for none.
const single: string[] = book.serveArguments({ tables: 'books=/data/books', capture: 'a.log', args: null })
// @ts-expect-error a table object names its location
book.serveArguments({ tables: [{ name: 'books' }] })
// @ts-expect-error an option the spawner does not take is refused
book.serveArguments({ table: 'books=/data/books' })

async function display(): Promise<ServeExit> {
  const options: ServeOptions = {
    tables: ['books=/data/books'],
    capture: 'bridge.log',
    bin: '/usr/local/bin/yggdryl',
    env: { ...process.env, RUST_LOG: 'info' },
    signal: AbortSignal.timeout(10_000),
  }
  const started: Serving = await book.serve(options)
  const endpoint: string = started.endpoint
  const pid: number | undefined = started.process.pid
  const stdin: null = started.process.stdin
  const stdout: Readable = started.process.stdout
  const status: ServeExit = await started.close()
  const code: number | null = status.code
  const signal: NodeJS.Signals | null = status.signal
  void [endpoint, pid, stdin, stdout, code, signal]
  // @ts-expect-error `close` answers a promise of the exit status, not the status
  const unawaited: ServeExit = started.close()
  void unawaited
  return book.serve().then((serving) => serving.close())
}

export { folder, files, bare, spelled, single, display }
