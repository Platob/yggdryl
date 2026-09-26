// Two books compared by the native facts the service served: a fact is one
// exact text, a side's limits are keyed by their price text (the unpriced
// limit by `null`), live entries and deltas by `curruuid`, executions by
// `curruuid`, the scopes a snapshot replaced by symbol and scope. Nothing
// here reads a rendered string or a float; two books are the same exactly
// when `stableHash` says so and every fact agrees. The names are the served
// row's: `bid` and `ask` are the side rows, `snapshotpartitions` the column.

const NESTED = new Set(['bid', 'ask', 'executions', 'snapshotpartitions'])

/** Whether two served values are one value: primitives by `Object.is`, arrays of primitives element-wise. */
export function sameValue(left, right) {
  if (Object.is(left, right)) return true
  if (Array.isArray(left) && Array.isArray(right)) {
    return left.length === right.length && left.every((value, at) => sameValue(value, right[at]))
  }
  if (left && right && typeof left === 'object' && typeof right === 'object') {
    const keys = new Set([...Object.keys(left), ...Object.keys(right)])
    for (const key of keys) if (!sameValue(left[key], right[key])) return false
    return true
  }
  return false
}

/** The facts that differ between two flat records: `[{ name, base, scenario }]`. */
export function diffFacts(base, scenario, skip = NESTED) {
  const names = new Set([...Object.keys(base ?? {}), ...Object.keys(scenario ?? {})])
  const changes = []
  for (const name of [...names].sort()) {
    if (skip.has(name)) continue
    const before = base?.[name]
    const after = scenario?.[name]
    if (!sameValue(before, after)) changes.push({ name, base: before, scenario: after })
  }
  return changes
}

/** Diff two lists of records keyed by `key`: added, removed and changed keys, in `scenario`'s order. */
export function diffKeyed(base, scenario, key) {
  const before = new Map((base ?? []).map((item) => [item[key], item]))
  const after = new Map((scenario ?? []).map((item) => [item[key], item]))
  const added = []
  const changed = []
  for (const [id, item] of after) {
    const held = before.get(id)
    if (held === undefined) {
      added.push(item)
      continue
    }
    const facts = diffFacts(held, item, new Set())
    if (facts.length) changed.push({ [key]: id, facts })
  }
  const removed = [...before.entries()].filter(([id]) => !after.has(id)).map(([, item]) => item)
  return { added, removed, changed, same: !added.length && !removed.length && !changed.length }
}

/** The key of a limit: its price text, `null` for the unpriced limit. */
function limitKey(limit) {
  return limit.price === null || limit.price === undefined ? null : String(limit.price)
}

/** The key of a replaced scope: its symbol (`null` for none) and its scope. */
function partitionKey(partition) {
  return JSON.stringify([partition.symbol ?? null, partition.scope])
}

/** Diff two sides: limits by price, live and deltas by `curruuid`. */
export function diffSides(base, scenario) {
  const limits = diffKeyed(
    (base?.limits ?? []).map((limit) => ({ ...limit, key: limitKey(limit) })),
    (scenario?.limits ?? []).map((limit) => ({ ...limit, key: limitKey(limit) })),
    'key',
  )
  const live = diffKeyed(base?.live, scenario?.live, 'curruuid')
  const deltas = diffKeyed(base?.deltas, scenario?.deltas, 'curruuid')
  const facts = diffFacts(base, scenario, new Set(['limits', 'live', 'deltas']))
  return { limits, live, deltas, facts, same: limits.same && live.same && deltas.same && !facts.length }
}

/**
 * Diff two served books at one instant. `same` is true only when the native
 * `stableHash` agrees and nothing else differs; a scenario that changed nothing
 * at an instant is therefore exactly the base there.
 */
export function diffBooks(base, scenario) {
  const facts = diffFacts(base, scenario)
  const bid = diffSides(base?.bid, scenario?.bid)
  const ask = diffSides(base?.ask, scenario?.ask)
  const executions = diffKeyed(base?.executions, scenario?.executions, 'curruuid')
  const keyed = (book) => (book?.snapshotpartitions ?? []).map((partition) => ({ ...partition, key: partitionKey(partition) }))
  const snapshotpartitions = diffKeyed(keyed(base), keyed(scenario), 'key')
  const same =
    !facts.length &&
    bid.same &&
    ask.same &&
    executions.same &&
    snapshotpartitions.same &&
    base?.stableHash !== undefined &&
    base?.stableHash === scenario?.stableHash
  return { same, facts, bid, ask, executions, snapshotpartitions }
}

/** Align two book streams by instant text and diff each instant present in either. */
export function diffStreams(base, scenario) {
  const at = (book) => String(book.snapunix ?? book.currunix)
  const before = new Map(base.map((book) => [at(book), book]))
  const after = new Map(scenario.map((book) => [at(book), book]))
  const instants = [...new Set([...before.keys(), ...after.keys()])].sort((a, b) =>
    BigInt(a) < BigInt(b) ? -1 : BigInt(a) > BigInt(b) ? 1 : 0,
  )
  return instants.map((instant) => ({
    at: instant,
    base: before.get(instant)?.stableHash ?? null,
    scenario: after.get(instant)?.stableHash ?? null,
    changes: diffBooks(before.get(instant), after.get(instant)),
  }))
}
