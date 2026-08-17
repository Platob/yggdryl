import { DataType, Field } from '..'

const type = DataType.from('struct<id: bigint not null>')
const clonedType: DataType = DataType.from(type)
const child: Field | null = type.get('id')
const indexedChild: Field | null = type.get(0)
const children: Field[] = [...type]
const typeHash: bigint = clonedType.stableHash()
const typeJson: unknown = type.toJSON()
const arrowType: DataType = DataType.fromArrow({
  toString: () => type.toString(),
})

void child
void indexedChild
void children
void typeHash
void typeJson
void arrowType

