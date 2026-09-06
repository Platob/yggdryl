'use strict'

// Deterministic 64 MiB boundary gate. Run after a release build:
// node benchmarks/filesystem.js

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { performance } = require('node:perf_hooks')
const { IOBase } = require('yggdryl')

const SIZE = 64 * 1024 * 1024
const CHUNK = 1024 * 1024
const SAMPLES = 5
const domain = {}

function input(location) {
  const fd = fs.openSync(location, 'r')
  let position = 0n
  let closed = false
  const readAt = (offset, length) => {
    const bytes = Buffer.allocUnsafe(Number(length))
    const count = fs.readSync(fd, bytes, 0, bytes.length, Number(offset))
    return bytes.subarray(0, count)
  }
  return {
    read(length) {
      const bytes = readAt(position, length)
      position += BigInt(bytes.length)
      return bytes
    },
    readAt,
    seek(offset, whence) {
      const base =
        whence === 'current'
          ? position
          : whence === 'end'
            ? fs.fstatSync(fd, { bigint: true }).size
            : 0n
      position = base + offset
      return position
    },
    tell: () => position,
    close() {
      if (!closed) fs.closeSync(fd)
      closed = true
    },
    get closed() {
      return closed
    },
  }
}

function output(location, append) {
  const fd = fs.openSync(location, append ? 'a' : 'w')
  let position = append ? fs.fstatSync(fd, { bigint: true }).size : 0n
  let closed = false
  return {
    write(bytes) {
      const count = fs.writeSync(fd, bytes)
      position += BigInt(count)
      return BigInt(count)
    },
    tell: () => position,
    flush: () => fs.fsyncSync(fd),
    close() {
      if (!closed) fs.closeSync(fd)
      closed = true
    },
    get closed() {
      return closed
    },
  }
}

const handler = {
  typeName: 'local',
  domain,
  equals: (other) => other?.domain === domain,
  normalizePath: path.normalize,
  fileInfo(location) {
    const stat = fs.statSync(location, { bigint: true, throwIfNoEntry: false })
    if (!stat) return { path: location, kind: 'not-found' }
    return stat.isDirectory()
      ? { path: location, kind: 'directory', mtimeNs: stat.mtimeNs }
      : {
          path: location,
          kind: 'file',
          size: stat.size,
          mtimeNs: stat.mtimeNs,
        }
  },
  list(selector) {
    return fs
      .readdirSync(selector.baseDir, {
        recursive: selector.recursive,
        withFileTypes: true,
      })
      .map((entry) =>
        this.fileInfo(path.join(entry.parentPath ?? entry.path, entry.name)),
      )
  },
  createDir: (location, recursive) => fs.mkdirSync(location, { recursive }),
  deleteDir: fs.rmdirSync,
  deleteDirContents(location, missingDirOk) {
    if (!fs.existsSync(location) && missingDirOk) return
    for (const child of fs.readdirSync(location))
      fs.rmSync(path.join(location, child), { recursive: true })
  },
  deleteRootDirContents() {
    throw new Error('Unsupported: root deletion')
  },
  deleteFile: fs.unlinkSync,
  copyFile: fs.copyFileSync,
  move: fs.renameSync,
  openInputFile: input,
  openInputStream: input,
  openOutputStream: (location) => output(location, false),
  openAppendStream: (location) => output(location, true),
}

function median(values) {
  return [...values].sort((left, right) => left - right)[
    Math.floor(values.length / 2)
  ]
}

function measure(operation) {
  operation()
  const samples = []
  for (let index = 0; index < SAMPLES; index += 1) {
    const started = performance.now()
    operation()
    samples.push(SIZE / 1024 / 1024 / ((performance.now() - started) / 1000))
  }
  return median(samples)
}

function gate(name, wrapped, direct) {
  const directRate = measure(direct)
  const wrappedRate = measure(wrapped)
  const ratio = wrappedRate / directRate
  console.log(
    `${name}: wrapper ${wrappedRate.toFixed(1)} MiB/s; direct ${directRate.toFixed(1)} MiB/s; ${(ratio * 100).toFixed(1)}%`,
  )
  assert.ok(
    ratio >= 0.75,
    `${name} wrapper throughput is more than 25% below direct`,
  )
}

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-filesystem-gate-'))
try {
  const source = path.join(root, 'source.bin')
  const wrappedOutput = path.join(root, 'wrapped.bin')
  const directOutput = path.join(root, 'direct.bin')
  const wrappedCopy = path.join(root, 'wrapped-copy.bin')
  const directCopy = path.join(root, 'direct-copy.bin')
  fs.writeFileSync(source, Buffer.alloc(SIZE, 0x5a))

  gate(
    'read',
    () => {
      const stream = IOBase.fromFs(handler, source).openInputStream()
      while (stream.read(BigInt(CHUNK)).length !== 0) {}
      stream.close()
    },
    () => {
      const stream = input(source)
      while (stream.read(BigInt(CHUNK)).length !== 0) {}
      stream.close()
    },
  )

  const chunk = Buffer.alloc(CHUNK, 0xa5)
  gate(
    'write',
    () => {
      const stream = IOBase.fromFs(handler, wrappedOutput).openOutputStream()
      for (let offset = 0; offset < SIZE; offset += CHUNK) stream.write(chunk)
      stream.close()
    },
    () => {
      const stream = output(directOutput, false)
      for (let offset = 0; offset < SIZE; offset += CHUNK) stream.write(chunk)
      stream.close()
    },
  )

  gate(
    'copy',
    () =>
      IOBase.fromFs(handler, source).copyInto(
        IOBase.fromFs(handler, wrappedCopy),
      ),
    () => fs.copyFileSync(source, directCopy),
  )
} finally {
  fs.rmSync(root, { recursive: true, force: true })
}
