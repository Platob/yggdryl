import { Arn, MediaType, MimeType, Uri, Url, Urn, type PartitionEntry } from '..'

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
const urlFromUrlStrict: Url = Url.fromUri(url)
const urlFromTextStrict: Url = Url.fromUri('https://example.com/data.csv')
const inferredUrl: Url = uri.intoUrl()
const uriLocator: Url = uri.locator()
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
const urlLocator: Url = url.locator()
url.setExtension('csv')

const urn = Urn.fromString('urn:lake:trades:2026:part.csv')
const urnFromUriStrict: Urn = Urn.fromUri(urn.intoUri())
const urnFromTextStrict: Urn = Urn.fromUri('urn:lake:trades:2026:part.csv')
// The strict door takes every identifier, and refuses this one at runtime.
const invalidUrnFromUri: Urn = Urn.fromUri(url)
const inferredUrn: Urn = urn.intoUri().intoUrn()
const urnClone: Urn = Urn.from(urn)
const resolvedUrn: Url = Url.from(urn)
const invalidUrnAtRuntime: Urn = Urn.from(url)
const urnUri: Uri = urn.intoUri()
const uriFromUrn: Uri = Uri.from(urn)
const namespace: string = urn.namespace
const namespaceSpecific: string = urn.namespaceSpecific
const urnSegments: string[] = [...urn]
const urnStem: string | null = urn.stem
const urnMimeType: MimeType = urn.mimeType
const urnMediaType: MediaType = urn.mediaType
const urnLocatorPath: string = urn.locatorPath()
const urnResolved: Url = urn.resolve('s3://market-data/warehouse/')
const urnResolvedUrl: Url = urn.resolve(Url.fromString('s3://market-data/warehouse/'))
const urnLocator: Url = urn.locator()
urn.setFileName('value.json')

const arn = Arn.fromString('arn:aws:s3:::market-data/2026/part.parquet')
const arnFromUriStrict: Arn = Arn.fromUri(arn.intoUri())
const arnFromTextStrict: Arn = Arn.fromUri('arn:aws:s3:::market-data')
// The strict door takes every identifier, and refuses this one at runtime.
const invalidArnFromUri: Arn = Arn.fromUri(urn)
const inferredArn: Arn = arn.intoUri().intoArn()
const arnClone: Arn = Arn.from(arn)
const arnParts: Arn = Arn.fromParts('aws', 's3', '', '', 'market-data/part.parquet')
const arnOptionalParts: Arn = Arn.fromParts('aws', 's3', null, undefined, 'market-data')
const invalidArnAtRuntime: Arn = Arn.from(urn)
const arnUri: Uri = arn.intoUri()
const uriFromArn: Uri = Uri.from(arn)
const urlFromArn: Url = Url.from(arn)
const arnPartition: string = arn.partition
const arnService: string = arn.service
const arnRegion: string | null = arn.region
const arnAccount: string | null = arn.account
const arnResource: string = arn.resource
const arnResourceSeparator: string | null = arn.resourceSeparator
const arnResourceType: string | null = arn.resourceType
const arnResourceId: string = arn.resourceId
const arnBucket: string | null = arn.bucket
const arnKey: string | null = arn.key
const arnTable: string | null = Arn.fromString(
  'arn:aws:s3tables:us-east-1:123456789012:bucket/lake/table/t-a1',
).table
const arnLocator: Url = arn.locator()
const arnSegments: string[] = [...arn]
const arnStem: string | null = arn.stem
const arnMimeType: MimeType = arn.mimeType
const arnMediaType: MediaType = arn.mediaType
arn.setFileName('value.json')

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
void urlFromUrlStrict
void urlFromTextStrict
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
void urlLocator
void uriLocator
void urnClone
void resolvedUrn
void invalidUrnAtRuntime
void urnFromUriStrict
void urnFromTextStrict
void invalidUrnFromUri
void inferredUrn
void urnUri
void uriFromUrn
void namespace
void namespaceSpecific
void urnSegments
void urnStem
void urnMimeType
void urnMediaType
void urnLocatorPath
void urnResolved
void urnResolvedUrl
void urnLocator
void arnClone
void arnParts
void arnOptionalParts
void invalidArnAtRuntime
void arnFromUriStrict
void arnFromTextStrict
void invalidArnFromUri
void inferredArn
void arnUri
void uriFromArn
void urlFromArn
void arnPartition
void arnService
void arnRegion
void arnAccount
void arnResource
void arnResourceSeparator
void arnResourceType
void arnResourceId
void arnBucket
void arnKey
void arnTable
void arnLocator
void arnSegments
void arnStem
void arnMimeType
void arnMediaType

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
