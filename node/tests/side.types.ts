import { Side, fields, type SideField, type SideName } from '..'

const code: number = Side.BUY
const name: SideName = 'SELL'
const unknown: 0 = Side.UNKNOWN
const field: SideField = fields.side('side', { nullable: false })

// @ts-expect-error a member is read-only
Side.BUY = 7
// @ts-expect-error no member goes by that name
const missing: number = Side.NOT_A_SIDE

void [code, name, unknown, field, missing]
