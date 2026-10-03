import {
  DEFAULT_SPILL_BYTE_SIZE,
  IOBase,
  SpillOptions,
  Url,
  type SpillOptionsInit,
} from '..'

// The bound is a u64: it crosses as a bigint both ways, and reads a whole
// number too.
const defaults: SpillOptions = new SpillOptions()
const cleared: SpillOptions = new SpillOptions(null)
const init: SpillOptionsInit = { byteSize: 0, folder: '/tmp/spill' }
const bounded: SpillOptions = new SpillOptions(init)
const exact: SpillOptions = new SpillOptions({ byteSize: 1n << 40n, folder: null })
const located: SpillOptions = new SpillOptions({ folder: Url.fromPath('/tmp/spill') })
const handled: SpillOptions = new SpillOptions({ folder: new IOBase('/tmp/spill') })
const never: bigint = SpillOptions.NEVER
const defaultBound: bigint = DEFAULT_SPILL_BYTE_SIZE
const bound: bigint = bounded.byteSize
const folder: IOBase | null = bounded.folder
const isNever: boolean = bounded.isNever()
const same: boolean = bounded.equals(exact)
const copy: SpillOptions = bounded.clone()
const text: string = bounded.toString()

// The process default: read once from the environment, or stated first.
const fromEnv: SpillOptions = SpillOptions.fromEnv()
const installed: void = SpillOptions.installEnv(bounded)

// @ts-expect-error the bound is a number or a bigint
new SpillOptions({ byteSize: '64 MiB' })
// @ts-expect-error the folder is a location
new SpillOptions({ folder: 7 })
// @ts-expect-error the bound is read, never assigned
bounded.byteSize = 0n
// @ts-expect-error installEnv takes a SpillOptions
SpillOptions.installEnv({ byteSize: 0 })
// @ts-expect-error the private constant bridges are hidden
SpillOptions._spillNeverNative

void [defaults, cleared, located, handled, never, defaultBound, bound, folder, isNever, same, copy,
  text, fromEnv, installed]
