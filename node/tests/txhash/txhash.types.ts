import { Buffer } from 'node:buffer'
import type { RecordBatch as ArrowRecordBatch } from 'apache-arrow'

import { DataType, Digest, Field, Scalar, TxHash, TxHasher, Xxh3, txhash, xxhash } from 'yggdryl'

const payload = Buffer.from('AAPL,187.23')
const instant = 1_700_000_000_000_000n

// One-shot: every algorithm answers the coupled value.
const narrow: TxHash = txhash.txh32(payload, instant)
const wide: TxHash = txhash.txh64(payload, instant, { seed: 42n })
const fast: TxHash = txhash.txh3('AAPL', new Date(), 42n)
const widest: TxHash = txhash.txh128(new Uint8Array(payload), '2023-11-14T22:13:20Z')
const chosen: TxHash = txhash.digest(payload, Scalar.fromJs(new Date()), 'xxh3-128')
void narrow
void wide
void fast
void widest
void chosen

// The value.
const unix: bigint = narrow.unix
const unit: string = narrow.unit
const algorithm: string = narrow.algorithm
const width: number = narrow.width
const half: Digest = narrow.digest
const dtype: DataType = narrow.dtype
const bytes: Uint8Array = narrow.bytes()
const restated: TxHash = narrow.withUnit('s')
const datetime: Scalar = narrow.intoDatetime()
const cell: Scalar = narrow.intoScalar()
const same: boolean = narrow.equals(TxHash.from(narrow.toString()))
const order: number = narrow.compare(wide)
const stable: bigint = narrow.stableHash()
const cloned: TxHash = narrow.clone()
const rendered: string = narrow.toJSON()
const parts: TxHash = TxHash.fromParts(instant, half, 'ms')
const rebuilt: TxHash = TxHash.fromBytes('us', 'xxh32', bytes)
void unix
void unit
void algorithm
void width
void dtype
void restated
void datetime
void cell
void same
void order
void stable
void cloned
void rendered
void parts
void rebuilt
void new TxHash(narrow.toString())

// The configuration.
const hasher: TxHasher = new TxHasher('xxh64', 's', 7n)
const carried: TxHasher = TxHasher.fromState(new Xxh3(1n), 'ms')
const hashed: TxHash = hasher.digest(payload, instant)
const scalarHashed: TxHash = hasher.digestScalar(Scalar.fromJs('AAPL'), new Date())
const read: bigint = hasher.unixOf('2023-11-14T22:13:20Z')
const hasherUnit: string = hasher.unit
const hasherAlgorithm: string = hasher.algorithm
const hasherWidth: number = hasher.width
const hasherDtype: DataType = hasher.dtype
void carried
void hashed
void scalarHashed
void read
void hasherUnit
void hasherAlgorithm
void hasherWidth
void hasherDtype

declare const root: Field
declare const batch: ArrowRecordBatch
const filled: ArrowRecordBatch = hasher.applyArrowBatch(root, batch)
const forced: ArrowRecordBatch = hasher.applyArrowBatch(root, batch, true)
void filled
void forced
// @ts-expect-error force is boolean
hasher.applyArrowBatch(root, batch, 'yes')

// The helpers.
const now: bigint = txhash.unixNow()
const later: bigint = txhash.unixNow('ns')
const at: bigint = txhash.unixOf(new Date(), 's')
const scaled: bigint = txhash.restateUnix(1_999n, 'ns', 'us')
const coupledWidth: number = txhash.width('xxh3-128')
const coupledType: DataType = txhash.dtype('xxh32')
const defaultUnit: string = txhash.DEFAULT_UNIT
const unixWidth: number = txhash.UNIX_WIDTH
void now
void later
void at
void scaled
void coupledWidth
void coupledType
void defaultUnit
void unixWidth
void xxhash.SECRET_MINIMUM_LENGTH

// The classes are also reachable through the namespace.
void new txhash.TxHasher()
void txhash.TxHash.from(narrow)
