import {
  BatchReader,
  Field,
  IOBase,
  MediaType,
  Scalar,
  Url,
  http,
  type Headers,
  type HttpCookie,
  type HttpHop,
  type HttpLink,
  type HttpRecorded,
  type HttpRequestOptions,
  type HttpStats,
  type Pages,
  type Request,
  type Response,
  type Server,
  type Session,
} from '..'
import type { Table as ArrowTable } from 'apache-arrow'

const session: Session = new http.Session('https://api.example.com/v1/', {
  headers: { 'X-Api-Key': 'k-123' },
  auth: ['user', 'secret'],
  timeout: 30_000,
  options: { max_attempts: 5, follow_redirects: true },
})
const bare: Session = new http.Session()
const shared: Session = http.session()
const base: Url | null = session.baseUrl
const timeout: number = session.timeout
const stats: HttpStats = session.stats
const cookies: HttpCookie[] = session.cookies
session.setCookie('sid', 'abc', 'api.example.com')

const options: HttpRequestOptions = {
  params: new URLSearchParams({ limit: '10' }),
  headers: new Map([['Accept', 'application/json']]),
  json: { id: 7 },
  auth: { bearer: 'token' },
  followRedirects: false,
  pagination: 'cursor:next_cursor:cursor',
  records: 'data',
  mediaType: 'application/json',
}
const response: Response = session.get('orders', options)
const posted: Response = http.post(new URL('https://api.example.com/v1/orders'), {
  data: 'raw',
})
const streamed: Response = session.stream('files/big.bin', { timeout: 5_000 })
const status: number = response.statusCode
const reason: string = response.reason
const ok: boolean = response.ok
const url: Url = response.url
const headers: Headers = response.headers
const history: HttpHop[] = response.history
const elapsed: number = response.elapsed
const links: HttpLink[] = response.links
const next: Request | null = response.next
response.close()
const reopened: boolean = response.opened
const encoding: string | null = response.encoding
const mediaType: MediaType = response.mediaType
const content: Buffer = response.content()
const text: string = response.text()
const document: unknown = response.json()
const scalar: Scalar = response.scalar()
const typed: Scalar = response.scalar('rows struct<id: int64>')
const raised: Response = response.raiseForStatus()
const body: IOBase = streamed.intoIOBase()

const contentType: string | null = headers.get('content-type')
const members: string[] = headers.getAll('accept')
const present: boolean = headers.has('etag')
const names: string[] = headers.keys()
const entries: Array<[string, string]> = headers.entries()
const count: number = headers.length
const plain: Record<string, unknown> = headers.toJSON()
const length: number | null = headers.contentLength
const modified: bigint | null = headers.lastModified
for (const [name, value] of headers) void [name, value]
const built: Headers = new http.Headers({ Accept: ['a', 'b'] })

const request: Request = new http.Request('GET', 'https://api.example.com/v1/orders')
  .withQuery({ limit: 10 })
  .withHeaders([['Accept', 'application/json']])
  .withAuthorization(['user', 'secret'])
  .withTimeout(1_000)
  .withPagination('link')
  .withRecords('data')
  .withSession(session)
const withBody: Request = request.withBody(new Uint8Array([1, 2])).withJson({ a: 1 })
const sent: Response = request.send()
const resource: IOBase = request.intoIOBase()
const wire: Buffer = request.intoBytes()
const parsed: Request = http.Request.fromBytes(wire)
const answers: Array<Response | Error> = [
  ...session.sendAll([request, 'orders?page=2', { method: 'POST', url: 'orders', json: { n: 1 } }], 4),
]

const pages: Pages = session.pages('orders', { records: 'data' })
for (const page of pages) void page.statusCode
const reader: BatchReader = request.pages().intoArrowReader(Field.from('rows struct<id: int64>'), 1_000)
const table: ArrowTable = request.pages().intoTable()

const server: Server = http.Server.bind('127.0.0.1:0', { keepAlive: false, readTimeout: 5_000 })
server.mount('/data', new IOBase('/tmp/lake'))
server.respond('/health', 200, { 'Content-Type': 'text/plain' }, 'ok')
server.respond('/created', 201, null, null, 'POST')
server.inject('/data/a.txt', { refuse: 503, retryAfter: 1_000 }, 2)
server.inject('/data/a.txt', 'closeBeforeAnswer')
const recorded: HttpRecorded[] = server.requests
const handled: number = server.requestCount
const port: number = server.port
server.clearRequests()
server.shutdown()

// @ts-expect-error a request option this binding does not read
session.get('orders', { retries: 3 })
// @ts-expect-error the fault names are closed
server.inject('/x', 'explode')
// @ts-expect-error a response has no public constructor arguments
new http.Response(200)

void [
  bare, shared, base, timeout, stats, cookies, posted, status, reason, ok, url, history,
  elapsed, links, next, encoding, mediaType, content, text, document, scalar, typed, raised,
  body, contentType, members, present, names, entries, count, plain, length, modified, built,
  withBody, sent, resource, parsed, answers, reader, table, recorded, handled, port,
]
