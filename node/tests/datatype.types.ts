import { Vector as ArrowVector } from 'apache-arrow'

import { AsciiDictionary, AsciiEnum, DataType, Field } from '..'

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
const asciiType: DataType = DataType.ascii(3)
const asciiWidth: number | null = asciiType.asciiWidth
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
void asciiWidth
void currencyType
void currencyTypeWidth

const currencies = new AsciiDictionary('ascii32')
const seededCurrencies: AsciiDictionary = AsciiDictionary.fromValues(
  DataType.ascii(3),
  ['USD', 'EUR'],
  'int64',
)
const prebuiltLists: Record<string, string[]> = AsciiDictionary.prebuilt()
const prebuiltMics: AsciiDictionary = AsciiDictionary.fromLogicalName('mic')
const currencyCode: number = currencies.push('USD')
const currencyValue: string | null = currencies.get(0)
const currencyLookup: number | null = currencies.getCode('USD')
const currencyValues: string[] = currencies.values()
const currencyMemberName: string = AsciiDictionary.memberName('n/a')
const currencyCount: number = currencies.length
const currencyDtype: DataType = currencies.dtype
const currencyKey: DataType = currencies.key
const currencyWidth: DataType = currencies.valuesDtype
const currencyEquals: boolean = currencies.equals(seededCurrencies)
const currencyText: string = currencies.toString()
const currencyClone: AsciiDictionary = currencies.clone()
const currencyEnum: Readonly<Record<string, bigint>> = currencies.intoEnum('Currency')
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
  currencyDeclaration.intoMembers('ascii32')
const currencyDeclarationDictionary: AsciiDictionary =
  currencyDeclaration.intoDictionary('ascii32')
const currencyDeclarationLength: number = currencyDeclaration.length
const currencyDeclarationEquals: boolean =
  currencyDeclaration.equals(currencyDeclarationParsed)
const currencyDeclarationClone: AsciiEnum = currencyDeclaration.clone()
const currencyDeclarationText: string = currencyDeclaration.toString()
const guidType: DataType = new DataType('guid')
const guidId: string = guidType.id
const declaredField: Field = new Field('side', 'ascii32', false)
declaredField.setAsciiEnum(currencyDeclaration)
const declaredFieldEnum: AsciiEnum | null = declaredField.asciiEnum
const declaredFieldRemoved: AsciiEnum | null = declaredField.removeAsciiEnum()
const currencyColumn: ArrowVector = currencies.intoArrowArray(['USD', null])
const recoveredCurrencies: AsciiDictionary =
  AsciiDictionary.fromArrowArray(currencyColumn)

void prebuiltLists
void prebuiltMics
void currencyCode
void currencyValue
void currencyLookup
void currencyValues
void currencyMemberName
void currencyCount
void currencyDtype
void currencyKey
void currencyWidth
void currencyEquals
void currencyText
void currencyClone
void currencyEnum
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
void currencyDeclarationDictionary
void currencyDeclarationLength
void currencyDeclarationEquals
void currencyDeclarationClone
void currencyDeclarationText
void guidType
void guidId
void declaredField
void declaredFieldEnum
void declaredFieldRemoved
void recoveredCurrencies
