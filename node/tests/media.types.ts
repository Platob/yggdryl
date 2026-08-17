import {
  MediaType,
  MimeType,
  type MediaTypeInput,
  type MimeTypeInput,
} from '..'

const mimeInput: MimeTypeInput = MimeType.JSON
const mime = new MimeType()
const parsedMime: MimeType = MimeType.from(mimeInput)
const knownMime: MimeType = MimeType.OCTET_STREAM
const mimeFormat: 'json' | 'json_lines' | 'yaml' | 'toml' | null =
  MimeType.JSON.format
const contentCoding: 'gzip' | 'compress' | 'deflate' | 'br' | 'zstd' | null =
  MimeType.GZIP.contentCoding

const mediaInput: MediaTypeInput = mime
const media = new MediaType()
const parsedMedia: MediaType = MediaType.from(mediaInput)
const compound: MediaType = MediaType.fromParts(MimeType.CSV, new Set([MimeType.GZIP]))
const inferred: MediaType = MediaType.fromExtensions(new Set(['json', 'gz']))
const encodings: MimeType[] = compound.encodings
const iterated: MimeType[] = [...compound]
compound.setBase('application/json')
compound.setEncodings(new Set<MimeTypeInput>([MimeType.GZIP, 'zstd']))
compound.pushEncoding(MimeType.BROTLI)

void parsedMime
void knownMime
void mimeFormat
void contentCoding
void media
void parsedMedia
void inferred
void encodings
void iterated
