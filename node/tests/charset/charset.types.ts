import { MediaType, TextOptions, charset } from 'yggdryl'
import type { Charset, CharsetMark } from 'yggdryl'

const names: readonly Charset[] = charset.CHARSETS
const canonical: Charset = charset.canonicalName('cp1252')
const decoded: string = charset.decode('windows-1252', Buffer.from([0x80]))
const lossy: string = charset.decodeLossy('us-ascii', Buffer.from([0xff]))
const encoded: Buffer = charset.encode(canonical, 'désk')
const mark: CharsetMark | null = charset.fromBom(encoded)
const bom: Buffer | null = charset.bom('utf-8')

const media: MediaType = MediaType.fromString('text/csv;charset=windows-1252')
const declared: string | null = media.charset
media.setCharset('latin1')

const options: TextOptions = new TextOptions()
const capture: string = options.charset
options.charset = 'cp1252'

export { names, canonical, decoded, lossy, encoded, mark, bom, declared, capture }
