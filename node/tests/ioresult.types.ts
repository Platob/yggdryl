import { IOResult } from '..'

const empty: IOResult = new IOResult()
const result: IOResult = new IOResult(10, 8)
const read: IOResult = new IOResult(10)
const stated: IOResult = new IOResult(6, 6, 4)
const counts: number[] = [result.readRows, result.writtenRows, result.skippedRows]
const isEmpty: boolean = empty.isEmpty()
const sum: IOResult = result.add(read)
const equal: boolean = result.equals(read)
const ordered: number = result.compare(read)
const hash: bigint = result.stableHash()
const clone: IOResult = result.clone()
const text: string = result.toString()

// @ts-expect-error counts are read-only
result.readRows = 1
// @ts-expect-error a count is a number
new IOResult('10')
// @ts-expect-error a count is a number, not a bigint
new IOResult(10n)
// @ts-expect-error results add to results
result.add(1)

void [empty, read, stated, counts, isEmpty, sum, equal, ordered, hash, clone, text]
