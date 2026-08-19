import {
  IOBase,
  type ArrowFileInfo,
  type ArrowFileKind,
  type ArrowFileSystemHandler,
  type BatchReader,
  type Field,
} from '..'
import type { Buffer } from 'node:buffer'

// A handler is a plain object implementing the vtable, and the interface is
// what says so: an implementation that answers the wrong shape is a type
// error here rather than a failure at the first read. State of its own is
// state of its own - the methods reach it through `this`.
interface MemoryHandler extends ArrowFileSystemHandler {
  readonly files: Map<string, Buffer>
}

const handler: MemoryHandler = {
  typeName: 'memory',
  files: new Map<string, Buffer>(),
  fileInfo(path: string): ArrowFileInfo {
    const stored = this.files.get(path)
    if (stored === undefined) return { path, kind: 'not-found' }
    // A length crosses as a bigint, and as a number where it is exact.
    return { path, kind: 'file', size: BigInt(stored.length) }
  },
  list(path: string, recursive: boolean): ArrowFileInfo[] {
    return recursive ? [{ path, kind: 'directory', size: 0 }] : []
  },
  readRange(path: string, offset: bigint, length: number): Uint8Array | null {
    const stored = this.files.get(path)
    if (stored === undefined) return null
    return stored.subarray(Number(offset), Number(offset) + length)
  },
  writeFull(path: string, bytes: Buffer): void {
    this.files.set(path, bytes)
  },
  createDir(_path: string): void {},
  deleteFile(path: string): void {
    this.files.delete(path)
  },
}

const kind: ArrowFileKind = 'directory'
const info: ArrowFileInfo = { path: 'bucket/key.arrows', kind: 'file', size: 12n }
const numbered: ArrowFileInfo = { kind: 'file', size: 12 }
const absent: ArrowFileInfo = { kind: 'unknown' }

// Both spellings of the same construction: the factory says the file system
// outright, the constructor infers it from a first argument that is one.
const handle: IOBase = IOBase.fromArrowFs(handler, 'bucket/key.arrows')
const inferred: IOBase = new IOBase(handler, 'bucket/key.arrows')

// What comes back is an ordinary handle, so the whole surface is the same.
const bytes: Buffer = handle.readBytes()
const written: number = handle.writeText('AAPL')
const children: IOBase[] = [...(handle.parent?.ls(true) ?? [])]
const globbed: IOBase[] = [...handle.glob('**/*.arrows')]
const joined: IOBase = handle.joinpath('year=2024', 'part-0.arrows')
const schema: Field = handle.readArrowField()
const rows: BatchReader = handle.readArrowBatchReader()
handle.flush()

void kind
void info
void numbered
void absent
void inferred
void bytes
void written
void children
void globbed
void joined
void schema
void rows
