import { gzip, zlib, zstd } from '../..'
import { Buffer } from 'node:buffer'

const content = new Uint8Array([1, 2, 3])
const gzipped: Buffer = gzip.dumps(content, 6)
const gunzipped: Buffer = gzip.loads(gzipped)
const deflated: Buffer = zlib.dumps(content)
const inflated: Buffer = zlib.loads(deflated)
const rawDeflated: Buffer = zlib.dumpsRaw(content, null)
const rawInflated: Buffer = zlib.loadsRaw(rawDeflated)
const compressed: Buffer = zstd.dumps(Buffer.from('abc'))
const decompressed: Buffer = zstd.loads(compressed)
void gunzipped
void inflated
void rawInflated
void decompressed

// @ts-expect-error gzip has no raw DEFLATE framing
gzip.loadsRaw(content)
// @ts-expect-error a level is a number on the shared 0-9 scale
zstd.dumps(content, 'fast')
