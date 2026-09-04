import { AsciiEnum, DataType, Field } from '..'

const type = DataType.from('struct<id: bigint not null>')
const clonedType: DataType = DataType.from(type)
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
const bareVariantKind: 'variant' = bareVariant.kind
const geometryType: DataType = DataType.geometry()
const projectedGeometry: DataType = DataType.geometry('EPSG:3857')
const geographyType: DataType = DataType.geography()
const vincentyGeography: DataType = DataType.geography('OGC:CRS84', 'vincenty')
const asciiType: DataType = new DataType('ascii')
const fixedAsciiType: DataType = DataType.ascii(3)
const asciiWidth: number | null = fixedAsciiType.asciiWidth
const currencyType: DataType = new DataType('currency')
const currencyTypeWidth: number | null = currencyType.asciiWidth

void child
void indexedChild
void children
void typeHash
void typeJson
void arrowType
void bareVariantId
void bareVariantKind
void geometryType
void projectedGeometry
void geographyType
void vincentyGeography
void asciiType
void asciiWidth
void currencyType
void currencyTypeWidth

const prebuiltLists: Record<string, string[]> = AsciiEnum.prebuilt()
const prebuiltMics: AsciiEnum = AsciiEnum.fromLogicalName('mic')
const currencyMemberName: string = AsciiEnum.memberName('n/a')
const currencyPacked: bigint = DataType.ascii(3).asciiPacked('USD')
const currencyUnpacked: string = DataType.ascii(3).asciiValue(currencyPacked)
const currencyDeclaration: AsciiEnum = new AsciiEnum('Currency', { USD: 'USD' })
const currencyDeclarationJson: string = currencyDeclaration.intoJson()
const currencyDeclarationParsed: AsciiEnum = AsciiEnum.fromJson(currencyDeclarationJson)
const currencyDeclarationName: string = currencyDeclaration.name
const currencyDeclarationMembers: Record<string, string> = currencyDeclaration.members
const currencyDeclarationValue: string | null = currencyDeclaration.get('USD')
const currencyDeclarationMember: string | null = currencyDeclaration.getMember('USD')
const currencyDeclarationPrior: string | null = currencyDeclaration.insert('EUR', 'EUR')
const currencyDeclarationRemoved: string | null = currencyDeclaration.remove('EUR')
const currencyDeclarationCodes: Record<string, bigint> =
  currencyDeclaration.intoMembers(DataType.ascii(3))
const currencyDeclarationEnum: Readonly<Record<string, bigint>> =
  currencyDeclaration.intoEnum('currency')
const currencyDeclarationLength: number = currencyDeclaration.length
const currencyDeclarationEquals: boolean =
  currencyDeclaration.equals(currencyDeclarationParsed)
const currencyDeclarationClone: AsciiEnum = currencyDeclaration.clone()
const currencyDeclarationText: string = currencyDeclaration.toString()
const guidType: DataType = new DataType('guid')
const guidId: string = guidType.id
const declaredField: Field = new Field('side', DataType.ascii(3), false)
declaredField.setAsciiEnum(currencyDeclaration)
const declaredFieldEnum: AsciiEnum | null = declaredField.asciiEnum
const declaredFieldRemoved: AsciiEnum | null = declaredField.removeAsciiEnum()

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
void guidType
void guidId
void declaredField
void declaredFieldEnum
void declaredFieldRemoved
