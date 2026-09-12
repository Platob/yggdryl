import {
  DataType,
  Field,
  StringEnum,
  type BytesParameters,
  type BytesParametersInput,
  type StringParameters,
  type StringParametersInput,
} from '../..'

const type = DataType.from('struct<id: bigint not null>')
const clonedType: DataType = DataType.from(type)
const regexType: DataType = DataType.fromRegex('(?<id>\\d+)')
const child: Field | null = type.getField('id')
const indexedChild: Field | null = type.getField(0)
const pathChild: Field | null = type.getFieldByPath('id')
const positionalChild: Field = type.fieldAt(0)
const raisingChild: Field = type.field('id')
void pathChild
void positionalChild
void raisingChild
const children: Field[] = [...type]
const typeHash: bigint = clonedType.stableHash()
const typeJson: unknown = type.toJSON()
const arrowType: DataType = DataType.fromArrow({
  toString: () => type.toString(),
})

// The parenthesis disambiguates: a bare variant() is the Variant datatype
// with its own literal id, and variant(fields) stays the dense-union sugar.
const bareVariant = DataType.variant()
const bareVariantId: 'variant' = bareVariant.id
const bareVariantKind: 'nested' = bareVariant.kind
const geometryType: DataType = DataType.geometry()
const projectedGeometry: DataType = DataType.geometry('EPSG:3857')
const geographyType: DataType = DataType.geography()
const vincentyGeography: DataType = DataType.geography('OGC:CRS84', 'vincenty')
const asciiType: DataType = DataType.ascii()
const fixedAsciiType: DataType = DataType.fixedAscii(3)
const fixedUtf8Type: DataType = DataType.fixedUtf8(8)
const utf8Type: DataType = DataType.utf8()
const largeUtf8Type: DataType = DataType.largeUtf8()
const utf8ViewType: DataType = DataType.utf8View()
const binaryType: DataType = DataType.binary()
const largeBinaryType: DataType = DataType.largeBinary()
const binaryViewType: DataType = DataType.binaryView()
const fixedBinaryType: DataType = DataType.fixedSizeBinary(16)
// The one string datatype and the one byte datatype, declared whole.
const plainString: DataType = DataType.string()
const latinString: DataType = DataType.string({
  layout: 'large_string',
  charset: 'windows-1252',
  max: 32,
})
const boundedBytes: DataType = DataType.bytes({ layout: 'binary_view', bound: 64 })
const stringParameters: StringParameters | null = latinString.stringParameters
const stringLayout: string = stringParameters!.layout
const stringCharset: string = stringParameters!.charset
const stringBound: number | undefined = stringParameters!.bound
const stringFixed: number | undefined = stringParameters!.fixed
const stringMax: number | undefined = stringParameters!.max
const bytesParameters: BytesParameters | null = boundedBytes.bytesParameters
const bytesLayout: string = bytesParameters!.layout
const bytesMax: number | undefined = bytesParameters!.max
const stringInput: StringParametersInput = { charset: 'us-ascii', fixed: 4 }
const bytesInput: BytesParametersInput = { fixed: 16 }
const charset: string | null = fixedAsciiType.charset
const fixedByteWidth: number | null = fixedAsciiType.fixedByteWidth
const currencyType: DataType = new DataType('currency')
const currencyTypeWidth: number | null = currencyType.fixedByteWidth
const urlType: DataType = new DataType('url')
const urlTypeWidth: number | null = urlType.fixedByteWidth

void child
void indexedChild
void children
void typeHash
void typeJson
void arrowType
void regexType
void bareVariantId
void bareVariantKind
void geometryType
void projectedGeometry
void geographyType
void vincentyGeography
void asciiType
void fixedAsciiType
void fixedUtf8Type
void utf8Type
void largeUtf8Type
void utf8ViewType
void binaryType
void largeBinaryType
void binaryViewType
void fixedBinaryType
void plainString
void latinString
void boundedBytes
void stringLayout
void stringCharset
void stringBound
void stringFixed
void stringMax
void bytesLayout
void bytesMax
void stringInput
void bytesInput
void charset
void fixedByteWidth
void currencyType
void currencyTypeWidth
void urlType
void urlTypeWidth

const prebuiltLists: Record<string, string[]> = StringEnum.prebuilt()
const prebuiltMics: StringEnum = StringEnum.fromLogicalName('mic')
const currencyMemberName: string = StringEnum.memberName('n/a')
const currencyPacked: bigint = DataType.fixedAscii(3).asciiPacked('USD')
const currencyUnpacked: string = DataType.fixedAscii(3).asciiValue(currencyPacked)
const currencyDeclaration: StringEnum = new StringEnum('Currency', { USD: 'USD' })
const currencyDeclarationJson: string = currencyDeclaration.intoJson()
const currencyDeclarationParsed: StringEnum = StringEnum.fromJson(currencyDeclarationJson)
const currencyDeclarationName: string = currencyDeclaration.name
const currencyDeclarationMembers: Record<string, string> = currencyDeclaration.members
const currencyDeclarationValue: string | null = currencyDeclaration.get('USD')
const currencyDeclarationMember: string | null = currencyDeclaration.getMember('USD')
const currencyDeclarationPrior: string | null = currencyDeclaration.insert('EUR', 'EUR')
const currencyDeclarationRemoved: string | null = currencyDeclaration.remove('EUR')
const currencyDeclarationCodes: Record<string, bigint> =
  currencyDeclaration.intoMembers(DataType.fixedAscii(3))
const currencyDeclarationEnum: Readonly<Record<string, bigint>> =
  currencyDeclaration.intoEnum('currency')
const currencyDeclarationLength: number = currencyDeclaration.length
const currencyDeclarationEquals: boolean =
  currencyDeclaration.equals(currencyDeclarationParsed)
const currencyDeclarationClone: StringEnum = currencyDeclaration.clone()
const currencyDeclarationText: string = currencyDeclaration.toString()
const uuidType: DataType = new DataType('uuid')
const uuidId: string = uuidType.id
const declaredField: Field = new Field('side', DataType.fixedAscii(3), false)
declaredField.setStringEnum(currencyDeclaration)
const declaredFieldEnum: StringEnum | null = declaredField.stringEnum
const declaredFieldRemoved: StringEnum | null = declaredField.removeStringEnum()

void prebuiltLists
void prebuiltMics
void currencyMemberName
void currencyPacked
void currencyUnpacked
void currencyDeclaration
void currencyDeclarationJson
void currencyDeclarationParsed
void currencyDeclarationName
void currencyDeclarationMembers
void currencyDeclarationValue
void currencyDeclarationMember
void currencyDeclarationPrior
void currencyDeclarationRemoved
void currencyDeclarationCodes
void currencyDeclarationEnum
void currencyDeclarationLength
void currencyDeclarationEquals
void currencyDeclarationClone
void currencyDeclarationText
void uuidType
void uuidId
void declaredField
void declaredFieldEnum
void declaredFieldRemoved
