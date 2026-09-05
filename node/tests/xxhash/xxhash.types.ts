import { Buffer } from 'node:buffer'

import { Digest, IOBase, Scalar, Xxh3, Xxh128, Xxh32, Xxh64, enums, xxhash } from 'yggdryl'

const payload = Buffer.from('AAPL,187.23')

// One-shot: XXH32 answers a number, the wider algorithms answer bigints.
const narrow: number = xxhash.xxh32(payload)
const wide: bigint = xxhash.xxh64(payload)
const fast: bigint = xxhash.xxh3(payload)
const widest: bigint = xxhash.xxh128(payload)
void narrow
void wide
void fast
void widest

// Every byte shape a digest reads.
xxhash.xxh3(new Uint8Array(payload))
xxhash.xxh3(payload.buffer)
xxhash.xxh3('AAPL,187.23')

// Seeds and secrets.
xxhash.xxh32(payload, { seed: 42 })
xxhash.xxh64(payload, { seed: 42n })
xxhash.xxh3(payload, { seed: 42n, secret: new Uint8Array(xxhash.SECRET_MINIMUM_LENGTH) })
xxhash.xxh128(payload, 42n)

// The value that carries its algorithm.
const digest: Digest = xxhash.digest(payload, 'xxh3-64')
const algorithm: string = digest.algorithm
const width: number = digest.width
const bits: number = digest.bits
const bytes: Uint8Array = digest.bytes()
const value: number | bigint = digest.value()
const same: boolean = digest.equals(Digest.from(digest.toString()))
const order: number = digest.compare(digest)
const stable: bigint = digest.stableHash()
const cloned: Digest = digest.clone()
const rendered: string = digest.toJSON()
void algorithm
void width
void bits
void bytes
void value
void same
void order
void stable
void cloned
void rendered
void Digest.fromBytes('xxh3-64', bytes)

// The four resumable states.
const state32: Xxh32 = new Xxh32(42)
const state64: Xxh64 = new Xxh64(42n)
const state3_64: Xxh3 = new Xxh3(42n, new Uint8Array(xxhash.SECRET_MINIMUM_LENGTH))
const state3_128: Xxh128 = new Xxh128()
state32.writeBytes(payload)
state64.writeBytes('AAPL')
state3_64.writeBytes(new Uint8Array(payload))
state3_128.writeScalar(Scalar.fromJs('AAPL'))
const streamed: Digest = state3_128.asDigest()
const seed32: number = state32.seed
const seed64: bigint = state64.seed
const custom: Uint8Array | null = state3_64.secret
const copied: Xxh3 = state3_64.clone()
state32.clear()
void streamed
void seed32
void seed64
void custom
void copied
void state3_64.algorithm

// The classes are also reachable through the namespace.
void new xxhash.Xxh3()
void xxhash.Digest.from('xxh3-64:78af5f94892f3950')

// Handles and values redirect to the same native path.
const handle = new IOBase('/tmp/trades.csv')
const fromHandle: Digest = handle.readDigest('xxh3-64')
const ranged: Digest = handle.readRangeDigest(0, 16)
const fromValue: Digest = Scalar.fromJs('AAPL').digest()
void fromHandle
void ranged
void fromValue

// The algorithm vocabulary is a narrowed union.
const algorithms: readonly ('xxh32' | 'xxh64' | 'xxh3-64' | 'xxh3-128')[] = enums.digestAlgorithms
void algorithms
