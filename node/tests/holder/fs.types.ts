import {
  IOBase,
  type ArrowFileInfo,
  type ArrowFileKind,
  type BatchReader,
  type ByteReader,
  type ByteWriter,
  type Field,
  type FileSelector,
  type FileSystemHandler,
  type OutputMetadata,
  type RandomAccessReader,
} from '../..'
import type { Buffer } from 'node:buffer'

interface MemoryHandler extends FileSystemHandler {
  readonly files: Map<string, Buffer>
}

function reader(): RandomAccessReader {
  return {
    closed: false,
    read(_length: bigint): Uint8Array {
      return new Uint8Array()
    },
    readAt(_offset: bigint, _length: bigint): Uint8Array {
      return new Uint8Array()
    },
    seek(offset: bigint, _whence = 'start'): bigint {
      return offset
    },
    tell(): bigint {
      return 0n
    },
    close(): void {},
    [Symbol.dispose](): void {},
  }
}

function writer(): ByteWriter {
  return {
    closed: false,
    write(bytes: Uint8Array): bigint {
      return BigInt(bytes.length)
    },
    tell(): bigint {
      return 0n
    },
    flush(): void {},
    close(): void {},
    [Symbol.dispose](): void {},
  }
}

const handler: MemoryHandler = {
  typeName: 'memory',
  files: new Map<string, Buffer>(),
  equals(other: FileSystemHandler): boolean {
    return other === this
  },
  normalizePath(path: string): string {
    return path
  },
  fileInfo(path: string): ArrowFileInfo {
    const stored = this.files.get(path)
    if (stored === undefined) return { path, kind: 'not-found' }
    return { path, kind: 'file', size: BigInt(stored.length) }
  },
  list(selector: FileSelector): Iterable<ArrowFileInfo> {
    return selector.recursive
      ? [{ path: selector.baseDir, kind: 'directory' }]
      : []
  },
  createDir(_path: string, _recursive: boolean): void {},
  deleteDir(_path: string): void {},
  deleteDirContents(_path: string, _missingDirOk: boolean): void {},
  deleteRootDirContents(): void {},
  deleteFile(path: string): void {
    this.files.delete(path)
  },
  copyFile(_source: string, _target: string): void {},
  move(_source: string, _target: string): void {},
  openInputFile(_path: string): RandomAccessReader {
    return reader()
  },
  openInputStream(_path: string): ByteReader {
    return reader()
  },
  openOutputStream(
    _path: string,
    _metadata?: Readonly<Record<string, string>>,
  ): ByteWriter {
    return writer()
  },
  openAppendStream(
    _path: string,
    _metadata?: Readonly<Record<string, string>>,
  ): ByteWriter {
    return writer()
  },
}

const kind: ArrowFileKind = 'directory'
const info: ArrowFileInfo = {
  path: 'bucket/key.arrows',
  kind: 'file',
  size: 12n,
  mtimeNs: 1_725_000_000_000_000_001n,
}
const metadata: OutputMetadata = new Map([
  ['content-type', 'application/octet-stream'],
])

const handle: IOBase = IOBase.fromFs(
  handler,
  'bucket/v=a%2Fb.arrows',
  's3://bucket/v=a%2Fb.arrows',
)
const local: IOBase = IOBase.fromUri('file:///tmp/key.bin')
const filesystem: object | null = handle.filesystem
const rawPath: string | null = handle.path
const exactUri: string | null = handle.uri
const maskedUri: string | null = handle.maskedUri
const stream: ByteReader = handle.openInputStream()
const input: RandomAccessReader = handle.openInputFile()
const output: ByteWriter = handle.openOutputStream(metadata)
const copied: bigint = handle.copyInto(handle)
const moved: IOBase = handle.moveInto(handle)

const bytes: Buffer = handle.readBytes()
const written: number = handle.writeText('AAPL')
const children: IOBase[] = [...(handle.parent?.ls(true) ?? [])]
const globbed: IOBase[] = [...handle.glob('**/*.arrows')]
const joined: IOBase = handle.joinpath('year=2024', 'part-0.arrows')
const schema: Field = handle.readArrowField()
const rows: BatchReader = handle.readArrowReader()
handle.flush()

// @ts-expect-error filesystem sizes are exact bigint values
const imprecise: ArrowFileInfo = { path: 'x', kind: 'file', size: 12 }

void kind
void info
void local
void filesystem
void rawPath
void exactUri
void maskedUri
void stream
void input
void output
void copied
void moved
void bytes
void written
void children
void globbed
void joined
void schema
void rows
void imprecise
