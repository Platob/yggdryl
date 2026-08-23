import { MediaType, MimeType, Uri, Url, Urn, type PartitionEntry } from '..'

const uri = Uri.from('https://example.com/data/file.tar.gz?q=1#part')
const uriClone: Uri = Uri.from(uri)
const fileUri: Uri = Uri.fromPath('C:\\data\\file.parquet')
const filePath: string = fileUri.intoPath()
const uriJson: unknown = uri.toJSON()
const uriSegments: string[] = [...uri]
const uriJoined: Uri = uri.joinPath('nested', 'file.parquet')
const uriJoinedArray: Uri = uri.joinPath(['nested', 'file.parquet'])
const uriExtension: string | null = uri.extension
const uriExtensions: string[] = uri.extensions
const uriStem: string | null = uri.stem
const uriUser: string | null = uri.user
const uriPassword: string | null = uri.password
const uriHostname: string | null = uri.hostname
const uriBucket: string | null = uri.bucket
const uriRegion: string | null = uri.region
uri.setFileName('renamed.csv')
uri.setStem('renamed')
uri.setExtension('json')
uri.setExtensions(['json', 'gz'])
const removedExtension: boolean = uri.removeExtension()
const clearedExtensions: boolean = uri.clearExtensions()
const uriMimeType: MimeType = uri.mimeType
const uriMediaType: MediaType = uri.mediaType
uri.setMimeType('application/json')
uri.setMimeType(MimeType.fromString('application/json'))
uri.setMediaType('text/csv;encodings=application/gzip')
uri.setMediaType(MediaType.fromParts('text/csv', ['application/gzip']))

const url = Url.fromUri(uri)
const inferredUrl: Url = uri.intoUrl()
// @ts-expect-error project conversions use into*, with no legacy alias
uri.toUrl()
const urlClone: Url = Url.from(url)
const fileUrl: Url = Url.fromPath('C:\\data\\file.parquet')
const fileUrlPath: string = fileUrl.intoPath()
const urlUri: Uri = url.intoUri()
const uriFromUrl: Uri = Uri.from(url)
const urlSegments: string[] = [...url]
const urlStem: string | null = url.stem
const urlUser: string | null = url.user
const urlPassword: string | null = url.password
const urlHostname: string | null = url.hostname
const urlBucket: string | null = url.bucket
const urlRegion: string | null = url.region
const urlMimeType: MimeType = url.mimeType
const urlMediaType: MediaType = url.mediaType
url.setExtension('csv')

const urn = Urn.fromString('urn:isbn:9780131103627')
const inferredUrn: Urn = urn.intoUri().intoUrn()
const urnClone: Urn = Urn.from(urn)
const invalidUrlAtRuntime: Url = Url.from(urn)
const invalidUrnAtRuntime: Urn = Urn.from(url)
const urnUri: Uri = urn.intoUri()
const uriFromUrn: Uri = Uri.from(urn)
const namespace: string = urn.namespace
const namespaceSpecific: string = urn.namespaceSpecific
const urnSegments: string[] = [...urn]
const urnStem: string | null = urn.stem
const urnMimeType: MimeType = urn.mimeType
const urnMediaType: MediaType = urn.mediaType
urn.setFileName('value.json')

void uriClone
void fileUri
void filePath
void uriJson
void uriSegments
void uriJoined
void uriJoinedArray
void uriExtension
void uriExtensions
void uriStem
void uriUser
void uriPassword
void uriHostname
void uriBucket
void uriRegion
void removedExtension
void clearedExtensions
void uriMimeType
void uriMediaType
void urlClone
void invalidUrlAtRuntime
void inferredUrl
void fileUrl
void fileUrlPath
void urlUri
void uriFromUrl
void urlSegments
void urlStem
void urlUser
void urlPassword
void urlHostname
void urlBucket
void urlRegion
void urlMimeType
void urlMediaType
void urnClone
void invalidUrnAtRuntime
void inferredUrn
void urnUri
void uriFromUrn
void namespace
void namespaceSpecific
void urnSegments
void urnStem
void urnMimeType
void urnMediaType

const pathlike = Url.fromString('file:///lake/trades/part-0.tar.gz')
const urlName: string = pathlike.name
const urlSuffix: string = pathlike.suffix
const urlSuffixes: string[] = pathlike.suffixes
const urlParts: string[] = pathlike.parts
const urlParent: Url = pathlike.parent
const urlParents: Url[] = pathlike.parents
const urlJoined: Url = pathlike.joinpath('nested', 'part-1.arrows')
const urlRenamed: Url = pathlike.withName('part-1.tar.gz')
const urlRestemmed: Url = pathlike.withStem('part-1')
const urlResuffixed: Url = pathlike.withSuffix('.parquet')
const urlAbsolute: boolean = pathlike.isAbsolute()
const urlPosix: string = pathlike.asPosix()
const urlHref: string = pathlike.asUri()
const urlMatches: boolean = pathlike.match('*.gz')
const urlFullMatch: boolean = pathlike.fullMatch('lake/**/*.gz')
const urlIsGlob: boolean = pathlike.isGlob()
const urlRelative: string = pathlike.relativeTo('file:///lake')
const urlIsRelative: boolean = pathlike.isRelativeTo(Url.fromString('file:///lake'))
const urlExists: boolean = pathlike.exists()
const urlIsDir: boolean = pathlike.isDir()
const urlIsFile: boolean = pathlike.isFile()
const urlIsPrivate: boolean = pathlike.isPrivate()
const urlPartitions: PartitionEntry[] = pathlike.partitions
const urlPartition: string | null = pathlike.partition('year')

void urlName
void urlSuffix
void urlSuffixes
void urlParts
void urlParent
void urlParents
void urlJoined
void urlRenamed
void urlRestemmed
void urlResuffixed
void urlAbsolute
void urlPosix
void urlHref
void urlMatches
void urlFullMatch
void urlIsGlob
void urlRelative
void urlIsRelative
void urlExists
void urlIsDir
void urlIsFile
void urlIsPrivate
void urlPartitions
void urlPartition
